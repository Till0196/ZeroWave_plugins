// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 放送のイベント：イベントメッセージ・NPT・データイベントと、時間の経過（タイマー・アニメーション・マーキー）。

use crate::document::dom;
use crate::document::style::Display;
use crate::media::text;

use super::*;

impl Browser {
    /// 汎用イベントメッセージが届いた。`npt` を指定すればその NPT になってから発生させる。
    /// `message_id` は `event_msg_id` の上 8 ビット、バージョンは下 8 ビット。
    pub fn event_message(
        &mut self,
        component: u16,
        data_event_id: u8,
        group_id: u16,
        event_msg_id: u16,
        npt: Option<u64>,
        private_data: &[u8],
    ) {
        if let Some(at) = npt
            && !self.npt_reached(at)
        {
            let host = self.host();
            let event = NptEvent {
                component,
                data_event_id,
                group_id,
                event_msg_id,
                npt: at,
                private_data: private_data.to_vec(),
            };
            if !host.npt_events.contains(&event) {
                host.npt_events.push(event);
            }
            return;
        }
        self.fire_event_message(
            component,
            data_event_id,
            group_id,
            event_msg_id,
            private_data,
        );
        self.finish();
    }

    fn npt_reached(&mut self, npt: u64) -> bool {
        let host = self.host();
        let Some((reference, at)) = host.npt else {
            return false;
        };
        let elapsed = ((host.now_ms - at) * 90.0) as i64;
        reference + elapsed >= npt as i64
    }

    fn fire_due_npt_events(&mut self) {
        loop {
            let due = {
                let host = self.host();
                let Some((reference, at)) = host.npt else {
                    return;
                };
                let now = reference + ((host.now_ms - at) * 90.0) as i64;
                let Some(index) = host
                    .npt_events
                    .iter()
                    .position(|event| now >= event.npt as i64)
                else {
                    return;
                };
                host.npt_events.remove(index)
            };
            self.fire_event_message(
                due.component,
                due.data_event_id,
                due.group_id,
                due.event_msg_id,
                &due.private_data,
            );
        }
    }

    fn fire_event_message(
        &mut self,
        component: u16,
        data_event_id: u8,
        group_id: u16,
        event_msg_id: u16,
        private_data: &[u8],
    ) {
        let message_id = (event_msg_id >> 8) as u8;
        let message_version = (event_msg_id & 0xff) as u8;
        let (base, page_component, current_data_event) = {
            let host = self.host();
            let base = host.page.as_ref().map_or(String::new(), |p| p.name.clone());
            let page_component = Resources::locate("", &base).map(|l| l.component);
            (
                base,
                page_component,
                host.data_events.get(&component).copied(),
            )
        };
        // グループ 0 は今のデータイベントのものだけ。まだ分からなければ通す。
        if group_id == 0 && current_data_event.is_some_and(|current| current != data_event_id) {
            return;
        }
        let wildcard = |value: Option<&str>| -> Option<u8> {
            match value.and_then(|v| v.trim().parse::<u32>().ok()) {
                None | Some(255) => None,
                Some(v) => Some(v as u8),
            }
        };
        let handlers = self.subscribed("EventMessageFired", |item| {
            let wanted_component = match item.attribute("es_ref") {
                Some(es_ref) => Resources::component_of(&base, es_ref),
                None => page_component,
            };
            wanted_component == Some(component)
                && item
                    .attribute("message_group_id")
                    .and_then(|group| group.trim().parse::<u16>().ok())
                    .unwrap_or(0)
                    == group_id
                && wildcard(item.attribute("message_id")).is_none_or(|id| id == message_id)
                && wildcard(item.attribute("message_version"))
                    .is_none_or(|version| version == message_version)
        });
        for (target, handler) in handlers {
            let (es_ref, already) = {
                let host = self.host();
                let es_ref = host
                    .page
                    .as_ref()
                    .and_then(|page| page.document.element(target))
                    .and_then(|e| e.attribute("es_ref").map(str::to_owned))
                    .unwrap_or_else(|| format!("/{component:02X}"));
                let already = host.fired.get(&(target, message_id)) == Some(&message_version);
                (es_ref, already)
            };
            if already {
                continue;
            }
            self.host()
                .fired
                .insert((target, message_id), message_version);
            let event = Event {
                kind: "EventMessageFired".into(),
                target: Some(target),
                status: Some(0),
                message_id: Some(u32::from(message_id)),
                message_version: Some(u32::from(message_version)),
                message_group_id: Some(u32::from(group_id)),
                private_data: Some(text::decode(private_data)),
                es_ref: Some(es_ref),
                ..Default::default()
            };
            if self.run_handler(&handler, event) {
                break;
            }
        }
    }

    pub fn tick(&mut self, now_ms: f64) {
        const TIMERS_PER_TICK: usize = 64;
        self.host().now_ms = now_ms;
        // 届く前に押された d を、開いた文書へ（待っている間は待つ）。
        if self.host().data_button_pending
            && self.is_open()
            && self.host().waiting.is_none()
            && self.host().launch.is_none()
        {
            self.host().data_button_pending = false;
            self.key_inner(Key::DataButton);
            self.finish();
        }
        // 一度の tick で鳴らすのは上限まで（`setTimeout(..., 0)` の積み直しでスレッドを取らない）。
        let mut fired = 0;
        loop {
            if fired >= TIMERS_PER_TICK {
                break;
            }
            fired += 1;
            let due = {
                let host = self.host();
                let Some(at) = host
                    .timers
                    .iter()
                    .position(|timer| !timer.paused && timer.next_at <= now_ms)
                else {
                    break;
                };
                let timer = &mut host.timers[at];
                timer.next_at += timer.interval_ms.max(1.0);
                if timer.next_at < now_ms - 1000.0 {
                    timer.next_at = now_ms;
                }
                let handler = timer.handler.clone();
                let id = timer.id;
                if let Some(remaining) = &mut timer.remaining {
                    *remaining = remaining.saturating_sub(1);
                    if *remaining == 0 {
                        host.timers.remove(at);
                    }
                }
                (id, handler)
            };
            let _ = due.0;
            let event = Event {
                kind: "TimerFired".into(),
                ..Default::default()
            };
            if self.run_handler(&due.1, event) {
                break;
            }
        }
        self.finish();
        self.fire_due_npt_events();
        self.finish();
        self.stop_media_if_over(now_ms);
        self.advance_marquees(now_ms);
        self.advance_animations(now_ms);
    }

    fn advance_animations(&mut self, now_ms: f64) {
        let host = self.host();
        let Some(page) = &mut host.page else {
            return;
        };
        let base = page.name.clone();
        let objects: Vec<(NodeId, String)> = page
            .document
            .elements()
            .filter_map(|id| {
                let e = page.document.element(id)?;
                if e.name != "object" && e.name != "img" {
                    return None;
                }
                let data = e.attribute("data").or_else(|| e.attribute("src"))?;
                Some((id, data.to_owned()))
            })
            .collect();
        let mut changed = false;
        let mut seen = std::collections::BTreeSet::new();
        for (id, data) in objects {
            let Some(file) = host.resources.get(&base, &data) else {
                continue;
            };
            let Some(info) = crate::media::mng::info(&file.body) else {
                continue;
            };
            seen.insert(id);
            let count = info.delays_ms.len();
            let entry = page
                .animations
                .entry(id)
                .or_insert_with(|| (0, now_ms + f64::from(info.delays_ms[0])));
            if now_ms < entry.1 {
                continue;
            }
            let next = entry.0 + 1;
            if next >= count && !info.repeats {
                entry.1 = f64::INFINITY;
                continue;
            }
            entry.0 = next % count;
            entry.1 = now_ms + f64::from(info.delays_ms[entry.0]);
            changed = true;
        }
        page.animations.retain(|id, _| seen.contains(id));
        if changed {
            page.dirty = true;
        }
    }

    fn advance_marquees(&mut self, now_ms: f64) {
        let dirty = self.host().page.as_ref().is_some_and(|page| page.dirty);
        if dirty {
            self.restyle();
        }
        let host = self.host();
        let Some(page) = &mut host.page else {
            return;
        };
        let mut changed = false;
        let ids: Vec<NodeId> = page
            .document
            .elements()
            .filter(|id| {
                page.styles
                    .get(id.0)
                    .is_some_and(|s| s.marquee.is_some() && s.display == Display::Block)
            })
            .collect();
        for id in ids {
            let style = &page.styles[id.0];
            let Some(marquee) = style.marquee else {
                continue;
            };
            let text_width = crate::document::html::text_width(&page.document, &page.styles, id);
            let inner =
                (style.width - 2.0 * style.border_width - style.padding[1] - style.padding[3])
                    .max(0.0);
            let state = page.marquees.entry(id).or_insert_with(|| {
                // `-wap-marquee-loop: 0` は流し終えた状態のまま（TR-B14）。
                let finished = marquee.loops == Some(0);
                MarqueeState {
                    next_at: now_ms + f64::from(marquee.interval_ms),
                    done: finished,
                    offset: if finished {
                        match marquee.style {
                            crate::document::style::MarqueeStyle::Slide => inner,
                            _ => inner + text_width,
                        }
                    } else {
                        0.0
                    },
                    ..Default::default()
                }
            });
            if state.done || now_ms < state.next_at {
                continue;
            }
            state.next_at = now_ms + f64::from(marquee.interval_ms);
            state.offset += style.font_size / 2.0;
            match marquee.style {
                crate::document::style::MarqueeStyle::Slide => {
                    if state.offset >= inner {
                        state.offset = inner;
                        state.done = true;
                    }
                }
                _ => {
                    if state.offset >= inner + text_width {
                        state.loops += 1;
                        if marquee.loops.is_some_and(|n| state.loops >= n) {
                            state.done = true;
                        } else {
                            state.offset = 0.0;
                        }
                    }
                }
            }
            changed = true;
        }
        if changed {
            page.dirty = true;
        }
    }

    pub(super) fn subscribed(
        &mut self,
        kind: &str,
        accept: impl Fn(&dom::Element) -> bool,
    ) -> Vec<(NodeId, String)> {
        let host = self.host();
        let Some(page) = &host.page else {
            return Vec::new();
        };
        page.document
            .elements()
            .filter_map(|id| {
                let element = page.document.element(id)?;
                if element.name != "beitem"
                    || element.attribute("type") != Some(kind)
                    || element.attribute("subscribe") != Some("subscribe")
                    || !accept(element)
                {
                    return None;
                }
                Some((id, element.attribute("onoccur")?.to_owned()))
            })
            .collect()
    }

    pub fn npt_reference(&mut self, npt: i64) {
        let now = self.host().now_ms;
        let first = self.host().npt.is_none();
        self.host().npt = Some((npt, now));
        self.fire_due_npt_events();
        self.finish();
        if !first {
            return;
        }
        let handlers = self.subscribed("NPTReferred", |_| true);
        for (target, handler) in handlers {
            let event = Event {
                kind: "NPTReferred".into(),
                target: Some(target),
                status: Some(0),
                ..Default::default()
            };
            if self.run_handler(&handler, event) {
                break;
            }
        }
        self.finish();
    }

    fn stop_media_if_over(&mut self, now_ms: f64) {
        let Some((at, target)) = self.host().media_ends else {
            return;
        };
        if now_ms < at {
            return;
        }
        self.host().media_ends = None;
        let handlers = self.subscribed("MediaStopped", |_| true);
        for (target_item, handler) in handlers {
            let event = Event {
                kind: "MediaStopped".into(),
                target: Some(target_item),
                status: Some(0),
                module_ref: None,
                ..Default::default()
            };
            let _ = target;
            if self.run_handler(&handler, event) {
                break;
            }
        }
        self.finish();
    }

    pub fn data_event(&mut self, component: u16, id: u8) {
        let was = self.host().data_events.insert(component, id);
        if was.is_none_or(|was| was == id) {
            return;
        }
        // ロックはそのコンポーネントのデータイベントが変わったら外れる（lockModuleOnMemoryEx）。
        self.host().locked.retain(|lock| lock.key.0 != component);
        let handlers = self.subscribed("DataEventChanged", |_| true);
        for (target, handler) in handlers {
            let event = Event {
                kind: "DataEventChanged".into(),
                target: Some(target),
                status: Some(0),
                ..Default::default()
            };
            if self.run_handler(&handler, event) {
                break;
            }
        }
        self.finish();
    }
}
