// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! バイナリテーブル（`.btb`、STD-B24 第二分冊 (2/2) 付属 2 7.2）。構造の文字列の先頭は行の長さの
//! 欄のバイト数、続きは列ごとに型（B/U/I/S/Z/P）・長さ・単位（B/b/V）。

use crate::media::text;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Boolean,
    Unsigned,
    Integer,
    Text,
    Zip,
    Pad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Byte,
    Bit,
    Variable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub kind: Kind,
    pub length: usize,
    pub unit: Unit,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Boolean(bool),
    Number(i64),
    Text(String),
    Zip(Vec<u8>),
}

impl Cell {
    pub fn number(&self) -> f64 {
        match self {
            Self::Boolean(value) => f64::from(u8::from(*value)),
            Self::Number(value) => *value as f64,
            Self::Text(text) => text.trim().parse().unwrap_or(f64::NAN),
            Self::Zip(_) => f64::NAN,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub fields: Vec<Field>,
    pub rows: Vec<Vec<Cell>>,
}

pub fn structure(text: &str) -> Option<(usize, Vec<Field>)> {
    let mut pieces = text.split(',');
    let length_bytes: usize = pieces.next()?.trim().parse().ok()?;
    let mut fields = Vec::new();
    for piece in pieces {
        let piece = piece.trim();
        let (kind, rest) = piece.split_at_checked(1)?;
        let rest = rest.strip_prefix(':')?;
        let (digits, unit) = rest.split_at_checked(rest.len().checked_sub(1)?)?;
        let kind = match kind {
            "B" => Kind::Boolean,
            "U" => Kind::Unsigned,
            "I" => Kind::Integer,
            "S" => Kind::Text,
            "Z" => Kind::Zip,
            "P" => Kind::Pad,
            _ => return None,
        };
        let unit = match unit {
            "B" => Unit::Byte,
            "b" => Unit::Bit,
            "V" | "v" => Unit::Variable,
            _ => return None,
        };
        fields.push(Field {
            kind,
            length: digits.parse().ok()?,
            unit,
        });
    }
    Some((length_bytes, fields))
}

struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Bits<'_> {
    fn read(&mut self, bits: usize) -> Option<u64> {
        let mut value = 0u64;
        for _ in 0..bits {
            let byte = *self.bytes.get(self.at >> 3)?;
            value = (value << 1) | u64::from((byte >> (7 - (self.at & 7))) & 1);
            self.at += 1;
        }
        Some(value)
    }

    fn aligned(&self) -> bool {
        self.at & 7 == 0
    }

    fn bytes(&mut self, count: usize) -> Option<&[u8]> {
        if !self.aligned() {
            return None;
        }
        let start = self.at >> 3;
        let out = self.bytes.get(start..start + count)?;
        self.at += count * 8;
        Some(out)
    }
}

fn field(bits: &mut Bits<'_>, field: &Field) -> Option<Option<Cell>> {
    Some(match field.kind {
        Kind::Boolean => Some(Cell::Boolean(bits.read(1)? != 0)),
        Kind::Unsigned => {
            let width = match field.unit {
                Unit::Bit => field.length,
                Unit::Byte => field.length * 8,
                Unit::Variable => return None,
            };
            if width > 32 {
                return None;
            }
            Some(Cell::Number(bits.read(width)? as i64))
        }
        Kind::Integer => {
            if field.unit != Unit::Byte {
                return None;
            }
            let bytes = bits.bytes(field.length)?;
            Some(Cell::Number(match bytes {
                [a] => i64::from(*a as i8),
                [a, b] => i64::from(i16::from_be_bytes([*a, *b])),
                [a, b, c, d] => i64::from(i32::from_be_bytes([*a, *b, *c, *d])),
                _ => return None,
            }))
        }
        Kind::Text | Kind::Zip => {
            let length = match field.unit {
                Unit::Byte if field.kind == Kind::Text => field.length,
                Unit::Variable => bits.read(field.length * 8)? as usize,
                _ => return None,
            };
            let bytes = bits.bytes(length)?;
            if field.kind == Kind::Zip {
                Some(Cell::Zip(bytes.to_vec()))
            } else {
                let decoded = text::decode(bytes);
                let end = decoded.find('\u{0}').unwrap_or(decoded.len());
                Some(Cell::Text(decoded[..end].to_owned()))
            }
        }
        Kind::Pad => {
            match field.unit {
                Unit::Byte => bits.at += field.length * 8,
                Unit::Bit => bits.at += field.length,
                Unit::Variable => return None,
            }
            None
        }
    })
}

pub fn parse(bytes: &[u8], structure_text: &str) -> Option<Table> {
    let (length_bytes, fields) = structure(structure_text)?;
    let mut bits = Bits { bytes, at: 0 };
    let mut rows = Vec::new();
    while bits.at < bytes.len() * 8 {
        let row_start = bits.at;
        let length = if length_bytes > 0 {
            bits.read(length_bytes * 8)? as usize
        } else {
            0
        };
        let body_start = bits.at;
        let mut row = Vec::new();
        for column in &fields {
            // 行が構造より短くても読む（局は最後の欄を省くことがある）。足りない欄は空か 0。
            match field(&mut bits, column) {
                Some(Some(cell)) => row.push(cell),
                Some(None) => {}
                None => match column.kind {
                    Kind::Pad => {}
                    Kind::Text => row.push(Cell::Text(String::new())),
                    Kind::Zip => row.push(Cell::Zip(Vec::new())),
                    Kind::Boolean => row.push(Cell::Boolean(false)),
                    Kind::Unsigned | Kind::Integer => row.push(Cell::Number(0)),
                },
            }
        }
        bits.at = if length_bytes > 0 {
            body_start + length * 8
        } else {
            bits.at.div_ceil(8) * 8
        };
        if bits.at <= row_start {
            return None;
        }
        rows.push(row);
    }
    Some(Table {
        fields: fields.into_iter().filter(|f| f.kind != Kind::Pad).collect(),
        rows,
    })
}

/// 郵便番号の範囲（STD-B24 第二分冊 (2/2) 付属 2 の郵便番号型）。
/// 一つの欄は `長さ, 除外の長さ, 除外の並び, 含める並び`。
fn zip_ranges(buffer: &[u8], length: usize) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut off = 0;
    let mut previous_flag = 0u8;
    let byte = |at: usize| buffer.get(at).copied().unwrap_or(0);
    while off < length && off < buffer.len() {
        if byte(off) & 0x80 != 0 {
            let digits = u32::from(byte(off) & 0x7f);
            let flag = byte(off + 1) >> 4;
            let a = u32::from(byte(off + 1) & 0xf);
            let b = u32::from(byte(off + 2) >> 4);
            let c = u32::from(byte(off + 2) & 0xf);
            let d = u32::from(byte(off + 3) >> 4);
            let e = u32::from(byte(off + 3) & 0xf);
            off += 4;
            match flag {
                0x8 => {
                    for x in [a, b, c, d, e] {
                        if x < 10 {
                            out.push((
                                (digits * 10 + x) * 10000,
                                (digits * 10 + x + 1) * 10000 - 1,
                            ));
                        }
                    }
                }
                0x9 => {
                    if b < 10 && c < 10 {
                        out.push(((digits * 10 + b) * 10000, (digits * 10 + c + 1) * 10000 - 1));
                    }
                    if d < 10 && e < 10 {
                        out.push(((digits * 10 + d) * 10000, (digits * 10 + e + 1) * 10000 - 1));
                    }
                }
                0xa => {
                    if a < 10 && b < 10 && c < 10 {
                        let base = digits * 1000 + a * 100 + b * 10 + c;
                        out.push((base * 100, (base + 1) * 100 - 1));
                        if d < 10 && e < 10 {
                            let base = digits * 1000 + a * 100 + d * 10 + e;
                            out.push((base * 100, (base + 1) * 100 - 1));
                        }
                    }
                }
                0xb => {
                    let base = digits * 1000 + a * 100 + b * 10 + c;
                    out.push((base * 100, base * 100));
                }
                0xc => {
                    if previous_flag == 0xb
                        && let Some(last) = out.last_mut()
                    {
                        let base = digits * 1000 + a * 100 + b * 10 + c;
                        last.1 = (base + 1) * 100 - 1;
                    }
                }
                0xd | 0xf => {
                    let v = digits * 100000 + a * 10000 + b * 1000 + c * 100 + d * 10 + e;
                    out.push((v, v));
                }
                0xe => {
                    if previous_flag == 0xd
                        && let Some(last) = out.last_mut()
                    {
                        last.1 = digits * 100000 + a * 10000 + b * 1000 + c * 100 + d * 10 + e;
                    }
                }
                _ => {}
            }
            previous_flag = flag;
        } else {
            let from = u32::from(byte(off) & 0x7f);
            let to = u32::from(byte(off + 1) & 0x7f);
            out.push((from * 100000, (to + 1) * 100000 - 1));
            off += 2;
            if byte(off) & 0x80 != 0 {
                let from = u32::from(byte(off) & 0x7f);
                let to = u32::from(byte(off + 1) & 0x7f);
                out.push((from * 100000, (to + 1) * 100000 - 1));
            }
            off += 2;
        }
    }
    out
}

pub fn zip_includes(field: &[u8], code: u32) -> bool {
    let Some(&length) = field.first() else {
        return false;
    };
    let exclude = usize::from(field.get(1).copied().unwrap_or(0));
    let length = usize::from(length);
    let excluded = zip_ranges(&field[2.min(field.len())..], exclude);
    let start = (2 + exclude).min(field.len());
    let included = zip_ranges(&field[start..], length.saturating_sub(1 + exclude));
    included
        .iter()
        .any(|(from, to)| (*from..=*to).contains(&code))
        && !excluded
            .iter()
            .any(|(from, to)| (*from..=*to).contains(&code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_with_a_length_byte_are_read_column_by_column() {
        let bytes = [6, 7, 2, b'a', b'b', 0x01, 0x02, 4, 9, 0, 0xff, 0xff];
        let table = parse(&bytes, "1,U:1B,S:1V,U:2B").expect("readable");
        assert_eq!(table.rows.len(), 2);
        assert_eq!(
            table.rows[0],
            [
                Cell::Number(7),
                Cell::Text("ab".into()),
                Cell::Number(0x0102)
            ]
        );
        assert_eq!(
            table.rows[1],
            [Cell::Number(9), Cell::Text("".into()), Cell::Number(0xffff)]
        );
    }

    #[test]
    fn bit_fields_and_padding_pack_into_bytes() {
        let bytes = [0b1010_0000, 0b0000_0011];
        let table = parse(&bytes, "0,B:1b,U:3b,P:4b,I:1B").expect("readable");
        assert_eq!(table.rows.len(), 1);
        assert_eq!(
            table.rows[0],
            [Cell::Boolean(true), Cell::Number(2), Cell::Number(3)]
        );
        assert_eq!(table.fields.len(), 3);
    }

    #[test]
    fn a_structure_that_cannot_be_read_is_refused() {
        assert!(structure("x,U:1B").is_none());
        assert!(structure("1,Q:1B").is_none());
        let short = parse(&[1, 2], "1,U:4B").expect("短い行は読める");
        assert_eq!(short.rows, vec![vec![Cell::Number(0)]]);
    }

    #[test]
    fn a_zip_field_is_read_the_way_the_standard_packs_it() {
        let field = [3, 0, 10, 10];
        assert!(zip_includes(&field, 1000000));
        assert!(zip_includes(&field, 1099999));
        assert!(!zip_includes(&field, 1100000));
        let field = [5, 0, 0x80 | 87, 0xd5, 0x85, 0x01];
        assert!(zip_includes(&field, 8758501));
        assert!(!zip_includes(&field, 8758502));
        let field = [7, 4, 0x80 | 87, 0x85, 0xff, 0xff, 87, 87];
        assert!(zip_includes(&field, 8700001));
        assert!(!zip_includes(&field, 8758501));
    }
}
