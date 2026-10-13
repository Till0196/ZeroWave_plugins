// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! `document` オブジェクト（`getElementById`、`currentFocus`、`currentEvent`）。

use super::*;

pub(super) fn document_object(context: &mut Context) -> JsObject {
    let mut object = ObjectInitializer::new(context);
    object.function(
        NativeFunction::from_fn_ptr(get_element_by_id),
        js_string!("getElementById"),
        1,
    );
    let focus_get = function(object.context(), current_focus_get);
    let focus_set = function(object.context(), current_focus_set);
    object.accessor(
        js_string!("currentFocus"),
        Some(focus_get),
        Some(focus_set),
        Attribute::all(),
    );
    let event_get = function(object.context(), current_event_get);
    object.accessor(
        js_string!("currentEvent"),
        Some(event_get),
        None,
        Attribute::all(),
    );
    let root_get = function(object.context(), document_element_get);
    object.accessor(
        js_string!("documentElement"),
        Some(root_get),
        None,
        Attribute::all(),
    );
    object.build()
}

fn get_element_by_id(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let id = string(args.first(), context);
    let found = host(context)
        .page
        .as_ref()
        .and_then(|page| page.document.by_id(&id));
    Ok(match found {
        Some(node) => node_object(context, node),
        None => JsValue::null(),
    })
}

fn document_element_get(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let root = host(context)
        .page
        .as_ref()
        .and_then(|page| page.document.root);
    Ok(match root {
        Some(node) => node_object(context, node),
        None => JsValue::null(),
    })
}

fn current_focus_get(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let focus = host(context).page.as_ref().and_then(|page| page.focus);
    Ok(match focus {
        Some(node) => node_object(context, node),
        None => JsValue::null(),
    })
}

fn current_focus_set(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let wanted = args.first().and_then(|value| node_of(value, context));
    host(context).pending_focus = Some(wanted);
    Ok(JsValue::undefined())
}

fn current_event_get(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(event) = host(context).event.clone() else {
        return Ok(JsValue::null());
    };
    let Event {
        kind,
        target,
        key_code,
        status,
        message_id,
        message_version,
        message_group_id,
        private_data,
        module_ref,
        es_ref,
    } = event;
    let target = match target {
        Some(target) => node_object(context, target),
        None => JsValue::null(),
    };
    let target_for_object = target.clone();
    let mut object = ObjectInitializer::new(context);
    object.property(js_string!("type"), js(&kind), Attribute::all());
    object.property(js_string!("target"), target, Attribute::all());
    let number =
        |value: Option<u32>| value.map_or(JsValue::undefined(), |v| JsValue::from(v as f64));
    object.property(js_string!("keyCode"), number(key_code), Attribute::all());
    object.property(
        js_string!("status"),
        status.map_or(JsValue::undefined(), |v| JsValue::from(f64::from(v))),
        Attribute::all(),
    );
    object.property(
        js_string!("messageId"),
        number(message_id),
        Attribute::all(),
    );
    object.property(
        js_string!("messageVersion"),
        number(message_version),
        Attribute::all(),
    );
    object.property(
        js_string!("messageGroupId"),
        number(message_group_id),
        Attribute::all(),
    );
    object.property(
        js_string!("privateData"),
        private_data.map_or(JsValue::undefined(), |v| js(&v)),
        Attribute::all(),
    );
    object.property(
        js_string!("moduleRef"),
        module_ref.map_or(JsValue::undefined(), |v| js(&v)),
        Attribute::all(),
    );
    object.property(
        js_string!("esRef"),
        es_ref.map_or(JsValue::undefined(), |v| js(&v)),
        Attribute::all(),
    );
    // BMLBeventEvent の残り（STD-B24 第二編 5.3.19）。ここでは起きない。
    for name in [
        "languageTag",
        "registerId",
        "serviceId",
        "eventId",
        "segmentId",
    ] {
        object.property(JsString::from(name), JsValue::undefined(), Attribute::all());
    }
    let object_element = if kind == "MediaStopped" {
        target_for_object.clone()
    } else {
        JsValue::null()
    };
    object.property(js_string!("object"), object_element, Attribute::all());
    Ok(object.build().into())
}
