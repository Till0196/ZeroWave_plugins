// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 受像機の API（ARIB STD-B62 第二分冊 3.3、運用は TR-B39 第 3 編 10.7）と、Blitz の DOM に無い
//! ブラウザの API を boa に生やす。文書のリクエストは [`Request`] に積み、スレッドの側が取り出す。

pub mod canvas;
mod cookies;
pub mod date;
pub(crate) mod dom;
pub mod receiver;
mod storage;
mod timers;
mod xhr;
mod xml;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use boa_engine::object::FunctionObjectBuilder;
use boa_engine::object::builtins::JsFunction;
use boa_engine::property::{Attribute, PropertyDescriptor, PropertyKey};
use boa_engine::{Context, JsObject, JsResult, JsString, JsValue, NativeFunction};

use crate::net::Shared;

/// いまのアプリが Hybridcast（AIT で起動した）か。違えば MH-AIT の A-PAB。
static HYBRIDCAST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn agent() -> bml::Agent {
    if HYBRIDCAST.load(std::sync::atomic::Ordering::Relaxed) {
        bml::Agent::Hybridcast
    } else {
        bml::Agent::Apab
    }
}

pub(crate) fn name_as(host: &Host) {
    HYBRIDCAST.store(host.profile == "hc", std::sync::atomic::Ordering::Relaxed);
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Host {
    pub network: u16,
    pub tlv_stream: u16,
    pub service: u16,
    pub event: Option<u16>,
    pub start: Option<i64>,
    pub duration: Option<u32>,
    pub name: String,
    pub description: String,
    pub zipcode: String,
    /// 受像機の識別子（十六進 12 桁）。空なら `getIRDID` は null。
    pub cas_id: String,
    pub profile: String,
    pub captions: Vec<Caption>,
    pub next: Option<(u16, Option<i64>, Option<u32>, String)>,
}

/// 放送の字幕のコンポーネント（MMT の `stpp` のアセット）。文書は `arib2://-1.-1.-1/<tag>` で指す。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Caption {
    pub tag: u16,
    pub language: String,
    pub kind: u8,
    /// 時刻制御モード（STD-B60 表 9-7）。
    pub tmd: u8,
    /// 表示解像度（表 9-9）。
    pub resolution: u8,
    /// 圧縮（表 9-10）: 0 なし、1 zlib。
    pub compression: u8,
    /// 起点の時刻（NTP の 64 ビット）。`tmd` が 0b0010 のとき。
    pub reference_start: Option<u64>,
}

impl Caption {
    pub fn url(&self) -> String {
        format!("arib2://-1.-1.-1/{:04x}", self.tag)
    }
}

pub(crate) fn caption_tag_of(url: &str) -> Option<u16> {
    let tail = url.rsplit('/').next()?;
    let tail = tail.split(';').next()?;
    u16::from_str_radix(tail, 16).ok()
}

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Log(String),
    Sound(u32),
    Audio(Vec<u8>),
    AudioStop,
    Tune {
        network: u16,
        tlv_stream: u16,
        service: u16,
    },
    /// アプリを終える。`launchDataBroadcastingBrowser` なら戻る先のコンポーネントタグ（無ければエントリーコンポーネント）。
    Quit {
        component: Option<u16>,
    },
    Navigate(String),
    Captions(bool),
}

#[derive(Debug, Default)]
pub struct Bridge {
    pub host: Host,
    pub requests: Vec<Request>,
    /// アプリが受け取るキーのグループ（`keySet.value`）。`setValue` を呼ぶまでは受像機の既定。
    pub key_set: Option<u32>,
    pub fullscreen: bool,
    pub mute: bool,
    pub caption_visible: bool,
}

impl Bridge {
    pub fn key_set_or_default(&self) -> u32 {
        self.key_set.unwrap_or(receiver::DEFAULT_KEY_SET)
    }
}

pub type SharedBridge = Rc<RefCell<Bridge>>;

/// 受像機に残るもの。閉じても選局しても消えない（ファイルには残さない）。局のアプリは
/// 領域が無いと初期化を何度も繰り返すので、開き直すたびに消してはいけない。
#[derive(Debug, Default)]
pub struct Kept {
    /// localStorage。局（ネットワークとサービス）ごとに分けたキーで持つ
    /// （storage の `scoped`）。`ureg0`〜`ureg63`、`greg0`〜`greg63` は全局で一つ。
    pub storage: BTreeMap<String, String>,
    pub persistent: BTreeMap<String, String>,
    /// cookie。期限は見ず、「過ぎた期限」を指定されたときだけ消す。
    pub cookies: cookies::Jar,
}

static KEPT: std::sync::Mutex<Kept> = std::sync::Mutex::new(Kept {
    storage: BTreeMap::new(),
    persistent: BTreeMap::new(),
    cookies: cookies::Jar::new(),
});

pub fn kept<R>(f: impl FnOnce(&mut Kept) -> R) -> R {
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut kept)
}

thread_local! {
    /// boa の native 関数は `fn` ポインタなので、渡す物はここで持つ。
    static BRIDGE: RefCell<Option<(SharedBridge, Shared)>> = const { RefCell::new(None) };
}

pub(crate) fn with_bridge<R>(f: impl FnOnce(&SharedBridge, &Shared) -> R) -> Option<R> {
    BRIDGE.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|(bridge, store)| f(bridge, store))
    })
}

pub(crate) fn request(wanted: Request) {
    with_bridge(|bridge, _| bridge.borrow_mut().requests.push(wanted));
}

pub fn install(context: &mut Context, bridge: SharedBridge, store: Shared) {
    bml::runtime::clock::limit(context);
    BRIDGE.with(|slot| *slot.borrow_mut() = Some((bridge, store)));
    receiver::install(context);
    storage::install(context);
    xhr::install(context);
    date::install(context);
    timers::install(context);
    xml::install(context);
    dom::install(context);
    dom::install_key_probe(context);
    let log = function(context, "__zw_log", 1, log_request);
    set_global(context, "__zw_log", log.into());
    let setlen = function(context, bml::runtime::script::SETLEN, 2, set_length);
    set_global(context, bml::runtime::script::SETLEN, setlen.into());
}

pub(crate) fn string(value: Option<&JsValue>, context: &mut Context) -> String {
    value
        .filter(|v| !v.is_undefined() && !v.is_null())
        .and_then(|v| v.to_string(context).ok())
        .map(|s| s.to_std_string_escaped())
        .unwrap_or_default()
}

pub(crate) fn js(text: &str) -> JsValue {
    JsValue::from(JsString::from(text))
}

pub(crate) fn number(value: Option<&JsValue>, context: &mut Context) -> f64 {
    value
        .and_then(|v| v.to_number(context).ok())
        .unwrap_or(f64::NAN)
}

pub(crate) type Native = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

pub(crate) fn function(
    context: &mut Context,
    name: &str,
    length: usize,
    body: Native,
) -> JsFunction {
    FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(body))
        .name(JsString::from(name))
        .length(length)
        .build()
}

pub(crate) fn constructor(
    context: &mut Context,
    name: &str,
    length: usize,
    body: Native,
) -> JsFunction {
    FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(body))
        .name(JsString::from(name))
        .length(length)
        .constructor(true)
        .build()
}

pub(crate) fn method(
    object: &JsObject,
    context: &mut Context,
    name: &str,
    length: usize,
    body: Native,
) {
    let f = function(context, name, length, body);
    let _ = object.define_property_or_throw(
        PropertyKey::from(JsString::from(name)),
        PropertyDescriptor::builder()
            .value(f)
            .writable(true)
            .enumerable(false)
            .configurable(true)
            .build(),
        context,
    );
}

pub(crate) fn value(object: &JsObject, context: &mut Context, name: &str, v: JsValue) {
    let _ = object.define_property_or_throw(
        PropertyKey::from(JsString::from(name)),
        PropertyDescriptor::builder()
            .value(v)
            .writable(true)
            .enumerable(true)
            .configurable(true)
            .build(),
        context,
    );
}

pub(crate) fn accessor(
    object: &JsObject,
    context: &mut Context,
    name: &str,
    get: Option<Native>,
    set: Option<Native>,
) {
    let getter = get.map(|g| function(context, &format!("get {name}"), 0, g));
    let setter = set.map(|s| function(context, &format!("set {name}"), 1, s));
    let _ = object.define_property_or_throw(
        PropertyKey::from(JsString::from(name)),
        PropertyDescriptor::builder()
            .maybe_get(getter)
            .maybe_set(setter)
            .enumerable(false)
            .configurable(true)
            .build(),
        context,
    );
}

pub(crate) fn object(context: &mut Context) -> JsObject {
    JsObject::with_object_proto(context.intrinsics())
}

pub(crate) fn global(context: &mut Context, name: &str) -> Option<JsObject> {
    context
        .global_object()
        .get(PropertyKey::from(JsString::from(name)), context)
        .ok()
        .and_then(|v| v.as_object())
}

pub(crate) fn set_global(context: &mut Context, name: &str, v: JsValue) {
    let _ = context.register_global_property(
        JsString::from(name),
        v,
        Attribute::WRITABLE.union(Attribute::CONFIGURABLE),
    );
}

/// `f` を次の機会に呼ぶ（`setTimeout(f, 0)`）。呼んだ側が準備の途中のことがあるので同期で呼ばない。
pub(crate) fn later(context: &mut Context, f: JsValue, args: Vec<JsValue>) {
    let Some(set_timeout) = global(context, "setTimeout") else {
        return;
    };
    let mut call_args = vec![f, JsValue::from(0)];
    call_args.extend(args);
    let _ = set_timeout.call(&JsValue::undefined(), &call_args, context);
}

pub(crate) fn this_object(this: &JsValue) -> JsResult<JsObject> {
    this.as_object().ok_or_else(|| {
        boa_engine::JsNativeError::typ()
            .with_message("オブジェクトではない")
            .into()
    })
}

pub(crate) fn get(object: &JsObject, name: &str, context: &mut Context) -> JsValue {
    object
        .get(PropertyKey::from(JsString::from(name)), context)
        .unwrap_or(JsValue::undefined())
}

pub(crate) fn set(object: &JsObject, name: &str, v: JsValue, context: &mut Context) {
    let _ = object.set(PropertyKey::from(JsString::from(name)), v, false, context);
}

pub(crate) fn host_json_number(v: Option<i64>) -> JsValue {
    v.map_or(JsValue::null(), JsValue::from)
}

pub(crate) fn host_object(context: &mut Context) -> JsObject {
    let h = with_bridge(|bridge, _| bridge.borrow().host.clone()).unwrap_or_default();
    let o = object(context);
    value(&o, context, "network", JsValue::from(h.network));
    value(&o, context, "tlv_stream", JsValue::from(h.tlv_stream));
    value(&o, context, "service", JsValue::from(h.service));
    value(
        &o,
        context,
        "event",
        host_json_number(h.event.map(i64::from)),
    );
    value(&o, context, "start", host_json_number(h.start));
    value(
        &o,
        context,
        "duration",
        host_json_number(h.duration.map(i64::from)),
    );
    value(&o, context, "name", js(&h.name));
    value(&o, context, "desc", js(&h.description));
    value(&o, context, "zipcode", js(&h.zipcode));
    value(&o, context, "profile", js(&h.profile));
    value(
        &o,
        context,
        "captions",
        JsValue::from(!h.captions.is_empty()),
    );
    match &h.next {
        Some((event, start, duration, name)) => {
            let n = object(context);
            value(&n, context, "event", JsValue::from(*event));
            value(&n, context, "start", host_json_number(*start));
            value(
                &n,
                context,
                "duration",
                host_json_number(duration.map(i64::from)),
            );
            value(&n, context, "name", js(name));
            value(&o, context, "next", n.into());
        }
        None => value(&o, context, "next", JsValue::null()),
    }
    o
}

pub(crate) fn date(context: &mut Context, seconds: Option<i64>) -> JsValue {
    let Some(seconds) = seconds else {
        return JsValue::null();
    };
    let Some(date) = global(context, "Date") else {
        return JsValue::null();
    };
    date.construct(&[JsValue::from(seconds as f64 * 1000.0)], None, context)
        .map(JsValue::from)
        .unwrap_or(JsValue::null())
}

/// `__bml_setlen(a, n)`: native の `set` は boa のインラインキャッシュに乗らないので、
/// `length` の特別な代入が二度目も走る（[`crate::script`]）。`n` を返す。
fn set_length(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let value = args.get(1).cloned().unwrap_or_default();
    if let Some(object) = args.first().and_then(JsValue::as_object) {
        object.set(
            boa_engine::js_string!("length"),
            value.clone(),
            false,
            context,
        )?;
    }
    Ok(value)
}

fn log_request(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let text = string(args.first(), context);
    request(Request::Log(format!("eval: {text}")));
    Ok(JsValue::undefined())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caption_tag_is_read_from_the_url() {
        assert_eq!(caption_tag_of("arib2://-1.-1.-1/0030"), Some(0x30));
        assert_eq!(caption_tag_of("arib://-1.-1.-1/-1;0"), None);
        assert_eq!(caption_tag_of(""), None);
        assert_eq!(
            Caption {
                tag: 0x31,
                ..Default::default()
            }
            .url(),
            "arib2://-1.-1.-1/0031"
        );
    }
}
