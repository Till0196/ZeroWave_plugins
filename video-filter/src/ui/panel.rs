// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 下のパネル（マニフェストの `ui.panels` の `filter`）：プリセットの切り替えと、処理時間とパス。

use super::*;

pub(super) const PANEL: &str = "filter";

pub(super) fn panel_page(ctx: &Ctx) -> Page {
    let filter = load();
    let chosen = filter.choose(target(ctx));
    let manual = ctx.program.as_ref().and_then(|p| filter.manual_here(p));
    let mut chips = vec![Chip {
        id: String::from("auto"),
        label: match (&chosen.preset, manual) {
            (Some(preset), None) => format!("自動（{preset}）"),
            _ => String::from("自動"),
        },
        on: manual.is_none(),
    }];
    chips.extend(filter.presets.iter().map(|preset| Chip {
        id: format!("preset:{}", preset.name),
        label: preset.name.clone(),
        on: matches!(manual, Some(Some(p)) if p.name == preset.name),
    }));
    chips.push(Chip {
        id: String::from("none"),
        label: String::from("なし"),
        on: matches!(manual, Some(None)),
    });
    let mut nodes = vec![line(
        "プリセット",
        vec![
            Inline::Chips {
                id: String::from("preset"),
                chips,
                removable: false,
            },
            Inline::Button(Button::new("settings", "設定…")),
        ],
    )];
    nodes.push(line(
        "処理時間",
        vec![Inline::Text {
            text: load_text(ctx),
            tone: Tone::Muted,
        }],
    ));
    let mut files = filter.files(&chosen, At::Deinterlace);
    files.extend(filter.files(&chosen, At::Picture));
    let path = if files.is_empty() {
        String::from("適用しない")
    } else {
        files
            .iter()
            .map(|file| filter.describe(file).0)
            .collect::<Vec<_>>()
            .join(" → ")
    };
    nodes.push(line(
        "パス",
        vec![Inline::Text {
            text: path,
            tone: Tone::Plain,
        }],
    ));
    Page {
        title: String::from("映像フィルター"),
        nodes,
        ..Page::default()
    }
}

fn load_text(ctx: &Ctx) -> String {
    let Some(video) = ctx.video.as_ref() else {
        return String::from("—");
    };
    let measured: Vec<f64> = video.stages.iter().filter_map(|stage| stage.ms).collect();
    if measured.is_empty() {
        return String::from("—");
    }
    let total: f64 = measured.iter().sum();
    if video.budget > 0.0 {
        format!("{total:.2} ms／枚（上限 {:.2} ms）", video.budget)
    } else {
        format!("{total:.2} ms／枚")
    }
}

impl Menu {
    pub(super) fn panel_event(&mut self, ctx: &Ctx, event: &Event) -> Reply {
        match event.id.as_str() {
            // 右クリックのメニューと同じ切り替え（押した時点で書く）。
            "preset" => {
                <Self as zwsdk::plug::Ui>::pick(self, ctx, &event.value);
                Reply::default()
            }
            "settings" => open("main"),
            _ => Reply::default(),
        }
    }
}
