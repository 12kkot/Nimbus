//! Small pictures of screens and windows for the "what to show" picker. Taken with plain GDI
//! (the window's own `PrintWindow`, a copy of the screen for a monitor), kept in memory, and
//! shown only to the person choosing.

use slint::{Rgba8Pixel, SharedPixelBuffer};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, GetMonitorInfoW, ReleaseDC, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC, HMONITOR, MONITORINFO, SRCCOPY,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

use crate::share::Source;

/// A picture fits into this box (the picker's cards are 16:9).
const BOX_W: u32 = 320;
const BOX_H: u32 = 180;
/// PW_RENDERFULLCONTENT: also windows drawn by the GPU.
const FULL_CONTENT: u32 = 2;

pub fn thumbnail(source: Source) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let (bgra, w, h) = unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let grabbed = grab(screen, source);
        ReleaseDC(None, screen);
        grabbed?
    };
    shrink(bgra, w, h)
}

unsafe fn grab(screen: HDC, source: Source) -> Option<(Vec<u8>, u32, u32)> {
    let (rect, window) = match source {
        Source::Window(handle) => {
            let hwnd = HWND(handle as *mut core::ffi::c_void);
            let mut r = RECT::default();
            GetWindowRect(hwnd, &mut r).ok()?;
            (r, Some(hwnd))
        }
        Source::Monitor(handle) => {
            let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !GetMonitorInfoW(HMONITOR(handle as *mut core::ffi::c_void), &mut info).as_bool() {
                return None;
            }
            (info.rcMonitor, None)
        }
    };
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
    if w <= 0 || h <= 0 {
        return None;
    }
    let dc = CreateCompatibleDC(Some(screen));
    let bitmap = CreateCompatibleBitmap(screen, w, h);
    let old = SelectObject(dc, bitmap.into());
    let drawn = match window {
        Some(hwnd) => PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(FULL_CONTENT)).as_bool(),
        None => BitBlt(dc, 0, 0, w, h, Some(screen), rect.left, rect.top, SRCCOPY).is_ok(),
    };
    let mut out = None;
    if drawn {
        let mut info = BITMAPINFO::default();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
        let lines = GetDIBits(dc, bitmap, 0, h as u32, Some(pixels.as_mut_ptr() as *mut core::ffi::c_void), &mut info, DIB_RGB_COLORS);
        if lines != 0 {
            out = Some((pixels, w as u32, h as u32));
        }
    }
    SelectObject(dc, old);
    let _ = DeleteObject(bitmap.into());
    let _ = DeleteDC(dc);
    out
}

/// BGRA at full size into a small RGBA picture.
fn shrink(mut bgra: Vec<u8>, w: u32, h: u32) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
        px[3] = 255;
    }
    let img = image::RgbaImage::from_raw(w, h, bgra)?;
    let small = image::DynamicImage::ImageRgba8(img).resize(BOX_W, BOX_H, image::imageops::FilterType::Triangle).to_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(small.as_raw(), small.width(), small.height()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_big_picture_shrinks_into_the_box_and_keeps_its_shape() {
        let (w, h) = (2560u32, 1440u32);
        let small = shrink(vec![10u8; (w * h * 4) as usize], w, h).expect("shrinks");
        assert!(small.width() <= BOX_W && small.height() <= BOX_H);
        assert_eq!(small.width() * 9, small.height() * 16, "16:9 stays 16:9");
        // BGRA turns into RGBA and nothing is see-through.
        let first = small.as_slice()[0];
        assert_eq!((first.r, first.g, first.b, first.a), (10, 10, 10, 255));
    }
}
