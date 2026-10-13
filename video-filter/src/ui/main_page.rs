// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 映像フィルターのウィンドウの本体（プリセット・自動選択・シェーダー・通知の各タブ）を作る。

use super::*;

impl Menu {
    pub(super) fn main_page(&mut self, ctx: &Ctx) -> Page {
        if self.draft.is_none() {
            let saved = load();
            if let Some(name) = saved.choose(target(ctx)).preset
                && let Some(at) = saved.presets.iter().position(|p| p.name == name)
            {
                self.preset = at;
            }
        }
        let changes = self.changes();
        let filter = self.draft().clone();
        let nodes = match self.tab.as_str() {
            "autos" => self.autos_nodes(&filter, ctx),
            "shaders" => self.shaders_nodes(&filter),
            "notify" => notify_nodes(&filter),
            _ => self.presets_nodes(&filter, ctx),
        };
        Page {
            title: String::from("映像フィルター"),
            size: Some([760.0, 680.0]),
            tabs: [
                ("presets", "プリセット"),
                ("autos", "自動選択"),
                ("shaders", "シェーダー"),
                ("notify", "通知"),
            ]
            .iter()
            .map(|(id, label)| ((*id).to_owned(), (*label).to_owned()))
            .collect(),
            tab: self.tab.clone(),
            nodes,
            status: if changes == 0 {
                String::from("変更はありません")
            } else {
                format!("保存されていない変更 {changes} 件")
            },
            footer: vec![
                Button::new("ok", "OK").primary(),
                Button::new("cancel", "キャンセル"),
                Button::new("apply", "適用").enabled(changes > 0),
            ],
            ..Page::default()
        }
    }

    fn presets_nodes(&mut self, filter: &Filter, ctx: &Ctx) -> Vec<Node> {
        self.preset = self.preset.min(filter.presets.len().saturating_sub(1));
        let selected = filter.presets.get(self.preset);
        let used = selected.map_or(0, |p| filter.uses_of_preset(&p.name));
        let left = vec![
            Node::Table {
                id: String::from("presets"),
                columns: vec![Column::new("", 0.0)],
                rows: filter
                    .presets
                    .iter()
                    .enumerate()
                    .map(|(i, preset)| {
                        let uses = filter.uses_of_preset(&preset.name);
                        Row {
                            id: i.to_string(),
                            cells: vec![Cell::with_sub(
                                preset.name.clone(),
                                if uses == 0 {
                                    String::from("未使用")
                                } else {
                                    format!("自動選択 {uses} 件で使用中")
                                },
                            )],
                            selected: i == self.preset,
                            dim: false,
                        }
                    })
                    .collect(),
                empty: String::from("プリセットがありません"),
            },
            Node::Buttons {
                buttons: vec![
                    Button::new("new", "＋ 新規"),
                    Button::new("copy", "複製").enabled(selected.is_some()),
                    Button::new("remove", "削除").enabled(selected.is_some() && used == 0),
                ],
                right: false,
            },
        ];
        let Some(preset) = selected else {
            return vec![Node::Columns {
                width: 200.0,
                left,
                right: Vec::new(),
            }];
        };
        let applied = load().choose(target(ctx));
        let measuring = applied.preset.as_deref() == Some(preset.name.as_str());
        let video = ctx.video.as_ref().filter(|_| measuring);
        let stage = |at: At| video.and_then(|v| v.stages.iter().find(|s| s.at == at));
        let unit_ms = |index: usize| -> Option<f64> {
            let stage = stage(At::Picture)?;
            let sum: f64 = stage
                .passes
                .iter()
                .filter(|(name, _)| crate::unit_of(name) == Some(index))
                .map(|(_, ms)| ms)
                .sum();
            (!stage.passes.is_empty()).then_some(sum)
        };
        let mut deinterlace_options = vec![(String::new(), String::from("内蔵（bwdif）"))];
        for shader in shader_files() {
            if let Some(name) = shader.strip_prefix(&format!("{DEINTERLACE_DIR}/")) {
                deinterlace_options.push((name.to_owned(), filter.describe(&shader).0));
            }
        }
        if !preset.deinterlace.is_empty()
            && !deinterlace_options
                .iter()
                .any(|(v, _)| *v == preset.deinterlace)
        {
            deinterlace_options.push((
                preset.deinterlace.clone(),
                format!("{}（ファイルが見つかりません）", preset.deinterlace),
            ));
        }
        let mut targets: Vec<(String, String)> = vec![
            (String::new(), String::from("画面のリフレッシュレート")),
            (String::from("60"), String::from("60 fps")),
            // 24p の三倍（フィルムの一コマを等間隔に三つへ割る）。
            (String::from("72"), String::from("72 fps")),
            (String::from("120"), String::from("120 fps")),
        ];
        if !preset.target.is_empty() && !targets.iter().any(|(v, _)| *v == preset.target) {
            targets.push((preset.target.clone(), format!("{} fps", preset.target)));
        }
        let last = preset.filters.len().saturating_sub(1);
        let host = ctx
            .video
            .as_ref()
            .map(|video| video.host.clone())
            .unwrap_or_default();
        let mut on_index = 0usize;
        let rows: Vec<Row> = preset
            .filters
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let (name, kind) = filter.describe(&item.file);
                // 警告は本体が実装を通知したときだけ。
                let usable = host.is_empty() || crate::hosted::usable(&item.file, &host);
                let time = if item.on {
                    let t = unit_ms(on_index);
                    on_index += 1;
                    ms(t)
                } else {
                    String::from("—")
                };
                Row {
                    id: i.to_string(),
                    cells: vec![
                        Cell::Toggle {
                            id: format!("on:{i}"),
                            on: item.on,
                        },
                        if usable {
                            Cell::with_sub(name, item.file.clone())
                        } else {
                            Cell::Text {
                                text: name,
                                tone: Tone::Warn,
                                sub: format!(
                                    "この GPU では使えないので {} にする",
                                    crate::hosted::FALLBACK
                                ),
                            }
                        },
                        Cell::Tag {
                            text: kind.clone(),
                            tone: if kind == UNSET {
                                Tone::Warn
                            } else {
                                Tone::Plain
                            },
                        },
                        Cell::muted(time),
                        Cell::Buttons {
                            buttons: vec![
                                Button::new(format!("up:{i}"), "上へ")
                                    .icon("up")
                                    .enabled(i > 0),
                                Button::new(format!("down:{i}"), "下へ")
                                    .icon("down")
                                    .enabled(i < last),
                                Button::new(format!("drop:{i}"), "外す").icon("remove"),
                            ],
                        },
                    ],
                    selected: false,
                    dim: !item.on,
                }
            })
            .collect();
        let mut right = vec![
            line("名前", vec![input("name", &preset.name, 0.0)]),
            line(
                "デインタレース",
                vec![choice(
                    "deinterlace",
                    deinterlace_options,
                    &preset.deinterlace,
                    0.0,
                )],
            ),
            Node::Section {
                label: String::from("利用するフィルター"),
            },
            Node::Table {
                id: String::from("filters"),
                columns: vec![
                    Column::new("", 30.0),
                    Column::new("フィルター", 0.0),
                    Column::new("種類", 84.0),
                    Column::new("負荷", 64.0).right(),
                    Column::new("", 92.0),
                ],
                rows,
                empty: String::from("フィルターがありません"),
            },
            Node::Buttons {
                buttons: vec![Button::new("add", "＋ 追加…")],
                right: true,
            },
        ];
        if preset
            .filters
            .iter()
            .any(|item| presets::is_interpolation(&item.file))
        {
            right.push(line(
                "補間の目標",
                vec![choice("target", targets, &preset.target, 220.0)],
            ));
        }
        let mut buffers: Vec<(String, String)> = vec![
            (String::new(), String::from("自動")),
            (String::from("0"), String::from("使わない")),
            (String::from("250"), String::from("0.25 秒")),
            (String::from("500"), String::from("0.5 秒")),
            (String::from("1000"), String::from("1 秒")),
        ];
        if !preset.buffer.is_empty() && !buffers.iter().any(|(v, _)| *v == preset.buffer) {
            buffers.push((preset.buffer.clone(), format!("{} ms", preset.buffer)));
        }
        right.push(line(
            "先行レンダリングのバッファ",
            vec![choice("buffer", buffers, &preset.buffer, 220.0)],
        ));
        if preset.filters.iter().any(|item| presets::lags(&item.file)) {
            let mut helds: Vec<(String, String)> = vec![
                (String::new(), String::from("自動")),
                (String::from("1"), String::from("1 フレーム")),
                (String::from("2"), String::from("2 フレーム")),
            ];
            if !helds.iter().any(|(v, _)| *v == preset.held) {
                helds.push((preset.held.clone(), preset.held.clone()));
            }
            right.push(line(
                "補間で遅らせる長さ",
                vec![choice("held", helds, &preset.held, 220.0)],
            ));
        }
        right.push(toggle(
            "keep",
            "処理が間に合わなくても一時停止しない",
            preset.keep,
            0,
            true,
        ));
        let deinterlace_ms = stage(At::Deinterlace).and_then(|s| s.ms);
        let picture_ms = stage(At::Picture).and_then(|s| s.ms);
        let budget = video.map_or(0.0, |v| v.budget);
        if (deinterlace_ms.is_some() || picture_ms.is_some()) && budget > 0.0 {
            let (d, p) = (deinterlace_ms.unwrap_or(0.0), picture_ms.unwrap_or(0.0));
            let over = d + p > budget;
            right.push(Node::Bar {
                label: String::from("負荷"),
                parts: vec![
                    ((d / budget) as f32, Tone::Muted),
                    (
                        (p / budget) as f32,
                        if over { Tone::Bad } else { Tone::Plain },
                    ),
                ],
                left: format!(
                    "合計 {:.1} ms（デインタレース {d:.1} ＋ フィルター {p:.1}）",
                    d + p
                ),
                right: format!("1 フレームの上限 {budget:.1} ms"),
            });
        } else {
            right.push(line(
                "負荷",
                vec![Inline::Text {
                    text: String::from("—"),
                    tone: Tone::Muted,
                }],
            ));
        }
        vec![Node::Columns {
            width: 200.0,
            left,
            right,
        }]
    }

    fn autos_nodes(&mut self, filter: &Filter, ctx: &Ctx) -> Vec<Node> {
        let names = |key: &str| channel_name(ctx, key);
        let last = filter.autos.len().saturating_sub(1);
        let rows = filter
            .autos
            .iter()
            .enumerate()
            .map(|(i, auto)| Row {
                id: i.to_string(),
                cells: vec![
                    Cell::muted((i + 1).to_string()),
                    Cell::text(auto.name.clone()),
                    if auto.conditions.is_empty() {
                        Cell::muted("条件がありません")
                    } else {
                        Cell::Chips {
                            chips: auto.conditions.iter().map(|c| c.summary(&names)).collect(),
                            joint: String::from(if auto.any { "または" } else { "かつ" }),
                        }
                    },
                    Cell::Choice {
                        id: format!("preset:{i}"),
                        options: preset_options(filter, &auto.preset),
                        value: auto.preset.clone(),
                    },
                    Cell::Buttons {
                        buttons: vec![
                            Button::new(format!("edit:{i}"), "編集…"),
                            Button::new(format!("up:{i}"), "上へ")
                                .icon("up")
                                .enabled(i > 0),
                            Button::new(format!("down:{i}"), "下へ")
                                .icon("down")
                                .enabled(i < last),
                            Button::new(format!("drop:{i}"), "削除").icon("remove"),
                        ],
                    },
                ],
                selected: false,
                dim: false,
            })
            .collect();
        vec![
            Node::Table {
                id: String::from("autos"),
                columns: vec![
                    Column::new("", 28.0),
                    Column::new("名前", 120.0),
                    Column::new("条件", 0.0),
                    Column::new("適用するプリセット", 140.0),
                    Column::new("", 160.0),
                ],
                rows,
                empty: String::from("自動選択がありません"),
            },
            line(
                "条件に一致しない場合",
                vec![choice(
                    "fallback",
                    preset_options(filter, &filter.fallback),
                    &filter.fallback,
                    200.0,
                )],
            ),
            Node::Buttons {
                buttons: vec![Button::new("new_auto", "＋ 自動選択の追加…")],
                right: false,
            },
            Node::Section {
                label: String::from("再生モード利用時"),
            },
            toggle(
                "playback",
                "再生モード中も自動選択を適用する",
                filter.playback,
                0,
                true,
            ),
        ]
    }

    fn shaders_nodes(&mut self, filter: &Filter) -> Vec<Node> {
        let mut kinds = vec![(String::new(), String::from("すべて"))];
        kinds.extend(KINDS.iter().map(|k| ((*k).to_owned(), (*k).to_owned())));
        kinds.push((UNSET.to_owned(), UNSET.to_owned()));
        let files = shader_files();
        let rows = files
            .iter()
            .filter(|file| self.kind.is_empty() || filter.describe(file).1 == self.kind)
            .map(|file| {
                let (name, kind) = filter.describe(file);
                let uses = filter.uses_of_shader(file);
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
                        Cell::muted(if uses == 0 {
                            String::from("—")
                        } else {
                            format!("{uses} 件")
                        }),
                        Cell::Buttons {
                            buttons: vec![
                                Button::new(format!("meta:{file}"), "編集…"),
                                Button::new(format!("delete:{file}"), "削除")
                                    .icon("remove")
                                    .enabled(uses == 0),
                            ],
                        },
                    ],
                    selected: false,
                    dim: false,
                }
            })
            .collect();
        let mut nodes = vec![
            Node::Buttons {
                buttons: vec![Button::new("import", "＋ 追加…")],
                right: true,
            },
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
                id: String::from("shaders"),
                columns: vec![
                    Column::new("名前", 0.0),
                    Column::new("種類", 100.0),
                    Column::new("プリセット", 76.0).right(),
                    Column::new("", 112.0),
                ],
                rows,
                empty: String::from("シェーダーがありません"),
            },
        ];
        if !self.error.is_empty() {
            nodes.push(note(self.error.clone(), Tone::Bad));
        }
        nodes
    }
}

fn notify_nodes(filter: &Filter) -> Vec<Node> {
    let n = &filter.notify;
    vec![
        Node::Section {
            label: String::from("プリセットの切り替え"),
        },
        toggle("preset", "通知する", n.preset, 0, true),
        toggle("rule", "一致した自動選択と条件", n.rule, 1, n.preset),
        toggle("chain", "フィルターの順序", n.chain, 1, n.preset),
        toggle("size", "入力と出力の解像度", n.size, 1, n.preset),
        toggle(
            "manual",
            "手動で切り替えたときも通知する",
            n.manual,
            0,
            n.preset,
        ),
        toggle(
            "quiet_same",
            "番組が変わってもプリセットが同じなら通知しない",
            n.quiet_same,
            0,
            n.preset,
        ),
        Node::Section {
            label: String::from("フィルターの状態"),
        },
        toggle("compile", "シェーダーコンパイル中", n.compile, 0, true),
        toggle("built", "準備完了", n.built, 0, true),
        toggle("paused", "一時停止", n.paused, 0, true),
        toggle("error", "エラー", n.error, 0, true),
        Node::Section {
            label: String::from("表示"),
        },
        line(
            "詳細",
            vec![choice(
                "lines",
                vec![
                    (String::from("3"), String::from("3 行まで")),
                    (String::from("1"), String::from("1 行まで")),
                    (String::from("0"), String::from("タイトルのみ")),
                ],
                &n.lines.to_string(),
                200.0,
            )],
        ),
        line(
            "表示時間",
            vec![choice(
                "extend",
                vec![
                    (String::from("true"), String::from("行数に応じて延長")),
                    (String::from("false"), String::from("3.5 秒")),
                ],
                &n.extend.to_string(),
                200.0,
            )],
        ),
    ]
}
