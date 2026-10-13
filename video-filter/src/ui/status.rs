// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 「状態…」のページ。

use super::*;

pub(super) fn status_page(ctx: &Ctx) -> Page {
    let filter = load();
    let chosen = filter.choose(target(ctx));
    let mut nodes = Vec::new();
    let title = chosen
        .preset
        .clone()
        .unwrap_or_else(|| String::from("適用しない"));
    nodes.push(note(title, Tone::Plain));
    let mut sub = Vec::new();
    if let Some((auto, why)) = &chosen.auto {
        sub.push(if why.is_empty() {
            format!("自動選択［{auto}］")
        } else {
            format!("自動選択［{auto}］：{why}")
        });
    } else if chosen.manual {
        sub.push(String::from("手動"));
    }
    let Some(video) = ctx.video.as_ref() else {
        nodes.push(note(sub.join("　"), Tone::Muted));
        return status_wrap(nodes);
    };
    if !video.device.is_empty() {
        sub.push(video.device.clone());
    }
    nodes.push(note(sub.join("　"), Tone::Muted));
    if video.building {
        nodes.push(note("シェーダーコンパイル中", Tone::Warn));
    }
    let mut rows = Vec::new();
    let deinterlace_files = filter.files(&chosen, At::Deinterlace);
    let picture_files = filter.files(&chosen, At::Picture);
    for stage in &video.stages {
        match stage.at {
            At::Deinterlace => {
                let name = deinterlace_files
                    .first()
                    .map_or_else(|| String::from("デインタレース"), |f| filter.describe(f).0);
                rows.push(Row {
                    id: String::from("deinterlace"),
                    cells: vec![
                        over_cell(
                            name,
                            format!("{}×{}", stage.input[0], stage.input[1]),
                            stage.over,
                        ),
                        Cell::muted(ms(stage.ms)),
                        Cell::muted(ms(stage.max)),
                    ],
                    selected: false,
                    dim: false,
                });
            }
            At::Picture => {
                if stage.passes.is_empty() {
                    rows.push(Row {
                        id: String::from("picture"),
                        cells: vec![
                            over_cell(
                                String::from("フィルター"),
                                format!(
                                    "{}×{} → {}×{}",
                                    stage.input[0],
                                    stage.input[1],
                                    stage.output[0],
                                    stage.output[1]
                                ),
                                stage.over,
                            ),
                            Cell::muted(ms(stage.ms)),
                            Cell::muted(ms(stage.max)),
                        ],
                        selected: false,
                        dim: false,
                    });
                    continue;
                }
                for (index, file) in picture_files.iter().enumerate() {
                    let sum: f64 = stage
                        .passes
                        .iter()
                        .filter(|(name, _)| crate::unit_of(name) == Some(index))
                        .map(|(_, ms)| ms)
                        .sum();
                    rows.push(Row {
                        id: file.clone(),
                        cells: vec![
                            over_cell(filter.describe(file).0, file.clone(), stage.over),
                            Cell::muted(format!("{sum:.1} ms")),
                            Cell::muted(String::from("")),
                        ],
                        selected: false,
                        dim: false,
                    });
                }
            }
        }
    }
    for paused in &video.paused {
        let what = match paused.at {
            At::Deinterlace => "デインタレース",
            At::Picture => "フィルター",
        };
        let files = match paused.at {
            At::Deinterlace => &deinterlace_files,
            At::Picture => &picture_files,
        };
        let names = crate::notices::names(&filter, files);
        rows.push(Row {
            id: format!("paused:{what}"),
            cells: vec![
                Cell::Text {
                    text: if names.is_empty() {
                        format!("{what}　一時停止中")
                    } else {
                        format!("{what}　一時停止中：{names}")
                    },
                    tone: Tone::Warn,
                    sub: format!(
                        "処理が間に合いません（{:.1} ms ／ {:.1} ms）",
                        paused.ms, paused.budget
                    ),
                },
                Cell::muted("—"),
                Cell::muted(format!("{:.1} ms", paused.ms)),
            ],
            selected: false,
            dim: false,
        });
    }
    nodes.push(Node::Section {
        label: String::from("フィルター"),
    });
    nodes.push(Node::Table {
        id: String::from("stages"),
        columns: vec![
            Column::new("", 0.0),
            Column::new("平均", 70.0).right(),
            Column::new("最大", 70.0).right(),
        ],
        rows,
        empty: String::from("映像フィルターは適用されていません"),
    });
    let total: f64 = video.stages.iter().filter_map(|s| s.ms).sum();
    if video.budget > 0.0 && video.stages.iter().any(|s| s.ms.is_some()) {
        let parts = video
            .stages
            .iter()
            .map(|s| {
                (
                    (s.ms.unwrap_or(0.0) / video.budget) as f32,
                    if s.at == At::Deinterlace {
                        Tone::Muted
                    } else {
                        Tone::Plain
                    },
                )
            })
            .collect();
        let rate = if video.rate > 0.0 {
            format!("（{:.2} fps）", video.rate)
        } else {
            String::new()
        };
        nodes.push(Node::Bar {
            label: String::from("負荷"),
            parts,
            left: format!("合計 {total:.1} ms"),
            right: format!("1 フレームの上限 {:.1} ms{rate}", video.budget),
        });
    }
    if video.stages.iter().any(|s| s.over) {
        nodes.push(note(
            "処理が間に合っていません（一時停止しない設定なので掛け続けています）",
            Tone::Warn,
        ));
    }
    for why in &video.failed {
        nodes.push(note(why.clone(), Tone::Bad));
    }
    nodes.push(Node::Section {
        label: String::from("レンダリング"),
    });
    nodes.push(line(
        "表示フレーム数",
        vec![Inline::Text {
            text: video.drawn.to_string(),
            tone: Tone::Plain,
        }],
    ));
    nodes.push(line(
        "破棄フレーム数",
        vec![Inline::Text {
            text: if video.late_recent > 0 {
                format!(
                    "{}（直近 3 秒で {}。処理が間に合っていません）",
                    video.late, video.late_recent
                )
            } else {
                video.late.to_string()
            },
            tone: if video.late_recent > 0 {
                Tone::Bad
            } else {
                Tone::Plain
            },
        }],
    ));
    status_wrap(nodes)
}

fn over_cell(name: String, sub: String, over: bool) -> Cell {
    if over {
        Cell::Text {
            text: format!("{name}　間に合っていません"),
            tone: Tone::Warn,
            sub,
        }
    } else {
        Cell::with_sub(name, sub)
    }
}

fn status_wrap(nodes: Vec<Node>) -> Page {
    Page {
        title: String::from("映像フィルターの状態"),
        size: Some([560.0, 480.0]),
        nodes,
        footer: vec![Button::new("close", "閉じる")],
        ..Page::default()
    }
}
