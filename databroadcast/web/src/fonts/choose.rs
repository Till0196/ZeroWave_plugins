// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! BML のフォントを選ぶ。設定（`font_round` など）にフォントの名前かファイルがあればそれ、空なら
//! OS のフォントを名前で探す。名前から探して読むのは本体（`zw_font_v1`）で、guest はファイルを開けない。

use super::{Bml, Face, pick};

pub const ROUND: &str = "font_round";
pub const ROUND_BOLD: &str = "font_round_bold";
pub const SQUARE: &str = "font_square";
pub const SQUARE_BOLD: &str = "font_square_bold";
pub const MONO: &str = "font_mono";

/// 丸ゴシック。Windows と多くの Linux には無いので、そのときは角ゴシックで代わりにする。
const ROUND_CANDIDATES: &[&str] = &["Hiragino Maru Gothic ProN W4"];

/// 角ゴシック。英数字が半角の幅の物を先に。
const SQUARE_CANDIDATES: &[&str] = &[
    "Hiragino Sans W3",
    "Hiragino Kaku Gothic ProN W3",
    "BIZ UDGothic",
    "Yu Gothic Regular",
    "Meiryo",
    "MS Gothic",
    "Noto Sans Mono CJK JP",
    "Noto Sans CJK JP",
    "IPAexGothic",
    "IPAGothic",
];

/// 太字（丸・角とも）。macOS に太い丸ゴシックは無いので、角ゴシックの W6。
const BOLD_CANDIDATES: &[&str] = &[
    "Hiragino Sans W6",
    "Hiragino Kaku Gothic ProN W6",
    "BIZ UDGothic Bold",
    "Yu Gothic Bold",
    "Meiryo Bold",
    "Noto Sans Mono CJK JP Bold",
    "Noto Sans CJK JP Bold",
];

struct Found {
    face: Face,
    from: String,
}

pub fn load() -> Bml {
    let mut read = Reader::default();
    let round = read.chosen(ROUND, false);
    let square = read
        .chosen(SQUARE, false)
        .or_else(|| read.first(SQUARE_CANDIDATES, false));
    let round = round
        .or_else(|| read.first(ROUND_CANDIDATES, false))
        .or_else(|| square.as_ref().map(Found::again));
    let round_bold = read
        .chosen(ROUND_BOLD, true)
        .or_else(|| read.first(BOLD_CANDIDATES, true));
    let square_bold = read
        .chosen(SQUARE_BOLD, true)
        .or_else(|| read.first(BOLD_CANDIDATES, true));
    for (what, found) in [
        ("丸ゴシック", &round),
        ("太丸ゴシック", &round_bold),
        ("角ゴシック", &square),
        ("角ゴシック（太字）", &square_bold),
    ] {
        match found {
            Some(found) => {
                zwsdk::plug::info(&format!("{what}: {}（{}）", found.face.name, found.from))
            }
            None => zwsdk::plug::info(&format!("{what}: 無し")),
        }
    }
    let face = |found: Option<Found>| found.map(|found| found.face);
    Bml {
        round: face(round),
        round_bold: face(round_bold),
        square: face(square),
        square_bold: face(square_bold),
    }
}

/// 設定で選んだ等幅のフォント（BML の文字が丸・角に無いとき）。空なら本体が渡す物のまま。
pub fn mono() -> Option<Vec<u8>> {
    Reader::default()
        .chosen(MONO, false)
        .map(|found| found.face.bytes.as_ref().clone())
}

impl Found {
    fn again(&self) -> Self {
        Self {
            face: self.face.clone(),
            from: self.from.clone(),
        }
    }
}

#[derive(Default)]
struct Reader {
    files: Vec<(String, Option<std::sync::Arc<Vec<u8>>>)>,
    picked: Vec<((String, bool), Option<Face>)>,
}

impl Reader {
    /// 利用者が入れた物。太い物を求めて細い物しか無くても、選ばれた物を使う。
    fn chosen(&mut self, key: &str, bold: bool) -> Option<Found> {
        let value = zwsdk::plug::host::cfg(key)?;
        let value = value.trim().trim_matches('"').to_owned();
        if value.is_empty() {
            return None;
        }
        let bytes = lend_chosen(key, &value);
        let face = bytes
            .as_deref()
            .and_then(|bytes| pick(bytes, bold, None).or_else(|| pick(bytes, false, None)));
        if face.is_none() {
            zwsdk::plug::warn(&format!("{key}: {value} が読めない"));
        }
        Some(Found {
            face: face?,
            from: value,
        })
    }

    fn first(&mut self, candidates: &[&str], bold: bool) -> Option<Found> {
        candidates.iter().find_map(|name| {
            let face = self.pick(name, bold)?;
            Some(Found {
                face,
                from: (*name).to_owned(),
            })
        })
    }

    fn pick(&mut self, name: &str, bold: bool) -> Option<Face> {
        let key = (name.to_owned(), bold);
        if let Some((_, face)) = self.picked.iter().find(|(had, _)| *had == key) {
            return face.clone();
        }
        let bytes = self.read(name);
        let face = bytes.and_then(|bytes| pick(&bytes, bold, None));
        self.picked.push((key, face.clone()));
        face
    }

    fn read(&mut self, name: &str) -> Option<std::sync::Arc<Vec<u8>>> {
        if let Some((_, bytes)) = self.files.iter().find(|(had, _)| had == name) {
            return bytes.clone();
        }
        let bytes = zwsdk::plug::host::font(name).map(std::sync::Arc::new);
        self.files.push((name.to_owned(), bytes.clone()));
        bytes
    }
}

/// wasm では本体が設定の値を読む。手元（`ZW_NATIVE`）ではファイルなら読み、名前なら探す。
fn lend_chosen(key: &str, value: &str) -> Option<Vec<u8>> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = value;
        zwsdk::plug::host::font(key)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = key;
        std::fs::read(value)
            .ok()
            .or_else(|| zwsdk::plug::host::font(value))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn without_a_round_gothic_the_square_one_stands_in() {
        if !std::path::Path::new("C:\\Windows\\Fonts\\BIZ-UDGothicB.ttc").exists() {
            return;
        }
        let bml = super::load();
        let name = |face: &Option<super::Face>| face.as_ref().map(|face| face.name.clone());
        assert_eq!(name(&bml.round), name(&bml.square));
        assert_eq!(
            name(&bml.square).as_deref(),
            Some("BIZ UDGothic"),
            "英数字が半角の物が先"
        );
        assert_eq!(name(&bml.round_bold).as_deref(), Some("BIZ UDGothic"));
        assert_eq!(bml.bold(), (true, true));
    }
}
