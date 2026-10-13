// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! NPT と音（AIFF の長さ、ROM の音）、コンポーネントと媒体の問い合わせ。

use super::*;

/// `browser.getNPT()`: 参照からの経過を足した今の NPT（90kHz）。来ていなければ NaN。
pub(super) fn get_npt(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let host = host(context);
    let Some((npt, at)) = host.npt else {
        return nan();
    };
    let elapsed = ((host.now_ms - at) * 90.0) as i64;
    Ok(JsValue::from((npt + elapsed) as f64))
}

fn aiff_length_ms(bytes: &[u8]) -> Option<f64> {
    if bytes.len() < 12 || &bytes[0..4] != b"FORM" {
        return None;
    }
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        if &bytes[at..at + 4] == b"COMM" {
            let body = bytes.get(at + 8..at + 8 + size.min(18))?;
            if body.len() < 18 {
                return None;
            }
            let frames = u32::from_be_bytes(body[2..6].try_into().ok()?);
            let exponent = i32::from(u16::from_be_bytes([body[8], body[9]]) & 0x7fff) - 16383;
            let mantissa = u64::from_be_bytes(body[10..18].try_into().ok()?);
            if !(0..=62).contains(&exponent) {
                return None;
            }
            let rate = (mantissa >> (63 - exponent)) as f64;
            if rate <= 0.0 {
                return None;
            }
            return Some(f64::from(frames) * 1000.0 / rate);
        }
        at += 8 + size + (size & 1);
    }
    None
}

pub(crate) fn request_audio(host: &mut Host, name: &str) {
    let base = host.page.as_ref().map_or(String::new(), |p| p.name.clone());
    match host.resources.get(&base, name) {
        Some(file) => {
            let bytes = file.body.clone();
            if let Some(length) = aiff_length_ms(&bytes) {
                host.media_ends = Some((host.now_ms + length, None));
            }
            host.requests.push(Request::Audio(bytes));
        }
        None => host
            .requests
            .push(Request::Log(format!("音が無い: {name}"))),
    }
}

pub(super) fn play_rom_sound(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let Some(number) = name
        .strip_prefix("romsound://")
        .and_then(|n| n.parse::<u32>().ok())
    else {
        return nan();
    };
    host(context).requests.push(Request::Sound(number));
    Ok(JsValue::from(1))
}

pub(super) fn detect_component(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let host = host(context);
    let base = host.page.as_ref().map_or(String::new(), |p| p.name.clone());
    let Some(component) = Resources::component_of(&base, &name) else {
        return nan();
    };
    Ok(JsValue::from(u8::from(
        host.resources.has_component(component),
    )))
}

pub(super) fn is_supported_media(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    Ok(JsValue::from(u8::from(matches!(
        name.as_str(),
        "1" | "2" | "4"
    ))))
}
