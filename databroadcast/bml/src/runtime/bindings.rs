// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! スクリプトから見えるもの（`browser`、`document`、要素、`BinaryTable`）。STD-B24 第二分冊 (2/2)
//! 付属 2 第 7 章のうち運用されるもの（TR-B14 第三分冊）。無いものは記録に残して NaN を返す。

use boa_engine::object::builtins::JsArray;
use boa_engine::object::{FunctionObjectBuilder, ObjectInitializer};
use boa_engine::property::Attribute;
use boa_engine::{
    Context, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction, js_string,
};

use crate::document::css;
use crate::document::dom::NodeId;
use crate::media::text;
use crate::resource::Resources;
use crate::resource::table::{self, Cell};
use crate::runtime::{Broadcast, Event, Host, Request, leaving};

mod binary_table;
mod browser;
mod document_object;
mod element;
mod epg;
mod identity;
mod media;
mod navigation;
mod network;
mod nvram;
mod style;
mod time;

use binary_table::*;
use browser::*;
use document_object::*;
use element::*;
use epg::*;
use identity::*;
use media::*;
use navigation::*;
use network::*;
use nvram::*;
use style::*;
use time::*;

pub use identity::{
    Agent, MAKER_ID, MAKER_OUI, MODEL_ID, browser_name, browser_version_text, maker_id, maker_oui,
    model_id, user_agent,
};
pub(crate) use media::request_audio;

fn host(context: &mut Context) -> &mut Host {
    context
        .host_defined_mut()
        .get_mut::<Host>()
        .expect("the host is in the context")
}

fn function(
    context: &mut Context,
    f: fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
) -> boa_engine::object::builtins::JsFunction {
    FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(f)).build()
}

fn string(value: Option<&JsValue>, context: &mut Context) -> String {
    match value {
        Some(value) if !value.is_undefined() && !value.is_null() => value
            .to_string(context)
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn number(value: Option<&JsValue>, context: &mut Context) -> f64 {
    match value {
        Some(value) => value.to_number(context).unwrap_or(f64::NAN),
        None => f64::NAN,
    }
}

fn js(text: &str) -> JsValue {
    JsValue::from(JsString::from(text))
}

fn nan() -> JsResult<JsValue> {
    Ok(JsValue::nan())
}

fn note(context: &mut Context, what: String) {
    host(context).requests.push(Request::Log(what));
}

fn integer_div(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let a = number(args.first(), context);
    let b = number(args.get(1), context);
    if b == 0.0 || a.is_nan() || b.is_nan() {
        return Ok(JsValue::nan());
    }
    Ok(JsValue::from((a / b).trunc()))
}

/// `__bml_setlen(a, n)`: native の `set` はインラインキャッシュに乗らないので、`length` の
/// 特別な代入が毎回走る。`n` を返す。
fn set_length(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let value = args.get(1).cloned().unwrap_or_default();
    if let Some(object) = args.first().and_then(JsValue::as_object) {
        object.set(js_string!("length"), value.clone(), false, context)?;
    }
    Ok(value)
}

fn log_request(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let text = string(args.first(), context);
    host(context)
        .requests
        .push(Request::Log(format!("eval: {text}")));
    Ok(JsValue::undefined())
}

/// 文字の符号は EUC-JP（STD-B24 第二分冊 (2/2) 付属 2 5.4.3.4）。あ → 0xA4A2。
fn char_code_at(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let text = this.to_string(context)?.to_std_string_escaped();
    let index = match args.first() {
        Some(v) if !v.is_undefined() => v.to_number(context)?,
        _ => 0.0,
    };
    if !index.is_finite() || index < 0.0 {
        return nan();
    }
    Ok(text
        .chars()
        .nth(index as usize)
        .and_then(crate::media::text::euc_code)
        .map_or(JsValue::nan(), JsValue::from))
}

fn from_char_code(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let mut out = String::new();
    for arg in args {
        let code = arg.to_number(context)?;
        if code.is_finite()
            && code >= 0.0
            && let Some(letter) = crate::media::text::from_euc_code(code as u32)
        {
            out.push(letter);
        }
    }
    Ok(js(&out))
}
fn patch(
    context: &mut Context,
    global: &str,
    on_prototype: bool,
    name: &str,
    f: fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
) {
    let Ok(object) = context.global_object().get(js_string!(global), context) else {
        return;
    };
    let Some(object) = object.as_object() else {
        return;
    };
    let target = if on_prototype {
        match object
            .get(js_string!("prototype"), context)
            .ok()
            .and_then(|p| p.as_object())
        {
            Some(p) => p,
            None => return,
        }
    } else {
        object
    };
    let function = function(context, f);
    let _ = target.set(JsString::from(name), function, false, context);
}

pub(crate) fn install(context: &mut Context) {
    let div = function(context, integer_div);
    let _ = context.register_global_property(
        js_string!(crate::runtime::script::DIV),
        div,
        Attribute::all(),
    );
    let setlen = function(context, set_length);
    let _ = context.register_global_property(
        js_string!(crate::runtime::script::SETLEN),
        setlen,
        Attribute::all(),
    );
    let log = function(context, log_request);
    let _ = context.register_global_property(js_string!("__bml_log"), log, Attribute::all());
    patch(context, "String", true, "charCodeAt", char_code_at);
    patch(context, "String", false, "fromCharCode", from_char_code);
    patch(context, "Date", true, "toString", date_to_string);
    patch(context, "Date", true, "toLocaleString", date_to_string);
    patch(context, "Date", true, "toUTCString", date_to_utc_string);
    let browser = browser_object(context);
    context
        .register_global_property(js_string!("browser"), browser, Attribute::all())
        .expect("a fresh global");
    let document = document_object(context);
    context
        .register_global_property(js_string!("document"), document, Attribute::all())
        .expect("a fresh global");
    let table = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(binary_table_new),
    )
    .name(js_string!("BinaryTable"))
    .length(2)
    .constructor(true)
    .build();
    context
        .register_global_property(js_string!("BinaryTable"), table, Attribute::all())
        .expect("a fresh global");
    let csv =
        FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(csv_table_new))
            .name(js_string!("CSVTable"))
            .length(2)
            .constructor(true)
            .build();
    context
        .register_global_property(js_string!("CSVTable"), csv, Attribute::all())
        .expect("a fresh global");
}

pub(crate) fn reset_globals(context: &mut Context) {
    let Some(host) = context.host_defined_mut().remove::<Host>() else {
        return;
    };
    let mut fresh = crate::runtime::clock::context(host.clock.clone());
    fresh.insert_data(*host);
    install(&mut fresh);
    *context = fresh;
}
