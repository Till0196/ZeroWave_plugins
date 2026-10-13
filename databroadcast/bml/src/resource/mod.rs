// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 文書が名前で参照する、カルーセルのモジュールのファイル。名前は `/40/0000/startup.bml`
//! （コンポーネントタグ、モジュールの番号、ファイル名）。`arib-dc://` の接頭辞は剥がす。

pub mod net;
pub mod table;
pub mod xait;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

pub use isdb::bxml::File;

/// 通信で置かれた文書（`http://`）の仮のコンポーネント・モジュール。名前は URL そのもの。
pub const WEB: u16 = 0xffff;

fn is_web(name: &str) -> bool {
    name.starts_with("http://") || name.starts_with("https://")
}

#[derive(Debug, Clone, Default)]
pub struct Module {
    pub version: u8,
    pub files: Vec<File>,
    used: Cell<u64>,
}

impl Module {
    fn bytes(&self) -> usize {
        self.files.iter().map(|f| f.body.len()).sum()
    }
}

pub const BUDGET: usize = 64 << 20;

pub const WEB_BUDGET: usize = 32 << 20;
/// 件数の上限（取れなかった物は 0 バイトで、大きさの上限に掛からない）。
const WEB_MOST: usize = 512;

#[derive(Debug, Clone, Default)]
pub struct Resources {
    modules: BTreeMap<(u16, u16), Module>,
    /// コンポーネントごとにカルーセルが示した一覧（番号とバージョン）。届いていなくても、あることは分かる。
    listed: BTreeMap<u16, Vec<(u16, u8)>>,
    components: Vec<u16>,
    web: RefCell<BTreeMap<String, (Option<File>, u64)>>,
    clock: Cell<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub component: u16,
    pub module: u16,
    pub file: String,
}

impl Located {
    pub fn path(&self) -> String {
        if self.component == WEB {
            return self.file.clone();
        }
        if self.file.is_empty() {
            format!("/{:02X}/{:04X}", self.component, self.module)
        } else {
            format!("/{:02X}/{:04X}/{}", self.component, self.module, self.file)
        }
    }
}

impl Resources {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn listed(&mut self, component: u16, modules: Vec<(u16, u8)>) {
        self.modules.retain(|(tag, id), _| {
            *tag != component || modules.iter().any(|(listed, _)| listed == id)
        });
        self.listed.insert(component, modules);
    }

    /// モジュールが揃った。前と同じバージョンなら偽。`keep` 以外は [`BUDGET`] を超えたら古い順に捨てる。
    pub fn arrived(
        &mut self,
        component: u16,
        module: u16,
        version: u8,
        files: Vec<File>,
        keep: &[(u16, u16)],
    ) -> bool {
        if let Some(had) = self.modules.get(&(component, module))
            && had.version == version
        {
            return false;
        }
        let used = Cell::new(self.tick());
        self.modules.insert(
            (component, module),
            Module {
                version,
                files,
                used,
            },
        );
        self.trim(BUDGET, &(component, module), keep);
        true
    }

    fn tick(&self) -> u64 {
        let now = self.clock.get() + 1;
        self.clock.set(now);
        now
    }

    fn trim(&mut self, budget: usize, just: &(u16, u16), keep: &[(u16, u16)]) {
        let mut total: usize = self.modules.values().map(Module::bytes).sum();
        while total > budget {
            let Some((&key, victim)) = self
                .modules
                .iter()
                .filter(|(key, _)| *key != just && !keep.contains(key))
                .min_by_key(|(_, module)| module.used.get())
            else {
                break;
            };
            total -= victim.bytes();
            self.modules.remove(&key);
        }
    }

    pub fn cached_bytes(&self) -> (usize, usize) {
        let modules = self.modules.values().map(Module::bytes).sum();
        let web = self
            .web
            .borrow()
            .values()
            .map(|(file, _)| file.as_ref().map_or(0, |f| f.body.len()))
            .sum();
        (modules, web)
    }

    pub fn has_module(&self, component: u16, module: u16) -> bool {
        component == WEB || self.modules.contains_key(&(component, module))
    }

    pub fn has_listing(&self, component: u16) -> bool {
        component == WEB || self.listed.contains_key(&component)
    }

    pub fn is_listed(&self, component: u16, module: u16) -> bool {
        component == WEB
            || self
                .listed
                .get(&component)
                .is_some_and(|list| list.iter().any(|(id, _)| *id == module))
    }

    pub fn listed_version(&self, component: u16, module: u16) -> Option<u8> {
        if component == WEB {
            return Some(1);
        }
        self.listed
            .get(&component)?
            .iter()
            .find(|(id, _)| *id == module)
            .map(|(_, version)| *version)
    }

    /// PMT に載っていたコンポーネントタグ。消えたコンポーネントは捨てる（残ると `detectComponent` が「ある」と返し続ける）。
    pub fn set_components(&mut self, components: Vec<u16>) {
        if !components.is_empty() {
            self.listed.retain(|tag, _| components.contains(tag));
            self.modules.retain(|(tag, _), _| components.contains(tag));
        }
        self.components = components;
    }

    pub fn forget_web(&mut self) {
        self.web.get_mut().clear();
    }

    /// コンポーネントが放送に乗っているか（`detectComponent`）。PMT を受け取っていなければ届いた物で判断する。
    pub fn has_component(&self, component: u16) -> bool {
        if component == WEB {
            return true;
        }
        if !self.components.is_empty() {
            return self.components.contains(&component);
        }
        self.listed.contains_key(&component)
            || self.modules.keys().any(|(tag, _)| *tag == component)
    }

    pub fn component_of(base: &str, name: &str) -> Option<u16> {
        let name = name.trim();
        if is_web(name) || (is_web(base) && !name.starts_with("arib-dc://")) {
            return Some(WEB);
        }
        let name = strip_scheme(name);
        let absolute = if name.starts_with('/') {
            name.to_owned()
        } else if let Some(rest) = name.strip_prefix("~/") {
            let base = strip_scheme(base);
            let component = base.trim_start_matches('/').split('/').next().unwrap_or("");
            format!("/{component}/{rest}")
        } else {
            let directory = strip_scheme(base)
                .rsplit_once('/')
                .map_or("/", |(dir, _)| dir);
            format!("{directory}/{name}")
        };
        let first = absolute
            .split('/')
            .find(|part| !part.is_empty() && *part != ".")?;
        u16::from_str_radix(first, 16).ok()
    }

    pub fn module(&self, component: u16, module: u16) -> Option<&Module> {
        let found = self.modules.get(&(component, module))?;
        found.used.set(self.tick());
        Some(found)
    }

    pub fn locate(base: &str, name: &str) -> Option<Located> {
        let name = name.trim();
        // 通信の文書からの相対か `http://` そのもの。`arib-dc://` は放送のもの。
        if is_web(name) || (is_web(base) && !name.starts_with("arib-dc://")) {
            let url = if is_web(name) {
                name.to_owned()
            } else {
                crate::resource::net::join(base, name)?
            };
            return Some(Located {
                component: WEB,
                module: WEB,
                file: url.split('#').next().unwrap_or("").to_owned(),
            });
        }
        let name = strip_scheme(name);
        let name = name.split(['#', '?']).next().unwrap_or("");
        let absolute = if name.starts_with('/') {
            name.to_owned()
        } else if let Some(rest) = name.strip_prefix("~/") {
            let base = strip_scheme(base);
            let component = base.trim_start_matches('/').split('/').next().unwrap_or("");
            format!("/{component}/{rest}")
        } else {
            let base = strip_scheme(base);
            let directory = base.rsplit_once('/').map_or("/", |(dir, _)| dir);
            format!("{directory}/{name}")
        };
        let mut parts: Vec<&str> = Vec::new();
        for part in absolute.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                other => parts.push(other),
            }
        }
        let component = u16::from_str_radix(parts.first()?, 16).ok()?;
        let module = u16::from_str_radix(parts.get(1)?, 16).ok()?;
        Some(Located {
            component,
            module,
            file: parts[2..].join("/"),
        })
    }

    pub fn get(&self, base: &str, name: &str) -> Option<File> {
        let located = Self::locate(base, name)?;
        if located.component == WEB {
            return self.fetch(&located.file);
        }
        let module = self.module(located.component, located.module)?;
        if located.file.is_empty() {
            return module
                .files
                .iter()
                .find(|file| file.location.is_empty())
                .cloned();
        }
        module
            .files
            .iter()
            .find(|file| file.location.eq_ignore_ascii_case(&located.file))
            .cloned()
    }

    fn fetch(&self, url: &str) -> Option<File> {
        if let Some((had, used)) = self.web.borrow_mut().get_mut(url) {
            *used = self.tick();
            return had.clone();
        }
        let file = crate::resource::net::get(url)
            .ok()
            .filter(|r| r.status == 200)
            .map(|reply| {
                let location = url.rsplit('/').next().unwrap_or("").to_owned();
                File {
                    content_type: mime_of(&location).to_owned(),
                    location,
                    body: reply.body,
                }
            });
        let now = self.tick();
        let mut web = self.web.borrow_mut();
        web.insert(url.to_owned(), (file.clone(), now));
        let size = |f: &Option<File>| f.as_ref().map_or(0, |f| f.body.len());
        let mut total: usize = web.values().map(|(f, _)| size(f)).sum();
        while total > WEB_BUDGET || web.len() > WEB_MOST {
            let Some((key, bytes)) = web
                .iter()
                .filter(|(key, _)| key.as_str() != url)
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, (f, _))| (key.clone(), size(f)))
            else {
                break;
            };
            total -= bytes;
            web.remove(&key);
        }
        file
    }
}

/// 名前の末尾から媒体の種類（通信で来るものも放送と同じ拡張子なので Content-Type を信じない）。
fn mime_of(name: &str) -> &'static str {
    let extension = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match extension.as_str() {
        "bml" => "text/X-arib-bml",
        "css" => "text/css",
        "ecm" => "text/X-arib-ecmascript",
        "png" => "image/X-arib-png",
        "jpg" | "jpeg" => "image/jpeg",
        "mng" => "image/X-arib-mng",
        "btb" | "bin" => "application/X-arib-btable",
        "clt" => "application/X-arib-bmlclut",
        "drc" => "application/X-arib-drcs",
        "aif" | "aiff" => "audio/X-arib-aiff",
        _ => "application/octet-stream",
    }
}

fn strip_scheme(name: &str) -> &str {
    if let Some(rest) = name.strip_prefix("arib-dc://") {
        rest.find('/').map_or("/", |at| &rest[at..])
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_modules_are_dropped_past_the_budget_but_kept_ones_stay() {
        let mut resources = Resources::new();
        let big = |name: &str| File {
            location: name.to_owned(),
            content_type: "image/png".into(),
            body: vec![0; 3 << 20],
        };
        for id in 0..30u16 {
            resources.arrived(0x40, id, 1, vec![big("a")], &[(0x40, 0)]);
        }
        let (bytes, _) = resources.cached_bytes();
        assert!(bytes <= BUDGET, "{bytes}");
        assert!(
            resources.module(0x40, 0).is_some(),
            "エントリーコンポーネントは残る"
        );
        assert!(resources.module(0x40, 1).is_none(), "古いものから消える");
        assert!(resources.module(0x40, 29).is_some(), "届いたばかりは残る");
    }

    #[test]
    fn names_are_resolved_against_the_document() {
        let located = Resources::locate("/40/0000/startup.bml", "top.bml").unwrap();
        assert_eq!(located.path(), "/40/0000/top.bml");
        let located = Resources::locate("/40/0000/startup.bml", "../0001/multi.btb").unwrap();
        assert_eq!(located.path(), "/40/0001/multi.btb");
        let located = Resources::locate("/40/0000/startup.bml", "/60/060F").unwrap();
        assert_eq!(
            (located.component, located.module, located.file.as_str()),
            (0x60, 0x60f, "")
        );
        let located = Resources::locate("", "arib-dc://0.0.0/40/0000/x.png#a").unwrap();
        assert_eq!(located.path(), "/40/0000/x.png");
        assert!(Resources::locate("/40/0000/a.bml", "/zz/0000/x").is_none());
        let located = Resources::locate("/50/1000/news.bml", "~/0000/common.clt").unwrap();
        assert_eq!(located.path(), "/50/0000/common.clt");
    }

    #[test]
    fn a_module_that_arrives_again_at_the_same_version_is_not_news() {
        let mut resources = Resources::new();
        let file = File {
            location: "a.bml".into(),
            content_type: "text/x-arib-bml".into(),
            body: b"x".to_vec(),
        };
        assert!(resources.arrived(0x40, 0, 3, vec![file.clone()], &[]));
        assert!(!resources.arrived(0x40, 0, 3, vec![file.clone()], &[]));
        assert!(resources.arrived(0x40, 0, 4, vec![file], &[]));
        assert!(resources.get("/40/0000/startup.bml", "A.BML").is_some());
        resources.listed(0x40, vec![(1, 0)]);
        assert!(!resources.has_module(0x40, 0));
        assert!(resources.is_listed(0x40, 1));
    }
}
