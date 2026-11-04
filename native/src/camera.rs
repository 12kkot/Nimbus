//! The camera, through Media Foundation: the list of cameras for the settings
//! (read without switching any of them on) and capture of one as NV12 frames,
//! for the hardware encoder (hwenc.rs) and our own small, mirrored picture.
//!
//! Capture runs on a thread of its own with a synchronous source reader; the
//! reader converts whatever the camera gives (YUY2, MJPG…) to NV12. Dropping
//! the `Capture` stops the thread, which shuts the camera down.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::core::{Interface, GUID, HSTRING, PWSTR};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED};

/// A camera as the settings list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// Windows' symbolic link of the device: stable between launches.
    pub id: String,
    pub name: String,
}

/// One captured frame.
pub struct Frame {
    /// NV12 without padding: `width`×`height` luma, then interleaved chroma.
    pub nv12: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    /// Since the capture started.
    pub time_us: i64,
}

/// What the camera is asked for, and what it was set to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

/// Discord's camera without Nitro: 720p, 30 frames a second.
pub const WANTED: Mode = Mode { width: 1280, height: 720, fps: 30 };

/// Where frames go (called on the capture thread).
pub type OnFrame = Box<dyn FnMut(Frame) + Send>;

fn err(context: &str, e: windows::core::Error) -> String {
    format!("{context}: {e}")
}

/// The cameras Windows knows. Only reads their names: none is switched on.
pub fn devices() -> Vec<Device> {
    // SAFETY: Media Foundation calls on this thread with COM initialised; the
    // activation array is freed below and its objects released by `take`.
    let listed = unsafe { list_devices() };
    listed.unwrap_or_else(|e| {
        crate::log_line(&format!("camera list: {e}"));
        Vec::new()
    })
}

unsafe fn list_devices() -> Result<Vec<Device>, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| err("MFStartup", e))?;
        let result = (|| {
            let attributes = video_attributes(None)?;
            let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
            let mut count = 0u32;
            MFEnumDeviceSources(&attributes, &mut list, &mut count).map_err(|e| err("MFEnumDeviceSources", e))?;
            if list.is_null() {
                return Ok(Vec::new());
            }
            let activates = std::slice::from_raw_parts_mut(list, count as usize);
            let mut out = Vec::new();
            for slot in activates.iter_mut() {
                if let Some(activate) = slot.take() {
                    let id = string_of(&activate, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK);
                    let name = string_of(&activate, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME);
                    if let Some(id) = id {
                        out.push(Device { name: name.unwrap_or_else(|| id.clone()), id });
                    }
                }
            }
            CoTaskMemFree(Some(list as *const _));
            Ok(out)
        })();
        let _ = MFShutdown();
        result
    }
}

/// Attributes that select video capture devices (and, with `id`, that one).
unsafe fn video_attributes(id: Option<&str>) -> Result<IMFAttributes, String> {
    unsafe {
        let mut attributes: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attributes, 2).map_err(|e| err("MFCreateAttributes", e))?;
        let attributes = attributes.ok_or("no attributes")?;
        attributes
            .SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)
            .map_err(|e| err("source type", e))?;
        if let Some(id) = id {
            attributes
                .SetString(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, &HSTRING::from(id))
                .map_err(|e| err("symbolic link", e))?;
        }
        Ok(attributes)
    }
}

unsafe fn string_of(activate: &IMFActivate, key: &GUID) -> Option<String> {
    unsafe {
        let mut text = PWSTR::null();
        let mut len = 0u32;
        activate.GetAllocatedString(key, &mut text, &mut len).ok()?;
        let value = text.to_string().ok();
        CoTaskMemFree(Some(text.0 as *const _));
        value
    }
}

/// A running capture; dropping it switches the camera off.
pub struct Capture {
    stop: Arc<AtomicBool>,
    /// Closed when the capture thread has let the camera go.
    done: std::sync::mpsc::Receiver<()>,
    pub mode: Mode,
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // The thread wakes with the next frame (a few tens of ms) and lets the camera go; a camera
        // that hangs is not waited for longer than this.
        let _ = self.done.recv_timeout(std::time::Duration::from_millis(600));
    }
}

/// Switches on the camera `id` (Windows' default first camera if None) at the mode
/// closest to `want`, and hands every frame to `on_frame` until dropped.
pub fn start(id: Option<String>, want: Mode, on_frame: OnFrame) -> Result<Capture, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<Mode, String>>();
    let (done_tx, done) = std::sync::mpsc::channel::<()>();
    let thread_stop = stop.clone();
    std::thread::Builder::new()
        .name("camera".into())
        .spawn(move || {
            // SAFETY: COM and Media Foundation are set up and torn down on this thread,
            // and every interface used stays on it.
            let result = unsafe { run(id, want, on_frame, &thread_stop, &ready_tx) };
            if let Err(e) = result {
                crate::log_line(&format!("camera stopped: {e}"));
                let _ = ready_tx.send(Err(e));
            }
            drop(done_tx);
        })
        .map_err(|e| e.to_string())?;
    match ready_rx.recv() {
        Ok(Ok(mode)) => Ok(Capture { stop, done, mode }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(crate::i18n::t("камера не запустилась")),
    }
}

unsafe fn run(id: Option<String>, want: Mode, mut on_frame: OnFrame, stop: &AtomicBool, ready: &std::sync::mpsc::Sender<Result<Mode, String>>) -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| err("MFStartup", e))?;
        let source = open_source(id.as_deref())?;
        let result = capture(&source, want, &mut on_frame, stop, ready);
        // Lets the camera go (its light goes off) whatever happened.
        let _ = source.Shutdown();
        let _ = MFShutdown();
        result
    }
}

/// The camera's media source: the one with this id, or the first one.
unsafe fn open_source(id: Option<&str>) -> Result<IMFMediaSource, String> {
    unsafe {
        let id = match id {
            Some(id) => id.to_string(),
            None => list_devices()?.into_iter().next().map(|d| d.id).ok_or_else(|| crate::i18n::t("камера не найдена"))?,
        };
        let attributes = video_attributes(Some(&id))?;
        MFCreateDeviceSource(&attributes).map_err(|e| err("MFCreateDeviceSource", e))
    }
}

const STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

unsafe fn capture(source: &IMFMediaSource, want: Mode, on_frame: &mut OnFrame, stop: &AtomicBool, ready: &std::sync::mpsc::Sender<Result<Mode, String>>) -> Result<(), String> {
    unsafe {
        let mut attributes: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attributes, 2).map_err(|e| err("MFCreateAttributes", e))?;
        let attributes = attributes.ok_or("no attributes")?;
        // The reader converts (and decodes MJPG) to NV12 itself.
        let _ = attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1);
        let _ = attributes.SetUINT32(&MF_LOW_LATENCY, 1);
        let reader = MFCreateSourceReaderFromMediaSource(source, &attributes).map_err(|e| err("MFCreateSourceReaderFromMediaSource", e))?;

        let mut natives = Vec::new();
        let mut modes = Vec::new();
        for index in 0..256 {
            let Ok(native) = reader.GetNativeMediaType(STREAM, index) else { break };
            let (width, height) = unpack(native.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0));
            let (num, den) = unpack(native.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or(0));
            modes.push(Mode { width, height, fps: if den == 0 { 0 } else { num / den } });
            natives.push(native);
        }
        let picked = pick_mode(&modes, want).ok_or_else(|| crate::i18n::t("камера не даёт подходящего видео"))?;
        let mode = Mode { fps: modes[picked].fps.clamp(1, want.fps), ..modes[picked] };
        reader.SetCurrentMediaType(STREAM, None, &natives[picked]).map_err(|e| err("SetCurrentMediaType native", e))?;
        let nv12 = MFCreateMediaType().map_err(|e| err("MFCreateMediaType", e))?;
        nv12.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(|e| err("major", e))?;
        nv12.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12).map_err(|e| err("subtype", e))?;
        nv12.SetUINT64(&MF_MT_FRAME_SIZE, pack(mode.width, mode.height)).map_err(|e| err("size", e))?;
        reader.SetCurrentMediaType(STREAM, None, &nv12).map_err(|e| err("SetCurrentMediaType NV12", e))?;
        crate::log_line(&format!("camera: {}x{} @{} ({} modes offered)", mode.width, mode.height, modes[picked].fps, modes.len()));
        let _ = ready.send(Ok(mode));

        let started = std::time::Instant::now();
        let (w, h) = (mode.width & !1, mode.height & !1);
        while !stop.load(Ordering::Relaxed) {
            let mut flags = 0u32;
            let mut sample: Option<IMFSample> = None;
            reader.ReadSample(STREAM, 0, None, Some(&mut flags), None, Some(&mut sample)).map_err(|e| err("ReadSample", e))?;
            if flags & (MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 | MF_SOURCE_READERF_ERROR.0 as u32) != 0 {
                return Err(crate::i18n::t("камера отключилась"));
            }
            let Some(sample) = sample else { continue }; // a gap in the stream
            if let Some(nv12) = read_nv12(&sample, w, h) {
                on_frame(Frame { nv12: Arc::new(nv12), width: w, height: h, time_us: started.elapsed().as_micros() as i64 });
            }
        }
        Ok(())
    }
}

/// The sample's picture as tight NV12, whatever the row pitch of its buffer.
unsafe fn read_nv12(sample: &IMFSample, w: u32, h: u32) -> Option<Vec<u8>> {
    unsafe {
        let buffer = sample.GetBufferByIndex(0).ok()?;
        if let Ok(two_d) = buffer.cast::<IMF2DBuffer>() {
            let mut scan0: *mut u8 = std::ptr::null_mut();
            let mut pitch = 0i32;
            if two_d.Lock2D(&mut scan0, &mut pitch).is_ok() {
                let pitch = pitch.unsigned_abs() as usize;
                let len = pitch * (h as usize * 3 / 2);
                let packed = pack_nv12(std::slice::from_raw_parts(scan0, len), pitch, w, h);
                let _ = two_d.Unlock2D();
                return packed;
            }
        }
        let mut ptr: *mut u8 = std::ptr::null_mut();
        let mut len = 0u32;
        buffer.Lock(&mut ptr, None, Some(&mut len)).ok()?;
        let packed = pack_nv12(std::slice::from_raw_parts(ptr, len as usize), w as usize, w, h);
        let _ = buffer.Unlock();
        packed
    }
}

fn pack(high: u32, low: u32) -> u64 {
    ((high as u64) << 32) | low as u64
}

fn unpack(value: u64) -> (u32, u32) {
    ((value >> 32) as u32, value as u32)
}

// ---------------------------------------------------------------- pure parts

/// Of the camera's own modes, the one nearest `want`: the biggest picture no taller
/// than wanted at a usable rate (15 frames a second or more), then the rate nearest
/// the wanted one. A camera whose every mode is taller gives its smallest.
pub fn pick_mode(modes: &[Mode], want: Mode) -> Option<usize> {
    let usable = |m: &Mode| m.width >= 2 && m.height >= 2;
    let score = |m: &Mode| {
        let fits = m.height <= want.height && m.width <= want.width;
        let smooth = m.fps >= 15;
        // Rates up to the wanted one count fully; faster ones are no better.
        (fits && smooth, if fits { m.height } else { u32::MAX - m.height }, m.fps.min(want.fps), u32::MAX - m.fps)
    };
    (0..modes.len()).filter(|&i| usable(&modes[i])).max_by_key(|&i| score(&modes[i]))
}

/// NV12 with `pitch` bytes a row (for both planes) as tight `w`×`h` NV12.
pub fn pack_nv12(src: &[u8], pitch: usize, w: u32, h: u32) -> Option<Vec<u8>> {
    let (w, h) = (w as usize, h as usize);
    if pitch < w || src.len() < pitch * (h + h / 2) - (pitch - w) {
        return None;
    }
    let mut out = Vec::with_capacity(w * h * 3 / 2);
    for row in 0..h + h / 2 {
        out.extend_from_slice(&src[row * pitch..row * pitch + w]);
    }
    Some(out)
}

/// The size of a small picture of a `w`×`h` frame, at most `max_w` wide (even sizes).
pub fn preview_size(w: u32, h: u32, max_w: u32) -> (u32, u32) {
    if w <= max_w {
        return ((w & !1).max(2), (h & !1).max(2));
    }
    (max_w & !1, ((h as u64 * max_w as u64 / w as u64) as u32 & !1).max(2))
}

/// RGBA (R, G, B, A in memory, as Slint holds it) of an NV12 frame, scaled to
/// `out_w`×`out_h` by picking the nearest pixel, mirrored left to right when asked
/// (our own camera, as in a mirror). BT.601 colours, studio range, as cameras give.
pub fn nv12_to_rgba(nv12: &[u8], w: u32, h: u32, out_w: u32, out_h: u32, mirror: bool) -> Vec<u8> {
    let (w, h, ow, oh) = (w as usize, h as usize, out_w as usize, out_h as usize);
    let mut out = vec![0u8; ow * oh * 4];
    if nv12.len() < w * h * 3 / 2 || ow == 0 || oh == 0 {
        return out;
    }
    let (luma, chroma) = nv12.split_at(w * h);
    for oy in 0..oh {
        let sy = oy * h / oh;
        for ox in 0..ow {
            // The mirror of the scaled picture: column ox shows what column ow-1-ox would.
            let sx = if mirror { (ow - 1 - ox) * w / ow } else { ox * w / ow };
            let y = luma[sy * w + sx] as i32 - 16;
            let at = (sy / 2) * w + (sx & !1);
            let (u, v) = (chroma[at] as i32 - 128, chroma[at + 1] as i32 - 128);
            let px = &mut out[(oy * ow + ox) * 4..][..4];
            px[0] = ((298 * y + 409 * v + 128) >> 8).clamp(0, 255) as u8;
            px[1] = ((298 * y - 100 * u - 208 * v + 128) >> 8).clamp(0, 255) as u8;
            px[2] = ((298 * y + 516 * u + 128) >> 8).clamp(0, 255) as u8;
            px[3] = 255;
        }
    }
    out
}

/// One RGB colour as NV12's (Y, U, V), studio range.
pub fn yuv_of((r, g, b): (u8, u8, u8)) -> (u8, u8, u8) {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    let y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
    let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
    let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
    (y.clamp(0, 255) as u8, u.clamp(0, 255) as u8, v.clamp(0, 255) as u8)
}

/// An invented NV12 picture (no camera): a `back` background darkening downwards, a
/// `mark` block near the left edge (so a mirrored copy shows it on the right) and a
/// light disc in the middle, moved along by `phase`. For the demo and the tests.
pub fn test_frame(w: u32, h: u32, phase: u32, back: (u8, u8, u8), mark: (u8, u8, u8)) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut nv12 = vec![0u8; w * h * 3 / 2];
    let colour_at = |x: usize, y: usize| {
        let (cx, cy, r) = ((w / 2 + phase as usize % (w / 8 + 1)) as i64, (h / 2) as i64, (h / 4) as i64);
        let (dx, dy) = (x as i64 - cx, y as i64 - cy);
        if x >= w / 12 && x < w / 12 + w / 6 && y >= h / 6 && y < h / 6 + h / 3 {
            mark
        } else if dx * dx + dy * dy < r * r {
            (235, 225, 210)
        } else {
            let dim = |c: u8| (c as usize * (2 * h - y) / (2 * h)) as u8;
            (dim(back.0), dim(back.1), dim(back.2))
        }
    };
    for y in 0..h {
        for x in 0..w {
            nv12[y * w + x] = yuv_of(colour_at(x, y)).0;
        }
    }
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            let (_, u, v) = yuv_of(colour_at(x, y));
            let at = w * h + (y / 2) * w + x;
            nv12[at] = u;
            nv12[at + 1] = v;
        }
    }
    nv12
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(width: u32, height: u32, fps: u32) -> Mode {
        Mode { width, height, fps }
    }

    #[test]
    fn picks_720p30_when_offered() {
        let modes = [mode(640, 480, 30), mode(1920, 1080, 30), mode(1280, 720, 10), mode(1280, 720, 30), mode(1280, 720, 60)];
        assert_eq!(pick_mode(&modes, WANTED), Some(3), "720p at exactly 30 beats 10 and 60");
    }

    #[test]
    fn a_smaller_camera_gives_what_it_has() {
        let modes = [mode(320, 240, 30), mode(640, 480, 30), mode(640, 480, 5)];
        assert_eq!(pick_mode(&modes, WANTED), Some(1));
        // Only big modes: the smallest of them.
        assert_eq!(pick_mode(&[mode(3840, 2160, 30), mode(1920, 1080, 30)], WANTED), Some(1));
        // A smooth small mode beats a juddering fitting one.
        assert_eq!(pick_mode(&[mode(1280, 720, 5), mode(960, 540, 30)], WANTED), Some(1));
        assert_eq!(pick_mode(&[], WANTED), None);
    }

    #[test]
    fn pitch_padding_is_dropped() {
        // 4×2 NV12 with rows of 6 bytes: 2 luma rows and 1 chroma row.
        let src = [1, 2, 3, 4, 0, 0, 5, 6, 7, 8, 0, 0, 9, 10, 11, 12];
        assert_eq!(pack_nv12(&src, 6, 4, 2), Some(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]));
        assert_eq!(pack_nv12(&src[..10], 6, 4, 2), None, "too short");
    }

    #[test]
    fn colours_survive_the_round_trip() {
        for colour in [(200, 40, 40), (30, 160, 90), (60, 80, 220), (128, 128, 128)] {
            let (y, u, v) = yuv_of(colour);
            let rgba = nv12_to_rgba(&[y, y, y, y, u, v], 2, 2, 1, 1, false);
            for (got, want) in rgba[..3].iter().zip([colour.0, colour.1, colour.2]) {
                assert!(got.abs_diff(want) <= 3, "{colour:?} came back as {:?}", &rgba[..3]);
            }
        }
    }

    #[test]
    fn our_picture_is_mirrored_and_scaled() {
        let red = (220, 30, 30);
        let frame = test_frame(64, 36, 0, (20, 40, 160), red);
        let (ow, oh) = preview_size(64, 36, 32);
        assert_eq!((ow, oh), (32, 18));
        let is_red = |px: &[u8]| px[0] > 150 && px[1] < 90 && px[2] < 90;
        let plain = nv12_to_rgba(&frame, 64, 36, ow, oh, false);
        let mirrored = nv12_to_rgba(&frame, 64, 36, ow, oh, true);
        assert_eq!(plain.len(), (ow * oh * 4) as usize);
        // The mark sits near the left edge; in the mirror near the right one, row for row.
        let row = 6usize;
        let at = |pixels: &[u8], x: usize| pixels[(row * ow as usize + x) * 4..][..4].to_vec();
        assert!(is_red(&at(&plain, 4)) && !is_red(&at(&plain, 27)));
        assert!(is_red(&at(&mirrored, 27)) && !is_red(&at(&mirrored, 4)));
        for x in 0..ow as usize {
            assert_eq!(at(&plain, x), at(&mirrored, ow as usize - 1 - x));
        }
    }

    #[test]
    fn small_pictures_keep_the_shape() {
        assert_eq!(preview_size(1280, 720, 640), (640, 360));
        assert_eq!(preview_size(640, 480, 640), (640, 480));
        assert_eq!(preview_size(1281, 721, 2000), (1280, 720));
    }
}
