// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 映像フィルターのウィンドウの本体（各タブ）の操作を受ける。

use super::*;

impl Menu {
    pub(super) fn main_event(&mut self, event: &Event) -> Reply {
        let value = event.value.as_str();
        match event.id.as_str() {
            "tab" => {
                self.tab = value.to_owned();
                return Reply::default();
            }
            "ok" => {
                save(self.draft());
                self.draft = None;
                return close();
            }
            "cancel" => {
                self.draft = None;
                return close();
            }
            "apply" => {
                save(self.draft());
                return Reply::default();
            }
            "files" => {
                self.error = value.to_owned();
                return Reply::default();
            }
            _ => {}
        }
        match self.tab.as_str() {
            "autos" => self.autos_event(event),
            "shaders" => self.shaders_event(event),
            "notify" => {
                notify_event(&mut self.draft().notify, event);
                Reply::default()
            }
            _ => self.presets_event(event),
        }
    }

    fn presets_event(&mut self, event: &Event) -> Reply {
        let value = event.value.as_str();
        let id = event.id.as_str();
        let at = self.preset;
        let index = |prefix: &str| -> Option<usize> { id.strip_prefix(prefix)?.parse().ok() };
        match id {
            "presets" => {
                if let Ok(i) = value.parse() {
                    self.preset = i;
                }
            }
            "new" => {
                let draft = self.draft();
                let name = unique(&|n| draft.preset(n).is_some(), "新しいプリセット");
                draft.presets.push(Preset {
                    name,
                    ..Preset::default()
                });
                self.preset = self.draft().presets.len() - 1;
            }
            "copy" => {
                let draft = self.draft();
                if let Some(preset) = draft.presets.get(at).cloned() {
                    let mut copy = preset.clone();
                    copy.name = unique(
                        &|n| draft.preset(n).is_some(),
                        &format!("{} のコピー", preset.name),
                    );
                    draft.presets.insert(at + 1, copy);
                    self.preset = at + 1;
                }
            }
            "remove" => {
                let draft = self.draft();
                if draft
                    .presets
                    .get(at)
                    .is_some_and(|p| draft.uses_of_preset(&p.name) == 0)
                {
                    draft.presets.remove(at);
                    self.preset = at.saturating_sub(1);
                }
            }
            "name" => {
                let name = value.trim().to_owned();
                let draft = self.draft();
                if let Some(old) = draft.presets.get(at).map(|p| p.name.clone())
                    && !name.is_empty()
                    && draft.preset(&name).is_none()
                {
                    draft.presets[at].name = name.clone();
                    for auto in &mut draft.autos {
                        if auto.preset == old {
                            auto.preset = name.clone();
                        }
                    }
                    if draft.fallback == old {
                        draft.fallback = name;
                    }
                }
            }
            "deinterlace" => {
                if let Some(preset) = self.draft().presets.get_mut(at) {
                    preset.deinterlace = value.to_owned();
                }
            }
            "target" => {
                if let Some(preset) = self.draft().presets.get_mut(at) {
                    preset.target = value.to_owned();
                }
            }
            "buffer" => {
                if let Some(preset) = self.draft().presets.get_mut(at) {
                    preset.buffer = value.to_owned();
                }
            }
            "held" => {
                if let Some(preset) = self.draft().presets.get_mut(at) {
                    preset.held = value.to_owned();
                }
            }
            "keep" => {
                if let Some(preset) = self.draft().presets.get_mut(at) {
                    preset.keep = value == "true";
                }
            }
            "add" => return open("add"),
            _ => {
                if let Some(preset) = self.draft().presets.get_mut(at) {
                    let filters = &mut preset.filters;
                    if let Some(i) = index("on:").filter(|&i| i < filters.len()) {
                        filters[i].on = value == "true";
                    } else if let Some(i) = index("up:").filter(|&i| i > 0 && i < filters.len()) {
                        filters.swap(i, i - 1);
                    } else if let Some(i) = index("down:").filter(|&i| i + 1 < filters.len()) {
                        filters.swap(i, i + 1);
                    } else if let Some(i) = index("drop:").filter(|&i| i < filters.len()) {
                        filters.remove(i);
                    }
                }
            }
        }
        Reply::default()
    }

    fn autos_event(&mut self, event: &Event) -> Reply {
        let id = event.id.as_str();
        let index = |prefix: &str| -> Option<usize> { id.strip_prefix(prefix)?.parse().ok() };
        let draft = self.draft();
        let count = draft.autos.len();
        if id == "new_auto" {
            let name = unique(
                &|n| draft.autos.iter().any(|a| a.name == n),
                "新しい自動選択",
            );
            let preset = draft
                .presets
                .first()
                .map(|p| p.name.clone())
                .unwrap_or_default();
            self.rule = Some((
                None,
                Auto {
                    name,
                    any: false,
                    conditions: vec![Kind::Genre.fresh()],
                    preset,
                },
            ));
            return open("rule");
        }
        if id == "fallback" {
            draft.fallback = event.value.clone();
        } else if id == "playback" {
            draft.playback = event.value == "true";
        } else if let Some(i) = index("preset:").filter(|&i| i < count) {
            draft.autos[i].preset = event.value.clone();
        } else if let Some(i) = index("edit:")
            .or_else(|| (id == "autos").then(|| event.value.parse().ok()).flatten())
            .filter(|&i| i < count)
        {
            self.rule = Some((Some(i), draft.autos[i].clone()));
            return open("rule");
        } else if let Some(i) = index("up:").filter(|&i| i > 0 && i < count) {
            draft.autos.swap(i, i - 1);
        } else if let Some(i) = index("down:").filter(|&i| i + 1 < count) {
            draft.autos.swap(i, i + 1);
        } else if let Some(i) = index("drop:").filter(|&i| i < count) {
            draft.autos.remove(i);
        }
        Reply::default()
    }

    fn shaders_event(&mut self, event: &Event) -> Reply {
        let id = event.id.as_str();
        if id == "kind" {
            self.kind = event.value.clone();
        } else if id == "import" {
            return Reply {
                pick_file: Some(PickFile {
                    id: String::from("picked"),
                    filters: vec![(
                        String::from("シェーダー"),
                        vec![
                            String::from("glsl"),
                            String::from("hook"),
                            String::from("wgsl"),
                        ],
                    )],
                }),
                ..Reply::default()
            };
        } else if id == "picked" {
            let file = event
                .value
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or_default()
                .to_owned();
            let stem = file
                .rsplit_once('.')
                .map_or(file.as_str(), |(s, _)| s)
                .to_owned();
            self.import = Some((
                event.value.clone(),
                Shader {
                    file,
                    name: stem,
                    kind: String::from("拡大"),
                },
            ));
            return open("import");
        } else if let Some(file) = id.strip_prefix("meta:") {
            let draft = self.draft();
            let (name, kind) = draft.describe(file);
            self.meta = Some(Shader {
                file: file.to_owned(),
                name,
                kind: if kind == UNSET { String::new() } else { kind },
            });
            return open("meta");
        } else if let Some(file) = id.strip_prefix("delete:") {
            let draft = self.draft();
            if draft.uses_of_shader(file) == 0 {
                draft.shaders.retain(|s| s.file != file);
                return Reply {
                    files: vec![FileOp::Remove {
                        folder: String::from(crate::FOLDER),
                        path: file.to_owned(),
                    }],
                    ..Reply::default()
                };
            }
        }
        Reply::default()
    }
}

fn notify_event(n: &mut presets::Notify, event: &Event) {
    let on = event.value == "true";
    match event.id.as_str() {
        "preset" => n.preset = on,
        "rule" => n.rule = on,
        "chain" => n.chain = on,
        "size" => n.size = on,
        "manual" => n.manual = on,
        "quiet_same" => n.quiet_same = on,
        "compile" => n.compile = on,
        "built" => n.built = on,
        "paused" => n.paused = on,
        "error" => n.error = on,
        "lines" => n.lines = event.value.parse().unwrap_or(3),
        "extend" => n.extend = on,
        _ => {}
    }
}
