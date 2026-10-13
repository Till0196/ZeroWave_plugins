// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 自動選択の条件のページと、ジャンル・チャンネルを選ぶページ。

use super::*;

impl Menu {
    pub(super) fn rule_page(&mut self, ctx: &Ctx) -> Page {
        let presets = self.draft().clone();
        let Some((_, auto)) = &self.rule else {
            return Page {
                title: String::from("自動選択の編集"),
                footer: vec![Button::new("cancel", "閉じる")],
                ..Page::default()
            };
        };
        let names = |key: &str| channel_name(ctx, key);
        let mut nodes = vec![
            line("名前", vec![input("name", &auto.name, 0.0)]),
            line(
                "一致条件",
                vec![choice(
                    "any",
                    vec![
                        (String::from("all"), String::from("すべての条件に一致")),
                        (String::from("any"), String::from("いずれかの条件に一致")),
                    ],
                    if auto.any { "any" } else { "all" },
                    0.0,
                )],
            ),
            Node::Section {
                label: String::from("条件"),
            },
        ];
        let kinds: Vec<(String, String)> = Kind::ALL
            .iter()
            .map(|k| (k.name().to_owned(), k.label().to_owned()))
            .collect();
        for (j, condition) in auto.conditions.iter().enumerate() {
            if j > 0 {
                nodes.push(note(if auto.any { "または" } else { "かつ" }, Tone::Muted));
            }
            let Some(kind) = Kind::parse(&condition.kind) else {
                continue;
            };
            let mut items = vec![choice(
                &format!("kind:{j}"),
                kinds.clone(),
                kind.name(),
                120.0,
            )];
            if kind.ops().len() > 1 {
                items.push(choice(
                    &format!("op:{j}"),
                    kind.ops()
                        .iter()
                        .map(|(v, l)| ((*v).to_owned(), (*l).to_owned()))
                        .collect(),
                    &condition.op,
                    100.0,
                ));
            }
            let first = condition.values.first().cloned().unwrap_or_default();
            match kind {
                Kind::Genre | Kind::Channel => {
                    let page = if kind == Kind::Genre {
                        "genre"
                    } else {
                        "channel"
                    };
                    items.push(Inline::Button(Button::new(
                        format!("{page}:{j}"),
                        "＋ 追加…",
                    )));
                    items.push(Inline::Button(
                        Button::new(format!("drop:{j}"), "削除").icon("remove"),
                    ));
                    items.push(Inline::Chips {
                        id: format!("chips:{j}"),
                        chips: condition
                            .values
                            .iter()
                            .map(|v| Chip {
                                id: v.clone(),
                                label: if kind == Kind::Genre {
                                    presets::genre_label(v)
                                } else {
                                    names(v)
                                },
                                on: true,
                            })
                            .collect(),
                        removable: true,
                    });
                    nodes.push(line("", items));
                    continue;
                }
                Kind::Network => {
                    items.push(Inline::Button(
                        Button::new(format!("drop:{j}"), "削除").icon("remove"),
                    ));
                    let mut networks = ctx.networks.clone();
                    for v in &condition.values {
                        if !networks.contains(v) {
                            networks.push(v.clone());
                        }
                    }
                    items.push(Inline::Chips {
                        id: format!("net:{j}"),
                        chips: networks
                            .into_iter()
                            .map(|n| Chip {
                                on: condition.values.contains(&n),
                                id: n.clone(),
                                label: n,
                            })
                            .collect(),
                        removable: false,
                    });
                    nodes.push(line("", items));
                    continue;
                }
                Kind::Resolution => items.push(choice(
                    &format!("value:{j}"),
                    Size::ALL
                        .iter()
                        .map(|s| (s.name().to_owned(), s.label().to_owned()))
                        .collect(),
                    &first,
                    180.0,
                )),
                Kind::Scan => items.push(choice(
                    &format!("value:{j}"),
                    vec![
                        (String::from("interlace"), String::from("インタレース")),
                        (String::from("progressive"), String::from("プログレッシブ")),
                    ],
                    &first,
                    160.0,
                )),
                Kind::Structure => items.push(choice(
                    &format!("value:{j}"),
                    ["field", "frame"]
                        .iter()
                        .map(|v| ((*v).to_owned(), presets::structure_label(v).to_owned()))
                        .collect(),
                    &first,
                    180.0,
                )),
                Kind::Source => items.push(choice(
                    &format!("value:{j}"),
                    vec![
                        (String::from("broadcast"), String::from("放送")),
                        (String::from("recording"), String::from("録画")),
                    ],
                    &first,
                    160.0,
                )),
                Kind::Title => items.push(input(
                    &format!("words:{j}"),
                    &condition.values.join("、"),
                    220.0,
                )),
                Kind::Time => {
                    let at = |k: usize| condition.values.get(k).cloned().unwrap_or_default();
                    items.push(input(&format!("from:{j}"), &at(0), 70.0));
                    items.push(Inline::Text {
                        text: String::from("〜"),
                        tone: Tone::Muted,
                    });
                    items.push(input(&format!("to:{j}"), &at(1), 70.0));
                    items.push(choice(
                        &format!("days:{j}"),
                        ["every", "weekday", "weekend"]
                            .iter()
                            .map(|d| ((*d).to_owned(), presets::day_label(d).to_owned()))
                            .collect(),
                        &at(2),
                        90.0,
                    ));
                }
            }
            items.push(Inline::Button(
                Button::new(format!("drop:{j}"), "削除").icon("remove"),
            ));
            nodes.push(line("", items));
        }
        let mut add = vec![(String::new(), String::from("＋ 条件の追加"))];
        add.extend(kinds);
        nodes.push(line("", vec![choice("add", add, "", 200.0)]));
        nodes.push(Node::Section {
            label: String::from("適用するプリセット"),
        });
        nodes.push(line(
            "",
            vec![choice(
                "preset",
                preset_options(&presets, &auto.preset),
                &auto.preset,
                220.0,
            )],
        ));
        Page {
            title: String::from("自動選択の編集"),
            size: Some([680.0, 560.0]),
            nodes,
            footer: vec![
                Button::new("save", "保存").primary(),
                Button::new("cancel", "キャンセル"),
            ],
            ..Page::default()
        }
    }

    pub(super) fn rule_event(&mut self, _ctx: &Ctx, event: &Event) -> Reply {
        match event.id.as_str() {
            "save" => {
                if let Some((at, auto)) = self.rule.take() {
                    let draft = self.draft();
                    match at.filter(|&i| i < draft.autos.len()) {
                        Some(i) => draft.autos[i] = auto,
                        None => draft.autos.push(auto),
                    }
                }
                return close();
            }
            "cancel" => {
                self.rule = None;
                return close();
            }
            _ => {}
        }
        let Some((_, auto)) = self.rule.as_mut() else {
            return close();
        };
        let value = event.value.as_str();
        let (head, j) = match event.id.split_once(':') {
            Some((head, rest)) => (head, rest.parse::<usize>().ok()),
            None => (event.id.as_str(), None),
        };
        match (head, j) {
            ("name", None) if !value.trim().is_empty() => auto.name = value.trim().to_owned(),
            ("any", None) => auto.any = value == "any",
            ("preset", None) => auto.preset = value.to_owned(),
            ("add", None) => {
                if let Some(kind) = Kind::parse(value) {
                    auto.conditions.push(kind.fresh());
                }
            }
            ("genre", Some(j)) => return open(format!("genre:{j}")),
            ("channel", Some(j)) => return open(format!("channel:{j}")),
            (head, Some(j)) if j < auto.conditions.len() => {
                let condition = &mut auto.conditions[j];
                match head {
                    "drop" => {
                        auto.conditions.remove(j);
                    }
                    "kind" => {
                        if let Some(kind) = Kind::parse(value)
                            && kind.name() != condition.kind
                        {
                            *condition = kind.fresh();
                        }
                    }
                    "op" => condition.op = value.to_owned(),
                    "value" => condition.values = vec![value.to_owned()],
                    "chips" => {
                        if let Some(gone) = value.strip_prefix("remove:") {
                            condition.values.retain(|v| v != gone);
                        }
                    }
                    "net" => {
                        if condition.values.iter().any(|v| v == value) {
                            condition.values.retain(|v| v != value);
                        } else {
                            condition.values.push(value.to_owned());
                        }
                    }
                    "words" => {
                        condition.values = value
                            .split(['、', ','])
                            .map(str::trim)
                            .filter(|w| !w.is_empty())
                            .map(str::to_owned)
                            .collect();
                    }
                    "from" | "to" | "days" => {
                        let k = match head {
                            "from" => 0,
                            "to" => 1,
                            _ => 2,
                        };
                        let fallback = Kind::Time.fresh().values;
                        while condition.values.len() < 3 {
                            let n = condition.values.len();
                            condition.values.push(fallback[n].clone());
                        }
                        if k == 2 || presets::clock(value).is_some() {
                            condition.values[k] = value.trim().to_owned();
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Reply::default()
    }

    pub(super) fn pick_values(&mut self, j: usize, event: &Event) -> Reply {
        if event.id == "close" {
            return close();
        }
        if let Some(value) = event.id.strip_prefix("v:")
            && let Some((_, auto)) = self.rule.as_mut()
            && let Some(condition) = auto.conditions.get_mut(j)
        {
            let on = event.value == "true";
            let has = condition.values.iter().any(|v| v == value);
            if on && !has {
                condition.values.push(value.to_owned());
            } else if !on {
                condition.values.retain(|v| v != value);
            }
        }
        Reply::default()
    }

    pub(super) fn genre_page(&self, j: usize) -> Page {
        let values = self
            .rule
            .as_ref()
            .and_then(|(_, a)| a.conditions.get(j))
            .map(|c| c.values.clone())
            .unwrap_or_default();
        let has = |value: &str| values.iter().any(|v| v == value);
        let mut rows = Vec::new();
        for &large in &crate::genre::LARGES {
            let value = format!("{large:x}");
            rows.push(Row {
                id: value.clone(),
                cells: vec![
                    Cell::Toggle {
                        id: format!("v:{value}"),
                        on: has(&value),
                    },
                    Cell::text(crate::genre::large_name(large)),
                ],
                selected: false,
                dim: false,
            });
            let middles = crate::genre::middles(large);
            for (middle, name) in middles
                .iter()
                .enumerate()
                .map(|(m, n)| (m as u8, *n))
                .chain((large <= 0xb).then_some((0xf, "その他")))
            {
                let value = format!("{large:x}.{middle:x}");
                rows.push(Row {
                    id: value.clone(),
                    cells: vec![
                        Cell::Toggle {
                            id: format!("v:{value}"),
                            on: has(&value),
                        },
                        Cell::muted(format!("　　{name}")),
                    ],
                    selected: false,
                    dim: false,
                });
            }
        }
        Page {
            title: String::from("ジャンルを選ぶ"),
            size: Some([440.0, 600.0]),
            nodes: vec![Node::Table {
                id: String::from("genres"),
                columns: vec![Column::new("", 32.0), Column::new("ジャンル", 0.0)],
                rows,
                empty: String::new(),
            }],
            footer: vec![Button::new("close", "閉じる")],
            ..Page::default()
        }
    }

    pub(super) fn channel_page(&self, ctx: &Ctx, j: usize) -> Page {
        let values = self
            .rule
            .as_ref()
            .and_then(|(_, a)| a.conditions.get(j))
            .map(|c| c.values.clone())
            .unwrap_or_default();
        let mut keys: Vec<(String, String)> = ctx
            .channels
            .iter()
            .map(|c| (channel_key(c.network, c.service), c.name.clone()))
            .collect();
        for value in &values {
            if !keys.iter().any(|(k, _)| k == value) {
                keys.push((value.clone(), value.clone()));
            }
        }
        let rows = keys
            .into_iter()
            .map(|(key, name)| Row {
                id: key.clone(),
                cells: vec![
                    Cell::Toggle {
                        id: format!("v:{key}"),
                        on: values.contains(&key),
                    },
                    Cell::text(name),
                    Cell::muted(key),
                ],
                selected: false,
                dim: false,
            })
            .collect();
        Page {
            title: String::from("チャンネルを選ぶ"),
            size: Some([480.0, 600.0]),
            nodes: vec![Node::Table {
                id: String::from("channels"),
                columns: vec![
                    Column::new("", 32.0),
                    Column::new("チャンネル", 0.0),
                    Column::new("", 100.0),
                ],
                rows,
                empty: String::from("チャンネルがありません"),
            }],
            footer: vec![Button::new("close", "閉じる")],
            ..Page::default()
        }
    }
}
