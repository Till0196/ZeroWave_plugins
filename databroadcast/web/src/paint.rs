// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 文書を RGBA にレンダリングする。映像の場所は透かす。`Compose::Clear` では `z-index` の層の
//! 下の背景が残るので、黒と白で二度レンダリングし、差から上に重なった物の透明度を求める。

use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::BaseDocument;
use blitz_paint::paint_scene;

/// 放送映像音声オブジェクトの型。MMT（STD-B62）と Hybridcast（IPTVFJ）で名前が違う。
const VIDEO: &str =
    "object[type=\"video/x-arib2-broadcast\"], object[type=\"video/x-iptvf-broadcast\"]";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

struct Hole;

thread_local! {
    static HOLE: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    static SECOND_MS: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
}

pub(crate) fn second_ms() -> f64 {
    SECOND_MS.with(std::cell::Cell::get)
}

impl blitz_dom::Widget for Hole {
    fn paint(
        &mut self,
        _render_ctx: &mut dyn anyrender::RenderContext,
        _styles: &blitz_dom::node::ComputedStyles,
        width: u32,
        height: u32,
        _scale: f64,
    ) -> anyrender::Scene {
        use anyrender::PaintScene;
        use peniko::kurbo::{Affine, Rect};
        use peniko::{Color, Fill};
        let mut scene = anyrender::Scene::new();
        let rect = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let level = HOLE.with(|h| h.get());
        let colour = Color::from_rgba8(level, level, level, 255);
        scene.fill(Fill::NonZero, Affine::IDENTITY, colour, None, &rect);
        scene
    }
}

pub(crate) fn punch(doc: &mut BaseDocument) {
    let Ok(Some(id)) = doc.query_selector(VIDEO) else {
        return;
    };
    doc.set_custom_widget(id, Box::new(Hole));
}

pub(crate) fn video_rect(doc: &mut BaseDocument) -> Option<VideoRect> {
    doc.resolve(0.0);
    let id = doc.query_selector(VIDEO).ok().flatten()?;
    let node = doc.get_node(id)?;
    let layout = node.final_layout();
    let mut x = layout.location.x;
    let mut y = layout.location.y;
    let mut parent = node.parent;
    while let Some(p) = parent {
        let n = doc.get_node(p)?;
        x += n.final_layout().location.x;
        y += n.final_layout().location.y;
        parent = n.parent;
    }
    Some(VideoRect {
        x,
        y,
        w: layout.size.width,
        h: layout.size.height,
    })
}

/// レンダリングした RGBA（`size × scale`）。切り上げると映像を透かす領域の縁が半透明の白い線になるので切り捨てる。
pub(crate) fn paint(doc: &mut BaseDocument, size: (u32, u32), scale: f64) -> (Vec<u8>, u32, u32) {
    let (w, h) = (
        (size.0 as f64 * scale).floor() as u32,
        (size.1 as f64 * scale).floor() as u32,
    );
    doc.resolve(0.0);
    HOLE.with(|h| h.set(0));
    SECOND_MS.with(|s| s.set(0.0));
    let mut over_black = render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| paint_scene(scene, doc, scale, w, h, 0, 0),
        w,
        h,
    );
    if video_rect(doc).is_none() {
        return (over_black, w, h);
    }
    let second = std::time::Instant::now();
    HOLE.with(|h| h.set(255));
    let over_white = render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| paint_scene(scene, doc, scale, w, h, 0, 0),
        w,
        h,
    );
    // 黒の上 A = α·c、白の上 B = α·c + (1−α)·255 から α = 1 − (B − A)/255、c = A/α。
    for (a, b) in over_black
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(over_white.as_chunks::<4>().0)
    {
        let (pa, pb) = (
            u32::from_le_bytes(*a) & 0x00ff_ffff,
            u32::from_le_bytes(*b) & 0x00ff_ffff,
        );
        if pa == pb {
            continue;
        }
        if pa == 0 && pb == 0x00ff_ffff {
            *a = [0, 0, 0, 0];
            continue;
        }
        let gap = (0..3)
            .map(|i| f32::from(b[i]) - f32::from(a[i]))
            .sum::<f32>()
            / 3.0;
        let alpha = (1.0 - gap / 255.0).clamp(0.0, 1.0);
        if alpha <= 0.002 {
            a.copy_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        for channel in a.iter_mut().take(3) {
            *channel = (f32::from(*channel) / alpha).round().clamp(0.0, 255.0) as u8;
        }
        a[3] = (alpha * 255.0).round() as u8;
    }
    SECOND_MS.with(|s| s.set(second.elapsed().as_secs_f64() * 1000.0));
    (over_black, w, h)
}

#[cfg(test)]
mod tests {
    use crate::still::{Files, Still};

    fn pixel(buffer: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * width + x) * 4) as usize;
        buffer[at..at + 4].try_into().expect("四つ")
    }

    /// CSS 2.1 付録 E の 8 段目: `z-index: 0` と `auto` の positioned な箱は同じ段でツリーの順。
    #[test]
    fn a_z_index_zero_box_in_a_static_wrapper_paints_above_an_earlier_auto_box() {
        let html = "<html><body style='margin:0'>\
            <div style='position:absolute;left:0;top:0;width:100px;height:100px;background:#00f'></div>\
            <div><div style='position:absolute;left:0;top:50px;width:100px;height:50px;\
                overflow:hidden;z-index:0;background:#f00'></div></div>\
            </body></html>";
        let faces = crate::fonts::Faces::default();
        let mut still = Still::open(html, (100, 100), 1.0, Files::default(), &faces);
        let (buffer, width, _) = still.paint();
        assert_eq!(pixel(&buffer, width, 50, 25), [0, 0, 255, 255]);
        assert_eq!(pixel(&buffer, width, 50, 75), [255, 0, 0, 255]);
    }

    #[test]
    fn content_that_exactly_fits_its_box_does_not_wrap_at_a_fractional_scale() {
        let faces = crate::fonts::Faces::find();
        if faces.is_empty() {
            zwsdk::plug::info("フォントが無いので飛ばす");
            return;
        }
        let html = "<html><body style='margin:0;word-break:break-all;font-size:28px'>\
            <div><span id='a' style='display:inline-block;padding:5px 15px;border:2px solid #000;\
            line-height:1.2em'>首都圏</span></div>\
            <div><span id='b' style='display:inline-block;padding:0;border:0;line-height:1.2em'>首都圏</span></div>\
            </body></html>";
        let mut still = Still::open(html, (1920, 1080), 0.75, Files::default(), &faces);
        let (_, _, _, one_line) = still.rect_of("#b").expect("b");
        let (_, _, _, height) = still.rect_of("#a").expect("a");
        assert!(
            (height - (one_line + 10.0 + 2.0 * 4.0 / 3.0)).abs() < 1.0,
            "二行に折れている: {height} vs {one_line}"
        );
    }

    #[test]
    fn text_shadow_is_painted_under_the_glyphs() {
        let faces = crate::fonts::Faces::find();
        if faces.is_empty() {
            zwsdk::plug::info("フォントが無いので飛ばす");
            return;
        }
        let html = "<html><body style='margin:0;background:#fff'>\
            <div style='position:absolute;left:0;top:0;font-size:80px;line-height:100px;\
            color:#000;text-shadow:40px 0 0 #f00'>I</div></body></html>";
        let mut still = Still::open(html, (200, 100), 1.0, Files::default(), &faces);
        let (buffer, width, height) = still.paint();
        let count = |want: [u8; 3]| {
            (0..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .filter(|&(x, y)| pixel(&buffer, width, x, y)[..3] == want)
                .count()
        };
        assert!(count([0, 0, 0]) > 0, "文字が無い");
        assert!(count([255, 0, 0]) > 0, "影が無い");
    }

    /// 縦長の背景画像を background-size で半分にして敷き詰めると、文書の右端まで、半分の幅の周期で描かれる
    /// （BML の body の背景、STD-B24 第二分冊 付属2 A2-4.4.7）。画像は左 8 画素が赤、右 8 画素が青。
    #[test]
    fn body_background_strip_tiles_across() {
        const RED: [u8; 3] = [0xc0, 0x20, 0x20];
        const BLUE: [u8; 3] = [0x20, 0x40, 0xc0];
        let (width, height) = (16u32, 1080u32);
        let row: Vec<u8> = (0..width)
            .flat_map(|x| if x < 8 { RED } else { BLUE })
            .collect();
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, width, height);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            writer
                .write_image_data(&row.repeat(height as usize))
                .expect("data");
        }
        let mut files = std::collections::BTreeMap::new();
        files.insert(String::from("bg.png"), encoded);
        let base = bml::document::html::IMAGE_BASE;
        let html = format!(
            "<html><body style=\"margin:0;width:960px;height:540px;background-image:url({base}bg.png);\
             background-repeat:repeat;background-size:8px 540px;\"></body></html>"
        );
        let faces = crate::fonts::Faces::default();
        let (rgba, width, height, _, _) = Still::paint_on_pixel_grid(
            &html,
            (960, 540),
            1.0,
            Files(std::sync::Arc::new(files)),
            &faces,
        );
        for (x, want) in [
            (1, RED),
            (6, BLUE),
            (481, RED),
            (486, BLUE),
            (953, RED),
            (958, BLUE),
        ] {
            let p = pixel(&rgba, width, x, height / 2);
            let near = p
                .iter()
                .zip(want.iter().chain(&[0xff]))
                .all(|(got, want)| got.abs_diff(*want) <= 2);
            assert!(near, "x={x}: {p:?}");
        }
    }
}
