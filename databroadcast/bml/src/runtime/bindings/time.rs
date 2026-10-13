// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 日付（`Date` の文字列、時計の種類、日付の足し引き）とタイマー、乱数。

use super::*;

/// BML の `Date.toString()` は日本標準時の `YYYY-MM-DDThh:mm:ss`。`length != 19` で時計のずれを確かめる
/// 起動文書がある。`toUTCString` も同じ形で UTC。
fn date_millis(this: &JsValue, context: &mut Context) -> Option<f64> {
    let object = this.as_object()?.clone();
    let date = boa_engine::object::builtins::JsDate::from_object(object).ok()?;
    date.get_time(context).ok()?.as_number()
}

fn date_string(this: &JsValue, context: &mut Context, offset: i32) -> JsResult<JsValue> {
    match date_millis(this, context) {
        Some(ms) if ms.is_finite() => Ok(js(&crate::runtime::clock::civil(ms, offset).bml())),
        _ => Ok(js("Invalid Date")),
    }
}

pub(super) fn date_to_string(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    date_string(this, context, crate::runtime::clock::JST_OFFSET_SECONDS)
}

pub(super) fn date_to_utc_string(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    date_string(this, context, 0)
}

pub(super) fn set_interval(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let handler = string(args.first(), context);
    let interval = number(args.get(1), context);
    let times = number(args.get(2), context);
    if !interval.is_finite() || interval < 0.0 {
        return nan();
    }
    let times = if times.is_finite() && times > 0.0 {
        Some(times as u32)
    } else {
        None
    };
    let id = host(context).set_timer(handler, interval, times);
    Ok(JsValue::from(id))
}

pub(super) fn set_timeout(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let handler = string(args.first(), context);
    let interval = number(args.get(1), context);
    if !interval.is_finite() || interval < 0.0 {
        return nan();
    }
    let id = host(context).set_timer(handler, interval, Some(1));
    Ok(JsValue::from(id))
}

pub(super) fn clear_timer(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let id = number(args.first(), context);
    Ok(JsValue::from(u8::from(
        host(context).clear_timer(id as u32),
    )))
}

pub(super) fn pause_timer(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let id = number(args.first(), context);
    Ok(JsValue::from(u8::from(
        host(context).pause_timer(id as u32, true),
    )))
}

pub(super) fn resume_timer(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let id = number(args.first(), context);
    Ok(JsValue::from(u8::from(
        host(context).pause_timer(id as u32, false),
    )))
}

pub(super) fn sleep(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    // 待たない（待つと映像も音も止まる）。時刻だけ進めて次の tick で追いつかせる。
    let interval = number(args.first(), context);
    if interval.is_finite() && interval > 0.0 {
        host(context).now_ms += interval;
    }
    Ok(JsValue::from(1))
}

pub(super) fn random(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let most = number(args.first(), context);
    if !most.is_finite() || most < 1.0 {
        return nan();
    }
    let host = host(context);
    host.seed = host
        .seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let value = (host.seed >> 33) % (most as u64).max(1);
    Ok(JsValue::from(value as f64 + 1.0))
}

/// 0 で受像機の時計、1 で放送の時刻（TOT）。文書は両方の差で録画の再生を見分ける。
pub(super) fn set_date_mode(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let mode = number(args.first(), context);
    if mode != 0.0 && mode != 1.0 {
        return nan();
    }
    let host = host(context);
    host.date_mode = mode as u32;
    host.apply_date_mode();
    Ok(JsValue::from(1))
}

fn date_seconds(value: Option<&JsValue>, context: &mut Context) -> Option<f64> {
    let object = value?.as_object()?;
    let get_time = object.get(js_string!("getTime"), context).ok()?;
    let function = get_time.as_object()?;
    let millis = function.call(&object.into(), &[], context).ok()?;
    millis.as_number().map(|ms| ms / 1000.0)
}

pub(super) fn sub_date(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (Some(target), Some(base)) = (
        date_seconds(args.first(), context),
        date_seconds(args.get(1), context),
    ) else {
        return nan();
    };
    let unit = number(args.get(2), context) as u32;
    let difference = target - base;
    // 単位の外は零（ミリ秒）として扱う（STD-B24 第二分冊 (2/2) 付属 2 7.6.14）。
    let scale = match unit {
        1 => 1.0,
        2 => 60.0,
        3 => 3600.0,
        4 => 86400.0,
        5 => 604800.0,
        _ => 0.001,
    };
    let result = (difference / scale).trunc();
    if !(-2_147_483_648.0..=2_147_483_647.0).contains(&result) {
        return nan();
    }
    Ok(JsValue::from(result))
}

pub(super) fn add_date(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(base) = date_seconds(args.first(), context) else {
        return nan();
    };
    let amount = number(args.get(1), context);
    if amount.is_nan() {
        return Ok(args.first().cloned().unwrap_or_default());
    }
    let unit = number(args.get(2), context) as u32;
    let scale = match unit {
        1 => 1.0,
        2 => 60.0,
        3 => 3600.0,
        4 => 86400.0,
        5 => 604800.0,
        _ => 0.001,
    };
    date_of(base + amount * scale, context)
}
