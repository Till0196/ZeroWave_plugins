// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 要素のオブジェクト（`node_object`）と、その属性・子・親・フォーカスのインターフェース。

use super::*;

pub(super) fn style_mode(this: &JsValue, context: &mut Context) -> u8 {
    this.as_object()
        .and_then(|o| o.get(js_string!("__m"), context).ok())
        .and_then(|v| v.as_number())
        .map_or(0, |n| n as u8)
}

pub(super) fn node_of(this: &JsValue, context: &mut Context) -> Option<NodeId> {
    let object = this.as_object()?;
    let index = object.get(js_string!("__n"), context).ok()?;
    index.as_number().map(|n| NodeId(n as usize))
}

pub(crate) fn node_object(context: &mut Context, id: NodeId) -> JsValue {
    let is_element = host(context)
        .page
        .as_ref()
        .is_some_and(|page| page.document.element(id).is_some());
    let mut object = ObjectInitializer::new(context);
    object.property(
        js_string!("__n"),
        JsValue::from(id.0 as f64),
        Attribute::all(),
    );
    if !is_element {
        let get = function(object.context(), node_text_get);
        let set = function(object.context(), node_text_set);
        object.accessor(js_string!("data"), Some(get), Some(set), Attribute::all());
        let get = function(object.context(), node_text_get);
        let set = function(object.context(), node_text_set);
        object.accessor(
            js_string!("nodeValue"),
            Some(get),
            Some(set),
            Attribute::all(),
        );
        object.property(js_string!("nodeName"), js("#text"), Attribute::all());
        object.property(js_string!("nodeType"), JsValue::from(3), Attribute::all());
        let get = function(object.context(), node_parent);
        object.accessor(js_string!("parentNode"), Some(get), None, Attribute::all());
        // テキストノードにも兄弟はある（CDATA のノードへ `firstChild.nextSibling.data` で書く文書がある）。
        let get = function(object.context(), next_sibling);
        object.accessor(js_string!("nextSibling"), Some(get), None, Attribute::all());
        let get = function(object.context(), previous_sibling);
        object.accessor(
            js_string!("previousSibling"),
            Some(get),
            None,
            Attribute::all(),
        );
        return object.build().into();
    }
    for (name, getter, setter) in [
        (
            "id",
            attribute_get as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
            Some(attribute_set as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>),
        ),
        ("className", attribute_get, Some(attribute_set)),
        ("data", attribute_get, Some(attribute_set)),
        ("type", attribute_get, Some(attribute_set)),
        ("value", attribute_get, Some(attribute_set)),
        ("href", attribute_get, Some(attribute_set)),
        ("src", attribute_get, Some(attribute_set)),
        ("remain", attribute_get, Some(attribute_set)),
        ("streamStatus", attribute_get, Some(attribute_set)),
        ("streamPosition", attribute_get, Some(attribute_set)),
        ("accessKey", attribute_get, Some(attribute_set)),
        ("maxLength", attribute_get, Some(attribute_set)),
        ("inputmode", attribute_get, Some(attribute_set)),
        ("moduleRef", attribute_get, Some(attribute_set)),
        ("messageGroupId", attribute_get, Some(attribute_set)),
        ("messageId", attribute_get, Some(attribute_set)),
        ("messageVersion", attribute_get, Some(attribute_set)),
        ("esRef", attribute_get, Some(attribute_set)),
        ("timeMode", attribute_get, Some(attribute_set)),
        ("timeValue", attribute_get, Some(attribute_set)),
        ("objectId", attribute_get, Some(attribute_set)),
        ("segmentId", attribute_get, Some(attribute_set)),
        ("languageTag", attribute_get, Some(attribute_set)),
        ("registerId", attribute_get, Some(attribute_set)),
        ("serviceId", attribute_get, Some(attribute_set)),
        ("eventId", attribute_get, Some(attribute_set)),
        ("peripheralRef", attribute_get, Some(attribute_set)),
        ("usedKeyList", attribute_get, Some(attribute_set)),
        // STD-B24 第二編 5.3 の要素のインターフェースの残り。値は属性そのまま。
        ("title", attribute_get, Some(attribute_set)),
        ("name", attribute_get, Some(attribute_set)),
        ("alt", attribute_get, Some(attribute_set)),
        ("action", attribute_get, Some(attribute_set)),
        ("method", attribute_get, Some(attribute_set)),
        ("defaultValue", attribute_get, Some(attribute_set)),
        ("classId", attribute_get, Some(attribute_set)),
        ("streamLooping", attribute_get, Some(attribute_set)),
        ("streamSpeedNumerator", attribute_get, Some(attribute_set)),
        ("streamSpeedDenominator", attribute_get, Some(attribute_set)),
        ("streamLevel", attribute_get, Some(attribute_set)),
        ("mainAudioStream", attribute_get, Some(attribute_set)),
    ] {
        let css_name: &str = match name {
            "className" => "class",
            "accessKey" => "accesskey",
            "maxLength" => "maxlength",
            "moduleRef" => "module_ref",
            "messageGroupId" => "message_group_id",
            "messageId" => "message_id",
            "messageVersion" => "message_version",
            "esRef" => "es_ref",
            "timeMode" => "time_mode",
            "timeValue" => "time_value",
            "objectId" => "object_id",
            "segmentId" => "segment_id",
            "languageTag" => "language_tag",
            "registerId" => "register_id",
            "serviceId" => "service_id",
            "eventId" => "event_id",
            "peripheralRef" => "peripheral_ref",
            "usedKeyList" => "used-key-list",
            "streamStatus" => "streamstatus",
            "streamPosition" => "streamposition",
            "streamLooping" => "streamlooping",
            "streamSpeedNumerator" => "streamspeednumerator",
            "streamSpeedDenominator" => "streamspeeddenominator",
            "streamLevel" => "streamlevel",
            "mainAudioStream" => "mainaudiostream",
            "classId" => "classid",
            "defaultValue" => "value",
            other => other,
        };
        let captured = JsString::from(css_name);
        let get = FunctionObjectBuilder::new(
            object.context().realm(),
            NativeFunction::from_copy_closure_with_captures(
                move |this, args, name, context| {
                    getter(
                        this,
                        &[JsValue::from(name.clone())]
                            .into_iter()
                            .chain(args.iter().cloned())
                            .collect::<Vec<_>>(),
                        context,
                    )
                },
                captured.clone(),
            ),
        )
        .build();
        let set = setter.map(|setter| {
            FunctionObjectBuilder::new(
                object.context().realm(),
                NativeFunction::from_copy_closure_with_captures(
                    move |this, args, name, context| {
                        setter(
                            this,
                            &[JsValue::from(name.clone())]
                                .into_iter()
                                .chain(args.iter().cloned())
                                .collect::<Vec<_>>(),
                            context,
                        )
                    },
                    captured.clone(),
                ),
            )
            .build()
        });
        object.accessor(JsString::from(name), Some(get), set, Attribute::all());
    }
    for (name, getter, setter) in [
        (
            "invisible",
            flag_get as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
            flag_set as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
        ),
        ("subscribe", flag_get, flag_set),
        ("readOnly", flag_get, flag_set),
        ("disabled", flag_get, flag_set),
        ("checked", flag_get, flag_set),
    ] {
        let captured = JsString::from(name);
        let get = FunctionObjectBuilder::new(
            object.context().realm(),
            NativeFunction::from_copy_closure_with_captures(
                move |this, args, name, context| {
                    getter(
                        this,
                        &[JsValue::from(name.clone())]
                            .into_iter()
                            .chain(args.iter().cloned())
                            .collect::<Vec<_>>(),
                        context,
                    )
                },
                captured.clone(),
            ),
        )
        .build();
        let set = FunctionObjectBuilder::new(
            object.context().realm(),
            NativeFunction::from_copy_closure_with_captures(
                move |this, args, name, context| {
                    setter(
                        this,
                        &[JsValue::from(name.clone())]
                            .into_iter()
                            .chain(args.iter().cloned())
                            .collect::<Vec<_>>(),
                        context,
                    )
                },
                captured,
            ),
        )
        .build();
        object.accessor(JsString::from(name), Some(get), Some(set), Attribute::all());
    }
    for (name, getter) in [
        (
            "tagName",
            element_name as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
        ),
        ("nodeName", element_name),
        ("style", style_get),
        ("firstChild", first_child),
        ("lastChild", last_child),
        ("parentNode", node_parent),
        ("childNodes", child_nodes),
        ("nextSibling", next_sibling),
        ("previousSibling", previous_sibling),
        ("normalStyle", style_get),
        ("focusStyle", focus_style_get),
        ("activeStyle", active_style_get),
    ] {
        let get = function(object.context(), getter);
        object.accessor(JsString::from(name), Some(get), None, Attribute::all());
    }
    object.property(js_string!("nodeType"), JsValue::from(1), Attribute::all());
    for (name, f, length) in [
        (
            "getAttribute",
            get_attribute as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
            1,
        ),
        ("setAttribute", set_attribute, 2),
        ("removeAttribute", remove_attribute, 1),
        ("hasAttribute", has_attribute, 1),
        ("focus", focus_element, 0),
        ("blur", blur_element, 0),
        ("normalize", ok_one, 0),
        ("hasChildNodes", has_children, 0),
        // 蓄積した映像のためのもの（STD-B24 第二編 5.3.14）。蓄積は無い。
        ("hasAssociatedIndex", false_fn, 0),
        ("assignToLocalEvent", false_fn, 1),
        ("assignToNodePlayMode", false_fn, 1),
        ("submit", submit_form, 0),
    ] {
        object.function(NativeFunction::from_fn_ptr(f), JsString::from(name), length);
    }
    object.build().into()
}

fn submit_form(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::undefined());
    };
    host(context).pending_submit = Some(id);
    Ok(JsValue::undefined())
}

fn node_text_get(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::undefined());
    };
    let text = host(context)
        .page
        .as_ref()
        .map(|page| page.document.text_of(id))
        .unwrap_or_default();
    Ok(js(&text))
}

fn node_text_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::undefined());
    };
    let text = string(args.first(), context);
    host(context).set_text(id, &text);
    Ok(JsValue::undefined())
}

fn element_name(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::undefined());
    };
    let name = host(context)
        .page
        .as_ref()
        .and_then(|page| page.document.element(id).map(|e| e.name.clone()))
        .unwrap_or_default();
    Ok(js(&name))
}

fn attribute_get(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::undefined());
    };
    let value = host(context).page.as_ref().and_then(|page| {
        page.document
            .element(id)
            .and_then(|e| e.attribute(&name).map(str::to_owned))
    });
    Ok(match value {
        Some(value) if name == "maxlength" => {
            JsValue::from(value.parse::<f64>().unwrap_or(f64::NAN))
        }
        Some(value) => js(&value),
        None if name == "maxlength" => JsValue::from(-1),
        None => js(""),
    })
}

fn attribute_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let value = string(args.get(1), context);
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::undefined());
    };
    let host = host(context);
    let mut audio: Option<Option<String>> = None;
    if let Some(page) = &mut host.page
        && let Some(element) = page.document.element_mut(id)
    {
        element.set_attribute(&name, &value);
        if name == "streamstatus"
            && element.name == "object"
            && element
                .attribute("type")
                .is_some_and(|t| t.eq_ignore_ascii_case("audio/X-arib-aiff"))
        {
            audio = Some(match value.as_str() {
                "play" => element.attribute("data").map(str::to_owned),
                _ => None,
            });
        }
        if name == "id" || name == "class" {
            page.document.reindex();
        }
        page.dirty = true;
    }
    if name == "es_ref" {
        host.fired.retain(|(node, _), _| *node != id);
    }
    match audio {
        Some(Some(data)) => request_audio(host, &data),
        Some(None) => {
            host.media_ends = None;
            host.requests.push(Request::AudioStop);
        }
        None => {}
    }
    Ok(JsValue::undefined())
}

fn flag_get(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let name = string(args.first(), context).to_ascii_lowercase();
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::undefined());
    };
    let set = host(context).page.as_ref().is_some_and(|page| {
        page.document
            .element(id)
            .is_some_and(|e| e.attribute(&name) == Some(name.as_str()))
    });
    Ok(JsValue::from(set))
}

fn flag_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let name = string(args.first(), context).to_ascii_lowercase();
    let on = args.get(1).is_some_and(JsValue::to_boolean);
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::undefined());
    };
    let host = host(context);
    if let Some(page) = &mut host.page
        && let Some(element) = page.document.element_mut(id)
    {
        if on {
            element.set_attribute(&name, &name);
        } else {
            element.remove_attribute(&name);
        }
        page.dirty = true;
        if name == "subscribe" && on {
            host.pending_subscribe.push(id);
            // 立て直した `beitem` は、発生させたバージョンを忘れる。
            host.fired.retain(|(node, _), _| *node != id);
        }
    }
    Ok(JsValue::undefined())
}

fn get_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::null());
    };
    let value = host(context).page.as_ref().and_then(|page| {
        page.document
            .element(id)
            .and_then(|e| e.attribute(&name).map(str::to_owned))
    });
    Ok(value.map_or(JsValue::null(), |v| js(&v)))
}

fn set_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    attribute_set(this, args, context)
}

fn remove_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::undefined());
    };
    let host = host(context);
    if let Some(page) = &mut host.page
        && let Some(element) = page.document.element_mut(id)
    {
        element.remove_attribute(&name);
        page.dirty = true;
    }
    Ok(JsValue::undefined())
}

fn has_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::from(false));
    };
    let has = host(context).page.as_ref().is_some_and(|page| {
        page.document
            .element(id)
            .is_some_and(|e| e.attribute(&name).is_some())
    });
    Ok(JsValue::from(has))
}

fn has_children(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::from(false));
    };
    let has = host(context).page.as_ref().is_some_and(|page| {
        page.document
            .element(id)
            .is_some_and(|e| !e.children.is_empty())
    });
    Ok(JsValue::from(has))
}

pub(super) fn ok_one(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(1))
}

fn child_at(
    this: &JsValue,
    context: &mut Context,
    pick: impl Fn(&[NodeId]) -> Option<NodeId>,
) -> JsResult<JsValue> {
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::null());
    };
    let child = host(context)
        .page
        .as_ref()
        .and_then(|page| page.document.element(id).and_then(|e| pick(&e.children)));
    Ok(match child {
        Some(child) => node_object(context, child),
        None => JsValue::null(),
    })
}

fn first_child(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    child_at(this, context, |children| children.first().copied())
}

fn last_child(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    child_at(this, context, |children| children.last().copied())
}

fn child_nodes(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::null());
    };
    let children: Vec<NodeId> = host(context)
        .page
        .as_ref()
        .and_then(|page| page.document.element(id).map(|e| e.children.clone()))
        .unwrap_or_default();
    let items: Vec<JsValue> = children
        .into_iter()
        .map(|child| node_object(context, child))
        .collect();
    Ok(JsArray::from_iter(items, context).into())
}

fn node_parent(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::null());
    };
    let parent = host(context)
        .page
        .as_ref()
        .and_then(|page| page.document.node(id).parent);
    Ok(match parent {
        Some(parent) => node_object(context, parent),
        None => JsValue::null(),
    })
}

fn sibling(this: &JsValue, context: &mut Context, step: isize) -> JsResult<JsValue> {
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::null());
    };
    let found = host(context).page.as_ref().and_then(|page| {
        let parent = page.document.node(id).parent?;
        let siblings = &page.document.element(parent)?.children;
        let at = siblings.iter().position(|s| *s == id)?;
        let wanted = at.checked_add_signed(step)?;
        siblings.get(wanted).copied()
    });
    Ok(match found {
        Some(found) => node_object(context, found),
        None => JsValue::null(),
    })
}

fn next_sibling(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    sibling(this, context, 1)
}

fn previous_sibling(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    sibling(this, context, -1)
}

fn focus_element(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::undefined());
    };
    // フォーカスの移動は onfocus を呼ぶので、スクリプトの外で。
    host(context).pending_focus = Some(Some(id));
    Ok(JsValue::undefined())
}

fn blur_element(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::undefined());
    };
    let host = host(context);
    if host
        .page
        .as_ref()
        .is_some_and(|page| page.focus == Some(id))
    {
        host.pending_focus = Some(None);
    }
    Ok(JsValue::undefined())
}
