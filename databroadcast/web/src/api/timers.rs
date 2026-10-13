// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 文字列を渡す `setTimeout("f()", 1000)` を通す。vibey のタイマーは関数しか
//! 受けないので `Function` に包む（通らないと読み込み待ちが終わらない局がある）。

use boa_engine::{Context, JsResult, JsValue};

use super::{global, js, set_global, string};

pub(super) fn install(context: &mut Context) {
    if global(context, "performance").is_none() {
        let performance = super::object(context);
        super::method(&performance, context, "now", 0, now);
        set_global(context, "performance", performance.into());
    }
    for (name, keep, wrapper) in [
        (
            "setTimeout",
            "__zw_setTimeout",
            set_timeout as super::Native,
        ),
        (
            "setInterval",
            "__zw_setInterval",
            set_interval as super::Native,
        ),
    ] {
        let Some(original) = global(context, name) else {
            continue;
        };
        set_global(context, keep, original.into());
        let f = super::function(context, name, 2, wrapper);
        set_global(context, name, f.into());
    }
}

fn set_timeout(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    forward("__zw_setTimeout", args, context)
}

fn set_interval(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    forward("__zw_setInterval", args, context)
}

fn forward(keep: &str, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(original) = global(context, keep) else {
        return Ok(JsValue::undefined());
    };
    let mut args = args.to_vec();
    if let Some(first) = args.first()
        && first.is_string()
    {
        let code = string(Some(first), context);
        let made = global(context, "Function")
            .and_then(|f| f.construct(&[js(&code)], None, context).ok())
            .map(JsValue::from);
        match made {
            Some(f) => args[0] = f,
            None => return Ok(JsValue::undefined()),
        }
    }
    original.call(&JsValue::undefined(), &args, context)
}

thread_local! {
    static BEGAN: f64 = zwsdk::plug::host::monotonic_ms();
}

fn now(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let ms = BEGAN.with(|began| zwsdk::plug::host::monotonic_ms() - began);
    Ok(JsValue::from(ms))
}
