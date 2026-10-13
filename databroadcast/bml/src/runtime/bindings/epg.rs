// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 番組（EPG、選局連動、番組の時刻、選局）と字幕のインターフェース。

use super::*;

pub(super) fn epg_is_reserved(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let uri = string(args.first(), context);
    if !uri.starts_with("arib://") {
        return nan();
    }
    Ok(JsValue::from(0))
}

pub(super) fn tuning_linkage_source(
    _: &JsValue,
    _: &[JsValue],
    _: &mut Context,
) -> JsResult<JsValue> {
    Ok(js(""))
}

pub(super) fn tuning_linkage_type(
    _: &JsValue,
    _: &[JsValue],
    _: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::from(-1))
}

pub(super) fn check_access_info(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let uri = string(args.first(), context);
    if uri.is_empty() {
        return nan();
    }
    Ok(JsValue::from(0))
}

pub(super) fn program_relative_time(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let host = host(context);
    let Some(start) = host.broadcast.start else {
        return nan();
    };
    let now_ms =
        crate::runtime::clock::BmlClock::machine_millis() + host.stream_offset_ms.unwrap_or(0);
    let seconds = (now_ms / 1000 - start).max(0);
    Ok(JsValue::from(seconds as f64))
}

fn language_argument(args: &[JsValue], context: &mut Context) -> Option<u32> {
    let language = number(args.first(), context);
    (1.0..=8.0).contains(&language).then_some(language as u32)
}

pub(super) fn cc_display_status(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(language) = language_argument(args, context) else {
        return nan();
    };
    let shown = u32::from(host(context).broadcast.caption_shown);
    Ok(JsValue::from(u32::from(shown != 0 && shown == language)))
}

pub(super) fn set_cc_display_status(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(language) = language_argument(args, context) else {
        return nan();
    };
    let on = args.get(1).is_some_and(|v| v.to_boolean());
    host(context)
        .requests
        .push(Request::CaptionDisplay { language, on });
    Ok(JsValue::from(1))
}

/// その言語の字幕が放送に乗っているか。分かるのは第一言語だけ。
pub(super) fn cc_language_status(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(language) = language_argument(args, context) else {
        return nan();
    };
    let carried = host(context).broadcast.captions;
    Ok(JsValue::from(u32::from(language == 1 && carried)))
}

pub(super) fn program_id(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let kind = number(args.first(), context) as u32;
    let broadcast = host(context).broadcast.clone();
    Ok(match kind {
        1 => js(&format!("0x{:04X}", broadcast.event)),
        2 => js(&format!("0x{:04X}", broadcast.service)),
        // 3 は original_network_id、4 は transport_stream_id（`browser.getProgramID`）。
        3 => js(&format!("0x{:04X}", broadcast.network)),
        4 => js(&format!("0x{:04X}", broadcast.transport)),
        // 6 は番組の参照、7 はサービスの参照、9 は network_id。
        // 5・8 の content_id は持たないので null。
        6 => js(&format!(
            "arib://{:04X}.{:04X}.{:04X}.{:04X}",
            broadcast.network, broadcast.transport, broadcast.service, broadcast.event
        )),
        7 => js(&format!(
            "arib://{:04X}.{:04X}.{:04X}",
            broadcast.network, broadcast.transport, broadcast.service
        )),
        9 => js(&format!("0x{:04X}", broadcast.network)),
        _ => JsValue::null(),
    })
}

pub(super) fn date_of(seconds: f64, context: &mut Context) -> JsResult<JsValue> {
    let date = context.global_object().get(js_string!("Date"), context)?;
    let Some(date) = date.as_object() else {
        return Ok(JsValue::null());
    };
    let value = date.construct(&[JsValue::from(seconds * 1000.0)], None, context)?;
    Ok(value.into())
}

pub(super) fn event_start_time(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(start) = host(context).broadcast.start else {
        return Ok(JsValue::null());
    };
    date_of(start as f64, context)
}

pub(super) fn event_duration(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(duration) = host(context).broadcast.duration else {
        return nan();
    };
    Ok(JsValue::from(duration))
}

pub(super) fn epg_tune(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let reference = string(args.first(), context);
    let Some(rest) = reference.strip_prefix("arib://") else {
        return nan();
    };
    let ids: Vec<u16> = rest
        .split('/')
        .next()
        .unwrap_or("")
        .split('.')
        .map(|part| {
            let part = part.trim_start_matches("0x").trim_start_matches("0X");
            u16::from_str_radix(part, 16).unwrap_or(0)
        })
        .collect();
    let [network, transport, service] = ids[..] else {
        return nan();
    };
    host(context).requests.push(Request::Tune {
        network,
        transport,
        service,
    });
    Ok(JsValue::from(1))
}
