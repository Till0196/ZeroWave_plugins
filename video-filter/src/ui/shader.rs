// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! シェーダーの追加・取り込み・情報のページと、種類の選択肢。

use super::*;

impl Menu {
    pub(super) fn add_page(&mut self, ctx: &Ctx) -> Page {
        let at = self.preset;
        let draft = self.draft().clone();
        let Some(preset) = draft.presets.get(at) else {
            return Page {
                title: String::from("フィルターを追加"),
                footer: vec![Button::new("close", "閉じる")],
                ..Page::default()
            };
        };
        let mut kinds = vec![(String::new(), String::from("すべて"))];
        kinds.extend(
            KINDS
                .iter()
                .filter(|k| **k != DEINTERLACE)
                .map(|k| ((*k).to_owned(), (*k).to_owned())),
        );
        kinds.push((UNSET.to_owned(), UNSET.to_owned()));
        let host = ctx
            .video
            .as_ref()
            .map(|video| video.host.clone())
            .unwrap_or_default();
        let rows = shader_files()
            .into_iter()
            .filter(|file| kind_of_dir(file) != Some(DEINTERLACE))
            .filter(|file| crate::hosted::usable(file, &host))
            .filter(|file| self.kind.is_empty() || draft.describe(file).1 == self.kind)
            .map(|file| {
                let (name, kind) = draft.describe(&file);
                let used = preset.filters.iter().any(|item| item.file == file);
                Row {
                    id: file.clone(),
                    cells: vec![
                        Cell::with_sub(name, file.clone()),
                        Cell::Tag {
                            text: kind.clone(),
                            tone: if kind == UNSET {
                                Tone::Warn
                            } else {
                                Tone::Plain
                            },
                        },
                        Cell::Buttons {
                            buttons: vec![
                                Button::new(
                                    format!("pick:{file}"),
                                    if used { "追加済み" } else { "追加" },
                                )
                                .enabled(!used),
                            ],
                        },
                    ],
                    selected: false,
                    dim: false,
                }
            })
            .collect();
        Page {
            title: format!("フィルターを追加：{}", preset.name),
            size: Some([560.0, 480.0]),
            nodes: vec![
                line(
                    "",
                    vec![Inline::Chips {
                        id: String::from("kind"),
                        chips: kinds
                            .into_iter()
                            .map(|(id, label)| Chip {
                                on: id == self.kind,
                                id,
                                label,
                            })
                            .collect(),
                        removable: false,
                    }],
                ),
                Node::Table {
                    id: String::from("files"),
                    columns: vec![
                        Column::new("フィルター", 0.0),
                        Column::new("種類", 90.0),
                        Column::new("", 96.0),
                    ],
                    rows,
                    empty: String::from("シェーダーがありません"),
                },
            ],
            footer: vec![
                Button::new("library", "シェーダーの管理…"),
                Button::new("close", "閉じる"),
            ],
            ..Page::default()
        }
    }

    pub(super) fn add_event(&mut self, event: &Event) -> Reply {
        let at = self.preset;
        match event.id.as_str() {
            "close" => return close(),
            "library" => {
                self.tab = String::from("shaders");
                return close();
            }
            "kind" => self.kind = event.value.clone(),
            id => {
                let draft = self.draft();
                if let Some(file) = id.strip_prefix("pick:")
                    && let Some(preset) = draft.presets.get(at).cloned()
                    && !preset.filters.iter().any(|item| item.file == file)
                {
                    let item = Item {
                        file: file.to_owned(),
                        on: true,
                    };
                    // 補間は一つだけ。既定の位置（レストアの後・拡大の前）へ。
                    if presets::is_interpolation(file) {
                        let mut preset = preset;
                        preset
                            .filters
                            .retain(|item| !presets::is_interpolation(&item.file));
                        let slot = draft.interpolation_slot(&preset);
                        preset.filters.insert(slot, item);
                        draft.presets[at] = preset;
                    } else {
                        draft.presets[at].filters.push(item);
                    }
                }
            }
        }
        Reply::default()
    }

    pub(super) fn import_page(&self) -> Page {
        let Some((path, shader)) = &self.import else {
            return Page {
                title: String::from("シェーダーの追加"),
                footer: vec![Button::new("cancel", "閉じる")],
                ..Page::default()
            };
        };
        Page {
            title: String::from("シェーダーの追加"),
            size: Some([560.0, 300.0]),
            nodes: vec![
                line(
                    "ファイル",
                    vec![Inline::Text {
                        text: path.clone(),
                        tone: Tone::Muted,
                    }],
                ),
                line("名前", vec![input("name", &shader.name, 0.0)]),
                line(
                    "種類",
                    vec![choice(
                        "kind",
                        kind_options(&shader.file),
                        &shader.kind,
                        200.0,
                    )],
                ),
            ],
            footer: vec![
                Button::new("add", "追加").primary(),
                Button::new("cancel", "キャンセル"),
            ],
            ..Page::default()
        }
    }

    pub(super) fn import_event(&mut self, event: &Event) -> Reply {
        match event.id.as_str() {
            "cancel" => {
                self.import = None;
                close()
            }
            "add" => {
                let Some((path, mut shader)) = self.import.take() else {
                    return close();
                };
                if let Some(dir) = dir_of_kind(&shader.kind) {
                    shader.file = format!("{dir}/{}", shader.file);
                }
                let target = shader.file.clone();
                let draft = self.draft();
                draft.shaders.retain(|s| s.file != target);
                draft.shaders.push(shader);
                Reply {
                    close: true,
                    files: vec![FileOp::Copy {
                        from: path,
                        folder: String::from(crate::FOLDER),
                        path: target,
                    }],
                    ..Reply::default()
                }
            }
            "files" => {
                self.error = event.value.clone();
                Reply::default()
            }
            "name" => {
                if let Some((_, shader)) = self.import.as_mut() {
                    shader.name = event.value.trim().to_owned();
                }
                Reply::default()
            }
            "kind" => {
                if let Some((_, shader)) = self.import.as_mut() {
                    shader.kind = event.value.clone();
                }
                Reply::default()
            }
            _ => Reply::default(),
        }
    }

    pub(super) fn meta_page(&self) -> Page {
        let Some(shader) = &self.meta else {
            return Page {
                title: String::from("シェーダーの編集"),
                footer: vec![Button::new("cancel", "閉じる")],
                ..Page::default()
            };
        };
        let fixed = kind_of_dir(&shader.file);
        let kind = match fixed {
            Some(kind) => vec![(kind.to_owned(), kind.to_owned())],
            None => kind_options(&shader.file),
        };
        Page {
            title: String::from("シェーダーの編集"),
            size: Some([560.0, 300.0]),
            nodes: vec![
                line(
                    "ファイル",
                    vec![Inline::Text {
                        text: shader.file.clone(),
                        tone: Tone::Muted,
                    }],
                ),
                line("名前", vec![input("name", &shader.name, 0.0)]),
                line(
                    "種類",
                    vec![choice("kind", kind, fixed.unwrap_or(&shader.kind), 200.0)],
                ),
            ],
            footer: vec![
                Button::new("save", "保存").primary(),
                Button::new("cancel", "キャンセル"),
            ],
            ..Page::default()
        }
    }

    pub(super) fn meta_event(&mut self, event: &Event) -> Reply {
        match event.id.as_str() {
            "cancel" => {
                self.meta = None;
                close()
            }
            "save" => {
                if let Some(mut shader) = self.meta.take() {
                    if let Some(kind) = kind_of_dir(&shader.file) {
                        shader.kind = kind.to_owned();
                    }
                    let draft = self.draft();
                    draft.shaders.retain(|s| s.file != shader.file);
                    draft.shaders.push(shader);
                }
                close()
            }
            "name" => {
                if let Some(shader) = self.meta.as_mut() {
                    shader.name = event.value.trim().to_owned();
                }
                Reply::default()
            }
            "kind" => {
                if let Some(shader) = self.meta.as_mut() {
                    shader.kind = event.value.clone();
                }
                Reply::default()
            }
            _ => Reply::default(),
        }
    }
}

fn kind_options(file: &str) -> Vec<(String, String)> {
    let wgsl = file.to_ascii_lowercase().ends_with(".wgsl");
    KINDS
        .iter()
        .filter(|k| (**k != DEINTERLACE && **k != INTERPOLATE) || wgsl)
        .map(|k| ((*k).to_owned(), (*k).to_owned()))
        .collect()
}
