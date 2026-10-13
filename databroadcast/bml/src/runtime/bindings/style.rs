// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! `style`・`focusStyle`・`activeStyle` のスタイルオブジェクトと、そのプロパティの読み書き。

use super::*;

const STYLE_NAMES: &[(&str, &str)] = &[
    ("left", "left"),
    ("top", "top"),
    ("width", "width"),
    ("height", "height"),
    ("paddingTop", "padding-top"),
    ("paddingRight", "padding-right"),
    ("paddingBottom", "padding-bottom"),
    ("paddingLeft", "padding-left"),
    ("borderWidth", "border-width"),
    ("borderStyle", "border-style"),
    ("borderTopColorIndex", "border-top-color-index"),
    ("borderRightColorIndex", "border-right-color-index"),
    ("borderBottomColorIndex", "border-bottom-color-index"),
    ("borderLeftColorIndex", "border-left-color-index"),
    ("visibility", "visibility"),
    ("fontSize", "font-size"),
    ("fontFamily", "font-family"),
    ("fontWeight", "font-weight"),
    ("lineHeight", "line-height"),
    ("letterSpacing", "letter-spacing"),
    ("textAlign", "text-align"),
    ("colorIndex", "color-index"),
    ("backgroundColorIndex", "background-color-index"),
    ("grayscaleColorIndex", "grayscale-color-index"),
    ("clut", "clut"),
    ("resolution", "resolution"),
    ("displayAspectRatio", "display-aspect-ratio"),
    ("navIndex", "nav-index"),
    ("navUp", "nav-up"),
    ("navDown", "nav-down"),
    ("navLeft", "nav-left"),
    ("navRight", "nav-right"),
    ("usedKeyList", "used-key-list"),
    ("color", "color"),
    ("backgroundColor", "background-color"),
    ("borderColor", "border-color"),
    ("borderTopColor", "border-top-color"),
    ("borderRightColor", "border-right-color"),
    ("borderBottomColor", "border-bottom-color"),
    ("borderLeftColor", "border-left-color"),
    ("borderColorIndex", "border-color-index"),
    ("backgroundImage", "background-image"),
    ("backgroundRepeat", "background-repeat"),
    ("zIndex", "z-index"),
    ("whiteSpace", "white-space"),
    ("display", "display"),
    ("padding", "padding"),
    ("margin", "margin"),
    // BMLCSS2Properties の残り（STD-B24 第二編 5.3）。読めば ""、書けば属性に残る。
    ("azimuth", "azimuth"),
    ("background", "background"),
    ("backgroundAttachment", "background-attachment"),
    ("backgroundPosition", "background-position"),
    ("border", "border"),
    ("borderBottom", "border-bottom"),
    ("borderBottomStyle", "border-bottom-style"),
    ("borderBottomWidth", "border-bottom-width"),
    ("borderCollapse", "border-collapse"),
    ("borderLeft", "border-left"),
    ("borderLeftStyle", "border-left-style"),
    ("borderLeftWidth", "border-left-width"),
    ("borderRight", "border-right"),
    ("borderRightStyle", "border-right-style"),
    ("borderRightWidth", "border-right-width"),
    ("borderSpacing", "border-spacing"),
    ("borderTop", "border-top"),
    ("borderTopStyle", "border-top-style"),
    ("borderTopWidth", "border-top-width"),
    ("bottom", "bottom"),
    ("captionSide", "caption-side"),
    ("clear", "clear"),
    ("clip", "clip"),
    ("content", "content"),
    ("counterIncrement", "counter-increment"),
    ("counterReset", "counter-reset"),
    ("cssFloat", "float"),
    ("cue", "cue"),
    ("cueAfter", "cue-after"),
    ("cueBefore", "cue-before"),
    ("cursor", "cursor"),
    ("direction", "direction"),
    ("elevation", "elevation"),
    ("emptyCells", "empty-cells"),
    ("font", "font"),
    ("fontSizeAdjust", "font-size-adjust"),
    ("fontStretch", "font-stretch"),
    ("fontStyle", "font-style"),
    ("fontVariant", "font-variant"),
    ("listStyle", "list-style"),
    ("listStyleImage", "list-style-image"),
    ("listStylePosition", "list-style-position"),
    ("listStyleType", "list-style-type"),
    ("marginBottom", "margin-bottom"),
    ("marginLeft", "margin-left"),
    ("marginRight", "margin-right"),
    ("marginTop", "margin-top"),
    ("markerOffset", "marker-offset"),
    ("marks", "marks"),
    ("maxHeight", "max-height"),
    ("maxWidth", "max-width"),
    ("minHeight", "min-height"),
    ("minWidth", "min-width"),
    ("orphans", "orphans"),
    ("outline", "outline"),
    ("outlineColor", "outline-color"),
    ("outlineColorIndex", "outline-color-index"),
    ("outlineStyle", "outline-style"),
    ("outlineWidth", "outline-width"),
    ("overflow", "overflow"),
    ("page", "page"),
    ("pageBreakAfter", "page-break-after"),
    ("pageBreakBefore", "page-break-before"),
    ("pageBreakInside", "page-break-inside"),
    ("pause", "pause"),
    ("pauseAfter", "pause-after"),
    ("pauseBefore", "pause-before"),
    ("pitch", "pitch"),
    ("pitchRange", "pitch-range"),
    ("playDuring", "play-during"),
    ("position", "position"),
    ("quotes", "quotes"),
    ("retained", "retained"),
    ("richness", "richness"),
    ("right", "right"),
    ("size", "size"),
    ("speak", "speak"),
    ("speakHeader", "speak-header"),
    ("speakNumeral", "speak-numeral"),
    ("speakPunctuation", "speak-punctuation"),
    ("speechRate", "speech-rate"),
    ("stress", "stress"),
    ("tableLayout", "table-layout"),
    ("textDecoration", "text-decoration"),
    ("textIndent", "text-indent"),
    ("textShadow", "text-shadow"),
    ("textTransform", "text-transform"),
    ("unicodeBidi", "unicode-bidi"),
    ("verticalAlign", "vertical-align"),
    ("voiceFamily", "voice-family"),
    ("volume", "volume"),
    ("wapAccesskey", "-wap-accesskey"),
    ("wapInputFormat", "-wap-input-format"),
    ("wapInputRequired", "-wap-input-required"),
    ("wapMarqueeDir", "-wap-marquee-dir"),
    ("wapMarqueeLoop", "-wap-marquee-loop"),
    ("wapMarqueeSpeed", "-wap-marquee-speed"),
    ("wapMarqueeStyle", "-wap-marquee-style"),
    ("widows", "widows"),
    ("wordSpacing", "word-spacing"),
];

pub(super) fn style_get(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    style_object(this, context, 0)
}

pub(super) fn focus_style_get(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    style_object(this, context, 1)
}

pub(super) fn active_style_get(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    style_object(this, context, 2)
}

/// スタイルオブジェクト。`__m` がどのスタイルか（0 normal、1 focus、2 active）。
fn style_object(this: &JsValue, context: &mut Context, mode: u8) -> JsResult<JsValue> {
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::undefined());
    };
    let mut object = ObjectInitializer::new(context);
    object.property(
        js_string!("__n"),
        JsValue::from(id.0 as f64),
        Attribute::all(),
    );
    object.property(js_string!("__m"), JsValue::from(mode), Attribute::all());
    for (js_name, css_name) in STYLE_NAMES {
        let captured = JsString::from(*css_name);
        let get = FunctionObjectBuilder::new(
            object.context().realm(),
            NativeFunction::from_copy_closure_with_captures(
                |this, _, name, context| style_property_get(this, name, context),
                captured.clone(),
            ),
        )
        .build();
        let set = FunctionObjectBuilder::new(
            object.context().realm(),
            NativeFunction::from_copy_closure_with_captures(style_property_set, captured),
        )
        .build();
        object.accessor(
            JsString::from(*js_name),
            Some(get),
            Some(set),
            Attribute::all(),
        );
    }
    object.function(
        NativeFunction::from_fn_ptr(style_get_property_value),
        js_string!("getPropertyValue"),
        1,
    );
    object.function(
        NativeFunction::from_fn_ptr(style_set_property),
        js_string!("setProperty"),
        2,
    );
    Ok(object.build().into())
}

fn style_property_get(this: &JsValue, name: &JsString, context: &mut Context) -> JsResult<JsValue> {
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::undefined());
    };
    let name = name.to_std_string_escaped();
    let mode = style_mode(this, context);
    let host = host(context);
    let Some(page) = &mut host.page else {
        return Ok(JsValue::undefined());
    };
    let own = match mode {
        1 => Some(&page.focus_overrides),
        2 => Some(&page.active_overrides),
        _ => None,
    };
    if let Some(declared) = own
        .and_then(|map| map.get(&id))
        .and_then(|list| list.iter().rev().find(|d| d.name == name))
    {
        return Ok(js(&declared.value.to_css()));
    }
    if let Some(declared) = page
        .overrides
        .get(&id)
        .and_then(|list| list.iter().rev().find(|d| d.name == name))
    {
        return Ok(js(&declared.value.to_css()));
    }
    if let Some(inline) = page
        .document
        .element(id)
        .and_then(|e| e.attribute("style"))
        .map(css::parse_declarations)
        .and_then(|list| list.into_iter().rev().find(|d| d.name == name))
        && inline.value.keyword() != Some("inherit")
    {
        return Ok(js(&inline.value.to_css()));
    }
    if page.dirty {
        page.styles = crate::document::style::compute_with(
            &page.document,
            &page.sheets,
            &crate::document::style::Scripted {
                normal: Some(&page.overrides),
                focus: Some(&page.focus_overrides),
                focused: page.focus,
            },
        );
    }
    let value = page
        .styles
        .get(id.0)
        .map(|style| style.get(&name))
        .unwrap_or_default();
    Ok(js(&value))
}

fn style_property_set(
    this: &JsValue,
    args: &[JsValue],
    name: &JsString,
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(id) = node_of(this, context) else {
        return Ok(JsValue::undefined());
    };
    let value = string(args.first(), context);
    let name = name.to_std_string_escaped();
    let mode = style_mode(this, context);
    let host = host(context);
    if let Some(page) = &mut host.page {
        let parsed = css::parse_declarations(&format!("{name}:{value}"));
        let map = match mode {
            1 => &mut page.focus_overrides,
            2 => &mut page.active_overrides,
            _ => &mut page.overrides,
        };
        let list = map.entry(id).or_default();
        list.retain(|d| d.name != name);
        list.extend(parsed);
        page.dirty = true;
    }
    Ok(JsValue::undefined())
}

fn style_get_property_value(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = JsString::from(string(args.first(), context));
    style_property_get(this, &name, context)
}

fn style_set_property(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = JsString::from(string(args.first(), context));
    style_property_set(this, &args[1.min(args.len())..], &name, context)
}
