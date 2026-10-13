// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! `navigator` に生やすもの（`receiverDevice`、`applicationManager`、`capabilities`、
//! `bmlCompat.browserPseudo`、`userAgent`）と、`VK_*`（TR-B39 第 3 編 図 10-8）とキーのグループ。

use boa_engine::object::builtins::JsArray;
use boa_engine::{Context, JsObject, JsResult, JsValue, NativeFunction, js_string};

use super::{
    Request, accessor, date, get, global, host_object, js, later, method, number, object, request,
    set_global, string, value, with_bridge,
};

/// キーの番号。値は受像機ごとで、CEA-2014/HbbTV の慣例に合わせる。
pub const KEYS: &[(&str, u16)] = &[
    ("VK_RED", 403),
    ("VK_GREEN", 404),
    ("VK_YELLOW", 405),
    ("VK_BLUE", 406),
    ("VK_UP", 38),
    ("VK_DOWN", 40),
    ("VK_LEFT", 37),
    ("VK_RIGHT", 39),
    ("VK_ENTER", 13),
    ("VK_BACK", 461),
    ("VK_0", 48),
    ("VK_1", 49),
    ("VK_2", 50),
    ("VK_3", 51),
    ("VK_4", 52),
    ("VK_5", 53),
    ("VK_6", 54),
    ("VK_7", 55),
    ("VK_8", 56),
    ("VK_9", 57),
    ("VK_10", 1010),
    ("VK_11", 1011),
    ("VK_12", 1012),
    ("VK_DBUTTON", 1001),
    ("VK_SUBTITLE", 460),
    ("VK_PLAY_PAUSE", 463),
    ("VK_PLAY", 415),
    ("VK_PAUSE", 19),
    ("VK_STOP", 413),
    ("VK_FAST_FWD", 417),
    ("VK_REWIND", 412),
    ("VK_TRACK_NEXT", 425),
    ("VK_TRACK_PREV", 424),
    ("VK_VCR_OTHER", 1020),
    ("VK_PAGE_UP", 33),
    ("VK_PAGE_DOWN", 34),
    ("VK_TAB", 9),
];

/// キーのグループ（TR-B39 第 3 編 図 10-9）。
const KEY_GROUPS: &[(&str, u32)] = &[
    ("RED", 1),
    ("GREEN", 2),
    ("YELLOW", 4),
    ("BLUE", 8),
    ("NAVIGATION", 16),
    ("NUMERIC", 32),
    ("VCR", 64),
    ("DBUTTON", 128),
    ("SUBTITLE", 256),
];

/// 起動直後の `keySet`：移動、色ボタン、d ボタン（数字は選局に取られるので含まない）。
pub const DEFAULT_KEY_SET: u32 = 1 | 2 | 4 | 8 | 16 | 128;

pub fn key_set() -> u32 {
    with_bridge(|bridge, _| bridge.borrow().key_set_or_default()).unwrap_or(DEFAULT_KEY_SET)
}

/// キーの番号が属するグループ。このグループが `keySet` に無ければ受像機が自分で扱う。
pub fn key_group(code: u16) -> u32 {
    match code {
        403 => 1,
        404 => 2,
        405 => 4,
        406 => 8,
        37..=40 | 13 | 461 | 33 | 34 | 9 => 16,
        48..=57 => 32,
        1001 => 128,
        460 => 256,
        _ => 0,
    }
}

pub(super) fn install(context: &mut Context) {
    for (name, code) in KEYS {
        set_global(context, name, JsValue::from(*code));
    }
    let Some(navigator) = global(context, "navigator") else {
        return;
    };
    // UA は設定に入っているときだけ。Hybridcast と A-PAB で別の設定（[`super::agent`]）。
    value(
        &navigator,
        context,
        "userAgent",
        js(&bml::user_agent(super::agent()).unwrap_or_default()),
    );
    let device = receiver_device(context);
    value(&navigator, context, "receiverDevice", device.into());
    let capabilities = capabilities(context);
    value(&navigator, context, "capabilities", capabilities.into());
    let manager = application_manager(context);
    value(&navigator, context, "applicationManager", manager.into());
    let compat = bml_compat(context);
    value(&navigator, context, "bmlCompat", compat.into());
}

fn receiver_device(context: &mut Context) -> JsObject {
    let d = object(context);
    method(&d, context, "getDeviceIdentifier", 2, get_device_identifier);
    method(
        &d,
        context,
        "getSystemInformation",
        1,
        get_system_information,
    );
    method(&d, context, "tuneTo", 3, tune_to);
    method(
        &d,
        context,
        "getCurrentEventInformation",
        1,
        get_current_event_information,
    );
    method(&d, context, "getZipCode", 0, get_zip_code);
    method(&d, context, "getCASModuleStatus", 0, zero);
    method(&d, context, "confirmIPNetwork", 3, confirm_ip_network);
    method(&d, context, "isIPConnected", 0, is_ip_connected);
    // Hybridcast（IPTVFJ）のコンパニオン機器と字幕・音声のインターフェース。機器は繋がっていない。
    for (name, length) in [
        ("addCompanionDeviceTextMessageListener", 1),
        ("removeCompanionDeviceTextMessageListener", 0),
        ("addCompanionDeviceConnectionListener", 1),
        ("removeCompanionDeviceConnectionListener", 0),
        ("addCompanionDeviceDisconnectionListener", 1),
        ("removeCompanionDeviceDisconnectionListener", 0),
        ("sendTextToCompanionDevice", 2),
        ("startCompanionDeviceDiscovery", 0),
        ("stopCompanionDeviceDiscovery", 0),
        ("setFullScreen", 1),
        ("setSubtitleDisplayStatus", 1),
        ("addSubtitleDisplayStatusListener", 1),
        ("removeSubtitleDisplayStatusListener", 0),
    ] {
        method(&d, context, name, length, noop);
    }
    method(
        &d,
        context,
        "getConnectedCompanionDeviceList",
        0,
        empty_array,
    );
    method(&d, context, "getCompanionDeviceList", 0, empty_array);
    method(&d, context, "getSubtitleDisplayStatus", 0, zero);
    let stream = object(context);
    for (name, length) in [
        ("addEventIDUpdateListener", 1),
        ("removeEventIDUpdateListener", 1),
        ("addNPTReferenceListener", 2),
        ("removeNPTReferenceListener", 2),
    ] {
        method(&stream, context, name, length, noop);
    }
    // 汎用イベントメッセージ（TR-B39 10.7.11、表 10-66〜68）。番組連動はこれで発生する。
    method(
        &stream,
        context,
        "addGeneralEventMessageListener",
        2,
        add_event_message_listener,
    );
    method(
        &stream,
        context,
        "removeGeneralEventMessageListener",
        2,
        remove_event_message_listener,
    );
    method(&stream, context, "sourceIs", 1, source_is);
    value(&d, context, "streamEvent", stream.into());
    let cache = object(context);
    method(&cache, context, "storeDataResource", 2, add_cache_listener);
    method(
        &cache,
        context,
        "addCacheEventListener",
        2,
        add_cache_listener,
    );
    method(
        &cache,
        context,
        "releaseDataResource",
        1,
        remove_cache_listener,
    );
    method(
        &cache,
        context,
        "removeCacheEventListener",
        1,
        remove_cache_listener,
    );
    value(&d, context, "cacheEvent", cache.into());
    d
}

fn noop(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

const EVENT_MESSAGE_LISTENERS: &str = "__zw_event_message_listeners";
const CACHE_LISTENERS: &str = "__zw_cache_listeners";

fn listeners(context: &mut Context, name: &str) -> JsResult<JsArray> {
    if let Some(list) = global(context, name).and_then(|o| JsArray::from_object(o).ok()) {
        return Ok(list);
    }
    let list = JsArray::new(context)?;
    let as_value: JsValue = list.clone().into();
    set_global(context, name, as_value);
    Ok(list)
}

/// `param` の `source.event_message_tag` と `message_id`（省けば -1、STD-B62 第二編 3.3.12）。
fn event_message_param(param: Option<&JsValue>, context: &mut Context) -> Option<(f64, f64)> {
    let param = param?.as_object()?;
    let source = param.get(js_string!("source"), context).ok()?;
    let tag = source
        .as_object()?
        .get(js_string!("event_message_tag"), context)
        .ok()?
        .as_number()?;
    let id = param
        .get(js_string!("message_id"), context)
        .ok()
        .and_then(|v| v.as_number())
        .unwrap_or(-1.0);
    Some((tag, id))
}

fn add_event_message_listener(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some((tag, id)) = event_message_param(args.first(), context) else {
        return Err(boa_engine::JsNativeError::typ()
            .with_message("INVALID_PARAM_ERR")
            .into());
    };
    let Some(f) = args.get(1).filter(|v| v.is_callable()).cloned() else {
        return Err(boa_engine::JsNativeError::typ()
            .with_message("INVALID_PARAM_ERR")
            .into());
    };
    // バージョンはリスナーごと。付けたばかりのリスナーには、繰り返し送られる現行のメッセージも
    // 初めてとして発生する（TR-B39 11.2.4 ③）。
    let entry: JsValue = JsArray::from_iter(
        [JsValue::from(tag), JsValue::from(id), f, JsValue::from(-1)],
        context,
    )
    .into();
    let _ = listeners(context, EVENT_MESSAGE_LISTENERS)?.push(entry, context);
    Ok(JsValue::undefined())
}

/// いまリスナーの付いている EMT の名前。受像機はこれだけを監視する（TR-B39 11.2.4 ①②④）。
pub(crate) fn watched_emts(context: &mut Context) -> Vec<u8> {
    let Ok(list) = listeners(context, EVENT_MESSAGE_LISTENERS) else {
        return Vec::new();
    };
    let length = list.length(context).unwrap_or(0);
    let mut tags = Vec::new();
    for i in 0..length {
        let Ok(entry) = list.get(i, context) else {
            continue;
        };
        if let Some(tag) = entry
            .as_object()
            .and_then(|o| o.get(0, context).ok())
            .and_then(|v| v.as_number())
            && (0.0..=255.0).contains(&tag)
            && !tags.contains(&(tag as u8))
        {
            tags.push(tag as u8);
        }
    }
    tags
}

fn message_object(
    context: &mut Context,
    tag: u8,
    message_id: u8,
    version: u8,
    private: &str,
) -> JsObject {
    let source = object(context);
    value(&source, context, "event_message_tag", JsValue::from(tag));
    let message = object(context);
    value(&message, context, "source", source.into());
    value(&message, context, "message_group_id", JsValue::from(1));
    value(&message, context, "message_id", JsValue::from(message_id));
    value(&message, context, "message_version", JsValue::from(version));
    value(&message, context, "private_data_byte", js(private));
    message
}

fn remove_event_message_listener(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some((tag, id)) = event_message_param(args.first(), context) else {
        return Ok(JsValue::undefined());
    };
    let list = listeners(context, EVENT_MESSAGE_LISTENERS)?;
    let length = list.length(context).unwrap_or(0);
    let mut kept = Vec::new();
    for i in 0..length {
        let Ok(entry) = list.get(i, context) else {
            continue;
        };
        let matches = entry.as_object().is_some_and(|o| {
            o.get(0, context).ok().and_then(|v| v.as_number()) == Some(tag)
                && o.get(1, context).ok().and_then(|v| v.as_number()) == Some(id)
        });
        if !matches {
            kept.push(entry);
        }
    }
    let fresh = JsArray::from_iter(kept, context);
    set_global(context, EVENT_MESSAGE_LISTENERS, fresh.into());
    Ok(JsValue::undefined())
}

pub(crate) fn event_message(
    context: &mut Context,
    tag: u8,
    message_id: u8,
    version: u8,
    private: &str,
) -> usize {
    let Ok(list) = listeners(context, EVENT_MESSAGE_LISTENERS) else {
        return 0;
    };
    let length = list.length(context).unwrap_or(0);
    let mut called = 0;
    for i in 0..length {
        let Ok(entry) = list.get(i, context) else {
            continue;
        };
        let Some(entry) = entry.as_object() else {
            continue;
        };
        let wanted_tag = entry.get(0, context).ok().and_then(|v| v.as_number());
        let wanted_id = entry.get(1, context).ok().and_then(|v| v.as_number());
        if wanted_tag != Some(f64::from(tag)) {
            continue;
        }
        if wanted_id.is_some_and(|id| id >= 0.0 && id != f64::from(message_id)) {
            continue;
        }
        // 同じバージョンはこのリスナーには一度だけ。放送は取りこぼし防止に繰り返す。
        let last = entry.get(3, context).ok().and_then(|v| v.as_number());
        if last == Some(f64::from(version)) {
            continue;
        }
        let _ = entry.set(3, JsValue::from(version), false, context);
        let Ok(f) = entry.get(2, context) else {
            continue;
        };
        let message = message_object(context, tag, message_id, version, private);
        later(context, f, vec![message.into()]);
        called += 1;
    }
    called
}

fn add_cache_listener(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let path = string(args.first(), context);
    if path.is_empty() {
        return Err(boa_engine::JsNativeError::typ()
            .with_message("INVALID_PARAM_ERR")
            .into());
    }
    let f = args
        .get(1)
        .filter(|v| v.is_callable())
        .cloned()
        .unwrap_or_default();
    let entry: JsValue =
        JsArray::from_iter([js(&path), f, js(""), JsValue::from(false)], context).into();
    let _ = listeners(context, CACHE_LISTENERS)?.push(entry, context);
    Ok(JsValue::undefined())
}

fn remove_cache_listener(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let path = args.first().map(|p| string(Some(p), context));
    let list = listeners(context, CACHE_LISTENERS)?;
    let length = list.length(context).unwrap_or(0);
    let mut kept = Vec::new();
    for i in 0..length {
        let Ok(entry) = list.get(i, context) else {
            continue;
        };
        let this = entry
            .as_object()
            .and_then(|o| o.get(0, context).ok())
            .map(|v| string(Some(&v), context));
        if let Some(path) = &path
            && this.as_deref() != Some(path.as_str())
        {
            kept.push(entry);
        }
    }
    let fresh = JsArray::from_iter(kept, context);
    set_global(context, CACHE_LISTENERS, fresh.into());
    Ok(JsValue::undefined())
}

/// 蓄積の要求にストアの状態で応答する（TR-B39 10.7.14、11.2.3）：初めて揃えば `store_finished`、
/// MPU が替われば `updated`（揃い直せば再び `store_finished`）、ディレクトリが消えれば `deleted`。
pub(crate) fn answer_cache_listeners(
    context: &mut Context,
    state: &dyn Fn(&str) -> Option<crate::store::DirectoryState>,
) -> bool {
    let Ok(list) = listeners(context, CACHE_LISTENERS) else {
        return false;
    };
    let length = list.length(context).unwrap_or(0);
    let mut any = false;
    for i in 0..length {
        let Ok(entry) = list.get(i, context) else {
            continue;
        };
        let Some(entry) = entry.as_object() else {
            continue;
        };
        let Ok(path) = entry.get(0, context) else {
            continue;
        };
        let path = string(Some(&path), context);
        let inside = path
            .strip_prefix(crate::net::BASE)
            .map(str::to_owned)
            .unwrap_or_else(|| path.clone());
        let seen = entry
            .get(2, context)
            .ok()
            .map(|v| string(Some(&v), context))
            .unwrap_or_default();
        let finished = entry.get(3, context).ok().and_then(|v| v.as_boolean()) == Some(true);
        let f = entry.get(1, context).ok().filter(|f| f.is_callable());
        let mut say = |event: &str, context: &mut Context| {
            if let Some(f) = &f {
                later(context, f.clone(), vec![js(&path), js(event)]);
                any = true;
            }
        };
        match state(&inside) {
            None => {
                if !seen.is_empty() && inside.ends_with('/') {
                    say("deleted", context);
                    let _ = entry.set(2, js(""), false, context);
                    let _ = entry.set(3, JsValue::from(false), false, context);
                }
            }
            Some(now) => {
                let mpus = now
                    .mpus
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                if !seen.is_empty() && seen != mpus {
                    say("updated", context);
                    let _ = entry.set(2, js(&mpus), false, context);
                    let _ = entry.set(3, JsValue::from(false), false, context);
                    continue;
                }
                if seen.is_empty() {
                    let _ = entry.set(2, js(&mpus), false, context);
                }
                if now.complete && !finished {
                    let _ = entry.set(3, JsValue::from(true), false, context);
                    say("store_finished", context);
                }
            }
        }
    }
    any
}

fn zero(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(0))
}

/// `getDeviceIdentifier(type, cb)`。種別 5 は BML の `getIRDID(5)` と同じ形。
fn get_device_identifier(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let kind = args
        .first()
        .and_then(|v| v.to_number(context).ok())
        .unwrap_or(f64::NAN);
    let cas_id = with_bridge(|bridge, _| bridge.borrow().host.cas_id.clone()).unwrap_or_default();
    let answer = if kind == 5.0 && !cas_id.is_empty() {
        js(&cas_id)
    } else {
        JsValue::null()
    };
    if let Some(cb) = args.get(1).filter(|v| v.is_callable()) {
        later(context, cb.clone(), vec![answer]);
    }
    Ok(JsValue::undefined())
}

/// `getSystemInformation(null)` → TR-B39 第 3 編 表 10-61。
fn get_system_information(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let host = host_object(context);
    let o = object(context);
    value(&o, context, "makerid", js(bml::maker_id()));
    value(&o, context, "browsername", js(&bml::browser_name()));
    value(
        &o,
        context,
        "browserversion",
        js(&bml::browser_version_text()),
    );
    value(&o, context, "modelname", js(bml::model_id()));
    value(&o, context, "baseurl", js(crate::net::BASE));
    let profile = get(&host, "profile", context);
    value(&o, context, "profile", profile);
    Ok(o.into())
}

fn tune_to(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let host = with_bridge(|bridge, _| bridge.borrow().host.clone()).unwrap_or_default();
    let field = |name: &str, fallback: u16, context: &mut Context| -> u16 {
        args.first()
            .and_then(JsValue::as_object)
            .map(|r| get(&r, name, context))
            .filter(|v| !v.is_undefined() && !v.is_null())
            .and_then(|v| v.to_number(context).ok())
            .map_or(fallback, |n| n as u16)
    };
    let network = field("original_network_id", host.network, context);
    let tlv_stream = field("tlv_stream_id", host.tlv_stream, context);
    let service = field("service_id", host.service, context);
    request(Request::Tune {
        network,
        tlv_stream,
        service,
    });
    Ok(JsValue::undefined())
}

/// `getCurrentEventInformation(cb)` → TR-B39 第 3 編 表 10-64。duration はミリ秒。
fn get_current_event_information(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(cb) = args.first().filter(|v| v.is_callable()).cloned() else {
        return Ok(JsValue::undefined());
    };
    let h = with_bridge(|bridge, _| bridge.borrow().host.clone()).unwrap_or_default();
    let info = object(context);
    value(
        &info,
        context,
        "original_network_id",
        JsValue::from(h.network),
    );
    value(&info, context, "tlv_stream_id", JsValue::from(h.tlv_stream));
    value(&info, context, "service_id", JsValue::from(h.service));
    value(
        &info,
        context,
        "event_id",
        h.event.map_or(JsValue::null(), JsValue::from),
    );
    let start = date(context, h.start);
    value(&info, context, "start_time", start);
    value(
        &info,
        context,
        "duration",
        h.duration
            .map_or(JsValue::null(), |d| JsValue::from(f64::from(d) * 1000.0)),
    );
    value(&info, context, "free_ca_mode", JsValue::from(false));
    value(&info, context, "name", js(&h.name));
    value(&info, context, "desc", js(&h.description));
    match &h.next {
        Some((event, start, duration, name)) => {
            value(&info, context, "f_event_id", JsValue::from(*event));
            let start = date(context, *start);
            value(&info, context, "f_start_time", start);
            value(
                &info,
                context,
                "f_duration",
                duration.map_or(JsValue::null(), |d| JsValue::from(f64::from(d) * 1000.0)),
            );
            value(&info, context, "f_free_ca_mode", JsValue::from(false));
            value(&info, context, "f_name", js(name));
            value(&info, context, "f_desc", js(""));
        }
        None => {
            for name in [
                "f_event_id",
                "f_start_time",
                "f_duration",
                "f_free_ca_mode",
                "f_name",
                "f_desc",
            ] {
                value(&info, context, name, JsValue::null());
            }
        }
    }
    later(context, cb, vec![info.into()]);
    Ok(JsValue::undefined())
}

fn ird_id(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let cas_id = with_bridge(|bridge, _| bridge.borrow().host.cas_id.clone()).unwrap_or_default();
    if cas_id.is_empty() {
        return Ok(JsValue::null());
    }
    Ok(js(&cas_id))
}

fn get_zip_code(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let zip = with_bridge(|bridge, _| bridge.borrow().host.zipcode.clone()).unwrap_or_default();
    Ok(js(&zip))
}

fn capabilities(context: &mut Context) -> JsObject {
    let c = object(context);
    method(&c, context, "hasCapability", 1, has_capability);
    c
}

fn has_capability(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(false))
}

fn application_manager(context: &mut Context) -> JsObject {
    let app = object(context);
    value(&app, context, "type", js("ARIB-HTML5"));
    value(&app, context, "organization_id", JsValue::from(0));
    value(&app, context, "application_id", JsValue::from(0));
    value(&app, context, "control_code", js("AUTOSTART"));
    value(&app, context, "autostart_priority", JsValue::from(0));
    method(&app, context, "destroyApplication", 0, destroy_application);
    method(&app, context, "replaceApplication", 3, noop);
    method(&app, context, "exitFromManagedState", 1, noop);
    method(&app, context, "getOwnerAIT", 0, get_owner_ait);
    method(
        &app,
        context,
        "getApplicationBoundaryAndPermissionDescriptor",
        0,
        null,
    );
    let key_set = object(context);
    for (name, v) in KEY_GROUPS {
        value(&key_set, context, name, JsValue::from(*v));
    }
    accessor(&key_set, context, "value", Some(key_set_value), None);
    method(&key_set, context, "setValue", 1, key_set_set_value);
    value(&app, context, "keySet", key_set.into());
    set_global(context, "__zw_owner_application", app.into());
    let manager = object(context);
    method(
        &manager,
        context,
        "getOwnerApplication",
        1,
        get_owner_application,
    );
    method(
        &manager,
        context,
        "launchDataBroadcastingBrowser",
        1,
        launch_data_broadcasting_browser,
    );
    manager
}

fn null(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::null())
}

fn empty_array(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(boa_engine::object::builtins::JsArray::new(context)?.into())
}

fn source_is(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let kind = string(args.first(), context);
    Ok(JsValue::from(kind == "onair"))
}

fn is_ip_connected(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(u32::from(bml::resource::net::has_ip())))
}

fn confirm_ip_network(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let destination = string(args.first(), context);
    let timeout = args
        .get(2)
        .and_then(|v| v.to_number(context).ok())
        .filter(|t| t.is_finite() && *t > 0.0)
        .unwrap_or(4000.0);
    let host = bml::resource::net::parse_url(&destination).map_or(destination.clone(), |u| u.host);
    let reached =
        bml::resource::net::reach(&host, 80, std::time::Duration::from_millis(timeout as u64))
            .is_some();
    Ok(JsValue::from(reached))
}

fn get_owner_application(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(global(context, "__zw_owner_application").map_or(JsValue::null(), JsValue::from))
}

fn destroy_application(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    request(Request::Log(String::from(
        "アプリが destroyApplication を呼んだ",
    )));
    request(Request::Quit { component: None });
    Ok(JsValue::undefined())
}

/// `launchDataBroadcastingBrowser({component_tag})`。戻り値が偽だと失敗と見る局があるので真を返す。
fn launch_data_broadcasting_browser(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let component = args
        .first()
        .and_then(JsValue::as_object)
        .map(|o| get(&o, "component_tag", context))
        .and_then(|v| v.to_number(context).ok())
        .filter(|n| n.is_finite() && *n >= 0.0)
        .map(|n| n as u16);
    let given = args
        .first()
        .and_then(|v| v.to_json(context).ok().flatten())
        .map_or(String::from("-"), |v| v.to_string());
    request(Request::Log(format!(
        "アプリが launchDataBroadcastingBrowser({given}) を呼んだ"
    )));
    request(Request::Quit { component });
    Ok(JsValue::from(true))
}

fn get_owner_ait(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let ait = object(context);
    method(&ait, context, "getApplications", 0, get_applications);
    Ok(ait.into())
}

fn get_applications(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let app = global(context, "__zw_owner_application").map_or(JsValue::null(), JsValue::from);
    Ok(JsArray::from_iter([app], context).into())
}

fn key_set_value(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let v = key_set();
    Ok(JsValue::from(v))
}

fn key_set_set_value(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let v = number(args.first(), context);
    let v = if v.is_finite() { v as u32 } else { 0 };
    with_bridge(|bridge, _| bridge.borrow_mut().key_set = Some(v));
    Ok(JsValue::from(v))
}

/// BML 互換（STD-B62 第二分冊 3.3.14）。`Ureg[n]`/`Greg[n]` は記憶の `ureg<n>`/`greg<n>` を見る。
fn bml_compat(context: &mut Context) -> JsObject {
    let pseudo = object(context);
    method(
        &pseudo,
        context,
        "readPersistentArray",
        2,
        read_persistent_array,
    );
    method(
        &pseudo,
        context,
        "writePersistentArray",
        3,
        write_persistent_array,
    );
    method(&pseudo, context, "getIRDID", 1, ird_id);
    for (name, prefix) in [("Ureg", "ureg"), ("Greg", "greg")] {
        let registers = object(context);
        for index in 0..64u32 {
            let key = format!("{prefix}{index}");
            let getter = NativeFunction::from_copy_closure_with_captures(
                |_, _, key: &JsValue, context| {
                    let key = string(Some(key), context);
                    let value = super::storage::read(&key);
                    if key.starts_with("greg") && zwsdk::plug::host::on("trace") {
                        zwsdk::plug::info(&format!("Greg を読んだ {key} = {value:?}"));
                    }
                    Ok(value.map_or(js(""), |v| js(&v)))
                },
                js(&key),
            );
            let setter = NativeFunction::from_copy_closure_with_captures(
                |_, args, key: &JsValue, context| {
                    let key = string(Some(key), context);
                    let v = string(args.first(), context);
                    super::storage::write(&key, &v);
                    Ok(JsValue::undefined())
                },
                js(&key),
            );
            let getter =
                boa_engine::object::FunctionObjectBuilder::new(context.realm(), getter).build();
            let setter =
                boa_engine::object::FunctionObjectBuilder::new(context.realm(), setter).build();
            let _ = registers.define_property_or_throw(
                boa_engine::property::PropertyKey::from(index),
                boa_engine::property::PropertyDescriptor::builder()
                    .get(getter)
                    .set(setter)
                    .enumerable(true)
                    .configurable(true)
                    .build(),
                context,
            );
        }
        value(&pseudo, context, name, registers.into());
    }
    let compat = object(context);
    value(&compat, context, "browserPseudo", pseudo.into());
    compat
}

fn read_persistent_array(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let structure = string(args.get(1), context);
    if let Some(item) = name.strip_prefix("nvram://receiverinfo/") {
        let v = match item {
            "zipcode" => {
                let zip = with_bridge(|bridge, _| bridge.borrow().host.zipcode.clone())
                    .unwrap_or_default();
                js(&zip)
            }
            "prefecture" | "regioncode" => JsValue::from(0),
            _ => return Ok(JsValue::null()),
        };
        return Ok(boa_engine::object::builtins::JsArray::from_iter([v], context).into());
    }
    let found = super::kept(|kept| kept.persistent.get(&name).cloned());
    let Some(json) = found else {
        // 書かれていない領域は空の並び。null だと NVRAM が壊れたと見る局がある。
        let Some((_, fields)) = bml::resource::table::structure(&format!("0,{structure}")) else {
            return Ok(JsValue::null());
        };
        let items: Vec<JsValue> = fields
            .iter()
            .map(|field| match field.kind {
                bml::resource::table::Kind::Text => js(""),
                _ => JsValue::from(0),
            })
            .collect();
        return Ok(boa_engine::object::builtins::JsArray::from_iter(items, context).into());
    };
    let parsed = global(context, "JSON")
        .and_then(|j| get(&j, "parse", context).as_callable())
        .and_then(|parse| {
            parse
                .call(&JsValue::undefined(), &[js(&json)], context)
                .ok()
        });
    Ok(parsed.unwrap_or(JsValue::null()))
}

fn write_persistent_array(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let Some(data) = args.get(2).cloned() else {
        return Ok(JsValue::nan());
    };
    let json = global(context, "JSON")
        .and_then(|j| get(&j, "stringify", context).as_callable())
        .and_then(|f| f.call(&JsValue::undefined(), &[data], context).ok())
        .map(|v| string(Some(&v), context));
    let Some(json) = json else {
        return Ok(JsValue::nan());
    };
    super::kept(|kept| {
        kept.persistent.insert(name, json);
    });
    Ok(JsValue::from(1))
}

/// 放送映像音声オブジェクトのメソッド（STD-B62 第二分冊 2.2.2）。要素はプロトタイプを分け合うので
/// `type` を見て効かせる。
pub(super) fn install_video_object(element_proto: &JsObject, context: &mut Context) {
    method(
        element_proto,
        context,
        "enableFullscreen",
        0,
        av_enable_fullscreen,
    );
    method(
        element_proto,
        context,
        "disableFullscreen",
        0,
        av_disable_fullscreen,
    );
    method(element_proto, context, "isFullscreen", 0, av_is_fullscreen);
    method(element_proto, context, "enableAudioMute", 0, av_enable_mute);
    method(
        element_proto,
        context,
        "disableAudioMute",
        0,
        av_disable_mute,
    );
    method(element_proto, context, "isAudioMute", 0, av_is_mute);
    method(element_proto, context, "setAudioSrc", 1, av_true);
    method(element_proto, context, "getAudioSrc", 0, av_default_src);
    method(element_proto, context, "setVideoSrc", 1, av_true);
    method(element_proto, context, "getVideoSrc", 0, av_default_src);
    method(element_proto, context, "setCaptionSrc", 1, av_true);
    method(
        element_proto,
        context,
        "getCaptionComponentURL",
        0,
        av_caption_url,
    );
    method(
        element_proto,
        context,
        "isCaptionExistent",
        1,
        av_caption_existent,
    );
    method(
        element_proto,
        context,
        "setCaptionVisibility",
        1,
        av_set_caption_visibility,
    );
    method(
        element_proto,
        context,
        "isCaptionVisible",
        0,
        av_is_caption_visible,
    );
    method(
        element_proto,
        context,
        "addCaptionListener",
        2,
        av_add_caption_listener,
    );
    method(
        element_proto,
        context,
        "removeCaptionListener",
        1,
        av_remove_caption_listener,
    );
}

fn av(this: &JsValue, context: &mut Context) -> JsResult<JsObject> {
    let element = super::this_object(this)?;
    let kind = element
        .get(boa_engine::js_string!("getAttribute"), context)
        .ok()
        .and_then(|f| f.as_callable())
        .and_then(|f| f.call(this, &[js("type")], context).ok())
        .map(|v| string(Some(&v), context))
        .unwrap_or_default();
    if kind != "video/x-arib2-broadcast" && kind != "video/x-iptvf-broadcast" {
        return Err(boa_engine::JsNativeError::typ()
            .with_message("放送映像音声オブジェクトではない")
            .into());
    }
    Ok(element)
}

fn av_enable_fullscreen(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    av(this, context)?;
    with_bridge(|b, _| b.borrow_mut().fullscreen = true);
    Ok(JsValue::from(true))
}

fn av_disable_fullscreen(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    av(this, context)?;
    with_bridge(|b, _| b.borrow_mut().fullscreen = false);
    Ok(JsValue::from(true))
}

fn av_is_fullscreen(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    av(this, context)?;
    Ok(JsValue::from(
        with_bridge(|b, _| b.borrow().fullscreen).unwrap_or(false),
    ))
}

fn av_enable_mute(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    av(this, context)?;
    with_bridge(|b, _| b.borrow_mut().mute = true);
    Ok(JsValue::from(true))
}

fn av_disable_mute(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    av(this, context)?;
    with_bridge(|b, _| b.borrow_mut().mute = false);
    Ok(JsValue::from(true))
}

fn av_is_mute(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    av(this, context)?;
    Ok(JsValue::from(
        with_bridge(|b, _| b.borrow().mute).unwrap_or(false),
    ))
}

fn av_true(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    av(this, context)?;
    Ok(JsValue::from(true))
}

fn av_default_src(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    av(this, context)?;
    Ok(js("arib2://-1.-1.-1/-1"))
}

fn av_caption_url(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    av(this, context)?;
    let first =
        with_bridge(|b, _| b.borrow().host.captions.first().map(super::Caption::url)).flatten();
    Ok(first.map_or(JsValue::null(), |url| js(&url)))
}

/// `isCaptionExistent(url)`。MPT にアセットがあるかで判断する（TR-B39 第 2 編 8.11.1）。
fn av_caption_existent(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    av(this, context)?;
    let url = string(args.first(), context);
    let wanted = super::caption_tag_of(&url);
    let has = with_bridge(|b, _| {
        b.borrow()
            .host
            .captions
            .iter()
            .any(|c| wanted.is_none_or(|tag| tag == c.tag))
    })
    .unwrap_or(false);
    Ok(JsValue::from(has))
}

fn av_set_caption_visibility(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    av(this, context)?;
    let on = args.first().is_some_and(JsValue::to_boolean);
    with_bridge(|b, _| b.borrow_mut().caption_visible = on);
    request(Request::Captions(on));
    Ok(JsValue::from(true))
}

fn av_is_caption_visible(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    av(this, context)?;
    Ok(JsValue::from(
        with_bridge(|b, _| b.borrow().caption_visible).unwrap_or(false),
    ))
}

fn av_add_caption_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    av(this, context)?;
    let Some(f) = args.first().filter(|v| v.is_callable()).cloned() else {
        return Ok(JsValue::undefined());
    };
    let url = string(args.get(1), context);
    let tag = super::caption_tag_of(&url).map_or(-1, i32::from);
    let f: JsValue = JsArray::from_iter([f, JsValue::from(tag)], context).into();
    let list = match global(context, "__zw_caption_listeners")
        .and_then(|o| JsArray::from_object(o).ok())
    {
        Some(list) => list,
        None => {
            let list = JsArray::new(context)?;
            let as_value: JsValue = list.clone().into();
            set_global(context, "__zw_caption_listeners", as_value);
            list
        }
    };
    let _ = list.push(f, context);
    Ok(JsValue::undefined())
}

fn av_remove_caption_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    av(this, context)?;
    let Some(list) =
        global(context, "__zw_caption_listeners").and_then(|o| JsArray::from_object(o).ok())
    else {
        return Ok(JsValue::undefined());
    };
    let kept: Vec<JsValue> = match args.first() {
        Some(target) => {
            let length = list.length(context).unwrap_or(0);
            let mut kept = Vec::new();
            for i in 0..length {
                let Ok(pair) = list.get(i, context) else {
                    continue;
                };
                let f = pair
                    .as_object()
                    .and_then(|o| o.get(0, context).ok())
                    .unwrap_or_default();
                if !f.strict_equals(target) {
                    kept.push(pair);
                }
            }
            kept
        }
        None => Vec::new(),
    };
    let fresh = JsArray::from_iter(kept, context);
    set_global(context, "__zw_caption_listeners", fresh.into());
    Ok(JsValue::undefined())
}
