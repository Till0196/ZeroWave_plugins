// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! スクリプトの無い文書（BML から変換した HTML）を Blitz でレンダリングする。画像は `arib-bml://res/<名前>` で参照される。

use std::collections::BTreeMap;
use std::sync::Arc;

use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_traits::net::{NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};

pub use crate::paint::VideoRect;

#[derive(Debug, Clone, Default)]
pub struct Files(pub Arc<BTreeMap<String, Vec<u8>>>);

impl NetProvider for Files {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.as_str();
        let Some(name) = url.strip_prefix(bml::document::html::IMAGE_BASE) else {
            return;
        };
        let name = percent_decode(name);
        if let Some(body) = self.0.get(&name) {
            handler.bytes(url.to_owned(), body.clone().into());
        }
    }
}

/// `%E3%81%82` を戻す（Blitz が URL を正規化するので日本語の名前はこう来る）。
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && at + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&text[at + 1..at + 3], 16)
        {
            out.push(v);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Took {
    pub open: f64,
    pub paint: f64,
    pub second: f64,
    pub shrink: f64,
    pub drawn: (u32, u32),
}

pub struct Still {
    document: HtmlDocument,
    size: (u32, u32),
    scale: f64,
}

impl std::fmt::Debug for Still {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "Still({}x{})", self.size.0, self.size.1)
    }
}

impl Still {
    pub fn open(
        html: &str,
        size: (u32, u32),
        scale: f64,
        files: Files,
        faces: &crate::fonts::Faces,
    ) -> Self {
        let mut document = HtmlDocument::from_html(
            html,
            DocumentConfig {
                base_url: Some(String::from(bml::document::html::IMAGE_BASE)),
                net_provider: Some(Arc::new(files) as _),
                viewport: Some(Viewport::new(
                    (size.0 as f64 * scale).floor() as u32,
                    (size.1 as f64 * scale).floor() as u32,
                    scale as f32,
                    ColorScheme::Light,
                )),
                font_ctx: Some(crate::fonts::context(faces)),
                ua_stylesheets: vec![blitz_dom::UserAgentStylesheet::default_sheet()],
                ..Default::default()
            },
        );
        crate::paint::punch(&mut document);
        Self {
            document,
            size,
            scale,
        }
    }

    pub fn video_rect(&mut self) -> Option<VideoRect> {
        crate::paint::video_rect(&mut self.document)
    }

    pub fn paint(&mut self) -> (Vec<u8>, u32, u32) {
        crate::paint::paint(&mut self.document, self.size, self.scale)
    }

    /// 端数の倍率では切り上げた整数倍でレンダリングして面積平均で縮める（外字の二次元コードに筋が出ないように）。
    /// 整数倍は [`MOST`] まで。
    pub fn paint_on_pixel_grid(
        markup: &str,
        size: (u32, u32),
        scale: f64,
        files: Files,
        faces: &crate::fonts::Faces,
    ) -> (Vec<u8>, u32, u32, Option<crate::paint::VideoRect>, Took) {
        let ms = |since: std::time::Instant| since.elapsed().as_secs_f64() * 1000.0;
        let mut took = Took::default();
        let grid = grid_scale(scale).min(most_scale(size));
        let began = std::time::Instant::now();
        let mut still = Self::open(markup, size, grid, files, faces);
        let video = still.video_rect();
        took.open = ms(began);
        let began = std::time::Instant::now();
        let (rgba, width, height) = still.paint();
        took.second = crate::paint::second_ms();
        took.paint = ms(began) - took.second;
        took.drawn = (width, height);
        if grid <= scale + 1e-6 {
            return (rgba, width, height, video, took);
        }
        let to = (
            (f64::from(size.0) * scale).floor() as u32,
            (f64::from(size.1) * scale).floor() as u32,
        );
        let began = std::time::Instant::now();
        let shrunk = shrink(&rgba, (width, height), to);
        took.shrink = ms(began);
        (shrunk, to.0, to.1, video, took)
    }

    pub fn rect_of(&mut self, selector: &str) -> Option<(f32, f32, f32, f32)> {
        self.document.resolve(0.0);
        let id = self.document.query_selector(selector).ok().flatten()?;
        let node = self.document.get_node(id)?;
        let layout = node.final_layout();
        let (mut x, mut y) = (layout.location.x, layout.location.y);
        let mut parent = node.parent;
        while let Some(p) = parent {
            let n = self.document.get_node(p)?;
            x += n.final_layout().location.x;
            y += n.final_layout().location.y;
            parent = n.parent;
        }
        Some((x, y, layout.size.width, layout.size.height))
    }
}

/// レンダリングする画素の上限。BML（960x540）なら 2 倍まで。
pub const MOST: (u32, u32) = (1920, 1080);

fn most_scale(size: (u32, u32)) -> f64 {
    if size.0 == 0 || size.1 == 0 {
        return 1.0;
    }
    (f64::from(MOST.0) / f64::from(size.0))
        .min(f64::from(MOST.1) / f64::from(size.1))
        .floor()
        .max(1.0)
}

fn grid_scale(scale: f64) -> f64 {
    if (scale - scale.round()).abs() < 1e-6 {
        scale.round().max(1.0)
    } else {
        scale.ceil()
    }
}

/// RGBA（掛けていない α）を面積平均で縮める。α を重みに混ぜる。
pub fn shrink(rgba: &[u8], from: (u32, u32), to: (u32, u32)) -> Vec<u8> {
    let (fw, fh) = (from.0 as usize, from.1 as usize);
    let (tw, th) = (to.0 as usize, to.1 as usize);
    let mut out = vec![0u8; tw * th * 4];
    if fw == 0 || fh == 0 || tw == 0 || th == 0 {
        return out;
    }
    let sx = fw as f64 / tw as f64;
    let sy = fh as f64 / th as f64;
    let spans = |n: usize, step: f64, limit: usize| -> Vec<Vec<(usize, f64)>> {
        (0..n)
            .map(|i| {
                let (a, b) = (i as f64 * step, (i as f64 + 1.0) * step);
                let mut taps = Vec::new();
                let mut k = a.floor() as usize;
                while (k as f64) < b && k < limit {
                    let w = (b.min(k as f64 + 1.0) - a.max(k as f64)).max(0.0);
                    if w > 0.0 {
                        taps.push((k, w));
                    }
                    k += 1;
                }
                taps
            })
            .collect()
    };
    let xs = spans(tw, sx, fw);
    let ys = spans(th, sy, fh);
    for (y, ytaps) in ys.iter().enumerate() {
        for (x, xtaps) in xs.iter().enumerate() {
            let (mut r, mut g, mut b, mut a, mut area) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for &(sy, wy) in ytaps {
                for &(sx, wx) in xtaps {
                    let w = wx * wy;
                    let at = (sy * fw + sx) * 4;
                    let alpha = f64::from(rgba[at + 3]) * w;
                    r += f64::from(rgba[at]) * alpha;
                    g += f64::from(rgba[at + 1]) * alpha;
                    b += f64::from(rgba[at + 2]) * alpha;
                    a += alpha;
                    area += w;
                }
            }
            let at = (y * tw + x) * 4;
            if a > 0.0 {
                out[at] = (r / a).round() as u8;
                out[at + 1] = (g / a).round() as u8;
                out[at + 2] = (b / a).round() as u8;
            }
            out[at + 3] = (a / area).round().min(255.0) as u8;
        }
    }
    out
}

#[cfg(test)]
mod grid_tests {
    use super::{grid_scale, most_scale, shrink};

    #[test]
    fn whole_scales_stop_at_full_hd() {
        assert_eq!(most_scale((960, 540)), 2.0);
        assert_eq!(most_scale((720, 480)), 2.0);
        assert_eq!(most_scale((240, 480)), 2.0);
        assert_eq!(most_scale((1920, 1080)), 1.0);
        assert_eq!(most_scale((3840, 2160)), 1.0);
    }

    #[test]
    fn fractional_scales_are_drawn_at_the_next_whole_scale() {
        assert_eq!(grid_scale(1.2104), 2.0);
        assert_eq!(grid_scale(2.0), 2.0);
        assert_eq!(grid_scale(0.6), 1.0);
        assert_eq!(grid_scale(1.0000000001), 1.0);
    }

    #[test]
    fn shrinking_keeps_runs_unbroken_and_ignores_transparent_colour() {
        let mut rgba = Vec::new();
        for y in 0..4 {
            for _ in 0..4 {
                rgba.extend_from_slice(if y < 2 {
                    &[0, 0, 0, 255]
                } else {
                    &[255, 255, 255, 0]
                });
            }
        }
        let out = shrink(&rgba, (4, 4), (3, 3));
        assert_eq!(&out[0..4], &[0, 0, 0, 255]);
        let middle = &out[12..16];
        assert_eq!(&middle[..3], &[0, 0, 0]);
        assert!((120..=135).contains(&middle[3]), "alpha {}", middle[3]);
        assert_eq!(out[24 + 3], 0);
    }
}
