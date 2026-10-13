// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! スクリプトを実行する前の書き換え。boa 0.22 のインラインキャッシュが二度目の
//! `a.length = n` をただのスロット書きにして要素が消えないため、BML と同じ書き換えを通す。

pub fn rewrite_inline(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut from = 0;
    while let Some(at) = lower[from..].find("<script") {
        let open = from + at;
        let Some(open_end) = lower[open..].find('>') else {
            break;
        };
        let open_end = open + open_end + 1;
        let Some(close) = lower[open_end..].find("</script") else {
            break;
        };
        let close = open_end + close;
        let tag = &lower[open..open_end];
        out.push_str(&html[from..open_end]);
        let body = &html[open_end..close];
        if is_script(tag) && !tag.contains("src=") {
            out.push_str(&bml::runtime::script::rewrite_length(body));
        } else {
            out.push_str(body);
        }
        from = close;
    }
    out.push_str(&html[from..]);
    out
}

fn is_script(tag: &str) -> bool {
    let Some(at) = tag.find("type=") else {
        return true;
    };
    let rest = &tag[at + 5..];
    let value = rest.trim_start_matches(['"', '\'']);
    let value = value
        .split(['"', '\'', ' ', '>'])
        .next()
        .unwrap_or("")
        .trim();
    matches!(
        value,
        "" | "text/javascript" | "application/javascript" | "text/ecmascript" | "module"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_scripts_are_rewritten_and_the_rest_is_left_alone() {
        let html = "<html><script>a.length = 0;</script><script src=\"x.js\"></script><script type=\"text/template\">b.length = 0;</script><p>c.length = 0;</p></html>";
        assert_eq!(
            rewrite_inline(html),
            "<html><script>__bml_setlen(a, 0);</script><script src=\"x.js\"></script><script type=\"text/template\">b.length = 0;</script><p>c.length = 0;</p></html>"
        );
    }
}
