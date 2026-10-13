// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 外から見る・渡す：診断の実行、レンダリング用の HTML、レジスターや不揮発メモリーの出し入れ、閉じると選局。

use crate::resource::File;

use super::*;

impl Browser {
    pub fn eval(&mut self, source: &str) {
        if self.host().page.is_none() {
            return;
        }
        let _ = self.run_script(source);
        self.settle_after_script();
        self.finish();
    }

    pub fn html(&mut self) -> Option<crate::document::html::Html> {
        let dirty = self.host().page.as_ref().is_some_and(|page| page.dirty);
        if dirty {
            self.laid();
        }
        self.measure_pictures();
        let page = self.host().page.as_ref()?;
        let offsets: BTreeMap<NodeId, f32> = page
            .marquees
            .iter()
            .map(|(id, state)| (*id, state.offset))
            .collect();
        let frames: BTreeMap<NodeId, usize> = page
            .animations
            .iter()
            .map(|(id, (frame, _))| (*id, *frame))
            .collect();
        Some(crate::document::html::render(
            &crate::document::html::Scene {
                document: &page.document,
                styles: &page.styles,
                clut: &page.clut,
                marquees: &offsets,
                frames: &frames,
                drcs: &page.drcs,
                sizes: &page.sizes,
            },
        ))
    }

    /// 文書が参照する画像（body の背景画像も）の大きさを、まだ読んでいない物のみ読む（画像は伸ばさず画像の大きさで置くので）。
    fn measure_pictures(&mut self) {
        let host = self.host();
        let Some(page) = host.page.as_mut() else {
            return;
        };
        let mut names = Vec::new();
        for id in page.document.elements() {
            let Some(element) = page.document.element(id) else {
                continue;
            };
            let name = match element.name.as_str() {
                "object"
                    if element
                        .attribute("type")
                        .is_some_and(|kind| kind.to_ascii_lowercase().starts_with("image/")) =>
                {
                    element.attribute("data")
                }
                "img" => element.attribute("src"),
                _ => None,
            };
            if let Some(name) = name
                && !name.is_empty()
                && !page.sizes.contains_key(name)
            {
                names.push(name.to_owned());
            }
        }
        if let Some(body) = page.document.first_named("body")
            && let Some(name) = page
                .styles
                .get(body.0)
                .and_then(|style| style.background_image.as_deref())
            && !page.sizes.contains_key(name)
        {
            names.push(name.to_owned());
        }
        for name in names {
            if let Some(file) = host.resources.get(&page.name, &name)
                && let Some(size) = crate::document::html::picture_size(&file.body)
            {
                page.sizes.insert(name, size);
            }
        }
    }

    pub fn invisible(&mut self) -> bool {
        self.host().page.as_ref().is_none_or(|page| {
            page.document.first_named("body").is_none_or(|body| {
                page.document
                    .element(body)
                    .is_some_and(|element| element.attribute("invisible") == Some("invisible"))
            })
        })
    }

    pub fn is_open(&mut self) -> bool {
        self.host().page.is_some()
    }

    /// いまの文書が取るキーの組（`used-key-list`）。受像機は無い組のキーを自分で使う
    /// （数字は選局に。TR-B14 第二分冊 2.1.10）。文書が無いか `none` なら空。
    pub fn used_keys(&mut self) -> Vec<String> {
        let host = self.host();
        let Some(page) = &host.page else {
            return Vec::new();
        };
        let used = page
            .document
            .first_named("body")
            .map(|body| page.styles[body.0].used_key_list.clone())
            .unwrap_or_default();
        if used.first().is_some_and(|first| first == "none") {
            return Vec::new();
        }
        used
    }

    /// 開こうとした文書（かそのテーブル）がまだ届かず待っているか。
    pub fn waiting(&mut self) -> bool {
        self.host().waiting.is_some()
    }

    pub fn clut(&mut self) -> Clut {
        self.host()
            .page
            .as_ref()
            .map_or_else(Clut::default, |page| page.clut.clone())
    }

    pub fn file(&mut self, name: &str) -> Option<File> {
        let host = self.host();
        let base = host.page.as_ref().map_or(String::new(), |p| p.name.clone());
        host.resources.get(&base, name)
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.host().requests)
    }

    /// `Ureg`（利用者）と `Greg`（局）のコピー。通信のアプリも同じものを見るので向こうへ渡す。
    pub fn registers(&mut self) -> (Vec<String>, Vec<String>) {
        let host = self.host();
        (host.ureg.clone(), host.greg.clone())
    }

    /// 通信のアプリが書いた `Ureg`/`Greg` を受け取る（局はここに戻り先を書く）。
    pub fn set_registers(&mut self, ureg: Vec<String>, greg: Vec<String>) {
        let host = self.host();
        for (slot, value) in host.ureg.iter_mut().zip(ureg) {
            *slot = value;
        }
        for (slot, value) in host.greg.iter_mut().zip(greg) {
            *slot = value;
        }
    }

    pub fn set_nvram(&mut self, nvram: BTreeMap<String, Vec<u8>>) {
        self.host().nvram = nvram;
    }

    pub fn set_zipcode(&mut self, zipcode: &str) {
        self.host().zipcode = zipcode.to_owned();
    }

    pub fn set_cas_id(&mut self, cas_id: &str) {
        self.host().cas_id = cas_id.to_owned();
    }

    pub fn nvram_if_changed(&mut self) -> Option<BTreeMap<String, Vec<u8>>> {
        let host = self.host();
        if !host.nvram_changed {
            return None;
        }
        host.nvram_changed = false;
        Some(host.nvram.clone())
    }

    pub fn set_wanted(&mut self, on: bool) {
        self.host().wanted = on;
        if on {
            self.try_start();
        } else {
            self.close();
        }
    }

    pub fn close(&mut self) {
        self.host().resources.forget_web();
        let host = self.host();
        host.page = None;
        host.data_button_pending = false;
        host.timers.clear();
        host.started = false;
        host.launch = None;
        host.waiting = None;
    }

    pub fn retune(&mut self) {
        self.host().wanted = false;
        self.close();
        let host = self.host();
        host.resources = Resources::new();
        host.entry = None;
        host.tables.clear();
        host.locked.clear();
        host.ait_url = None;
        host.root_certificates.clear();
        host.event = None;
        host.pending_focus = None;
        host.quit = false;
        // 放送の識別子も忘れる（前の番号のままだと、カルーセルを分け合うサービスで前の行き先へ進む）。
        let shown = host.broadcast.caption_shown;
        host.broadcast = crate::runtime::Broadcast {
            caption_shown: shown,
            ..Default::default()
        };
        // `Ureg` は選局で空に戻す（TR-B14 第三分冊 8.1.15.1）。
        for slot in &mut host.ureg {
            slot.clear();
        }
    }
}
