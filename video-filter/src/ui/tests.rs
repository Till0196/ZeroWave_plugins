// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 映像フィルターのウィンドウのテスト。

use super::*;

#[test]
fn the_window_saves_only_on_ok() {
    let mut menu = <Menu as zwsdk::plug::Ui>::new();
    let ctx = Ctx::default();
    let page = <Menu as zwsdk::plug::Ui>::page(&mut menu, "main", &ctx);
    assert_eq!(page.status, "変更はありません");
    let event = |id: &str| Event {
        id: id.to_owned(),
        value: String::new(),
    };
    <Menu as zwsdk::plug::Ui>::event(&mut menu, "main", &ctx, &event("new"));
    assert!(load().presets.is_empty(), "まだ書かない");
    let page = <Menu as zwsdk::plug::Ui>::page(&mut menu, "main", &ctx);
    assert_eq!(page.status, "保存されていない変更 1 件");
    let reply = <Menu as zwsdk::plug::Ui>::event(&mut menu, "main", &ctx, &event("ok"));
    assert!(reply.close);
    assert_eq!(load().presets.len(), 1);
}

#[test]
fn the_panel_switches_presets() {
    let mut menu = <Menu as zwsdk::plug::Ui>::new();
    let ctx = Ctx::default();
    let page = <Menu as zwsdk::plug::Ui>::page(&mut menu, "filter", &ctx);
    let labels: Vec<&str> = page
        .nodes
        .iter()
        .filter_map(|node| match node {
            Node::Line { label, .. } => Some(label.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, ["プリセット", "処理時間", "パス"]);
    let Node::Line { items, .. } = &page.nodes[0] else {
        panic!("{:?}", page.nodes[0]);
    };
    assert!(
        matches!(&items[0], Inline::Chips { id, chips, .. }
            if id == "preset" && chips.first().is_some_and(|c| c.id == "auto" && c.on)
                && chips.last().is_some_and(|c| c.id == "none")),
        "{items:?}"
    );
    let event = |id: &str, value: &str| Event {
        id: id.to_owned(),
        value: value.to_owned(),
    };
    let reply = <Menu as zwsdk::plug::Ui>::event(&mut menu, "filter", &ctx, &event("settings", ""));
    assert_eq!(reply.open.as_deref(), Some("main"));
    <Menu as zwsdk::plug::Ui>::event(&mut menu, "filter", &ctx, &event("preset", "none"));
    assert!(load().manual.is_none());
}
