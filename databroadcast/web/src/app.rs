// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 一つのアプリ。文書を開き、スクリプトを実行し、レイアウトして、レンダリングする。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use blitz_dom::{Document, DocumentConfig, NodeId};
use blitz_traits::events::{BlitzKeyEvent, DomEvent, DomEventData, KeyState};
use blitz_traits::shell::{ColorScheme, Viewport};
use blitz_vibey_script::ScriptDocument;
use keyboard_types::{Code, Key, Location, Modifiers};
use url::Url;

use crate::api::{Bridge, Host, Request, SharedBridge};
use crate::net::{BASE, Shared};

/// 受像機が足す CSS。キーフレームは止める（無限のものが再レンダリングを止めない）が、
/// トランジションは回す（`transitionend` で進むアプリがある）。Blitz は `<object>` を
/// replaced と見ないので、放送の映像オブジェクトに幅と高さを持たせる。
const RECEIVER_CSS: &str = "\
* { animation: none !important; }\
object[type=\"video/x-arib2-broadcast\"], object[type=\"video/x-iptvf-broadcast\"] { display: inline-block; }\
object[type=\"video/x-arib2-broadcast\"] > param, object[type=\"video/x-iptvf-broadcast\"] > param { display: none; }";

/// [`RECEIVER_CSS`] を一度だけ解析した物（同じ UA のスタイルシートの文書どうしでカスケードを共有する）。
fn receiver_stylesheet() -> blitz_dom::UserAgentStylesheet {
    thread_local! {
        static SHEET: blitz_dom::UserAgentStylesheet =
            blitz_dom::UserAgentStylesheet::parse(RECEIVER_CSS);
    }
    SHEET.with(Clone::clone)
}

pub struct App {
    document: ScriptDocument,
    bridge: SharedBridge,
    store: Shared,
    pub size: (u32, u32),
    loaded_images: std::collections::BTreeMap<blitz_dom::NodeId, String>,
    pub scale: f64,
    faces: crate::fonts::Faces,
    keyed: bool,
    began_ms: f64,
}

impl std::fmt::Debug for App {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "App({}x{})", self.size.0, self.size.1)
    }
}

/// 字幕の MFU をリスナーに渡す JSON に（TR-B39 第 2 編 図 8-23）。中身は TTML と SVG のフォントだけ。
fn caption_json(stream: &crate::api::Caption, unit: &isdb::mhcaption::Mfu<'_>) -> String {
    use isdb::mhcaption::{DATA_FONT_SVG, DATA_TTML};
    let body = match unit.data_type {
        DATA_TTML if stream.compression == 1 => {
            miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(
                unit.data,
                crate::store::INFLATED_MOST,
            )
            .unwrap_or_default()
        }
        DATA_TTML | DATA_FONT_SVG => unit.data.to_vec(),
        _ => Vec::new(),
    };
    let body = String::from_utf8_lossy(&body);
    let (seconds, fraction) = stream
        .reference_start
        .map_or((0, 0), |ntp| ((ntp >> 32) as u32, ntp as u32));
    format!(
        concat!(
            "{{\"ISO_639_language_code\":{},\"TMD\":\"{:04b}\",\"resolution\":\"{:04b}\",",
            "\"reference_start_time_seconds\":{},\"reference_start_time_fraction\":{},",
            "\"subtitle_sequence_number\":{},\"subsample_number\":{},",
            "\"last_subsample_number\":{},\"data_type\":\"{:04b}\",\"data\":{}}}"
        ),
        json_string(&stream.language),
        stream.tmd,
        stream.resolution,
        seconds,
        fraction,
        unit.sequence,
        unit.subsample,
        unit.last_subsample,
        unit.data_type,
        json_string(&body),
    )
}

fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn key_of(code: u16) -> Key {
    match code {
        38 => Key::ArrowUp,
        40 => Key::ArrowDown,
        37 => Key::ArrowLeft,
        39 => Key::ArrowRight,
        13 => Key::Enter,
        461 => Key::GoBack,
        403 => Key::ColorF0Red,
        404 => Key::ColorF1Green,
        405 => Key::ColorF2Yellow,
        406 => Key::ColorF3Blue,
        460 => Key::Subtitle,
        33 => Key::PageUp,
        34 => Key::PageDown,
        9 => Key::Tab,
        48..=57 => Key::Character(((b'0' + (code - 48) as u8) as char).to_string()),
        1001 => Key::Character(String::from("d")),
        _ => Key::Unidentified,
    }
}

pub use crate::paint::VideoRect;

fn fit(html: &str, size: (u32, u32), scale: f64) -> ((u32, u32), f64) {
    let Some(width) = viewport_width(html) else {
        return (size, scale);
    };
    if width == 0 || width == size.0 {
        return (size, scale);
    }
    let screen = size.0 as f64 * scale;
    let height = (u64::from(width) * u64::from(size.1) / u64::from(size.0)) as u32;
    ((width, height), screen / f64::from(width))
}

fn viewport_width(html: &str) -> Option<u32> {
    let head = &html[..html.len().min(16_384)];
    let lower = head.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("<meta") {
        let start = from + at;
        let Some(end) = lower[start..].find('>') else {
            break;
        };
        let tag = &lower[start..start + end];
        from = start + end;
        if !tag.contains("viewport") {
            continue;
        }
        let Some(content) = tag.find("content=") else {
            continue;
        };
        let rest = tag[content + 8..].trim_start_matches(['"', '\'']);
        if let Some(width) = rest
            .split([',', ';', '"', '\'', ' '])
            .find_map(|part| part.trim().strip_prefix("width="))
            .and_then(|w| w.trim().parse().ok())
        {
            return Some(width);
        }
    }
    None
}

fn resolve(name: &str) -> Option<Url> {
    if name.starts_with("http://") || name.starts_with("https://") {
        return Url::parse(name).ok();
    }
    let path = name.strip_prefix(BASE).unwrap_or(name);
    Url::parse(BASE)
        .ok()?
        .join(path.trim_start_matches('/'))
        .ok()
}

impl App {
    pub fn navigate(&mut self, url: &str) -> bool {
        let Some(url) = resolve(url) else {
            return false;
        };
        let Some((_, html)) = self.store.fetch(&url) else {
            return false;
        };
        let html = crate::script::rewrite_inline(&String::from_utf8_lossy(&html));
        let (size, scale) = fit(&html, self.size, self.scale);
        self.size = size;
        self.scale = scale;
        crate::api::canvas::forget_canvases();
        // `keySet` は文書ごと。移った先は既定から（運用規定 1-3）。
        self.bridge.borrow_mut().key_set = None;
        let bridge = Rc::clone(&self.bridge);
        let store = self.store.clone();
        let mut document = ScriptDocument::from_html_with_host_hooks(
            &html,
            DocumentConfig {
                base_url: Some(url.to_string()),
                net_provider: Some(Arc::new(store.clone()) as _),
                viewport: Some(Viewport::new(
                    (self.size.0 as f64 * self.scale).floor() as u32,
                    (self.size.1 as f64 * self.scale).floor() as u32,
                    self.scale as f32,
                    ColorScheme::Light,
                )),
                font_ctx: Some(crate::fonts::context(&self.faces)),
                ua_stylesheets: vec![
                    blitz_dom::UserAgentStylesheet::default_sheet(),
                    receiver_stylesheet(),
                ],
                ..Default::default()
            },
            Rc::new(bml::runtime::clock::Jst),
        )
        // タイマーは `tick` で自分で回す（wasm にはスレッドが無い）。
        .without_timer_thread()
        .with_fetcher(store.clone());
        document.with_context(move |context| crate::api::install(context, bridge, store));
        crate::paint::punch(&mut document.inner_mut());
        document.inner_mut().start_animation_clock();
        document.execute_scripts();
        self.document = document;
        self.keyed = false;
        self.loaded_images.clear();
        true
    }

    /// ストアの中のパスの文書を開く。`scale` はレイアウトの時から決める（レンダリングのときに縮めると文字だけずれる）。
    pub fn open(
        store: Shared,
        path: &str,
        host: Host,
        size: (u32, u32),
        scale: f64,
        faces: &crate::fonts::Faces,
    ) -> Option<Self> {
        let url = resolve(path)?;
        let (_, html) = store.fetch(&url)?;
        let html = crate::script::rewrite_inline(&String::from_utf8_lossy(&html));
        let (size, scale) = fit(&html, size, scale);
        crate::api::name_as(&host);
        crate::api::canvas::forget_canvases();
        let bridge = Rc::new(RefCell::new(Bridge {
            host,
            ..Default::default()
        }));
        let mut document = ScriptDocument::from_html_with_host_hooks(
            &html,
            DocumentConfig {
                base_url: Some(url.to_string()),
                net_provider: Some(Arc::new(store.clone()) as _),
                viewport: Some(Viewport::new(
                    (size.0 as f64 * scale).floor() as u32,
                    (size.1 as f64 * scale).floor() as u32,
                    scale as f32,
                    ColorScheme::Light,
                )),
                font_ctx: Some(crate::fonts::context(faces)),
                ua_stylesheets: vec![
                    blitz_dom::UserAgentStylesheet::default_sheet(),
                    receiver_stylesheet(),
                ],
                ..Default::default()
            },
            Rc::new(bml::runtime::clock::Jst),
        )
        .without_timer_thread()
        .with_fetcher(store.clone());
        let (b, s) = (Rc::clone(&bridge), store.clone());
        crate::api::canvas::install_fonts(faces);
        document.with_context(move |context| crate::api::install(context, b, s));
        crate::paint::punch(&mut document.inner_mut());
        document.inner_mut().start_animation_clock();
        document.execute_scripts();
        Some(Self {
            document,
            bridge,
            store,
            size,
            scale,
            faces: faces.clone(),
            began_ms: zwsdk::plug::host::monotonic_ms(),
            keyed: false,
            loaded_images: std::collections::BTreeMap::new(),
        })
    }

    pub fn tick(&mut self) -> bool {
        let store = self.store.clone();
        let answered = self.document.with_context(|context| {
            crate::api::receiver::answer_cache_listeners(context, &|path| {
                store
                    .0
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .directory_state(path)
            })
        });
        let ran = self.document.poll(None) || answered;
        // 配る前に一度レイアウトする。でないとリスナーが替えた class に変化前のスタイルが無く、
        // トランジションが始まらない。
        if ran {
            self.document.inner_mut().resolve(0.0);
        }
        let loaded = self.fire_image_loads();
        let ended = self.fire_audio_ended();
        let transitioned = self.fire_transition_ends();
        let animating = self.document.inner().is_animating();
        ran || loaded || ended || transitioned || animating
    }

    fn fire_transition_ends(&mut self) -> bool {
        self.document.inner_mut().resolve(0.0);
        let transitions = self.document.inner_mut().take_finished_transitions();
        let animations = self.document.inner_mut().take_finished_animations();
        let mut any = false;
        for (id, _property) in transitions {
            any |= self
                .document
                .dispatch_simple_event(id, "transitionend", true);
        }
        for (id, _name) in animations {
            any |= self
                .document
                .dispatch_simple_event(id, "animationend", true);
        }
        any
    }

    fn fire_audio_ended(&mut self) -> bool {
        let now = crate::api::date::now_millis() as u64;
        let done: Vec<blitz_dom::NodeId> = self
            .document
            .inner()
            .tree()
            .iter()
            .filter(|(_, node)| {
                node.attr(blitz_dom::LocalName::from(crate::api::dom::PLAYING_UNTIL))
                    .and_then(|until| until.parse::<u64>().ok())
                    .is_some_and(|until| until <= now)
            })
            .map(|(id, _)| id)
            .collect();
        let mut any = false;
        for id in done {
            let name = blitz_dom::QualName::new(
                None,
                blitz_dom::ns!(),
                blitz_dom::LocalName::from(crate::api::dom::PLAYING_UNTIL),
            );
            self.document.inner_mut().mutate().clear_attribute(id, name);
            any |= self.document.dispatch_simple_event(id, "ended", false);
        }
        any
    }

    /// `img` の `load`。Blitz は発生させないので、読めた画像に一度だけ送る（読めなければ `error`）。
    fn fire_image_loads(&mut self) -> bool {
        let pending: Vec<(blitz_dom::NodeId, String, bool)> = {
            let doc = self.document.inner();
            let base = doc.base_url().clone();
            // ツリーの全部を見る。どこにも付いていない `img` にも `load` を待つアプリがある。
            doc.tree()
                .iter()
                .filter(|(_, node)| {
                    node.data
                        .is_element_with_tag_name(&blitz_dom::local_name!("img"))
                })
                .filter_map(|(id, node)| {
                    let src = node.attr(blitz_dom::local_name!("src"))?;
                    if self
                        .loaded_images
                        .get(&id)
                        .is_some_and(|fired| fired == src)
                    {
                        return None;
                    }
                    let url = base.join(src).ok()?;
                    let ok = self.store.fetch(&url).is_some();
                    Some((id, src.to_owned(), ok))
                })
                .collect()
        };
        let mut any = false;
        for (id, src, ok) in pending {
            self.loaded_images.insert(id, src);
            any |=
                self.document
                    .dispatch_simple_event(id, if ok { "load" } else { "error" }, false);
        }
        any
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        let mut out = std::mem::take(&mut self.bridge.borrow_mut().requests);
        for message in self.document.take_messages() {
            out.push(Request::Log(message));
        }
        for error in self.document.take_js_errors() {
            out.push(Request::Log(format!("JS: {error}")));
        }
        out
    }

    pub fn set_host(&mut self, host: Host) {
        crate::api::name_as(&host);
        self.bridge.borrow_mut().host = host;
    }

    pub fn used_keys(&self) -> Vec<String> {
        let set = self.bridge.borrow().key_set_or_default();
        [
            (16, "basic"),
            (32, "numeric-tuning"),
            (128, "data-button"),
            (1 | 2 | 4 | 8, "color"),
        ]
        .into_iter()
        .filter(|(group, _)| set & group != 0)
        .map(|(_, name)| String::from(name))
        .collect()
    }

    pub fn accepts_key(&self, code: u16) -> bool {
        let group = crate::api::receiver::key_group(code);
        group == 0 || self.bridge.borrow().key_set_or_default() & group != 0
    }

    pub fn key(&mut self, code: u16) {
        let key = key_of(code);
        self.probe_key_code();
        let target = self.key_target();
        for state in [KeyState::Pressed, KeyState::Released] {
            let event = BlitzKeyEvent {
                key: key.clone(),
                code: Code::Unidentified,
                modifiers: Modifiers::empty(),
                location: Location::Standard,
                is_auto_repeating: false,
                is_composing: false,
                state,
                text: None,
            };
            let data = match state {
                KeyState::Pressed => DomEventData::KeyDown(event),
                KeyState::Released => DomEventData::KeyUp(event),
            };
            self.document
                .dispatch_dom_event(DomEvent::new(target, data));
        }
        // 決定はフォーカスのある要素の `click` にする（STD-B62 第二分冊、TR-B39）。既定の動作は実行しない
        // （文書の `onclick` が自分で移るので二重になる）。
        if code == 13 {
            let focussed = self.document.inner().get_focussed_node_id();
            if let Some(focussed) = focussed {
                let called = self.document.dispatch_simple_event(focussed, "click", true);
                // `onclick` の属性はリスナーとして登録されていないので、ここで実行する。
                let attribute = self.handler_of(focussed, "onclick");
                if let Some(code) = attribute.clone() {
                    self.eval(&code);
                }
                if !called
                    && attribute.is_none()
                    && let Some(url) = self.link_of(focussed)
                {
                    self.bridge
                        .borrow_mut()
                        .requests
                        .push(crate::api::Request::Navigate(url));
                }
            }
        }
    }

    fn handler_of(&self, node_id: NodeId, name: &str) -> Option<String> {
        let doc = self.document.inner();
        let mut at = Some(node_id);
        while let Some(id) = at {
            let node = doc.get_node(id)?;
            if let Some(element) = node.element_data()
                && let Some(handler) = element
                    .attrs()
                    .iter()
                    .find(|attribute| &*attribute.name.local == name)
                && !handler.value.is_empty()
            {
                return Some(handler.value.clone());
            }
            at = node.parent;
        }
        None
    }

    fn link_of(&self, node_id: NodeId) -> Option<String> {
        let doc = self.document.inner();
        let mut at = Some(node_id);
        while let Some(id) = at {
            let node = doc.get_node(id)?;
            if node
                .data
                .is_element_with_tag_name(&blitz_dom::local_name!("a"))
                && let Some(href) = node.attr(blitz_dom::local_name!("href"))
                && !href.is_empty()
                && !href.starts_with('#')
            {
                return Some(href.to_owned());
            }
            at = node.parent;
        }
        None
    }

    fn key_target(&mut self) -> NodeId {
        let doc = self.document.inner();
        let focussed = doc.get_focussed_node_id().filter(|id| {
            doc.get_node(*id).is_some_and(|node| {
                !node
                    .data
                    .is_element_with_tag_name(&blitz_dom::local_name!("html"))
            })
        });
        focussed
            .or_else(|| doc.query_selector("body").ok().flatten())
            .unwrap_or_else(|| doc.root_node().id)
    }

    fn probe_key_code(&mut self) {
        if self.keyed {
            return;
        }
        // 調べるためのイベントは文書そのものへ送る（途中で `stopPropagation` するアプリがある）。
        let target = self.document.inner().root_node().id;
        let event = BlitzKeyEvent {
            key: Key::Unidentified,
            code: Code::Unidentified,
            modifiers: Modifiers::empty(),
            location: Location::Standard,
            is_auto_repeating: false,
            is_composing: false,
            state: KeyState::Released,
            text: None,
        };
        self.document
            .dispatch_dom_event(DomEvent::new(target, DomEventData::KeyUp(event)));
        self.keyed = self.document.with_context(crate::api::dom::define_key_code);
    }

    /// 放送の字幕の MFU を `addCaptionListener` のリスナーへ（TR-B39 第 2 編 8.10.1.1、図 8-23）。
    pub fn caption(&mut self, stream: &crate::api::Caption, mfu: &[u8]) {
        let Some(unit) = isdb::mhcaption::mfu(mfu) else {
            return;
        };
        let text = caption_json(stream, &unit);
        let tag = stream.tag;
        self.document
            .with_context(|context| crate::api::dom::caption(context, tag, &text));
    }

    pub fn event_message(&mut self, tag: u8, message_id: u8, version: u8, private: &[u8]) -> usize {
        let private = String::from_utf8_lossy(private).into_owned();
        self.document.with_context(|context| {
            crate::api::receiver::event_message(context, tag, message_id, version, &private)
        })
    }

    pub fn watched_emts(&mut self) -> Vec<u8> {
        self.document
            .with_context(crate::api::receiver::watched_emts)
    }

    pub fn eval(&mut self, code: &str) {
        self.document.eval(code);
    }

    pub fn store(&self) -> &Shared {
        &self.store
    }

    pub fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(
            (zwsdk::plug::host::monotonic_ms() - self.began_ms).max(0.0) / 1000.0,
        )
    }

    pub fn video_rect(&mut self) -> Option<VideoRect> {
        crate::paint::video_rect(&mut self.document.inner_mut())
    }

    pub fn set_scale(&mut self, scale: f64) {
        if scale <= 0.0 || (scale - self.scale).abs() < 1e-6 {
            return;
        }
        self.scale = scale;
        self.document.inner_mut().set_viewport(Viewport::new(
            (self.size.0 as f64 * scale).floor() as u32,
            (self.size.1 as f64 * scale).floor() as u32,
            scale as f32,
            ColorScheme::Light,
        ));
    }

    pub fn paint(&mut self) -> (Vec<u8>, u32, u32) {
        crate::paint::paint(&mut self.document.inner_mut(), self.size, self.scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_of(html: &str, size: (u32, u32)) -> App {
        let dir = std::env::temp_dir().join(format!("zw-web-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp");
        std::fs::write(dir.join("index.html"), html).expect("write");
        let store = crate::store::Store::from_directory(&dir).expect("読める");
        let _ = std::fs::remove_dir_all(&dir);
        let faces = crate::fonts::Faces::find();
        App::open(
            Shared::new(store),
            "index.html",
            Host::default(),
            size,
            1.0,
            &faces,
        )
        .expect("開ける")
    }

    fn pixel(buffer: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * width + x) * 4) as usize;
        buffer[at..at + 4].try_into().expect("四つ")
    }

    #[test]
    fn a_canvas_paints_what_the_script_drew() {
        let html = r##"<html><body style="margin:0;background:#fff">
<canvas id="c" width="100" height="100" style="width:100px;height:100px"></canvas>
<script>
var c = document.getElementById("c").getContext("2d");
c.fillStyle = "#ff0000";
c.fillRect(0, 0, 50, 100);
var off = document.createElement("canvas");
off.width = 10; off.height = 10;
var o = off.getContext("2d");
o.fillStyle = "rgb(0, 0, 255)";
o.fillRect(0, 0, 10, 10);
c.drawImage(off, 60, 10, 30, 30);
c.fillStyle = "#000";
c.font = "bold 20px sans-serif";
c.fillText("あ", 55, 90);
</script></body></html>"##;
        if crate::fonts::Faces::find().sans.is_none() {
            zwsdk::plug::info("フォントが無いので飛ばす");
            return;
        }
        let mut app = app_of(html, (100, 100));
        app.tick();
        let (buffer, width, height) = app.paint();
        assert_eq!(pixel(&buffer, width, 25, 50), [255, 0, 0, 255], "fillRect");
        assert_eq!(pixel(&buffer, width, 75, 25), [0, 0, 255, 255], "drawImage");
        let dark = (60..100)
            .flat_map(|y| (55..100).map(move |x| (x, y)))
            .filter(|&(x, y)| pixel(&buffer, width, x, y)[..3].iter().all(|&c| c < 64))
            .count();
        assert!(dark > 10, "fillText の文字が無い（{height}）");
    }

    #[test]
    fn the_viewport_width_resizes_the_document_not_the_screen() {
        let html = r#"<html><head><meta charset="UTF-8"><meta name="viewport" content="width=1920"></head>"#;
        assert_eq!(viewport_width(html), Some(1920));
        assert_eq!(
            viewport_width("<meta content=\"width=1920,height=1080\" name=viewport>"),
            Some(1920)
        );
        assert_eq!(fit(html, (3840, 2160), 0.5), ((1920, 1080), 1.0));
        assert_eq!(fit("<html>", (3840, 2160), 0.5), ((3840, 2160), 0.5));
    }

    #[test]
    fn a_caption_unit_becomes_the_listeners_json() {
        let stream = crate::api::Caption {
            tag: 0x30,
            language: String::from("jpn"),
            tmd: 0b0010,
            resolution: 1,
            reference_start: Some(0x0000_0001_8000_0000),
            ..Default::default()
        };
        let mut bytes = vec![0x30, 0x07, 0x00, 0x00, 0x00, 0x00, 0x0a];
        bytes.extend_from_slice(b"<tt a=\"1\">\n</tt>");
        let unit = isdb::mhcaption::mfu(&bytes).expect("読める");
        let json = caption_json(&stream, &unit);
        assert_eq!(
            json,
            concat!(
                "{\"ISO_639_language_code\":\"jpn\",\"TMD\":\"0010\",\"resolution\":\"0001\",",
                "\"reference_start_time_seconds\":1,\"reference_start_time_fraction\":2147483648,",
                "\"subtitle_sequence_number\":7,\"subsample_number\":0,\"last_subsample_number\":0,",
                "\"data_type\":\"0000\",\"data\":\"<tt a=\\\"1\\\">\\n</tt>\"}"
            )
        );
        let bytes = [0x30, 0x07, 0x01, 0x01, 0x10, 0x00, 0x02, 0xff, 0xfe];
        let unit = isdb::mhcaption::mfu(&bytes).expect("読める");
        assert!(caption_json(&stream, &unit).ends_with("\"data_type\":\"0001\",\"data\":\"\"}"));
    }
}
