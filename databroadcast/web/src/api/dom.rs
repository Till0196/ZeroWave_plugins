// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! Blitz の DOM に無いブラウザの API（`document.implementation`、`location` の遷移、
//! 動的に足された `<script>`、`audio.play()`、キーの `keyCode` など）。

use boa_engine::JsString;
use boa_engine::object::FunctionObjectBuilder;
use boa_engine::object::builtins::JsArray;
use boa_engine::property::{PropertyDescriptor, PropertyKey};
use boa_engine::{Context, JsObject, JsResult, JsValue, NativeFunction, Source};

use super::{
    Request, accessor, function, get, global, js, later, method, object, request, set, set_global,
    string, this_object, value, with_bridge,
};

pub(super) fn install(context: &mut Context) {
    let Some(document) = global(context, "document") else {
        return;
    };
    if get(&document, "implementation", context).is_undefined() {
        let implementation = object(context);
        method(
            &implementation,
            context,
            "createHTMLDocument",
            1,
            create_html_document,
        );
        method(&implementation, context, "hasFeature", 2, r#true);
        value(&document, context, "implementation", implementation.into());
    }
    if get(&document, "baseURI", context).is_undefined() {
        accessor(&document, context, "baseURI", Some(base_uri), None);
    }
    // Vue 2 は `performance.now` があると `document.createEvent("Event").timeStamp` で時計を選ぶ。
    if !get(&document, "createEvent", context).is_callable() {
        method(&document, context, "createEvent", 1, create_event);
    }
    // document.cookie。jQuery.cookie ごしに設定を置き、無いと初期化で落ちるアプリがある。
    accessor(
        &document,
        context,
        "cookie",
        Some(cookie_get),
        Some(cookie_set),
    );
    if let Some(location) = global(context, "location") {
        let held = get(&location, "href", context);
        set_global(context, "__zw_href", held);
        method(&location, context, "replace", 1, navigate);
        method(&location, context, "assign", 1, navigate);
        method(&location, context, "reload", 0, reload);
        accessor(&location, context, "href", Some(href), Some(navigate));
        accessor(
            &document,
            context,
            "location",
            Some(document_location),
            Some(navigate),
        );
    }
    let history = object(context);
    value(&history, context, "length", JsValue::from(1));
    value(&history, context, "scrollRestoration", js("auto"));
    value(&history, context, "state", JsValue::null());
    method(&history, context, "pushState", 3, push_state);
    method(&history, context, "replaceState", 3, push_state);
    for name in ["go", "back", "forward"] {
        method(&history, context, name, 0, undefined);
    }
    set_global(context, "history", history.into());
    let node_proto = document
        .prototype()
        .and_then(|document_proto| document_proto.prototype());
    if let Some(node_proto) = node_proto {
        for (name, length) in [("appendChild", 1), ("insertBefore", 2)] {
            let original = get(&node_proto, name, context);
            if let Some(original) = original.as_object() {
                let wrapped = NativeFunction::from_copy_closure_with_captures(
                    |this, args, original: &JsObject, context| {
                        let result = original.call(this, args, context)?;
                        if let Some(node) = args.first() {
                            run_if_script(node, context);
                        }
                        Ok(result)
                    },
                    original.clone(),
                );
                let wrapped =
                    boa_engine::object::FunctionObjectBuilder::new(context.realm(), wrapped)
                        .name(boa_engine::JsString::from(name))
                        .length(length)
                        .build();
                set(&node_proto, name, wrapped.into(), context);
            }
        }
    }
    let element_proto = get(&document, "documentElement", context)
        .as_object()
        .and_then(|e| e.prototype());
    if let Some(element_proto) = element_proto {
        method(&element_proto, context, "play", 0, play);
        method(&element_proto, context, "pause", 0, pause);
        method(&element_proto, context, "load", 0, load);
        method(&element_proto, context, "canPlayType", 1, can_play_type);
        for (name, v) in [
            ("paused", JsValue::from(true)),
            ("duration", JsValue::from(0)),
            ("volume", JsValue::from(1)),
            ("muted", JsValue::from(false)),
        ] {
            if get(&element_proto, name, context).is_undefined() {
                value(&element_proto, context, name, v);
            }
        }
        if get(&element_proto, "currentTime", context).is_undefined() {
            value(&element_proto, context, "currentTime", JsValue::from(0));
        }
        super::receiver::install_video_object(&element_proto, context);
        super::canvas::install(&element_proto, context);
        // リンク先の URL を解析したプロパティ。Blitz に無く、undefined だと pjax が何もしない。
        for (name, part) in [
            ("protocol", UrlPart::Protocol),
            ("host", UrlPart::Host),
            ("hostname", UrlPart::Hostname),
            ("port", UrlPart::Port),
            ("pathname", UrlPart::Pathname),
            ("search", UrlPart::Search),
            ("hash", UrlPart::Hash),
            ("origin", UrlPart::Origin),
        ] {
            let getter = NativeFunction::from_copy_closure_with_captures(
                |this, _, part: &JsValue, context| {
                    let part = UrlPart::of(part.as_number().unwrap_or(0.0) as u8);
                    Ok(js(&url_part(this, part, context)))
                },
                JsValue::from(part as u8),
            );
            let getter = FunctionObjectBuilder::new(context.realm(), getter).build();
            let _ = element_proto.define_property_or_throw(
                PropertyKey::from(JsString::from(name)),
                PropertyDescriptor::builder()
                    .get(getter)
                    .enumerable(false)
                    .configurable(true)
                    .build(),
                context,
            );
        }
        // 属性を反映するプロパティ（`img.src = "…"`）。Blitz に無く、画像の差し替えに使われる。
        for name in ["src", "href", "alt"] {
            if get(&element_proto, name, context).is_undefined() {
                let key = js(name);
                let getter = NativeFunction::from_copy_closure_with_captures(
                    |this, _, key: &JsValue, context| {
                        let element = this_object(this)?;
                        let f = get(&element, "getAttribute", context);
                        let v = f
                            .as_callable()
                            .and_then(|f| f.call(this, std::slice::from_ref(key), context).ok())
                            .filter(|v| !v.is_null_or_undefined())
                            .unwrap_or_else(|| js(""));
                        Ok(v)
                    },
                    key.clone(),
                );
                let setter = NativeFunction::from_copy_closure_with_captures(
                    |this, args, key: &JsValue, context| {
                        let element = this_object(this)?;
                        let f = get(&element, "setAttribute", context);
                        let v = args.first().cloned().unwrap_or_else(|| js(""));
                        if let Some(f) = f.as_callable() {
                            f.call(this, &[key.clone(), v], context)?;
                        }
                        Ok(JsValue::undefined())
                    },
                    key,
                );
                let getter = FunctionObjectBuilder::new(context.realm(), getter).build();
                let setter = FunctionObjectBuilder::new(context.realm(), setter).build();
                let _ = element_proto.define_property_or_throw(
                    PropertyKey::from(JsString::from(name)),
                    PropertyDescriptor::builder()
                        .get(getter)
                        .set(setter)
                        .enumerable(false)
                        .configurable(true)
                        .build(),
                    context,
                );
            }
        }
    }
    // document のキーのリスナーは window へ。Blitz のイベントの伝播は document を通らない。
    method(
        &document,
        context,
        "addEventListener",
        3,
        document_add_listener,
    );
    method(
        &document,
        context,
        "removeEventListener",
        3,
        document_remove_listener,
    );
    for (name, getter, setter) in [
        (
            "onkeydown",
            on_keydown_get as super::Native,
            on_keydown_set as super::Native,
        ),
        (
            "onkeyup",
            on_keyup_get as super::Native,
            on_keyup_set as super::Native,
        ),
        (
            "onkeypress",
            on_keypress_get as super::Native,
            on_keypress_set as super::Native,
        ),
    ] {
        accessor(&document, context, name, Some(getter), Some(setter));
    }
}

/// `document.cookie` は文書の置かれたホストの物（通信の相手が置いた物は見せない）。
fn cookie_get(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let host = super::xhr::host_of(&location_href(context));
    let all = super::kept(|kept| kept.cookies.header_for(&host));
    Ok(js(&all))
}

/// `document.cookie = "…"`。期限は過ぎたかだけを見て、過ぎていれば消す。
fn cookie_set(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let text = string(args.first(), context);
    let host = super::xhr::host_of(&location_href(context));
    super::kept(|kept| kept.cookies.set(&host, &text, super::date::is_past));
    Ok(JsValue::undefined())
}

#[derive(Clone, Copy)]
enum UrlPart {
    Protocol = 0,
    Host = 1,
    Hostname = 2,
    Port = 3,
    Pathname = 4,
    Search = 5,
    Hash = 6,
    Origin = 7,
}

impl UrlPart {
    fn of(number: u8) -> Self {
        match number {
            1 => Self::Host,
            2 => Self::Hostname,
            3 => Self::Port,
            4 => Self::Pathname,
            5 => Self::Search,
            6 => Self::Hash,
            7 => Self::Origin,
            _ => Self::Protocol,
        }
    }
}

fn url_part(this: &JsValue, part: UrlPart, context: &mut Context) -> String {
    let Ok(element) = this_object(this) else {
        return String::new();
    };
    let attribute = |name: &str, context: &mut Context| -> String {
        get(&element, "getAttribute", context)
            .as_callable()
            .and_then(|f| f.call(this, &[js(name)], context).ok())
            .map(|v| string(Some(&v), context))
            .unwrap_or_default()
    };
    let mut raw = attribute("href", context);
    if raw.is_empty() {
        raw = attribute("src", context);
    }
    if raw.is_empty() {
        return String::new();
    }
    resolve(&raw, context).map_or(String::new(), |url| part.of_url(&url))
}

impl UrlPart {
    fn of_url(self, url: &url::Url) -> String {
        match self {
            UrlPart::Protocol => format!("{}:", url.scheme()),
            UrlPart::Host => match url.port() {
                Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
                None => url.host_str().unwrap_or_default().to_owned(),
            },
            UrlPart::Hostname => url.host_str().unwrap_or_default().to_owned(),
            UrlPart::Port => url.port().map(|p| p.to_string()).unwrap_or_default(),
            UrlPart::Pathname => url.path().to_owned(),
            UrlPart::Search => url.query().map_or(String::new(), |q| format!("?{q}")),
            UrlPart::Hash => url.fragment().map_or(String::new(), |f| format!("#{f}")),
            UrlPart::Origin => format!(
                "{}://{}",
                url.scheme(),
                match url.port() {
                    Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
                    None => url.host_str().unwrap_or_default().to_owned(),
                }
            ),
        }
    }
}

fn create_event(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let kind = string(args.first(), context).to_lowercase();
    if let Some(event_class) = global(context, "Event")
        && event_class.is_constructor()
        && let Ok(event) = event_class.construct(&[js(&kind)], None, context)
    {
        return Ok(event.into());
    }
    let event = object(context);
    value(&event, context, "type", js(""));
    for name in ["bubbles", "cancelable", "defaultPrevented"] {
        value(&event, context, name, JsValue::from(false));
    }
    let stamp = global(context, "performance")
        .map(|performance| get(&performance, "now", context))
        .and_then(|now| {
            now.as_callable()
                .map(|f| f.call(&JsValue::undefined(), &[], context))
        })
        .and_then(Result::ok)
        .unwrap_or_else(|| JsValue::from(zwsdk::plug::host::unix_ms()));
    value(&event, context, "timeStamp", stamp);
    value(&event, context, "target", JsValue::null());
    value(&event, context, "currentTarget", JsValue::null());
    method(&event, context, "initEvent", 3, init_event);
    method(&event, context, "preventDefault", 0, prevent_default);
    method(&event, context, "stopPropagation", 0, undefined);
    method(&event, context, "stopImmediatePropagation", 0, undefined);
    Ok(event.into())
}

fn init_event(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let event = this_object(this)?;
    set(&event, "type", js(&string(args.first(), context)), context);
    set(
        &event,
        "bubbles",
        JsValue::from(args.get(1).is_some_and(JsValue::to_boolean)),
        context,
    );
    set(
        &event,
        "cancelable",
        JsValue::from(args.get(2).is_some_and(JsValue::to_boolean)),
        context,
    );
    Ok(JsValue::undefined())
}

fn prevent_default(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let event = this_object(this)?;
    set(&event, "defaultPrevented", JsValue::from(true), context);
    Ok(JsValue::undefined())
}

fn r#true(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(true))
}

pub(super) fn undefined(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

/// 別の文書は作らず、外れた要素を返す。入れたノードはあとで本物の文書へ付け替えられる。
fn create_html_document(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(document) = global(context, "document") else {
        return Ok(JsValue::null());
    };
    let create = get(&document, "createElement", context);
    let Some(create) = create.as_callable() else {
        return Ok(JsValue::null());
    };
    let this: JsValue = document.clone().into();
    let head = create.call(&this, &[js("head")], context)?;
    let body = create.call(&this, &[js("body")], context)?;
    let fake = object(context);
    value(&fake, context, "head", head);
    value(&fake, context, "body", body.clone());
    value(&fake, context, "documentElement", body);
    value(&fake, context, "nodeType", JsValue::from(9));
    value(&fake, context, "ownerDocument", JsValue::null());
    for name in ["createElement", "createTextNode", "createDocumentFragment"] {
        let f = get(&document, name, context);
        if let Some(f) = f.as_object() {
            let bound = NativeFunction::from_copy_closure_with_captures(
                |_, args, f: &JsObject, context| {
                    let document =
                        global(context, "document").map_or(JsValue::undefined(), JsValue::from);
                    f.call(&document, args, context)
                },
                f.clone(),
            );
            let bound =
                boa_engine::object::FunctionObjectBuilder::new(context.realm(), bound).build();
            value(&fake, context, name, bound.into());
        }
    }
    for name in ["getElementsByTagName", "querySelectorAll", "querySelector"] {
        let body = get(&fake, "body", context);
        let f = body.as_object().map(|b| get(&b, name, context));
        if let Some(f) = f.and_then(|f| f.as_object()) {
            let bound = NativeFunction::from_copy_closure_with_captures(
                |_, args, (f, body): &(JsObject, JsValue), context| f.call(body, args, context),
                (f.clone(), body.clone()),
            );
            let bound =
                boa_engine::object::FunctionObjectBuilder::new(context.realm(), bound).build();
            value(&fake, context, name, bound.into());
        }
    }
    Ok(fake.into())
}

pub(crate) fn location_href(context: &mut Context) -> String {
    let v = context
        .global_object()
        .get(boa_engine::js_string!("__zw_href"), context)
        .unwrap_or(JsValue::undefined());
    string(Some(&v), context)
}

fn base_uri(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(js(&location_href(context)))
}

fn href(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(js(&location_href(context)))
}

fn resolve(target: &str, context: &mut Context) -> Option<url::Url> {
    let base = location_href(context);
    url::Url::parse(&base).and_then(|b| b.join(target)).ok()
}

fn navigate(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let target = string(args.first(), context);
    if let Some(url) = resolve(&target, context) {
        request(Request::Navigate(url.to_string()));
    }
    Ok(JsValue::undefined())
}

/// URL を書き換えるだけでページは読み直さない（読み直すと pjax の差し替えを捨てる）。
fn push_state(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Ok(history) = this_object(this) {
        let state = args.first().cloned().unwrap_or(JsValue::null());
        set(&history, "state", state, context);
    }
    if let Some(url) = args.get(2).filter(|u| !u.is_null_or_undefined()) {
        let target = string(Some(url), context);
        if let Some(url) = resolve(&target, context) {
            set_href(&url, context);
        }
    }
    Ok(JsValue::undefined())
}

fn set_href(url: &url::Url, context: &mut Context) {
    set_global(context, "__zw_href", js(url.as_str()));
    let Some(location) = global(context, "location") else {
        return;
    };
    for (name, part) in [
        ("protocol", UrlPart::Protocol),
        ("host", UrlPart::Host),
        ("hostname", UrlPart::Hostname),
        ("port", UrlPart::Port),
        ("pathname", UrlPart::Pathname),
        ("search", UrlPart::Search),
        ("hash", UrlPart::Hash),
        ("origin", UrlPart::Origin),
    ] {
        set(&location, name, js(&part.of_url(url)), context);
    }
}

fn reload(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    request(Request::Navigate(location_href(context)));
    Ok(JsValue::undefined())
}

fn run_if_script(node: &JsValue, context: &mut Context) {
    let Some(element) = node.as_object() else {
        return;
    };
    let tag = string(Some(&get(&element, "tagName", context)), context).to_ascii_lowercase();
    if tag != "script" {
        return;
    }
    let get_attribute = get(&element, "getAttribute", context);
    let attribute = |name: &str, context: &mut Context| -> String {
        get_attribute
            .as_callable()
            .and_then(|f| f.call(node, &[js(name)], context).ok())
            .map(|v| string(Some(&v), context))
            .unwrap_or_default()
    };
    let kind = attribute("type", context).trim().to_ascii_lowercase();
    if !kind.is_empty() && kind != "text/javascript" && kind != "application/javascript" {
        return;
    }
    // `script.src = …` は accessor が無く素のプロパティに乗るので、両方見る。
    let mut src = attribute("src", context);
    if src.is_empty() {
        src = string(Some(&get(&element, "src", context)), context);
    }
    let ok = if src.is_empty() {
        // `script.text = …` も中身。Blitz では `textContent` から見えない。
        let mut code = string(Some(&get(&element, "textContent", context)), context);
        if code.trim().is_empty() {
            code = string(Some(&get(&element, "text", context)), context);
        }
        eval(context, &code, "(inline)")
    } else {
        let base = location_href(context);
        match url::Url::parse(&base).and_then(|b| b.join(&src)) {
            Ok(url) => match with_bridge(|_, store| store.fetch(&url)).flatten() {
                Some((_, body)) => eval(context, &String::from_utf8_lossy(&body), url.as_str()),
                None => false,
            },
            Err(_) => false,
        }
    };
    let event = object(context);
    value(
        &event,
        context,
        "type",
        js(if ok { "load" } else { "error" }),
    );
    value(&event, context, "target", node.clone());
    let handler = get(&element, if ok { "onload" } else { "onerror" }, context);
    if handler.is_callable() {
        let call = function(context, "call", 2, call_with_this);
        later(
            context,
            call.into(),
            vec![handler, node.clone(), event.into()],
        );
    }
}

fn call_with_this(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (Some(f), Some(this)) = (args.first().and_then(JsValue::as_callable), args.get(1)) else {
        return Ok(JsValue::undefined());
    };
    f.call(this, &args[2..], context)
}

fn eval(context: &mut Context, code: &str, name: &str) -> bool {
    match context.eval(Source::from_bytes(code.as_bytes())) {
        Ok(_) => true,
        Err(error) => {
            request(Request::Log(format!("JS ({name}): {error}")));
            false
        }
    }
}

fn audio_src(this: &JsValue, element: &JsObject, context: &mut Context) -> String {
    let get_attribute = get(element, "getAttribute", context);
    let mut src = get_attribute
        .as_callable()
        .and_then(|f| f.call(this, &[js("src")], context).ok())
        .map(|v| string(Some(&v), context))
        .unwrap_or_default();
    if src.is_empty() {
        let query = get(element, "querySelector", context);
        if let Some(source) = query
            .as_callable()
            .and_then(|f| f.call(this, &[js("source")], context).ok())
            .and_then(|v| v.as_object())
        {
            let ga = get(&source, "getAttribute", context);
            src = ga
                .as_callable()
                .and_then(|f| f.call(&source.clone().into(), &[js("src")], context).ok())
                .map(|v| string(Some(&v), context))
                .unwrap_or_default();
        }
    }
    src
}

fn fetch_audio(src: &str, context: &mut Context) -> Option<Vec<u8>> {
    let base = global(context, "document")
        .map(|d| get(&d, "baseURI", context))
        .map(|v| string(Some(&v), context))
        .unwrap_or_default();
    let url = url::Url::parse(&base)
        .and_then(|base| base.join(src))
        .or_else(|_| url::Url::parse(src))
        .ok()?;
    with_bridge(|_, store| store.fetch(&url).map(|(_, body)| body)).flatten()
}

/// AIFF/AIFF-C の長さ（ms）。COMM の numSampleFrames ÷ sampleRate（80 ビット）。
fn aiff_duration_ms(bytes: &[u8]) -> Option<u64> {
    if bytes.len() < 12 || &bytes[0..4] != b"FORM" {
        return None;
    }
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_be_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let body = bytes.get(at + 8..at + 8 + size)?;
        if id == b"COMM" && body.len() >= 18 {
            let frames = u32::from_be_bytes(body[2..6].try_into().ok()?);
            let exponent = i32::from(u16::from_be_bytes([body[8] & 0x7f, body[9]])) - 16383;
            let mantissa = u64::from_be_bytes(body[10..18].try_into().ok()?);
            let rate = (mantissa as f64) * 2f64.powi(exponent - 63);
            if rate <= 0.0 {
                return None;
            }
            return Some((f64::from(frames) * 1000.0 / rate).round() as u64);
        }
        at += 8 + size + (size & 1);
    }
    None
}

fn load(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let element = this_object(this)?;
    let src = audio_src(this, &element, context);
    if !src.is_empty() && !src.starts_with("romsound://") {
        fetch_audio(&src, context);
    }
    Ok(JsValue::undefined())
}

/// 鳴らしている `audio` に付けるマーク。値は鳴り終わる時刻（`Date.now()` の ms）。
pub(crate) const PLAYING_UNTIL: &str = "data-zw-playing-until";

fn pause(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let element = this_object(this)?;
    let get_attribute = get(&element, "getAttribute", context);
    let playing = get_attribute
        .as_callable()
        .and_then(|f| f.call(this, &[js(PLAYING_UNTIL)], context).ok())
        .is_some_and(|v| !v.is_null_or_undefined());
    if playing {
        if let Some(remove) = get(&element, "removeAttribute", context).as_callable() {
            let _ = remove.call(this, &[js(PLAYING_UNTIL)], context);
        }
        request(Request::AudioStop);
    }
    Ok(JsValue::undefined())
}

fn play(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let element = this_object(this)?;
    let src = audio_src(this, &element, context);
    if let Some(number) = src
        .strip_prefix("romsound://")
        .and_then(|n| n.parse::<u32>().ok())
    {
        request(Request::Sound(number));
    } else if !src.is_empty()
        && let Some(bytes) = fetch_audio(&src, context)
    {
        let until = super::date::now_millis() as u64 + aiff_duration_ms(&bytes).unwrap_or(0);
        if let Some(set) = get(&element, "setAttribute", context).as_callable() {
            let _ = set.call(this, &[js(PLAYING_UNTIL), js(&until.to_string())], context);
        }
        request(Request::Audio(bytes));
    }
    let resolved = global(context, "Promise")
        .and_then(|p| get(&p, "resolve", context).as_callable().map(|f| (p, f)))
        .and_then(|(p, f)| f.call(&p.into(), &[], context).ok());
    Ok(resolved.unwrap_or(JsValue::undefined()))
}

fn can_play_type(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let kind = string(args.first(), context);
    Ok(js(if kind.contains("romsound") || kind.contains("arib") {
        "probably"
    } else {
        ""
    }))
}

pub(crate) fn caption(context: &mut Context, tag: u16, text: &str) {
    let Some(list) =
        global(context, "__zw_caption_listeners").and_then(|o| JsArray::from_object(o).ok())
    else {
        return;
    };
    let length = list.length(context).unwrap_or(0);
    for i in 0..length {
        let Ok(pair) = list.get(i, context) else {
            continue;
        };
        let Some(pair) = pair.as_object() else {
            continue;
        };
        let wanted = pair
            .get(1, context)
            .ok()
            .and_then(|v| v.as_number())
            .unwrap_or(-1.0);
        if wanted >= 0.0 && wanted as u16 != tag {
            continue;
        }
        if let Ok(f) = pair.get(0, context)
            && let Some(f) = f.as_callable()
        {
            let _ = f.call(&JsValue::undefined(), &[js(text)], context);
        }
    }
}

fn is_key_event(kind: &str) -> bool {
    matches!(kind, "keydown" | "keyup" | "keypress")
}

fn document_add_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    forward_listener(this, args, context, "addEventListener")
}

fn document_remove_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    forward_listener(this, args, context, "removeEventListener")
}

fn forward_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
    name: &str,
) -> JsResult<JsValue> {
    let kind = string(args.first(), context);
    let document = this_object(this)?;
    if is_key_event(&kind) {
        let window = context.global_object();
        let f = get(&window, name, context);
        if let Some(f) = f.as_callable() {
            return f.call(&window.into(), args, context);
        }
        return Ok(JsValue::undefined());
    }
    let proto = document.prototype();
    let f = proto.map(|p| get(&p, name, context));
    if let Some(f) = f.and_then(|f| f.as_callable()) {
        return f.call(this, args, context);
    }
    Ok(JsValue::undefined())
}

fn on_key_get(kind: &str, context: &mut Context) -> JsResult<JsValue> {
    Ok(global(context, &format!("__zw_document_{kind}")).map_or(JsValue::null(), JsValue::from))
}

fn on_key_set(kind: &str, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let window = context.global_object();
    let key = format!("__zw_document_{kind}");
    if let Some(wrapper) = global(context, &format!("{key}_wrapper")) {
        let remove = get(&window, "removeEventListener", context);
        if let Some(remove) = remove.as_callable() {
            let _ = remove.call(&window.clone().into(), &[js(kind), wrapper.into()], context);
        }
        set_global(context, &format!("{key}_wrapper"), JsValue::undefined());
    }
    let handler = args.first().cloned().unwrap_or(JsValue::null());
    set_global(context, &key, handler.clone());
    let Some(handler) = handler.as_object().filter(|h| h.is_callable()) else {
        return Ok(JsValue::undefined());
    };
    let document: JsValue = global(context, "document").map_or(JsValue::undefined(), JsValue::from);
    let wrapper = NativeFunction::from_copy_closure_with_captures(
        |_, args, (handler, document): &(JsObject, JsValue), context| {
            handler.call(document, args, context)
        },
        (handler, document),
    );
    let wrapper = boa_engine::object::FunctionObjectBuilder::new(context.realm(), wrapper)
        .name(boa_engine::JsString::from(kind))
        .length(1)
        .build();
    let add = get(&window, "addEventListener", context);
    if let Some(add) = add.as_callable() {
        let _ = add.call(
            &window.clone().into(),
            &[js(kind), wrapper.clone().into()],
            context,
        );
    }
    set_global(context, &format!("{key}_wrapper"), wrapper.into());
    Ok(JsValue::undefined())
}

fn on_keydown_get(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    on_key_get("keydown", context)
}
fn on_keydown_set(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    on_key_set("keydown", args, context)
}
fn on_keyup_get(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    on_key_get("keyup", context)
}
fn on_keyup_set(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    on_key_set("keyup", args, context)
}
fn on_keypress_get(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    on_key_get("keypress", context)
}
fn on_keypress_set(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    on_key_set("keypress", args, context)
}

/// `key` の名前 → `keyCode`。リモコンの d ボタンは `Key::Character("d")` で届く。
fn key_code_of(key: &str) -> u16 {
    let name = match key {
        "ArrowUp" => "VK_UP",
        "ArrowDown" => "VK_DOWN",
        "ArrowLeft" => "VK_LEFT",
        "ArrowRight" => "VK_RIGHT",
        "Enter" => "VK_ENTER",
        "GoBack" | "Backspace" | "Escape" => "VK_BACK",
        "ColorF0Red" => "VK_RED",
        "ColorF1Green" => "VK_GREEN",
        "ColorF2Yellow" => "VK_YELLOW",
        "ColorF3Blue" => "VK_BLUE",
        "d" => "VK_DBUTTON",
        "Subtitle" => "VK_SUBTITLE",
        "PageUp" => "VK_PAGE_UP",
        "PageDown" => "VK_PAGE_DOWN",
        "Tab" => "VK_TAB",
        "0" => "VK_0",
        "1" => "VK_1",
        "2" => "VK_2",
        "3" => "VK_3",
        "4" => "VK_4",
        "5" => "VK_5",
        "6" => "VK_6",
        "7" => "VK_7",
        "8" => "VK_8",
        "9" => "VK_9",
        _ => return 0,
    };
    super::receiver::KEYS
        .iter()
        .find(|(n, _)| *n == name)
        .map_or(0, |(_, code)| *code)
}

fn key_code(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let event = this_object(this)?;
    let key = string(Some(&get(&event, "key", context)), context);
    Ok(JsValue::from(key_code_of(&key)))
}

fn remember_event_proto(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Some(proto) = args
        .first()
        .and_then(JsValue::as_object)
        .and_then(|e| e.prototype())
    {
        set_global(context, "__zw_event_proto", proto.into());
    }
    Ok(JsValue::undefined())
}

pub(crate) fn install_key_probe(context: &mut Context) {
    let window = context.global_object();
    let add = get(&window, "addEventListener", context);
    let Some(add) = add.as_callable() else {
        return;
    };
    let probe = function(context, "probe", 1, remember_event_proto);
    let options = object(context);
    value(&options, context, "once", JsValue::from(true));
    let _ = add.call(
        &window.clone().into(),
        &[js("keyup"), probe.into(), options.into()],
        context,
    );
    // `window.event`（引数を取らず `event.keyCode` を読む書き方のため）。
    for kind in ["keydown", "keyup", "keypress"] {
        let remember = function(context, "remember", 1, remember_current_event);
        let capture = object(context);
        value(&capture, context, "capture", JsValue::from(true));
        let _ = add.call(
            &window.clone().into(),
            &[js(kind), remember.into(), capture.into()],
            context,
        );
    }
}

fn remember_current_event(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let event = args.first().cloned().unwrap_or(JsValue::undefined());
    set_global(context, "event", event);
    Ok(JsValue::undefined())
}

pub(crate) fn define_key_code(context: &mut Context) -> bool {
    let Some(proto) = global(context, "__zw_event_proto") else {
        return false;
    };
    if !get(&proto, "__zw_keyed", context).is_undefined() {
        return true;
    }
    accessor(&proto, context, "keyCode", Some(key_code), None);
    accessor(&proto, context, "which", Some(key_code), None);
    value(&proto, context, "__zw_keyed", JsValue::from(true));
    true
}

fn document_location(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(global(context, "location").map_or(JsValue::undefined(), JsValue::from))
}

#[cfg(test)]
mod audio_tests {
    use super::aiff_duration_ms;

    #[test]
    fn the_duration_comes_from_the_comm_chunk() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"FORM");
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(b"AIFC");
        bytes.extend_from_slice(b"FVER");
        bytes.extend_from_slice(&4u32.to_be_bytes());
        bytes.extend_from_slice(&[0xa2, 0x80, 0x51, 0x40]);
        bytes.extend_from_slice(b"COMM");
        bytes.extend_from_slice(&22u32.to_be_bytes());
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(&96_000u32.to_be_bytes());
        bytes.extend_from_slice(&16u16.to_be_bytes());
        bytes.extend_from_slice(&[0x40, 0x0e, 0xbb, 0x80, 0, 0, 0, 0, 0, 0]);
        bytes.extend_from_slice(b"NONE");
        assert_eq!(aiff_duration_ms(&bytes), Some(2000));
        assert_eq!(aiff_duration_ms(b"RIFF...."), None);
    }
}

#[cfg(test)]
mod event_tests {
    use super::*;

    #[test]
    fn create_event_gives_something_with_a_time_stamp() {
        let mut context = Context::default();
        assert!(eval(
            &mut context,
            "globalThis.document = {}; globalThis.location = { href: \"http://a/\" };",
            "(setup)"
        ));
        install(&mut context);
        super::super::timers::install(&mut context);
        let code = r##"
            var e = document.createEvent("Event");
            e.initEvent("resize", true, false);
            [typeof e.timeStamp, e.type, performance.now() >= 0].join("|")
        "##;
        let got = context
            .eval(Source::from_bytes(code.as_bytes()))
            .expect("走る")
            .to_string(&mut context)
            .expect("文字に")
            .to_std_string_escaped();
        assert_eq!(got, "number|resize|true");
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    #[test]
    fn push_state_rewrites_the_address_without_moving() {
        let mut context = Context::default();
        let setup = r##"
            globalThis.document = {};
            globalThis.location = { href: "http://a.example:8080/x/y.html?q#h",
                protocol: "http:", host: "a.example:8080", hostname: "a.example",
                pathname: "/x/y.html", search: "?q", hash: "#h" };
        "##;
        assert!(eval(&mut context, setup, "(setup)"));
        install(&mut context);
        let code = r##"
            history.pushState({ n: 1 }, "", "../z?k=v#top");
            var a = [location.href, location.protocol, location.host, location.hostname,
                location.port, location.pathname, location.search, location.hash,
                location.origin, history.state.n, document.baseURI].join("|");
            history.replaceState(null, "", null);
            a + "|" + location.href + "|" + history.state + "|" + history.length
        "##;
        let got = context
            .eval(Source::from_bytes(code.as_bytes()))
            .expect("走る")
            .to_string(&mut context)
            .expect("文字に")
            .to_std_string_escaped();
        assert_eq!(
            got,
            "http://a.example:8080/z?k=v#top|http:|a.example:8080|a.example|8080|/z|?k=v|#top\
             |http://a.example:8080|1|http://a.example:8080/z?k=v#top\
             |http://a.example:8080/z?k=v#top|null|1"
        );
    }
}
