// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! RIFE v4 を本体の GPU で（WGSL の compute）。重みは rife-ncnn-vulkan（MIT）の
//! `flownet.param`・`flownet.bin` を設定のフォルダーから読む。
//! ネットワークはレンダリングと同じ GPU のキューで動くので、一入力遅らせて前の結果を使っても速くならない。

mod model;
mod plan;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use zwsdk::video::{Access, Bind, Blob, BlobAt, Buffer, Device, Env, Kind, Pass};

use crate::build::{Builder, declare};
use model::{Form, Layer};
use plan::{Caps, Kernel, Plan, Slot};

pub const FILE: &str = "interpolate/@rife-gpu.wgsl";
/// 設定：重みのフォルダー（`kind = "folder"`）。下にモデルごとのフォルダーを置く。
pub const MODELS: &str = "rife_models";
/// 設定：モデルのフォルダーの名前（既定は [`DEFAULT_MODEL`]）。
pub const MODEL: &str = "rife_model";
/// 設定：フローを求める大きさ（`1`・`0.5`・`0.25`）。
pub const SCALE: &str = "rife_scale";
/// 設定：使うカーネル（`auto` は GPU に合わせる、`f16` は協調行列を使わない、`f32` は f32 の物だけ）。
pub const KERNELS: &str = "rife_kernels";

pub const DEFAULT_MODEL: &str = "rife-v4.6";
pub const DEFAULT_SCALE: f32 = 0.5;

const NN: &str = include_str!("wgsl/rife/nn.wgsl");
const WINO: &str = include_str!("wgsl/rife/wino.wgsl");
const WINO_H: &str = include_str!("wgsl/rife/wino_h.wgsl");
const WINO_TC: &str = include_str!("wgsl/rife/wino_tc.wgsl");
const TC: &str = include_str!("wgsl/rife/tc.wgsl");
const PACK: &str = include_str!("wgsl/rife/pack.wgsl");
const MERGE: &str = include_str!("wgsl/rife/merge.wgsl");

const TC_STORAGE: u32 = 64 * 64 * 4 + 1728 * 2;

pub fn scale_of(settings: &BTreeMap<String, String>) -> f32 {
    settings
        .get(SCALE)
        .and_then(|text| {
            let text = text.trim();
            match text.split_once('/') {
                Some((a, b)) => Some(a.trim().parse::<f32>().ok()? / b.trim().parse::<f32>().ok()?),
                None => text.parse().ok(),
            }
        })
        .filter(|s| s.is_finite() && *s > 0.0)
        .map_or(DEFAULT_SCALE, |s| s.min(1.0))
}

/// 使える GPU の機能。テンソルコアの物はサブグループが 32 スレッドの GPU の Vulkan だけ（NVIDIA）。
/// Apple の simdgroup（8×8）の行列積は協調行列を使わない物より遅いので使わない。
pub fn caps_of(device: &Device) -> Caps {
    let has = |m: u32, n: u32, k: u32| {
        device
            .matrices
            .iter()
            .any(|x| (x.m, x.n, x.k) == (m, n, k) && x.ab == "f16" && x.cr == "f32")
    };
    let half = device.shader_f16;
    Caps {
        half,
        tc16: half
            && device.backend == "vulkan"
            && device.limits.subgroup == [32, 32]
            && device.limits.workgroup_storage >= TC_STORAGE
            && has(16, 16, 16),
        reduce_below: 600,
    }
}

struct Loaded {
    key: String,
    layers: Arc<Vec<Layer>>,
}

static MODEL_CACHE: Mutex<Option<Loaded>> = Mutex::new(None);

static WEIGHTS: Mutex<Option<(String, Vec<(usize, Form)>, Blob, Vec<u64>)>> = Mutex::new(None);

fn size_of(listed: &[zwsdk::plug::host::files::Entry], name: &str) -> Option<u64> {
    listed
        .iter()
        .find(|e| !e.dir && e.name == name)
        .map(|e| e.size)
}

fn load(model: &str) -> Result<Arc<Vec<Layer>>, String> {
    use zwsdk::plug::host::files;
    let Some(listed) = files::list(MODELS, model) else {
        return Err(format!(
            "RIFE（GPU 共通）：設定の {MODELS} の下に {model} のフォルダーがありません（flownet.param と flownet.bin を置いてください）。"
        ));
    };
    let (Some(param_size), Some(bin_size)) = (
        size_of(&listed, "flownet.param"),
        size_of(&listed, "flownet.bin"),
    ) else {
        let names: Vec<&str> = listed.iter().map(|e| e.name.as_str()).collect();
        return Err(format!(
            "RIFE（GPU 共通）：{model} に flownet.param と flownet.bin がありません（あるのは {names:?}）。"
        ));
    };
    let key = format!("{model}:{param_size}:{bin_size}");
    let mut cache = MODEL_CACHE
        .lock()
        .map_err(|_| "モデルのキャッシュが壊れています。")?;
    if let Some(loaded) = cache.as_ref().filter(|l| l.key == key) {
        return Ok(Arc::clone(&loaded.layers));
    }
    let text = files::read_text(MODELS, &format!("{model}/flownet.param"))
        .ok_or_else(|| format!("{model}/flownet.param を読めません。"))?;
    let bytes = files::read(MODELS, &format!("{model}/flownet.bin"))
        .ok_or_else(|| format!("{model}/flownet.bin を読めません。"))?;
    let mut layers = model::parse_param(&text).map_err(|e| format!("{model}：{e}"))?;
    model::load_weights(&mut layers, &bytes).map_err(|e| format!("{model}：{e}"))?;
    let layers = Arc::new(layers);
    *cache = Some(Loaded {
        key,
        layers: Arc::clone(&layers),
    });
    Ok(layers)
}

fn weights(
    key: &str,
    layers: &[Layer],
    wanted: &[(usize, Form)],
) -> Result<(Blob, Vec<u64>), String> {
    let mut cache = WEIGHTS
        .lock()
        .map_err(|_| "重みのキャッシュが壊れています。")?;
    if let Some((_, _, blob, offsets)) = cache
        .as_ref()
        .filter(|(k, w, _, _)| k == key && w.as_slice() == wanted)
    {
        return Ok((blob.clone(), offsets.clone()));
    }
    let mut bytes = Vec::new();
    let mut offsets = Vec::with_capacity(wanted.len());
    for &(layer, form) in wanted {
        offsets.push(bytes.len() as u64);
        bytes.extend(model::weights_of(&layers[layer], form)?);
        bytes.resize(bytes.len().div_ceil(4) * 4, 0);
    }
    let blob = Blob::new(bytes);
    *cache = Some((
        key.to_owned(),
        wanted.to_vec(),
        blob.clone(),
        offsets.clone(),
    ));
    Ok((blob, offsets))
}

/// `//@unroll 名前 a..b` から `//@end` までを、`{名前}` を数にして並べる（入れ子も）。wgpu は
/// ループごとに回数のカウンターを足し、決まった回数のループでも展開されなくなるので、重いループは書く側で展開する。
fn unroll(source: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    expand(&lines, &mut out);
    out.join("\n")
}

fn expand(lines: &[&str], out: &mut Vec<String>) {
    let mut i = 0;
    while i < lines.len() {
        let head = lines[i].trim();
        let Some((name, (from, to))) = head
            .strip_prefix("//@unroll ")
            .and_then(|rest| rest.trim().split_once(' '))
            .and_then(|(name, range)| Some((name, range.trim().split_once("..")?)))
        else {
            out.push(lines[i].to_owned());
            i += 1;
            continue;
        };
        let (Ok(from), Ok(to)) = (from.parse::<u32>(), to.parse::<u32>()) else {
            out.push(lines[i].to_owned());
            i += 1;
            continue;
        };
        let mut depth = 1;
        let mut end = i + 1;
        while end < lines.len() {
            let line = lines[end].trim();
            if line.starts_with("//@unroll ") {
                depth += 1;
            } else if line == "//@end" {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            end += 1;
        }
        let key = format!("{{{name}}}");
        for value in from..to {
            let body: Vec<String> = lines[i + 1..end.min(lines.len())]
                .iter()
                .map(|line| line.replace(&key, &value.to_string()))
                .collect();
            let refs: Vec<&str> = body.iter().map(String::as_str).collect();
            expand(&refs, out);
        }
        i = end + 1;
    }
}

fn strip_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| match line.find("//") {
            Some(at) => line[..at].trim_end(),
            None => line.trim_end(),
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn kernel_source(kernel: Kernel, caps: Caps) -> (String, &'static str) {
    let wino = || {
        WINO.replace("{{ENABLE}}", if caps.half { "enable f16;" } else { "" })
            .replace("{{S}}", if caps.half { "f16" } else { "f32" })
            .replace("{{GEMM16}}", if caps.half { WINO_H } else { "" })
    };
    let source = match kernel {
        Kernel::Nn(entry) => return (strip_comments(&unroll(NN)), entry),
        Kernel::Tc(ocw) => (
            TC.replace("{{OCW}}", &ocw.to_string())
                .replace("{{CS}}", &(64 * ocw).to_string()),
            "conv3tc",
        ),
        Kernel::WinoIn => (wino(), "input_transform"),
        Kernel::WinoGemm => (wino(), "gemm"),
        Kernel::WinoGemmHalf => (wino(), "gemm_h"),
        Kernel::WinoOut => (wino(), "output_transform"),
        Kernel::WinoTc | Kernel::WinoTcOc => (
            WINO_TC.to_owned(),
            if kernel == Kernel::WinoTc {
                "gemm_tc"
            } else {
                "gemm_tc_oc"
            },
        ),
    };
    (strip_comments(&unroll(&source.0)), source.1)
}

fn access_of(binding: u32) -> Access {
    match binding {
        2 | 7 | 8 => Access::ReadWrite,
        4 => Access::Uniform,
        _ => Access::Read,
    }
}

const PREFIX: &str = "rife";

fn uniform_stride(device: &Device) -> u64 {
    let align = u64::from(device.limits.uniform_align.max(16));
    256u64.div_ceil(align) * align
}

/// パイプラインに RIFE を足す。入力は `builder.main`、出力は同じ大きさの新しいテクスチャ。
pub fn unit(builder: &mut Builder<'_>) -> Result<(), String> {
    let env: &Env = builder.env;
    if !env.device.compute {
        return Err(String::from(
            "RIFE（GPU 共通）：この GPU は compute に対応していません。",
        ));
    }
    // D3D12 で f16 が無いのは FXC でコンパイルしているとき。FXC はカーネルのループを
    // 展開できず、コンパイルにも何分もかかる。
    if env.device.backend == "dx12" && !env.device.shader_f16 {
        return Err(String::from(
            "RIFE（GPU 共通）：D3D12 のシェーダーを FXC でコンパイルしているので使えません。本体と同じフォルダーに dxcompiler.dll と dxil.dll（DirectXShaderCompiler）を置いてください。",
        ));
    }
    let model_name = env
        .settings
        .get(MODEL)
        .map(|m| m.trim())
        .filter(|m| !m.is_empty())
        .unwrap_or(DEFAULT_MODEL)
        .to_owned();
    let scale = scale_of(&env.settings);
    let mut caps = caps_of(&env.device);
    match env.settings.get(KERNELS).map(|k| k.trim()) {
        Some("f32") => {
            caps.half = false;
            caps.tc16 = false;
        }
        Some("f16") => caps.tc16 = false,
        _ => {}
    }
    let layers = load(&model_name)?;
    let (input, size) = builder.main.clone();
    let run = if scale < 1.0 {
        [
            ((size[0] as f32 * scale).round() as u32).max(32),
            ((size[1] as f32 * scale).round() as u32).max(32),
        ]
    } else {
        size
    };
    let plan = plan::plan(&layers, run, caps).map_err(|e| format!("{model_name}：{e}"))?;
    check(&plan, &env.device, builder.graph.passes.len())?;

    let midway = input != "picture";
    let (prev, cur) = if midway {
        if let Some(texture) = builder.graph.textures.get_mut(&input) {
            texture.previous = true;
        }
        (format!("{input}:1"), input.clone())
    } else {
        (String::from("picture:1"), input.clone())
    };
    builder.graph.retime = builder.retime;
    builder.graph.history = builder.graph.history.max(1);

    let unit = builder.unit;
    let name = |what: &str| format!("{PREFIX}{unit}_{what}");
    for (i, len) in plan.buffers.iter().enumerate() {
        builder.graph.buffers.insert(
            name(&format!("b{i}")),
            Buffer {
                size: len * 4,
                ..Buffer::default()
            },
        );
    }
    let key = format!("{model_name}:{}", layers.len());
    let (blob, offsets) = weights(&key, &layers, &plan.weights)?;
    let blob_name = format!("{PREFIX}-weights");
    for j in 0..plan.weights.len() {
        let end = offsets.get(j + 1).copied().unwrap_or(blob.0.len() as u64);
        builder.graph.buffers.insert(
            name(&format!("w{j}")),
            Buffer {
                size: (end - offsets[j]).max(4),
                blob: Some(BlobAt {
                    name: blob_name.clone(),
                    offset: offsets[j],
                }),
                ..Buffer::default()
            },
        );
    }
    builder.graph.blobs.insert(blob_name, blob);

    let stride = uniform_stride(&env.device);
    let mut params: Vec<u8> = Vec::new();
    let mut put = |words: &[u32]| -> u64 {
        let at = params.len() as u64;
        params.extend(words.iter().flat_map(|w| w.to_le_bytes()));
        params.resize((at + stride) as usize, 0);
        at
    };
    let net_at: Vec<u64> = plan.uniforms.iter().map(|u| put(u)).collect();
    let padded = plan.padded;
    let taps = ((size[0] as f32 / run[0] as f32) / 2.0).round().max(1.0) as u32;
    let pack_at = put(&[
        padded.w,
        padded.h,
        run[0],
        run[1],
        taps,
        0.5f32.to_bits(),
        0,
        0,
    ]);
    let merge_at = put(&[
        padded.w,
        padded.h,
        (size[0] as f32 / run[0] as f32).to_bits(),
        (size[1] as f32 / run[1] as f32).to_bits(),
        plan.flow0.offset,
        plan.flow1.offset,
        plan.mask.offset,
        0,
    ]);
    let params_name = name("params");
    builder.graph.buffers.insert(
        params_name.clone(),
        Buffer {
            size: params.len() as u64,
            blob: Some(BlobAt {
                name: params_name.clone(),
                offset: 0,
            }),
            ..Buffer::default()
        },
    );
    builder
        .graph
        .blobs
        .insert(params_name.clone(), Blob::new(params));

    let buffer_of = |slot: Slot| -> String {
        match slot {
            Slot::Buffer(i) => name(&format!("b{i}")),
            Slot::Weights(j) => name(&format!("w{j}")),
            Slot::Uniform(_) => params_name.clone(),
        }
    };
    let bind = |binding: u32, slot: Slot| -> Bind {
        let (offset, size) = match slot {
            Slot::Uniform(u) => (net_at[u], 96),
            _ => (0, 0),
        };
        Bind {
            binding,
            name: buffer_of(slot),
            access: access_of(binding),
            offset,
            size,
        }
    };
    let reads_decl = [
        (String::from("prev"), "texture_2d<f32>"),
        (String::from("cur"), "texture_2d<f32>"),
    ];

    let mut pack_source = declare(&reads_decl, None);
    pack_source.push_str(PACK);
    let [in0, in1, in2] = plan.inputs;
    builder.push(Pass {
        name: String::from("RIFE の入力"),
        kind: Kind::Compute,
        source: pack_source,
        entry: String::from("pack"),
        reads: vec![prev.clone(), cur.clone()],
        writes: String::new(),
        groups: [padded.w.div_ceil(8), padded.h.div_ceil(8), 1],
        buffers: vec![
            Bind {
                binding: 0,
                name: buffer_of(Slot::Buffer(in0.buffer)),
                access: Access::ReadWrite,
                ..Bind::default()
            },
            Bind {
                binding: 1,
                name: buffer_of(Slot::Buffer(in1.buffer)),
                access: Access::ReadWrite,
                ..Bind::default()
            },
            Bind {
                binding: 2,
                name: buffer_of(Slot::Buffer(in2.buffer)),
                access: Access::ReadWrite,
                ..Bind::default()
            },
            Bind {
                binding: 3,
                name: params_name.clone(),
                access: Access::Uniform,
                offset: pack_at,
                size: 32,
            },
        ],
        ..Pass::default()
    });
    let last = plan.steps.len().saturating_sub(1);
    for (i, step) in plan.steps.iter().enumerate() {
        let (source, entry) = kernel_source(step.kernel, caps);
        builder.push(Pass {
            name: if i == last {
                format!("RIFE のネットワーク（{model_name}、{}x{}）", run[0], run[1])
            } else {
                format!("rife {entry} {}", step.note)
            },
            kind: Kind::Compute,
            source,
            entry: entry.to_owned(),
            writes: String::new(),
            groups: step.groups,
            buffers: step
                .bindings
                .iter()
                .map(|&(binding, slot)| bind(binding, slot))
                .collect(),
            // カーネルはどれも共有メモリを書いてから読む。
            zero_workgroup: false,
            ..Pass::default()
        });
    }

    builder.graph.retime_from = builder.graph.passes.len();
    let out = builder.texture(&format!("u{unit}_rife"), size);
    let mut merge_source = declare(&reads_decl, Some("rgba16float"));
    merge_source.push_str(MERGE);
    builder.push(Pass {
        name: String::from("RIFE の位相"),
        kind: Kind::Compute,
        source: merge_source,
        entry: String::from("merge"),
        reads: vec![prev, cur],
        writes: out.clone(),
        block: [8, 8],
        buffers: vec![
            Bind {
                access: Access::Read,
                ..bind(0, Slot::Buffer(plan.flow0.buffer))
            },
            Bind {
                access: Access::Read,
                ..bind(1, Slot::Buffer(plan.flow1.buffer))
            },
            Bind {
                access: Access::Read,
                ..bind(2, Slot::Buffer(plan.mask.buffer))
            },
            Bind {
                binding: 3,
                name: params_name,
                access: Access::Uniform,
                offset: merge_at,
                size: 32,
            },
        ],
        ..Pass::default()
    });
    builder.main = (out, size);
    zwsdk::plug::info(&format!(
        "RIFE（GPU 共通）：{model_name}、フローは {}x{}（{scale}）で、{} 回の dispatch・{:.1} G 積和{}{}",
        run[0],
        run[1],
        plan.steps.len(),
        plan.macs / 1e9,
        if caps.half { "、f16" } else { "" },
        if caps.tc16 { "、協調行列" } else { "" },
    ));
    Ok(())
}

fn check(plan: &Plan, device: &Device, before: usize) -> Result<(), String> {
    let biggest = plan.buffers.iter().copied().max().unwrap_or(0) * 4;
    let most = device.limits.storage_binding.min(device.limits.buffer);
    if biggest > most {
        return Err(format!(
            "RIFE（GPU 共通）：作業バッファ（{} MB）がこの GPU の上限（{} MB）を超えます。フローの大きさ（{SCALE}）を小さくしてください。",
            biggest >> 20,
            most >> 20
        ));
    }
    let passes = before + plan.steps.len() + 2;
    if passes > zwsdk::video::MOST_PASSES {
        return Err(format!("RIFE（GPU 共通）：パスが多すぎます（{passes}）。"));
    }
    if let Some(step) = plan
        .steps
        .iter()
        .find(|s| s.groups.iter().any(|n| *n > device.limits.groups))
    {
        return Err(format!(
            "RIFE（GPU 共通）：ワークグループの数 {:?} がこの GPU の上限を超えます。",
            step.groups
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scale_is_read_from_the_settings() {
        let of = |v: &str| scale_of(&[(SCALE.to_owned(), v.to_owned())].into());
        assert_eq!(of("0.25"), 0.25);
        assert_eq!(of("1/2"), 0.5);
        assert_eq!(of("2"), 1.0, "広げはしない");
        assert_eq!(of("x"), DEFAULT_SCALE);
        assert_eq!(scale_of(&BTreeMap::new()), DEFAULT_SCALE);
    }

    #[test]
    fn tensor_cores_need_the_matrix_and_warps_of_32() {
        let mut device = Device {
            backend: String::from("vulkan"),
            compute: true,
            shader_f16: true,
            matrices: vec![zwsdk::video::Matrix {
                m: 16,
                n: 16,
                k: 16,
                ab: String::from("f16"),
                cr: String::from("f32"),
            }],
            ..Device::default()
        };
        device.limits.subgroup = [32, 32];
        device.limits.workgroup_storage = 32 << 10;
        assert!(caps_of(&device).tc16);
        device.limits.subgroup = [8, 32];
        assert!(
            !caps_of(&device).tc16,
            "Intel はサブグループの大きさが変わる"
        );
        device.backend = String::from("dx12");
        device.shader_f16 = false;
        let caps = caps_of(&device);
        assert!(!caps.half && !caps.tc16);
    }

    const TINY: &str = "7767517
17 30
Input in0 0 1 in0
Split s0 1 2 in0 a0 a1
Input in1 0 1 in1
Split s1 1 2 in1 b0 b1
Input in2 0 1 in2
Concat cat 3 1 a0 b0 in2 x
Interp down 1 1 x xs 0=2 1=0.5 2=0.5
Convolution conv 1 1 xs y 0=16 1=3 4=1 5=1 6=1008 9=2 -23310=1,0.2
Convolution conv2 1 1 y z 0=16 1=3 4=1 5=1 6=2304
Interp up 1 1 z zu 0=2 1=2.0 2=2.0
Convolution head 1 1 zu f 0=5 1=3 4=1 5=1 6=720
Split s2 1 3 f f0 f1 f2
Crop c0 1 1 f0 fl0 -23309=1,0 -23310=1,2 -23311=1,0
Crop c1 1 1 f1 fl1 -23309=1,2 -23310=1,4 -23311=1,0
Crop c2 1 1 f2 mk -23309=1,4 -23310=1,5 -23311=1,0
Sigmoid sig 1 1 mk m
rife.Warp w0 2 1 a1 fl0 o0
rife.Warp w1 2 1 b1 fl1 o1
BinaryOp mix 2 1 o0 o1 out0 0=0
";

    fn tiny_bin() -> Vec<u8> {
        let mut bytes = Vec::new();
        for (count, out) in [(1008usize, 16usize), (2304, 16), (720, 5)] {
            bytes.extend(0u32.to_le_bytes());
            for i in 0..count + out {
                bytes.extend(((i % 7) as f32 * 0.01).to_le_bytes());
            }
        }
        bytes
    }

    #[test]
    fn a_tiny_network_becomes_a_graph() {
        let root = std::env::temp_dir().join(format!("zw-rife-gpu-{}", std::process::id()));
        let model = root.join("tiny");
        std::fs::create_dir_all(&model).unwrap();
        std::fs::write(model.join("flownet.param"), TINY).unwrap();
        std::fs::write(model.join("flownet.bin"), tiny_bin()).unwrap();
        zwsdk::plug::host::files::set_root(MODELS, &root);
        let env = Env {
            at: zwsdk::video::At::Picture,
            format: zwsdk::video::Format {
                width: 1440,
                height: 1080,
                depth: 8,
                interlaced: false,
                aspect: [16, 9],
                transfer: String::from("sdr"),
            },
            input: [1440, 1080],
            target: [1920, 1080],
            rate: [30000, 1001],
            display: 120.0,
            device: Device {
                backend: String::from("vulkan"),
                compute: true,
                shader_f16: true,
                ..Device::default()
            },
            settings: [
                (MODEL.to_owned(), String::from("tiny")),
                (SCALE.to_owned(), String::from("0.5")),
            ]
            .into(),
            program: None,
        };
        let graph = crate::assemble(&env, FILE, &|_| Some(String::new())).unwrap();
        graph.check(zwsdk::video::At::Picture).unwrap();
        assert_eq!(graph.output, [1440, 1080]);
        assert_eq!(graph.retime, 120.0);
        assert_eq!(graph.history, 1);
        let last = graph.passes.last().unwrap();
        assert_eq!(last.writes, "output");
        assert_eq!(
            graph.retime_from,
            graph.passes.len() - 1,
            "位相のパスだけ毎回"
        );
        assert!(
            graph.passes[..graph.passes.len() - 1]
                .iter()
                .all(Pass::dispatches)
        );
        assert!(!graph.passes.iter().any(|p| p.entry == "warp"));
        assert!(
            graph.passes.iter().any(|p| p.entry == "gemm_h"),
            "f16 の Winograd"
        );
        assert!(graph.passes.iter().any(|p| !p.zero_workgroup));
        assert_eq!(graph.blobs.len(), 2);
        for buffer in graph.buffers.values() {
            if let Some(at) = &buffer.blob {
                assert!(at.offset < graph.blobs[&at.name].0.len() as u64);
            }
        }
        for pass in &graph.passes {
            for bind in pass.buffers.iter().filter(|b| b.access == Access::Uniform) {
                assert_eq!(bind.offset % 256, 0, "{}", pass.name);
            }
            let source = &pass.source;
            naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|e| panic!("{}：{}", pass.name, e.emit_to_string(source)));
        }
        let mut missing = env.clone();
        missing
            .settings
            .insert(MODEL.to_owned(), String::from("nothing"));
        let why = crate::assemble(&missing, FILE, &|_| Some(String::new())).unwrap_err();
        assert!(why.contains("flownet.param"), "{why}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn loops_are_unrolled_in_place() {
        let text =
            "a\n  //@unroll i 0..2\n  x{i}\n  //@unroll j 1..3\n  y{i}{j}\n  //@end\n  //@end\nb";
        assert_eq!(unroll(text), "a\n  x0\n  y01\n  y02\n  x1\n  y11\n  y12\nb");
        assert_eq!(
            unroll("//@unroll i x..y\nz"),
            "//@unroll i x..y\nz",
            "読めない物はそのまま"
        );
        let caps = Caps {
            half: true,
            tc16: false,
            reduce_below: 600,
        };
        for kernel in [Kernel::WinoGemmHalf, Kernel::WinoIn, Kernel::Nn("conv3s")] {
            let (source, _) = kernel_source(kernel, caps);
            assert!(!source.contains("//@"), "{kernel:?}");
            assert!(!source.contains("{kk}") && !source.contains("{r}") && !source.contains("{j}"));
        }
    }

    #[test]
    fn every_kernel_source_parses() {
        for caps in [
            Caps {
                half: false,
                tc16: false,
                reduce_below: 600,
            },
            Caps {
                half: true,
                tc16: false,
                reduce_below: 600,
            },
        ] {
            let mut kernels = vec![
                Kernel::Nn("conv"),
                Kernel::Nn("conv3s"),
                Kernel::Nn("deconv4"),
                Kernel::WinoIn,
                Kernel::WinoGemm,
                Kernel::WinoOut,
            ];
            if caps.half {
                kernels.push(Kernel::WinoGemmHalf);
            }
            for kernel in kernels {
                let (source, entry) = kernel_source(kernel, caps);
                let module = naga::front::wgsl::parse_str(&source)
                    .unwrap_or_else(|e| panic!("{kernel:?}：{}", e.emit_to_string(&source)));
                assert!(
                    module.entry_points.iter().any(|e| e.name == entry),
                    "{entry}"
                );
            }
        }
        for (extra, body) in [(None, PACK), (Some("rgba16float"), MERGE)] {
            let reads = [
                (String::from("prev"), "texture_2d<f32>"),
                (String::from("cur"), "texture_2d<f32>"),
            ];
            let source = format!("{}{body}", declare(&reads, extra));
            naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        }
    }
}
