// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 受け取り：受像機を作り、選局・サービスの識別子・時刻・モジュールの一覧と中身を受け取って最初の文書を開く。

use crate::resource::File;

use super::*;

impl Browser {
    pub fn new() -> Self {
        let clock = std::rc::Rc::new(crate::runtime::clock::BmlClock::new());
        let mut context = crate::runtime::clock::context(clock.clone());
        context.insert_data(Host {
            sources: Vec::new(),
            clock,
            stream_offset_ms: None,
            clock_as_now: false,
            root_certificates: Vec::new(),
            ait_url: None,
            resources: Resources::new(),
            page: None,
            broadcast: Broadcast::default(),
            now_ms: 0.0,
            timers: Vec::new(),
            next_timer: 1,
            ureg: vec![String::new(); 64],
            greg: vec![String::new(); 64],
            nvram: BTreeMap::new(),
            zipcode: String::new(),
            cas_id: String::new(),
            nvram_changed: false,
            tables: Vec::new(),
            requests: Vec::new(),
            launch: None,
            fragment: None,
            locked: Vec::new(),
            event: None,
            entry: None,
            wanted: false,
            data_button_pending: false,
            started: false,
            date_mode: 0,
            pending_focus: None,
            pending_submit: None,
            quit: false,
            pending_subscribe: Vec::new(),
            pending_locked: Vec::new(),
            waiting: None,
            waiting_module: None,
            seed: 0x9e37_79b9_7f4a_7c15,
            watched: BTreeMap::new(),
            data_events: BTreeMap::new(),
            media_ends: None,
            npt: None,
            fired: BTreeMap::new(),
            npt_events: Vec::new(),
        });
        bindings::install(&mut context);
        Self { context }
    }

    pub fn set_clock(&mut self, unix_seconds: i64) {
        let host = self.host();
        host.stream_offset_ms =
            Some(unix_seconds * 1000 - crate::runtime::clock::BmlClock::machine_millis());
        host.apply_date_mode();
    }

    /// 録画の再生で、録った時刻を「いま」として見せる（局は `Date` の差で録画を見分ける）。
    pub fn set_clock_as_now(&mut self, on: bool) {
        let host = self.host();
        host.clock_as_now = on;
        host.apply_date_mode();
    }

    pub(crate) fn host(&mut self) -> &mut Host {
        self.context
            .host_defined_mut()
            .get_mut::<Host>()
            .expect("the host is in the context")
    }

    pub fn set_broadcast(&mut self, broadcast: Broadcast) {
        let was = self.host().broadcast.audio;
        if was.is_some() && broadcast.audio.is_some() && was != broadcast.audio {
            self.host().broadcast.audio = broadcast.audio;
            let handlers = self.subscribed("MainAudioStreamChanged", |_| true);
            for (target, handler) in handlers {
                let event = Event {
                    kind: "MainAudioStreamChanged".into(),
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
        let shown = self.host().broadcast.caption_shown;
        self.host().broadcast = broadcast;
        self.host().broadcast.caption_shown = shown;
        self.try_start();
        self.finish();
    }

    pub fn set_caption_shown(&mut self, shown: u8) {
        if self.host().broadcast.caption_shown == shown {
            return;
        }
        self.host().broadcast.caption_shown = shown;
        let handlers = self.subscribed("CCStatusChanged", |_| true);
        for (target, handler) in handlers {
            let event = Event {
                kind: "CCStatusChanged".into(),
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

    pub fn set_entry(&mut self, component: u16, module: u16) {
        let host = self.host();
        host.entry = Some((component, module));
        host.started = false;
    }

    /// どのプロファイルの文書か。文字のデコードから変わるので、開いている文書は閉じる。
    pub fn set_profile(&mut self, profile: crate::runtime::profile::Profile) {
        if crate::runtime::profile::current() == profile {
            return;
        }
        crate::runtime::profile::set(profile);
        self.close();
    }

    pub fn set_components(&mut self, components: Vec<u16>) {
        self.host().resources.set_components(components);
    }

    pub fn listed(&mut self, component: u16, modules: Vec<(u16, u8)>) {
        self.host().resources.listed(component, modules.clone());
        // 一覧からエントリーコンポーネントが消えても文書は閉じず、`ModuleUpdated`（消失）を配って文書に決めさせる。
        if let Some((entry_component, entry_module)) = self.host().entry
            && entry_component == component
            && !self.host().started
            && self.host().wanted
        {
            let listed = modules
                .iter()
                .find(|(id, _)| *id == entry_module)
                .map(|(_, v)| *v);
            let held = self
                .host()
                .resources
                .module(component, entry_module)
                .map(|m| m.version);
            if listed.is_some() && listed == held {
                let path = self
                    .host()
                    .resources
                    .module(component, entry_module)
                    .and_then(|present| entry_document(component, entry_module, present))
                    .unwrap_or_else(|| format!("/{component:02X}/{entry_module:04X}/startup.bml"));
                self.host().started = true;
                self.launch(&path);
                return;
            }
        }
        let watching = self.subscribed("ModuleUpdated", |_| true);
        for (id, handler) in watching {
            let Some((located, was)) = ({
                let host = self.host();
                let page = host.page.as_ref();
                let module_ref = page
                    .and_then(|page| page.document.element(id))
                    .and_then(|e| e.attribute("module_ref").map(str::to_owned));
                let located = module_ref
                    .and_then(|r| Resources::locate(page.map_or("", |p| p.name.as_str()), &r));
                let was = host.watched.get(&id).copied();
                located.map(|l| (l, was))
            }) else {
                continue;
            };
            if located.component != component {
                continue;
            }
            let now = modules
                .iter()
                .find(|(m, _)| *m == located.module)
                .map(|(_, v)| *v);
            let status = match (was, now) {
                (Some((true, Some(old))), Some(new)) if old != new => Some(0),
                (Some((true, _)), None) => Some(1),
                (Some((false, _)), Some(_)) => Some(2),
                _ => None,
            };
            self.host().watched.insert(id, (now.is_some(), now));
            if let Some(status) = status {
                let event = Event {
                    kind: "ModuleUpdated".into(),
                    target: Some(id),
                    status: Some(status),
                    module_ref: Some(located.path()),
                    ..Default::default()
                };
                if self.run_handler(&handler, event) {
                    break;
                }
            }
        }
        self.finish();
    }

    pub fn arrived(&mut self, component: u16, module: u16, version: u8, files: Vec<File>) {
        // モジュール 0xFFFF の `type.id.version` の DER。
        for file in &files {
            if let Some(key) = certificate_name(&file.location)
                && file.body.starts_with(&[0x30, 0x82])
            {
                crate::resource::net::add_root_certificate(&file.body);
                let host = self.host();
                if !host.root_certificates.contains(&key) {
                    host.root_certificates.push(key);
                }
            }
        }
        let host = self.host();
        // 捨ててはいけないもの：エントリーコンポーネント、ロックしたもの、いまの文書のもの。
        let mut keep: Vec<(u16, u16)> = host.locked.iter().map(|lock| lock.key).collect();
        if let Some(entry) = host.entry {
            keep.push(entry);
        }
        if let Some(page) = &host.page
            && let Some(located) = Resources::locate("", &page.name)
        {
            keep.push((located.component, located.module));
        }
        if !host
            .resources
            .arrived(component, module, version, files, &keep)
        {
            return;
        }
        let entry = host.entry;
        if !host.started && host.wanted && entry == Some((component, module)) {
            self.try_start();
            return;
        }
        // 待っていたのがテーブルなら、届いたときに文書を開き直す（スクリプトの中では待てない）。
        if let Some(waiting) = self.host().waiting.clone() {
            let base = self
                .host()
                .page
                .as_ref()
                .map_or(String::new(), |p| p.name.clone());
            let wanted_module = self.host().waiting_module;
            if wanted_module == Some((component, module))
                || (wanted_module.is_none()
                    && Resources::locate(&base, &waiting)
                        .is_some_and(|l| (l.component, l.module) == (component, module)))
            {
                self.host().waiting = None;
                self.host().waiting_module = None;
                self.launch(&waiting);
                return;
            }
        }
        if self.host().is_locked(component, module) {
            let module_ref = format!("/{component:02X}/{module:04X}");
            self.module_locked(&module_ref, 0);
            self.finish();
        }
    }

    /// 最初の文書を開けるなら開く。識別子が 0 のまま開くと startup.bml が自局を見分けられない。
    pub(super) fn try_start(&mut self) {
        let host = self.host();
        if host.started || !host.wanted {
            return;
        }
        let Some((component, module)) = host.entry else {
            return;
        };
        if host.broadcast.network == 0 || host.broadcast.service == 0 {
            return;
        }
        let listed = host.resources.listed_version(component, module);
        let Some(present) = host.resources.module(component, module) else {
            return;
        };
        let Some(path) = entry_document(component, module, present) else {
            return;
        };
        if listed.is_none() || listed != Some(present.version) {
            return;
        }
        host.started = true;
        self.launch(&path);
    }

    /// スクリプトが要求した次の文書があれば開く。handler の中では開かない（再帰で呼び出しが深くなる）。
    pub(super) fn finish(&mut self) {
        for _ in 0..16 {
            let Some(next) = self.host().launch.take() else {
                return;
            };
            self.launch(&next);
        }
    }
}

/// `0.3.4` のようなファイル名 → (type, id, version)。
fn certificate_name(name: &str) -> Option<(u32, u32, u32)> {
    let mut parts = name.split('.');
    let kind = parts.next()?.parse().ok()?;
    let id = parts.next()?.parse().ok()?;
    let version = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((kind, id, version))
}

/// エントリーコンポーネントのモジュールで最初に開く文書（ふつうは `startup.bml`、BML 一本ならそれ）。
fn entry_document(
    component: u16,
    module: u16,
    present: &crate::resource::Module,
) -> Option<String> {
    if present
        .files
        .iter()
        .any(|f| f.location.eq_ignore_ascii_case("startup.bml"))
    {
        return Some(format!("/{component:02X}/{module:04X}/startup.bml"));
    }
    match present.files.as_slice() {
        [only]
            if only.location.is_empty()
                && only.content_type.to_ascii_lowercase().contains("bml") =>
        {
            Some(format!("/{component:02X}/{module:04X}"))
        }
        _ => None,
    }
}
