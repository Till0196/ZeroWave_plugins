// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 自前の書式：WGSL に `//!` の指示行を付けたファイル。デインタレースのフックで読めるのはこれだけ。
//! 指示行は OUTPUTS・RETIME・TEXTURE など、パスごとに PASS・READS・WRITES・COMPUTE・HOST など。
//! 最初の `//!PASS` より前がファイル全体の設定。

use zwsdk::video::{Kind, Pass, Source, Texture, ident};

use crate::build::{Builder, declare, sizes};
use crate::text::{blocks, rpn_size};

pub fn unit(builder: &mut Builder<'_>, file: &str, text: &str, first: bool) -> Result<(), String> {
    let what = |e: String| format!("{file}：{e}");
    let in_pass = |pass: &str, e: String| format!("{file}（{pass}）：{e}");
    let found = blocks(text);
    // 設定の行は最初の `//!PASS` と同じブロックに入ることがあるので、全部のブロックから読む。
    let setup: Vec<_> = found.iter().collect();
    let passes: Vec<_> = found.iter().filter(|b| b.has("PASS")).collect();
    let input = builder.main.clone();
    let sizes = sizes(builder.env, input.1);

    // RETIME はチェーンの途中でもよい。そのときは入力に一つ前の中身も持たせ、
    // `picture:1` をそれに読み替える。
    let retimes = setup.iter().any(|block| block.has("RETIME"));
    let midway = retimes && input.0 != "picture";
    if retimes {
        builder.graph.retime = builder.retime;
        builder.graph.retime_from = builder.graph.passes.len();
        if midway && let Some(texture) = builder.graph.textures.get_mut(&input.0) {
            texture.previous = true;
        }
    }
    let mut output_size = input.1;
    let mut textures: Vec<(String, String)> = Vec::new();
    for block in &setup {
        if let Some(n) = block.header("OUTPUTS") {
            let n: u32 = n
                .parse()
                .map_err(|_| what("OUTPUTS が数値ではありません。".into()))?;
            builder.graph.outputs = builder.graph.outputs.max(n);
        }
        if let Some(n) = block.header("HISTORY") {
            let n: u32 = n
                .parse()
                .map_err(|_| what("HISTORY が数値ではありません。".into()))?;
            builder.graph.history = builder.graph.history.max(n);
        }
        if let Some(frames) = block.header("FRAMES") {
            let numbers: Vec<i32> = frames
                .split_whitespace()
                .filter_map(|n| n.parse().ok())
                .collect();
            match numbers.as_slice() {
                [lo, hi] if first => builder.graph.frames = [*lo, *hi],
                [_, _] => {}
                _ => return Err(what("FRAMES には二つの数値を指定してください。".into())),
            }
        }
        if let Some(expr) = block.header("OUTPUT") {
            output_size = two(expr, &sizes).map_err(what)?;
        }
        for line in block.all("TEXTURE") {
            let words: Vec<&str> = line.split_whitespace().collect();
            let name = *words
                .first()
                .ok_or_else(|| what("TEXTURE に名前がありません。".into()))?;
            let flags = ["keep", "nearest"];
            let mut format = String::from("rgba16float");
            let mut rest: Vec<&str> = Vec::new();
            let (mut keep, mut linear) = (false, true);
            for word in &words[1..] {
                match *word {
                    "keep" => keep = true,
                    "nearest" => linear = false,
                    w if zwsdk::video::format_name(w).is_some() => format = w.to_owned(),
                    w if !flags.contains(&w) => rest.push(w),
                    _ => {}
                }
            }
            let size = two(&rest.join(" "), &sizes).map_err(what)?;
            let graph_name = format!("u{}_{}", builder.unit, ident(name));
            builder.graph.textures.insert(
                graph_name.clone(),
                Texture {
                    size: vec![size[0], size[1]],
                    format,
                    linear,
                    keep,
                    previous: false,
                    data: None,
                },
            );
            textures.push((name.to_owned(), graph_name));
        }
    }
    if passes.is_empty() {
        return Err(what("//!PASS がありません。".into()));
    }

    let out = builder.texture(&format!("u{}_out", builder.unit), output_size);
    for block in passes {
        let name = block.header("PASS").unwrap_or("pass").to_owned();
        let reads: Vec<&str> = block
            .header("READS")
            .map(|r| r.split_whitespace().collect())
            .unwrap_or_default();
        let writes = block.header("WRITES").unwrap_or("output");
        let compute = match block.header("COMPUTE") {
            None => None,
            Some(args) => {
                let n: Vec<u32> = args
                    .split_whitespace()
                    .filter_map(|n| n.parse().ok())
                    .collect();
                match n.as_slice() {
                    [w, h] => Some([*w, *h]),
                    _ => {
                        return Err(in_pass(
                            &name,
                            "COMPUTE には二つの数値を指定してください。".into(),
                        ));
                    }
                }
            }
        };
        let map = |read: &str| -> Result<String, String> {
            if let Some((_, graph_name)) = textures.iter().find(|(n, _)| n == read) {
                return Ok(graph_name.clone());
            }
            match Source::of(read) {
                Source::Picture(0) => Ok(input.0.clone()),
                Source::Picture(1) if midway => Ok(format!("{}:1", input.0)),
                Source::Picture(_) if midway => {
                    Err(String::from("途中のパスで読めるのは picture:1 までです。"))
                }
                Source::Picture(_) | Source::Output(_) => Ok(read.to_owned()),
                Source::Named(_) => Err(format!("{read} が宣言されていません。")),
            }
        };
        let graph_reads: Vec<String> = reads
            .iter()
            .map(|r| map(r))
            .collect::<Result<_, _>>()
            .map_err(|e| in_pass(&name, e))?;
        let (graph_writes, out_format) = if writes == "output" {
            (out.clone(), String::from("rgba16float"))
        } else {
            let graph_name = textures
                .iter()
                .find(|(n, _)| n == writes)
                .map(|(_, g)| g.clone())
                .ok_or_else(|| {
                    in_pass(
                        &name,
                        format!("書き込み先の {writes} が宣言されていません。"),
                    )
                })?;
            let format = builder.graph.textures[&graph_name].format.clone();
            (
                graph_name,
                zwsdk::video::format_name(&format)
                    .unwrap_or("rgba16float")
                    .to_owned(),
            )
        };
        if let Some(job) = block.header("HOST") {
            builder.push(Pass {
                name,
                kind: Kind::Host,
                source: job.trim().to_owned(),
                entry: String::from("main"),
                reads: graph_reads,
                writes: graph_writes,
                block: [0, 0],
                process: block
                    .header("PROGRAM")
                    .map(|program| zwsdk::video::Process {
                        program: program.to_owned(),
                        args: block.all("ARG").into_iter().map(str::to_owned).collect(),
                        dir: block.header("DIR").unwrap_or_default().to_owned(),
                        path: block.all("PATH").into_iter().map(str::to_owned).collect(),
                        ways: block.all("WAY").into_iter().map(str::to_owned).collect(),
                    }),
                ..Pass::default()
            });
            continue;
        }
        let declared: Vec<(String, &'static str)> = reads
            .iter()
            .map(|r| (ident(r), "texture_2d<f32>"))
            .collect();
        let mut source = declare(&declared, compute.map(|_| out_format.as_str()));
        source.push_str(&block.body);
        builder.push(Pass {
            name,
            kind: if compute.is_some() {
                Kind::Compute
            } else {
                Kind::Fragment
            },
            source,
            entry: String::from("main"),
            reads: graph_reads,
            writes: graph_writes,
            block: compute.unwrap_or([0, 0]),
            ..Pass::default()
        });
    }
    builder.main = (out, output_size);
    Ok(())
}

fn two(
    text: &str,
    sizes: &std::collections::BTreeMap<String, [u32; 2]>,
) -> Result<[u32; 2], String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    for cut in 1..words.len() {
        let (a, b) = words.split_at(cut);
        if let (Ok(w), Ok(h)) = (rpn_size(&a.join(" "), sizes), rpn_size(&b.join(" "), sizes)) {
            return Ok([w, h]);
        }
    }
    Err(format!("大きさの式を読み取れません：{text}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn two_size_expressions_are_found_in_one_line() {
        let sizes: BTreeMap<String, [u32; 2]> = [
            ("input".to_string(), [1440, 1080]),
            ("target".to_string(), [1920, 1080]),
        ]
        .into();
        assert_eq!(two("input.w 2 * input.h 2 *", &sizes), Ok([2880, 2160]));
        assert_eq!(two("target.w target.h", &sizes), Ok([1920, 1080]));
        assert_eq!(two("640 360", &sizes), Ok([640, 360]));
        assert!(two("input.w", &sizes).is_err());
    }
}
