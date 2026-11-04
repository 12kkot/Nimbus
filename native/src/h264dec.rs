//! H.264 decoding through Windows' own decoder (Media Foundation), for
//! Discord Go Live streams: Annex B frames in, RGBA pictures out.
//!
//! The decoder is a synchronous MFT on a thread of its own. Frames before the
//! first keyframe are skipped (nothing to decode them against), and so is
//! everything after a dropped frame until the next keyframe.

use std::mem::ManuallyDrop;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};

use livekit::webrtc::native::yuv_helper;
use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Resource, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

use crate::Pixels;

/// Where decoded pictures go (called on the decoder thread).
pub type FrameSink = Box<dyn Fn(Pixels) + Send>;

/// Handle to the decoder thread; dropping it ends the thread.
pub struct Decoder {
    frames: SyncSender<Vec<u8>>,
    /// A frame was dropped: wait for a keyframe before decoding again.
    resync: bool,
    /// Frames dropped because decoding fell behind.
    dropped: u64,
}

impl Decoder {
    pub fn start(sink: FrameSink) -> Self {
        // About two seconds of frames: decoding should keep up, this only absorbs bursts.
        let (frames, queue) = sync_channel::<Vec<u8>>(60);
        let _ = std::thread::Builder::new().name("h264-decoder".into()).spawn(move || {
            if let Err(e) = unsafe { run(queue, sink) } {
                crate::log_line(&format!("h264 decoder stopped: {e}"));
            }
        });
        Self { frames, resync: true, dropped: 0 }
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// A frame went missing: what follows can't be decoded cleanly until a keyframe.
    pub fn lost(&mut self) {
        self.resync = true;
    }

    /// Queues an Annex B frame; `key` if it carries an IDR picture.
    pub fn push(&mut self, frame: Vec<u8>, key: bool) {
        if self.resync && !key {
            return;
        }
        match self.frames.try_send(frame) {
            Ok(()) => self.resync = false,
            Err(TrySendError::Full(_)) => {
                self.resync = true;
                self.dropped += 1;
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

fn err(context: &str, e: windows::core::Error) -> String {
    format!("{context}: {e}")
}

/// The graphics card's video decoder: Media Foundation's decoder is given a Direct3D device and then
/// decodes on the card (NVDEC, AMD's and Intel's video engines) instead of on the processor, handing
/// back pictures that stay in video memory. Only the finished picture is copied out for the window.
struct Gpu {
    /// The decoder keeps using it; it must stay alive as long as the decoder does.
    manager: IMFDXGIDeviceManager,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    /// One CPU-readable texture reused for every picture (size it was made for).
    staging: Option<(ID3D11Texture2D, u32, u32)>,
}

impl Gpu {
    unsafe fn open() -> Result<Self, String> {
        unsafe {
            let (mut device, mut context) = (None, None);
            D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_VIDEO_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context))
                .map_err(|e| err("D3D11CreateDevice", e))?;
            let device: ID3D11Device = device.ok_or("no Direct3D device")?;
            let context: ID3D11DeviceContext = context.ok_or("no Direct3D context")?;
            // The decoder works on this device from threads of its own.
            if let Ok(multithread) = device.cast::<ID3D11Multithread>() {
                let _ = multithread.SetMultithreadProtected(true);
            }
            let (mut token, mut manager) = (0u32, None);
            MFCreateDXGIDeviceManager(&mut token, &mut manager).map_err(|e| err("MFCreateDXGIDeviceManager", e))?;
            let manager = manager.ok_or("no device manager")?;
            manager.ResetDevice(&device, token).map_err(|e| err("ResetDevice", e))?;
            let mut gpu = Self { manager, device, context, staging: None };
            // The card must be able to hand NV12 pictures back: try once with a small one.
            gpu.staging_texture(64, 64).ok_or("the card cannot give NV12 pictures to the processor")?;
            gpu.staging = None;
            Ok(gpu)
        }
    }

    /// The reusable readable texture for pictures of this (coded) size.
    unsafe fn staging_texture(&mut self, width: u32, height: u32) -> Option<ID3D11Texture2D> {
        unsafe {
            if let Some((texture, w, h)) = &self.staging {
                if (*w, *h) == (width, height) {
                    return Some(texture.clone());
                }
            }
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_NV12,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut made = None;
            self.device.CreateTexture2D(&desc, None, Some(&mut made)).ok()?;
            let texture = made?;
            self.staging = Some((texture.clone(), width, height));
            Some(texture)
        }
    }

    /// A decoded picture that lives in video memory, as RGBA. `None` when the sample is not one of those.
    unsafe fn rgba(&mut self, sample: &IMFSample, w: u32, h: u32) -> Option<Option<Pixels>> {
        unsafe {
            let buffer = sample.GetBufferByIndex(0).ok()?;
            let dxgi: IMFDXGIBuffer = buffer.cast().ok()?;
            let mut raw = std::ptr::null_mut();
            dxgi.GetResource(&ID3D11Texture2D::IID, &mut raw).ok()?;
            if raw.is_null() {
                return None;
            }
            let texture = ID3D11Texture2D::from_raw(raw);
            let index = dxgi.GetSubresourceIndex().ok()?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            let staging = self.staging_texture(desc.Width, desc.Height)?;
            let (staging, texture): (ID3D11Resource, ID3D11Resource) = (staging.cast().ok()?, texture.cast().ok()?);
            self.context.CopySubresourceRegion(&staging, 0, 0, 0, 0, &texture, index, None);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).ok()?;
            let stride = mapped.RowPitch;
            // Y first, then the interleaved chroma plane, after the texture's full (padded) height.
            let all = std::slice::from_raw_parts(mapped.pData as *const u8, (stride * desc.Height * 3 / 2) as usize);
            let (y, uv) = all.split_at((stride * desc.Height) as usize);
            let picture = (w <= desc.Width && h <= desc.Height && w > 0 && h > 0).then(|| {
                let mut pixels = Pixels::new(w, h);
                yuv_helper::nv12_to_abgr(y, stride, uv, stride, pixels.make_mut_bytes(), w * 4, w as i32, h as i32);
                pixels
            });
            self.context.Unmap(&staging, 0);
            Some(picture)
        }
    }
}

struct Mft {
    transform: IMFTransform,
    /// Picture size, and the decoder's row pitch for it.
    width: u32,
    height: u32,
    stride: u32,
    output_size: u32,
    provides_samples: bool,
    gpu: Option<Gpu>,
}

impl Mft {
    unsafe fn open() -> Result<Self, String> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| err("MFStartup", e))?;
            let transform: IMFTransform =
                CoCreateInstance(&CLSID_MSH264DecoderMFT, None, CLSCTX_INPROC_SERVER).map_err(|e| err("H.264 decoder", e))?;
            // On the graphics card when it can; otherwise (and for any trouble) on the processor, as before.
            let gpu = if std::env::var_os("NIMBUS_SOFTWARE_DECODE").is_some() {
                None
            } else {
                match Gpu::open() {
                    Ok(gpu) => match transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, gpu.manager.as_raw() as usize) {
                        Ok(()) => Some(gpu),
                        Err(e) => {
                            crate::log_line(&format!("h264 decoder: the card's decoder is not available ({e}); using the processor"));
                            None
                        }
                    },
                    Err(e) => {
                        crate::log_line(&format!("h264 decoder: the card's decoder is not available ({e}); using the processor"));
                        None
                    }
                }
            };
            if let Ok(attributes) = transform.GetAttributes() {
                // Show each frame as soon as it's decoded instead of buffering for reordering.
                let _ = attributes.SetUINT32(&MF_LOW_LATENCY, 1);
            }
            let input = MFCreateMediaType().map_err(|e| err("MFCreateMediaType", e))?;
            input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(|e| err("in major", e))?;
            input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264).map_err(|e| err("in subtype", e))?;
            transform.SetInputType(0, &input, 0).map_err(|e| err("SetInputType", e))?;
            crate::log_line(if gpu.is_some() { "h264 decoder: on the graphics card" } else { "h264 decoder: on the processor" });
            let mut mft = Self { transform, width: 0, height: 0, stride: 0, output_size: 0, provides_samples: false, gpu };
            mft.choose_output()?;
            mft.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0).map_err(|e| err("begin streaming", e))?;
            mft.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0).map_err(|e| err("start of stream", e))?;
            Ok(mft)
        }
    }

    /// Picks NV12 output and learns the picture size (again after a stream change).
    unsafe fn choose_output(&mut self) -> Result<(), String> {
        unsafe {
            let mut index = 0;
            loop {
                let candidate = self.transform.GetOutputAvailableType(0, index).map_err(|e| err("GetOutputAvailableType", e))?;
                if candidate.GetGUID(&MF_MT_SUBTYPE).ok() == Some(MFVideoFormat_NV12) {
                    self.transform.SetOutputType(0, &candidate, 0).map_err(|e| err("SetOutputType", e))?;
                    let size = candidate.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0);
                    self.width = (size >> 32) as u32;
                    self.height = size as u32;
                    // The frame size is the coded one (1080 decodes as 1088);
                    // the visible picture is the display aperture, when given.
                    let mut area = [0u8; 16];
                    let mut len = 0u32;
                    if candidate.GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, &mut area, Some(&mut len)).is_ok() && len >= 16 {
                        let (cx, cy) = (i32::from_le_bytes(area[8..12].try_into().unwrap()), i32::from_le_bytes(area[12..16].try_into().unwrap()));
                        if cx > 0 && cy > 0 && (cx as u32) <= self.width && (cy as u32) <= self.height {
                            (self.width, self.height) = (cx as u32, cy as u32);
                        }
                    }
                    self.stride = candidate.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| s as i32).ok().filter(|s| *s > 0).map(|s| s as u32).unwrap_or(self.width);
                    break;
                }
                index += 1;
            }
            let info = self.transform.GetOutputStreamInfo(0).map_err(|e| err("GetOutputStreamInfo", e))?;
            self.provides_samples = info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32) != 0;
            self.output_size = info.cbSize.max(self.stride * self.height * 3 / 2).max(1);
            Ok(())
        }
    }

    unsafe fn decode(&mut self, frame: &[u8], time: i64, sink: &FrameSink) -> Result<(), String> {
        unsafe {
            let media = MFCreateMemoryBuffer(frame.len() as u32).map_err(|e| err("MFCreateMemoryBuffer", e))?;
            let mut ptr: *mut u8 = std::ptr::null_mut();
            media.Lock(&mut ptr, None, None).map_err(|e| err("Lock", e))?;
            std::ptr::copy_nonoverlapping(frame.as_ptr(), ptr, frame.len());
            let _ = media.Unlock();
            media.SetCurrentLength(frame.len() as u32).map_err(|e| err("SetCurrentLength", e))?;
            let sample = MFCreateSample().map_err(|e| err("MFCreateSample", e))?;
            sample.AddBuffer(&media).map_err(|e| err("AddBuffer", e))?;
            sample.SetSampleTime(time).map_err(|e| err("SetSampleTime", e))?;
            self.transform.ProcessInput(0, &sample, 0).map_err(|e| err("ProcessInput", e))?;
            loop {
                let own = if self.provides_samples {
                    None
                } else {
                    let buffer = MFCreateMemoryBuffer(self.output_size).map_err(|e| err("MFCreateMemoryBuffer", e))?;
                    let out = MFCreateSample().map_err(|e| err("MFCreateSample", e))?;
                    out.AddBuffer(&buffer).map_err(|e| err("AddBuffer", e))?;
                    Some(out)
                };
                let mut buffers = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: ManuallyDrop::new(own), dwStatus: 0, pEvents: ManuallyDrop::new(None) }];
                let mut status = 0u32;
                let result = self.transform.ProcessOutput(0, &mut buffers, &mut status);
                let out = ManuallyDrop::take(&mut buffers[0].pSample);
                drop(ManuallyDrop::take(&mut buffers[0].pEvents));
                match result {
                    Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                    Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                        self.choose_output()?;
                        crate::log_line(&format!("h264 decoder: picture {}x{} (pitch {})", self.width, self.height, self.stride));
                        continue;
                    }
                    Err(e) => return Err(err("ProcessOutput", e)),
                    Ok(()) => {}
                }
                let Some(out) = out else { continue };
                // Decoded either way (later pictures lean on this one), but only turned into a picture when someone can see it.
                if !crate::WINDOW_SHOWN.load(std::sync::atomic::Ordering::Relaxed) {
                    continue;
                }
                if let Some(pixels) = self.to_rgba(&out) {
                    sink(pixels);
                }
            }
        }
    }

    /// NV12 (with the decoder's padding) to RGBA at the real picture size.
    unsafe fn to_rgba(&mut self, sample: &IMFSample) -> Option<Pixels> {
        unsafe {
            let (w, h, stride) = (self.width, self.height, self.stride);
            if w == 0 || h == 0 {
                return None;
            }
            // A picture that stayed in video memory is read from there.
            if let Some(gpu) = &mut self.gpu {
                if let Some(picture) = gpu.rgba(sample, w, h) {
                    return picture;
                }
            }
            let media = sample.ConvertToContiguousBuffer().ok()?;
            let mut len = media.GetCurrentLength().ok()?;
            // The real row pitch, when the buffer can tell (unusual widths get padded).
            let pitch_2d = media.cast::<IMF2DBuffer>().ok().and_then(|b| {
                let (mut scan0, mut pitch) = (std::ptr::null_mut(), 0i32);
                b.Lock2D(&mut scan0, &mut pitch).ok()?;
                let _ = b.Unlock2D();
                (pitch > 0).then_some(pitch as u32)
            });
            let stride = pitch_2d.unwrap_or(stride);
            let mut ptr: *mut u8 = std::ptr::null_mut();
            media.Lock(&mut ptr, None, Some(&mut len)).ok()?;
            let data = std::slice::from_raw_parts(ptr, len as usize);
            // Rows may be padded below the picture (1080 decodes as 1088): the
            // chroma plane starts after the padded height, which the length gives away.
            let padded_h = (len / stride * 2 / 3).max(h);
            let result = (|| {
                let y = data.get(..(stride * h) as usize)?;
                let uv = data.get((stride * padded_h) as usize..)?;
                if uv.len() < (stride * h.div_ceil(2)) as usize {
                    return None;
                }
                let mut pixels = Pixels::new(w, h);
                // libyuv "ABGR" is R,G,B,A in memory, which is what Slint's Rgba8Pixel holds.
                yuv_helper::nv12_to_abgr(y, stride, uv, stride, pixels.make_mut_bytes(), w * 4, w as i32, h as i32);
                Some(pixels)
            })();
            let _ = media.Unlock();
            result
        }
    }
}

unsafe fn run(queue: Receiver<Vec<u8>>, sink: FrameSink) -> Result<(), String> {
    unsafe {
        let mut mft = Mft::open()?;
        let mut time = 0i64;
        let mut failures = 0;
        while let Ok(frame) = queue.recv() {
            time += 333_333; // nominal 30 fps; the decoder only needs it increasing
            if let Err(e) = mft.decode(&frame, time, &sink) {
                failures += 1;
                if failures <= 3 {
                    crate::log_line(&format!("h264 decoder: {e}"));
                }
            }
        }
        let _ = mft.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn pack(high: u32, low: u32) -> u64 {
        ((high as u64) << 32) | low as u64
    }

    /// Windows' software H.264 encoder: a few solid-colour NV12 frames in, Annex B out.
    unsafe fn encode(w: u32, h: u32, frames: usize) -> Vec<Vec<u8>> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).unwrap();
            let enc: IMFTransform = CoCreateInstance(&CLSID_MSH264EncoderMFT, None, CLSCTX_INPROC_SERVER).unwrap();
            let out = MFCreateMediaType().unwrap();
            out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).unwrap();
            out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264).unwrap();
            out.SetUINT32(&MF_MT_AVG_BITRATE, 2_000_000).unwrap();
            out.SetUINT64(&MF_MT_FRAME_SIZE, pack(w, h)).unwrap();
            out.SetUINT64(&MF_MT_FRAME_RATE, pack(30, 1)).unwrap();
            out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32).unwrap();
            enc.SetOutputType(0, &out, 0).unwrap();
            let inp = MFCreateMediaType().unwrap();
            inp.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).unwrap();
            inp.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12).unwrap();
            inp.SetUINT64(&MF_MT_FRAME_SIZE, pack(w, h)).unwrap();
            inp.SetUINT64(&MF_MT_FRAME_RATE, pack(30, 1)).unwrap();
            inp.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32).unwrap();
            enc.SetInputType(0, &inp, 0).unwrap();
            enc.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0).unwrap();
            let info = enc.GetOutputStreamInfo(0).unwrap();
            let provides = info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32) != 0;
            // Pure red-ish: Y 81, U 90, V 240.
            let mut nv12 = vec![81u8; (w * h) as usize];
            for _ in 0..(w * h / 4) {
                nv12.extend_from_slice(&[90, 240]);
            }
            let mut out_frames = Vec::new();
            enc.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0).unwrap();
            let collect = |out_frames: &mut Vec<Vec<u8>>| loop {
                let own = (!provides).then(|| {
                    let b = MFCreateMemoryBuffer(info.cbSize.max(1 << 20)).unwrap();
                    let s = MFCreateSample().unwrap();
                    s.AddBuffer(&b).unwrap();
                    s
                });
                let mut buffers = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: ManuallyDrop::new(own), dwStatus: 0, pEvents: ManuallyDrop::new(None) }];
                let mut status = 0;
                let r = enc.ProcessOutput(0, &mut buffers, &mut status);
                let s = ManuallyDrop::take(&mut buffers[0].pSample);
                drop(ManuallyDrop::take(&mut buffers[0].pEvents));
                if let Err(e) = r {
                    if e.code() != MF_E_TRANSFORM_NEED_MORE_INPUT {
                        eprintln!("encoder ProcessOutput: {e}");
                    }
                    break;
                }
                let b = s.unwrap().ConvertToContiguousBuffer().unwrap();
                let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
                b.Lock(&mut p, None, Some(&mut len)).unwrap();
                out_frames.push(std::slice::from_raw_parts(p, len as usize).to_vec());
                b.Unlock().unwrap();
            };
            for i in 0..frames {
                let media = MFCreateMemoryBuffer(nv12.len() as u32).unwrap();
                let mut ptr = std::ptr::null_mut();
                media.Lock(&mut ptr, None, None).unwrap();
                std::ptr::copy_nonoverlapping(nv12.as_ptr(), ptr, nv12.len());
                media.Unlock().unwrap();
                media.SetCurrentLength(nv12.len() as u32).unwrap();
                let sample = MFCreateSample().unwrap();
                sample.AddBuffer(&media).unwrap();
                sample.SetSampleTime(i as i64 * 333_333).unwrap();
                sample.SetSampleDuration(333_333).unwrap();
                if let Err(e) = enc.ProcessInput(0, &sample, 0) {
                    eprintln!("encoder ProcessInput: {e}");
                }
                collect(&mut out_frames);
            }
            // The encoder holds frames back for lookahead until asked to drain.
            enc.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0).unwrap();
            enc.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0).unwrap();
            collect(&mut out_frames);
            eprintln!("encoded {} frames", out_frames.len());
            out_frames
        }
    }

    #[test]
    fn decodes_1080p_with_padded_rows() {
        let frames = unsafe { encode(1920, 1080, 30) };
        assert!(!frames.is_empty(), "encoder produced nothing");
        let got: Arc<Mutex<Vec<Pixels>>> = Arc::default();
        let sink_got = got.clone();
        let mut decoder = Decoder::start(Box::new(move |p| sink_got.lock().unwrap().push(p)));
        for (i, f) in frames.into_iter().enumerate() {
            decoder.push(f, i == 0);
        }
        drop(decoder);
        std::thread::sleep(std::time::Duration::from_secs(2));
        let got = got.lock().unwrap();
        assert!(!got.is_empty(), "no pictures decoded");
        let p = &got[got.len() - 1];
        assert_eq!((p.width(), p.height()), (1920, 1080));
        // Centre and bottom-right pixels are the encoded red, not padding garbage.
        for (x, y) in [(960u32, 540u32), (1919, 1079)] {
            let px = p.as_slice()[(y * 1920 + x) as usize];
            assert!(px.r > 200 && px.g < 60 && px.b < 60, "pixel at {x},{y} is {px:?}");
        }
    }
}
