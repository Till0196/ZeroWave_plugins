// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 文書の文字。EUC-JP に放送の外字（85〜94 区）を足したもの
//! （STD-B24 第二分冊 第二編 付属 1 13.2.1）。

/// 文書の文字の列を読む。読めない並びは U+FFFD。C プロファイルは Shift_JIS（TR-B14 第三分冊）。
pub fn decode(bytes: &[u8]) -> String {
    if crate::runtime::profile::is_c() {
        let (text, _, _) = encoding_rs::SHIFT_JIS.decode(bytes);
        return text.into_owned();
    }
    decode_euc(bytes)
}

pub fn decode_euc(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let first = bytes[at];
        match first {
            0x00..=0x7f => {
                out.push(char::from(first));
                at += 1;
            }
            0xa1..=0xfe => {
                let Some(&second) = bytes.get(at + 1) else {
                    out.push('\u{fffd}');
                    break;
                };
                if !(0xa1..=0xfe).contains(&second) {
                    out.push('\u{fffd}');
                    at += if second < 0x80 { 1 } else { 2 };
                    continue;
                }
                out.push(wide(first & 0x7f, second & 0x7f).unwrap_or('\u{fffd}'));
                at += 2;
            }
            0x8e => {
                out.push('\u{fffd}');
                at += 2;
            }
            0x8f => {
                out.push('\u{fffd}');
                at += 3;
            }
            _ => {
                out.push('\u{fffd}');
                at += 1;
            }
        }
    }
    out
}

/// 区点の文字。テーブルに無い 87〜88 区は外字（[`crate::media::drcs`]）の領域。
fn wide(first: u8, second: u8) -> Option<char> {
    isdb::arib::wide(first, second).or_else(|| crate::media::drcs::char_of(first, second))
}

pub fn encode(text: &str) -> Vec<u8> {
    let coding = if crate::runtime::profile::is_c() {
        encoding_rs::SHIFT_JIS
    } else {
        encoding_rs::EUC_JP
    };
    let (bytes, _, _) = coding.encode(text);
    bytes.into_owned()
}

/// 一文字の EUC-JP の符号（STD-B24 第二分冊 (2/2) 付属 2 5.4.3.4 の `charCodeAt`）。
pub fn euc_code(letter: char) -> Option<u32> {
    if letter.is_ascii() {
        return Some(u32::from(letter));
    }
    if let Some((row, cell)) = crate::media::drcs::cell_of(letter) {
        return Some(u32::from(row | 0x80) << 8 | u32::from(cell | 0x80));
    }
    static INVERSE: std::sync::LazyLock<std::collections::HashMap<char, u32>> =
        std::sync::LazyLock::new(|| {
            let mut map = std::collections::HashMap::new();
            for first in 0x21u8..=0x7e {
                for second in 0x21u8..=0x7e {
                    if let Some(c) = isdb::arib::wide(first, second) {
                        map.entry(c)
                            .or_insert(u32::from(first | 0x80) << 8 | u32::from(second | 0x80));
                    }
                }
            }
            map
        });
    INVERSE.get(&letter).copied()
}

pub fn from_euc_code(code: u32) -> Option<char> {
    if code < 0x80 {
        return char::from_u32(code);
    }
    let first = (code >> 8) as u8;
    let second = code as u8;
    if (0xa1..=0xfe).contains(&first) && (0xa1..=0xfe).contains(&second) {
        return wide(first & 0x7f, second & 0x7f);
    }
    None
}

pub fn is_wide(letter: char) -> bool {
    !matches!(letter, '\u{0}'..='\u{7f}' | '\u{ff61}'..='\u{ff9f}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kanji_and_kana_and_ascii_are_read() {
        let bytes = [0xb1, 0xd2, 0xc0, 0xb1, b'A'];
        assert_eq!(decode(&bytes), "衛星A");
    }

    #[test]
    fn the_rows_the_broadcast_added_are_read() {
        let text = decode(&[0xfa, 0xa1]);
        assert_ne!(text, "\u{fffd}");
    }

    #[test]
    fn a_broken_pair_is_one_replacement_and_the_next_byte_stands() {
        assert_eq!(decode(&[0xb1, b'A']), "\u{fffd}A");
        assert_eq!(decode(&[0xb1]), "\u{fffd}");
    }

    #[test]
    fn width_is_by_the_grid() {
        assert!(is_wide('衛'));
        assert!(!is_wide('A'));
        assert!(!is_wide('ｱ'));
    }

    #[test]
    fn character_codes_are_euc_jp() {
        assert_eq!(euc_code('あ'), Some(0xa4a2));
        assert_eq!(euc_code('A'), Some(0x41));
        assert_eq!(from_euc_code(0xa4a2), Some('あ'));
        assert_eq!(from_euc_code(0x41), Some('A'));
        let gaiji = decode(&[0xfa, 0xa1]);
        let c = gaiji.chars().next().unwrap();
        assert_eq!(euc_code(c), Some(0xfaa1));
    }
}
