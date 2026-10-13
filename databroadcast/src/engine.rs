// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! データ放送の中身。本体のメッセージ（[`zwsdk::layer::Message`]）を受け、画像とリクエストを返す。
//! BML（`bml::Browser`）と HTML5（`web::app::App`）を一つにまとめる。

use std::collections::BTreeMap;
use std::sync::Arc;

use mpegts::dsmcc::{Brought, Carousel};
use zwsdk::layer::{
    ApplicationService, Broadcast, CaptionStream, Config, DataStream, Event, Frame, Key, Message,
    Rect, Ticked,
};

/// C プロファイルの `data_component_id`（ARIB STD-B24 / TR-B14）。
const CODING_C_PROFILE: u16 = 0x000d;

/// 文書を `area` に収める倍率。画面の画素に合わせ、丸めない（丸めると映像を透かす領域の縁に線が出る）。
fn scale_for(area: (u32, u32), size: (u32, u32)) -> f64 {
    if size.0 == 0 || size.1 == 0 {
        return 1.0;
    }
    let fit = (f64::from(area.0) / f64::from(size.0)).min(f64::from(area.1) / f64::from(size.1));
    fit.clamp(0.125, 4.0)
}

/// 通信・MMT のアプリをレンダリングする倍率。1920x1080 に収まる大きさ（か等倍）まで。
fn app_scale(area: (u32, u32), size: (u32, u32)) -> f64 {
    let most = web::still::MOST;
    let cap = if size.0 == 0 || size.1 == 0 {
        1.0
    } else {
        (f64::from(most.0) / f64::from(size.0))
            .min(f64::from(most.1) / f64::from(size.1))
            .max(1.0)
    };
    scale_for(area, size).min(cap)
}

const PAINT_EVERY_MS: f64 = 200.0;
/// キーを受けてからこの間の BML の変化は間引かない。
const KEY_HURRY_MS: f64 = 500.0;

fn now_ms() -> f64 {
    zwsdk::plug::host::monotonic_ms()
}

struct Mmt {
    store: web::net::Shared,
    entry: Option<String>,
    app: Option<web::app::App>,
    seen: u64,
    /// ストアが最後に変わった時。テーブルが揃わなくても、しばらく変わらなければ開く。
    changed_ms: f64,
    dirty: bool,
    urgent: bool,
    painted_ms: f64,
    faces: web::fonts::Faces,
    captions: Vec<CaptionStream>,
    tried_ms: Option<f64>,
    /// BML から起動した Hybridcast か。UA と映像オブジェクトの型が違う。
    hybridcast: bool,
    /// アプリの最後の画像が全面透明か（起動したまま何も出していない）。
    blank: bool,
    application: Option<ApplicationService>,
    emt_seen: BTreeMap<(u8, u8), u8>,
    waited: Option<String>,
}

pub struct Engine {
    browser: bml::Browser,
    zipcode: String,
    cas_id: String,
    streams: Vec<DataStream>,
    carousels: BTreeMap<u16, Carousel>,
    pngs: BTreeMap<String, (u64, Vec<u8>)>,
    bml_markup: Option<String>,
    area: (u32, u32),
    popup: Option<(u32, u32)>,
    bml_scale: f64,
    bml_keys: Vec<String>,
    bml_painted_ms: f64,
    bml_key_ms: f64,
    began_ms: f64,
    mmt: Mmt,
    broadcast: Broadcast,
    caption_shown: u8,
    events: Vec<Event>,
    audios: Vec<Vec<u8>>,
    frame: Option<(Frame, Vec<u8>)>,
    frame_changed: bool,
    said_ip: Option<bool>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_struct("Engine")
            .field("streams", &self.streams)
            .field("area", &self.area)
            .finish_non_exhaustive()
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(&Config::default())
    }
}

fn cas_id() -> String {
    zwsdk::plug::host::cfg("card_id")
        .and_then(|text| card_id_hex(&text))
        .unwrap_or_default()
}

/// 20 桁の番号を十六進 12 桁に（`getIRDID(5)` の形）。十進のままだと文書で `NaN` になる。
fn card_id_hex(text: &str) -> Option<String> {
    let text = text.trim();
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    if digits.len() == 20
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || " -_".contains(c))
    {
        let value: u64 = digits[..15].parse().ok()?;
        if value >> 48 != 0 {
            return None;
        }
        return Some(format!("{value:012X}"));
    }
    let hex = text.trim_start_matches("0x").trim_start_matches("0X");
    if (1..=12).contains(&hex.len()) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(format!("{:0>12}", hex.to_ascii_uppercase()));
    }
    None
}

fn zipcode() -> String {
    zwsdk::plug::host::cfg("receiver.zipcode")
        .filter(|zipcode| zipcode.len() == 7 && zipcode.bytes().all(|b| b.is_ascii_digit()))
        .unwrap_or_else(|| String::from("0000000"))
}

impl Engine {
    pub fn new(config: &Config) -> Self {
        let now = now_ms();
        if zwsdk::plug::host::on("trace") && log::set_logger(&JS_CONSOLE).is_ok() {
            log::set_max_level(log::LevelFilter::Debug);
        }
        web::fonts::set_bml(web::fonts::choose::load());
        let mut faces = web::fonts::Faces::find();
        if let Some(mono) = web::fonts::choose::mono() {
            faces.mono = Some(mono);
        }
        let mut browser = bml::Browser::new();
        // NVRAM は本体が残す（値が残っていないと一分待ってから書く起動文書がある）。
        browser.set_nvram(config.nvram.iter().cloned().collect());
        browser.set_zipcode(&zipcode());
        browser.set_cas_id(&cas_id());
        let web_open = zwsdk::plug::host::cfg("web_open");
        if let Some(pairs) = zwsdk::plug::host::cfg("web_storage") {
            web::api::kept(|kept| {
                for pair in pairs.split(';') {
                    if let Some((key, value)) = pair.split_once('=') {
                        kept.storage.insert(key.to_owned(), value.to_owned());
                    }
                }
            });
        }
        Self {
            zipcode: zipcode(),
            cas_id: cas_id(),
            browser,
            streams: Vec::new(),
            carousels: BTreeMap::new(),
            pngs: BTreeMap::new(),
            bml_markup: None,
            area: (1920, 1080),
            popup: None,
            bml_scale: 0.0,
            bml_keys: Vec::new(),
            bml_painted_ms: f64::NEG_INFINITY,
            bml_key_ms: f64::NEG_INFINITY,
            began_ms: now,
            mmt: Mmt {
                store: web::net::Shared::new(web::store::Store::new()),
                hybridcast: web_open.is_some(),
                blank: false,
                entry: web_open,
                app: None,
                seen: 0,
                changed_ms: now,
                dirty: false,
                urgent: false,
                painted_ms: now,
                faces,
                captions: Vec::new(),
                tried_ms: None,
                application: None,
                waited: None,
                emt_seen: BTreeMap::new(),
            },
            broadcast: Broadcast::default(),
            caption_shown: 0,
            events: Vec::new(),
            audios: Vec::new(),
            frame: None,
            frame_changed: false,
            said_ip: None,
        }
    }

    fn log(&mut self, text: impl Into<String>) {
        self.events.push(Event::Log(text.into()));
    }

    /// HTML5 の経路の記録の接頭辞。BML から起動した Hybridcast と、MMT/TLV の文書を分ける。
    fn web(&self) -> &'static str {
        if self.mmt.hybridcast {
            "Hybridcast"
        } else {
            "MMT"
        }
    }

    pub fn message(&mut self, message: Message, bytes: &[u8]) {
        if zwsdk::plug::host::on("trace")
            && !matches!(message, Message::Section { .. } | Message::Item { .. })
        {
            zwsdk::plug::info(&format!(
                "メッセージ {message:?} アプリ {}",
                self.mmt.app.is_some()
            ));
        }
        match message {
            Message::Streams(streams) => self.streams(streams),
            Message::Application(application) => self.application(application),
            Message::Section { pid } => self.section(pid, bytes),
            Message::Item {
                pid,
                mpu,
                item,
                fragmentation,
            } => {
                let mut store = self.mmt.store.0.write().unwrap_or_else(|e| e.into_inner());
                store.item(pid, mpu, item, fragmentation, bytes);
            }
            Message::CaptionStreams(streams) => {
                if self.mmt.captions != streams {
                    self.mmt.captions = streams;
                    let host = self.host();
                    if let Some(app) = &mut self.mmt.app {
                        app.set_host(host);
                    }
                }
            }
            Message::Caption { pid } => {
                let stream = self.mmt.captions.iter().find(|c| c.pid == pid).cloned();
                if let (Some(stream), Some(app)) = (stream, &mut self.mmt.app) {
                    app.caption(&caption_of(&stream), bytes);
                    self.mmt.dirty = true;
                }
            }
            Message::Key(key) => self.key(key),
            Message::Clock { unix, recorded } => {
                self.browser.set_clock_as_now(recorded);
                self.browser.set_clock(unix);
                web::api::date::set_clock(unix);
            }
            Message::Area { width, height } => {
                if self.area != (width, height) && width > 0 && height > 0 {
                    self.area = (width, height);
                    if let Some(app) = &mut self.mmt.app {
                        app.set_scale(app_scale(self.area, app.size));
                        self.mmt.dirty = true;
                    }
                }
            }
            Message::Broadcast(broadcast) => {
                self.broadcast = broadcast.clone();
                self.browser
                    .set_broadcast(bml_broadcast(&broadcast, self.caption_shown));
                let host = self.host();
                if let Some(app) = &mut self.mmt.app {
                    app.set_host(host);
                }
            }
            Message::CaptionShown(shown) => {
                self.caption_shown = shown;
                self.browser.set_caption_shown(shown);
            }
            Message::Popup(area) => {
                if self.popup.is_some() && area.is_none() && self.c_profile() {
                    self.browser.set_wanted(false);
                    self.bml_markup = None;
                }
                self.popup = area;
            }
            Message::Sought => {
                // HTML5 のアプリは開き直す。覚えたバージョンが残ると、シークした先でイベントが発生しない。
                if self.mmt.app.take().is_some() {
                    let web = self.web();
                    self.log(format!("{web}: 録画の位置が動いたので開き直す"));
                    self.mmt.dirty = true;
                }
                self.mmt.emt_seen.clear();
                self.mmt.waited = None;
            }
            Message::Retuned => {
                // 選局したら閉じて前の局のリソースを全部捨てる。次は d（か自動起動）まで出さない。
                self.browser.retune();
                self.broadcast = Default::default();
                web::api::date::clear_clock();
                self.streams.clear();
                self.carousels.clear();
                self.pngs.clear();
                self.bml_markup = None;
                self.mmt.app = None;
                self.mmt.entry = None;
                self.mmt.hybridcast = false;
                self.mmt.store = web::net::Shared::new(web::store::Store::new());
                self.mmt.seen = 0;
                self.mmt.application = None;
                self.mmt.emt_seen.clear();
                self.mmt.captions.clear();
                self.frame = None;
                self.frame_changed = true;
            }
        }
        self.say();
    }

    fn key(&mut self, key: Key) {
        if let Some(app) = &mut self.mmt.app {
            // `keySet` に無いグループのキーはアプリへ渡さず、受像機が扱う。
            let code = vk_of(key);
            let accepted = app.accepts_key(code);
            if accepted {
                app.key(code);
                self.mmt.dirty = true;
            }
            if zwsdk::plug::host::on("trace") {
                self.events
                    .push(Event::Log(format!("キー {code} をアプリへ: {accepted}")));
            }
            self.mmt.urgent = true;
            // d は Hybridcast を終える。ただしアプリが d を受け取るなら渡すだけ。
            if key == Key::DataButton && self.mmt.hybridcast && !accepted {
                self.quit_app(None);
            }
        } else {
            self.bml_key_ms = now_ms();
            self.browser.key(bml_key(key));
        }
    }

    fn streams(&mut self, streams: Vec<DataStream>) {
        if streams == self.streams {
            return;
        }
        let entry_was = self.streams.iter().find(|s| s.entry).map(|s| s.tag);
        let entry_now = streams.iter().find(|s| s.entry).map(|s| s.tag);
        // 自動起動（PMT の auto_start_flag）なら d を待たずに出す。C プロファイルは d を待つ。
        let c_profile = streams
            .iter()
            .any(|s| s.entry && s.coding == CODING_C_PROFILE);
        let auto_start = streams.iter().any(|s| s.entry && s.auto_start) && !c_profile;
        self.streams = streams;
        // 文字のデコードの仕方から違うので、文書を読む前に。
        self.browser.set_profile(if c_profile {
            bml::runtime::profile::Profile::C
        } else {
            bml::runtime::profile::Profile::A
        });
        self.carousels
            .retain(|pid, _| self.streams.iter().any(|s| s.pid == *pid));
        self.browser
            .set_components(self.streams.iter().map(|s| s.tag).collect());
        if entry_was != entry_now {
            self.browser.close();
            self.pngs.clear();
            self.bml_markup = None;
            if let Some(tag) = entry_now {
                self.browser.set_entry(tag, 0);
            }
        }
        if auto_start {
            self.browser.set_wanted(true);
        }
    }

    /// MPT のアプリケーションサービス記述子（MMT、TR-B39 11.1.1、11.1.2）。
    fn application(&mut self, application: Option<ApplicationService>) {
        if self.mmt.application == application {
            return;
        }
        self.mmt.application = application.clone();
        match application {
            Some(found) => self.log(format!(
                "MMT: MH-AIT は packet 0x{:04x}、ファイルは {}、文書は {}",
                found.ait,
                found
                    .data
                    .map_or(String::from("通信"), |id| format!("packet 0x{id:04x}")),
                match found.resolution {
                    0 => "1080",
                    1 => "2160",
                    2 => "4320",
                    _ => "?",
                }
            )),
            None => {
                if self.mmt.entry.is_some() || self.mmt.app.is_some() {
                    self.log("MMT: アプリケーションサービス記述子が消えたので閉じる");
                }
                self.mmt.entry = None;
                self.mmt.app = None;
                self.mmt.dirty = true;
            }
        }
    }

    fn c_profile(&self) -> bool {
        self.streams
            .iter()
            .any(|s| s.entry && s.coding == CODING_C_PROFILE)
    }

    fn area_for_page(&self) -> (u32, u32) {
        match self.popup {
            Some(popup) if self.c_profile() => popup,
            _ => self.area,
        }
    }

    fn section(&mut self, pid: u16, section: &[u8]) {
        match section.first() {
            Some(&isdb::mhdata::TABLE_DDMT) | Some(&isdb::mhdata::TABLE_DAMT) => {
                let mut store = self.mmt.store.0.write().unwrap_or_else(|e| e.into_inner());
                store.table(section);
                return;
            }
            Some(&isdb::mmttables::TABLE_MH_AIT) => {
                self.ait(section);
                return;
            }
            Some(&isdb::mmttables::TABLE_EMT) => {
                self.emt(pid, section);
                return;
            }
            _ => {}
        }
        let Some(stream) = self.streams.iter().find(|s| s.pid == pid) else {
            return;
        };
        let tag = stream.tag;
        let carousel = self.carousels.entry(pid).or_default();
        match carousel.push(section) {
            Some(Brought::Listed(dii)) => {
                let modules = dii.modules.iter().map(|m| (m.id, m.version)).collect();
                self.browser.listed(tag, modules);
                // データイベントは downloadId の上 4 ビット（STD-B24 第三分冊）。
                self.browser.data_event(tag, (dii.download_id >> 28) as u8);
            }
            Some(Brought::Module(module)) => {
                let described = isdb::bxml::describe(&module.info);
                let Some(data) = isdb::bxml::unpack(&described, &module.data) else {
                    return;
                };
                let files = isdb::bxml::files(described.content_type.as_deref(), &data);
                self.browser.arrived(tag, module.id, module.version, files);
                // 画像やテーブルは後から揃うので、同じスナップショットでも再レンダリングする。
                self.bml_markup = None;
            }
            Some(Brought::Stream(section)) => {
                // 汎用イベントメッセージ。運用される time_mode は即時と NPT だけ（TR-B14 3-2-46）。
                let (data_event, group, events) = isdb::bxml::events(&section);
                for event in events {
                    match event {
                        isdb::bxml::Event::General {
                            id,
                            time,
                            private_data,
                            ..
                        } => {
                            let npt = match time {
                                isdb::bxml::Timing::Immediate => None,
                                isdb::bxml::Timing::Npt(npt) => Some(npt),
                                _ => continue,
                            };
                            self.browser.event_message(
                                tag,
                                data_event,
                                group,
                                id,
                                npt,
                                &private_data,
                            );
                        }
                        isdb::bxml::Event::NptReference { npt, .. } => {
                            self.browser.npt_reference(npt);
                        }
                    }
                }
            }
            None => {}
        }
    }

    /// MH-AIT: 自動起動のアプリのエントリーポイント（ARIB STD-B60 10.3.3.2、10.3.3.3）。
    fn ait(&mut self, section: &[u8]) {
        let Some(ait) = isdb::mmttables::application_information(section) else {
            return;
        };
        if ait.application_type != 0x0011 {
            return;
        }
        let Some(app) = ait.applications.iter().find(|app| app.control_code == 0x01) else {
            return;
        };
        let base = app
            .descriptors
            .iter()
            .find(|d| d.tag == 0x802a)
            .and_then(|d| transport_base(&d.body));
        let location = app
            .descriptors
            .iter()
            .find(|d| d.tag == 0x802b)
            .map(|d| String::from_utf8_lossy(&d.body).into_owned());
        let (Some(base), Some(location)) = (base, location) else {
            return;
        };
        let entry = format!("{base}{location}");
        // 視聴者が終えたアプリでも、AIT が AUTOSTART なら起動し直す。
        if self.mmt.entry.as_deref() != Some(&entry) {
            self.log(format!("MMT のエントリーポイント {entry}"));
            self.mmt.entry = Some(entry);
            self.mmt.hybridcast = false;
            self.mmt.app = None;
        }
    }

    /// EMT：番組連動のイベント（ARIB STD-B60 11.2、TR-B39 6.2.4）。即時のものをアプリのリスナーへ。
    fn emt(&mut self, pid: u16, section: &[u8]) {
        let Some(tag) = self.mmt.application.as_ref().and_then(|application| {
            application
                .emts
                .iter()
                .find(|(_, id)| *id == pid)
                .map(|(tag, _)| *tag)
        }) else {
            return;
        };
        // リスナーの付いている EMT だけ見る（TR-B39 11.2.4）。
        let Some(app) = &mut self.mmt.app else {
            return;
        };
        if !app.watched_emts().contains(&tag) {
            return;
        }
        let Some(table) = isdb::mmttables::event_message_table(section) else {
            return;
        };
        // NPT の参照（グループ 0xfff）は運用されていない（TR-B39 6.2.4.1）。
        if table.group_id != 0x001 {
            if self
                .mmt
                .emt_seen
                .insert((tag, 0xff), table.version)
                .is_none()
            {
                let group = table.group_id;
                self.log(format!(
                    "MMT: EMT 0x{tag:02x} はグループ 0x{group:03x}（汎用イベントメッセージではない）"
                ));
            }
            return;
        }
        let mut lines = Vec::new();
        for found in &table.descriptors {
            if found.tag != isdb::mmttables::DESCRIPTOR_EVENT_MESSAGE {
                continue;
            }
            let Some(message) = isdb::mmttables::event_message(&found.body) else {
                continue;
            };
            // STD-B62：`event_msg_id` の上 8 ビットが id、下 8 ビットがバージョン。
            let (id, version) = (message.message_id(), message.message_version());
            if self.mmt.emt_seen.insert((tag, id), version) != Some(version) {
                lines.push(format!(
                    "MMT: イベントメッセージ EMT 0x{tag:02x} id {id} バージョン {version} 時 {} 「{}」",
                    match message.time_mode {
                        0 => String::from("即時"),
                        2 => format!("NPT {}", message.time),
                        other => format!("mode {other}"),
                    },
                    String::from_utf8_lossy(&message.private)
                ));
            }
            if !message.immediate() {
                continue;
            }
            app.event_message(tag, id, version, &message.private);
        }
        for line in lines {
            self.log(line);
        }
    }

    fn host(&self) -> web::api::Host {
        let b = &self.broadcast;
        web::api::Host {
            network: b.network,
            tlv_stream: b.transport,
            service: b.service,
            event: Some(b.event),
            start: b.start,
            duration: b.duration,
            name: b.name.clone(),
            description: String::new(),
            zipcode: zipcode(),
            cas_id: self.cas_id.clone(),
            profile: String::from(if self.mmt.hybridcast { "hc" } else { "sh4" }),
            captions: self.mmt.captions.iter().map(caption_of).collect(),
            next: b.next.clone(),
        }
    }

    pub fn tick(&mut self) -> Ticked {
        let zipcode = zipcode();
        if zipcode != self.zipcode {
            self.browser.set_zipcode(&zipcode);
            self.zipcode = zipcode;
        }
        let cas_id = cas_id();
        if cas_id != self.cas_id {
            self.browser.set_cas_id(&cas_id);
            self.cas_id = cas_id;
        }
        if self.said_ip.is_none() {
            // ネットワークに出られるかを一度伝える。偽だと通信のアプリを出さない局がある。
            let has = bml::resource::net::has_ip();
            self.said_ip = Some(has);
            self.log(format!(
                "ネットワーク接続: {}",
                if has { "あり" } else { "なし" }
            ));
        }
        let now = now_ms() - self.began_ms;
        self.browser.tick(now);
        self.tick_mmt();
        self.say();
        let nvram = self
            .browser
            .nvram_if_changed()
            .map(|written| written.into_iter().collect());
        let dirty = self.compose();
        Ticked { dirty, nvram }
    }

    fn tick_mmt(&mut self) {
        let generation = self
            .mmt
            .store
            .0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .generation;
        let changed_now = generation != self.mmt.seen;
        self.mmt.seen = generation;
        if changed_now {
            self.mmt.changed_ms = now_ms();
            if zwsdk::plug::host::on("trace") {
                let store = self.mmt.store.0.read().unwrap_or_else(|e| e.into_inner());
                let (n, complete) = (store.len(), store.is_complete());
                drop(store);
                let web = self.web();
                self.log(format!("{web}: ファイル {n} 個、揃った {complete}"));
            }
        }
        if self.mmt.app.is_none()
            && let Some(entry) = self.mmt.entry.clone()
        {
            let (has_entry, complete) = {
                let store = self.mmt.store.0.read().unwrap_or_else(|e| e.into_inner());
                (store.get(&entry).is_some(), store.is_complete())
            };
            let quiet = now_ms() - self.mmt.changed_ms > 3000.0;
            // 通信のエントリーポイントは放送のファイルを待たない。開けなければ次は三十秒後。
            let web = entry.starts_with("http://") || entry.starts_with("https://");
            let ready = if web {
                self.mmt.tried_ms.is_none_or(|t| now_ms() - t > 30_000.0)
            } else {
                has_entry && (complete || quiet)
            };
            if !ready && !web {
                let key = format!("{entry}/{has_entry}/{complete}");
                if self.mmt.waited.as_deref() != Some(&key) {
                    let web = self.web();
                    self.log(format!(
                        "{web}: {entry} を待つ（エントリー {}、揃った {}）",
                        if has_entry { "ある" } else { "まだ" },
                        complete
                    ));
                    self.mmt.waited = Some(key);
                }
            }
            if ready && web {
                self.mmt.tried_ms = Some(now_ms());
            }
            if ready {
                let host = self.host();
                match web::app::App::open(
                    self.mmt.store.clone(),
                    &entry,
                    host,
                    (3840, 2160),
                    scale_for(self.area, (3840, 2160)),
                    &self.mmt.faces,
                ) {
                    Some(mut app) => {
                        self.log(format!("開く {entry}"));
                        app.set_scale(app_scale(self.area, app.size));
                        if let Some(code) = zwsdk::plug::host::cfg("web_eval") {
                            app.eval(&code);
                        }
                        self.mmt.app = Some(app);
                        self.mmt.blank = true;
                        self.mmt.dirty = true;
                        self.bml_markup = None;
                    }
                    None => {
                        self.log(format!("開けない {entry}"));
                        if self.mmt.hybridcast {
                            self.quit_app(None);
                        }
                    }
                }
            }
        }
        let Some(app) = &mut self.mmt.app else {
            return;
        };
        if app.tick() || changed_now {
            self.mmt.dirty = true;
        }
        let requests = app.take_requests();
        let mut quit = None;
        for request in requests {
            match request {
                web::api::Request::Log(text) => self.events.push(Event::Log(text)),
                web::api::Request::Sound(number) => self.rom_sound(number),
                web::api::Request::Audio(bytes) => {
                    self.events.push(Event::Audio);
                    self.audios.push(bytes);
                }
                web::api::Request::AudioStop => self.events.push(Event::AudioStop),
                web::api::Request::Tune {
                    network,
                    tlv_stream,
                    service,
                } => self.events.push(Event::Tune {
                    network,
                    transport: tlv_stream,
                    service,
                }),
                web::api::Request::Quit { component } => {
                    quit = Some(component);
                    break;
                }
                web::api::Request::Captions(on) => {
                    self.events.push(Event::CaptionDisplay { language: 1, on });
                }
                web::api::Request::Navigate(url) => {
                    self.events.push(Event::Log(format!("移る {url}")));
                    if let Some(app) = &mut self.mmt.app {
                        if app.navigate(&url) {
                            app.set_scale(app_scale(self.area, app.size));
                        } else {
                            self.events.push(Event::Log(format!("移れない {url}")));
                        }
                    }
                    self.mmt.dirty = true;
                    self.mmt.urgent = true;
                }
            }
        }
        if let Some(component) = quit {
            self.quit_app(component);
        }
    }

    fn say(&mut self) {
        for request in self.browser.take_requests() {
            match request {
                bml::Request::Log(text) => self.events.push(Event::Log(text)),
                bml::Request::Sound(number) => self.rom_sound(number),
                bml::Request::Audio(bytes) => {
                    self.events.push(Event::Audio);
                    self.audios.push(bytes);
                }
                bml::Request::AudioStop => self.events.push(Event::AudioStop),
                bml::Request::CaptionDisplay { language, on } => {
                    self.events.push(Event::CaptionDisplay { language, on });
                }
                bml::Request::Tune {
                    network,
                    transport,
                    service,
                } => self.events.push(Event::Tune {
                    network,
                    transport,
                    service,
                }),
                bml::Request::LaunchApp {
                    org_id,
                    app_id,
                    url,
                } => {
                    self.events
                        .push(Event::Log(format!("通信のアプリ {org_id}/{app_id} {url}")));
                    self.mmt.entry = Some(url);
                    self.mmt.hybridcast = true;
                    self.mmt.app = None;
                    self.mmt.tried_ms = None;
                    self.mmt.dirty = true;
                    // `Ureg`/`Greg` を通信のアプリへ渡す（局は Greg[58]〜[61] に戻り先を書いてから起動する）。
                    let (ureg, greg) = self.browser.registers();
                    self.events.push(Event::Log(format!(
                        "通信のアプリへ渡す Greg[58..62] {:?}",
                        greg.get(58..62).unwrap_or_default()
                    )));
                    web::api::kept(|kept| {
                        for (prefix, values) in [("ureg", &ureg), ("greg", &greg)] {
                            for (index, value) in values.iter().enumerate() {
                                kept.storage
                                    .insert(format!("{prefix}{index}"), value.clone());
                            }
                        }
                    });
                    // データ放送は終えてから通信のアプリを起動する（戻るときは `quit_app` が起動し直す）。
                    self.browser.set_wanted(false);
                    self.bml_markup = None;
                }
            }
        }
    }

    fn quit_app(&mut self, component: Option<u16>) {
        self.log("アプリが終わった");
        self.mmt.app = None;
        // 通信のエントリーポイントは終わったら開き直さない。放送のエントリーポイントは残す（次の `d` で開く）。
        if self
            .mmt
            .entry
            .as_deref()
            .is_some_and(|e| e.starts_with("http"))
        {
            self.mmt.entry = None;
            if self.mmt.hybridcast {
                self.mmt.hybridcast = false;
                // 通信のアプリが書いた `Ureg`/`Greg` を戻す（最初の文書がそれを読んで元の画面へ移る）。
                let (ureg, greg) = web::api::kept(|kept| {
                    let take = |prefix: &str| -> Vec<String> {
                        (0..64)
                            .map(|i| {
                                kept.storage
                                    .get(&format!("{prefix}{i}"))
                                    .cloned()
                                    .unwrap_or_default()
                            })
                            .collect()
                    };
                    (take("ureg"), take("greg"))
                });
                self.browser.set_registers(ureg, greg);
                let tag = component.unwrap_or(0x40);
                if self.streams.iter().any(|s| s.tag == tag) {
                    self.browser.set_entry(tag, 0);
                }
                self.browser.set_wanted(true);
                let open = self.browser.is_open();
                self.log(format!(
                    "戻る: コンポーネント {} 開いている {open}",
                    component.map_or(String::from("-"), |t| format!("{t:02x}"))
                ));
            }
        }
        self.mmt.dirty = true;
        // 次のスナップショットは作り直す（でないと通信のアプリの画像が残る）。
        self.bml_markup = None;
    }

    fn compose(&mut self) -> bool {
        if let Some(app) = &mut self.mmt.app {
            // 画像が変わらなくても `keySet` が変われば送り直す（本体が数字を選局に使えるように）。
            let keys = app_keys(app.used_keys(), self.mmt.blank);
            if let Some((frame, _)) = &mut self.frame
                && frame.keys != keys
            {
                frame.keys = keys.clone();
                self.frame_changed = true;
            }
            let since = now_ms() - self.mmt.painted_ms;
            if !self.mmt.dirty && since < 1000.0 {
                return false;
            }
            // 3840x2160 は一枚 100〜250 ms かかるので、キーのとき以外は間を置く。
            if !self.mmt.urgent && since < PAINT_EVERY_MS {
                return false;
            }
            self.mmt.dirty = false;
            self.mmt.urgent = false;
            self.mmt.painted_ms = now_ms();
            let video = app.video_rect().map(|r| Rect {
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
            });
            let began = now_ms();
            let (rgba, width, height) = app.paint();
            let blank = rgba.chunks_exact(4).all(|pixel| pixel[3] == 0);
            if blank != self.mmt.blank {
                self.mmt.blank = blank;
                self.events.push(Event::Log(format!(
                    "アプリの画像が{}",
                    if blank { "空になった" } else { "出た" }
                )));
            }
            let keys = app_keys(app.used_keys(), blank);
            if zwsdk::plug::host::on("trace") || zwsdk::plug::host::on("times") {
                let took = now_ms() - began;
                self.events.push(Event::Log(format!(
                    "{} のレンダリングに {took:.0} ms（{width}x{height}）",
                    if self.mmt.hybridcast {
                        "Hybridcast"
                    } else {
                        "MMT"
                    }
                )));
            }
            self.frame = Some((
                Frame {
                    source: zwsdk::layer::Source::Rgba,
                    width,
                    height,
                    size: (app.size.0 as f32, app.size.1 as f32),
                    aspect: None,
                    invisible: false,
                    video,
                    c_profile: false,
                    waiting: false,
                    keys,
                },
                rgba,
            ));
            self.frame_changed = true;
            return true;
        }
        if !self.browser.is_open() {
            if self.frame.is_some() {
                self.frame = None;
                self.bml_markup = None;
                self.frame_changed = true;
                return true;
            }
            return false;
        }
        let invisible = self.browser.invisible();
        let waiting = self.browser.waiting();
        let Some(html) = self.browser.html() else {
            return false;
        };
        let scale = scale_for(self.area_for_page(), html.size);
        let keys = self.browser.used_keys();
        if self.bml_markup.as_deref() == Some(html.markup.as_str())
            && self.bml_scale == scale
            && self.bml_keys == keys
            && self
                .frame
                .as_ref()
                .is_some_and(|(f, _)| f.invisible == invisible && f.waiting == waiting)
        {
            return false;
        }
        // 変わり続けるときは PAINT_EVERY_MS に一度にまとめる。キーの後はすぐレンダリングする。
        let began = now_ms();
        if began - self.bml_painted_ms < PAINT_EVERY_MS && began - self.bml_key_ms > KEY_HURRY_MS {
            return false;
        }
        self.bml_painted_ms = began;
        self.bml_scale = scale;
        self.bml_keys = keys.clone();
        let mut files = BTreeMap::new();
        for name in &html.images {
            if let Some(made) = html.made.get(name) {
                files.insert(name.clone(), made.clone());
            } else if let Some(png) = self.png(name) {
                files.insert(name.clone(), png);
            }
        }
        let files_ms = now_ms() - began;
        let (rgba, width, height, video, took) = web::still::Still::paint_on_pixel_grid(
            &html.markup,
            html.size,
            scale,
            web::still::Files(Arc::new(files)),
            &self.mmt.faces,
        );
        if zwsdk::plug::host::on("trace") || zwsdk::plug::host::on("times") {
            let total = now_ms() - began;
            self.events.push(Event::Log(format!(
                "BML のレンダリングに {total:.0} ms（画像 {files_ms:.0}・開く {:.0}・レンダリング {:.0}・二度目 {:.0}・縮め {:.0}、{}x{} → {width}x{height}、倍率 {scale:.3}）",
                took.open, took.paint, took.second, took.shrink, took.drawn.0, took.drawn.1
            )));
        }
        let video = video.map(|r| Rect {
            x: r.x,
            y: r.y,
            w: r.w,
            h: r.h,
        });
        self.bml_markup = Some(html.markup);
        self.frame = Some((
            Frame {
                source: zwsdk::layer::Source::Rgba,
                width,
                height,
                size: (html.size.0 as f32, html.size.1 as f32),
                aspect: html.aspect.map(|(w, h)| w as f32 / h as f32),
                invisible,
                video,
                c_profile: self.c_profile(),
                waiting,
                keys,
            },
            rgba,
        ));
        self.frame_changed = true;
        true
    }

    pub fn take_frame(&mut self) -> Option<Option<(Frame, Vec<u8>)>> {
        if !self.frame_changed {
            return None;
        }
        self.frame_changed = false;
        Some(self.frame.clone())
    }

    fn rom_sound(&mut self, number: u32) {
        match crate::romsound::aiff(number) {
            Some(bytes) => {
                self.events.push(Event::Audio);
                self.audios.push(bytes);
            }
            None => self.events.push(Event::Sound(number)),
        }
    }

    pub fn take_events(&mut self) -> (Vec<Event>, Vec<Vec<u8>>) {
        (
            std::mem::take(&mut self.events),
            std::mem::take(&mut self.audios),
        )
    }

    /// ARIB の PNG は色を文書の色のテーブルで決めるので、テーブルごとに持つ。
    fn png(&mut self, src: &str) -> Option<Vec<u8>> {
        let file_name = src.split_once('#').map_or(src, |(name, _)| name);
        // 名前の無い画像はまだ無い画像（空の名前で参照するとモジュールそのものが返る）。
        if file_name.trim().is_empty() {
            return None;
        }
        let file = self.browser.file(file_name)?;
        let clut = self.browser.clut();
        let stamp = fnv(&file.body, clut_hash(&clut));
        if let Some((had, png)) = self.pngs.get(src)
            && *had == stamp
        {
            return Some(png.clone());
        }
        let decoded = web::picture::decode(&file.content_type, &file.body, src, &clut)?;
        let png = decoded.png();
        self.pngs.insert(src.to_owned(), (stamp, png.clone()));
        Some(png)
    }

    #[doc(hidden)]
    pub fn browser(&mut self) -> &mut bml::Browser {
        &mut self.browser
    }
}

/// MH 伝送プロトコル記述子の URL の基底（STD-B60 10.3.3.2）。
/// 本体へ伝えるアプリのキーの組（`used-key-list` の名前）。何も出していない間は d と色だけ取る
/// （矢印や数字は本体の選局・音量に残す）。出している間の矢印・決定・戻るは `keySet` に無くても
/// 取る。届いたキーは [`Engine::key`] が `keySet` で振り分け、無いグループのものは捨てる
/// （運用規定 1-3：設定の無いグループのキーはアプリへ伝えない）。
fn app_keys(used: Vec<String>, blank: bool) -> Vec<String> {
    if blank {
        return used
            .into_iter()
            .filter(|group| group == "data-button" || group == "color")
            .collect();
    }
    let mut keys = used;
    if !keys.iter().any(|group| group == "basic") {
        keys.push(String::from("basic"));
    }
    keys
}

fn transport_base(body: &[u8]) -> Option<String> {
    if body.len() < 4 {
        return None;
    }
    let protocol = u16::from_be_bytes([body[0], body[1]]);
    if protocol != 0x0005 && protocol != 0x0003 {
        return None;
    }
    let length = usize::from(body[3]);
    let base = body.get(4..4 + length)?;
    Some(String::from_utf8_lossy(base).into_owned())
}

fn clut_hash(clut: &bml::media::clut::Clut) -> u64 {
    let bytes: Vec<u8> = clut.0.iter().flatten().copied().collect();
    fnv(&bytes, 0xcbf2_9ce4_8422_2325)
}

fn fnv(bytes: &[u8], seed: u64) -> u64 {
    bytes.iter().fold(seed, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn caption_of(stream: &CaptionStream) -> web::api::Caption {
    web::api::Caption {
        tag: stream.tag,
        language: stream.language.clone(),
        kind: stream.kind,
        tmd: stream.tmd,
        resolution: stream.resolution,
        compression: stream.compression,
        reference_start: stream.reference_start,
    }
}

fn bml_broadcast(b: &Broadcast, caption_shown: u8) -> bml::Broadcast {
    bml::Broadcast {
        network: b.network,
        transport: b.transport,
        service: b.service,
        event: b.event,
        start: b.start,
        duration: b.duration,
        name: b.name.clone(),
        captions: b.captions,
        next: b.next.clone(),
        audio: b.audio,
        caption_shown,
    }
}

fn bml_key(key: Key) -> bml::Key {
    match key {
        Key::Up => bml::Key::Up,
        Key::Down => bml::Key::Down,
        Key::Left => bml::Key::Left,
        Key::Right => bml::Key::Right,
        Key::Digit(n) => bml::Key::Digit(n),
        Key::Enter => bml::Key::Enter,
        Key::Back => bml::Key::Back,
        Key::Blue => bml::Key::Blue,
        Key::Red => bml::Key::Red,
        Key::Green => bml::Key::Green,
        Key::Yellow => bml::Key::Yellow,
        Key::DataButton => bml::Key::DataButton,
    }
}

fn vk_of(key: Key) -> u16 {
    match key {
        Key::Up => 38,
        Key::Down => 40,
        Key::Left => 37,
        Key::Right => 39,
        Key::Enter => 13,
        Key::Back => 461,
        Key::Red => 403,
        Key::Green => 404,
        Key::Yellow => 405,
        Key::Blue => 406,
        Key::Digit(n) => 48 + u16::from(n.min(9)),
        Key::DataButton => 1001,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apps_are_drawn_no_larger_than_full_hd_unless_the_page_is() {
        assert_eq!(app_scale((3840, 2160), (1280, 720)), 1.5);
        assert_eq!(app_scale((1280, 720), (1280, 720)), 1.0);
        assert_eq!(app_scale((2880, 1620), (3840, 2160)), 0.75);
        assert_eq!(app_scale((3840, 2160), (3840, 2160)), 1.0);
    }

    #[test]
    fn a_card_number_is_handed_over_as_twelve_hex_digits() {
        assert_eq!(
            card_id_hex("0012-5099-9896-4915-2719").as_deref(),
            Some("0123456789AB")
        );
        assert_eq!(
            card_id_hex("00125099989649152719").as_deref(),
            Some("0123456789AB")
        );
        assert_eq!(
            card_id_hex("0x123456789ab").as_deref(),
            Some("0123456789AB")
        );
        assert_eq!(card_id_hex(""), None);
        assert_eq!(card_id_hex("カード"), None);
        assert_eq!(card_id_hex("9999-9999-9999-9999-9999"), None);
    }
}

struct JsConsole;

static JS_CONSOLE: JsConsole = JsConsole;

impl log::Log for JsConsole {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.target() == "js_console"
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            zwsdk::plug::info(&format!("console  {}", record.args()));
        }
    }

    fn flush(&self) {}
}
