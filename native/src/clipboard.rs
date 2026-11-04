//! Pictures and files on the Windows clipboard, pasted into the message box as attachments.
//! Only read when Ctrl+V is pressed in the message box; text on the clipboard is left to the box itself.

use std::path::PathBuf;

/// The name Discord's own client gives a pasted picture.
pub const PICTURE_NAME: &str = "image.png";
/// Discord takes at most ten files with one message.
pub const MAX_FILES: usize = 10;
/// Pending pictures are drawn up to 200 px wide; thumbnails are made at twice that.
const THUMB_PX: u32 = 400;

// Standard clipboard formats.
const CF_UNICODETEXT: u32 = 13;
const CF_DIB: u32 = 8;
const CF_DIBV5: u32 = 17;
const CF_HDROP: u32 = 15;

/// What was on the clipboard.
#[derive(Debug, Clone, PartialEq)]
pub enum Pasted {
    /// A picture already in PNG (browsers put one there besides the bitmap).
    Png(Vec<u8>),
    /// A device-independent bitmap (screenshots, most programs).
    Dib(Vec<u8>),
    /// A file copied in Explorer.
    File(PathBuf),
}

/// A file waiting in the message box until the message is sent.
#[derive(Clone)]
pub struct Pending {
    pub name: String,
    pub bytes: Vec<u8>,
    /// A small copy of a picture, for its card.
    pub thumb: Option<image::RgbaImage>,
}

fn png_format() -> u32 {
    // SAFETY: registers (or looks up) a named clipboard format; no pointers are kept.
    unsafe { windows::Win32::System::DataExchange::RegisterClipboardFormatW(windows::core::w!("PNG")) }
}

fn available(format: u32) -> bool {
    // SAFETY: only asks whether a format is there; the clipboard isn't opened.
    format != 0 && unsafe { windows::Win32::System::DataExchange::IsClipboardFormatAvailable(format) }.is_ok()
}

/// Whether Ctrl+V should attach rather than type: files copied in Explorer, or a picture with no text
/// beside it (a copied table in Office carries both: that stays text).
pub fn has_attachments() -> bool {
    available(CF_HDROP) || (!available(CF_UNICODETEXT) && (available(png_format()) || available(CF_DIBV5) || available(CF_DIB)))
}

/// Holds the clipboard open; closes it when dropped.
struct Open;

impl Open {
    fn new() -> Option<Open> {
        use windows::Win32::System::DataExchange::OpenClipboard;
        // Another program may hold it for a moment.
        for _ in 0..5 {
            // SAFETY: opened without an owner window, closed by Drop.
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Some(Open);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        None
    }

    /// The bytes of one format, copied out.
    fn bytes(&self, format: u32) -> Option<Vec<u8>> {
        use windows::Win32::Foundation::HGLOBAL;
        use windows::Win32::System::DataExchange::GetClipboardData;
        use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
        // SAFETY: the clipboard is open; the handle stays valid until it closes, and the memory is only read
        // between GlobalLock and GlobalUnlock, within GlobalSize.
        unsafe {
            let handle = GetClipboardData(format).ok()?;
            let global = HGLOBAL(handle.0);
            let size = GlobalSize(global);
            let ptr = GlobalLock(global) as *const u8;
            if ptr.is_null() || size == 0 {
                return None;
            }
            let out = std::slice::from_raw_parts(ptr, size).to_vec();
            let _ = GlobalUnlock(global);
            Some(out)
        }
    }

    fn files(&self) -> Vec<PathBuf> {
        use windows::Win32::System::DataExchange::GetClipboardData;
        use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
        // SAFETY: the clipboard is open; DragQueryFileW only writes into the buffer it is given.
        unsafe {
            let Ok(handle) = GetClipboardData(CF_HDROP) else { return Vec::new() };
            let drop = HDROP(handle.0);
            let count = DragQueryFileW(drop, u32::MAX, None);
            (0..count.min(MAX_FILES as u32))
                .filter_map(|i| {
                    let len = DragQueryFileW(drop, i, None) as usize;
                    let mut buf = vec![0u16; len + 1];
                    let got = DragQueryFileW(drop, i, Some(&mut buf)) as usize;
                    (got > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..got])))
                })
                .filter(|p| p.is_file())
                .collect()
        }
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: closes the clipboard this value opened.
        let _ = unsafe { windows::Win32::System::DataExchange::CloseClipboard() };
    }
}

/// The files or the picture on the clipboard (files first; a PNG before a bitmap).
pub fn read() -> Vec<Pasted> {
    let Some(open) = Open::new() else { return Vec::new() };
    if available(CF_HDROP) {
        return open.files().into_iter().map(Pasted::File).collect();
    }
    let png = png_format();
    if let Some(bytes) = available(png).then(|| open.bytes(png)).flatten().filter(|b| b.starts_with(b"\x89PNG")) {
        return vec![Pasted::Png(bytes)];
    }
    for format in [CF_DIBV5, CF_DIB] {
        if let Some(bytes) = available(format).then(|| open.bytes(format)).flatten() {
            return vec![Pasted::Dib(bytes)];
        }
    }
    Vec::new()
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A channel of a pixel taken out by its bit mask, scaled to 0..255.
fn channel(px: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let max = mask >> shift;
    (((px & mask) >> shift) * 255 / max) as u8
}

/// A device-independent bitmap (BITMAPINFOHEADER or a later header, then the pixels) as RGBA:
/// 1, 4, 8 bits with a palette, 24 bits, 16 and 32 bits plain or with bit masks; bottom-up or top-down.
pub fn dib_to_rgba(dib: &[u8]) -> Result<image::RgbaImage, String> {
    let bad = || "unreadable bitmap".to_string();
    let header = u32_at(dib, 0).ok_or_else(bad)? as usize;
    let width = u32_at(dib, 4).ok_or_else(bad)? as i32;
    let height = u32_at(dib, 8).ok_or_else(bad)? as i32;
    let bpp = u16_at(dib, 14).ok_or_else(bad)? as u32;
    let compression = u32_at(dib, 16).ok_or_else(bad)?;
    let colors_used = u32_at(dib, 32).ok_or_else(bad)?;
    if header < 40 || width <= 0 || height == 0 || height == i32::MIN {
        return Err(bad());
    }
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    if w.saturating_mul(h) > 100_000_000 {
        return Err("bitmap too big".into());
    }
    const BI_RGB: u32 = 0;
    const BI_BITFIELDS: u32 = 3;
    const BI_ALPHABITFIELDS: u32 = 6;
    // Bit masks: after a plain header, or inside a later one.
    let fields = matches!(compression, BI_BITFIELDS | BI_ALPHABITFIELDS);
    let (masks, extra) = match (fields, header) {
        (true, 40) => {
            let n = if compression == BI_ALPHABITFIELDS { 4 } else { 3 };
            let m: Vec<u32> = (0..n).map(|i| u32_at(dib, 40 + i * 4).unwrap_or(0)).collect();
            ([m[0], m[1], m[2], m.get(3).copied().unwrap_or(0)], n * 4)
        }
        (true, _) => ([u32_at(dib, 40).unwrap_or(0), u32_at(dib, 44).unwrap_or(0), u32_at(dib, 48).unwrap_or(0), if header >= 56 { u32_at(dib, 52).unwrap_or(0) } else { 0 }], 0),
        (false, _) if compression != BI_RGB => return Err("compressed bitmaps are not supported".into()),
        (false, _) if bpp == 16 => ([0x7c00, 0x03e0, 0x001f, 0], 0),
        (false, _) => ([0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0], 0),
    };
    let palette_len = if bpp <= 8 { if colors_used == 0 { 1usize << bpp } else { colors_used as usize } } else { 0 };
    let palette_at = header + extra;
    let pixels_at = palette_at + palette_len * 4;
    let stride = (w * bpp as usize).div_ceil(32) * 4;
    if !matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32) || dib.len() < pixels_at + stride * h {
        return Err(bad());
    }
    let palette = |i: usize| -> [u8; 4] {
        let at = palette_at + i.min(palette_len.saturating_sub(1)) * 4;
        [dib[at + 2], dib[at + 1], dib[at], 255]
    };
    // A plain 32-bit bitmap usually leaves its fourth byte at zero: then it is opaque.
    let plain32 = bpp == 32 && !fields;
    let alpha_used = plain32 && (0..h).any(|y| (0..w).any(|x| dib[pixels_at + y * stride + x * 4 + 3] != 0));
    let mut out = image::RgbaImage::new(w as u32, h as u32);
    for y in 0..h {
        let row = &dib[pixels_at + y * stride..pixels_at + (y + 1) * stride];
        let to = if height > 0 { h - 1 - y } else { y };
        for x in 0..w {
            let px = match bpp {
                1 | 4 | 8 => {
                    let bit = x * bpp as usize;
                    let byte = row[bit / 8];
                    let index = (byte >> (8 - bpp as usize - bit % 8)) & ((1u16 << bpp) - 1) as u8;
                    palette(index as usize)
                }
                24 => [row[x * 3 + 2], row[x * 3 + 1], row[x * 3], 255],
                32 if plain32 => [row[x * 4 + 2], row[x * 4 + 1], row[x * 4], if alpha_used { row[x * 4 + 3] } else { 255 }],
                _ => {
                    let v = if bpp == 16 { u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]) as u32 } else { u32::from_le_bytes(row[x * 4..x * 4 + 4].try_into().unwrap_or_default()) };
                    let a = if masks[3] == 0 { 255 } else { channel(v, masks[3]) };
                    [channel(v, masks[0]), channel(v, masks[1]), channel(v, masks[2]), a]
                }
            };
            out.put_pixel(x as u32, to as u32, image::Rgba(px));
        }
    }
    Ok(out)
}

pub fn png_of(picture: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = std::io::Cursor::new(Vec::new());
    picture.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// A pasted thing made ready to send: a bitmap becomes `image.png`, a file is read. `limit` is the biggest
/// file the account may send; bigger ones are refused with the reason.
pub fn prepare(pasted: Pasted, limit: u64) -> Result<Pending, String> {
    let (name, bytes) = match pasted {
        Pasted::Png(bytes) => (PICTURE_NAME.to_string(), bytes),
        Pasted::Dib(dib) => (PICTURE_NAME.to_string(), png_of(&dib_to_rgba(&dib)?)?),
        Pasted::File(path) => {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
            let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
            if size > limit {
                return Err(too_big(&name, limit));
            }
            (name, std::fs::read(&path).map_err(|e| e.to_string())?)
        }
    };
    if bytes.len() as u64 > limit {
        return Err(too_big(&name, limit));
    }
    let thumb = image::load_from_memory(&bytes).ok().map(|p| if p.width() > THUMB_PX || p.height() > THUMB_PX { p.thumbnail(THUMB_PX, THUMB_PX) } else { p }.to_rgba8());
    Ok(Pending { name, bytes, thumb })
}

fn too_big(name: &str, limit: u64) -> String {
    crate::i18n::tf("Файл «{}» больше {}", &[&name, &crate::human_size(limit)])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2×2 bitmap with a 40-byte header: bottom-up rows of BGR padded to four bytes.
    fn dib24() -> Vec<u8> {
        let mut d = Vec::new();
        d.extend(40u32.to_le_bytes());
        d.extend(2i32.to_le_bytes());
        d.extend(2i32.to_le_bytes());
        d.extend(1u16.to_le_bytes());
        d.extend(24u16.to_le_bytes());
        d.extend([0u8; 24]);
        // Bottom row: blue, green; top row: red, white.
        d.extend([255, 0, 0, 0, 255, 0, 0, 0]);
        d.extend([0, 0, 255, 255, 255, 255, 0, 0]);
        d
    }

    #[test]
    fn a_24_bit_bitmap_becomes_a_png() {
        let rgba = dib_to_rgba(&dib24()).unwrap();
        assert_eq!(rgba.dimensions(), (2, 2));
        assert_eq!(rgba.get_pixel(0, 0).0, [255, 0, 0, 255], "the top row comes last in the bitmap");
        assert_eq!(rgba.get_pixel(1, 0).0, [255, 255, 255, 255]);
        assert_eq!(rgba.get_pixel(0, 1).0, [0, 0, 255, 255]);
        assert_eq!(rgba.get_pixel(1, 1).0, [0, 255, 0, 255]);
        let png = png_of(&rgba).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        let back = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(back, rgba);
        let pending = prepare(Pasted::Dib(dib24()), 10 << 20).unwrap();
        assert_eq!(pending.name, "image.png");
        assert_eq!(pending.thumb.map(|t| t.dimensions()), Some((2, 2)));
    }

    #[test]
    fn a_32_bit_top_down_bitmap_keeps_or_ignores_alpha() {
        let mut d = Vec::new();
        d.extend(40u32.to_le_bytes());
        d.extend(1i32.to_le_bytes());
        d.extend((-2i32).to_le_bytes());
        d.extend(1u16.to_le_bytes());
        d.extend(32u16.to_le_bytes());
        d.extend([0u8; 24]);
        d.extend([10, 20, 30, 0, 40, 50, 60, 0]);
        let rgba = dib_to_rgba(&d).unwrap();
        assert_eq!(rgba.get_pixel(0, 0).0, [30, 20, 10, 255], "zero alpha everywhere means opaque");
        assert_eq!(rgba.get_pixel(0, 1).0, [60, 50, 40, 255]);
        let n = d.len();
        d[n - 1] = 128;
        assert_eq!(dib_to_rgba(&d).unwrap().get_pixel(0, 1).0, [60, 50, 40, 128]);
    }

    #[test]
    fn bit_masks_and_palettes_are_read() {
        // A V5-style header (124 bytes) with masks inside, BI_BITFIELDS, 32 bits.
        let mut d = vec![0u8; 124];
        d[0..4].copy_from_slice(&124u32.to_le_bytes());
        d[4..8].copy_from_slice(&1i32.to_le_bytes());
        d[8..12].copy_from_slice(&1i32.to_le_bytes());
        d[14..16].copy_from_slice(&32u16.to_le_bytes());
        d[16..20].copy_from_slice(&3u32.to_le_bytes());
        d[40..44].copy_from_slice(&0x00ff_0000u32.to_le_bytes());
        d[44..48].copy_from_slice(&0x0000_ff00u32.to_le_bytes());
        d[48..52].copy_from_slice(&0x0000_00ffu32.to_le_bytes());
        d[52..56].copy_from_slice(&0xff00_0000u32.to_le_bytes());
        d.extend([1, 2, 3, 200]);
        assert_eq!(dib_to_rgba(&d).unwrap().get_pixel(0, 0).0, [3, 2, 1, 200]);
        // 1 bit with a two-colour palette: black, yellow; pixels 0,1,1.
        let mut p = vec![0u8; 40];
        p[0..4].copy_from_slice(&40u32.to_le_bytes());
        p[4..8].copy_from_slice(&3i32.to_le_bytes());
        p[8..12].copy_from_slice(&1i32.to_le_bytes());
        p[14..16].copy_from_slice(&1u16.to_le_bytes());
        p.extend([0, 0, 0, 0, 0, 255, 255, 0]);
        p.extend([0b0110_0000, 0, 0, 0]);
        let rgba = dib_to_rgba(&p).unwrap();
        assert_eq!((rgba.get_pixel(0, 0).0, rgba.get_pixel(2, 0).0), ([0, 0, 0, 255], [255, 255, 0, 255]));
    }

    #[test]
    fn broken_bitmaps_and_big_files_are_refused() {
        assert!(dib_to_rgba(&[0u8; 10]).is_err());
        let mut short = dib24();
        short.truncate(50);
        assert!(dib_to_rgba(&short).is_err());
        let mut rle = dib24();
        rle[16] = 1;
        assert!(dib_to_rgba(&rle).is_err());
        let refused = prepare(Pasted::Png(vec![0x89, b'P', b'N', b'G', 0, 0, 0, 0]), 4).err().unwrap();
        assert!(refused.contains("image.png"), "{refused}");
    }
}
