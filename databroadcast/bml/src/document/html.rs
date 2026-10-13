// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 文書を HTML に。算出したスタイル（[`crate::document::style`]）を Blitz が読む CSS に変換する。
//! 文字は丸ゴシック・角ゴシック（[`ROUND`]・[`SQUARE`]）で、無い文字は等幅の日本語フォント（[`FAMILY`]）でレンダリングする。

use std::collections::BTreeMap;

use crate::document::dom::{Document, Kind, NodeId};
use crate::document::style::{Align, BorderStyle, Display, Family, Style, Visibility, WhiteSpace};
use crate::media::clut::{Clut, Colour};

pub const IMAGE_BASE: &str = "arib-bml://res/";

/// 等幅の日本語フォントの名前。`style` 属性は二重引用符で括るので一重引用符。
pub const FAMILY: &str =
    "'zerowave-bml', 'MS Gothic', 'Noto Sans Mono CJK JP', 'Osaka-Mono', monospace";

pub const ROUND: &str = "zerowave-bml-round";
pub const SQUARE: &str = "zerowave-bml-square";

/// 太いウェイトを持っているフォント（丸 1、角 2 の和）。持たない物の太字は疑似太字（一画素ずらして二回レンダリング）にする。
static BOLD_FACES: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub fn set_bold_faces(round: bool, square: bool) {
    BOLD_FACES.store(
        u8::from(round) | (u8::from(square) << 1),
        std::sync::atomic::Ordering::Relaxed,
    );
}

fn has_bold_face(family: Family) -> bool {
    let bit = match family {
        Family::RoundGothic => 1,
        Family::SquareGothic => 2,
    };
    BOLD_FACES.load(std::sync::atomic::Ordering::Relaxed) & bit != 0
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Html {
    pub markup: String,
    pub size: (u32, u32),
    /// 画面で見せる形（`display-aspect-ratio`）。SD は 720×480 を 16:9 に伸ばす。
    pub aspect: Option<(u32, u32)>,
    pub images: Vec<String>,
    pub made: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug)]
pub struct Scene<'a> {
    pub document: &'a Document,
    pub styles: &'a [Style],
    pub clut: &'a Clut,
    pub marquees: &'a BTreeMap<NodeId, f32>,
    pub frames: &'a BTreeMap<NodeId, usize>,
    pub drcs: &'a [crate::media::drcs::Glyph],
    pub sizes: &'a BTreeMap<String, (u32, u32)>,
}

/// 箱の中に置く画像の大きさ。画像は左上にそのまま置く（伸ばすと文字が潰れる）。
/// JPEG だけは 960×540 の文書に 1920×1080 で来るので半分に。720×480 では幅を偶数に切る。
fn picture_css(
    kind: &str,
    size: Option<(u32, u32)>,
    resolution: (u32, u32),
    boxed: (f32, f32),
) -> String {
    let Some((width, height)) = size else {
        return String::new();
    };
    let (width, height) = if kind == "image/jpeg" && !crate::runtime::profile::is_c() {
        if resolution == (720, 480) {
            (width & !1, height)
        } else if boxed == (960.0, 540.0) && (width, height) == (960, 540) {
            (width, height)
        } else {
            (width / 2, height / 2)
        }
    } else {
        (width, height)
    };
    format!("width:{width}px;height:{height}px;")
}

/// body の背景画像（STD-B24 第二分冊 付属2 A2-4.4.7：JPEG だけ、画面の左上から、background-repeat は
/// 固定値 repeat）の大きさ。JPEG は静止画プレーン（1920×1080）の画素で描くので、960×540 の文書では
/// 半分にする。TR-B14 第二編 表 1-4 で JPEG の拡大率は 128/128、256/128 は 960×540 の画像だけ。
/// 720×480 の文書では静止画プレーンと文書の画素が同じ。
fn background_css(size: Option<(u32, u32)>, resolution: (u32, u32)) -> String {
    let Some((width, height)) = size else {
        return String::new();
    };
    let (width, height) = (width as f32, height as f32);
    let (width, height) = if crate::runtime::profile::is_c()
        || resolution == (720, 480)
        || (width, height) == (960.0, 540.0)
    {
        (width, height)
    } else {
        (width / 2.0, height / 2.0)
    };
    format!("background-size:{width}px {height}px;")
}

pub fn picture_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) && bytes.len() >= 10 {
        let width = u32::from(u16::from_le_bytes([bytes[6], bytes[7]]));
        let height = u32::from(u16::from_le_bytes([bytes[8], bytes[9]]));
        return Some((width, height));
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 {
        let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        return Some((width, height));
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        let mut at = 2;
        while at + 4 <= bytes.len() {
            if bytes[at] != 0xff {
                at += 1;
                continue;
            }
            let marker = bytes[at + 1];
            if marker == 0xff {
                at += 1;
                continue;
            }
            if (0xd0..=0xd9).contains(&marker) || marker == 0x01 {
                at += 2;
                continue;
            }
            let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
            if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
                if at + 9 > bytes.len() {
                    return None;
                }
                let height = u32::from(u16::from_be_bytes([bytes[at + 5], bytes[at + 6]]));
                let width = u32::from(u16::from_be_bytes([bytes[at + 7], bytes[at + 8]]));
                return Some((width, height));
            }
            at += 2 + length;
        }
    }
    None
}

pub fn render(scene: &Scene<'_>) -> Html {
    let (document, styles, clut) = (scene.document, scene.styles, scene.clut);
    let mut out = Html::default();
    let Some(body) = document.first_named("body") else {
        return out;
    };
    let (width, height) = styles[body.0].resolution.unwrap_or((960, 540));
    out.size = (width, height);
    out.aspect = styles[body.0].aspect.filter(|(w, h)| *w > 0 && *h > 0);
    let mut markup = String::with_capacity(4096);
    markup.push_str("<!DOCTYPE html><html><head><meta charset=\"utf-8\"><style>");
    markup.push_str(
        "html,body{margin:0;padding:0}\
         body{position:relative;overflow:hidden}\
         div,p,object,input,img{position:absolute;box-sizing:content-box;overflow:hidden;margin:0;padding:0}\
         p{white-space:pre-wrap;word-break:break-all}\
         span{white-space:pre-wrap}",
    );
    markup.push_str("</style></head>");
    let style = &styles[body.0];
    let mut body_css = format!(
        "width:{width}px;height:{height}px;background-color:{};{}",
        colour_css(clut.colour(style.background)),
        text_css(style, clut),
    );
    if let Some(image) = &style.background_image {
        out.images.push(image.clone());
        body_css.push_str(&format!(
            "background-image:url({IMAGE_BASE}{});background-repeat:repeat;{}",
            escape_attribute(image),
            background_css(scene.sizes.get(image).copied(), (width, height)),
        ));
    }
    markup.push_str(&format!("<body style=\"{body_css}\">"));
    for child in &document
        .element(body)
        .map(|e| e.children.clone())
        .unwrap_or_default()
    {
        block(scene, *child, &mut markup, &mut out);
    }
    markup.push_str("</body></html>");
    out.markup = markup;
    out
}

fn block(scene: &Scene<'_>, id: NodeId, out: &mut String, html: &mut Html) {
    let Scene {
        document,
        styles,
        clut,
        marquees,
        frames,
        sizes,
        ..
    } = *scene;
    let images = &mut html.images;
    let Some(element) = document.element(id) else {
        return;
    };
    let style = &styles[id.0];
    if style.display != Display::Block {
        return;
    }
    // `visibility` は継承し子は `visible` で覆せるが、Blitz は hidden の要素を子孫ごとレンダリングしないので、
    // 子を持つ箱は自分の塗りを捨てて visible で出す。
    let hidden = style.visibility == Visibility::Hidden;
    let container = !matches!(
        element.name.as_str(),
        "p" | "pre" | "input" | "textarea" | "object" | "img"
    );
    let mut css = format!(
        "left:{}px;top:{}px;width:{}px;height:{}px;",
        style.left, style.top, style.width, style.height
    );
    if hidden && !container {
        css.push_str("visibility:hidden;");
    } else {
        css.push_str("visibility:visible;");
    }
    if style.z_index != 0 {
        css.push_str(&format!("z-index:{};", style.z_index));
    }
    let paints = !(hidden && container);
    let background = clut.colour(style.background);
    if paints && background[3] != 0 {
        css.push_str(&format!("background-color:{};", colour_css(background)));
    }
    if style.border_style == BorderStyle::Solid && style.border_width > 0.0 {
        css.push_str(&format!(
            "border:{}px solid {};",
            style.border_width,
            if paints { "" } else { "transparent" }
        ));
        if paints {
            for (side, index) in ["top", "right", "bottom", "left"]
                .iter()
                .zip(style.border_colour)
            {
                css.push_str(&format!(
                    "border-{side}-color:{};",
                    colour_css(clut.colour(index))
                ));
            }
        }
    }
    if paints && let Some(image) = &style.background_image {
        images.push(image.clone());
        css.push_str(&format!(
            "background-image:url({IMAGE_BASE}{});background-repeat:no-repeat;",
            escape_attribute(image)
        ));
    }
    // 箱は CSS 2 の content-box。余白が効くのは文字の箱だけで、div の箱は枠だけを足す。
    let [pt, pr, pb, pl] = style.padding;
    if style.padding != [0.0; 4] && !container {
        css.push_str(&format!("padding:{pt}px {pr}px {pb}px {pl}px;"));
    }
    match element.name.as_str() {
        "object" => {
            let kind = element.attribute("type").unwrap_or("").to_ascii_lowercase();
            if kind.starts_with("video/") {
                out.push_str(&format!(
                    "<object type=\"video/x-arib2-broadcast\" style=\"{css}\"></object>"
                ));
            } else if kind.starts_with("image/")
                && let Some(src) = element.attribute("data")
            {
                let name = match frames.get(&id) {
                    Some(frame) => format!("{src}#{frame}"),
                    None => src.to_owned(),
                };
                images.push(name.clone());
                let picture = picture_css(
                    &kind,
                    sizes.get(src).copied(),
                    html.size,
                    (style.width, style.height),
                );
                out.push_str(&format!(
                    "<div style=\"{css}\"><img src=\"{IMAGE_BASE}{}\" style=\"left:0;top:0;{picture}\">{}</div>",
                    escape_attribute(&name),
                    focus_ring(style)
                ));
            }
        }
        "img" => {
            if let Some(src) = element.attribute("src") {
                images.push(src.to_owned());
                let picture = picture_css(
                    "",
                    sizes.get(src).copied(),
                    html.size,
                    (style.width, style.height),
                );
                out.push_str(&format!(
                    "<div style=\"{css}\"><img src=\"{IMAGE_BASE}{}\" style=\"left:0;top:0;{picture}\">{}</div>",
                    escape_attribute(src),
                    focus_ring(style)
                ));
            }
        }
        "p" if style.marquee.is_some() => {
            let offset = marquees.get(&id).copied().unwrap_or(0.0);
            let inner = style.width.max(0.0);
            let width = text_width(document, styles, id);
            out.push_str(&format!("<div style=\"{css}\">"));
            let left = if style.marquee.is_some_and(|m| m.left_to_right) {
                offset - width
            } else {
                inner - offset
            };
            let mut text = format!(
                "left:{}px;top:0px;width:{}px;height:{}px;white-space:pre;",
                left,
                width.max(1.0),
                style.height.max(0.0)
            );
            text.push_str(&text_css(style, clut));
            out.push_str(&format!("<p style=\"{text}\">"));
            inline(
                scene,
                id,
                WhiteSpace::Nowrap,
                &mut Flow::unbounded(),
                out,
                html,
            );
            out.push_str("</p></div>");
        }
        "p" | "pre" | "textarea" => {
            css.push_str(&text_css(style, clut));
            let white_space = if element.name == "pre" || element.name == "textarea" {
                WhiteSpace::Pre
            } else {
                style.white_space
            };
            // 折り返しはこちらでマス目を数える（Blitz は行末の全角空白を次の行へ送る）。
            css.push_str("white-space:pre;");
            out.push_str(&format!("<p style=\"{css}\">"));
            let mut flow = match white_space {
                WhiteSpace::Nowrap => Flow::unbounded(),
                _ => Flow::within(style.width),
            };
            inline(scene, id, white_space, &mut flow, out, html);
            out.push_str("</p>");
            // 太字は太いウェイトのフォントでレンダリングする。無ければ一画素右にもう一度レンダリングする（受像機も同じ疑似太字）。
            // parley の合成の太字（embolden）は Blitz がレンダリングしないので効かない。
            if style.bold && paints && !has_bold_face(style.family) {
                let mut twice = format!(
                    "left:{}px;top:{}px;width:{}px;height:{}px;visibility:{};",
                    style.left + 1.0,
                    style.top,
                    style.width,
                    style.height,
                    if hidden { "hidden" } else { "visible" }
                );
                if style.border_style == BorderStyle::Solid && style.border_width > 0.0 {
                    twice.push_str(&format!(
                        "border:{}px solid transparent;",
                        style.border_width
                    ));
                }
                if style.padding != [0.0; 4] {
                    twice.push_str(&format!("padding:{pt}px {pr}px {pb}px {pl}px;"));
                }
                twice.push_str(&text_css(style, clut));
                twice.push_str("white-space:pre;");
                out.push_str(&format!("<p style=\"{twice}\">"));
                let mut flow = match white_space {
                    WhiteSpace::Nowrap => Flow::unbounded(),
                    _ => Flow::within(style.width),
                };
                inline(scene, id, white_space, &mut flow, out, html);
                out.push_str("</p>");
            }
        }
        "input" => {
            css.push_str(&text_css(style, clut));
            // HTML の input では Blitz が白い枠をレンダリングするので使わない。
            // `type="password"` は伏せ字（表 5-9）。
            let value = element.attribute("value").unwrap_or("");
            let shown = if element.attribute("type") == Some("password") {
                "*".repeat(value.chars().count())
            } else {
                value.to_owned()
            };
            out.push_str(&format!(
                "<p style=\"{css}white-space:nowrap\">{}</p>",
                grid_text(&shown)
            ));
        }
        _ => {
            css.push_str(&text_css(style, clut));
            out.push_str(&format!("<div style=\"{css}\">"));
            for child in &element.children {
                block(scene, *child, out, html);
            }
            out.push_str("</div>");
        }
    }
}

struct Flow {
    used: f32,
    width: Option<f32>,
}

impl Flow {
    fn unbounded() -> Self {
        Self {
            used: 0.0,
            width: None,
        }
    }

    fn within(width: f32) -> Self {
        Self {
            used: 0.0,
            width: (width > 0.0).then_some(width),
        }
    }

    fn place(&mut self, width: f32, out: &mut String) {
        if let Some(limit) = self.width
            && self.used + width > limit + 0.01
            && self.used > 0.0
        {
            out.push_str("<br>");
            self.used = 0.0;
        }
        self.used += width;
    }

    fn newline(&mut self) {
        self.used = 0.0;
    }
}

fn cell_width(style: &Style, letter: char) -> f32 {
    if crate::media::text::is_wide(letter) {
        style.font_size
    } else {
        style.font_size / 2.0
    }
    .max(0.0)
        + style.letter_spacing
}

fn inline(
    scene: &Scene<'_>,
    id: NodeId,
    white_space: WhiteSpace,
    flow: &mut Flow,
    out: &mut String,
    html: &mut Html,
) {
    let (document, styles, clut) = (scene.document, scene.styles, scene.clut);
    let Some(element) = document.element(id) else {
        return;
    };
    let style = &styles[id.0];
    let children = &element.children;
    for (n, child) in children.iter().enumerate() {
        match &document.node(*child).kind {
            Kind::Text(text) => {
                let first = n == 0;
                let last = n + 1 == children.len();
                let shown = if white_space == WhiteSpace::Pre || document.node(*child).cdata {
                    text.replace('\r', "")
                } else {
                    display_text(text, first, last)
                };
                if shown.is_empty() {
                    continue;
                }
                text_with_drcs(scene, style, &shown, flow, out, html);
            }
            Kind::Element(inner) => match inner.name.as_str() {
                "br" => {
                    out.push_str("<br>");
                    flow.newline();
                }
                _ if styles[child.0].display == Display::Inline => {
                    let inner_style = &styles[child.0];
                    let mut css = text_css(inner_style, clut);
                    // 親と違う背景色を持つときだけ塗る（C プロファイルのフォーカスの青い背景）。
                    if inner_style.background != style.background {
                        css.push_str(&format!(
                            "background-color:{};",
                            colour_css(clut.colour(inner_style.background))
                        ));
                    }
                    out.push_str(&format!("<span style=\"{css}\">"));
                    inline(scene, *child, white_space, flow, out, html);
                    out.push_str("</span>");
                }
                _ => {}
            },
        }
    }
}

/// 文字を出す。外字は `loadDRCS` で読ませた画像に置き換える（無ければそのまま）。
fn text_with_drcs(
    scene: &Scene<'_>,
    style: &Style,
    text: &str,
    flow: &mut Flow,
    out: &mut String,
    html: &mut Html,
) {
    let font = match (style.family, style.bold) {
        (Family::SquareGothic, _) => 2,
        (Family::RoundGothic, true) => 3,
        (Family::RoundGothic, false) => 1,
    };
    let colour = scene.clut.colour(style.colour);
    let mut plain = String::new();
    for letter in text.chars() {
        if letter == '\n' {
            plain.push(letter);
            flow.newline();
            continue;
        }
        let mut ahead = String::new();
        flow.place(cell_width(style, letter), &mut ahead);
        if !ahead.is_empty() {
            out.push_str(&grid_text(&plain));
            plain.clear();
            out.push_str(&ahead);
        }
        let glyph = if scene.drcs.is_empty() {
            None
        } else {
            crate::media::drcs::pick(scene.drcs, letter, style.font_size, font)
        };
        let Some(glyph) = glyph else {
            plain.push(letter);
            continue;
        };
        out.push_str(&grid_text(&plain));
        plain.clear();
        let name = format!(
            "drcs:{:02x}{:02x}:{}x{}:{}:{:02x}{:02x}{:02x}{:02x}",
            glyph.row,
            glyph.cell,
            glyph.width,
            glyph.height,
            glyph.font,
            colour[0],
            colour[1],
            colour[2],
            colour[3]
        );
        html.made
            .entry(name.clone())
            .or_insert_with(|| crate::media::drcs::png(glyph, colour));
        html.images.push(name.clone());
        // 外字の画像はぼかさない（縁が透けて、外字で並べる二次元コードに筋が出る）。
        out.push_str(&format!(
            "<img src=\"{IMAGE_BASE}{}\" style=\"display:inline-block;width:{}px;height:{}px;vertical-align:bottom;position:static;image-rendering:pixelated\">",
            escape_attribute(&name),
            style.font_size,
            style.font_size
        ));
    }
    out.push_str(&grid_text(&plain));
}

pub fn text_width(document: &Document, styles: &[Style], id: NodeId) -> f32 {
    let Some(style) = styles.get(id.0) else {
        return 0.0;
    };
    let text = display_text(&document.text_of(id), true, true);
    text.chars()
        .filter(|c| *c != '\n')
        .map(|c| {
            let cell = if crate::media::text::is_wide(c) {
                style.font_size
            } else {
                style.font_size / 2.0
            };
            cell + style.letter_spacing
        })
        .sum()
}

fn text_css(style: &Style, clut: &Clut) -> String {
    let line_height = style.line_height.unwrap_or(style.font_size);
    let family = match style.family {
        Family::RoundGothic => format!("'{ROUND}', {FAMILY}"),
        Family::SquareGothic => format!("'{SQUARE}', {FAMILY}"),
    };
    format!(
        "color:{};font-size:{}px;line-height:{}px;letter-spacing:{}px;text-align:{};font-family:{};font-weight:{};",
        colour_css(clut.colour(style.colour)),
        style.font_size,
        line_height,
        style.letter_spacing,
        match style.text_align {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
        },
        family,
        if style.bold { "bold" } else { "normal" },
    )
}

fn colour_css(c: Colour) -> String {
    format!(
        "rgba({},{},{},{})",
        c[0],
        c[1],
        c[2],
        f32::from(c[3]) / 255.0
    )
}

/// テキストノードを受像機が表示するとおりに。空白の並びは一つに、`p` の先頭と末尾の空白は落とし、
/// 全角文字に挟まれた空白は消す。改行は改行のまま。
pub fn display_text(text: &str, first: bool, last: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    let mut previous_wide = false;
    for letter in text.chars() {
        match letter {
            '\n' => {
                pending_space = false;
                out.push('\n');
                previous_wide = false;
                continue;
            }
            '\r' => continue,
            ' ' | '\t' => {
                pending_space = true;
                continue;
            }
            _ => {}
        }
        if pending_space {
            let at_start = out.is_empty();
            if !(at_start && first) && !(previous_wide && crate::media::text::is_wide(letter)) {
                out.push(' ');
            }
            pending_space = false;
        }
        out.push(letter);
        previous_wide = crate::media::text::is_wide(letter);
    }
    if pending_space && !last && !out.is_empty() {
        out.push(' ');
    }
    out
}

/// 半角の英数字を半角の字形（`hwid`）で出し、プロポーショナルなフォントでもマス目に揃える。
/// かなに `hwid` を掛けると半角のかなになるので、英数字だけを囲む。
fn grid_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut half = false;
    for letter in text.chars() {
        let ascii = letter.is_ascii_graphic() || letter == ' ';
        if ascii != half {
            out.push_str(if ascii {
                "<span style=\"font-feature-settings:'hwid'\">"
            } else {
                "</span>"
            });
            half = ascii;
        }
        out.push_str(&escape_text(letter.encode_utf8(&mut [0; 4])));
    }
    if half {
        out.push_str("</span>");
    }
    out
}

fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for letter in text.chars() {
        match letter {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

fn focus_ring(style: &Style) -> String {
    if !style.focus_ring {
        return String::new();
    }
    format!(
        "<div style=\"position:absolute;left:0;top:0;width:{}px;height:{}px;border:1px dashed rgb(0,0,255);box-sizing:border-box;\"></div>",
        style.width.max(0.0),
        style.height.max(0.0)
    )
}

fn escape_attribute(text: &str) -> String {
    escape_text(text).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{css, dom, style};
    use std::collections::BTreeMap;

    fn html(markup: &str) -> Html {
        let document = dom::parse(markup);
        let sheets = css::parse_stylesheet("");
        let styles = style::compute(&document, &sheets, &BTreeMap::new());
        render(&Scene {
            document: &document,
            styles: &styles,
            clut: &Clut::default(),
            marquees: &BTreeMap::new(),
            frames: &BTreeMap::new(),
            drcs: &[],
            sizes: &BTreeMap::new(),
        })
    }

    #[test]
    fn bold_is_drawn_twice_only_without_a_bold_face() {
        let markup = r#"<bml><body><p style="left:0px;top:0px;width:100px;height:20px;font-family:太丸ゴシック">あ</p><p style="left:0px;top:20px;width:100px;height:20px;font-family:角ゴシック;font-weight:bold">い</p></body></bml>"#;
        let count = |html: &Html, letter: &str| html.markup.matches(letter).count();
        set_bold_faces(false, false);
        let twice = html(markup);
        assert_eq!(
            count(&twice, ">あ</p>"),
            2,
            "太いフォントが無ければ疑似太字"
        );
        assert_eq!(count(&twice, ">い</p>"), 2);
        set_bold_faces(true, false);
        let once = html(markup);
        set_bold_faces(false, false);
        assert_eq!(
            count(&once, ">あ</p>"),
            1,
            "太丸ゴシックは太いフォントで一度だけ"
        );
        assert_eq!(
            count(&once, ">い</p>"),
            2,
            "角に太い物が無ければ疑似太字のまま"
        );
        assert!(once.markup.contains(&format!(
            "font-family:'{ROUND}', {FAMILY};font-weight:bold;"
        )));
        assert!(once.markup.contains(&format!(
            "font-family:'{SQUARE}', {FAMILY};font-weight:bold;"
        )));
    }

    #[test]
    fn half_width_letters_take_half_width_glyphs() {
        assert_eq!(grid_text("あいう"), "あいう");
        assert_eq!(
            grid_text("気温12℃ <b>"),
            "気温<span style=\"font-feature-settings:'hwid'\">12</span>℃<span style=\"font-feature-settings:'hwid'\"> &lt;b&gt;</span>"
        );
    }

    #[test]
    fn the_display_aspect_of_the_body_is_carried() {
        let sd = html(
            "<bml><body style=\"resolution:720x480;display-aspect-ratio:16v9\"><p>a</p></body></bml>",
        );
        assert_eq!(sd.size, (720, 480));
        assert_eq!(sd.aspect, Some((16, 9)));
        let hd = html("<bml><body><p>a</p></body></bml>");
        assert_eq!(hd.size, (960, 540));
        assert_eq!(hd.aspect, None);
    }

    #[test]
    fn drcs_letters_become_pictures_of_the_text_colour() {
        let document =
            dom::parse("<bml><body><p style=\"font-size:20px\">A\u{ec00}B</p></body></bml>");
        let sheets = css::parse_stylesheet("");
        let styles = style::compute(&document, &sheets, &BTreeMap::new());
        let glyphs = crate::media::drcs::parse(&[1, 0x77, 0x21, 1, 0x11, 2, 2, 2, 0b0001_1011]);
        let html = render(&Scene {
            document: &document,
            styles: &styles,
            clut: &Clut::default(),
            marquees: &BTreeMap::new(),
            frames: &BTreeMap::new(),
            drcs: &glyphs,
            sizes: &BTreeMap::new(),
        });
        assert!(
            html.markup
                .contains("A</span><img src=\"arib-bml://res/drcs:7721:2x2:1:"),
            "{}",
            html.markup
        );
        assert!(html.markup.contains("width:20px;height:20px"));
        assert_eq!(html.made.len(), 1);
        assert_eq!(html.images.len(), 1);
    }

    #[test]
    fn boxes_become_absolute_divs_inside_their_parents() {
        let out = html(
            r#"<bml><body style="background-color-index:8"><div id="d" style="left:10px;top:20px;width:100px;height:50px;"><p style="left:5px;top:5px;width:50px;height:20px;color-index:7;">あ a</p></div><object type="video/X-arib-mpeg2" style="left:48px;top:93px;width:480px;height:270px;"/><object type="image/jpeg" data="a.jpg" style="width:10px;height:10px"/></body></bml>"#,
        );
        assert_eq!(out.size, (960, 540));
        assert!(
            out.markup
                .contains("<div style=\"left:10px;top:20px;width:100px;height:50px;")
        );
        assert!(
            out.markup
                .contains("<p style=\"left:5px;top:5px;width:50px;height:20px;")
        );
        assert!(
            out.markup
                .contains(">あ<span style=\"font-feature-settings:'hwid'\"> a</span></p>")
        );
        assert!(out.markup.contains("video/x-arib2-broadcast"));
        assert!(out.markup.contains("<img src=\"arib-bml://res/a.jpg\""));
        assert_eq!(out.images, ["a.jpg"]);
        // `style="…"` の中に二重引用符があると属性がそこで切れる。
        for piece in out.markup.split("style=\"").skip(1) {
            let inside = piece.split('"').next().unwrap_or("");
            assert!(
                !inside.is_empty() && !inside.contains("font-family:\""),
                "{inside}"
            );
            assert!(
                inside.contains("font-family") || !inside.contains("color"),
                "{inside}"
            );
        }
    }

    #[test]
    fn pictures_keep_their_own_size_inside_the_box() {
        let document = dom::parse(
            r#"<bml><body><object type="image/X-arib-png" data="t.png" style="left:26px;top:0px;width:200px;height:30px"/><object type="image/jpeg" data="a.jpg" style="width:480px;height:270px"/></body></bml>"#,
        );
        let sheets = css::parse_stylesheet("");
        let styles = style::compute(&document, &sheets, &BTreeMap::new());
        let mut sizes = BTreeMap::new();
        sizes.insert(String::from("t.png"), (140, 29));
        sizes.insert(String::from("a.jpg"), (960, 540));
        let html = render(&Scene {
            document: &document,
            styles: &styles,
            clut: &Clut::default(),
            marquees: &BTreeMap::new(),
            frames: &BTreeMap::new(),
            drcs: &[],
            sizes: &sizes,
        });
        assert!(
            html.markup.contains("width:200px;height:30px;"),
            "{}",
            html.markup
        );
        assert!(
            html.markup.contains(
                "<img src=\"arib-bml://res/t.png\" style=\"left:0;top:0;width:140px;height:29px;\">"
            ),
            "{}",
            html.markup
        );
        assert!(
            html.markup
                .contains("<img src=\"arib-bml://res/a.jpg\" style=\"left:0;top:0;width:480px;height:270px;\">"),
            "{}",
            html.markup
        );
    }

    /// body の背景画像は常に敷き詰め（STD-B24 第二分冊 付属2 A2-4.4.7）、JPEG は静止画プレーンの画素なので
    /// 960×540 の文書では半分の大きさで並べる。960×540 の画像だけは等倍（TR-B14 第二編 表 1-4）。
    #[test]
    fn body_background_tiles_at_still_plane_size() {
        let body = |image: &str, size: (u32, u32), extra: &str| {
            let document = dom::parse(&format!(
                r#"<bml><body style="background-image:url({image});{extra}"></body></bml>"#
            ));
            let sheets = css::parse_stylesheet("");
            let styles = style::compute(&document, &sheets, &BTreeMap::new());
            let mut sizes = BTreeMap::new();
            sizes.insert(image.to_owned(), size);
            render(&Scene {
                document: &document,
                styles: &styles,
                clut: &Clut::default(),
                marquees: &BTreeMap::new(),
                frames: &BTreeMap::new(),
                drcs: &[],
                sizes: &sizes,
            })
            .markup
        };
        let strip = body("bg.jpg", (16, 1080), "background-repeat:no-repeat");
        assert!(
            strip.contains(
                "background-image:url(arib-bml://res/bg.jpg);background-repeat:repeat;background-size:8px 540px;"
            ),
            "{strip}"
        );
        let full = body("bg.jpg", (960, 540), "");
        assert!(full.contains("background-size:960px 540px;"), "{full}");
        let sd = body("bg.jpg", (720, 480), "resolution:720x480");
        assert!(sd.contains("background-size:720px 480px;"), "{sd}");
    }

    #[test]
    fn picture_sizes_are_read_from_the_head() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&140u32.to_be_bytes());
        png.extend_from_slice(&29u32.to_be_bytes());
        assert_eq!(picture_size(&png), Some((140, 29)));
        let jpeg = [
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x02,
            0x1c, 0x03, 0xc0,
        ];
        assert_eq!(picture_size(&jpeg), Some((960, 540)));
        assert_eq!(picture_size(b"MNG"), None);
    }

    #[test]
    fn spaces_are_shown_the_way_the_receiver_shows_them() {
        assert_eq!(display_text("  あ  い  ", true, true), "あい");
        assert_eq!(display_text("a  b\t c", true, true), "a b c");
        assert_eq!(
            display_text("天気\r\nニュース\r\n", true, true),
            "天気\nニュース\n"
        );
        assert_eq!(display_text("あ a あ", true, true), "あ a あ");
        assert_eq!(display_text(" x ", false, false), " x ");
        assert_eq!(display_text("　", true, true), "　");
    }

    #[test]
    fn a_hidden_box_hides_its_children_unless_they_say_visible() {
        let out = html(
            r#"<bml><body><div style="width:100px;height:100px;visibility:hidden"><p style="width:10px;height:10px">x</p><p style="width:10px;height:10px;visibility:visible">y</p></div></body></bml>"#,
        );
        assert!(out.markup.contains("height:100px;visibility:visible;"));
        assert!(out.markup.contains("visibility:hidden;color"));
        assert!(out.markup.contains("height:10px;visibility:visible;color"));
    }
}
