// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! キー操作とフォーカスの移動。

use crate::document::dom;
use crate::document::style::{Display, Visibility};

use super::*;

impl Browser {
    pub fn key(&mut self, key: Key) {
        if key == Key::DataButton {
            // d は起動していなければ起動し、起動していれば `DataButtonPressed` を渡すだけ（TR-B14 2.1.10.4）。
            // 文書がまだ届いていなければ覚えて開いたら渡す。
            if !self.is_open() {
                if self.host().wanted {
                    self.host().data_button_pending = true;
                } else {
                    self.set_wanted(true);
                }
            } else {
                self.key_inner(key);
            }
            self.finish();
            return;
        }
        // 待っている間の「戻る」は、待つのをやめていまの文書を開き直す。
        if key == Key::Back && self.host().waiting.is_some() {
            let host = self.host();
            host.waiting = None;
            host.waiting_module = None;
            host.launch = None;
            let name = host.page.as_ref().map(|page| page.name.clone());
            host.requests
                .push(Request::Log(String::from("待つのをやめた（戻る）")));
            if let Some(name) = name {
                self.launch(&name);
            }
            self.finish();
            return;
        }
        if !self.key_inner(key) {
            let handler = self.host().page.as_ref().and_then(|page| {
                let focus = page.focus?;
                let used = page
                    .document
                    .first_named("body")
                    .map(|body| page.styles[body.0].used_key_list.clone())
                    .unwrap_or_default();
                if used.first().is_some_and(|first| first == "none")
                    || !used.iter().any(|group| group == key.group())
                {
                    return None;
                }
                page.document
                    .element(focus)
                    .and_then(|e| e.attribute("onkeyup").map(|h| (focus, h.to_owned())))
            });
            if let Some((focus, handler)) = handler {
                let event = Event {
                    kind: "keyup".into(),
                    target: Some(focus),
                    key_code: Some(key.code()),
                    ..Default::default()
                };
                self.run_handler(&handler, event);
            }
        }
        self.finish();
    }

    pub(super) fn key_inner(&mut self, key: Key) -> bool {
        if key == Key::DataButton {
            let handlers = self.subscribed("DataButtonPressed", |_| true);
            for (target, handler) in handlers {
                let event = Event {
                    kind: "DataButtonPressed".into(),
                    target: Some(target),
                    ..Default::default()
                };
                if self.run_handler(&handler, event) {
                    return true;
                }
            }
            return false;
        }
        let (used, focus) = {
            let host = self.host();
            let Some(page) = &host.page else {
                return false;
            };
            let body = page.document.first_named("body");
            let used = body
                .map(|body| page.styles[body.0].used_key_list.clone())
                .unwrap_or_default();
            (used, page.focus)
        };
        if used.first().is_some_and(|first| first == "none") {
            return false;
        }
        if !used.iter().any(|group| group == key.group()) {
            return false;
        }
        if crate::runtime::profile::is_c() {
            // C プロファイルの上下左右は受像機のもの（文書の順にフォーカスを動かす）。色ボタンは C に無い。
            match key {
                Key::Up | Key::Left | Key::Down | Key::Right => {
                    if self.host().page.as_ref().is_some_and(|page| page.dirty) {
                        self.restyle();
                    }
                    let next = {
                        let host = self.host();
                        let page = host.page.as_ref().unwrap();
                        let order = c_profile_focusables(page);
                        match focus.and_then(|focus| order.iter().position(|id| *id == focus)) {
                            None => order.first().copied(),
                            Some(at) => match key {
                                Key::Up | Key::Left => at.checked_sub(1).map(|at| order[at]),
                                _ => order.get(at + 1).copied(),
                            },
                        }
                    };
                    if let Some(next) = next
                        && self.focus(Some(next))
                    {
                        return true;
                    }
                    return false;
                }
                Key::Blue | Key::Red | Key::Green | Key::Yellow => return false,
                _ => {}
            }
        }
        // `accesskey`（STD-B24 第二編 付属 2 5.4.4.6）：割り当てた要素があれば決定を押したことにする。
        // 順は keydown → フォーカスを移す → keyup → keydown（決定）→ click → keyup（決定）。
        let access = match key {
            Key::Blue => Some("B"),
            Key::Red => Some("R"),
            Key::Green => Some("G"),
            Key::Yellow => Some("Y"),
            Key::Digit(n) => {
                Some(["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"][usize::from(n.min(9))])
            }
            _ => None,
        };
        let access_target = access.and_then(|access| {
            let host = self.host();
            let page = host.page.as_ref()?;
            page.document.elements().find(|id| {
                page.document
                    .element(*id)
                    .and_then(|e| e.attribute("accesskey"))
                    .is_some_and(|a| a.eq_ignore_ascii_case(access))
                    && focusable(page, *id)
            })
        });
        if let Some(target) = access_target {
            if let Some(focus) = focus
                && self.fire_key(focus, "onkeydown", key.code())
            {
                return true;
            }
            if self.focus(Some(target)) {
                return true;
            }
            if self.fire_key(target, "onkeyup", key.code())
                || self.fire_key(target, "onkeydown", Key::Enter.code())
            {
                return true;
            }
            let onclick = self.host().page.as_ref().and_then(|page| {
                page.document
                    .element(target)
                    .and_then(|e| e.attribute("onclick").map(str::to_owned))
            });
            if let Some(onclick) = onclick {
                let event = Event {
                    kind: "click".into(),
                    target: Some(target),
                    ..Default::default()
                };
                if self.run_handler(&onclick, event) {
                    return true;
                }
            }
            self.fire_key(target, "onkeyup", Key::Enter.code());
            return true;
        }
        if let (Key::Digit(digit), Some(focus)) = (key, focus) {
            let host = self.host();
            if let Some(page) = &mut host.page
                && let Some(element) = page.document.element_mut(focus)
                && element.name == "input"
                // `readonly`・`disabled` の欄は書けない（表 5-9）。
                && element.attribute("readonly").is_none()
                && element.attribute("disabled").is_none()
            {
                // `indirect` の受像機の入力画面は無いので、`direct` と同じく数字を入れる。
                {
                    let most: usize = element
                        .attribute("maxlength")
                        .and_then(|max| max.parse().ok())
                        .unwrap_or(usize::MAX);
                    let mut value = element.attribute("value").unwrap_or("").to_owned();
                    if value.chars().count() < most {
                        value.push(char::from(b'0' + digit));
                        element.set_attribute("value", &value);
                        page.dirty = true;
                    }
                }
            }
        }
        if let Some(focus) = focus {
            let handler = self.host().page.as_ref().and_then(|page| {
                page.document
                    .element(focus)
                    .and_then(|element| element.attribute("onkeydown").map(str::to_owned))
            });
            if let Some(handler) = handler {
                let event = Event {
                    kind: "keydown".into(),
                    target: Some(focus),
                    key_code: Some(key.code()),
                    ..Default::default()
                };
                if self.run_handler(&handler, event) {
                    return true;
                }
            }
        }
        let direction = match key {
            Key::Up => Some(0),
            Key::Down => Some(1),
            Key::Left => Some(2),
            Key::Right => Some(3),
            _ => None,
        };
        if let (Some(direction), Some(focus)) =
            (direction, self.host().page.as_ref().and_then(|p| p.focus))
        {
            let next = {
                let host = self.host();
                let page = host.page.as_ref().unwrap();
                let mut from = focus;
                let mut found = None;
                for _ in 0..64 {
                    let Some(index) = page.styles[from.0].nav[direction] else {
                        break;
                    };
                    let Some(next) = find_nav_index(page, index) else {
                        break;
                    };
                    if focusable(page, next) {
                        found = Some(next);
                        break;
                    }
                    from = next;
                }
                found
            };
            if let Some(next) = next
                && self.focus(Some(next))
            {
                return true;
            }
        }
        if key == Key::Enter
            && let Some(focus) = self.host().page.as_ref().and_then(|p| p.focus)
        {
            // `submit` の入力欄で決定 → 囲む `form` の `onsubmit`（TR-B14 第三分冊 表 7-13）。
            let submit = {
                let host = self.host();
                let page = host.page.as_ref().unwrap();
                page.document
                    .element(focus)
                    .filter(|e| e.name == "input" && e.attribute("type") == Some("submit"))
                    .and_then(|_| {
                        page.document.ancestors(focus).find_map(|ancestor| {
                            page.document
                                .element(ancestor)
                                .filter(|e| e.name == "form")
                                .and_then(|form| {
                                    form.attribute("onsubmit").map(|h| (ancestor, h.to_owned()))
                                })
                        })
                    })
            };
            if let Some((form, handler)) = submit {
                let event = Event {
                    kind: "submit".into(),
                    target: Some(form),
                    ..Default::default()
                };
                if self.run_handler(&handler, event) {
                    return true;
                }
            }
            let (onclick, href) = {
                let host = self.host();
                let page = host.page.as_ref().unwrap();
                let element = page.document.element(focus);
                (
                    element.and_then(|e| e.attribute("onclick").map(str::to_owned)),
                    element
                        .filter(|e| e.name == "a")
                        .and_then(|e| e.attribute("href").map(str::to_owned)),
                )
            };
            if let Some(onclick) = onclick {
                let event = Event {
                    kind: "click".into(),
                    target: Some(focus),
                    ..Default::default()
                };
                if self.run_handler(&onclick, event) {
                    return true;
                }
            }
            if let Some(href) = href
                && !href.is_empty()
                && !href.starts_with('#')
            {
                self.launch(&href);
                return true;
            }
        }
        false
    }

    fn fire_key(&mut self, target: NodeId, attribute: &str, code: u32) -> bool {
        let handler = self.host().page.as_ref().and_then(|page| {
            page.document
                .element(target)
                .and_then(|e| e.attribute(attribute).map(str::to_owned))
        });
        let Some(handler) = handler else {
            return false;
        };
        let event = Event {
            kind: attribute.trim_start_matches("on").into(),
            target: Some(target),
            key_code: Some(code),
            ..Default::default()
        };
        self.run_handler(&handler, event)
    }

    pub(crate) fn focus(&mut self, next: Option<NodeId>) -> bool {
        let (previous, change, blur, focus_handler) = {
            let host = self.host();
            let Some(page) = &mut host.page else {
                return false;
            };
            if page.focus == next {
                return false;
            }
            let previous = page.focus;
            page.focus = next;
            page.dirty = true;
            // `onchange` は値がフォーカスを得たときと違えば、`onblur` より先
            // （TR-B14 第一分冊 表 5-9 注 2）。
            let change = previous.and_then(|id| {
                let element = page.document.element(id)?;
                if element.name != "input" {
                    return None;
                }
                let now = element.attribute("value").unwrap_or("").to_owned();
                let was = page.input_value_at_focus.take().unwrap_or_default();
                if now == was {
                    return None;
                }
                element.attribute("onchange").map(str::to_owned)
            });
            let blur = previous.and_then(|id| {
                page.document.element(id).and_then(|e| {
                    e.attribute("onblur")
                        .or_else(|| e.attribute("bml:onblur"))
                        .map(str::to_owned)
                })
            });
            let focus_handler = next.and_then(|id| {
                page.document.element(id).and_then(|e| {
                    e.attribute("onfocus")
                        .or_else(|| e.attribute("bml:onfocus"))
                        .map(str::to_owned)
                })
            });
            page.input_value_at_focus = next.and_then(|id| {
                page.document
                    .element(id)
                    .filter(|e| e.name == "input")
                    .map(|e| e.attribute("value").unwrap_or("").to_owned())
            });
            (previous, change, blur, focus_handler)
        };
        if let Some(handler) = change {
            let event = Event {
                kind: "change".into(),
                target: previous,
                ..Default::default()
            };
            if self.run_handler(&handler, event) {
                return true;
            }
        }
        if let Some(handler) = blur {
            let event = Event {
                kind: "blur".into(),
                target: previous,
                ..Default::default()
            };
            if self.run_handler(&handler, event) {
                return true;
            }
        }
        if let Some(handler) = focus_handler {
            let event = Event {
                kind: "focus".into(),
                target: next,
                ..Default::default()
            };
            if self.run_handler(&handler, event) {
                return true;
            }
        }
        false
    }
}

pub(crate) fn find_nav_index(page: &Page, index: i32) -> Option<NodeId> {
    page.document.elements().find(|id| {
        page.styles
            .get(id.0)
            .is_some_and(|style| style.nav_index == Some(index))
    })
}

pub(crate) fn fragment_target(page: &Page, fragment: &str) -> Option<NodeId> {
    let named = page.document.elements().find(|id| {
        page.document
            .element(*id)
            .and_then(|e| e.attribute("id"))
            .is_some_and(|name| name == fragment)
    })?;
    let candidates: Vec<NodeId> = if crate::runtime::profile::is_c() {
        c_profile_focusables(page)
    } else {
        page.document
            .elements()
            .filter(|id| page.styles[id.0].nav_index.is_some() && focusable(page, *id))
            .collect()
    };
    if candidates.contains(&named) {
        return Some(named);
    }
    let order: Vec<NodeId> = page.document.elements().collect();
    let at = order.iter().position(|id| *id == named)?;
    order[at..]
        .iter()
        .chain(order[..at].iter().rev())
        .find(|id| candidates.contains(id))
        .copied()
}

/// C プロファイルでフォーカスを置ける要素を文書の順に（STD-B24 第二分冊 (2/2) 付属 4 5.1.6）。
pub(crate) fn c_profile_focusables(page: &Page) -> Vec<NodeId> {
    page.document
        .elements()
        .filter(|id| {
            let Some(element) = page.document.element(*id) else {
                return false;
            };
            let kind = matches!(element.name.as_str(), "a" | "input" | "textarea");
            let handled = ["onclick", "onfocus", "onblur", "onkeydown", "onkeyup"]
                .iter()
                .any(|name| element.attribute(name).is_some());
            if !kind && !handled {
                return false;
            }
            if !focusable(page, *id) {
                return false;
            }
            if element.name == "a"
                && element
                    .children
                    .iter()
                    .all(|child| match &page.document.node(*child).kind {
                        dom::Kind::Text(text) => text.trim().is_empty(),
                        _ => false,
                    })
            {
                return false;
            }
            let style = &page.styles[id.0];
            // インラインの `a` は自分の箱を持たない。親（`p`）の大きさで見る。
            let (width, height) = if style.display == Display::Inline {
                page.document
                    .node(*id)
                    .parent
                    .map(|parent| (page.styles[parent.0].width, page.styles[parent.0].height))
                    .unwrap_or((0.0, 0.0))
            } else {
                (style.width, style.height)
            };
            width > 0.0 && height > 0.0
        })
        .collect()
}

pub(crate) fn focusable(page: &Page, id: NodeId) -> bool {
    let style = &page.styles[id.0];
    if style.display == Display::None || style.visibility != Visibility::Visible {
        return false;
    }
    if page
        .document
        .element(id)
        .is_some_and(|e| e.name == "input" && e.attribute("disabled").is_some())
    {
        return false;
    }
    page.document.ancestors(id).all(|ancestor| {
        let style = &page.styles[ancestor.0];
        style.display != Display::None && style.visibility == Visibility::Visible
    })
}
