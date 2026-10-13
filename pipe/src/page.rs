// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 設定ページ。上の切り替えで TS と MMT/TLV を分け、実行ファイル・引数・環境変数と、起動するコマンドを
//! 見せる。ここで決めるのは並びと状態のラベルのみ。

use zwsdk::plug::host;
use zwsdk::ui::{Ctx, Event, MenuItem, Node, Page, Reply, Tone};

use crate::{Keys, MOST_DEATHS, Shown, TLV, TS};

#[derive(Debug, Default)]
pub struct Settings {
    showing: String,
}

impl Settings {
    fn keys(&self) -> Keys {
        if self.showing == TLV.program { TLV } else { TS }
    }
}

fn note(text: impl Into<String>) -> Node {
    Node::Note {
        text: text.into(),
        tone: Tone::Muted,
    }
}

/// ページの要素の並び。`state` は現在の状態（`*_state` の値）。
fn nodes(keys: Keys, state: &str) -> Vec<Node> {
    vec![
        note("受けたストリームを、選んだプログラムに標準入力で渡し、標準出力を受け取ります。"),
        Node::Segments {
            id: String::from("kind"),
            options: [TS, TLV]
                .iter()
                .map(|k| (k.program.to_owned(), k.what.to_owned()))
                .collect(),
            value: keys.program.to_owned(),
        },
        Node::Program {
            key: keys.program.to_owned(),
            label: String::from("実行ファイル"),
            badge: Shown::of(state).badge(),
            hint: format!(
                "標準入力で {} を受け、標準出力へ出す実行ファイル",
                keys.what
            ),
        },
        note(format!(
            "止まったときは自動で起動し直します（{MOST_DEATHS} 回まで）。この欄にファイルをドロップしても入力できます。"
        )),
        Node::List {
            key: keys.args.to_owned(),
            label: String::from("引数"),
            hint: String::from("引数を入力して Enter"),
        },
        Node::Map {
            key: keys.env.to_owned(),
            label: String::from("環境変数"),
            empty: String::from("環境変数はありません"),
        },
        note(
            "値の $TMPDIR・${HOME} などは起動するときに展開します。左の切り替えで一時的に外せます。",
        ),
        Node::Command {
            program: keys.program.to_owned(),
            args: keys.args.to_owned(),
            env: keys.env.to_owned(),
            label: String::from("実行するコマンド"),
        },
    ]
}

impl zwsdk::plug::Ui for Settings {
    fn new() -> Self {
        Self::default()
    }

    fn menu(&mut self, _: &Ctx) -> Vec<MenuItem> {
        Vec::new()
    }

    fn pick(&mut self, _: &Ctx, _: &str) {}

    fn page(&mut self, _: &str, _: &Ctx) -> Page {
        let keys = self.keys();
        let state = host::cfg(keys.state).unwrap_or_default();
        Page {
            title: String::from("パイプ の設定"),
            size: Some([680.0, 680.0]),
            nodes: nodes(keys, &state),
            confirm: true,
            ..Page::default()
        }
    }

    fn event(&mut self, _: &str, _: &Ctx, event: &Event) -> Reply {
        if event.id == "kind" {
            self.showing = event.value.clone();
        }
        Reply::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zwsdk::plug::Ui;

    #[test]
    fn the_switch_chooses_which_keys_are_shown() {
        let mut settings = Settings::new();
        assert_eq!(settings.keys(), TS);
        settings.event(
            "main",
            &Ctx::default(),
            &Event {
                id: String::from("kind"),
                value: String::from("tlv"),
            },
        );
        assert_eq!(settings.keys(), TLV);
        let page = settings.page("main", &Ctx::default());
        assert!(page.confirm);
        assert!(
            page.nodes
                .iter()
                .any(|node| matches!(node, Node::Program { key, .. } if key == "tlv"))
        );
    }

    #[test]
    fn the_state_shows_as_a_badge() {
        let badge = |state: &str| {
            nodes(TS, state).into_iter().find_map(|node| match node {
                Node::Program { badge, .. } => badge,
                _ => None,
            })
        };
        assert_eq!(badge("").map(|b| b.text), None);
        assert_eq!(badge("running").map(|b| b.tone), Some(Tone::Good));
        assert_eq!(badge("restarting").map(|b| b.tone), Some(Tone::Warn));
        assert_eq!(badge("unused").map(|b| b.tone), Some(Tone::Muted));
        assert_eq!(badge("stopped").map(|b| b.tone), Some(Tone::Bad));
    }
}
