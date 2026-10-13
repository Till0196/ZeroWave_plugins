// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 要素ごとの算出スタイル。順は CSS のとおり（既定、スタイルシート、`style` 属性、`!important`）。
//! 既定は STD-B24 第二分冊 (2/2) 付属 2 の既定のスタイルシートから、運用される要素のみ。

use std::collections::BTreeMap;

use crate::document::css::{self, Declaration, Rule, Selector, Value};
use crate::document::dom::{Document, Element, NodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Display {
    #[default]
    Block,
    Inline,
    None,
}

/// マーキーの設定（`-wap-marquee-*`、TR-B14 の C プロファイル）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Marquee {
    pub style: MarqueeStyle,
    pub loops: Option<u32>,
    pub interval_ms: f32,
    pub left_to_right: bool,
}

impl Default for Marquee {
    fn default() -> Self {
        Self {
            style: MarqueeStyle::Scroll,
            loops: Some(1),
            interval_ms: 170.0,
            left_to_right: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarqueeStyle {
    #[default]
    Scroll,
    Slide,
    Alternate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WhiteSpace {
    #[default]
    Normal,
    Pre,
    Nowrap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    #[default]
    Visible,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderStyle {
    #[default]
    None,
    Solid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Family {
    #[default]
    RoundGothic,
    SquareGothic,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub display: Display,
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub padding: [f32; 4],
    pub border_width: f32,
    pub border_style: BorderStyle,
    pub border_colour: [u32; 4],
    pub visibility: Visibility,
    pub font_size: f32,
    pub family: Family,
    pub bold: bool,
    pub line_height: Option<f32>,
    pub letter_spacing: f32,
    pub text_align: Align,
    pub colour: u32,
    pub background: u32,
    pub grayscale: (u32, u32),
    pub clut: Option<String>,
    pub background_image: Option<String>,
    pub z_index: i32,
    pub white_space: WhiteSpace,
    pub marquee: Option<Marquee>,
    pub resolution: Option<(u32, u32)>,
    pub aspect: Option<(u32, u32)>,
    pub nav_index: Option<i32>,
    pub nav: [Option<i32>; 4],
    pub used_key_list: Vec<String>,
    /// フォーカスの乗った画像（C プロファイルの `object`・`img`）。受像機が青い破線で囲む。
    pub focus_ring: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            display: Display::Block,
            left: 0.0,
            top: 0.0,
            width: 0.0,
            height: 0.0,
            padding: [0.0; 4],
            border_width: 0.0,
            border_style: BorderStyle::None,
            border_colour: [0; 4],
            visibility: Visibility::Visible,
            font_size: 24.0,
            family: Family::RoundGothic,
            bold: false,
            line_height: None,
            letter_spacing: 0.0,
            text_align: Align::Left,
            // 文字の色の既定は黒（color-index を書かない局がある）。テーブルの番号ではなく直の色。
            colour: crate::media::clut::direct(0, 0, 0),
            background: 8,
            grayscale: (15, 0),
            clut: None,
            background_image: None,
            z_index: 0,
            white_space: WhiteSpace::Normal,
            marquee: None,
            resolution: None,
            aspect: None,
            nav_index: None,
            nav: [None; 4],
            used_key_list: vec!["basic".to_owned(), "data-button".to_owned()],
            focus_ring: false,
        }
    }
}

impl Style {
    pub fn default_for(name: &str, parent: Option<&Style>) -> Self {
        let mut style = Self::default();
        if let Some(parent) = parent {
            style.inherit(parent);
        }
        style.display = match name {
            // ルート要素は `bml`（A）か `html`（C）。
            // `form`・`textarea` は C の要素（TR-B14 第三分冊 表 7-4）。
            "bml" | "html" | "body" | "div" | "p" | "pre" | "object" | "input" | "img" | "form"
            | "textarea" => Display::Block,
            "br" | "span" | "a" => Display::Inline,
            _ => Display::None,
        };
        if name == "body" {
            style.visibility = Visibility::Visible;
            style.resolution = Some((960, 540));
            if crate::runtime::profile::is_c() {
                // C プロファイルのサーフェスは縦長の 240×480、背景は白。
                style.resolution = Some((240, 480));
                style.background = crate::media::clut::direct(255, 255, 255);
            }
        }
        if parent.is_none() && crate::runtime::profile::is_c() {
            // 文字の大きさの既定は `medium`＝20px（TR-B14 第三分冊 8.1.9.2 表 8-1）。
            style.font_size = 20.0;
        }
        style
    }

    fn inherit(&mut self, parent: &Style) {
        self.visibility = parent.visibility;
        self.font_size = parent.font_size;
        self.family = parent.family;
        self.bold = parent.bold;
        self.line_height = parent.line_height;
        self.letter_spacing = parent.letter_spacing;
        self.text_align = parent.text_align;
        self.colour = parent.colour;
        self.grayscale = parent.grayscale;
        self.clut = parent.clut.clone();
        self.white_space = parent.white_space;
        self.resolution = parent.resolution;
        self.aspect = parent.aspect;
    }

    pub fn apply(&mut self, name: &str, value: &Value, parent: Option<&Style>) {
        if value.keyword() == Some("inherit") {
            if let Some(parent) = parent {
                let mut from = self.clone();
                from.inherit(parent);
                self.copy_named(name, &from, parent);
            }
            return;
        }
        let px = |v: &Value| v.px();
        match name {
            "left" => self.left = px(value).unwrap_or(self.left),
            "top" => self.top = px(value).unwrap_or(self.top),
            "width" => self.width = px(value).unwrap_or(self.width),
            "height" => self.height = px(value).unwrap_or(self.height),
            "padding-top" => self.padding[0] = px(value).unwrap_or(0.0),
            "padding-right" => self.padding[1] = px(value).unwrap_or(0.0),
            "padding-bottom" => self.padding[2] = px(value).unwrap_or(0.0),
            "padding-left" => self.padding[3] = px(value).unwrap_or(0.0),
            "padding" => {
                let items: Vec<f32> = value.items().iter().filter_map(px).collect();
                self.padding = match items.as_slice() {
                    [all] => [*all; 4],
                    [v, h] => [*v, *h, *v, *h],
                    [t, h, b] => [*t, *h, *b, *h],
                    [t, r, b, l] => [*t, *r, *b, *l],
                    _ => self.padding,
                };
            }
            "border-width" => self.border_width = px(value).unwrap_or(0.0),
            "border-style" => {
                self.border_style = match value.keyword() {
                    Some("solid") => BorderStyle::Solid,
                    _ => BorderStyle::None,
                }
            }
            "border-top-color-index" => self.border_colour[0] = index(value),
            "border-right-color-index" => self.border_colour[1] = index(value),
            "border-bottom-color-index" => self.border_colour[2] = index(value),
            "border-left-color-index" => self.border_colour[3] = index(value),
            "border-color-index" => {
                let items: Vec<u32> = value.items().iter().map(index).collect();
                self.border_colour = match items.as_slice() {
                    [all] => [*all; 4],
                    [v, h] => [*v, *h, *v, *h],
                    [t, h, b] => [*t, *h, *b, *h],
                    [t, r, b, l] => [*t, *r, *b, *l],
                    _ => self.border_colour,
                };
            }
            "visibility" => {
                self.visibility = match value.keyword() {
                    Some("hidden") => Visibility::Hidden,
                    Some("visible") => Visibility::Visible,
                    _ => self.visibility,
                }
            }
            "display" => {
                self.display = match value.keyword() {
                    Some("none") => Display::None,
                    Some("inline") => Display::Inline,
                    Some("block") => Display::Block,
                    Some("-wap-marquee") => {
                        self.marquee.get_or_insert_with(Marquee::default);
                        Display::Block
                    }
                    _ => self.display,
                };
                if !matches!(value.keyword(), Some("-wap-marquee")) {
                    self.marquee = None;
                }
            }
            "-wap-marquee-style" => {
                let m = self.marquee.get_or_insert_with(Marquee::default);
                m.style = match value.keyword() {
                    Some("slide") => MarqueeStyle::Slide,
                    Some("alternate") => MarqueeStyle::Alternate,
                    _ => MarqueeStyle::Scroll,
                };
            }
            "-wap-marquee-dir" => {
                let m = self.marquee.get_or_insert_with(Marquee::default);
                m.left_to_right = matches!(value.keyword(), Some("ltr"));
            }
            // どれも読むだけでレンダリングは変えない（箱は常に切り、入力は数字しか入らない、背景画像は常に
            // 敷き詰める。STD-B24 第二分冊 付属2 A2-4.4.7 で background-repeat は固定値 repeat）。
            "overflow" | "-wap-input-format" | "background-repeat" => {}
            "-wap-marquee-loop" => {
                let m = self.marquee.get_or_insert_with(Marquee::default);
                m.loops = match value.keyword() {
                    Some("infinite") => None,
                    _ => Some(value.number().map_or(1, |n| n.max(0.0) as u32)),
                };
            }
            "-wap-marquee-speed" => {
                let m = self.marquee.get_or_insert_with(Marquee::default);
                // 受像機は一ステップで文字の大きさの半分ずつ動かす。≒ 35 / 70 / 140 px/s になる間隔。
                m.interval_ms = match value.keyword() {
                    Some("slow") => 340.0,
                    Some("fast") => 85.0,
                    _ => 170.0,
                };
            }
            "font-size" => {
                // C プロファイルはキーワードで指定する（TR-B14 第三分冊 表 8-1）。
                let worded = match value.keyword() {
                    Some("small") => Some(16.0),
                    Some("medium") => Some(20.0),
                    Some("large") => Some(30.0),
                    _ => None,
                };
                self.font_size = px(value).or(worded).unwrap_or(self.font_size);
            }
            "font-family" => {
                let named = match value {
                    Value::Text(text) | Value::Keyword(text) => text.as_str(),
                    Value::List(items) => items
                        .first()
                        .and_then(|item| match item {
                            Value::Text(text) | Value::Keyword(text) => Some(text.as_str()),
                            _ => None,
                        })
                        .unwrap_or(""),
                    _ => "",
                };
                self.family = match named {
                    "角ゴシック" => Family::SquareGothic,
                    "太丸ゴシック" => {
                        self.bold = true;
                        Family::RoundGothic
                    }
                    _ => Family::RoundGothic,
                };
            }
            "font-weight" => {
                self.bold = match value {
                    Value::Keyword(word) => word == "bold" || word == "bolder",
                    Value::Number(number) => *number >= 600.0,
                    _ => self.bold,
                }
            }
            "line-height" => {
                self.line_height = match value {
                    Value::Px(pixels) => Some(*pixels),
                    Value::Number(times) => Some(times * self.font_size),
                    Value::Keyword(word) if word == "normal" => None,
                    _ => self.line_height,
                }
            }
            "letter-spacing" => self.letter_spacing = px(value).unwrap_or(0.0),
            "text-align" => {
                self.text_align = match value.keyword() {
                    Some("center") => Align::Center,
                    Some("right") => Align::Right,
                    Some("left") => Align::Left,
                    _ => self.text_align,
                }
            }
            "color-index" => self.colour = index(value),
            "background-color-index" => self.background = index(value),
            "color" => {
                if let Some(c) = value.keyword().and_then(crate::media::clut::parse_direct) {
                    self.colour = c;
                }
            }
            "background-color" => {
                if let Some(c) = value.keyword().and_then(crate::media::clut::parse_direct) {
                    self.background = c;
                }
            }
            "border-color" => {
                let items: Vec<u32> = value
                    .items()
                    .iter()
                    .filter_map(|v| v.keyword().and_then(crate::media::clut::parse_direct))
                    .collect();
                self.border_colour = match items.as_slice() {
                    [all] => [*all; 4],
                    [v, h] => [*v, *h, *v, *h],
                    [t, h, b] => [*t, *h, *b, *h],
                    [t, r, b, l] => [*t, *r, *b, *l],
                    _ => self.border_colour,
                };
            }
            "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color" => {
                if let Some(c) = value.keyword().and_then(crate::media::clut::parse_direct) {
                    let side = match name {
                        "border-top-color" => 0,
                        "border-right-color" => 1,
                        "border-bottom-color" => 2,
                        _ => 3,
                    };
                    self.border_colour[side] = c;
                }
            }
            "z-index" => self.z_index = value.number().map_or(0, |n| n as i32),
            "white-space" => {
                self.white_space = match value.keyword() {
                    Some("pre") => WhiteSpace::Pre,
                    Some("nowrap") => WhiteSpace::Nowrap,
                    _ => WhiteSpace::Normal,
                }
            }
            "grayscale-color-index" => {
                let items = value.items();
                if let [text, back] = items {
                    self.grayscale = (index(text), index(back));
                }
            }
            "clut" => {
                if let Value::Url(url) = value {
                    self.clut = Some(url.clone());
                }
            }
            "background-image" => {
                self.background_image = match value {
                    Value::Url(url) => Some(url.clone()),
                    _ => None,
                };
            }
            "resolution" => {
                if let Value::Size(w, h) = value {
                    self.resolution = Some((*w, *h));
                }
            }
            "display-aspect-ratio" => {
                if let Value::Aspect(w, h) = value {
                    self.aspect = Some((*w, *h));
                }
            }
            "nav-index" => self.nav_index = value.number().map(|n| n as i32),
            "nav-up" => self.nav[0] = value.number().map(|n| n as i32),
            "nav-down" => self.nav[1] = value.number().map(|n| n as i32),
            "nav-left" => self.nav[2] = value.number().map(|n| n as i32),
            "nav-right" => self.nav[3] = value.number().map(|n| n as i32),
            "used-key-list" => {
                self.used_key_list = value
                    .items()
                    .iter()
                    .filter_map(|item| item.keyword().map(str::to_owned))
                    .collect();
            }
            _ => {}
        }
    }

    fn copy_named(&mut self, name: &str, from: &Style, parent: &Style) {
        match name {
            "visibility" => self.visibility = from.visibility,
            "font-size" => self.font_size = from.font_size,
            "font-family" => {
                self.family = from.family;
                self.bold = from.bold;
            }
            "font-weight" => self.bold = from.bold,
            "line-height" => self.line_height = from.line_height,
            "letter-spacing" => self.letter_spacing = from.letter_spacing,
            "text-align" => self.text_align = from.text_align,
            "color-index" => self.colour = from.colour,
            "background-color-index" => self.background = parent.background,
            "grayscale-color-index" => self.grayscale = from.grayscale,
            _ => {}
        }
    }

    pub fn get(&self, name: &str) -> String {
        let px = |v: f32| Value::Px(v).to_css();
        match name {
            "left" => px(self.left),
            "top" => px(self.top),
            "width" => px(self.width),
            "height" => px(self.height),
            "padding-top" => px(self.padding[0]),
            "padding-right" => px(self.padding[1]),
            "padding-bottom" => px(self.padding[2]),
            "padding-left" => px(self.padding[3]),
            "border-width" => px(self.border_width),
            "border-style" => match self.border_style {
                BorderStyle::None => "none".into(),
                BorderStyle::Solid => "solid".into(),
            },
            "border-top-color-index" => self.border_colour[0].to_string(),
            "border-right-color-index" => self.border_colour[1].to_string(),
            "border-bottom-color-index" => self.border_colour[2].to_string(),
            "border-left-color-index" => self.border_colour[3].to_string(),
            "visibility" => match self.visibility {
                Visibility::Visible => "visible".into(),
                Visibility::Hidden => "hidden".into(),
            },
            "font-size" => px(self.font_size),
            "font-family" => match (self.family, self.bold) {
                (Family::SquareGothic, _) => "角ゴシック".into(),
                (Family::RoundGothic, true) => "太丸ゴシック".into(),
                (Family::RoundGothic, false) => "丸ゴシック".into(),
            },
            "font-weight" => if self.bold { "bold" } else { "normal" }.into(),
            "line-height" => self.line_height.map_or("normal".into(), px),
            "letter-spacing" => px(self.letter_spacing),
            "text-align" => match self.text_align {
                Align::Left => "left".into(),
                Align::Center => "center".into(),
                Align::Right => "right".into(),
            },
            "color-index" => self.colour.to_string(),
            "background-color-index" => self.background.to_string(),
            "grayscale-color-index" => format!("{} {}", self.grayscale.0, self.grayscale.1),
            "clut" => self
                .clut
                .as_ref()
                .map_or(String::new(), |url| format!("url({url})")),
            "background-image" => self
                .background_image
                .as_ref()
                .map_or(String::from("none"), |url| format!("url({url})")),
            "resolution" => self
                .resolution
                .map_or(String::new(), |(w, h)| format!("{w}x{h}")),
            "display-aspect-ratio" => self
                .aspect
                .map_or(String::new(), |(w, h)| format!("{w}v{h}")),
            "nav-index" => self.nav_index.map_or(String::new(), |n| n.to_string()),
            "nav-up" => self.nav[0].map_or(String::new(), |n| n.to_string()),
            "nav-down" => self.nav[1].map_or(String::new(), |n| n.to_string()),
            "nav-left" => self.nav[2].map_or(String::new(), |n| n.to_string()),
            "nav-right" => self.nav[3].map_or(String::new(), |n| n.to_string()),
            "used-key-list" => self.used_key_list.join(" "),
            _ => String::new(),
        }
    }
}

fn index(value: &Value) -> u32 {
    value.number().map_or(0, |n| n.max(0.0) as u32)
}

pub fn matches(document: &Document, selector: &Selector, id: NodeId) -> bool {
    matches_focused(document, selector, id, None)
}

/// フォーカスを考慮して適用する。`:active` は押している間の状態を持たないので当たらない。
pub fn matches_focused(
    document: &Document,
    selector: &Selector,
    id: NodeId,
    focused: Option<NodeId>,
) -> bool {
    let Some((last, rest)) = selector.chain.split_last() else {
        return false;
    };
    let Some(element) = document.element(id) else {
        return false;
    };
    if !simple_matches(last, element) {
        return false;
    }
    match last.pseudo.as_deref() {
        Some("focus") if focused != Some(id) => return false,
        Some("active") => return false,
        _ => {}
    }
    if rest.iter().any(|simple| simple.pseudo.is_some()) {
        return false;
    }
    let mut wanted = rest.iter().rev();
    let mut next = wanted.next();
    for ancestor in document.ancestors(id) {
        let Some(simple) = next else {
            break;
        };
        if document
            .element(ancestor)
            .is_some_and(|element| simple_matches(simple, element))
        {
            next = wanted.next();
        }
    }
    next.is_none()
}

fn simple_matches(simple: &css::Simple, element: &Element) -> bool {
    if let Some(tag) = &simple.tag
        && *tag != element.name
    {
        return false;
    }
    if let Some(id) = &simple.id
        && element.attribute("id") != Some(id)
    {
        return false;
    }
    if !simple.classes.is_empty() {
        let classes: Vec<&str> = element
            .attribute("class")
            .map(|list| list.split_whitespace().collect())
            .unwrap_or_default();
        if !simple
            .classes
            .iter()
            .all(|class| classes.contains(&class.as_str()))
        {
            return false;
        }
    }
    true
}

type Applicable<'a> = (bool, (u32, u32, u32), usize, &'a Declaration);

#[derive(Debug, Clone, Default)]
pub struct Scripted<'a> {
    pub normal: Option<&'a BTreeMap<NodeId, Vec<Declaration>>>,
    pub focus: Option<&'a BTreeMap<NodeId, Vec<Declaration>>>,
    pub focused: Option<NodeId>,
}

pub fn compute(
    document: &Document,
    sheets: &[Rule],
    overrides: &BTreeMap<NodeId, Vec<Declaration>>,
) -> Vec<Style> {
    compute_with(
        document,
        sheets,
        &Scripted {
            normal: Some(overrides),
            ..Default::default()
        },
    )
}

pub fn compute_with(document: &Document, sheets: &[Rule], scripted: &Scripted<'_>) -> Vec<Style> {
    let focused = scripted.focused;
    let mut styles: Vec<Style> = vec![Style::default(); document.nodes.len()];
    let mut stack: Vec<NodeId> = document.root.into_iter().collect();
    while let Some(id) = stack.pop() {
        let Some(element) = document.element(id) else {
            continue;
        };
        let parent = document
            .node(id)
            .parent
            .map(|parent| styles[parent.0].clone());
        let mut style = Style::default_for(&element.name, parent.as_ref());
        let mut applicable: Vec<Applicable<'_>> = Vec::new();
        for (order, rule) in sheets.iter().enumerate() {
            for selector in &rule.selectors {
                if matches_focused(document, selector, id, focused) {
                    let specificity = selector.specificity();
                    for declaration in &rule.declarations {
                        applicable.push((declaration.important, specificity, order, declaration));
                    }
                }
            }
        }
        applicable
            .sort_by_key(|(important, specificity, order, _)| (*important, *specificity, *order));
        let inline: Vec<Declaration> = element
            .attribute("style")
            .map(css::parse_declarations)
            .unwrap_or_default();
        let scripted_normal = scripted.normal.and_then(|o| o.get(&id));
        let scripted_focus = if focused == Some(id) {
            scripted.focus.and_then(|o| o.get(&id))
        } else {
            None
        };
        for (important, _, _, declaration) in &applicable {
            if !*important {
                style.apply(&declaration.name, &declaration.value, parent.as_ref());
            }
        }
        for declaration in &inline {
            if !declaration.important {
                style.apply(&declaration.name, &declaration.value, parent.as_ref());
            }
        }
        for list in [scripted_normal, scripted_focus].into_iter().flatten() {
            for declaration in list {
                style.apply(&declaration.name, &declaration.value, parent.as_ref());
            }
        }
        for (important, _, _, declaration) in &applicable {
            if *important {
                style.apply(&declaration.name, &declaration.value, parent.as_ref());
            }
        }
        for declaration in &inline {
            if declaration.important {
                style.apply(&declaration.name, &declaration.value, parent.as_ref());
            }
        }
        // C プロファイルのフォーカスは受像機が表示する（青い背景に白い文字）。C の文書は `:focus` を書かない。
        if focused == Some(id) && crate::runtime::profile::is_c() {
            match element.name.as_str() {
                "a" | "input" | "textarea" => {
                    style.background = crate::media::clut::direct(0, 0, 255);
                    style.colour = crate::media::clut::direct(255, 255, 255);
                }
                "object" | "img" => style.focus_ring = true,
                _ => {}
            }
        }
        styles[id.0] = style;
        for child in element.children.iter().rev() {
            stack.push(*child);
        }
    }
    styles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::dom;

    #[test]
    fn the_sheet_then_the_attribute_then_the_script_decide() {
        let document = dom::parse(
            r#"<bml><body style="clut:url(c.clt);"><p id="a" class="x" style="left:5px;">t<span id="s">u</span></p></body></bml>"#,
        );
        let sheets = css::parse_stylesheet(
            "p{left:1px;top:2px;font-size:16px;color-index:3} #a{top:4px} .x{top:9px !important}",
        );
        let mut overrides = BTreeMap::new();
        let a = document.by_id("a").unwrap();
        overrides.insert(a, css::parse_declarations("width:100px"));
        let styles = compute(&document, &sheets, &overrides);
        let style = &styles[a.0];
        assert_eq!(style.left, 5.0);
        assert_eq!(style.top, 9.0);
        assert_eq!(style.width, 100.0);
        assert_eq!(style.font_size, 16.0);
        let span = &styles[document.by_id("s").unwrap().0];
        assert_eq!(span.font_size, 16.0);
        assert_eq!(span.colour, 3);
        assert_eq!(span.display, Display::Inline);
        assert_eq!(span.clut.as_deref(), Some("c.clt"));
        assert_eq!(span.left, 0.0);
    }

    #[test]
    fn focus_rules_and_focus_style_reach_only_the_focused_element() {
        let document = dom::parse(
            r#"<bml><body><p id="a" style="left:0px">t</p><p id="b" style="left:0px">u</p></body></bml>"#,
        );
        let sheets =
            css::parse_stylesheet("p:focus { color-index: 3 } p:active { color-index: 4 }");
        let a = document.by_id("a").unwrap();
        let b = document.by_id("b").unwrap();
        let mut focus = BTreeMap::new();
        focus.insert(a, css::parse_declarations("background-color-index: 5"));
        let styles = compute_with(
            &document,
            &sheets,
            &Scripted {
                normal: None,
                focus: Some(&focus),
                focused: Some(a),
            },
        );
        assert_eq!(styles[a.0].colour, 3);
        assert_eq!(styles[a.0].background, 5);
        assert_eq!(styles[b.0].colour, Style::default().colour);
        assert_ne!(styles[b.0].background, 5);
    }

    #[test]
    fn marquee_properties_are_read() {
        let document = dom::parse(
            r#"<bml><body><p id="m" style="display:-wap-marquee;-wap-marquee-speed:fast;-wap-marquee-loop:infinite;">x</p></body></bml>"#,
        );
        let styles = compute(&document, &[], &BTreeMap::new());
        let m = document.by_id("m").unwrap();
        let marquee = styles[m.0].marquee.expect("マーキー");
        assert_eq!(marquee.loops, None);
        assert_eq!(marquee.interval_ms, 85.0);
        assert_eq!(styles[m.0].display, Display::Block);
    }

    #[test]
    fn a_descendant_selector_needs_the_ancestor() {
        let document =
            dom::parse(r#"<bml><body><div id="d"><p id="in"/></div><p id="out"/></body></bml>"#);
        let sheets = css::parse_stylesheet("div p{left:7px}");
        let styles = compute(&document, &sheets, &BTreeMap::new());
        assert_eq!(styles[document.by_id("in").unwrap().0].left, 7.0);
        assert_eq!(styles[document.by_id("out").unwrap().0].left, 0.0);
    }

    #[test]
    fn what_was_set_reads_back_as_written() {
        let mut style = Style::default();
        style.apply("left", &Value::Px(12.0), None);
        style.apply("visibility", &Value::Keyword("hidden".into()), None);
        style.apply("font-family", &Value::Text("角ゴシック".into()), None);
        assert_eq!(style.get("left"), "12px");
        assert_eq!(style.get("visibility"), "hidden");
        assert_eq!(style.get("font-family"), "角ゴシック");
    }
}
