// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! パイプラインをビルドし、本体と同じ検査（`Graph::check`）と naga の検査に通す。
//! `ZW_MPV_SHADERS` があれば、そのフォルダーの mpv のシェーダーも一つずつビルドする。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use zwcore::output::video::shader::{Shape, frames_prelude};
use zwsdk::video::{At, Device, Env, Format, Graph};

fn env(at: At, input: [u32; 2], target: [u32; 2], chain: &str) -> Env {
    Env {
        at,
        format: Format {
            width: input[0],
            height: input[1],
            depth: 8,
            interlaced: true,
            aspect: [16, 9],
            transfer: "sdr".into(),
        },
        input,
        target,
        rate: [30000, 1001],
        display: 60.0,
        device: Device {
            backend: "vulkan".into(),
            compute: true,
            float32_filterable: false,
            max_texture: 16384,
            host: Vec::new(),
            ..Device::default()
        },
        settings: [("chain".to_string(), chain.to_string())].into(),
        program: None,
    }
}

fn assemble(
    e: &Env,
    read: &dyn Fn(&str) -> Option<String>,
    deinterlace: bool,
) -> Result<Graph, String> {
    assert_eq!(deinterlace, e.at == At::Deinterlace);
    zerowave_video_filter::assemble(e, e.settings.get("chain").map_or("", String::as_str), read)
}

fn reader(dir: PathBuf) -> impl Fn(&str) -> Option<String> {
    move |name| std::fs::read_to_string(dir.join(name)).ok()
}

fn validates(label: &str, text: &str) {
    let module = naga::front::wgsl::parse_str(text)
        .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(text)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(text)));
}

/// 本体の頂点と同じ場所に `@location(0)` を出す頂点。
const VERTEX: &str = "
struct ZwVertex { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn zw_vertex(@builtin(vertex_index) i: u32) -> ZwVertex {
    var out: ZwVertex;
    out.position = vec4<f32>(f32(i), 0.0, 0.0, 1.0);
    out.uv = vec2<f32>(0.0);
    return out;
}
";

fn check_all(graph: &Graph, at: At, prelude: Option<&str>) {
    graph.check(at).expect("本体の検査");
    for pass in graph
        .passes
        .iter()
        .filter(|pass| pass.kind != zwsdk::video::Kind::Host)
    {
        let text = match prelude {
            Some(prelude) => format!("{prelude}\n{}", pass.source),
            None => pass.source.clone(),
        };
        validates(&pass.name, &text);
    }
}

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/shaders")
}

#[test]
fn nothing_chained_is_a_pass_through() {
    let graph = assemble(
        &env(At::Picture, [1440, 1080], [1920, 1080], ""),
        &reader(examples()),
        false,
    )
    .expect("作れる");
    assert!(graph.passes.is_empty());
}

#[test]
fn the_examples_chain_into_one_graph() {
    let e = env(
        At::Picture,
        [1440, 1080],
        [1920, 1080],
        "sharpen.wgsl, blend2x.wgsl",
    );
    let graph = assemble(&e, &reader(examples()), false).expect("作れる");
    assert_eq!(graph.passes.len(), 2);
    assert_eq!(graph.outputs(), 2);
    assert_eq!(graph.history, 1);
    assert_eq!(graph.output, [1440, 1080]);
    assert_eq!(graph.passes[1].writes, "output");
    assert_eq!(graph.passes[1].reads[0], graph.passes[0].writes);
    assert_eq!(graph.passes[1].reads[1], "picture:1");
    check_all(&graph, At::Picture, None);
    for pass in &graph.passes {
        validates(
            &format!("{} と頂点", pass.name),
            &format!("{VERTEX}\n{}", pass.source),
        );
    }
}

#[test]
fn the_deinterlace_example_validates_with_every_prelude() {
    let e = env(
        At::Deinterlace,
        [1440, 1080],
        [1920, 1080],
        "deinterlace/ela.wgsl",
    );
    let graph = assemble(&e, &reader(examples()), true).expect("作れる");
    assert_eq!(graph.frames, [0, 0]);
    for shape in [Shape::Planar, Shape::Biplanar] {
        for depth in [8, 10] {
            let prelude = frames_prelude(depth, shape, graph.frames);
            check_all(&graph, At::Deinterlace, Some(&prelude));
        }
    }
    let e = env(
        At::Deinterlace,
        [1440, 1080],
        [1920, 1080],
        "deinterlace/tgmc.wgsl",
    );
    let graph = assemble(&e, &reader(examples()), true).expect("作れる");
    assert_eq!(graph.frames, [-2, 2], "KMergeStatic は前後二フレームを見る");
    assert_eq!(
        graph.passes.len(),
        12,
        "NNEDI3 は欠けた行だけのパス（afield・aedi・cedi・bedi）、動きは向きごと（pack）"
    );
    assert_eq!(
        graph
            .textures
            .values()
            .filter(|t| t.size == vec![90, 68])
            .count(),
        1,
        "動きは 16x16 ごと"
    );
    assert!(
        graph
            .passes
            .iter()
            .all(|pass| pass.kind == zwsdk::video::Kind::Fragment),
        "FXC でビルドできるよう compute を使わない"
    );
    for shape in [Shape::Planar, Shape::Biplanar] {
        for depth in [8, 10] {
            let prelude = frames_prelude(depth, shape, graph.frames);
            check_all(&graph, At::Deinterlace, Some(&prelude));
        }
    }
    let refused = assemble(
        &env(At::Deinterlace, [1440, 1080], [1920, 1080], "a.glsl"),
        &|_| Some(String::new()),
        true,
    );
    assert!(refused.unwrap_err().contains(".wgsl のシェーダーだけ"));
}

#[test]
fn a_mpv_unit_doubles_luma_and_merges_it_back() {
    let doubler = "
//!TEXTURE weights
//!SIZE 2 1
//!FORMAT rgba16f
//!FILTER NEAREST
003c003c003c003c003c003c003c003c

//!HOOK LUMA
//!BIND HOOKED
//!BIND weights
//!SAVE EDGES
//!DESC edges
vec4 hook() {
    return HOOKED_texOff(vec2(1.0, 0.0)) - HOOKED_texOff(vec2(-1.0, 0.0)) + texture(weights, vec2(0.25, 0.5)) * 0.0;
}

//!HOOK LUMA
//!BIND HOOKED
//!BIND EDGES
//!WIDTH HOOKED.w 2 *
//!HEIGHT HOOKED.h 2 *
//!WHEN OUTPUT.w HOOKED.w 1.2 * >
//!COMPUTE 16 16 8 8
//!DESC double
void hook() {
    vec2 pos = HOOKED_map(ivec2(gl_GlobalInvocationID.xy));
    float y = HOOKED_tex(pos).x + EDGES_tex(pos).x * 0.1;
    imageStore(out_image, ivec2(gl_GlobalInvocationID.xy), vec4(y, 0.0, 0.0, 1.0));
}
";
    let tint = "
//!HOOK MAIN
//!BIND HOOKED
//!DESC tint
vec4 hook() {
    return MAIN_tex(MAIN_pos) * vec4(1.0, 0.98, 1.0, 1.0);
}
";
    let files: BTreeMap<&str, &str> = [("double.glsl", doubler), ("tint.hook", tint)].into();
    let read = |name: &str| files.get(name).map(|s| s.to_string());
    let graph = assemble(
        &env(
            At::Picture,
            [1920, 1080],
            [3840, 2160],
            "tint.hook, double.glsl",
        ),
        &read,
        false,
    )
    .expect("作れる");
    let names: Vec<&str> = graph.passes.iter().map(|p| p.name.as_str()).collect();
    // LUMA が先（ファイルの順ではなくフック先の順）。
    assert_eq!(
        names,
        vec!["0|luma", "0|edges", "0|double", "0|merge luma", "0|tint"]
    );
    assert_eq!(graph.output, [3840, 2160], "倍にした輝度の大きさ");
    assert_eq!(graph.passes[2].block, [16, 16]);
    assert_eq!(graph.passes.last().unwrap().writes, "output");
    assert!(
        graph.textures.values().any(|t| t.data.is_some()),
        "添えたテクスチャ"
    );
    check_all(&graph, At::Picture, None);

    let small = assemble(
        &env(At::Picture, [1920, 1080], [1920, 1080], "double.glsl"),
        &read,
        false,
    )
    .expect("作れる");
    assert!(!small.passes.iter().any(|p| p.name == "double"));
    assert_eq!(small.output, [1920, 1080]);
}

#[test]
fn every_collected_mpv_shader_that_assembles_validates() {
    let Ok(root) = std::env::var("ZW_MPV_SHADERS") else {
        eprintln!("ZW_MPV_SHADERS が無いので飛ばす");
        return;
    };
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, out);
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("glsl" | "hook")
            ) {
                out.push(path);
            }
        }
    }
    let mut paths = Vec::new();
    walk(Path::new(&root), &mut paths);
    let (mut good, mut bad) = (0, Vec::new());
    for path in &paths {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let dir = path.parent().unwrap().to_path_buf();
        // 拡大するものが WHEN で外れないよう、画面は入力の倍。
        let e = env(At::Picture, [960, 540], [1920, 1080], &name);
        match assemble(&e, &reader(dir), false) {
            Ok(graph) => {
                check_all(&graph, At::Picture, None);
                good += 1;
            }
            Err(why) => bad.push(format!(
                "{}: {}",
                path.display(),
                why.lines().next().unwrap_or("")
            )),
        }
    }
    eprintln!("ビルドできた {good} / {}", paths.len());
    for line in &bad {
        eprintln!("  ビルドできない {line}");
    }
}

/// `ZW_TRY_DIR`・`ZW_TRY_CHAIN`・`ZW_TRY_SIZE=入力幅x高さ:画面幅x高さ` の並びをビルドする。
#[test]
fn a_chain_named_in_the_environment() {
    let (Ok(dir), Ok(chain)) = (std::env::var("ZW_TRY_DIR"), std::env::var("ZW_TRY_CHAIN")) else {
        return;
    };
    let size = std::env::var("ZW_TRY_SIZE").unwrap_or_else(|_| "1440x1080:2560x1440".into());
    let parse = |s: &str| -> [u32; 2] {
        let (w, h) = s.split_once('x').expect("幅x高さ");
        [w.parse().expect("幅"), h.parse().expect("高さ")]
    };
    let (input, target) = size.split_once(':').expect("入力:画面");
    let e = env(At::Picture, parse(input), parse(target), &chain);
    match assemble(&e, &reader(PathBuf::from(dir)), false) {
        Ok(graph) => {
            check_all(&graph, At::Picture, None);
            let names: Vec<&str> = graph.passes.iter().map(|p| p.name.as_str()).collect();
            eprintln!(
                "ビルドできた：{} パス → {:?}\n  {}",
                graph.passes.len(),
                graph.output,
                names.join(" / ")
            );
        }
        Err(why) => panic!("ビルドできない：{why}"),
    }
}

/// NATIVE は元の画像（前のフィルターが広げた後ではない）。
#[test]
fn native_is_the_original_picture_across_filters() {
    let double = "
//!HOOK MAIN
//!BIND HOOKED
//!WIDTH HOOKED.w 2 *
//!HEIGHT HOOKED.h 2 *
//!DESC double
vec4 hook() { return HOOKED_tex(HOOKED_pos); }
";
    let fit = "
//!HOOK MAIN
//!BIND HOOKED
//!BIND NATIVE
//!WHEN OUTPUT.w NATIVE.w / 2.0 <
//!WIDTH OUTPUT.w
//!HEIGHT OUTPUT.h
//!DESC fit
vec4 hook() { return HOOKED_tex(HOOKED_pos) + NATIVE_tex(NATIVE_pos) * 0.0; }
";
    let files: BTreeMap<&str, &str> = [("double.glsl", double), ("fit.glsl", fit)].into();
    let read = |name: &str| files.get(name).map(|s| s.to_string());
    let graph = assemble(
        &env(
            At::Picture,
            [1440, 1080],
            [2560, 1440],
            "double.glsl | fit.glsl",
        ),
        &read,
        false,
    )
    .expect("作れる");
    check_all(&graph, At::Picture, None);
    assert_eq!(graph.output, [2560, 1440], "元の 1.78 倍なので fit が働く");
}

#[test]
fn the_interpolation_example_retimes_and_validates() {
    let e = env(
        At::Picture,
        [1440, 1080],
        [2560, 1440],
        "interpolate/motion.wgsl",
    );
    let graph = zerowave_video_filter::assemble_retimed(
        &e,
        "interpolate/motion.wgsl",
        &reader(examples()),
        Some(120.0),
    )
    .expect("作れる");
    check_all(&graph, At::Picture, None);
    assert_eq!(graph.retime(), Some(120.0));
    assert_eq!(graph.history, 1);
    assert_eq!(graph.output, [1440, 1080], "大きさは変えない");
}

#[test]
fn host_interpolation_follows_the_device() {
    use zerowave_video_filter::hosted;
    use zwsdk::video::Kind;
    for (who, host, kinds) in [
        (
            "@videotoolbox",
            "interpolate/videotoolbox",
            vec![Kind::Host],
        ),
        ("@amf", "interpolate/amf", vec![Kind::Host]),
        ("@vpl", "interpolate/vpl", vec![Kind::Host]),
        ("@nvofa", "flow/nvofa", vec![Kind::Host, Kind::Fragment]),
        ("@flow", "flow/nvofa", vec![Kind::Host, Kind::Fragment]),
    ] {
        let file = format!("interpolate/{who}.wgsl");
        for chain in [file.clone(), format!("sharpen.wgsl | {file}")] {
            let mut e = env(At::Picture, [1440, 1080], [2560, 1440], &chain);
            e.device.host = vec![host.to_owned()];
            let files = reader(examples());
            let device = e.device.clone();
            let read = move |name: &str| match hosted::entry(name) {
                Some(entry) => hosted::text(entry, &device, &Default::default()),
                None => files(name),
            };
            let graph = zerowave_video_filter::assemble_retimed(&e, &chain, &read, Some(120.0))
                .unwrap_or_else(|why| panic!("{chain}: {why}"));
            check_all(&graph, At::Picture, None);
            assert_eq!(graph.retime(), Some(120.0));
            let unit = if chain.contains('|') { "1|" } else { "0|" };
            let got: Vec<Kind> = graph
                .passes
                .iter()
                .filter(|p| p.name.starts_with(unit))
                .map(|p| p.kind)
                .collect();
            assert_eq!(got, kinds, "{chain}");
            if who == "@nvofa" || who == "@flow" {
                let flow = &graph
                    .passes
                    .iter()
                    .find(|p| p.kind == Kind::Host)
                    .expect("フロー")
                    .writes;
                assert_eq!(graph.textures[flow].size, vec![360, 270], "画像の 1/4");
            }
        }
        let entry = hosted::entry(&file).expect("実装");
        assert!(
            hosted::text(
                entry,
                &env(At::Picture, [1440, 1080], [2560, 1440], "").device,
                &Default::default()
            )
            .is_none()
        );
    }
}

#[test]
fn flow_follows_the_capabilities_of_any_vendor() {
    use zerowave_video_filter::hosted;
    use zwsdk::video::{FlowCaps, Kind};
    for who in ["nvofa", "intel"] {
        let file = "interpolate/@flow.wgsl";
        let mut e = env(At::Picture, [1440, 1080], [2560, 1440], file);
        e.device.host = vec![format!("flow/{who}")];
        e.device.flows = vec![FlowCaps {
            who: who.to_owned(),
            grids: vec![2, 4],
            confidence: true,
        }];
        let files = reader(examples());
        let device = e.device.clone();
        let read = move |name: &str| match hosted::entry(name) {
            Some(entry) => hosted::text(entry, &device, &Default::default()),
            None => files(name),
        };
        let graph = zerowave_video_filter::assemble_retimed(&e, file, &read, Some(120.0))
            .unwrap_or_else(|why| panic!("{who}: {why}"));
        check_all(&graph, At::Picture, None);
        let kinds: Vec<Kind> = graph.passes.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, vec![Kind::Host, Kind::Host, Kind::Fragment], "{who}");
        let flow = &graph.passes[0].writes;
        let confidence = &graph.passes[1].writes;
        assert_eq!(graph.textures[flow].size, vec![720, 540], "グリッド 2");
        assert_eq!(graph.textures[confidence].size, vec![720, 540]);
        assert_eq!(graph.passes[1].source, "flow-confidence");
    }
}

#[test]
fn interpolation_midway_reads_the_previous_filter_output() {
    let chain = "sharpen.wgsl | interpolate/motion.wgsl | sharpen.wgsl";
    let e = env(At::Picture, [1440, 1080], [2560, 1440], chain);
    let graph = zerowave_video_filter::assemble_retimed(&e, chain, &reader(examples()), Some(60.0))
        .expect("作れる");
    check_all(&graph, At::Picture, None);
    let first = graph
        .passes
        .iter()
        .position(|p| p.name.starts_with("1|"))
        .expect("補間のパス");
    assert_eq!(graph.retime_from, first, "補間のパスから打ち直す");
    let input = &graph.passes[first].reads[0];
    assert!(graph.textures[input].previous, "{input} が一コマ前を持つ");
    assert!(
        graph.passes[first].reads.contains(&format!("{input}:1")),
        "picture:1 は前のフィルターの出力の一コマ前"
    );
}

#[test]
fn rife_hands_the_program_to_the_host() {
    use zerowave_video_filter::hosted;
    use zwsdk::video::Kind;
    let file = "interpolate/@rife.wgsl";
    let mut e = env(At::Picture, [1440, 1080], [2560, 1440], file);
    e.device.host = vec![String::from("interpolate/process")];
    let device = e.device.clone();
    let settings: BTreeMap<String, String> = [
        (
            hosted::RIFE_PROGRAM.to_owned(),
            String::from("C:/bin/zwtrt.exe"),
        ),
        (hosted::RIFE_DIR.to_owned(), String::from("C:/trt")),
    ]
    .into();
    let read = move |name: &str| {
        hosted::entry(name).and_then(|entry| hosted::text(entry, &device, &settings))
    };
    let graph = zerowave_video_filter::assemble_retimed(&e, file, &read, Some(120.0))
        .unwrap_or_else(|why| panic!("{why}"));
    check_all(&graph, At::Picture, None);
    let host = graph
        .passes
        .iter()
        .find(|p| p.kind == Kind::Host)
        .expect("本体のパス");
    assert_eq!(host.source, "interpolate/process");
    let process = host.process.as_ref().expect("起動するプロセス");
    assert_eq!(process.program, hosted::RIFE_PROGRAM);
    assert_eq!(process.args, ["serve", "C:/trt/rife.onnx", "C:/trt"]);
    assert_eq!(process.dir, hosted::RIFE_DIR);
    assert!(process.ways.is_empty(), "受け渡しの方式は本体に任せる");
}
