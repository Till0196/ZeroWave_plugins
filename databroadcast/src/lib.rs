// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! データ放送（BML・HTML5・Hybridcast）のプラグイン。中身は [`Engine`]。

pub mod engine;
pub mod romsound;

pub use engine::Engine;

impl zwsdk::plug::Layer for Engine {
    fn new(config: &zwsdk::layer::Config) -> Self {
        Engine::new(config)
    }

    fn message(&mut self, message: zwsdk::layer::Message, bytes: &[u8]) {
        Engine::message(self, message, bytes);
    }

    fn tick(&mut self) -> zwsdk::layer::Ticked {
        Engine::tick(self)
    }

    fn frame(&mut self) -> Option<zwsdk::plug::Painted> {
        self.take_frame()
    }

    fn events(&mut self) -> (Vec<zwsdk::layer::Event>, Vec<Vec<u8>>) {
        self.take_events()
    }
}

#[cfg(target_arch = "wasm32")]
zwsdk::export_layer!(
    Engine,
    name = "データ放送",
    version = "0.1.1",
    layer = r#"{"at":"databroadcast","wants":["streams","captions","keys","clock","area","broadcast","popup"]}"#,
    // 設定はこのプラグインが持つ（`plugins/databroadcast.toml`）。
    config = r#"[
        {"key":"net","kind":"bool","default":"false",
         "label":"通信を許可する"},
        {"key":"net_local","kind":"bool","default":"false",
         "label":"LAN 内やこの PC への通信を許可する"},
        {"key":"nvram","kind":"bool","default":"true",
         "label":"受信機の記憶領域 (NVRAM) をファイルに保存する"},
        {"key":"card_id","kind":"text","default":"","label":"カードID:",
         "hint":"(無し) カードの 20 桁。番組が読み取り、通信で送ることがあります"},
        {"key":"user_agent","kind":"text","default":"","label":"BML:",
         "hint":"(なし) 入力したときだけ、この文字列をそのまま送ります","section":"ユーザーエージェント"},
        {"key":"user_agent_hybridcast","kind":"text","default":"","label":"Hybridcast:",
         "hint":"(なし) AIT で起動したアプリ"},
        {"key":"user_agent_apab","kind":"text","default":"","label":"A-PAB:",
         "hint":"(なし) MH-AIT で起動したアプリ"},
        {"key":"browser_name","kind":"text","default":"","label":"名前:","hint":"(BRW9K) 英大文字と数字 20 字まで","section":"ブラウザ"},
        {"key":"browser_version","kind":"text","default":"","label":"バージョン:","hint":"(1.0) 主.副"},
        {"key":"font_round","kind":"font","default":"","label":"丸ゴシック:",
         "hint":"(自動) フォント名かファイル","section":"フォント"},
        {"key":"font_round_bold","kind":"font","default":"","label":"太丸ゴシック:",
         "hint":"(自動) 空欄なら太字のフォントを探します"},
        {"key":"font_square","kind":"font","default":"","label":"角ゴシック:",
         "hint":"(自動) フォント名かファイル"},
        {"key":"font_square_bold","kind":"font","default":"","label":"角ゴシック (太字):",
         "hint":"(自動) 空欄なら太字のフォントを探します"},
        {"key":"font_mono","kind":"font","default":"","label":"代替:",
         "hint":"(自動) 上のフォントに無い文字に使う等幅のフォント"},
        {"key":"trace","kind":"bool","default":"false",
         "label":"BML のトレースをログに出力する","section":"ログ"}
    ]"#
);

#[cfg(test)]
mod tests {
    /// wasm でしか展開しないので、ソースから読む。
    fn declared() -> Vec<zwsdk::host::Setting> {
        let source = include_str!("lib.rs");
        let open = "config = r#\"";
        let start = source.find(open).expect("宣言") + open.len();
        let end = start + source[start..].find("\"#").expect("宣言の終わり");
        zwsdk::ui::from_json(source[start..end].as_bytes()).expect("宣言が読める")
    }

    #[test]
    fn the_fonts_are_files_chosen_by_the_user_and_automatic_when_empty() {
        let settings = declared();
        for key in [
            web::fonts::choose::ROUND,
            web::fonts::choose::ROUND_BOLD,
            web::fonts::choose::SQUARE,
            web::fonts::choose::SQUARE_BOLD,
            web::fonts::choose::MONO,
        ] {
            let setting = settings
                .iter()
                .find(|setting| setting.key == key)
                .unwrap_or_else(|| panic!("{key} を宣言していない"));
            assert_eq!(setting.kind, "font", "{key}");
            assert_eq!(setting.default, "", "{key} の既定は自動");
        }
        let first = settings
            .iter()
            .position(|setting| setting.key == web::fonts::choose::ROUND)
            .unwrap();
        assert_eq!(settings[first].section, "フォント");
    }
}
