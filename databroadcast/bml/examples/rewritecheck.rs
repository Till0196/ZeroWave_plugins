// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! BML の文書か式を [`bml::runtime::script::rewrite`] に通した結果を出す。
//! `ZW_LENGTH_ONLY=1` なら `length` だけの書き換え（MMT・通信のアプリ向け）。

fn main() {
    let arg = std::env::args().nth(1).expect("path or code");
    let src = if std::path::Path::new(&arg).exists() {
        let bytes = std::fs::read(&arg).expect("read");
        let text = if std::env::var_os("ZW_LENGTH_ONLY").is_some() {
            String::from_utf8_lossy(&bytes).into_owned()
        } else {
            bml::media::text::decode(&bytes)
        };
        let start = text.find("<script><![CDATA[").map(|i| i + 17).unwrap_or(0);
        let end = text[start..]
            .find("]]></script>")
            .map(|i| start + i)
            .unwrap_or(text.len());
        text[start..end].to_owned()
    } else {
        arg
    };
    if std::env::var_os("ZW_LENGTH_ONLY").is_some() {
        print!("{}", bml::runtime::script::rewrite_length(&src));
    } else {
        print!("{}", bml::runtime::script::rewrite(&src));
    }
}
