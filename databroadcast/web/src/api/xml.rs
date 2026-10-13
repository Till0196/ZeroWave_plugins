// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! `DOMParser` と `XMLSerializer`。Blitz の DOM は要素の名前を小文字にするので、
//! XML は別の軽いツリーに作り、jQuery（Sizzle）が触る API だけを生やす。
//! querySelectorAll は生やさない（あると Sizzle がそちらを使い、面倒が増える）。

use boa_engine::object::builtins::JsArray;
use boa_engine::{Context, JsObject, JsResult, JsValue};

use super::{accessor, function, get, global, js, method, object, set, set_global, string};

const ELEMENT: i32 = 1;
const TEXT: i32 = 3;
const CDATA: i32 = 4;
const COMMENT: i32 = 8;
const DOCUMENT: i32 = 9;
const FRAGMENT: i32 = 11;

const PROTO: &str = "__zw_xml_node_proto";

pub(super) fn install(context: &mut Context) {
    let proto = object(context);
    for (name, getter) in [
        ("firstChild", first_child as super::Native),
        ("lastChild", last_child),
        ("nextSibling", next_sibling),
        ("previousSibling", previous_sibling),
        ("tagName", tag_name),
        ("localName", local_name),
        ("textContent", text_content),
        ("data", node_data),
        ("children", children),
        ("attributes", attributes),
        ("namespaceURI", namespace_uri),
        ("documentElement", document_element),
    ] {
        accessor(&proto, context, name, Some(getter), None);
    }
    // Sizzle の assert は `el.className = "i"` と `el.id = expando` を試す。
    accessor(&proto, context, "id", Some(id_get), Some(id_set));
    accessor(
        &proto,
        context,
        "className",
        Some(class_get),
        Some(class_set),
    );
    for (name, length, body) in [
        ("getAttribute", 1, get_attribute as super::Native),
        ("setAttribute", 2, set_attribute),
        ("hasAttribute", 1, has_attribute),
        ("removeAttribute", 1, remove_attribute),
        ("getAttributeNode", 1, get_attribute_node),
        ("appendChild", 1, append_child),
        ("removeChild", 1, remove_child),
        ("insertBefore", 2, insert_before),
        ("hasChildNodes", 0, has_child_nodes),
        ("contains", 1, contains),
        ("getElementsByTagName", 1, get_elements_by_tag_name),
        ("getElementById", 1, get_element_by_id),
        ("createElement", 1, create_element),
        ("createElementNS", 2, create_element_ns),
        ("createTextNode", 1, create_text_node),
        ("createComment", 1, create_comment),
        ("createDocumentFragment", 0, create_fragment),
    ] {
        method(&proto, context, name, length, body);
    }
    set_global(context, PROTO, proto.into());

    let parser = constructor(context, "DOMParser");
    let parser_proto = get(&parser, "prototype", context)
        .as_object()
        .unwrap_or_else(|| object(context));
    method(
        &parser_proto,
        context,
        "parseFromString",
        2,
        parse_from_string,
    );
    set_global(context, "DOMParser", parser.into());
    let serializer = constructor(context, "XMLSerializer");
    let serializer_proto = get(&serializer, "prototype", context)
        .as_object()
        .unwrap_or_else(|| object(context));
    method(
        &serializer_proto,
        context,
        "serializeToString",
        1,
        serialize_to_string,
    );
    set_global(context, "XMLSerializer", serializer.into());
    let parse_xml = function(context, "__zw_parse_xml", 1, parse_xml);
    set_global(context, "__zw_parse_xml", parse_xml.into());
}

fn constructor(context: &mut Context, name: &str) -> JsObject {
    let f = boa_engine::object::FunctionObjectBuilder::new(
        context.realm(),
        boa_engine::NativeFunction::from_fn_ptr(construct),
    )
    .name(boa_engine::JsString::from(name))
    .constructor(true)
    .build();
    let proto = object(context);
    set(&proto, "constructor", f.clone().into(), context);
    set(&f, "prototype", proto.into(), context);
    f.into()
}

fn construct(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let proto = this
        .as_object()
        .map(|target| get(&target, "prototype", context))
        .and_then(|p| p.as_object());
    let made = match proto {
        Some(proto) => JsObject::from_proto_and_data(Some(proto), ()),
        None => object(context),
    };
    Ok(made.into())
}

fn node_proto(context: &mut Context) -> Option<JsObject> {
    global(context, PROTO)
}

fn new_node(context: &mut Context, kind: i32, name: &str, owner: Option<&JsObject>) -> JsObject {
    let node = match node_proto(context) {
        Some(proto) => JsObject::from_proto_and_data(Some(proto), ()),
        None => object(context),
    };
    set(&node, "nodeType", JsValue::from(kind), context);
    set(&node, "nodeName", js(name), context);
    if let Ok(children) = JsArray::new(context) {
        set(&node, "childNodes", children.into(), context);
    }
    set(&node, "parentNode", JsValue::null(), context);
    set(
        &node,
        "ownerDocument",
        owner.map_or(JsValue::null(), |o| o.clone().into()),
        context,
    );
    set(&node, "nodeValue", JsValue::null(), context);
    if let Ok(attrs) = JsArray::new(context) {
        set(&node, "_attrs", attrs.into(), context);
    }
    node
}

fn kind_of(node: &JsObject, context: &mut Context) -> i32 {
    get(node, "nodeType", context)
        .to_number(context)
        .unwrap_or(0.0) as i32
}

fn child_nodes(node: &JsObject, context: &mut Context) -> Option<JsArray> {
    JsArray::from_object(get(node, "childNodes", context).as_object()?).ok()
}

fn attrs_of(node: &JsObject, context: &mut Context) -> Option<JsArray> {
    JsArray::from_object(get(node, "_attrs", context).as_object()?).ok()
}

fn items(array: &JsArray, context: &mut Context) -> Vec<JsValue> {
    let length = array.length(context).unwrap_or(0);
    (0..length)
        .filter_map(|i| array.get(i, context).ok())
        .collect()
}

fn index_of(array: &JsArray, wanted: &JsObject, context: &mut Context) -> Option<u64> {
    items(array, context)
        .iter()
        .position(|v| v.as_object().is_some_and(|o| JsObject::equals(&o, wanted)))
        .map(|i| i as u64)
}

fn remove_at(array: &JsArray, at: u64, context: &mut Context) {
    let _ = array.splice(at as u32, Some(1), &[], context);
}

fn insert_at(array: &JsArray, at: u64, v: JsValue, context: &mut Context) {
    let _ = array.splice(at as u32, Some(0), &[v], context);
}

fn attach(parent: &JsObject, child: &JsObject, context: &mut Context) {
    if let Some(had) = get(child, "parentNode", context).as_object()
        && let Some(list) = child_nodes(&had, context)
        && let Some(at) = index_of(&list, child, context)
    {
        remove_at(&list, at, context);
    }
    set(child, "parentNode", parent.clone().into(), context);
    let owner = if kind_of(parent, context) == DOCUMENT {
        parent.clone().into()
    } else {
        get(parent, "ownerDocument", context)
    };
    set(child, "ownerDocument", owner, context);
    if let Some(list) = child_nodes(parent, context) {
        let _ = list.push(child.clone(), context);
    }
}

fn text_node(context: &mut Context, kind: i32, value: &str, owner: &JsObject) -> JsObject {
    let name = match kind {
        CDATA => "#cdata-section",
        COMMENT => "#comment",
        _ => "#text",
    };
    let node = new_node(context, kind, name, Some(owner));
    set(&node, "nodeValue", js(value), context);
    node
}

fn push_attr(node: &JsObject, name: &str, value: &str, context: &mut Context) {
    let attr = object(context);
    set(&attr, "name", js(name), context);
    set(&attr, "value", js(value), context);
    set(&attr, "specified", JsValue::from(true), context);
    if let Some(attrs) = attrs_of(node, context) {
        let _ = attrs.push(attr, context);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Read {
    Element {
        name: String,
        attrs: Vec<(String, String)>,
        children: Vec<Read>,
    },
    Text(String),
    Cdata(String),
}

fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';') else {
            out.push_str(rest);
            return out;
        };
        let body = &rest[1..end];
        let replaced = if let Some(number) = body.strip_prefix('#') {
            let code = if let Some(hex) = number.strip_prefix(['x', 'X']) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                number.parse::<u32>().ok()
            };
            code.and_then(char::from_u32).map(|c| c.to_string())
        } else {
            match body.to_ascii_lowercase().as_str() {
                "lt" => Some("<".into()),
                "gt" => Some(">".into()),
                "amp" => Some("&".into()),
                "quot" => Some("\"".into()),
                "apos" => Some("'".into()),
                "nbsp" => Some(" ".into()),
                _ => None,
            }
        };
        match replaced {
            Some(s) if body.chars().all(|c| c.is_ascii_alphanumeric() || c == '#') => {
                out.push_str(&s);
                rest = &rest[end + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn tag_end(text: &[char], from: usize) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, &c) in text.iter().enumerate().skip(from + 1) {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '>' => return Some(i),
            None => {}
        }
    }
    None
}

fn element(inner: &str) -> Read {
    let inner = inner.trim_start();
    let name_end = inner
        .find(|c: char| c.is_whitespace() || c == '/')
        .unwrap_or(inner.len());
    let name = inner[..name_end].to_owned();
    let mut attrs = Vec::new();
    let rest: Vec<char> = inner[name_end..].chars().collect();
    let mut i = 0;
    while i < rest.len() {
        while i < rest.len() && (rest[i].is_whitespace() || rest[i] == '/') {
            i += 1;
        }
        if i >= rest.len() {
            break;
        }
        let start = i;
        while i < rest.len() && !rest[i].is_whitespace() && rest[i] != '=' && rest[i] != '/' {
            i += 1;
        }
        let key: String = rest[start..i].iter().collect();
        if key.is_empty() {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < rest.len() && rest[j].is_whitespace() {
            j += 1;
        }
        let mut value = String::new();
        if j < rest.len() && rest[j] == '=' {
            j += 1;
            while j < rest.len() && rest[j].is_whitespace() {
                j += 1;
            }
            if j < rest.len() && (rest[j] == '"' || rest[j] == '\'') {
                let q = rest[j];
                j += 1;
                let vs = j;
                while j < rest.len() && rest[j] != q {
                    j += 1;
                }
                value = rest[vs..j].iter().collect();
                j += 1;
            } else {
                let vs = j;
                while j < rest.len() && !rest[j].is_whitespace() && rest[j] != '>' {
                    j += 1;
                }
                value = rest[vs..j].iter().collect();
            }
            i = j;
        }
        attrs.push((key, decode(&value)));
    }
    Read::Element {
        name,
        attrs,
        children: Vec::new(),
    }
}

/// 文書を読む。壊れていても読めたところまで（jQuery はルート要素が無いことで見分ける）。
fn parse(text: &str) -> Vec<Read> {
    let chars: Vec<char> = text.chars().collect();
    let mut stack: Vec<Read> = vec![Read::Element {
        name: String::new(),
        attrs: Vec::new(),
        children: Vec::new(),
    }];
    let push_child = |stack: &mut Vec<Read>, child: Read| {
        if let Some(Read::Element { children, .. }) = stack.last_mut() {
            children.push(child);
        }
    };
    let find = |from: usize, pattern: &str| -> Option<usize> {
        let p: Vec<char> = pattern.chars().collect();
        (from..chars.len().saturating_sub(p.len() - 1)).find(|&i| chars[i..i + p.len()] == p[..])
    };
    let starts = |at: usize, pattern: &str| -> bool {
        let p: Vec<char> = pattern.chars().collect();
        chars.len() >= at + p.len() && chars[at..at + p.len()] == p[..]
    };
    let slice = |a: usize, b: usize| -> String { chars[a..b.min(chars.len())].iter().collect() };
    let mut at = 0;
    let length = chars.len();
    while at < length {
        let Some(lt) = find(at, "<") else {
            push_child(&mut stack, Read::Text(decode(&slice(at, length))));
            break;
        };
        if lt > at {
            push_child(&mut stack, Read::Text(decode(&slice(at, lt))));
        }
        if starts(lt, "<!--") {
            at = find(lt, "-->").map_or(length, |e| e + 3);
            continue;
        }
        if starts(lt, "<![CDATA[") {
            let end = find(lt, "]]>");
            push_child(
                &mut stack,
                Read::Cdata(slice(lt + 9, end.unwrap_or(length))),
            );
            at = end.map_or(length, |e| e + 3);
            continue;
        }
        if starts(lt, "<?") || starts(lt, "<!") {
            at = find(lt, ">").map_or(length, |e| e + 1);
            continue;
        }
        let Some(gt) = tag_end(&chars, lt) else {
            break;
        };
        let inner = slice(lt + 1, gt);
        at = gt + 1;
        if let Some(closing) = inner.strip_prefix('/') {
            let closing = closing.trim_end_matches([' ', '/', '\t', '\n', '\r']);
            if let Some(depth) = stack
                .iter()
                .rposition(|n| matches!(n, Read::Element { name, .. } if name == closing))
                && depth > 0
            {
                while stack.len() > depth {
                    let done = stack.pop().expect("あった");
                    push_child(&mut stack, done);
                }
            }
            continue;
        }
        let empty = inner.ends_with('/');
        let inner = if empty {
            &inner[..inner.len() - 1]
        } else {
            &inner
        };
        let el = element(inner);
        if empty {
            push_child(&mut stack, el);
        } else {
            stack.push(el);
        }
    }
    while stack.len() > 1 {
        let done = stack.pop().expect("あった");
        push_child(&mut stack, done);
    }
    match stack.pop() {
        Some(Read::Element { children, .. }) => children,
        _ => Vec::new(),
    }
}

fn build(read: &Read, parent: &JsObject, owner: &JsObject, context: &mut Context) {
    match read {
        Read::Element {
            name,
            attrs,
            children,
        } => {
            let el = new_node(context, ELEMENT, name, Some(owner));
            for (k, v) in attrs {
                push_attr(&el, k, v, context);
            }
            attach(parent, &el, context);
            for child in children {
                build(child, &el, owner, context);
            }
        }
        Read::Text(text) => {
            if !text.is_empty() {
                let node = text_node(context, TEXT, text, owner);
                attach(parent, &node, context);
            }
        }
        Read::Cdata(text) => {
            let node = text_node(context, CDATA, text, owner);
            attach(parent, &node, context);
        }
    }
}

fn new_document(context: &mut Context) -> JsObject {
    new_node(context, DOCUMENT, "#document", None)
}

fn broken(context: &mut Context, why: &str) -> JsObject {
    let doc = new_document(context);
    let el = new_node(context, ELEMENT, "parsererror", Some(&doc));
    attach(&doc, &el, context);
    let text = text_node(context, TEXT, why, &doc);
    attach(&el, &text, context);
    doc
}

fn parse_document(context: &mut Context, text: &str) -> JsObject {
    let read = parse(text);
    if !read.iter().any(|r| matches!(r, Read::Element { .. })) {
        return broken(context, "no root element");
    }
    let doc = new_document(context);
    for node in &read {
        build(node, &doc, &doc, context);
    }
    doc
}

fn parse_from_string(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(text) = args.first().filter(|v| v.is_string()) else {
        return Ok(broken(context, "no source").into());
    };
    let kind = string(args.get(1), context);
    if kind.contains("html") {
        return Ok(broken(context, "html is not parsed here").into());
    }
    let text = string(Some(text), context);
    Ok(parse_document(context, &text).into())
}

fn parse_xml(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let text = string(args.first(), context);
    Ok(parse_document(context, &text).into())
}

fn this_node(this: &JsValue) -> JsResult<JsObject> {
    super::this_object(this)
}

fn first_child(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    Ok(child_nodes(&node, context)
        .and_then(|list| list.get(0, context).ok())
        .filter(|v| !v.is_undefined())
        .unwrap_or(JsValue::null()))
}

fn last_child(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    Ok(child_nodes(&node, context)
        .and_then(|list| {
            let n = list.length(context).unwrap_or(0);
            (n > 0).then(|| list.get(n - 1, context).ok()).flatten()
        })
        .unwrap_or(JsValue::null()))
}

fn sibling(this: &JsValue, step: i64, context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    let Some(parent) = get(&node, "parentNode", context).as_object() else {
        return Ok(JsValue::null());
    };
    let Some(list) = child_nodes(&parent, context) else {
        return Ok(JsValue::null());
    };
    let Some(at) = index_of(&list, &node, context) else {
        return Ok(JsValue::null());
    };
    let wanted = at as i64 + step;
    if wanted < 0 {
        return Ok(JsValue::null());
    }
    Ok(list
        .get(wanted as u64, context)
        .ok()
        .filter(|v| !v.is_undefined())
        .unwrap_or(JsValue::null()))
}

fn next_sibling(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    sibling(this, 1, context)
}

fn previous_sibling(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    sibling(this, -1, context)
}

fn tag_name(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    if kind_of(&node, context) == ELEMENT {
        Ok(get(&node, "nodeName", context))
    } else {
        Ok(JsValue::undefined())
    }
}

fn local_name(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    let name = string(Some(&get(&node, "nodeName", context)), context);
    Ok(js(name.rsplit(':').next().unwrap_or(&name)))
}

fn text_of(node: &JsObject, context: &mut Context, out: &mut String) {
    match kind_of(node, context) {
        TEXT | CDATA => out.push_str(&string(Some(&get(node, "nodeValue", context)), context)),
        COMMENT => {}
        _ => {
            if let Some(list) = child_nodes(node, context) {
                for child in items(&list, context) {
                    if let Some(child) = child.as_object() {
                        text_of(&child, context, out);
                    }
                }
            }
        }
    }
}

fn text_content(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    let mut out = String::new();
    text_of(&node, context, &mut out);
    Ok(js(&out))
}

fn node_data(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    Ok(get(&node, "nodeValue", context))
}

fn children(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    let out = JsArray::new(context)?;
    if let Some(list) = child_nodes(&node, context) {
        for child in items(&list, context) {
            if child
                .as_object()
                .is_some_and(|c| kind_of(&c, context) == ELEMENT)
            {
                let _ = out.push(child, context);
            }
        }
    }
    Ok(out.into())
}

fn attributes(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    Ok(get(&node, "_attrs", context))
}

fn namespace_uri(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::null())
}

fn document_element(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    if kind_of(&node, context) != DOCUMENT {
        return Ok(JsValue::undefined());
    }
    if let Some(list) = child_nodes(&node, context) {
        for child in items(&list, context) {
            if child
                .as_object()
                .is_some_and(|c| kind_of(&c, context) == ELEMENT)
            {
                return Ok(child);
            }
        }
    }
    Ok(JsValue::null())
}

fn find_attr(
    node: &JsObject,
    name: &str,
    context: &mut Context,
) -> Option<(JsArray, u64, JsObject)> {
    let attrs = attrs_of(node, context)?;
    for (i, attr) in items(&attrs, context).into_iter().enumerate() {
        let Some(attr) = attr.as_object() else {
            continue;
        };
        if string(Some(&get(&attr, "name", context)), context) == name {
            return Some((attrs, i as u64, attr));
        }
    }
    None
}

fn get_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    let name = string(args.first(), context);
    Ok(find_attr(&node, &name, context)
        .map(|(_, _, attr)| get(&attr, "value", context))
        .unwrap_or(JsValue::null()))
}

fn set_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    let name = string(args.first(), context);
    let value = string(args.get(1), context);
    match find_attr(&node, &name, context) {
        Some((_, _, attr)) => set(&attr, "value", js(&value), context),
        None => push_attr(&node, &name, &value, context),
    }
    Ok(JsValue::undefined())
}

fn has_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    let name = string(args.first(), context);
    Ok(JsValue::from(find_attr(&node, &name, context).is_some()))
}

fn remove_attribute(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    let name = string(args.first(), context);
    if let Some((attrs, at, _)) = find_attr(&node, &name, context) {
        remove_at(&attrs, at, context);
    }
    Ok(JsValue::undefined())
}

fn get_attribute_node(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = this_node(this)?;
    let name = string(args.first(), context);
    Ok(find_attr(&node, &name, context)
        .map(|(_, _, attr)| attr.into())
        .unwrap_or(JsValue::null()))
}

fn id_get(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    Ok(find_attr(&node, "id", context)
        .map(|(_, _, attr)| get(&attr, "value", context))
        .unwrap_or_else(|| js("")))
}

fn id_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    set_attribute(
        this,
        &[js("id"), args.first().cloned().unwrap_or_default()],
        context,
    )
}

fn class_get(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    Ok(find_attr(&node, "class", context)
        .map(|(_, _, attr)| get(&attr, "value", context))
        .unwrap_or_else(|| js("")))
}

fn class_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    set_attribute(
        this,
        &[js("class"), args.first().cloned().unwrap_or_default()],
        context,
    )
}

fn append_child(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let parent = this_node(this)?;
    let Some(child) = args.first().and_then(JsValue::as_object) else {
        return Ok(JsValue::null());
    };
    attach(&parent, &child, context);
    Ok(child.into())
}

fn remove_child(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let parent = this_node(this)?;
    let Some(child) = args.first().and_then(JsValue::as_object) else {
        return Ok(JsValue::null());
    };
    if let Some(list) = child_nodes(&parent, context)
        && let Some(at) = index_of(&list, &child, context)
    {
        remove_at(&list, at, context);
        set(&child, "parentNode", JsValue::null(), context);
    }
    Ok(child.into())
}

fn insert_before(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let parent = this_node(this)?;
    let Some(child) = args.first().and_then(JsValue::as_object) else {
        return Ok(JsValue::null());
    };
    let before = args.get(1).and_then(JsValue::as_object);
    let at = before
        .as_ref()
        .and_then(|b| child_nodes(&parent, context).and_then(|l| index_of(&l, b, context)));
    let Some(at) = at else {
        attach(&parent, &child, context);
        return Ok(child.into());
    };
    if let Some(had) = get(&child, "parentNode", context).as_object()
        && let Some(list) = child_nodes(&had, context)
        && let Some(i) = index_of(&list, &child, context)
    {
        remove_at(&list, i, context);
    }
    set(&child, "parentNode", parent.clone().into(), context);
    let owner = if kind_of(&parent, context) == DOCUMENT {
        parent.clone().into()
    } else {
        get(&parent, "ownerDocument", context)
    };
    set(&child, "ownerDocument", owner, context);
    if let Some(list) = child_nodes(&parent, context) {
        // 外した後で場所が動いていることがある。
        let at = before
            .as_ref()
            .and_then(|b| index_of(&list, b, context))
            .unwrap_or(at);
        insert_at(&list, at, child.clone().into(), context);
    }
    Ok(child.into())
}

fn has_child_nodes(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    Ok(JsValue::from(
        child_nodes(&node, context).is_some_and(|l| l.length(context).unwrap_or(0) > 0),
    ))
}

fn contains(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    let mut other = args.first().and_then(JsValue::as_object);
    while let Some(o) = other {
        if JsObject::equals(&o, &node) {
            return Ok(JsValue::from(true));
        }
        other = get(&o, "parentNode", context).as_object();
    }
    Ok(JsValue::from(false))
}

fn walk_elements(
    node: &JsObject,
    context: &mut Context,
    f: &mut dyn FnMut(&JsObject, &mut Context) -> bool,
) -> bool {
    let Some(list) = child_nodes(node, context) else {
        return false;
    };
    for child in items(&list, context) {
        let Some(child) = child.as_object() else {
            continue;
        };
        if kind_of(&child, context) == ELEMENT {
            if f(&child, context) {
                return true;
            }
            if walk_elements(&child, context, f) {
                return true;
            }
        }
    }
    false
}

/// 大小は問わない。Sizzle は書かれた綴りも小文字も渡し、余分に拾った物は後段の照合で除かれる。
fn get_elements_by_tag_name(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = this_node(this)?;
    let wanted = string(args.first(), context).to_ascii_lowercase();
    let all = wanted == "*";
    let out = JsArray::new(context)?;
    walk_elements(&node, context, &mut |el, context| {
        let name = string(Some(&get(el, "nodeName", context)), context);
        if all || name.to_ascii_lowercase() == wanted {
            let _ = out.push(el.clone(), context);
        }
        false
    });
    Ok(out.into())
}

fn get_element_by_id(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = this_node(this)?;
    let wanted = string(args.first(), context);
    let mut found = JsValue::null();
    walk_elements(&node, context, &mut |el, context| {
        let id = find_attr(el, "id", context)
            .map(|(_, _, a)| string(Some(&get(&a, "value", context)), context));
        if id.as_deref() == Some(wanted.as_str()) {
            found = el.clone().into();
            return true;
        }
        false
    });
    Ok(found)
}

fn owner_of(node: &JsObject, context: &mut Context) -> JsObject {
    if kind_of(node, context) == DOCUMENT {
        node.clone()
    } else {
        get(node, "ownerDocument", context)
            .as_object()
            .unwrap_or_else(|| node.clone())
    }
}

fn create_element(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let doc = this_node(this)?;
    let owner = owner_of(&doc, context);
    let name = string(args.first(), context);
    Ok(new_node(context, ELEMENT, &name, Some(&owner)).into())
}

fn create_element_ns(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let doc = this_node(this)?;
    let owner = owner_of(&doc, context);
    let name = string(args.get(1), context);
    Ok(new_node(context, ELEMENT, &name, Some(&owner)).into())
}

fn create_text_node(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let doc = this_node(this)?;
    let owner = owner_of(&doc, context);
    let value = string(args.first(), context);
    Ok(text_node(context, TEXT, &value, &owner).into())
}

fn create_comment(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let doc = this_node(this)?;
    let owner = owner_of(&doc, context);
    let value = string(args.first(), context);
    Ok(text_node(context, COMMENT, &value, &owner).into())
}

fn create_fragment(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let doc = this_node(this)?;
    let owner = owner_of(&doc, context);
    Ok(new_node(context, FRAGMENT, "#document-fragment", Some(&owner)).into())
}

fn serialize(node: &JsObject, context: &mut Context, out: &mut String) {
    match kind_of(node, context) {
        TEXT => {
            let text = string(Some(&get(node, "nodeValue", context)), context);
            out.push_str(
                &text
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;"),
            );
        }
        CDATA => {
            out.push_str("<![CDATA[");
            out.push_str(&string(Some(&get(node, "nodeValue", context)), context));
            out.push_str("]]>");
        }
        COMMENT => {
            out.push_str("<!--");
            out.push_str(&string(Some(&get(node, "nodeValue", context)), context));
            out.push_str("-->");
        }
        kind => {
            let mut inner = String::new();
            if let Some(list) = child_nodes(node, context) {
                for child in items(&list, context) {
                    if let Some(child) = child.as_object() {
                        serialize(&child, context, &mut inner);
                    }
                }
            }
            if kind != ELEMENT {
                out.push_str(&inner);
                return;
            }
            let name = string(Some(&get(node, "nodeName", context)), context);
            let mut open = name.clone();
            if let Some(attrs) = attrs_of(node, context) {
                for attr in items(&attrs, context) {
                    if let Some(attr) = attr.as_object() {
                        let k = string(Some(&get(&attr, "name", context)), context);
                        let v = string(Some(&get(&attr, "value", context)), context);
                        open.push(' ');
                        open.push_str(&k);
                        open.push_str("=\"");
                        open.push_str(&v.replace('"', "&quot;"));
                        open.push('"');
                    }
                }
            }
            if inner.is_empty() {
                out.push('<');
                out.push_str(&open);
                out.push_str("/>");
            } else {
                out.push('<');
                out.push_str(&open);
                out.push('>');
                out.push_str(&inner);
                out.push_str("</");
                out.push_str(&name);
                out.push('>');
            }
        }
    }
}

fn serialize_to_string(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(node) = args.first().and_then(JsValue::as_object) else {
        return Ok(js(""));
    };
    let mut out = String::new();
    serialize(&node, context, &mut out);
    Ok(js(&out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use boa_engine::Source;

    #[test]
    fn an_xml_document_is_read() {
        let mut context = Context::default();
        install(&mut context);
        let code = r#"
            var d = new DOMParser().parseFromString(
                '<?xml version="1.0"?><top><annouce><timeFMT>16</timeFMT></annouce>' +
                '<spot codeCsv="44131"><name>&#26550;&#31354;&#21306;</name>' +
                '<note><![CDATA[a<b]]></note></spot><spot codeCsv="1"/></top>',
                "text/xml");
            [
                d.documentElement.nodeName,
                d.getElementsByTagName("parsererror").length,
                d.getElementsByTagName("timeFMT")[0].textContent,
                d.getElementsByTagName("spot").length,
                d.getElementsByTagName("spot")[0].getAttribute("codeCsv"),
                d.getElementsByTagName("name")[0].textContent,
                d.getElementsByTagName("note")[0].textContent,
                d.getElementsByTagName("spot")[1].nextSibling === null,
                new DOMParser().parseFromString("<a", "text/xml")
                    .getElementsByTagName("parsererror").length
            ].join("|")
        "#;
        let got = context
            .eval(Source::from_bytes(code.as_bytes()))
            .expect("読める")
            .to_string(&mut context)
            .expect("文字に")
            .to_std_string_escaped();
        assert_eq!(got, "top|0|16|2|44131|架空区|a<b|true|1");
    }

    #[test]
    fn the_tree_can_be_edited_and_written_back() {
        let mut context = Context::default();
        install(&mut context);
        let code = r#"
            var d = new DOMParser().parseFromString('<r><a id="x">1</a><b/></r>', "text/xml");
            var r = d.documentElement;
            var c = d.createElement("C");
            c.setAttribute("k", 'v"');
            c.appendChild(d.createTextNode("t&"));
            r.insertBefore(c, r.getElementsByTagName("b")[0]);
            r.removeChild(d.getElementById("x"));
            c.className = "i";
            [
                new XMLSerializer().serializeToString(d),
                r.children.length,
                r.firstChild === c,
                c.previousSibling === null,
                r.contains(c),
                c.parentNode === r,
                c.ownerDocument === d,
                c.hasAttribute("k"),
                c.getAttributeNode("k").value,
                c.localName
            ].join("|")
        "#;
        let got = context
            .eval(Source::from_bytes(code.as_bytes()))
            .expect("読める")
            .to_string(&mut context)
            .expect("文字に")
            .to_std_string_escaped();
        assert_eq!(
            got,
            "<r><C k=\"v&quot;\" class=\"i\">t&amp;</C><b/></r>|2|true|true|true|true|true|true|v\"|C"
        );
    }
}
