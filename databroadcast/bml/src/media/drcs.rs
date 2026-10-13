// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 外字（DRCS、STD-B24 第二分冊 (2/2) 付属 2 4.1 と付属 3 4.6.10）。
//! 漢字集合の 87〜88 区に置かれ、フォントと大きさごとの画像が来る。運用は mode 1・depth 2 だけ。

/// 87 区の第一の点を割り当てる先。U+EC00〜U+ECBB。
pub const FIRST_CHAR: u32 = 0xec00;
pub const FIRST_ROW: u8 = 0x77;
pub const LAST_ROW: u8 = 0x78;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glyph {
    pub row: u8,
    pub cell: u8,
    /// フォント: 1 丸ゴシック、2 角ゴシック、3 太丸ゴシック。
    pub font: u8,
    pub width: u8,
    pub height: u8,
    pub levels: u8,
    /// 画素の階調、一行ずつ。0 は透明、levels−1 が文字の色。
    pub bitmap: Vec<u8>,
}

pub fn char_of(row: u8, cell: u8) -> Option<char> {
    if !(FIRST_ROW..=LAST_ROW).contains(&row) || !(0x21..=0x7e).contains(&cell) {
        return None;
    }
    char::from_u32(FIRST_CHAR + u32::from(row - FIRST_ROW) * 94 + u32::from(cell - 0x21))
}

pub fn cell_of(letter: char) -> Option<(u8, u8)> {
    let index = u32::from(letter).checked_sub(FIRST_CHAR)?;
    let rows = u32::from(LAST_ROW - FIRST_ROW) + 1;
    if index >= rows * 94 {
        return None;
    }
    Some((FIRST_ROW + (index / 94) as u8, 0x21 + (index % 94) as u8))
}

pub fn parse(bytes: &[u8]) -> Vec<Glyph> {
    let mut out = Vec::new();
    let mut at = 0usize;
    let Some(&count) = bytes.first() else {
        return out;
    };
    at += 1;
    for _ in 0..count {
        let Some(&[row, cell, fonts]) = bytes.get(at..at + 3) else {
            break;
        };
        at += 3;
        for _ in 0..fonts {
            let Some(&[mode, depth, width, height]) = bytes.get(at..at + 4) else {
                return out;
            };
            at += 4;
            let font = mode >> 4;
            if mode & 0x0f > 1 {
                // 幾何は運用されず読み方も分からないので、ここまで。
                return out;
            }
            let levels = u16::from(depth) + 2;
            let bits = (16 - (levels - 1).leading_zeros()) as usize;
            let pixels = usize::from(width) * usize::from(height);
            let length = (pixels * bits).div_ceil(8);
            let Some(data) = bytes.get(at..at + length) else {
                return out;
            };
            at += length;
            let mut bitmap = Vec::with_capacity(pixels);
            let mut bit = 0usize;
            for _ in 0..pixels {
                let mut value = 0u8;
                for _ in 0..bits {
                    let byte = data[bit >> 3];
                    value = value << 1 | (byte >> (7 - (bit & 7))) & 1;
                    bit += 1;
                }
                bitmap.push(value);
            }
            out.push(Glyph {
                row,
                cell,
                font,
                width,
                height,
                levels: levels as u8,
                bitmap,
            });
        }
    }
    out
}

/// 文字に合う画像を選ぶ。大きさは一番近いもの、フォントは同じものを先に。
pub fn pick(glyphs: &[Glyph], letter: char, size: f32, font: u8) -> Option<&Glyph> {
    let (row, cell) = cell_of(letter)?;
    glyphs
        .iter()
        .filter(|g| g.row == row && g.cell == cell && g.width > 0 && g.height > 0)
        .min_by_key(|g| {
            let distance = (f32::from(g.width) - size).abs() as u32;
            (distance, u32::from(g.font != font))
        })
}

pub fn png(glyph: &Glyph, colour: [u8; 4]) -> Vec<u8> {
    let width = usize::from(glyph.width);
    let height = usize::from(glyph.height);
    let top = f32::from(glyph.levels.max(2) - 1);
    let mut raw = Vec::with_capacity(height * (1 + width * 4));
    for row in glyph.bitmap.chunks(width.max(1)).take(height) {
        raw.push(0); // フィルタ無し
        for &level in row {
            let alpha = (f32::from(colour[3]) * f32::from(level) / top).round() as u8;
            raw.extend_from_slice(&[colour[0], colour[1], colour[2], alpha]);
        }
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 ビット RGBA
    crate::media::mng::push_chunk(&mut out, b"IHDR", &ihdr);
    crate::media::mng::push_chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    crate::media::mng::push_chunk(&mut out, b"IEND", &[]);
    out
}

fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut blocks = data.chunks(65_535).peekable();
    if blocks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(block) = blocks.next() {
        out.push(u8::from(blocks.peek().is_none()));
        let length = block.len() as u16;
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&(!length).to_le_bytes());
        out.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    out.extend_from_slice(&(b << 16 | a).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_glyph_is_read_and_mapped() {
        let bytes = [1, 0x77, 0x21, 1, 0x11, 2, 2, 2, 0b0001_1011];
        let glyphs = parse(&bytes);
        assert_eq!(glyphs.len(), 1);
        assert_eq!(glyphs[0].bitmap, [0, 1, 2, 3]);
        assert_eq!(glyphs[0].levels, 4);
        let letter = char_of(0x77, 0x21).unwrap();
        assert_eq!(letter, '\u{ec00}');
        assert_eq!(cell_of(letter), Some((0x77, 0x21)));
        assert_eq!(char_of(0x78, 0x7e), Some('\u{ecbb}'));
        assert_eq!(cell_of('\u{ecbb}'), Some((0x78, 0x7e)));
        assert!(cell_of('\u{ecbc}').is_none());
        let picked = pick(&glyphs, letter, 20.0, 2).unwrap();
        assert_eq!(picked.font, 1);
        let png = png(picked, [255, 0, 0, 255]);
        assert!(png.starts_with(b"\x89PNG"));
        assert_eq!(&png[png.len() - 4..], &[0xae, 0x42, 0x60, 0x82]);
    }
}
