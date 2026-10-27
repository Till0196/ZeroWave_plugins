// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 映像フィルター：シェーダーファイル（mpv の `.glsl`・`.hook`、自前の `.wgsl`）を読んで、
//! パイプラインをビルドするローダー。画素の処理は本体の GPU が行う。プリセットは番組などで自動選択する。

pub mod build;
pub mod genre;
pub mod glsl;
pub mod hosted;
pub mod mpv;
pub mod native;
pub mod notices;
pub mod presets;
pub mod rife_gpu;
pub mod text;
pub mod ui;

use zwsdk::video::{At, Env, Graph};

use presets::{Chosen, Filter, Target};

pub const FOLDER: &str = "folder";

/// パイプラインをビルドする。`chain` は掛けるファイルの並び（`,` で並べ、`|` でグループを区切る）、
/// `read` はフォルダーのファイルを読む関数。デインタレースのフックで読めるのは `.wgsl` だけ。
pub fn assemble(
    env: &Env,
    chain: &str,
    read: &dyn Fn(&str) -> Option<String>,
) -> Result<Graph, String> {
    assemble_retimed(env, chain, read, None)
}

pub fn assemble_retimed(
    env: &Env,
    chain: &str,
    read: &dyn Fn(&str) -> Option<String>,
    retime: Option<f64>,
) -> Result<Graph, String> {
    let deinterlace = env.at == At::Deinterlace;
    let chain = build::chain_of(chain);
    if chain.is_empty() {
        return Ok(Graph::default());
    }
    let mut files = Vec::new();
    for name in &chain {
        if name == build::BREAK {
            files.push((name.clone(), String::new()));
            continue;
        }
        let text = read(name).ok_or_else(|| format!("ファイルが見つかりません：{name}"))?;
        files.push((name.clone(), text));
    }
    let mut builder = build::Builder::new(env);
    if let Some(retime) = retime {
        builder.retime = retime;
    }
    let mut at = 0;
    let mut first = true;
    // パスの名前の先頭に何番目のフィルターかを付ける（本体が測った時間を結び付けるため）。
    let mut filter = 0usize;
    while at < files.len() {
        let before = builder.graph.passes.len();
        let name = files[at].0.to_ascii_lowercase();
        if name == build::BREAK {
            at += 1;
            continue;
        }
        if name == rife_gpu::FILE {
            rife_gpu::unit(&mut builder)?;
            at += 1;
        } else if name.ends_with(".wgsl") {
            let (file, text) = &files[at];
            native::unit(&mut builder, file, text, first)?;
            at += 1;
        } else if name.ends_with(".glsl") || name.ends_with(".hook") {
            if deinterlace {
                return Err(format!(
                    "{}：デインターレースには .wgsl のシェーダーだけ使えます。",
                    files[at].0
                ));
            }
            let end = (at..files.len())
                .find(|&i| {
                    let next = files[i].0.to_ascii_lowercase();
                    next.ends_with(".wgsl") || next == build::BREAK
                })
                .unwrap_or(files.len());
            mpv::unit(&mut builder, &files[at..end])?;
            at = end;
        } else {
            return Err(format!(
                "{}：対応していない形式です\n（対応形式: .glsl / .hook / .wgsl ）。",
                files[at].0
            ));
        }
        for pass in &mut builder.graph.passes[before..] {
            pass.name = format!("{filter}|{}", pass.name);
        }
        filter += 1;
        builder.unit += 1;
        first = false;
    }
    Ok(builder.finish())
}

pub fn unit_of(pass: &str) -> Option<usize> {
    pass.split_once('|')?.0.parse().ok()
}

pub fn chosen(filter: &Filter, env: &Env) -> Chosen {
    filter.choose(env.program.as_ref().map(|program| Target {
        program,
        format: Some(&env.format),
    }))
}

#[derive(Debug, Default)]
pub struct Shaders {
    told: Option<Chosen>,
    settings: std::collections::BTreeMap<String, String>,
    built: Option<Chosen>,
    compiling: Option<zwsdk::ui::Notice>,
    /// このビルドのプリセットの通知。準備中の通知に押し出されるので、済んだら出し直す。
    preset: Option<zwsdk::ui::Notice>,
    shown_preset: Option<zwsdk::ui::Notice>,
}

impl Shaders {
    /// プリセットが切り替わったら通知する（二度出ないようフィルターチェーンの側だけ）。
    fn tell(&mut self, env: &Env, filter: &Filter, chosen: &Chosen) {
        self.settings = env.settings.clone();
        if env.at != At::Picture || env.program.is_none() {
            return;
        }
        let same_preset = self
            .told
            .as_ref()
            .is_some_and(|told| told.preset == chosen.preset);
        let same = self.told.as_ref().is_some_and(|told| {
            told.preset == chosen.preset && told.auto == chosen.auto && told.manual == chosen.manual
        });
        let first = self.told.is_none();
        self.told = Some(chosen.clone());
        if same || (same_preset && filter.notify.quiet_same) {
            return;
        }
        // 設定のキーを問い合わせられた後にビルドでもう一度来るので、同じなら覚えた通知は残す。
        self.preset = None;
        if first && chosen.preset.is_none() {
            return;
        }
        if let Some(notice) = notices::preset(filter, chosen, env) {
            zwsdk::plug::host::notice(&notice);
            self.shown_preset = Some(notice.clone());
            self.preset = Some(notice);
        }
    }
}

impl zwsdk::plug::Video for Shaders {
    fn new() -> Self {
        Self::default()
    }

    fn build(&mut self, env: &Env) -> Result<Graph, String> {
        // 本体はビルドの途中で作り直すと前のビルドを捨て、済んだことを知らせない。
        if let Some(shown) = self.compiling.take() {
            zwsdk::plug::host::notice(&notices::withdrawn(shown));
        }
        let filter = Filter::load(&env.settings);
        let chosen = chosen(&filter, env);
        self.tell(env, &filter, &chosen);
        let chain = filter.chain(&chosen, env.at);
        self.built = Some(chosen.clone());
        let read = |name: &str| match hosted::entry(name) {
            Some(entry) => hosted::text(entry, &env.device, &env.settings).or_else(|| {
                zwsdk::plug::info(&format!(
                    "{}：この本体では使えないので {} にする",
                    entry.name,
                    hosted::FALLBACK
                ));
                zwsdk::plug::host::files::read_text(FOLDER, hosted::FALLBACK)
            }),
            None => zwsdk::plug::host::files::read_text(FOLDER, name),
        };
        let retime = filter.retime(&chosen, env.display);
        let mut graph = assemble_retimed(env, &chain, &read, Some(retime))?;
        if !graph.passes.is_empty() {
            graph.buffer_ms = filter.buffer_ms(&chosen);
            graph.keep = filter.keeps(&chosen);
            let held = filter.held(&chosen);
            for pass in graph.passes.iter_mut().filter(|pass| {
                pass.kind == zwsdk::video::Kind::Host
                    && pass.source.starts_with(zwsdk::video::HOST_INTERPOLATE)
            }) {
                pass.held = held;
            }
        }
        if !graph.passes.is_empty() {
            zwsdk::plug::info(&format!(
                "{}：{} パスをビルドした（{}x{} → {}x{}）",
                chosen.preset.as_deref().unwrap_or(""),
                graph.passes.len(),
                env.input[0],
                env.input[1],
                graph.output[0],
                graph.output[1]
            ));
        }
        Ok(graph)
    }

    fn key(&mut self, env: &Env) -> Option<String> {
        let filter = Filter::load(&env.settings);
        let chosen = chosen(&filter, env);
        self.tell(env, &filter, &chosen);
        let chain = filter.chain(&chosen, env.at);
        let interpolating = filter.interpolates(&chosen);
        let key = if interpolating && env.at == At::Picture {
            format!("{chain}@{}", filter.retime(&chosen, env.display))
        } else {
            chain
        };
        let key = match filter.buffer_ms(&chosen) {
            0 => key,
            ms => format!("{key}#{ms}"),
        };
        let key = match filter.held(&chosen) {
            0 => key,
            held => format!("{key}~{held}"),
        };
        Some(if filter.keeps(&chosen) {
            format!("{key}!keep")
        } else {
            key
        })
    }

    fn event(&mut self, event: &zwsdk::video::Event) {
        use zwsdk::video::Event;
        let filter = Filter::load(&self.settings);
        let notice = notices::event(&filter, self.built.as_ref().or(self.told.as_ref()), event);
        match event {
            Event::Building { .. } | Event::Progress { .. } | Event::Preparing { .. } => {
                self.compiling = notice.clone()
            }
            // 実装の準備はシェーダーのビルドの後なので、プリセットの通知は覚えた物を出し直す。
            Event::Prepared => {
                if let (None, Some(shown)) = (&notice, self.compiling.take()) {
                    match self.shown_preset.clone() {
                        Some(preset) => zwsdk::plug::host::notice(&preset),
                        None => zwsdk::plug::host::notice(&notices::withdrawn(shown)),
                    }
                }
            }
            Event::Built { .. } => {
                let preset = self.preset.take();
                if let (None, Some(shown)) = (&notice, self.compiling.take()) {
                    match preset {
                        Some(preset) => zwsdk::plug::host::notice(&preset),
                        None => zwsdk::plug::host::notice(&notices::withdrawn(shown)),
                    }
                }
            }
            _ if notice.is_some() => {
                self.compiling = None;
                self.preset = None;
            }
            _ => {}
        }
        if let Some(notice) = notice {
            zwsdk::plug::host::notice(&notice);
        }
    }
}

zwsdk::export_video!(
    Shaders,
    name = "映像フィルター",
    version = "0.1.1",
    video = r#"{"at":["deinterlace","picture"],"program":true}"#,
    ui = ui::Menu,
    ui_manifest = r#"{"title":"映像フィルター","settings":"main","panels":[{"id":"filter","title":"映像フィルター"}]}"#,
    config = r#"[
        {"key":"folder","label":"シェーダーのフォルダー","kind":"folder","default":".","hint":"空ならプラグインと同じ名前のフォルダー（plugins/video_filter/）"},
        {"key":"rife_program","label":"RIFE（TensorRT・CoreML）の実行ファイル","kind":"program","hint":"zwtrt（mac は zwcoreml）"},
        {"key":"rife_dir","label":"RIFE（TensorRT・CoreML）のフォルダー","kind":"folder","hint":"ONNX・エンジン・モデルのフォルダー。空なら実行ファイルと同じフォルダー"},
        {"key":"rife_tensorrt","label":"RIFE（TensorRT）：TensorRT の DLL のフォルダー","kind":"folder"},
        {"key":"rife_cuda","label":"RIFE（TensorRT）：CUDA の DLL のフォルダー","kind":"folder"},
        {"key":"rife_models","label":"RIFE（GPU 共通）の重みのフォルダー","kind":"folder","default":"rife-models","hint":"下にモデルのフォルダー（rife-ncnn-vulkan の rife-v4.6 など、flownet.param と flownet.bin）を置く"},
        {"key":"rife_model","label":"RIFE（GPU 共通）のモデル","kind":"text","default":"rife-v4.6"},
        {"key":"rife_scale","label":"RIFE（GPU 共通）でフローを求める大きさ","kind":"choice","default":"0.5","choices":[["1","等倍"],["0.5","1/2"],["0.25","1/4"]],"hint":"小さいほど軽い。動きの細かさは落ちる"},
        {"key":"rife_kernels","label":"RIFE（GPU 共通）の計算","kind":"choice","default":"auto","choices":[["auto","GPU に合わせる"],["f16","f16（協調行列を使わない）"],["f32","f32 だけ"]]}
    ]"#
);
