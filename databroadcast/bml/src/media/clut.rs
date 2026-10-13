// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 色のテーブル。BML の色は番号で、テーブルが番号を色にする（STD-B24 第二分冊 (2/2) A3 5.1.7 表 5-7・5-8）。
//! 運用される書式は YCbCr・八ビット・start_end_flag=1 だけ（TR-B14）。

pub type Colour = [u8; 4];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clut(pub [Colour; 256]);

impl Default for Clut {
    fn default() -> Self {
        let mut table = [[0, 0, 0, 255]; 256];
        table[..DEFAULT.len()].copy_from_slice(&DEFAULT);
        Self(table)
    }
}

/// 番号ではなく色そのものを持つことを示すフラグ。下の 24 ビットが RGB。
pub const DIRECT: u32 = 0x8000_0000;
pub const TRANSPARENT: u32 = 0x4000_0000;

pub fn direct(r: u8, g: u8, b: u8) -> u32 {
    DIRECT | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
}

pub fn parse_direct(word: &str) -> Option<u32> {
    let word = word.trim();
    if let Some(hex) = word.strip_prefix('#') {
        let digits: Vec<u8> = hex
            .chars()
            .map(|c| c.to_digit(16).map(|d| d as u8))
            .collect::<Option<_>>()?;
        return match digits.as_slice() {
            [r, g, b] => Some(direct(r * 17, g * 17, b * 17)),
            [r1, r2, g1, g2, b1, b2] => Some(direct(r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2)),
            _ => None,
        };
    }
    Some(match word.to_ascii_lowercase().as_str() {
        "black" => direct(0, 0, 0),
        "silver" => direct(192, 192, 192),
        "gray" | "grey" => direct(128, 128, 128),
        "white" => direct(255, 255, 255),
        "maroon" => direct(128, 0, 0),
        "red" => direct(255, 0, 0),
        "purple" => direct(128, 0, 128),
        "fuchsia" => direct(255, 0, 255),
        "green" => direct(0, 128, 0),
        "lime" => direct(0, 255, 0),
        "olive" => direct(128, 128, 0),
        "yellow" => direct(255, 255, 0),
        "navy" => direct(0, 0, 128),
        "blue" => direct(0, 0, 255),
        "teal" => direct(0, 128, 128),
        "aqua" => direct(0, 255, 255),
        "transparent" => TRANSPARENT,
        _ => return None,
    })
}

impl Clut {
    pub fn colour(&self, index: u32) -> Colour {
        if index & DIRECT != 0 {
            return [(index >> 16) as u8, (index >> 8) as u8, index as u8, 255];
        }
        if index == TRANSPARENT {
            return [0, 0, 0, 0];
        }
        self.0
            .get(index as usize)
            .copied()
            .unwrap_or([0, 0, 0, 255])
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let mut table = Self::default();
        let first = *bytes.first()?;
        let ycbcr = first & 0x80 == 0;
        let depth = (first >> 5) & 3;
        let region = first & 0x10 != 0;
        let start_end = first & 0x08 != 0;
        let mut at = 1;
        if region {
            at += 8;
        }
        if !start_end {
            return None;
        }
        let (start, end) = match depth {
            0 => {
                let byte = *bytes.get(at)?;
                at += 1;
                (usize::from(byte >> 4), usize::from(byte & 15))
            }
            1 => {
                let pair = bytes.get(at..at + 2)?;
                at += 2;
                (usize::from(pair[0]), usize::from(pair[1]))
            }
            2 => {
                let quad = bytes.get(at..at + 4)?;
                at += 4;
                (
                    usize::from(u16::from_be_bytes([quad[0], quad[1]])),
                    usize::from(u16::from_be_bytes([quad[2], quad[3]])),
                )
            }
            _ => return None,
        };
        for index in start..=end.min(255) {
            let entry = bytes.get(at..at + 4)?;
            at += 4;
            let (r, g, b) = if ycbcr {
                let y = f32::from(entry[0]) - 16.0;
                let cb = f32::from(entry[1]) - 128.0;
                let cr = f32::from(entry[2]) - 128.0;
                (
                    (y * 1.16438 + 1.79274 * cr).floor().clamp(0.0, 255.0) as u8,
                    (y * 1.16438 - 0.213249 * cb - 0.532909 * cr)
                        .floor()
                        .clamp(0.0, 255.0) as u8,
                    (y * 1.16438 + 2.1124 * cb).floor().clamp(0.0, 255.0) as u8,
                )
            } else {
                (entry[0], entry[1], entry[2])
            };
            table.0[index] = [r, g, b, entry[3]];
        }
        Some(table)
    }
}

/// 既定のテーブル（表 5-7）。局ロゴも同じテーブルでデコードするので isdb が持つ。
const DEFAULT: [Colour; 128] = isdb::logo::DEFAULT_CLUT;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_table_has_the_eight_primaries_first() {
        let clut = Clut::default();
        assert_eq!(clut.colour(7), [255, 255, 255, 255]);
        assert_eq!(clut.colour(8), [0, 0, 0, 0]);
        assert_eq!(clut.colour(15), [170, 170, 170, 255]);
        assert_eq!(clut.colour(200), [0, 0, 0, 255]);
    }

    #[test]
    fn a_table_file_overlays_the_default() {
        let bytes = [0x28, 17, 17, 235, 128, 128, 255];
        let clut = Clut::parse(&bytes).expect("readable");
        assert_eq!(clut.colour(17), [254, 254, 254, 255]);
        assert_eq!(clut.colour(7), [255, 255, 255, 255]);
    }
}
