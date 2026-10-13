// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! NVRAM（`nvram://`）のデータの読み書き。

use super::*;

/// `nvram://` の名前をキーに。事業者専用の領域（`~/`・`local/`・`local_web/`）は
/// `original_network_id` と `transport_stream_id` で分ける（ARIB TR-B14）。
fn nvram_key(name: &str, broadcast: &Broadcast) -> Option<String> {
    let rest = name.strip_prefix("nvram://")?;
    // 本体は一行に「名前、タブ、十六進」で残す。改行・タブなどの制御文字と長すぎる名前は拒否する。
    if rest.is_empty() || rest.len() > KEY_MOST || rest.chars().any(char::is_control) {
        return None;
    }
    let (area, _) = rest.split_once('/').unwrap_or((rest, ""));
    Some(match area {
        "~" | "local" | "local_web" => format!(
            "{:04x}.{:04x}/{rest}",
            broadcast.network, broadcast.transport
        ),
        _ => rest.to_owned(),
    })
}

const KEY_MOST: usize = 256;

/// 一つの領域の大きさの上限（バイト）。受像機の不揮発性メモリはこれよりずっと小さい。
const RECORD_MOST: usize = 64 << 10;

const RECORDS_MOST: usize = 1024;
const TOTAL_MOST: usize = 1 << 20;

fn field_bits(field: &table::Field) -> Option<usize> {
    let bits = if field.unit == table::Unit::Byte {
        field.length.checked_mul(8)?
    } else {
        field.length
    };
    (bits <= RECORD_MOST * 8).then_some(bits)
}

pub(super) fn read_persistent(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let structure = string(args.get(1), context);
    let broadcast = host(context).broadcast.clone();
    let Some(key) = nvram_key(&name, &broadcast) else {
        return Ok(JsValue::null());
    };
    let Some((_, fields)) = table::structure(&format!("0,{structure}")) else {
        return Ok(JsValue::null());
    };
    // 書かれていない領域は空の配列（文字列は ""、数は 0）。null は構造が読めないときだけ。
    let bytes = host(context).nvram.get(&key).cloned();
    if key == "receiverinfo/zipcode" && bytes.is_none() {
        let zipcode = host(context).zipcode.clone();
        let zipcode = if zipcode.is_empty() {
            String::from("0000000")
        } else {
            zipcode
        };
        let items: Vec<JsValue> = fields
            .iter()
            .enumerate()
            .map(|(at, field)| match field.kind {
                table::Kind::Text if at == 0 => cell_value(&Cell::Text(zipcode.clone()), context),
                table::Kind::Text => cell_value(&Cell::Text(String::new()), context),
                _ => cell_value(&Cell::Number(0), context),
            })
            .collect();
        return Ok(JsArray::from_iter(items, context).into());
    }
    let row: Vec<Cell> = match bytes
        .as_deref()
        .and_then(|bytes| table::parse(bytes, &format!("0,{structure}")))
        .and_then(|parsed| parsed.rows.into_iter().next())
    {
        Some(row) => row,
        None => fields
            .iter()
            .map(|field| match field.kind {
                table::Kind::Text => Cell::Text(String::new()),
                _ => Cell::Number(0),
            })
            .collect(),
    };
    let items: Vec<JsValue> = row.iter().map(|cell| cell_value(cell, context)).collect();
    Ok(JsArray::from_iter(items, context).into())
}

pub(super) fn write_persistent(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let structure = string(args.get(1), context);
    let broadcast = host(context).broadcast.clone();
    let Some(key) = nvram_key(&name, &broadcast) else {
        return nan();
    };
    let Some((_, fields)) = table::structure(&format!("0,{structure}")) else {
        return nan();
    };
    let Some(data) = args.get(2).and_then(JsValue::as_object) else {
        return nan();
    };
    let length = (data
        .get(js_string!("length"), context)?
        .to_number(context)? as usize)
        .min(fields.len());
    let mut values = Vec::with_capacity(length);
    for index in 0..length {
        values.push(data.get(index as u32, context)?);
    }
    let mut bytes = Vec::new();
    let mut bit_count = 0usize;
    let mut bits: Vec<bool> = Vec::new();
    let mut at = 0;
    for field in &fields {
        if bit_count > RECORD_MOST * 8 {
            return nan();
        }
        if field.kind == table::Kind::Pad {
            let Some(width) = field_bits(field) else {
                return nan();
            };
            bits.extend(std::iter::repeat_n(false, width));
            bit_count += width;
            continue;
        }
        let value = values.get(at).cloned().unwrap_or_default();
        at += 1;
        match field.kind {
            table::Kind::Boolean => {
                bits.push(value.to_boolean());
                bit_count += 1;
            }
            table::Kind::Unsigned | table::Kind::Integer => {
                let Some(width) = field_bits(field) else {
                    return nan();
                };
                let number = value.to_number(context).unwrap_or(0.0) as i64;
                for step in (0..width).rev() {
                    // 64 ビットより上は符号で埋める（シフトがあふれないように）。
                    let shifted = number.checked_shr(step as u32).unwrap_or(number >> 63);
                    bits.push(shifted & 1 != 0);
                }
                bit_count += width;
            }
            table::Kind::Text => {
                let text = value
                    .to_string(context)
                    .map(|s| s.to_std_string_escaped())
                    .unwrap_or_default();
                let encoded = text::encode(&text);
                let mut body = encoded;
                if field.length > RECORD_MOST || body.len() > RECORD_MOST {
                    return nan();
                }
                if field.unit == table::Unit::Variable {
                    let length = body.len();
                    for step in (0..field.length * 8).rev() {
                        bits.push(length.checked_shr(step as u32).unwrap_or(0) & 1 != 0);
                    }
                    bit_count += field.length * 8;
                } else {
                    body.resize(field.length, 0);
                }
                for byte in body {
                    for step in (0..8).rev() {
                        bits.push((byte >> step) & 1 != 0);
                    }
                    bit_count += 8;
                }
            }
            table::Kind::Zip | table::Kind::Pad => return nan(),
        }
    }
    if bit_count > RECORD_MOST * 8 {
        return nan();
    }
    for chunk in bits.chunks(8) {
        let mut byte = 0u8;
        for (index, bit) in chunk.iter().enumerate() {
            if *bit {
                byte |= 0x80 >> index;
            }
        }
        bytes.push(byte);
    }
    let written = bytes.len();
    let host = host(context);
    let others: usize = host
        .nvram
        .iter()
        .filter(|(had, _)| **had != key)
        .map(|(had, bytes)| had.len() + bytes.len())
        .sum();
    let count = host.nvram.len() + usize::from(!host.nvram.contains_key(&key));
    if count > RECORDS_MOST || others + key.len() + written > TOTAL_MOST {
        return nan();
    }
    host.nvram.insert(key, bytes);
    host.nvram_changed = true;
    Ok(JsValue::from(written as f64))
}

#[cfg(test)]
mod nvram_tests {
    use super::*;

    #[test]
    fn broadcaster_areas_are_kept_apart_by_the_multiplex() {
        let east = Broadcast {
            network: 0x7fe2,
            transport: 0x7fe2,
            ..Broadcast::default()
        };
        let west = Broadcast {
            network: 0x7fe8,
            transport: 0x7fe8,
            ..Broadcast::default()
        };
        assert_ne!(
            nvram_key("nvram://~/mark", &east),
            nvram_key("nvram://~/mark", &west)
        );
        assert_eq!(
            nvram_key("nvram://~/mark", &east).unwrap(),
            "7fe2.7fe2/~/mark"
        );
        assert_ne!(
            nvram_key("nvram://local/x", &east),
            nvram_key("nvram://local/x", &west)
        );
        assert_eq!(
            nvram_key("nvram://receiverinfo/zipcode", &east),
            nvram_key("nvram://receiverinfo/zipcode", &west)
        );
        assert_eq!(nvram_key("nvram://4/x", &east).unwrap(), "4/x");
        assert!(nvram_key("http://x", &east).is_none());
        assert!(nvram_key("nvram://local/a\tあ", &east).is_none(), "タブ");
        assert!(nvram_key("nvram://4/a\nb", &east).is_none(), "改行");
        assert!(nvram_key(&format!("nvram://4/{}", "x".repeat(300)), &east).is_none());
    }
}
