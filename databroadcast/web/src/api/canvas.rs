// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! `canvas` の `getContext("2d")`。Blitz に canvas は無いので、命令を [`crate::draw::Canvas`]
//! に記録し、custom widget で文書と一緒にレンダリングさせる。`img`・映像の `drawImage` と
//! `getImageData` は持たない（呼べるだけ）。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::draw::{Align, Baseline, Canvas, Fonts};
use anyrender::Scene;
use boa_engine::{Context, JsResult, JsValue};

use super::dom::undefined;
use super::{accessor, get, js, method, number, object, set, string, this_object, value};

thread_local! {
    static FONTS: RefCell<Option<Fonts>> = const { RefCell::new(None) };
    /// 番号 → canvas。JS の context オブジェクトは番号（`__zw_canvas`）で持つ。
    static CANVASES: RefCell<HashMap<u64, SharedCanvas>> = RefCell::new(HashMap::new());
    static NEXT: RefCell<u64> = const { RefCell::new(1) };
}

pub fn forget_canvases() {
    CANVASES.with(|all| all.borrow_mut().clear());
}

pub fn install_fonts(faces: &crate::fonts::Faces) {
    FONTS.with(|fonts| {
        if fonts.borrow().is_none() {
            *fonts.borrow_mut() = Some(Fonts::new(crate::fonts::context(faces)));
        }
    });
}

fn with_fonts<R>(f: impl FnOnce(&mut Fonts) -> R) -> Option<R> {
    FONTS.with(|fonts| fonts.borrow_mut().as_mut().map(f))
}

type SharedCanvas = Rc<RefCell<Canvas>>;

struct CanvasWidget {
    canvas: SharedCanvas,
}

impl blitz_dom::Widget for CanvasWidget {
    fn requires_redraw(&self) -> bool {
        true
    }

    fn paint(
        &mut self,
        _render_ctx: &mut dyn anyrender::RenderContext,
        _styles: &blitz_dom::node::ComputedStyles,
        width: u32,
        height: u32,
        _scale: f64,
    ) -> Scene {
        self.canvas
            .borrow()
            .scene_for(f64::from(width), f64::from(height))
    }
}

pub(super) fn install(element_proto: &boa_engine::JsObject, context: &mut Context) {
    method(element_proto, context, "getContext", 1, get_context);
    method(element_proto, context, "toDataURL", 1, to_data_url);
    let canvas_class = super::constructor(context, "HTMLCanvasElement", 0, undefined);
    let has_instance = super::function(context, "[Symbol.hasInstance]", 1, is_canvas);
    let _ = canvas_class.define_property_or_throw(
        boa_engine::property::PropertyKey::from(boa_engine::JsSymbol::has_instance()),
        boa_engine::property::PropertyDescriptor::builder()
            .value(has_instance)
            .writable(false)
            .enumerable(false)
            .configurable(true)
            .build(),
        context,
    );
    super::set_global(context, "HTMLCanvasElement", canvas_class.into());
    if super::global(context, "OffscreenCanvas").is_none() {
        let offscreen = super::constructor(context, "OffscreenCanvas", 2, offscreen_canvas);
        super::set_global(context, "OffscreenCanvas", offscreen.into());
    }
}

fn is_canvas(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(object) = args.first().and_then(JsValue::as_object) else {
        return Ok(JsValue::from(false));
    };
    let tag = string(Some(&get(&object, "tagName", context)), context);
    Ok(JsValue::from(tag == "CANVAS"))
}

fn offscreen_canvas(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(document) = super::global(context, "document") else {
        return Ok(JsValue::undefined());
    };
    let create = get(&document, "createElement", context);
    let Some(create) = create.as_callable() else {
        return Ok(JsValue::undefined());
    };
    let canvas = create.call(&document.into(), &[js("canvas")], context)?;
    if let Some(element) = canvas.as_object() {
        set(
            &element,
            "width",
            JsValue::from(number(args.first(), context)),
            context,
        );
        set(
            &element,
            "height",
            JsValue::from(number(args.get(1), context)),
            context,
        );
    }
    Ok(canvas)
}

fn to_data_url(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let element = this_object(this)?;
    let (width, height) = size_of(&element, context);
    let (width, height) = (width.floor() as u32, height.floor() as u32);
    // 出力の画像を確保するので、大きすぎる canvas は空で返す（20000² で約 1.6GB になる）。
    if width == 0 || height == 0 || !crate::picture::fits(width, height) {
        return Ok(js("data:,"));
    }
    let ctx = get(&element, "__zw_context", context);
    let canvas = ctx.as_object().and_then(|ctx| canvas_of(&ctx, context));
    let rgba = match canvas {
        Some(canvas) => {
            let scene = canvas
                .borrow()
                .scene_for(f64::from(width), f64::from(height));
            unpremultiply(anyrender::render_to_buffer::<
                anyrender_vello_cpu::VelloCpuImageRenderer,
                _,
            >(
                |painter| {
                    use anyrender::PaintScene as _;
                    painter.append_scene(scene, Default::default());
                },
                width,
                height,
            ))
        }
        None => vec![0; width as usize * height as usize * 4],
    };
    let png = crate::picture::Picture {
        width,
        height,
        rgba,
    }
    .png();
    Ok(js(&format!("data:image/png;base64,{}", base64(&png))))
}

fn unpremultiply(mut rgba: Vec<u8>) -> Vec<u8> {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = u32::from(pixel[3]);
        if alpha == 0 || alpha == 255 {
            continue;
        }
        for channel in &mut pixel[..3] {
            *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
        }
    }
    rgba
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(chunk.get(1).copied().unwrap_or(0)) << 8)
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn size_of(element: &boa_engine::JsObject, context: &mut Context) -> (f64, f64) {
    let read = |name: &str, context: &mut Context| -> Option<f64> {
        let got = get(element, name, context);
        if got.is_undefined() || got.is_null() {
            return attribute_of(element, name, context);
        }
        Some(number(Some(&got), context))
    };
    let width = read("width", context).unwrap_or(300.0).max(0.0);
    let height = read("height", context).unwrap_or(150.0).max(0.0);
    (width, height)
}

fn get_context(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let element = this_object(this)?;
    let kind = string(args.first(), context);
    if kind != "2d" {
        return Ok(JsValue::null());
    }
    let had = get(&element, "__zw_context", context);
    if had.is_object() {
        return Ok(had);
    }
    let (width, height) = size_of(&element, context);
    let canvas: SharedCanvas = Rc::new(RefCell::new(Canvas::new(width, height)));
    let id = NEXT.with(|next| {
        let mut next = next.borrow_mut();
        let id = *next;
        *next += 1;
        id
    });
    CANVASES.with(|all| all.borrow_mut().insert(id, Rc::clone(&canvas)));
    if let Some(node_id) = blitz_vibey_script::host::node_id_of(this)
        && let Some(doc) = blitz_vibey_script::host::document_of(context)
    {
        doc.borrow_mut().set_custom_widget(
            node_id,
            Box::new(CanvasWidget {
                canvas: Rc::clone(&canvas),
            }),
        );
    }
    // `canvas.width = n` は属性を書き、レンダリングした内容を消す（ブラウザと同じ）。
    accessor(&element, context, "width", Some(width_get), Some(width_set));
    accessor(
        &element,
        context,
        "height",
        Some(height_get),
        Some(height_set),
    );
    let ctx = object(context);
    value(&ctx, context, "canvas", element.clone().into());
    value(&ctx, context, "__zw_canvas", JsValue::from(id as f64));
    for (name, v) in [
        ("fillStyle", js("#000000")),
        ("strokeStyle", js("#000000")),
        ("font", js("10px sans-serif")),
        ("textAlign", js("start")),
        ("textBaseline", js("alphabetic")),
        ("globalAlpha", JsValue::from(1)),
        ("globalCompositeOperation", js("source-over")),
        ("lineWidth", JsValue::from(1)),
        ("lineCap", js("butt")),
        ("lineJoin", js("miter")),
        ("imageSmoothingEnabled", JsValue::from(true)),
    ] {
        value(&ctx, context, name, v);
    }
    let methods: [(&str, usize, super::Native); 34] = [
        ("fillRect", 4, fill_rect),
        ("strokeRect", 4, stroke_rect),
        ("clearRect", 4, clear_rect),
        ("fillText", 3, fill_text),
        ("strokeText", 3, stroke_text),
        ("measureText", 1, measure_text),
        ("drawImage", 3, draw_image),
        ("beginPath", 0, begin_path),
        ("closePath", 0, close_path),
        ("moveTo", 2, move_to),
        ("lineTo", 2, line_to),
        ("rect", 4, rect_path),
        ("arc", 5, arc),
        ("quadraticCurveTo", 4, quadratic_curve_to),
        ("bezierCurveTo", 6, bezier_curve_to),
        ("fill", 0, fill_path),
        ("stroke", 0, stroke_path),
        ("clip", 0, clip),
        ("save", 0, save),
        ("restore", 0, restore),
        ("translate", 2, translate),
        ("scale", 2, scale),
        ("rotate", 1, rotate),
        ("transform", 6, transform),
        ("setTransform", 6, set_transform),
        ("resetTransform", 0, reset_transform),
        ("getTransform", 0, get_transform),
        ("getImageData", 4, image_data),
        ("createImageData", 2, image_data),
        ("createLinearGradient", 4, gradient),
        ("createRadialGradient", 6, gradient),
        ("createPattern", 2, gradient),
        ("isPointInPath", 2, r#false),
        ("arcTo", 0, undefined),
    ];
    for (name, n, f) in methods {
        method(&ctx, context, name, n, f);
    }
    for name in ["putImageData", "setLineDash"] {
        method(&ctx, context, name, 0, undefined);
    }
    value(&element, context, "__zw_context", ctx.clone().into());
    Ok(ctx.into())
}

fn r#false(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(false))
}

fn attribute_of(element: &boa_engine::JsObject, name: &str, context: &mut Context) -> Option<f64> {
    let attr = element
        .get(boa_engine::js_string!("getAttribute"), context)
        .ok()?;
    let got = attr
        .as_callable()?
        .call(&element.clone().into(), &[js(name)], context)
        .ok()?;
    if got.is_null() || got.is_undefined() {
        return None;
    }
    string(Some(&got), context).trim().parse().ok()
}

fn set_attribute_of(element: &boa_engine::JsObject, name: &str, v: f64, context: &mut Context) {
    if let Ok(attr) = element.get(boa_engine::js_string!("setAttribute"), context)
        && let Some(f) = attr.as_callable()
    {
        let _ = f.call(
            &element.clone().into(),
            &[js(name), js(&format!("{}", v.max(0.0).floor() as i64))],
            context,
        );
    }
}

fn width_get(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let element = this_object(this)?;
    Ok(JsValue::from(
        attribute_of(&element, "width", context).unwrap_or(300.0),
    ))
}

fn height_get(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let element = this_object(this)?;
    Ok(JsValue::from(
        attribute_of(&element, "height", context).unwrap_or(150.0),
    ))
}

fn resize(
    this: &JsValue,
    name: &str,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let element = this_object(this)?;
    let v = number(args.first(), context).max(0.0).floor();
    set_attribute_of(&element, name, v, context);
    let ctx = get(&element, "__zw_context", context);
    if let Some(ctx) = ctx.as_object()
        && let Some(canvas) = canvas_of(&ctx, context)
    {
        {
            let mut canvas = canvas.borrow_mut();
            let (w, h) = (canvas.width(), canvas.height());
            if name == "width" {
                canvas.resize(v, h);
            } else {
                canvas.resize(w, v);
            }
        }
        write_back(&ctx, &canvas.borrow(), context);
    }
    Ok(JsValue::undefined())
}

fn width_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    resize(this, "width", args, context)
}

fn height_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    resize(this, "height", args, context)
}

fn canvas_of(ctx: &boa_engine::JsObject, context: &mut Context) -> Option<SharedCanvas> {
    let id = number(Some(&get(ctx, "__zw_canvas", context)), context) as u64;
    CANVASES.with(|all| all.borrow().get(&id).cloned())
}

fn this_canvas(
    this: &JsValue,
    context: &mut Context,
) -> JsResult<(boa_engine::JsObject, SharedCanvas)> {
    let ctx = this_object(this)?;
    let canvas = canvas_of(&ctx, context)
        .ok_or_else(|| boa_engine::JsNativeError::typ().with_message("not a canvas context"))?;
    {
        let text =
            |name: &str, context: &mut Context| string(Some(&get(&ctx, name, context)), context);
        let num =
            |name: &str, context: &mut Context| number(Some(&get(&ctx, name, context)), context);
        let fill = crate::draw::colour(&text("fillStyle", context));
        let stroke = crate::draw::colour(&text("strokeStyle", context));
        let font = text("font", context);
        let align = Align::parse(&text("textAlign", context));
        let baseline = Baseline::parse(&text("textBaseline", context));
        let alpha = num("globalAlpha", context);
        let width = num("lineWidth", context);
        let mut canvas = canvas.borrow_mut();
        let state = &mut canvas.state;
        state.fill = fill;
        state.stroke = stroke;
        state.font = font;
        state.align = align;
        state.baseline = baseline;
        state.alpha = if alpha.is_finite() {
            alpha.clamp(0.0, 1.0) as f32
        } else {
            1.0
        };
        state.line_width = if width.is_finite() && width > 0.0 {
            width
        } else {
            1.0
        };
    }
    Ok((ctx, canvas))
}

fn write_back(ctx: &boa_engine::JsObject, canvas: &Canvas, context: &mut Context) {
    let colour = |c: peniko::Color| {
        let [r, g, b, a] = c.to_rgba8().to_u8_array();
        format!("rgba({r}, {g}, {b}, {})", f64::from(a) / 255.0)
    };
    let state = &canvas.state;
    set(ctx, "fillStyle", js(&colour(state.fill)), context);
    set(ctx, "strokeStyle", js(&colour(state.stroke)), context);
    set(ctx, "font", js(&state.font), context);
    let align = match state.align {
        Align::Start => "start",
        Align::Center => "center",
        Align::End => "end",
    };
    set(ctx, "textAlign", js(align), context);
    let baseline = match state.baseline {
        Baseline::Alphabetic => "alphabetic",
        Baseline::Top => "top",
        Baseline::Middle => "middle",
        Baseline::Bottom => "bottom",
    };
    set(ctx, "textBaseline", js(baseline), context);
    set(
        ctx,
        "globalAlpha",
        JsValue::from(f64::from(state.alpha)),
        context,
    );
    set(ctx, "lineWidth", JsValue::from(state.line_width), context);
}

fn arg(args: &[JsValue], n: usize, context: &mut Context) -> f64 {
    let v = number(args.get(n), context);
    if v.is_finite() { v } else { 0.0 }
}

macro_rules! op {
    ($name:ident, $count:literal, |$canvas:ident, $a:ident| $body:expr) => {
        fn $name(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
            let (_, canvas) = this_canvas(this, context)?;
            let $a: Vec<f64> = (0..$count).map(|n| arg(args, n, context)).collect();
            let mut $canvas = canvas.borrow_mut();
            $body;
            Ok(JsValue::undefined())
        }
    };
}

op!(fill_rect, 4, |c, a| c.fill_rect(a[0], a[1], a[2], a[3]));
op!(stroke_rect, 4, |c, a| c.stroke_rect(a[0], a[1], a[2], a[3]));
op!(clear_rect, 4, |c, a| c.clear_rect(a[0], a[1], a[2], a[3]));
op!(move_to, 2, |c, a| c.move_to(a[0], a[1]));
op!(line_to, 2, |c, a| c.line_to(a[0], a[1]));
op!(rect_path, 4, |c, a| c.rect(a[0], a[1], a[2], a[3]));
op!(quadratic_curve_to, 4, |c, a| c
    .quad_to(a[0], a[1], a[2], a[3]));
op!(bezier_curve_to, 6, |c, a| c
    .bezier_to(a[0], a[1], a[2], a[3], a[4], a[5]));
op!(translate, 2, |c, a| c.translate(a[0], a[1]));
op!(rotate, 1, |c, a| c.rotate(a[0]));
op!(transform, 6, |c, a| c
    .transform([a[0], a[1], a[2], a[3], a[4], a[5]]));
op!(begin_path, 0, |c, _a| c.begin_path());
op!(close_path, 0, |c, _a| c.close_path());
op!(fill_path, 0, |c, _a| c.fill());
op!(stroke_path, 0, |c, _a| c.stroke());
op!(clip, 0, |c, _a| c.clip());
op!(save, 0, |c, _a| c.save());
op!(reset_transform, 0, |c, _a| c
    .set_transform([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]));

fn scale(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (_, canvas) = this_canvas(this, context)?;
    let x = arg(args, 0, context);
    let y = if args.len() > 1 {
        arg(args, 1, context)
    } else {
        x
    };
    canvas.borrow_mut().scale(x, y);
    Ok(JsValue::undefined())
}

fn arc(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (_, canvas) = this_canvas(this, context)?;
    let a: Vec<f64> = (0..5).map(|n| arg(args, n, context)).collect();
    let anticlockwise = args.get(5).is_some_and(|v| v.to_boolean());
    canvas
        .borrow_mut()
        .arc(a[0], a[1], a[2], a[3], a[4], anticlockwise);
    Ok(JsValue::undefined())
}

fn restore(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (ctx, canvas) = this_canvas(this, context)?;
    let restored = canvas.borrow_mut().restore();
    if restored {
        write_back(&ctx, &canvas.borrow(), context);
    }
    Ok(JsValue::undefined())
}

fn set_transform(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (_, canvas) = this_canvas(this, context)?;
    let mut m = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    if args.len() >= 6 {
        for (n, slot) in m.iter_mut().enumerate() {
            *slot = arg(args, n, context);
        }
    } else if let Some(matrix) = args.first().and_then(JsValue::as_object) {
        for (slot, name) in m.iter_mut().zip(["a", "b", "c", "d", "e", "f"]) {
            let v = number(Some(&get(&matrix, name, context)), context);
            if v.is_finite() {
                *slot = v;
            }
        }
    }
    canvas.borrow_mut().set_transform(m);
    Ok(JsValue::undefined())
}

fn get_transform(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (_, canvas) = this_canvas(this, context)?;
    let m = canvas.borrow().get_transform();
    let out = object(context);
    for (name, v) in ["a", "b", "c", "d", "e", "f"].iter().zip(m) {
        value(&out, context, name, JsValue::from(v));
    }
    Ok(out.into())
}

fn draw_text(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
    stroke: bool,
) -> JsResult<JsValue> {
    let (_, canvas) = this_canvas(this, context)?;
    let text = string(args.first(), context);
    if text.is_empty() {
        return Ok(JsValue::undefined());
    }
    let (x, y) = (arg(args, 1, context), arg(args, 2, context));
    let mut canvas = canvas.borrow_mut();
    with_fonts(|fonts| {
        if stroke {
            canvas.stroke_text(fonts, &text, x, y);
        } else {
            canvas.fill_text(fonts, &text, x, y);
        }
    });
    Ok(JsValue::undefined())
}

fn fill_text(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    draw_text(this, args, context, false)
}

fn stroke_text(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    draw_text(this, args, context, true)
}

fn measure_text(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (_, canvas) = this_canvas(this, context)?;
    let text = string(args.first(), context);
    let line = {
        let canvas = canvas.borrow();
        with_fonts(|fonts| canvas.measure_text(fonts, &text)).unwrap_or(crate::draw::Line {
            ascent: 0.0,
            descent: 0.0,
            baseline: 0.0,
            width: 0.0,
        })
    };
    let (width, ascent, descent) = (
        f64::from(line.width),
        f64::from(line.ascent),
        f64::from(line.descent),
    );
    let metrics = object(context);
    for (name, v) in [
        ("width", width),
        ("actualBoundingBoxAscent", ascent),
        ("actualBoundingBoxDescent", descent),
        ("fontBoundingBoxAscent", ascent),
        ("fontBoundingBoxDescent", descent),
        ("actualBoundingBoxLeft", 0.0),
        ("actualBoundingBoxRight", width),
    ] {
        value(&metrics, context, name, JsValue::from(v));
    }
    Ok(metrics.into())
}

fn draw_image(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (_, canvas) = this_canvas(this, context)?;
    let Some(source) = args.first().and_then(JsValue::as_object) else {
        return Ok(JsValue::undefined());
    };
    let source_ctx = get(&source, "__zw_context", context);
    let Some(source_ctx) = source_ctx.as_object() else {
        return Ok(JsValue::undefined());
    };
    let Some(from) = canvas_of(&source_ctx, context) else {
        return Ok(JsValue::undefined());
    };
    if Rc::ptr_eq(&from, &canvas) {
        return Ok(JsValue::undefined());
    }
    let a: Vec<f64> = (1..args.len()).map(|n| arg(args, n, context)).collect();
    let from = from.borrow();
    let (sw, sh) = (from.width(), from.height());
    let mut canvas = canvas.borrow_mut();
    match a.len() {
        8 => canvas.draw_canvas(&from, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]),
        4 => canvas.draw_canvas(&from, 0.0, 0.0, sw, sh, a[0], a[1], a[2], a[3]),
        2 => canvas.draw_canvas_at(&from, a[0], a[1], None),
        _ => {}
    }
    Ok(JsValue::undefined())
}

fn image_data(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (w, h) = match args.len() {
        4 => (args[2].to_number(context)?, args[3].to_number(context)?),
        _ => (
            args.first().map_or(Ok(0.0), |v| v.to_number(context))?,
            args.get(1).map_or(Ok(0.0), |v| v.to_number(context))?,
        ),
    };
    let data = object(context);
    let count = (w.max(0.0) * h.max(0.0) * 4.0) as usize;
    let array = boa_engine::object::builtins::JsArray::from_iter(
        std::iter::repeat_n(JsValue::from(0), count.min(1 << 20)),
        context,
    );
    value(&data, context, "width", JsValue::from(w));
    value(&data, context, "height", JsValue::from(h));
    value(&data, context, "data", array.into());
    Ok(data.into())
}

fn gradient(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let g = object(context);
    method(&g, context, "addColorStop", 2, undefined);
    Ok(g.into())
}

#[cfg(test)]
mod class_tests {
    use super::*;

    #[test]
    fn the_canvas_class_answers_instanceof_and_offscreen_makes_one() {
        let mut context = Context::default();
        let setup = r##"
            globalThis.document = {
                createElement: function (tag) { return { tagName: String(tag).toUpperCase() }; }
            };
        "##;
        context
            .eval(boa_engine::Source::from_bytes(setup.as_bytes()))
            .expect("走る");
        let proto = object(&mut context);
        install(&proto, &mut context);
        let code = r##"
            var c = new OffscreenCanvas(320, 240);
            [c instanceof HTMLCanvasElement, ({}) instanceof HTMLCanvasElement,
             c.width, c.height, typeof HTMLCanvasElement].join("|")
        "##;
        let got = context
            .eval(boa_engine::Source::from_bytes(code.as_bytes()))
            .expect("走る")
            .to_string(&mut context)
            .expect("文字に")
            .to_std_string_escaped();
        assert_eq!(got, "true|false|320|240|function");
    }
}

#[cfg(test)]
mod tests {
    use super::{base64, unpremultiply};

    #[test]
    fn base64_pads_to_four_characters() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe]), "//4=");
    }

    #[test]
    fn unpremultiply_restores_straight_colour() {
        assert_eq!(
            unpremultiply(vec![64, 0, 32, 128, 255, 0, 0, 255, 9, 9, 9, 0]),
            vec![128, 0, 64, 128, 255, 0, 0, 255, 9, 9, 9, 0]
        );
    }
}
