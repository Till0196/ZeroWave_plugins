// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 処理のグラフ（`zwsdk::video::Graph`）を組み立てる。続いた mpv のファイルは一つの単位
//! （中はフック先の順）、WGSL は一つずつが単位。同じパスで読み書きできないので、
//! テクスチャの名前は書くたびに新しくする（`main_1`、`main_2` …）。

use std::collections::BTreeMap;

use zwsdk::video::{self as zv, Env, Graph, Kind, Pass, Texture};

pub fn declare(reads: &[(String, &'static str)], out: Option<&str>) -> String {
    let mut s = String::from(zv::ZW_WGSL);
    s.push_str("@group(0) @binding(0) var<uniform> zw: Zw;\n");
    for (i, (name, kind)) in reads.iter().enumerate() {
        s.push_str(&format!(
            "@group(0) @binding({}) var {name}: {kind};\n@group(0) @binding({}) var {name}_smp: sampler;\n",
            1 + 2 * i,
            2 + 2 * i
        ));
    }
    if let Some(format) = out {
        s.push_str(&format!(
            "@group(0) @binding({}) var zw_out: texture_storage_2d<{format}, write>;\n",
            1 + 2 * reads.len()
        ));
    }
    s
}

#[derive(Debug)]
pub struct Builder<'e> {
    pub env: &'e Env,
    pub graph: Graph,
    pub main: (String, [u32; 2]),
    counter: usize,
    pub unit: usize,
    pub retime: f64,
}

impl<'e> Builder<'e> {
    pub fn new(env: &'e Env) -> Self {
        Self {
            env,
            graph: Graph {
                frames: [-1, 1],
                ..Graph::default()
            },
            main: (String::from("picture"), env.input),
            counter: 0,
            unit: 0,
            retime: if env.display > 1.0 { env.display } else { 60.0 },
        }
    }

    pub fn texture(&mut self, base: &str, size: [u32; 2]) -> String {
        self.texture_as(base, size, "rgba16float")
    }

    pub fn texture_as(&mut self, base: &str, size: [u32; 2], format: &str) -> String {
        self.counter += 1;
        let name = format!("{}_{}", zv::ident(base).to_lowercase(), self.counter);
        self.graph.textures.insert(
            name.clone(),
            Texture {
                size: vec![size[0], size[1]],
                format: format.to_owned(),
                linear: true,
                keep: false,
                previous: false,
                data: None,
            },
        );
        name
    }

    pub fn push(&mut self, pass: Pass) {
        self.graph.passes.push(pass);
    }

    pub fn simple(&mut self, name: &str, reads: &[&str], body: &str, writes: &str) {
        let declared: Vec<(String, &'static str)> = (0..reads.len())
            .map(|i| (format!("r{i}"), "texture_2d<f32>"))
            .collect();
        let mut source = declare(&declared, None);
        source.push_str(&format!(
            "@fragment\nfn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {{\n{body}\n}}\n"
        ));
        self.push(Pass {
            name: name.to_owned(),
            kind: Kind::Fragment,
            source,
            entry: String::from("main"),
            reads: reads.iter().map(|r| (*r).to_owned()).collect(),
            writes: writes.to_owned(),
            ..Pass::default()
        });
    }

    pub fn scale(&mut self, size: [u32; 2]) {
        if self.main.1 == size {
            return;
        }
        let from = self.main.0.clone();
        let to = self.texture("scaled", size);
        self.simple("scale", &[&from], COPY.trim_end(), &to);
        self.main = (to, size);
    }

    pub fn finish(mut self) -> Graph {
        if self.graph.passes.is_empty() {
            return Graph::default();
        }
        let (main, size) = self.main.clone();
        let last_writes_main = self
            .graph
            .passes
            .last()
            .is_some_and(|pass| pass.writes == main && main != "picture");
        if last_writes_main {
            self.graph.textures.remove(&main);
            if let Some(pass) = self.graph.passes.last_mut() {
                pass.writes = String::from("output");
            }
        } else {
            self.simple("copy", &[&main], COPY.trim_end(), "output");
        }
        self.graph.output = size;
        self.graph
    }
}

pub const BREAK: &str = "|";

const COPY: &str = include_str!("wgsl/copy.wgsl");

/// 名前の並び。`|` は単位の区切り（[`BREAK`]）。mpv の書式は単位の中をフック先の順に
/// 並べ替えるので、拡大より先にノイズを取りたいときは `ノイズ取り.glsl | 拡大.hook` と区切る。
pub fn chain_of(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, part) in text.split(BREAK).enumerate() {
        if i > 0 && out.last().is_some_and(|last: &String| last != BREAK) {
            out.push(BREAK.to_owned());
        }
        out.extend(
            part.split([',', '\n', ';'])
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
        );
    }
    if out.last().is_some_and(|last| last == BREAK) {
        out.pop();
    }
    out
}

pub fn sizes(env: &Env, input: [u32; 2]) -> BTreeMap<String, [u32; 2]> {
    [
        ("input".to_string(), input),
        ("target".to_string(), env.target),
        ("picture".to_string(), env.input),
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chain_is_split_on_commas_and_lines() {
        assert_eq!(
            chain_of(" a.glsl, b.wgsl\nc.hook ;; "),
            vec!["a.glsl", "b.wgsl", "c.hook"]
        );
        assert!(chain_of("  ").is_empty());
        assert_eq!(
            chain_of("a.glsl | b.hook, c.glsl"),
            vec!["a.glsl", "|", "b.hook", "c.glsl"]
        );
        assert_eq!(
            chain_of("| a.glsl ||"),
            vec!["a.glsl"],
            "端と重なりの区切りは捨てる"
        );
    }
}
