// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 右クリックの「プラグイン › 映像フィルター」と映像フィルターのウィンドウ（`zwsdk::ui`）。
//! `main` の変更は「OK」か「適用」で設定に書き、右クリックの切り替えは押した時点で書く。

use std::collections::BTreeMap;

use zwsdk::plug::host;
use zwsdk::ui::{
    Button, Cell, Chip, Column, Ctx, Event, FileOp, Inline, MenuItem, Node, Open, Page, PickFile,
    Reply, Row, Tone,
};
use zwsdk::video::At;

use crate::presets::{
    self, Auto, Chosen, DEINTERLACE_DIR, Filter, INTERPOLATE_DIR, Item, KINDS, Kind, Manual,
    Preset, Shader, Size, Target, UNSET, channel_key,
};

mod main_event;
mod main_page;
mod panel;
mod rule;
mod shader;
mod status;
#[cfg(test)]
mod tests;

use status::status_page;

const DEINTERLACE: &str = "デインタレース";
const INTERPOLATE: &str = "補間";

fn dir_of_kind(kind: &str) -> Option<&'static str> {
    match kind {
        DEINTERLACE => Some(DEINTERLACE_DIR),
        INTERPOLATE => Some(INTERPOLATE_DIR),
        _ => None,
    }
}

fn kind_of_dir(file: &str) -> Option<&'static str> {
    if file.starts_with(&format!("{DEINTERLACE_DIR}/")) {
        Some(DEINTERLACE)
    } else if file.starts_with(&format!("{INTERPOLATE_DIR}/")) {
        Some(INTERPOLATE)
    } else {
        None
    }
}

#[derive(Debug, Default)]
pub struct Menu {
    draft: Option<Filter>,
    tab: String,
    preset: usize,
    kind: String,
    rule: Option<(Option<usize>, Auto)>,
    import: Option<(String, Shader)>,
    meta: Option<Shader>,
    error: String,
}

fn load() -> Filter {
    let settings: BTreeMap<String, String> = host::cfgs().into_iter().collect();
    Filter::load(&settings)
}

fn save(filter: &Filter) {
    for (key, value) in filter.values() {
        host::set_cfg(key, &value);
    }
}

fn target(ctx: &Ctx) -> Option<Target<'_>> {
    ctx.program.as_ref().map(|program| Target {
        program,
        format: ctx.format.as_ref(),
    })
}

fn note(text: impl Into<String>, tone: Tone) -> Node {
    Node::Note {
        text: text.into(),
        tone,
    }
}

fn line(label: &str, items: Vec<Inline>) -> Node {
    Node::Line {
        label: label.to_owned(),
        items,
    }
}

fn choice(id: &str, options: Vec<(String, String)>, value: &str, width: f32) -> Inline {
    Inline::Choice {
        id: id.to_owned(),
        options,
        value: value.to_owned(),
        width,
    }
}

fn input(id: &str, value: &str, width: f32) -> Inline {
    Inline::Input {
        id: id.to_owned(),
        value: value.to_owned(),
        width,
        hint: String::new(),
    }
}

fn toggle(id: &str, label: &str, on: bool, indent: u8, enabled: bool) -> Node {
    Node::Toggle {
        id: id.to_owned(),
        label: label.to_owned(),
        on,
        indent,
        enabled,
    }
}

fn close() -> Reply {
    Reply {
        close: true,
        ..Reply::default()
    }
}

fn open(page: impl Into<String>) -> Reply {
    Reply {
        open: Some(page.into()),
        ..Reply::default()
    }
}

fn shader_files() -> Vec<String> {
    let of = |dir: &str, exts: &[&str]| -> Vec<String> {
        host::files::list(crate::FOLDER, dir)
            .unwrap_or_default()
            .into_iter()
            .filter(|f| {
                let lower = f.name.to_ascii_lowercase();
                !f.dir && exts.iter().any(|e| lower.ends_with(e))
            })
            .map(|f| {
                if dir.is_empty() {
                    f.name
                } else {
                    format!("{dir}/{}", f.name)
                }
            })
            .collect()
    };
    let mut files = of(DEINTERLACE_DIR, &[".wgsl"]);
    files.extend(of(INTERPOLATE_DIR, &[".wgsl"]));
    files.extend(
        crate::hosted::ENTRIES
            .iter()
            .map(|entry| entry.file.to_owned()),
    );
    files.extend(of("", &[".glsl", ".hook", ".wgsl"]));
    files
}

fn channel_name(ctx: &Ctx, key: &str) -> String {
    ctx.channels
        .iter()
        .find(|c| channel_key(c.network, c.service) == key)
        .map_or_else(|| key.to_owned(), |c| c.name.clone())
}

fn unique(taken: &dyn Fn(&str) -> bool, base: &str) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}（{n}）"))
        .find(|name| !taken(name))
        .unwrap_or_default()
}

fn preset_options(filter: &Filter, current: &str) -> Vec<(String, String)> {
    let mut options = vec![(String::new(), String::from("適用しない"))];
    options.extend(
        filter
            .presets
            .iter()
            .map(|p| (p.name.clone(), p.name.clone())),
    );
    if !current.is_empty() && filter.preset(current).is_none() {
        options.push((current.to_owned(), format!("{current}（見つかりません）")));
    }
    options
}

fn ms(value: Option<f64>) -> String {
    value.map_or_else(|| String::from("—"), |v| format!("{v:.1} ms"))
}

impl zwsdk::plug::Ui for Menu {
    fn new() -> Self {
        Self {
            tab: String::from("presets"),
            ..Self::default()
        }
    }

    fn menu(&mut self, ctx: &Ctx) -> Vec<MenuItem> {
        let filter = load();
        let status = MenuItem::Open {
            label: String::from("状態…"),
            open: Open::Page(String::from("status")),
        };
        let settings = MenuItem::Open {
            label: String::from("設定…"),
            open: Open::Settings,
        };
        let panel = MenuItem::Open {
            label: String::from("パネル"),
            open: Open::Panel(String::from(panel::PANEL)),
        };
        if filter.presets.is_empty() {
            return vec![
                MenuItem::Item {
                    id: String::new(),
                    label: String::from("プリセットがありません"),
                    checked: None,
                    enabled: false,
                },
                MenuItem::Separator,
                status,
                panel,
                settings,
            ];
        }
        let manual = ctx.program.as_ref().and_then(|p| filter.manual_here(p));
        let auto_label = match target(ctx).map(|t| filter.choose(Some(t))) {
            Some(Chosen {
                auto: Some((auto, _)),
                preset: Some(preset),
                manual: false,
            }) => format!("自動選択（{auto} → {preset}）"),
            Some(Chosen {
                manual: false,
                preset: Some(preset),
                ..
            }) => format!("自動選択（{preset}）"),
            _ => String::from("自動選択"),
        };
        let has_program = ctx.program.is_some();
        let mut items = vec![
            MenuItem::Item {
                id: String::from("auto"),
                label: auto_label,
                checked: Some(manual.is_none()),
                enabled: true,
            },
            MenuItem::Separator,
        ];
        for preset in &filter.presets {
            items.push(MenuItem::Item {
                id: format!("preset:{}", preset.name),
                label: preset.name.clone(),
                checked: Some(matches!(manual, Some(Some(p)) if p.name == preset.name)),
                enabled: has_program,
            });
        }
        items.push(MenuItem::Item {
            id: String::from("none"),
            label: String::from("適用しない"),
            checked: Some(matches!(manual, Some(None))),
            enabled: has_program,
        });
        items.extend([MenuItem::Separator, status, panel, settings]);
        items
    }

    fn pick(&mut self, ctx: &Ctx, id: &str) {
        let channel = ctx
            .program
            .as_ref()
            .map(|p| channel_key(p.network, p.service));
        let mut filter = load();
        filter.manual = match (id, channel) {
            ("auto", _) => None,
            ("none", Some(channel)) => Some(Manual {
                preset: None,
                channel,
            }),
            (id, Some(channel)) => id.strip_prefix("preset:").map(|name| Manual {
                preset: Some(name.to_owned()),
                channel,
            }),
            _ => return,
        };
        host::set_cfg(presets::MANUAL, &filter.manual_value());
    }

    fn page(&mut self, page: &str, ctx: &Ctx) -> Page {
        let (name, arg) = page.split_once(':').unwrap_or((page, ""));
        let index: Option<usize> = arg.parse().ok();
        match (name, index) {
            ("rule", _) => self.rule_page(ctx),
            ("genre", Some(j)) => self.genre_page(j),
            ("channel", Some(j)) => self.channel_page(ctx, j),
            ("add", _) => self.add_page(ctx),
            ("import", _) => self.import_page(),
            ("meta", _) => self.meta_page(),
            ("status", _) => status_page(ctx),
            (panel::PANEL, _) => panel::panel_page(ctx),
            _ => self.main_page(ctx),
        }
    }

    fn event(&mut self, page: &str, ctx: &Ctx, event: &Event) -> Reply {
        let (name, arg) = page.split_once(':').unwrap_or((page, ""));
        let index: Option<usize> = arg.parse().ok();
        match (name, index) {
            ("rule", _) => self.rule_event(ctx, event),
            ("genre", Some(j)) | ("channel", Some(j)) => self.pick_values(j, event),
            ("add", _) => self.add_event(event),
            ("import", _) => self.import_event(event),
            ("meta", _) => self.meta_event(event),
            // 閉じるのは「閉じる」を押したときだけ（テーブルの行を押しても閉じない）。
            ("status", _) if event.id == "close" => close(),
            ("status", _) => Reply::default(),
            (panel::PANEL, _) => self.panel_event(ctx, event),
            _ => self.main_event(event),
        }
    }
}

impl Menu {
    fn draft(&mut self) -> &mut Filter {
        self.draft.get_or_insert_with(load)
    }

    fn changes(&mut self) -> usize {
        let saved = load().values();
        let draft = self.draft().values();
        saved.iter().zip(&draft).filter(|(a, b)| a != b).count()
    }
}
