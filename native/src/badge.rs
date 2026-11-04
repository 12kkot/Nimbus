//! The little red circle with a number that sits on the taskbar button: drawn here
//! (a 3×5 pixel font of our own, scaled up), handed to Windows as a PNG icon.

const SIZE: u32 = 32;

/// Each digit as 5 rows of 3 bits, top row first; `+` is the 11th glyph.
const GLYPHS: [[u8; 5]; 11] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], // 0
    [0b010, 0b110, 0b010, 0b010, 0b111], // 1
    [0b111, 0b001, 0b111, 0b100, 0b111], // 2
    [0b111, 0b001, 0b111, 0b001, 0b111], // 3
    [0b101, 0b101, 0b111, 0b001, 0b001], // 4
    [0b111, 0b100, 0b111, 0b001, 0b111], // 5
    [0b111, 0b100, 0b111, 0b101, 0b111], // 6
    [0b111, 0b001, 0b010, 0b010, 0b010], // 7
    [0b111, 0b101, 0b111, 0b101, 0b111], // 8
    [0b111, 0b101, 0b111, 0b001, 0b111], // 9
    [0b000, 0b010, 0b111, 0b010, 0b000], // +
];

/// What the badge says: the number, or "9+" from ten up.
pub fn label(count: u32) -> Vec<usize> {
    if count > 9 {
        vec![9, 10]
    } else {
        vec![count as usize]
    }
}

/// The badge as RGBA pixels, `SIZE`×`SIZE`, transparent around a red circle.
pub fn pixels(count: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    // A smaller circle than the whole picture, so the badge doesn't swallow the taskbar icon.
    let c = SIZE as f32 / 2.0;
    let r = c * 0.72;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let cover = (r - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            let i = ((y * SIZE + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&[0xf2, 0x3f, 0x43, (cover * 255.0) as u8]);
        }
    }
    let glyphs = label(count);
    // One digit is drawn big, two a bit smaller.
    let scale: u32 = if glyphs.len() == 1 { 4 } else { 3 };
    let gap = if glyphs.len() == 1 { 0 } else { 1 };
    let width = glyphs.len() as u32 * 3 * scale + (glyphs.len() as u32 - 1) * gap;
    let (x0, y0) = ((SIZE - width) / 2, (SIZE - 5 * scale) / 2);
    for (n, &g) in glyphs.iter().enumerate() {
        let gx = x0 + n as u32 * (3 * scale + gap);
        for (row, bits) in GLYPHS[g].iter().enumerate() {
            for col in 0..3u32 {
                if bits >> (2 - col) & 1 == 1 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let (x, y) = (gx + col * scale + dx, y0 + row as u32 * scale + dy);
                            let i = ((y * SIZE + x) * 4) as usize;
                            rgba[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
                        }
                    }
                }
            }
        }
    }
    rgba
}

/// The badge as a PNG file's bytes.
pub fn png(count: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut out);
    image::ImageEncoder::write_image(encoder, &pixels(count), SIZE, SIZE, image::ExtendedColorType::Rgba8).expect("a PNG in memory");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * SIZE + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    #[test]
    fn red_circle_with_white_digits() {
        let one = pixels(3);
        assert_eq!(at(&one, 0, 0)[3], 0, "the corner is clear");
        assert_eq!(at(&one, 7, 16)[..3], [0xf2, 0x3f, 0x43], "red near the edge");
        assert_eq!(at(&one, 2, 16)[3], 0, "and clear outside the smaller circle");
        assert!(
            one.chunks(4).filter(|p| p[..3] == [255, 255, 255] && p[3] == 255).count() > 30,
            "the digit is drawn"
        );
    }

    #[test]
    fn ten_and_more_say_nine_plus() {
        assert_eq!(label(3), vec![3]);
        assert_eq!(label(10), vec![9, 10]);
        assert_eq!(label(250), vec![9, 10]);
        assert_ne!(pixels(10), pixels(5));
    }

    #[test]
    fn png_decodes_back() {
        let bytes = png(7);
        let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (SIZE, SIZE));
        assert_eq!(img.as_raw().as_slice(), pixels(7).as_slice());
    }
}
