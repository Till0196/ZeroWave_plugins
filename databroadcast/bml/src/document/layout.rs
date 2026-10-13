// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! レイアウト。BML の箱は絶対配置で、流れは `p` の中の文字だけ。文字はマス目に並べ、
//! 禁則はしない（STD-B24 第二分冊 (2/2) 付属 2 5.3）。出力は文書の座標でのレンダリング命令。

use crate::document::dom::{Document, Kind, NodeId};
use crate::document::style::{BorderStyle, Display, Family, Style, Visibility};
use crate::media::clut::{Clut, Colour};
use crate::media::text::is_wide;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn intersect(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        Rect {
            x,
            y,
            w: (right - x).max(0.0),
            h: (bottom - y).max(0.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub colour: Colour,
    pub family: Family,
    pub bold: bool,
    pub glyphs: Vec<(f32, char)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// これ以降の命令を、この矩形でクリップする。`None` で解除する。
    Clip(Option<Rect>),
    Fill {
        rect: Rect,
        colour: Colour,
    },
    Border {
        rect: Rect,
        width: f32,
        colours: [Colour; 4],
    },
    Text(Run),
    Image {
        rect: Rect,
        src: String,
    },
    Video {
        rect: Rect,
    },
}

#[derive(Debug, Clone, Default)]
pub struct Laid {
    pub items: Vec<Item>,
    pub boxes: Vec<Option<Rect>>,
    pub size: (f32, f32),
}

pub fn lay_out(document: &Document, styles: &[Style], clut: &Clut) -> Laid {
    let mut laid = Laid {
        boxes: vec![None; document.nodes.len()],
        ..Default::default()
    };
    let Some(root) = document.root else {
        return laid;
    };
    let Some(body) = document.first_named("body") else {
        return laid;
    };
    let _ = root;
    let (width, height) = styles[body.0].resolution.unwrap_or((960, 540));
    laid.size = (width as f32, height as f32);
    let page = Rect {
        x: 0.0,
        y: 0.0,
        w: width as f32,
        h: height as f32,
    };
    block(document, styles, clut, body, page, page, &mut laid);
    laid
}

/// 一つの箱。`origin` は親の内側、`clip` は見える範囲。
fn block(
    document: &Document,
    styles: &[Style],
    clut: &Clut,
    id: NodeId,
    origin: Rect,
    clip: Rect,
    laid: &mut Laid,
) {
    let Some(element) = document.element(id) else {
        return;
    };
    let style = &styles[id.0];
    if style.display == Display::None {
        return;
    }
    let rect = if element.name == "body" {
        origin
    } else {
        Rect {
            x: origin.x + style.left,
            y: origin.y + style.top,
            w: style.width,
            h: style.height,
        }
    };
    laid.boxes[id.0] = Some(rect);
    let visible = style.visibility == Visibility::Visible;
    let shown = rect.intersect(&clip);
    if visible && shown.w > 0.0 && shown.h > 0.0 {
        laid.items.push(Item::Clip(Some(shown)));
        match element.name.as_str() {
            "object" => {
                let kind = element.attribute("type").unwrap_or("").to_ascii_lowercase();
                if kind.starts_with("video/") {
                    laid.items.push(Item::Video { rect });
                } else if kind.starts_with("image/")
                    && let Some(src) = element.attribute("data")
                {
                    let background = clut.colour(style.background);
                    if background[3] != 0 {
                        laid.items.push(Item::Fill {
                            rect,
                            colour: background,
                        });
                    }
                    laid.items.push(Item::Image {
                        rect,
                        src: src.to_owned(),
                    });
                }
            }
            _ => {
                let background = clut.colour(style.background);
                if background[3] != 0 {
                    laid.items.push(Item::Fill {
                        rect,
                        colour: background,
                    });
                }
                if style.border_style == BorderStyle::Solid && style.border_width > 0.0 {
                    laid.items.push(Item::Border {
                        rect,
                        width: style.border_width,
                        colours: style.border_colour.map(|index| clut.colour(index)),
                    });
                }
                if matches!(element.name.as_str(), "p" | "pre" | "input" | "textarea") {
                    let inner = Rect {
                        x: rect.x + style.border_width + style.padding[3],
                        y: rect.y + style.border_width + style.padding[0],
                        w: (rect.w
                            - 2.0 * style.border_width
                            - style.padding[1]
                            - style.padding[3])
                            .max(0.0),
                        h: (rect.h
                            - 2.0 * style.border_width
                            - style.padding[0]
                            - style.padding[2])
                            .max(0.0),
                    };
                    let text = if element.name == "input" {
                        vec![(
                            element.attribute("value").unwrap_or("").to_owned(),
                            style.clone(),
                        )]
                    } else {
                        let mut pieces = Vec::new();
                        inline_text(document, styles, id, style, &mut pieces);
                        pieces
                    };
                    paragraph(&text, style, inner, clut, laid);
                }
            }
        }
        laid.items.push(Item::Clip(None));
    }
    let inner = Rect {
        x: rect.x + style.border_width,
        y: rect.y + style.border_width,
        w: rect.w,
        h: rect.h,
    };
    let child_clip = if visible { shown } else { Rect::default() };
    for child in &element.children {
        if styles[child.0].display == Display::Block {
            block(document, styles, clut, *child, inner, child_clip, laid);
        }
    }
}

fn inline_text(
    document: &Document,
    styles: &[Style],
    id: NodeId,
    style: &Style,
    out: &mut Vec<(String, Style)>,
) {
    let Some(element) = document.element(id) else {
        return;
    };
    for child in &element.children {
        match &document.node(*child).kind {
            Kind::Text(text) => out.push((text.clone(), style.clone())),
            Kind::Element(inner) => match inner.name.as_str() {
                "br" => out.push(("\n".to_owned(), style.clone())),
                _ if styles[child.0].display == Display::Inline => {
                    inline_text(document, styles, *child, &styles[child.0], out);
                }
                _ => {}
            },
        }
    }
}

fn paragraph(pieces: &[(String, Style)], style: &Style, inner: Rect, clut: &Clut, laid: &mut Laid) {
    let line_height = style.line_height.unwrap_or(style.font_size);
    let mut lines: Vec<Vec<(char, f32, &Style)>> = vec![Vec::new()];
    let mut used = 0.0;
    for (text, piece) in pieces {
        for letter in text.chars() {
            match letter {
                '\n' => {
                    lines.push(Vec::new());
                    used = 0.0;
                    continue;
                }
                '\r' => continue,
                _ => {}
            }
            let letter = if letter == '\t' { ' ' } else { letter };
            let width = if is_wide(letter) {
                piece.font_size
            } else {
                piece.font_size / 2.0
            } + piece.letter_spacing;
            if used + width > inner.w + 0.01 && used > 0.0 {
                lines.push(Vec::new());
                used = 0.0;
            }
            lines.last_mut().unwrap().push((letter, width, piece));
            used += width;
        }
    }
    let mut y = inner.y;
    for line in &lines {
        let total: f32 = line.iter().map(|(_, width, _)| width).sum();
        let mut x = inner.x
            + match style.text_align {
                crate::document::style::Align::Left => 0.0,
                crate::document::style::Align::Center => ((inner.w - total) / 2.0).max(0.0),
                crate::document::style::Align::Right => (inner.w - total).max(0.0),
            };
        let mut run: Option<Run> = None;
        for (letter, width, piece) in line {
            let colour = clut.colour(piece.colour);
            let same = run.as_ref().is_some_and(|run| {
                run.size == piece.font_size
                    && run.colour == colour
                    && run.family == piece.family
                    && run.bold == piece.bold
            });
            if !same {
                if let Some(done) = run.take() {
                    laid.items.push(Item::Text(done));
                }
                run = Some(Run {
                    x,
                    y: y + (line_height - piece.font_size) / 2.0,
                    size: piece.font_size,
                    colour,
                    family: piece.family,
                    bold: piece.bold,
                    glyphs: Vec::new(),
                });
            }
            if *letter != ' ' {
                let run = run.as_mut().unwrap();
                run.glyphs.push((x - run.x, *letter));
            }
            x += width;
        }
        if let Some(done) = run.take()
            && !done.glyphs.is_empty()
        {
            laid.items.push(Item::Text(done));
        }
        y += line_height;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{css, dom, style};
    use std::collections::BTreeMap;

    fn laid(markup: &str, sheet: &str) -> (Document, Laid) {
        let document = dom::parse(markup);
        let sheets = css::parse_stylesheet(sheet);
        let styles = style::compute(&document, &sheets, &BTreeMap::new());
        let laid = lay_out(&document, &styles, &Clut::default());
        (document, laid)
    }

    #[test]
    fn boxes_are_placed_inside_their_parents() {
        let (document, laid) = laid(
            r#"<bml><body style="background-color-index:8"><div id="d" style="left:10px;top:20px;width:100px;height:50px;"><p id="p" style="left:5px;top:5px;width:50px;height:20px;background-color-index:7;">あa</p></div></body></bml>"#,
            "",
        );
        let p = document.by_id("p").unwrap();
        assert_eq!(
            laid.boxes[p.0],
            Some(Rect {
                x: 15.0,
                y: 25.0,
                w: 50.0,
                h: 20.0
            })
        );
        let text = laid
            .items
            .iter()
            .find_map(|item| match item {
                Item::Text(run) => Some(run),
                _ => None,
            })
            .expect("text");
        assert_eq!(text.glyphs, [(0.0, 'あ'), (24.0, 'a')]);
        assert_eq!(text.colour, [0, 0, 0, 255]);
    }

    #[test]
    fn text_wraps_on_the_grid_and_breaks_at_br() {
        let (_, laid) = laid(
            r#"<bml><body><p style="width:32px;height:64px;font-size:16px;">あいう<br/>え</p></body></bml>"#,
            "",
        );
        let runs: Vec<&Run> = laid
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Text(run) => Some(run),
                _ => None,
            })
            .collect();
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].glyphs.len(), 2);
        assert_eq!(runs[1].glyphs, [(0.0, 'う')]);
        assert_eq!(runs[1].y, 16.0);
        assert_eq!(runs[2].y, 32.0);
    }

    #[test]
    fn a_hidden_box_hides_its_children_and_a_video_object_is_placed() {
        let (_, laid) = laid(
            r#"<bml><body><div style="width:100px;height:100px;visibility:hidden"><p style="width:10px;height:10px">x</p></div><object type="video/X-arib-mpeg2" style="left:48px;top:93px;width:480px;height:270px;"/></body></bml>"#,
            "",
        );
        assert!(!laid.items.iter().any(|item| matches!(item, Item::Text(_))));
        assert!(laid.items.iter().any(|item| matches!(
            item,
            Item::Video {
                rect: Rect {
                    x: 48.0,
                    y: 93.0,
                    w: 480.0,
                    h: 270.0
                }
            }
        )));
    }
}
