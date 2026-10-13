// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 文書を開いて実行する：読み込み、スクリプト、実行したあとの片付け、レイアウトのやり直し。

use boa_engine::Source;

use crate::document::css;
use crate::document::dom;
use crate::document::layout;
use crate::document::style;
use crate::media::text;

use super::keys::{c_profile_focusables, find_nav_index, focusable, fragment_target};
use super::*;

impl Browser {
    pub fn launch(&mut self, name: &str) {
        let mut name = name.to_owned();
        for _ in 0..8 {
            match self.load(&name) {
                Ok(()) => {}
                Err(Missing::NotYet(path)) => {
                    // 届くまで待つ。
                    self.host()
                        .requests
                        .push(Request::Log(format!("待つ {path}")));
                    self.host().waiting = Some(name);
                    return;
                }
                Err(Missing::Unreadable(message)) => {
                    self.host().requests.push(Request::Log(message));
                    return;
                }
            }
            let Some(next) = self.host().launch.take() else {
                return;
            };
            name = next;
        }
    }

    fn load(&mut self, name: &str) -> Result<(), Missing> {
        let (base, bytes) = {
            let host = self.host();
            let base = host
                .page
                .as_ref()
                .map_or(String::new(), |page| page.name.clone());
            let located = Resources::locate(&base, name)
                .ok_or_else(|| Missing::Unreadable(format!("読めない名前: {name}")))?;
            let file = host
                .resources
                .get(&base, name)
                .ok_or_else(|| Missing::NotYet(located.path()))?;
            (located.path(), file.body.clone())
        };
        {
            let source = text::decode(&bytes);
            let document = dom::parse(&source);
            let wanted: Vec<String> = document
                .elements()
                .filter_map(|id| document.element(id))
                .filter_map(|element| match element.name.as_str() {
                    "script" => element.attribute("src"),
                    "link" => element.attribute("href"),
                    _ => None,
                })
                .map(str::to_owned)
                .collect();
            for src in wanted {
                if self.host().resources.get(&base, &src).is_none() {
                    let path = Resources::locate(&base, &src).map_or(src, |located| located.path());
                    return Err(Missing::NotYet(path));
                }
            }
        }
        self.host().waiting = None;
        // 前の文書の `onunload` は次の文書が読めると決まってから、前の DOM ツリーがある間に。要求された遷移は無視。
        let onunload = self.host().page.as_ref().and_then(|page| {
            let body = page.document.first_named("body")?;
            page.document
                .element(body)?
                .attribute("onunload")
                .map(str::to_owned)
        });
        if let Some(onunload) = onunload {
            let target = self
                .host()
                .page
                .as_ref()
                .and_then(|p| p.document.first_named("body"));
            self.host().event = Some(Event {
                kind: "unload".into(),
                target,
                ..Default::default()
            });
            self.run_script(&onunload);
            self.host().event = None;
            self.host().launch = None;
        }
        let source = text::decode(&bytes);
        let document = dom::parse(&source);
        let mut sheets = Vec::new();
        let mut scripts: Vec<String> = Vec::new();
        for id in document.elements() {
            let element = document.element(id).unwrap();
            match element.name.as_str() {
                "style" => sheets.extend(css::parse_stylesheet(&document.text_of(id))),
                "link" => {
                    if let Some(href) = element.attribute("href")
                        && let Some(file) = self.host().resources.get(&base, href)
                    {
                        let css = text::decode(&file.body);
                        sheets.extend(css::parse_stylesheet(&css));
                    }
                }
                "script" => {
                    if let Some(src) = element.attribute("src") {
                        match self.host().resources.get(&base, src) {
                            Some(file) => scripts.push(text::decode(&file.body)),
                            None => self
                                .host()
                                .requests
                                .push(Request::Log(format!("script が無い: {src}"))),
                        }
                    } else {
                        scripts.push(document.text_of(id));
                    }
                }
                _ => {}
            }
        }
        {
            let host = self.host();
            host.sources.clear();
            host.timers.clear();
            host.tables.clear();
            host.launch = None;
            let previous_component = host
                .page
                .as_ref()
                .and_then(|page| Resources::locate("", &page.name))
                .map(|located| located.component);
            let next_component = Resources::locate("", &base).map(|l| l.component);
            // `lockModuleOnMemory` のロックはコンポーネントを跨ぐと外れる。`Ex` は残る。
            if previous_component != next_component {
                host.locked.retain(|lock| lock.ex);
            }
            host.pending_focus = None;
            host.pending_submit = None;
            host.pending_subscribe.clear();
            host.pending_locked.clear();
            host.watched.clear();
            host.fired.clear();
            host.page = Some(Page {
                name: base.clone(),
                document,
                sheets,
                overrides: BTreeMap::new(),
                focus_overrides: BTreeMap::new(),
                active_overrides: BTreeMap::new(),
                marquees: BTreeMap::new(),
                animations: BTreeMap::new(),
                drcs: Vec::new(),
                styles: Vec::new(),
                clut: Clut::default(),
                laid: Laid::default(),
                dirty: true,
                focus: None,
                input_value_at_focus: None,
                sizes: BTreeMap::new(),
            });
            host.requests.push(Request::Log(format!("開く {base}")));
        }
        self.restyle();
        {
            let clut_name = self.host().page.as_ref().and_then(|page| {
                page.document
                    .first_named("body")
                    .and_then(|body| page.styles[body.0].clut.clone())
            });
            if let Some(clut_name) = clut_name
                && let Some(file) = self.host().resources.get(&base, &clut_name)
                && let Some(clut) = Clut::parse(&file.body)
            {
                self.host().page.as_mut().unwrap().clut = clut;
            }
        }
        // フォーカスは nav-index 0 に（onfocus は走らない）。C プロファイルは文書の順の最初の要素（付属 4 5.1.6）。
        {
            let host = self.host();
            let page = host.page.as_mut().unwrap();
            page.focus = if crate::runtime::profile::is_c() {
                c_profile_focusables(page).into_iter().next()
            } else {
                find_nav_index(page, 0).filter(|id| focusable(page, *id))
            };
        }
        bindings::reset_globals(&mut self.context);
        for script in scripts {
            self.run_script(&script);
            if self.settle_after_script() {
                return Ok(());
            }
        }
        self.play_audio_objects();
        let onload = self.host().page.as_ref().and_then(|page| {
            page.document.first_named("body").and_then(|body| {
                page.document
                    .element(body)
                    .and_then(|element| element.attribute("onload").map(str::to_owned))
            })
        });
        if let Some(onload) = onload {
            let target = self
                .host()
                .page
                .as_ref()
                .and_then(|p| p.document.first_named("body"));
            let event = Event {
                kind: "load".into(),
                target,
                ..Default::default()
            };
            if self.run_handler(&onload, event) {
                return Ok(());
            }
        }
        self.host().watched.clear();
        let watching: Vec<NodeId> = self
            .subscribed("ModuleUpdated", |_| true)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        self.watch(watching);
        Ok(())
    }

    fn watch(&mut self, items: Vec<NodeId>) -> bool {
        for id in items {
            let Some(handler) = self.host().page.as_ref().and_then(|page| {
                let element = page.document.element(id)?;
                if element.attribute("type") != Some("ModuleUpdated")
                    || element.attribute("subscribe") != Some("subscribe")
                {
                    return None;
                }
                element.attribute("onoccur").map(str::to_owned)
            }) else {
                continue;
            };
            let Some((located, listed, known)) = ({
                let host = self.host();
                let page = host.page.as_ref();
                let module_ref = page
                    .and_then(|page| page.document.element(id))
                    .and_then(|e| e.attribute("module_ref").map(str::to_owned));
                module_ref
                    .and_then(|r| Resources::locate(page.map_or("", |p| p.name.as_str()), &r))
                    .map(|l| {
                        let listed = host.resources.listed_version(l.component, l.module);
                        let known = host.resources.has_component(l.component);
                        (l, listed, known)
                    })
            }) else {
                continue;
            };
            if !known {
                continue;
            }
            self.host().watched.insert(id, (listed.is_some(), listed));
            let event = Event {
                kind: "ModuleUpdated".into(),
                target: Some(id),
                status: Some(if listed.is_some() { 2 } else { 1 }),
                module_ref: Some(located.path()),
                ..Default::default()
            };
            if self.run_handler(&handler, event) {
                return true;
            }
        }
        false
    }

    pub(super) fn run_script(&mut self, source: &str) -> bool {
        let source = crate::runtime::script::rewrite_declaring(source);
        if source.len() > 200 {
            // エラーの箇所を探すための記録。ページを開き直すまでたまるので、新しい物だけを残す。
            let sources = &mut self.host().sources;
            if sources.len() >= 64 {
                sources.remove(0);
            }
            sources.push(source.clone());
        }
        match self.context.eval(Source::from_bytes(&source)) {
            Ok(_) => {}
            Err(error) => {
                if !is_leaving(&error) {
                    let message = error.to_string();
                    // エラーの箇所の前後を添える（関数の名前だけではどの `browser.*` が null を返したか分からない）。
                    let host = self.host();
                    let context = failing_context(&message, &host.sources);
                    host.requests
                        .push(Request::Log(format!("スクリプト: {message}{context}")));
                }
            }
        }
        self.host().launch.is_some()
    }

    pub(super) fn run_handler(&mut self, handler: &str, event: Event) -> bool {
        self.host().event = Some(event);
        let leaving = self.run_script(handler);
        self.host().event = None;
        self.settle_after_script() || leaving
    }

    pub(super) fn settle_after_script(&mut self) -> bool {
        if self.host().quit {
            self.host().quit = false;
            // 文書が自分で終わった（`quitDocument`）。d で開き直すまで出さない。
            self.host().wanted = false;
            self.close();
            return true;
        }
        if self.host().launch.is_some() {
            return true;
        }
        if let Some(wanted) = self.host().pending_focus.take()
            && self.focus(wanted)
        {
            return true;
        }
        if let Some(form) = self.host().pending_submit.take() {
            let handler = self.host().page.as_ref().and_then(|page| {
                page.document
                    .element(form)
                    .filter(|e| e.name == "form")
                    .and_then(|e| e.attribute("onsubmit").map(str::to_owned))
            });
            if let Some(handler) = handler {
                let event = Event {
                    kind: "submit".into(),
                    target: Some(form),
                    ..Default::default()
                };
                if self.run_handler(&handler, event) {
                    return true;
                }
            }
        }
        if let Some(fragment) = self.host().fragment.take() {
            let target = {
                let host = self.host();
                host.page
                    .as_ref()
                    .and_then(|page| fragment_target(page, &fragment))
            };
            if let Some(target) = target
                && self.focus(Some(target))
            {
                return true;
            }
        }
        let pending = std::mem::take(&mut self.host().pending_subscribe);
        if !pending.is_empty() && self.watch(pending) {
            return true;
        }
        let locked = std::mem::take(&mut self.host().pending_locked);
        for (name, status) in locked {
            if self.module_locked(&name, status) {
                return true;
            }
        }
        false
    }

    fn play_audio_objects(&mut self) {
        let host = self.host();
        let Some(page) = &host.page else {
            return;
        };
        let names: Vec<String> = page
            .document
            .elements()
            .filter_map(|id| page.document.element(id))
            .filter(|e| {
                e.name == "object"
                    && e.attribute("type")
                        .is_some_and(|t| t.eq_ignore_ascii_case("audio/X-arib-aiff"))
                    && e.attribute("streamstatus") == Some("play")
            })
            .filter_map(|e| e.attribute("data").map(str::to_owned))
            .collect();
        for name in names {
            crate::runtime::bindings::request_audio(host, &name);
        }
    }

    /// `ModuleLocked` を監視している `beitem` へ。`status` は 0 がロックできた、−1/−2 が一覧に無い。
    pub(super) fn module_locked(&mut self, name: &str, status: i32) -> bool {
        let base = self
            .host()
            .page
            .as_ref()
            .map_or(String::new(), |p| p.name.clone());
        let Some(located) = Resources::locate(&base, name) else {
            return false;
        };
        let handlers = self.subscribed("ModuleLocked", |item| {
            item.attribute("module_ref").is_some_and(|wanted| {
                Resources::locate(&base, wanted)
                    .is_some_and(|l| (l.component, l.module) == (located.component, located.module))
            })
        });
        for (target, handler) in handlers {
            let event = Event {
                kind: "ModuleLocked".into(),
                target: Some(target),
                status: Some(status),
                module_ref: Some(name.to_owned()),
                ..Default::default()
            };
            if self.run_handler(&handler, event) {
                return true;
            }
        }
        false
    }

    pub(super) fn restyle(&mut self) {
        let host = self.host();
        let Some(page) = &mut host.page else {
            return;
        };
        page.styles = style::compute_with(
            &page.document,
            &page.sheets,
            &style::Scripted {
                normal: Some(&page.overrides),
                focus: Some(&page.focus_overrides),
                focused: page.focus,
            },
        );
    }

    pub fn laid(&mut self) -> &Laid {
        let dirty = self.host().page.as_ref().is_some_and(|page| page.dirty);
        if dirty {
            self.restyle();
            let host = self.host();
            let page = host.page.as_mut().unwrap();
            page.laid = layout::lay_out(&page.document, &page.styles, &page.clut);
            page.dirty = false;
        }
        static EMPTY: std::sync::LazyLock<Laid> = std::sync::LazyLock::new(Laid::default);
        match &self.host().page {
            Some(page) => &page.laid,
            None => &EMPTY,
        }
    }
}

fn failing_context(message: &str, sources: &[String]) -> String {
    let place = message
        .lines()
        .filter(|line| line.trim_start().starts_with("at "))
        .find_map(|line| {
            let rest = line.rsplit_once(" at :")?.1;
            let mut parts = rest.split(':');
            let row: usize = parts.next()?.parse().ok()?;
            let column: usize = parts
                .next()?
                .split(|c: char| !c.is_ascii_digit())
                .next()?
                .parse()
                .ok()?;
            Some((row, column))
        });
    let Some((row, column)) = place else {
        return String::new();
    };
    let mut out = String::new();
    for source in sources
        .iter()
        .filter(|source| source.lines().count() >= row)
        .take(2)
    {
        let Some(line) = source.lines().nth(row - 1) else {
            continue;
        };
        let chars: Vec<char> = line.chars().collect();
        let at = column.saturating_sub(1).min(chars.len());
        let from = at.saturating_sub(120);
        let to = (at + 80).min(chars.len());
        let before: String = chars[from..at].iter().collect();
        let after: String = chars[at..to].iter().collect();
        out.push_str(&format!(
            "\n    エラーの箇所（{row} 行）: …{before}▶{after}…"
        ));
    }
    out
}
