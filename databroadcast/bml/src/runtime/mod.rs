// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 文書を開き、スクリプトを実行し、キーと時刻と放送のイベントを渡す。
//! `browser.launchDocument` のようにその場で文書を終える関数は、スクリプトを例外で抜けて外で開く。

mod access;
pub(crate) mod bindings;
pub mod clock;
mod events;
mod keys;
mod load;
pub mod profile;
mod receive;
pub mod script;

use std::collections::BTreeMap;

use boa_engine::{Context, JsError, JsNativeError};

use crate::document::css::{Declaration, Rule};
use crate::document::dom::{Document, Kind, NodeId};
use crate::document::layout::Laid;
use crate::document::style::Style;
use crate::media::clut::Clut;
use crate::resource::Resources;
use crate::resource::table;

/// リモコンのキー。STD-B24 第二分冊 (2/2) 付属 2 表 5-13 の番号。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Digit(u8),
    Enter,
    Back,
    Blue,
    Red,
    Green,
    Yellow,
    DataButton,
}

impl Key {
    pub fn code(self) -> u32 {
        match self {
            Self::Up => 1,
            Self::Down => 2,
            Self::Left => 3,
            Self::Right => 4,
            Self::Digit(n) => 5 + u32::from(n.min(9)),
            Self::Enter => 18,
            Self::Back => 19,
            Self::Blue => 21,
            Self::Red => 22,
            Self::Green => 23,
            Self::Yellow => 24,
            Self::DataButton => 20,
        }
    }

    fn group(self) -> &'static str {
        match self {
            Self::Digit(_) => "numeric-tuning",
            Self::DataButton => "data-button",
            _ => "basic",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Tune {
        network: u16,
        transport: u16,
        service: u16,
    },
    Sound(u32),
    Audio(Vec<u8>),
    AudioStop,
    CaptionDisplay {
        language: u32,
        on: bool,
    },
    LaunchApp {
        org_id: u32,
        app_id: u32,
        url: String,
    },
    Log(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Broadcast {
    pub network: u16,
    pub transport: u16,
    pub service: u16,
    pub event: u16,
    pub start: Option<i64>,
    pub duration: Option<u32>,
    pub name: String,
    pub captions: bool,
    /// 出ている字幕の言語（0 無し、1 第 1、2 第 2）。変わると `CCStatusChanged`。
    pub caption_shown: u8,
    pub next: Option<(u16, Option<i64>, Option<u32>, String)>,
    pub audio: Option<u16>,
}

#[derive(Debug, Clone)]
pub(crate) struct Timer {
    id: u32,
    handler: String,
    interval_ms: f64,
    next_at: f64,
    remaining: Option<u32>,
    paused: bool,
}

#[derive(Debug)]
pub(crate) struct Page {
    pub name: String,
    pub document: Document,
    pub sheets: Vec<Rule>,
    pub overrides: BTreeMap<NodeId, Vec<Declaration>>,
    pub focus_overrides: BTreeMap<NodeId, Vec<Declaration>>,
    pub active_overrides: BTreeMap<NodeId, Vec<Declaration>>,
    pub marquees: BTreeMap<NodeId, MarqueeState>,
    pub animations: BTreeMap<NodeId, (usize, f64)>,
    pub drcs: Vec<crate::media::drcs::Glyph>,
    pub styles: Vec<Style>,
    pub clut: Clut,
    pub laid: Laid,
    pub dirty: bool,
    pub focus: Option<NodeId>,
    pub input_value_at_focus: Option<String>,
    /// 文書が参照する画像の画素の大きさ。まだ届いていない画像は入れない（届いたら読む）。
    pub sizes: BTreeMap<String, (u32, u32)>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MarqueeState {
    pub offset: f32,
    pub loops: u32,
    pub next_at: f64,
    pub done: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Lock {
    pub key: (u16, u16),
    pub ex: bool,
}

#[derive(Debug)]
pub(crate) struct Host {
    /// 実行したスクリプトの本文（boa はエラーの位置を返さないので、エラーの箇所を切り出すため）。
    pub sources: Vec<String>,
    pub resources: Resources,
    pub page: Option<Page>,
    pub broadcast: Broadcast,
    pub now_ms: f64,
    pub timers: Vec<Timer>,
    pub next_timer: u32,
    pub ureg: Vec<String>,
    pub greg: Vec<String>,
    pub nvram: BTreeMap<String, Vec<u8>>,
    /// 受像機の郵便番号（七桁）。空なら「決めていない」（`0000000`）。
    pub zipcode: String,
    /// 受像機の識別子（48 ビットを十六進 12 桁）。`getIRDID` が返す。空なら null。
    pub cas_id: String,
    pub nvram_changed: bool,
    pub tables: Vec<table::Table>,
    pub requests: Vec<Request>,
    pub launch: Option<String>,
    pub fragment: Option<String>,
    /// ロックしたモジュール。`lockModuleOnMemory` は別のコンポーネントの文書へ移ると外れ、`Ex` は残る。
    pub locked: Vec<Lock>,
    pub event: Option<Event>,
    pub entry: Option<(u16, u16)>,
    pub wanted: bool,
    /// 文書が届く前に押された d。文書が開いたら一度だけ渡す（捨てると d を待つ文書が見えない）。
    pub data_button_pending: bool,
    pub started: bool,
    pub date_mode: u32,
    pub ait_url: Option<String>,
    pub root_certificates: Vec<(u32, u32, u32)>,
    pub clock: std::rc::Rc<crate::runtime::clock::BmlClock>,
    pub stream_offset_ms: Option<i64>,
    pub clock_as_now: bool,
    /// スクリプトが移したフォーカス。`onfocus` が走るので handler の外で効かせる。
    pub pending_focus: Option<Option<NodeId>>,
    pub pending_submit: Option<NodeId>,
    pub quit: bool,
    pub pending_subscribe: Vec<NodeId>,
    pub pending_locked: Vec<(String, i32)>,
    pub waiting: Option<String>,
    pub waiting_module: Option<(u16, u16)>,
    pub seed: u64,
    pub watched: BTreeMap<NodeId, (bool, Option<u8>)>,
    pub data_events: BTreeMap<u16, u8>,
    pub media_ends: Option<(f64, Option<NodeId>)>,
    pub npt: Option<(i64, f64)>,
    /// `beitem` と message_id ごとの最後に発生させたバージョン（TR-B14 3-2-46）。
    pub fired: BTreeMap<(NodeId, u8), u8>,
    pub npt_events: Vec<NptEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NptEvent {
    pub component: u16,
    pub data_event_id: u8,
    pub group_id: u16,
    pub event_msg_id: u16,
    pub npt: u64,
    pub private_data: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Event {
    pub kind: String,
    pub target: Option<NodeId>,
    pub key_code: Option<u32>,
    pub status: Option<i32>,
    pub message_id: Option<u32>,
    pub message_version: Option<u32>,
    pub message_group_id: Option<u32>,
    pub private_data: Option<String>,
    pub module_ref: Option<String>,
    pub es_ref: Option<String>,
}

pub struct Browser {
    context: Context,
}

impl std::fmt::Debug for Browser {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str("Browser")
    }
}

pub(crate) const LEAVING: &str = "__bml_leaving__";

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

enum Missing {
    NotYet(String),
    Unreadable(String),
}

pub(crate) fn is_leaving(error: &JsError) -> bool {
    error.to_string().contains(LEAVING)
}

pub(crate) fn leaving() -> JsError {
    JsNativeError::error().with_message(LEAVING).into()
}

impl Host {
    pub fn is_locked(&self, component: u16, module: u16) -> bool {
        self.locked
            .iter()
            .any(|lock| lock.key == (component, module))
    }

    /// モジュールが一覧にあってまだ届いていなければ待ちにして真。一覧に無ければ待たない。
    pub fn wait_for_module(&mut self, name: &str) -> bool {
        let base = self.page.as_ref().map_or(String::new(), |p| p.name.clone());
        let Some(located) = Resources::locate(&base, name) else {
            return false;
        };
        let (component, module) = (located.component, located.module);
        if self.resources.listed_version(component, module).is_none()
            || self.resources.module(component, module).is_some()
        {
            return false;
        }
        let Some(page) = self.page.as_ref() else {
            return false;
        };
        self.requests.push(Request::Log(format!(
            "待つ {name}（{} を開き直す）",
            page.name
        )));
        self.waiting = Some(page.name.clone());
        self.waiting_module = Some((component, module));
        true
    }

    pub(crate) fn apply_date_mode(&self) {
        let offset = match self.date_mode {
            1 => self.stream_offset_ms.unwrap_or(0),
            _ if self.clock_as_now => self.stream_offset_ms.unwrap_or(0),
            _ => 0,
        };
        self.clock.set_offset_ms(offset);
    }

    pub(crate) fn set_timer(
        &mut self,
        handler: String,
        interval_ms: f64,
        times: Option<u32>,
    ) -> u32 {
        // 消さずに作り続ける文書で伸び続けないよう、同時に持てる数に上限（0 は作れなかった）。
        if self.timers.len() >= 1024 {
            return 0;
        }
        let id = self.next_timer;
        self.next_timer += 1;
        self.timers.push(Timer {
            id,
            handler,
            interval_ms,
            next_at: self.now_ms + interval_ms,
            remaining: times,
            paused: false,
        });
        id
    }

    pub(crate) fn clear_timer(&mut self, id: u32) -> bool {
        let before = self.timers.len();
        self.timers.retain(|timer| timer.id != id);
        before != self.timers.len()
    }

    pub(crate) fn pause_timer(&mut self, id: u32, paused: bool) -> bool {
        if let Some(timer) = self.timers.iter_mut().find(|timer| timer.id == id) {
            timer.paused = paused;
            true
        } else {
            false
        }
    }

    pub(crate) fn set_text(&mut self, id: NodeId, text: &str) {
        if let Some(page) = &mut self.page {
            match &mut page.document.node_mut(id).kind {
                Kind::Text(held) => *held = text.to_owned(),
                Kind::Element(_) => page.document.set_text_of(id, text),
            }
            page.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod event_message_tests;

#[cfg(test)]
mod cdata_sibling_tests;
