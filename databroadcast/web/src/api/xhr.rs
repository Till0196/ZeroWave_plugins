// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! `XMLHttpRequest`。放送のファイルはストアから同期で読み、応答は `setTimeout(…, 0)` で返す。
//! GET 以外かヘッダーを付けたリクエストはストアを通さずそのまま送り、応答はキャッシュしない。

use boa_engine::object::builtins::JsArray;
use boa_engine::{Context, JsObject, JsResult, JsValue};

use super::{
    function, get, global, js, later, method, object, set, set_global, string, this_object,
    with_bridge,
};

pub(super) fn install(context: &mut Context) {
    let proto = object(context);
    method(&proto, context, "open", 2, open);
    method(&proto, context, "send", 1, send);
    method(&proto, context, "abort", 0, noop);
    method(&proto, context, "setRequestHeader", 2, set_request_header);
    method(&proto, context, "overrideMimeType", 1, noop);
    method(&proto, context, "getResponseHeader", 1, get_response_header);
    method(
        &proto,
        context,
        "getAllResponseHeaders",
        0,
        get_all_response_headers,
    );
    method(&proto, context, "addEventListener", 2, add_event_listener);
    method(
        &proto,
        context,
        "removeEventListener",
        2,
        remove_event_listener,
    );
    for (name, v) in [
        ("UNSENT", 0),
        ("OPENED", 1),
        ("HEADERS_RECEIVED", 2),
        ("LOADING", 3),
        ("DONE", 4),
    ] {
        super::value(&proto, context, name, JsValue::from(v));
    }
    let constructor = boa_engine::object::FunctionObjectBuilder::new(
        context.realm(),
        boa_engine::NativeFunction::from_fn_ptr(construct),
    )
    .name(boa_engine::JsString::from("XMLHttpRequest"))
    .constructor(true)
    .build();
    let constructor: JsObject = constructor.into();
    set(&constructor, "prototype", proto.clone().into(), context);
    for (name, v) in [
        ("UNSENT", 0),
        ("OPENED", 1),
        ("HEADERS_RECEIVED", 2),
        ("LOADING", 3),
        ("DONE", 4),
    ] {
        super::value(&constructor, context, name, JsValue::from(v));
    }
    set_global(context, "__zw_xhr_proto", proto.into());
    set_global(context, "XMLHttpRequest", constructor.into());
}

fn noop(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

/// `new XMLHttpRequest()`。boa の native 関数は `new` でも `this` が使えないので自分で作る。
fn construct(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let proto = global(context, "__zw_xhr_proto");
    let xhr = JsObject::from_proto_and_data(proto, ());
    set(&xhr, "readyState", JsValue::from(0), context);
    set(&xhr, "status", JsValue::from(0), context);
    set(&xhr, "statusText", js(""), context);
    set(&xhr, "responseText", js(""), context);
    set(&xhr, "response", js(""), context);
    set(&xhr, "responseType", js(""), context);
    set(&xhr, "responseURL", js(""), context);
    set(&xhr, "timeout", JsValue::from(0), context);
    set(&xhr, "withCredentials", JsValue::from(false), context);
    set(&xhr, "onreadystatechange", JsValue::null(), context);
    set(&xhr, "onload", JsValue::null(), context);
    set(&xhr, "onerror", JsValue::null(), context);
    set(&xhr, "onloadend", JsValue::null(), context);
    set(&xhr, "ontimeout", JsValue::null(), context);
    set(&xhr, "onabort", JsValue::null(), context);
    set(&xhr, "__listeners", object(context).into(), context);
    Ok(xhr.into())
}

fn set_request_header(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let xhr = this_object(this)?;
    let name = string(args.first(), context);
    let value = string(args.get(1), context);
    let headers = match get(&xhr, "__headers", context).as_object() {
        Some(h) => h.clone(),
        None => {
            let h = object(context);
            set(&xhr, "__headers", h.clone().into(), context);
            h
        }
    };
    let key = name.to_ascii_lowercase();
    let had = get(&headers, &key, context);
    let joined = if had.is_undefined() {
        value
    } else {
        format!("{}, {value}", string(Some(&had), context))
    };
    set(&headers, &key, js(&joined), context);
    Ok(JsValue::undefined())
}

fn keep_cookies(host: &str, headers: &[(String, String)]) {
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("set-cookie") {
            super::kept(|kept| kept.cookies.set(host, value, super::date::is_past));
        }
    }
}

pub(super) fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default()
}

fn request_headers(xhr: &JsObject, context: &mut Context) -> Vec<(String, String)> {
    let Some(headers) = get(xhr, "__headers", context).as_object() else {
        return Vec::new();
    };
    let Ok(keys) = headers.own_property_keys(context) else {
        return Vec::new();
    };
    keys.into_iter()
        .filter_map(|key| {
            let name = key.to_string();
            let value = headers.get(key, context).ok()?;
            Some((name, string(Some(&value), context)))
        })
        .collect()
}

fn open(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let xhr = this_object(this)?;
    let method_name = string(args.first(), context).to_ascii_uppercase();
    set(&xhr, "__method", js(&method_name), context);
    set(&xhr, "__headers", object(context).into(), context);
    let url = string(args.get(1), context);
    let base = super::dom::location_href(context);
    let resolved = url::Url::parse(&base)
        .and_then(|b| b.join(&url))
        .map(|u| u.to_string())
        .unwrap_or(url);
    set(&xhr, "__url", js(&resolved), context);
    set(&xhr, "readyState", JsValue::from(1), context);
    fire(&xhr, "readystatechange", context);
    Ok(JsValue::undefined())
}

fn send(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let xhr = this_object(this)?;
    let url = string(Some(&get(&xhr, "__url", context)), context);
    let method_name = string(Some(&get(&xhr, "__method", context)), context);
    let headers = request_headers(&xhr, context);
    let body = args
        .first()
        .filter(|v| !v.is_null_or_undefined())
        .map(|v| string(Some(v), context).into_bytes());
    let plain_get = (method_name.is_empty() || method_name == "GET") && headers.is_empty();
    let mut status: u16 = 200;
    let mut headers_seen: Vec<(String, String)> = Vec::new();
    let web = url.starts_with("http://") || url.starts_with("https://");
    let found = if plain_get || !web {
        url::Url::parse(&url)
            .ok()
            .and_then(|u| with_bridge(|_, store| store.fetch(&u)).flatten())
    } else {
        let method_name = if method_name.is_empty() {
            "GET"
        } else {
            method_name.as_str()
        };
        // cookie を持って行き、持ち帰る（局の API は応答で設定した cookie を次のリクエストで見る）。
        // 付けるのはこのホストが設定した物だけ（転送先へは本体が付け直さない）。
        let mut headers = headers;
        let host = host_of(&url);
        if !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("cookie"))
        {
            let cookie = super::kept(|kept| kept.cookies.header_for(&host));
            if !cookie.is_empty() {
                headers.push((String::from("Cookie"), cookie));
            }
        }
        match bml::resource::net::send_as(
            super::agent(),
            method_name,
            &url,
            &headers,
            body.as_deref(),
        ) {
            Ok(reply) => {
                if !(200..300).contains(&reply.status) {
                    zwsdk::plug::info(&format!("通信  {method_name} {url} → {}", reply.status));
                }
                keep_cookies(&host, &reply.headers);
                status = reply.status;
                headers_seen = reply.headers;
                Some((reply.content_type.unwrap_or_default(), reply.body))
            }
            Err(failure) => {
                zwsdk::plug::info(&format!("通信  {method_name} {url} → {failure:?}"));
                None
            }
        }
    };
    let finish = function(context, "finish", 0, finish);
    let payload = object(context);
    match found {
        Some((kind, body)) => {
            set(&payload, "ok", JsValue::from(true), context);
            set(&payload, "status", JsValue::from(status), context);
            set(&payload, "kind", js(&kind), context);
            let all: String = headers_seen
                .iter()
                .map(|(name, value)| format!("{}: {value}\r\n", name.to_ascii_lowercase()))
                .collect();
            set(&payload, "headers", js(&all), context);
            set(
                &payload,
                "text",
                js(&String::from_utf8_lossy(&body)),
                context,
            );
            let block = boa_engine::object::builtins::AlignedVec::from_iter(0, body);
            if let Ok(buffer) =
                boa_engine::object::builtins::JsArrayBuffer::from_byte_block(block, context)
            {
                set(&payload, "bytes", buffer.into(), context);
            }
        }
        None => set(&payload, "ok", JsValue::from(false), context),
    }
    later(
        context,
        finish.into(),
        vec![xhr.clone().into(), payload.into()],
    );
    Ok(JsValue::undefined())
}

fn finish(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(xhr) = args.first().and_then(JsValue::as_object) else {
        return Ok(JsValue::undefined());
    };
    let Some(payload) = args.get(1).and_then(JsValue::as_object) else {
        return Ok(JsValue::undefined());
    };
    if !get(&payload, "ok", context).to_boolean() {
        set(&xhr, "status", JsValue::from(404), context);
        set(&xhr, "statusText", js("Not Found"), context);
        set(&xhr, "readyState", JsValue::from(4), context);
        fire(&xhr, "readystatechange", context);
        fire(&xhr, "error", context);
        fire(&xhr, "loadend", context);
        return Ok(JsValue::undefined());
    }
    let text = get(&payload, "text", context);
    let kind = get(&payload, "kind", context);
    let status = get(&payload, "status", context)
        .as_number()
        .map_or(200, |n| n as u16);
    set(&xhr, "status", JsValue::from(status), context);
    set(
        &xhr,
        "statusText",
        js(if status == 200 { "OK" } else { "" }),
        context,
    );
    set(&xhr, "__type", kind, context);
    set(
        &xhr,
        "__headers_in",
        get(&payload, "headers", context),
        context,
    );
    set(&xhr, "responseText", text.clone(), context);
    set(&xhr, "responseURL", get(&xhr, "__url", context), context);
    let response_type = string(Some(&get(&xhr, "responseType", context)), context);
    let response = if response_type == "json" {
        global(context, "JSON")
            .and_then(|json| {
                json.get(boa_engine::js_string!("parse"), context)
                    .ok()
                    .and_then(|p| p.as_callable())
            })
            .and_then(|parse| {
                parse
                    .call(&JsValue::undefined(), std::slice::from_ref(&text), context)
                    .ok()
            })
            .unwrap_or(JsValue::null())
    } else if response_type == "arraybuffer" {
        // 局のバイナリテーブル（`.bin`）は `new Uint8Array(xhr.response)` で読む。
        get(&payload, "bytes", context)
    } else {
        text
    };
    set(&xhr, "response", response, context);
    for state in [2, 3, 4] {
        set(&xhr, "readyState", JsValue::from(state), context);
        fire(&xhr, "readystatechange", context);
    }
    fire(&xhr, "load", context);
    fire(&xhr, "loadend", context);
    Ok(JsValue::undefined())
}

fn fire(xhr: &JsObject, kind: &str, context: &mut Context) {
    let event = object(context);
    set(&event, "type", js(kind), context);
    set(&event, "target", xhr.clone().into(), context);
    set(&event, "currentTarget", xhr.clone().into(), context);
    let this: JsValue = xhr.clone().into();
    let handler = get(xhr, &format!("on{kind}"), context);
    if let Some(f) = handler.as_callable() {
        let _ = f.call(&this, &[event.clone().into()], context);
    }
    let listeners = get(xhr, "__listeners", context);
    if let Some(listeners) = listeners.as_object() {
        let list = get(&listeners, kind, context);
        if let Some(list) = list.as_object() {
            let array = JsArray::from_object(list).ok();
            if let Some(array) = array {
                let length = array.length(context).unwrap_or(0);
                for i in 0..length {
                    if let Ok(f) = array.get(i, context)
                        && let Some(f) = f.as_callable()
                    {
                        let _ = f.call(&this, &[event.clone().into()], context);
                    }
                }
            }
        }
    }
}

fn add_event_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let xhr = this_object(this)?;
    let kind = string(args.first(), context);
    let Some(f) = args.get(1).cloned() else {
        return Ok(JsValue::undefined());
    };
    let listeners = get(&xhr, "__listeners", context);
    let Some(listeners) = listeners.as_object() else {
        return Ok(JsValue::undefined());
    };
    let list = get(&listeners, &kind, context);
    let array = match list.as_object().and_then(|o| JsArray::from_object(o).ok()) {
        Some(array) => array,
        None => {
            let array = JsArray::new(context)?;
            let as_value: JsValue = array.clone().into();
            set(&listeners, &kind, as_value, context);
            array
        }
    };
    let _ = array.push(f, context);
    Ok(JsValue::undefined())
}

fn remove_event_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let xhr = this_object(this)?;
    let kind = string(args.first(), context);
    let Some(target) = args.get(1) else {
        return Ok(JsValue::undefined());
    };
    let listeners = get(&xhr, "__listeners", context);
    if let Some(listeners) = listeners.as_object() {
        let list = get(&listeners, &kind, context);
        if let Some(array) = list.as_object().and_then(|o| JsArray::from_object(o).ok()) {
            let length = array.length(context).unwrap_or(0);
            let mut kept = Vec::new();
            for i in 0..length {
                if let Ok(f) = array.get(i, context)
                    && !f.strict_equals(target)
                {
                    kept.push(f);
                }
            }
            let fresh = JsArray::from_iter(kept, context);
            set(&listeners, &kind, fresh.into(), context);
        }
    }
    Ok(JsValue::undefined())
}

fn get_response_header(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let xhr = this_object(this)?;
    let name = string(args.first(), context).to_ascii_lowercase();
    let all = get(&xhr, "__headers_in", context);
    let all = if all.is_undefined() {
        String::new()
    } else {
        string(Some(&all), context)
    };
    let values: Vec<&str> = all
        .split("\r\n")
        .filter_map(|line| line.split_once(": "))
        .filter(|(n, _)| *n == name)
        .map(|(_, v)| v)
        .collect();
    if !values.is_empty() {
        return Ok(js(&values.join(", ")));
    }
    if name == "content-type" {
        return Ok(get(&xhr, "__type", context));
    }
    Ok(JsValue::null())
}

fn get_all_response_headers(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let xhr = this_object(this)?;
    let all = get(&xhr, "__headers_in", context);
    if !all.is_undefined() {
        let all = string(Some(&all), context);
        if !all.is_empty() {
            return Ok(js(&all));
        }
    }
    let kind = string(Some(&get(&xhr, "__type", context)), context);
    Ok(if kind.is_empty() {
        js("")
    } else {
        js(&format!("content-type: {kind}\r\n"))
    })
}
