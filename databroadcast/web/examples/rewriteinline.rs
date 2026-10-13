// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! HTML の中の `<script>` を [`web::script::rewrite_inline`] に通した結果を出す。

fn main() {
    let path = std::env::args().nth(1).expect("文書のパス");
    let html = std::fs::read_to_string(path).expect("読める");
    print!("{}", web::script::rewrite_inline(&html));
}
