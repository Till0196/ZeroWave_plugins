// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! シェーダーファイルのテキスト：`//!` の行で区切られたブロックと、大きさの式。

use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct Block {
    pub headers: Vec<String>,
    pub body: String,
}

impl Block {
    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers.iter().find_map(|line| value_of(line, key))
    }

    pub fn all(&self, key: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter_map(|line| value_of(line, key))
            .collect()
    }

    pub fn has(&self, key: &str) -> bool {
        self.header(key).is_some()
    }
}

fn value_of<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(key)?;
    if rest.is_empty() {
        return Some("");
    }
    rest.starts_with(char::is_whitespace).then(|| rest.trim())
}

pub fn blocks(text: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut body_seen = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("//!") {
            // 本文の無いパス（`//!HOST`）の後ろの `//!PASS` も、新しいブロック。
            let another_pass = rest.trim_start().starts_with("PASS")
                && out
                    .last()
                    .is_some_and(|block| block.headers.iter().any(|h| h.starts_with("PASS")));
            if out.is_empty() || body_seen || another_pass {
                out.push(Block::default());
                body_seen = false;
            }
            out.last_mut()
                .expect("pushed")
                .headers
                .push(rest.trim().to_owned());
        } else if let Some(block) = out.last_mut() {
            if !line.trim().is_empty() {
                body_seen = true;
            }
            block.body.push_str(line);
            block.body.push('\n');
        }
    }
    out
}

/// 大きさの式（mpv の逆ポーランド記法）。`sizes` に無い名前は誤り。
pub fn rpn(expr: &str, sizes: &BTreeMap<String, [u32; 2]>) -> Result<f64, String> {
    let mut stack: Vec<f64> = Vec::new();
    for token in expr.split_whitespace() {
        let binary = |stack: &mut Vec<f64>, f: fn(f64, f64) -> f64| -> Result<(), String> {
            let b = stack.pop().ok_or("式の項が足りません。")?;
            let a = stack.pop().ok_or("式の項が足りません。")?;
            stack.push(f(a, b));
            Ok(())
        };
        match token {
            "+" => binary(&mut stack, |a, b| a + b)?,
            "-" => binary(&mut stack, |a, b| a - b)?,
            "*" => binary(&mut stack, |a, b| a * b)?,
            "/" => binary(&mut stack, |a, b| a / b)?,
            ">" => binary(&mut stack, |a, b| f64::from(u8::from(a > b)))?,
            "<" => binary(&mut stack, |a, b| f64::from(u8::from(a < b)))?,
            "=" => binary(&mut stack, |a, b| f64::from(u8::from((a - b).abs() < 1e-9)))?,
            "!" => {
                let a = stack.pop().ok_or("式の項が足りません。")?;
                stack.push(f64::from(u8::from(a == 0.0)));
            }
            _ => {
                if let Ok(number) = token.parse::<f64>() {
                    stack.push(number);
                    continue;
                }
                let (name, field) = token
                    .rsplit_once('.')
                    .ok_or_else(|| format!("式に使えない文字があります：{token}"))?;
                let size = sizes
                    .get(name)
                    .ok_or_else(|| format!("式の {name} の大きさが分かりません。"))?;
                let value = match field {
                    "w" | "width" => size[0],
                    "h" | "height" => size[1],
                    _ => return Err(format!("式に使えない文字があります：{token}")),
                };
                stack.push(f64::from(value));
            }
        }
    }
    match stack.as_slice() {
        [value] => Ok(*value),
        _ => Err(format!("式の結果が一つの値になりません：{expr}")),
    }
}

pub fn rpn_size(expr: &str, sizes: &BTreeMap<String, [u32; 2]>) -> Result<u32, String> {
    let value = rpn(expr, sizes)?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!("大きさが数値になりません：{expr}"));
    }
    Ok((value as u32).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_split_where_a_header_follows_a_body() {
        let text = "// 説明\n//!HOOK MAIN\n//!BIND HOOKED\nvec4 hook() {}\n\n//!TEXTURE lut\n//!SIZE 2 2\nff00\n";
        let found = blocks(text);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].header("HOOK"), Some("MAIN"));
        assert_eq!(found[0].all("BIND"), vec!["HOOKED"]);
        assert!(found[0].body.contains("hook()"));
        assert_eq!(found[1].header("TEXTURE"), Some("lut"));
        assert_eq!(found[1].header("SIZE"), Some("2 2"));
        assert_eq!(found[0].header("BINDX"), None);
    }

    #[test]
    fn the_size_expressions_read_like_mpv() {
        let sizes: BTreeMap<String, [u32; 2]> = [
            ("HOOKED".to_string(), [1440, 1080]),
            ("OUTPUT".to_string(), [3840, 2160]),
        ]
        .into();
        assert_eq!(rpn("HOOKED.w 2 *", &sizes), Ok(2880.0));
        assert_eq!(rpn("OUTPUT.w HOOKED.w / 1.4 >", &sizes), Ok(1.0));
        assert_eq!(rpn("HOOKED.w OUTPUT.w / 0.707106 < !", &sizes), Ok(0.0));
        assert_eq!(rpn("HOOKED.width HOOKED.height =", &sizes), Ok(0.0));
        assert!(rpn("LUMA.w", &sizes).is_err());
        assert!(rpn("1 2", &sizes).is_err());
        assert_eq!(rpn_size("HOOKED.h 2 /", &sizes), Ok(540));
    }
}
