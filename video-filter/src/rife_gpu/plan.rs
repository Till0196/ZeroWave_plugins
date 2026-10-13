// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 層を順に通して、dispatch の並びとバッファを決める。
//! 切り出し（Crop）はコピーせずに位置をずらし、結合（Concat）は作る側が結合先へ直接書く。
//! 3×3 の畳み込みは Winograd F(4,3)（入出力が 16 以上）、テンソルコアがあればその行列積。

use std::collections::{HashMap, HashSet};

use super::model::{Form, Layer, has};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Caps {
    pub half: bool,
    pub tc16: bool,
    /// 画素がこれ以下で入力の多い 3×3 は、入力をスレッドで分ける物で。
    pub reduce_below: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kernel {
    Nn(&'static str),
    Tc(u32),
    WinoIn,
    WinoGemm,
    WinoGemmHalf,
    WinoOut,
    WinoTc,
    WinoTcOc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Buffer(usize),
    Weights(usize),
    Uniform(usize),
}

#[derive(Debug, Clone)]
pub struct Step {
    pub kernel: Kernel,
    pub note: String,
    pub bindings: Vec<(u32, Slot)>,
    pub groups: [u32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    pub c: u32,
    pub h: u32,
    pub w: u32,
}

impl Shape {
    pub fn len(&self) -> u64 {
        u64::from(self.c) * u64::from(self.h) * u64::from(self.w)
    }

    pub fn plane(&self) -> u32 {
        self.h * self.w
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    pub buffer: usize,
    pub offset: u32,
    pub shape: Shape,
}

#[derive(Debug)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub buffers: Vec<u64>,
    pub uniforms: Vec<[u32; 24]>,
    pub weights: Vec<(usize, Form)>,
    pub inputs: [View; 3],
    pub flow0: View,
    pub flow1: View,
    pub mask: View,
    pub padded: Shape,
    pub macs: f64,
}

// `P` の並び（nn.wgsl）。
const C_IN: usize = 0;
const H_IN: usize = 1;
const W_IN: usize = 2;
const C_OUT: usize = 3;
const H_OUT: usize = 4;
const W_OUT: usize = 5;
const STRIDE: usize = 6;
const PAD: usize = 7;
const KERNEL: usize = 8;
const ACT: usize = 9;
const OP: usize = 10;
const C_B: usize = 11;
const SLOPE: usize = 12;
const SCALAR: usize = 13;
const SCALE_H: usize = 14;
const SCALE_W: usize = 15;
const COEF_A: usize = 16;
const COEF_B: usize = 17;
const BIAS_AT: usize = 18;
const HAS_BIAS: usize = 19;
const X_OFF: usize = 20;
const Y_OFF: usize = 21;
const OUT_OFF: usize = 22;
const TILES: usize = 8;

fn flat_groups(n: u64) -> [u32; 3] {
    let groups = n.div_ceil(64);
    if groups <= 65535 {
        [groups as u32, 1, 1]
    } else {
        [65535, groups.div_ceil(65535) as u32, 1]
    }
}

const PLACEABLE: [&str; 9] = [
    "Convolution",
    "Deconvolution",
    "Interp",
    "PixelShuffle",
    "BinaryOp",
    "Eltwise",
    "ReLU",
    "Sigmoid",
    "rife.Warp",
];

#[derive(Default)]
struct Walked {
    shapes: HashMap<String, Shape>,
    canon: HashMap<String, String>,
    producer: HashMap<String, String>,
    buffers: Vec<u64>,
    uniforms: Vec<[u32; 24]>,
    steps: Vec<Step>,
    weights: Vec<(usize, Form)>,
    inputs: Vec<View>,
    warps: Vec<(View, View)>,
    sigmoid: Option<View>,
    macs: f64,
    wino_u: u64,
    wino_m: u64,
}

impl Walked {
    fn alloc(&mut self, shape: Shape) -> View {
        self.buffers.push(shape.len().max(1));
        View {
            buffer: self.buffers.len() - 1,
            offset: 0,
            shape,
        }
    }

    fn uniform(&mut self, words: &[(usize, u32)], floats: &[(usize, f32)]) -> usize {
        let mut slots = [0u32; 24];
        for &(i, v) in words {
            slots[i] = v;
        }
        for &(i, v) in floats {
            slots[i] = v.to_bits();
        }
        self.uniforms.push(slots);
        self.uniforms.len() - 1
    }

    fn weight(&mut self, layer: usize, form: Form) -> Slot {
        let at = match self.weights.iter().position(|w| *w == (layer, form)) {
            Some(at) => at,
            None => {
                self.weights.push((layer, form));
                self.weights.len() - 1
            }
        };
        Slot::Weights(at)
    }
}

/// パディングの単位：一番粗いレベルの縮小率の逆数 ×4（v4.6 は 32、4.25-lite は 128）。
pub fn unit(layers: &[Layer]) -> u32 {
    let coarsest = layers
        .iter()
        .filter(|l| l.kind == "Interp")
        .map(|l| l.float(1, 1.0))
        .filter(|s| *s > 0.0 && *s < 1.0)
        .fold(1.0f32, f32::min);
    ((1.0 / coarsest).round() as u32 * 4).max(32)
}

pub fn plan(layers: &[Layer], run: [u32; 2], caps: Caps) -> Result<Plan, String> {
    let unit = unit(layers);
    let padded = Shape {
        c: 3,
        h: run[1].div_ceil(unit) * unit,
        w: run[0].div_ceil(unit) * unit,
    };
    let dry = walk(layers, padded, &HashMap::new(), None, caps)?;
    let mut place: HashMap<String, (String, u32)> = HashMap::new();
    for layer in layers.iter().filter(|l| l.kind == "Concat") {
        let sources: Vec<String> = layer
            .bottoms
            .iter()
            .map(|b| dry.canon.get(b).cloned().unwrap_or_else(|| b.clone()))
            .collect();
        let mut channel = 0;
        for source in &sources {
            let shape = dry.shapes[source];
            let once = sources.iter().filter(|s| *s == source).count() == 1;
            let placeable = dry
                .producer
                .get(source)
                .is_some_and(|k| PLACEABLE.contains(&k.as_str()));
            if once && placeable && !place.contains_key(source) {
                place.insert(source.clone(), (layer.tops[0].clone(), channel));
            }
            channel += shape.c;
        }
    }
    let walked = walk(layers, padded, &place, Some(&dry), caps)?;
    let [a, b, t] = walked.inputs[..] else {
        return Err(format!(
            "入力が三つではありません（{}）。",
            walked.inputs.len()
        ));
    };
    // 最後の二つの rife.Warp：元の画像が in0 の物が前の側（マスクを掛ける側）。
    let last: Vec<(View, View)> = walked.warps.iter().rev().take(2).copied().collect();
    let flow_of = |input: View| {
        last.iter()
            .find(|(image, _)| image.buffer == input.buffer && image.offset == input.offset)
            .map(|(_, flow)| *flow)
    };
    let (Some(flow0), Some(flow1), Some(mask)) = (flow_of(a), flow_of(b), walked.sigmoid) else {
        return Err(String::from("最後のフローとマスクが見つかりません。"));
    };
    Ok(Plan {
        steps: walked.steps,
        buffers: walked.buffers,
        uniforms: walked.uniforms,
        weights: walked.weights,
        inputs: [a, b, t],
        flow0,
        flow1,
        mask,
        padded,
        macs: walked.macs,
    })
}

fn walk(
    layers: &[Layer],
    padded: Shape,
    place: &HashMap<String, (String, u32)>,
    dry: Option<&Walked>,
    caps: Caps,
) -> Result<Walked, String> {
    let mut w = Walked::default();
    let mut by_name: HashMap<String, View> = HashMap::new();
    let mut concat_dest: HashMap<String, View> = HashMap::new();
    let pre = dry.map(|d| &d.shapes);

    // Winograd の作業用（U と M）は一番大きい層に合わせて一度だけ。U はテンソルコアの
    // 行列積が最後の 64 行の組で読み過ぎる分をパディングする。
    let scratch = dry.map(|d| {
        let u = w.alloc(Shape {
            c: 1,
            h: 1,
            w: (d.wino_u.max(1) + 64 * 256) as u32,
        });
        let m = w.alloc(Shape {
            c: 1,
            h: 1,
            w: d.wino_m.max(1) as u32,
        });
        (u.buffer, m.buffer)
    });
    let mut uses: HashMap<&str, usize> = HashMap::new();
    for layer in layers {
        for b in &layer.bottoms {
            *uses.entry(b.as_str()).or_default() += 1;
        }
    }
    let consumer = |name: &str| -> Option<usize> {
        if uses.get(name).copied() != Some(1) {
            return None;
        }
        layers
            .iter()
            .position(|l| l.bottoms.iter().any(|b| b == name))
    };
    let mut skip: HashSet<usize> = HashSet::new();
    for (index, layer) in layers.iter().enumerate() {
        if skip.contains(&index) {
            continue;
        }
        let src: Vec<View> = layer
            .bottoms
            .iter()
            .map(|b| {
                by_name
                    .get(b)
                    .copied()
                    .ok_or_else(|| format!("{}：入力 {b} がありません。", layer.name))
            })
            .collect::<Result<_, _>>()?;
        let top = layer.tops.first().cloned().unwrap_or_default();
        let out_view_for = |name: &String,
                            shape: Shape,
                            w: &mut Walked,
                            concat_dest: &mut HashMap<String, View>|
         -> View {
            if let Some((dest, channel)) = place.get(name) {
                let dest_view = *concat_dest.entry(dest.clone()).or_insert_with(|| {
                    let shape = pre.and_then(|p| p.get(dest).copied()).unwrap_or(shape);
                    w.alloc(shape)
                });
                View {
                    buffer: dest_view.buffer,
                    offset: dest_view.offset + channel * shape.plane(),
                    shape,
                }
            } else {
                w.alloc(shape)
            }
        };
        let out_view = |shape: Shape, w: &mut Walked, concat_dest: &mut HashMap<String, View>| {
            out_view_for(&top, shape, w, concat_dest)
        };
        let mut produced: Option<Shape> = None;
        match layer.kind.as_str() {
            "Input" => {
                let shape = if layer.name == "in2" {
                    Shape { c: 1, ..padded }
                } else {
                    padded
                };
                let view = w.alloc(shape);
                w.inputs.push(view);
                by_name.insert(top.clone(), view);
                w.producer.insert(top.clone(), layer.kind.clone());
            }
            "Split" => {
                let canon = w
                    .canon
                    .get(&layer.bottoms[0])
                    .cloned()
                    .unwrap_or_else(|| layer.bottoms[0].clone());
                for t in &layer.tops {
                    by_name.insert(t.clone(), src[0]);
                    w.canon.insert(t.clone(), canon.clone());
                    w.shapes.insert(t.clone(), src[0].shape);
                }
            }
            "MemoryData" => {
                let shape = Shape {
                    w: layer.int(0, 1).max(1) as u32,
                    h: layer.int(1, 1).max(1) as u32,
                    c: layer.int(2, 1).max(1) as u32,
                };
                let slot = w.weight(index, Form::Raw);
                let Slot::Weights(at) = slot else {
                    unreachable!()
                };
                let view = View {
                    buffer: usize::MAX - at,
                    offset: 0,
                    shape,
                };
                by_name.insert(top.clone(), view);
                w.producer.insert(top.clone(), layer.kind.clone());
            }
            "Convolution" | "Deconvolution" => {
                let s = src[0].shape;
                let k = layer.int(1, 1) as u32;
                let stride = layer.int(3, 1) as u32;
                let pad = layer.int(4, 0) as u32;
                let c_out = layer.int(0, 0) as u32;
                let deconv = layer.kind == "Deconvolution";
                let (h, wd) = if deconv {
                    (
                        (s.h - 1) * stride + k - 2 * pad,
                        (s.w - 1) * stride + k - 2 * pad,
                    )
                } else {
                    (
                        (s.h + 2 * pad - k) / stride + 1,
                        (s.w + 2 * pad - k) / stride + 1,
                    )
                };
                let shape = Shape { c: c_out, h, w: wd };
                // 書き出しの処理に融合できる並び：（係数）→ 残差 →（leaky）。途中の値が他で使われないときだけ。
                let wino_ok = !deconv && has(layer, Form::Wino);
                let mut fused: Option<(
                    String,
                    Option<View>,
                    Option<View>,
                    Option<f32>,
                    Vec<usize>,
                )> = None;
                if wino_ok && !place.contains_key(&top) {
                    let mut name = top.clone();
                    let mut skipped = Vec::new();
                    let mut beta = None;
                    let mut at = consumer(&name);
                    if let Some(i) = at {
                        let l = &layers[i];
                        if l.kind == "BinaryOp"
                            && l.int(0, 0) == 2
                            && l.int(1, 0) == 0
                            && l.bottoms.len() == 2
                        {
                            let other = if l.bottoms[0] == name {
                                &l.bottoms[1]
                            } else {
                                &l.bottoms[0]
                            };
                            if let Some(v) = by_name
                                .get(other)
                                .filter(|v| v.shape.h == 1 && v.shape.w == 1 && v.shape.c == c_out)
                            {
                                beta = Some(*v);
                                skipped.push(i);
                                name = l.tops[0].clone();
                                at = consumer(&name);
                            }
                        }
                    }
                    if let Some(i) = at {
                        let l = &layers[i];
                        if l.kind == "BinaryOp"
                            && l.int(0, 0) == 0
                            && l.int(1, 0) == 0
                            && l.bottoms.len() == 2
                        {
                            let other = if l.bottoms[0] == name {
                                &l.bottoms[1]
                            } else {
                                &l.bottoms[0]
                            };
                            if let Some(r) = by_name.get(other).filter(|v| v.shape == shape) {
                                let residual = *r;
                                skipped.push(i);
                                name = l.tops[0].clone();
                                let mut slope = None;
                                if let Some(j) = consumer(&name) {
                                    let l = &layers[j];
                                    if l.kind == "ReLU" {
                                        slope = Some(l.float(0, 0.0));
                                        skipped.push(j);
                                        name = l.tops[0].clone();
                                    }
                                }
                                fused = Some((name, Some(residual), beta, slope, skipped));
                            }
                        }
                    }
                }
                let out = match &fused {
                    Some((name, ..)) => out_view_for(name, shape, &mut w, &mut concat_dest),
                    None => out_view(shape, &mut w, &mut concat_dest),
                };
                let (_, bias_at) = layer
                    .weights
                    .as_ref()
                    .ok_or_else(|| format!("{}：重みがありません。", layer.name))?;
                let bias_at = *bias_at as u32;
                let has_bias = layer.int(5, 0) as u32;
                let act = layer.int(9, 0) as u32;
                let slope = layer.array(10).first().copied().unwrap_or(0.0);
                let raw = w.weight(index, Form::Raw);
                w.macs += f64::from(c_out)
                    * f64::from(h)
                    * f64::from(wd)
                    * f64::from(s.c)
                    * f64::from(k * k)
                    / if deconv {
                        f64::from(stride * stride)
                    } else {
                        1.0
                    };
                let note = format!("{}→{} {}x{}", s.c, c_out, wd, h);
                if wino_ok {
                    let tiles = h.div_ceil(4) * wd.div_ceil(4);
                    w.wino_u = w.wino_u.max(36 * u64::from(tiles) * u64::from(s.c));
                    w.wino_m = w.wino_m.max(36 * u64::from(tiles) * u64::from(c_out));
                    if let Some((scratch_u, scratch_m)) = scratch {
                        let u_in = w.uniform(
                            &[
                                (C_IN, s.c),
                                (H_IN, s.h),
                                (W_IN, s.w),
                                (H_OUT, h),
                                (W_OUT, wd),
                                (TILES, tiles),
                                (X_OFF, src[0].offset),
                            ],
                            &[],
                        );
                        let u_gemm = w.uniform(&[(C_IN, s.c), (C_OUT, c_out), (TILES, tiles)], &[]);
                        let (residual, beta, fused_slope) = match &fused {
                            Some((_, r, b, s, _)) => (*r, *b, *s),
                            None => (None, None, None),
                        };
                        let flags =
                            u32::from(residual.is_some()) | (u32::from(beta.is_some()) << 1);
                        let (act, slope) = match fused_slope {
                            Some(s) => (2, s),
                            None => (act, slope),
                        };
                        let u_out = w.uniform(
                            &[
                                (C_OUT, c_out),
                                (H_OUT, h),
                                (W_OUT, wd),
                                (TILES, tiles),
                                (ACT, act),
                                (BIAS_AT, bias_at),
                                (HAS_BIAS, has_bias),
                                (OUT_OFF, out.offset),
                                (OP, flags),
                                (Y_OFF, residual.map_or(0, |v| v.offset)),
                                (C_B, beta.map_or(0, |v| v.offset)),
                            ],
                            &[(SLOPE, slope)],
                        );
                        w.steps.push(Step {
                            note: note.clone(),
                            kernel: Kernel::WinoIn,
                            bindings: vec![
                                (0, slot_of(src[0])),
                                (7, Slot::Buffer(scratch_u)),
                                (4, Slot::Uniform(u_in)),
                            ],
                            groups: [tiles.div_ceil(16), s.c.div_ceil(8), 1],
                        });
                        let tc = caps.tc16 && caps.half && has(layer, Form::Wino16);
                        if tc {
                            let w16 = w.weight(index, Form::Wino16);
                            w.steps.push(Step {
                                note: note.clone(),
                                kernel: if c_out % 64 == 0 {
                                    Kernel::WinoTc
                                } else {
                                    Kernel::WinoTcOc
                                },
                                bindings: vec![
                                    (5, Slot::Buffer(scratch_u)),
                                    (8, Slot::Buffer(scratch_m)),
                                    (10, w16),
                                    (4, Slot::Uniform(u_gemm)),
                                ],
                                groups: [tiles.div_ceil(64), c_out.div_ceil(64), 36],
                            });
                        } else {
                            let weights = w.weight(index, Form::Wino);
                            w.steps.push(Step {
                                note: note.clone(),
                                kernel: if caps.half {
                                    Kernel::WinoGemmHalf
                                } else {
                                    Kernel::WinoGemm
                                },
                                bindings: vec![
                                    (5, Slot::Buffer(scratch_u)),
                                    (8, Slot::Buffer(scratch_m)),
                                    (3, weights),
                                    (4, Slot::Uniform(u_gemm)),
                                ],
                                groups: [c_out.div_ceil(64), tiles.div_ceil(64), 36],
                            });
                        }
                        w.steps.push(Step {
                            note,
                            kernel: Kernel::WinoOut,
                            bindings: vec![
                                (6, Slot::Buffer(scratch_m)),
                                (2, Slot::Buffer(out.buffer)),
                                (3, raw),
                                (4, Slot::Uniform(u_out)),
                                (1, residual.map_or(raw, slot_of)),
                                (9, beta.map_or(raw, slot_of)),
                            ],
                            groups: [tiles.div_ceil(16), c_out.div_ceil(8), 1],
                        });
                    }
                    let final_top = fused.as_ref().map_or(top.clone(), |f| f.0.clone());
                    if let Some((_, _, _, _, skipped)) = &fused {
                        skip.extend(skipped.iter().copied());
                    }
                    by_name.insert(final_top.clone(), out);
                    w.producer
                        .insert(final_top.clone(), String::from("Convolution"));
                    w.shapes.insert(final_top, shape);
                    continue;
                }
                let u = w.uniform(
                    &[
                        (C_IN, s.c),
                        (H_IN, s.h),
                        (W_IN, s.w),
                        (C_OUT, c_out),
                        (H_OUT, h),
                        (W_OUT, wd),
                        (STRIDE, stride),
                        (PAD, pad),
                        (KERNEL, k),
                        (ACT, act),
                        (BIAS_AT, bias_at),
                        (HAS_BIAS, has_bias),
                        (X_OFF, src[0].offset),
                        (OUT_OFF, out.offset),
                    ],
                    &[(SLOPE, slope)],
                );
                let transposed = has(layer, Form::Transposed);
                let tensor = caps.tc16 && !deconv && has(layer, Form::Tensor);
                let (kernel, groups) = match (transposed, deconv) {
                    _ if tensor && c_out >= 64 => (
                        Kernel::Tc(64),
                        [wd.div_ceil(16), h.div_ceil(4), c_out.div_ceil(64)],
                    ),
                    _ if tensor => (
                        Kernel::Tc(32),
                        [wd.div_ceil(16), h.div_ceil(4), c_out.div_ceil(32)],
                    ),
                    (true, false) if stride == 1 && wd >= 128 => (
                        Kernel::Nn("conv3"),
                        [wd.div_ceil(64), h.div_ceil(8), c_out.div_ceil(16)],
                    ),
                    (true, false) if wd * h <= caps.reduce_below && s.c >= 64 => {
                        (Kernel::Nn("conv3r"), [wd * h, c_out.div_ceil(4), 1])
                    }
                    (true, false) => (
                        Kernel::Nn("conv3s"),
                        [wd.div_ceil(32), h.div_ceil(8), c_out.div_ceil(16)],
                    ),
                    // 出力が 8 以下の逆畳み込みは 4 出力ずつの物（16 ずつでは大半が無駄）。
                    (true, true) if c_out <= 8 => (
                        Kernel::Nn("deconv4s"),
                        [s.w.div_ceil(8), s.h.div_ceil(8), c_out.div_ceil(4)],
                    ),
                    (true, true) => (
                        Kernel::Nn("deconv4"),
                        [s.w.div_ceil(8), s.h.div_ceil(8), c_out.div_ceil(16)],
                    ),
                    (false, _) => (
                        Kernel::Nn(if deconv { "deconv" } else { "conv" }),
                        [wd.div_ceil(8), h.div_ceil(8), c_out.div_ceil(8)],
                    ),
                };
                let mut bindings = vec![
                    (0, slot_of(src[0])),
                    (2, Slot::Buffer(out.buffer)),
                    (3, raw),
                    (4, Slot::Uniform(u)),
                ];
                if tensor {
                    bindings.push((6, w.weight(index, Form::Tensor)));
                } else if transposed {
                    bindings.push((5, w.weight(index, Form::Transposed)));
                }
                w.steps.push(Step {
                    note,
                    kernel,
                    bindings,
                    groups,
                });
                by_name.insert(top.clone(), out);
                produced = Some(shape);
            }
            "Interp" => {
                let s = src[0].shape;
                let (sh, sw) = (layer.float(1, 1.0), layer.float(2, 1.0));
                if layer.int(0, 2) != 2 {
                    return Err(format!("{}：双線形でない Interp です。", layer.name));
                }
                let shape = Shape {
                    c: s.c,
                    h: (s.h as f32 * sh).round() as u32,
                    w: (s.w as f32 * sw).round() as u32,
                };
                let out = out_view(shape, &mut w, &mut concat_dest);
                let u = w.uniform(
                    &[
                        (C_IN, s.c),
                        (H_IN, s.h),
                        (W_IN, s.w),
                        (C_OUT, shape.c),
                        (H_OUT, shape.h),
                        (W_OUT, shape.w),
                        (X_OFF, src[0].offset),
                        (OUT_OFF, out.offset),
                    ],
                    &[(SCALE_H, sh), (SCALE_W, sw)],
                );
                w.steps.push(Step {
                    note: String::new(),
                    kernel: Kernel::Nn("interp"),
                    bindings: vec![
                        (0, slot_of(src[0])),
                        (2, Slot::Buffer(out.buffer)),
                        (4, Slot::Uniform(u)),
                    ],
                    groups: flat_groups(u64::from(shape.plane())),
                });
                by_name.insert(top.clone(), out);
                produced = Some(shape);
            }
            "PixelShuffle" => {
                let s = src[0].shape;
                let r = layer.int(0, 1) as u32;
                let shape = Shape {
                    c: s.c / (r * r),
                    h: s.h * r,
                    w: s.w * r,
                };
                let out = out_view(shape, &mut w, &mut concat_dest);
                let u = w.uniform(
                    &[
                        (C_IN, s.c),
                        (H_IN, s.h),
                        (W_IN, s.w),
                        (C_OUT, shape.c),
                        (H_OUT, shape.h),
                        (W_OUT, shape.w),
                        (KERNEL, r),
                        (X_OFF, src[0].offset),
                        (OUT_OFF, out.offset),
                    ],
                    &[],
                );
                w.steps.push(Step {
                    note: String::new(),
                    kernel: Kernel::Nn("shuffle"),
                    bindings: vec![
                        (0, slot_of(src[0])),
                        (2, Slot::Buffer(out.buffer)),
                        (4, Slot::Uniform(u)),
                    ],
                    groups: flat_groups(shape.len()),
                });
                by_name.insert(top.clone(), out);
                produced = Some(shape);
            }
            "Crop" => {
                let s = src[0].shape;
                if layer.array(11).first().copied().unwrap_or(0.0) != 0.0 {
                    return Err(format!("{}：チャンネル以外の Crop です。", layer.name));
                }
                let start = layer.array(9).first().copied().unwrap_or(0.0) as u32;
                let end = (layer.array(10).first().copied().unwrap_or(s.c as f32) as u32).min(s.c);
                let view = View {
                    buffer: src[0].buffer,
                    offset: src[0].offset + start * s.plane(),
                    shape: Shape {
                        c: end - start,
                        ..s
                    },
                };
                by_name.insert(top.clone(), view);
                w.producer.insert(top.clone(), layer.kind.clone());
                w.shapes.insert(top.clone(), view.shape);
            }
            "Concat" => {
                let first = src[0].shape;
                let c: u32 = src.iter().map(|v| v.shape.c).sum();
                let shape = Shape { c, ..first };
                let dest = match concat_dest.get(&top) {
                    Some(v) => *v,
                    None => {
                        let v = w.alloc(shape);
                        concat_dest.insert(top.clone(), v);
                        v
                    }
                };
                let mut at = 0;
                for (bottom, v) in layer.bottoms.iter().zip(&src) {
                    if (v.shape.h, v.shape.w) != (first.h, first.w) {
                        return Err(format!("{}：大きさの違う Concat です。", layer.name));
                    }
                    let source = w
                        .canon
                        .get(bottom)
                        .cloned()
                        .unwrap_or_else(|| bottom.clone());
                    let direct = place
                        .get(&source)
                        .is_some_and(|(d, ch)| *d == top && *ch == at);
                    if !direct {
                        let u = w.uniform(
                            &[
                                (H_IN, v.shape.h),
                                (W_IN, v.shape.w),
                                (C_OUT, v.shape.c),
                                (X_OFF, v.offset),
                                (OUT_OFF, dest.offset + at * v.shape.plane()),
                            ],
                            &[],
                        );
                        w.steps.push(Step {
                            note: String::new(),
                            kernel: Kernel::Nn("copy"),
                            bindings: vec![
                                (0, slot_of(*v)),
                                (2, Slot::Buffer(dest.buffer)),
                                (4, Slot::Uniform(u)),
                            ],
                            groups: flat_groups(v.shape.len()),
                        });
                    }
                    at += v.shape.c;
                }
                let view = View { shape, ..dest };
                by_name.insert(top.clone(), view);
                w.producer.insert(top.clone(), layer.kind.clone());
                w.shapes.insert(top.clone(), shape);
            }
            "BinaryOp" => {
                let op = layer.int(0, 0) as u32;
                if layer.int(1, 0) == 1 {
                    let s = src[0].shape;
                    let out = out_view(s, &mut w, &mut concat_dest);
                    let u = w.uniform(
                        &[
                            (C_OUT, s.c),
                            (H_OUT, s.h),
                            (W_OUT, s.w),
                            (OP, op),
                            (X_OFF, src[0].offset),
                            (OUT_OFF, out.offset),
                        ],
                        &[(SCALAR, layer.float(2, 0.0))],
                    );
                    w.steps.push(Step {
                        note: String::new(),
                        kernel: Kernel::Nn("binary_scalar"),
                        bindings: vec![
                            (0, slot_of(src[0])),
                            (2, Slot::Buffer(out.buffer)),
                            (4, Slot::Uniform(u)),
                        ],
                        groups: flat_groups(s.len()),
                    });
                    by_name.insert(top.clone(), out);
                    produced = Some(s);
                } else {
                    let (mut a, mut b) = (src[0], src[1]);
                    // 二つ目が一枚なら全チャンネルへ。一つ目が一枚なら入れ替えられる演算だけ。
                    if a.shape.c == 1 && b.shape.c > 1 && matches!(op, 0 | 2) {
                        std::mem::swap(&mut a, &mut b);
                    }
                    let (sa, sb) = (a.shape, b.shape);
                    // 二つ目が C×1×1 ならチャンネルごとの係数（has_bias を 2 にして示す）。
                    let per_channel =
                        sb.h == 1 && sb.w == 1 && sb.c == sa.c && (sa.h, sa.w) != (1, 1);
                    if !per_channel && ((sa.h, sa.w) != (sb.h, sb.w) || (sb.c != sa.c && sb.c != 1))
                    {
                        return Err(format!(
                            "{}：形の合わない BinaryOp です（{sa:?} {sb:?}）。",
                            layer.name
                        ));
                    }
                    let out = out_view(sa, &mut w, &mut concat_dest);
                    let u = w.uniform(
                        &[
                            (C_OUT, sa.c),
                            (H_OUT, sa.h),
                            (W_OUT, sa.w),
                            (OP, op),
                            (C_B, if per_channel { 0 } else { sb.c }),
                            (HAS_BIAS, if per_channel { 2 } else { 0 }),
                            (X_OFF, a.offset),
                            (Y_OFF, b.offset),
                            (OUT_OFF, out.offset),
                        ],
                        &[],
                    );
                    w.steps.push(Step {
                        note: String::new(),
                        kernel: Kernel::Nn("binary_pair"),
                        bindings: vec![
                            (0, slot_of(a)),
                            (1, slot_of(b)),
                            (2, Slot::Buffer(out.buffer)),
                            (4, Slot::Uniform(u)),
                        ],
                        groups: flat_groups(sa.len()),
                    });
                    by_name.insert(top.clone(), out);
                    produced = Some(sa);
                }
            }
            "Eltwise" => {
                let s = src[0].shape;
                if layer.int(0, 1) != 1 || src.len() != 2 {
                    return Err(format!("{}：二つの和でない Eltwise です。", layer.name));
                }
                let coefs = layer.array(1);
                let out = out_view(s, &mut w, &mut concat_dest);
                let u = w.uniform(
                    &[
                        (C_OUT, s.c),
                        (H_OUT, s.h),
                        (W_OUT, s.w),
                        (X_OFF, src[0].offset),
                        (Y_OFF, src[1].offset),
                        (OUT_OFF, out.offset),
                    ],
                    &[
                        (COEF_A, coefs.first().copied().unwrap_or(1.0)),
                        (COEF_B, coefs.get(1).copied().unwrap_or(1.0)),
                    ],
                );
                w.steps.push(Step {
                    note: String::new(),
                    kernel: Kernel::Nn("weighted"),
                    bindings: vec![
                        (0, slot_of(src[0])),
                        (1, slot_of(src[1])),
                        (2, Slot::Buffer(out.buffer)),
                        (4, Slot::Uniform(u)),
                    ],
                    groups: flat_groups(s.len()),
                });
                by_name.insert(top.clone(), out);
                produced = Some(s);
            }
            "ReLU" | "Sigmoid" => {
                let s = src[0].shape;
                let out = out_view(s, &mut w, &mut concat_dest);
                if layer.kind == "Sigmoid" {
                    w.sigmoid = Some(out);
                }
                let op = u32::from(layer.kind == "Sigmoid");
                let u = w.uniform(
                    &[
                        (C_OUT, s.c),
                        (H_OUT, s.h),
                        (W_OUT, s.w),
                        (OP, op),
                        (X_OFF, src[0].offset),
                        (OUT_OFF, out.offset),
                    ],
                    &[(SLOPE, layer.float(0, 0.0))],
                );
                w.steps.push(Step {
                    note: String::new(),
                    kernel: Kernel::Nn("unary"),
                    bindings: vec![
                        (0, slot_of(src[0])),
                        (2, Slot::Buffer(out.buffer)),
                        (4, Slot::Uniform(u)),
                    ],
                    groups: flat_groups(s.len()),
                });
                by_name.insert(top.clone(), out);
                produced = Some(s);
            }
            "rife.Warp" => {
                w.warps.push((src[0], src[1]));
                let s = src[0].shape;
                let out = out_view(s, &mut w, &mut concat_dest);
                let u = w.uniform(
                    &[
                        (H_IN, s.h),
                        (W_IN, s.w),
                        (C_OUT, s.c),
                        (X_OFF, src[0].offset),
                        (Y_OFF, src[1].offset),
                        (OUT_OFF, out.offset),
                    ],
                    &[],
                );
                w.steps.push(Step {
                    note: String::new(),
                    kernel: Kernel::Nn("warp"),
                    bindings: vec![
                        (0, slot_of(src[0])),
                        (1, slot_of(src[1])),
                        (2, Slot::Buffer(out.buffer)),
                        (4, Slot::Uniform(u)),
                    ],
                    groups: flat_groups(u64::from(s.plane())),
                });
                by_name.insert(top.clone(), out);
                produced = Some(s);
            }
            other => return Err(format!("扱えない層 {other} です。")),
        }
        if let Some(shape) = produced {
            w.producer.insert(top.clone(), layer.kind.clone());
            w.shapes.insert(top.clone(), shape);
        } else if let Some(v) = by_name.get(&top) {
            w.shapes.entry(top.clone()).or_insert(v.shape);
        }
    }
    if dry.is_none() {
        w.steps.clear();
    }
    prune(&mut w);
    Ok(w)
}

fn slot_of(view: View) -> Slot {
    if view.buffer > usize::MAX / 2 {
        Slot::Weights(usize::MAX - view.buffer)
    } else {
        Slot::Buffer(view.buffer)
    }
}

/// 最後のフローとマスクに要らない処理（最後の混ぜ合わせ）を捨てる。位相ごとのフレームは別のパスが作る。
fn prune(w: &mut Walked) {
    let Some(mask) = w.sigmoid else {
        return;
    };
    let mut needed = vec![mask.buffer];
    needed.extend(w.warps.iter().rev().take(2).map(|(_, flow)| flow.buffer));
    let writes_of = |step: &Step| -> Vec<usize> {
        step.bindings
            .iter()
            .filter(|(binding, _)| matches!(binding, 2 | 7 | 8))
            .filter_map(|(_, slot)| match slot {
                Slot::Buffer(i) => Some(*i),
                _ => None,
            })
            .collect()
    };
    let reads_of = |step: &Step| -> Vec<usize> {
        step.bindings
            .iter()
            .filter(|(binding, _)| !matches!(binding, 2 | 7 | 8))
            .filter_map(|(_, slot)| match slot {
                Slot::Buffer(i) => Some(*i),
                _ => None,
            })
            .collect()
    };
    let mut live: HashSet<usize> = needed.into_iter().collect();
    let mut keep = vec![false; w.steps.len()];
    for (i, step) in w.steps.iter().enumerate().rev() {
        if writes_of(step).iter().any(|b| live.contains(b)) {
            keep[i] = true;
            live.extend(reads_of(step));
        }
    }
    let mut at = 0;
    w.steps.retain(|_| {
        at += 1;
        keep[at - 1]
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_follows_the_coarsest_level() {
        let layers = super::super::model::parse_param(
            "7767517\n2 2\nInput in0 0 1 x\nInterp i 1 1 x y 0=2 1=0.125 2=0.125\n",
        )
        .unwrap();
        assert_eq!(unit(&layers), 32);
        let layers = super::super::model::parse_param(
            "7767517\n2 2\nInput in0 0 1 x\nInterp i 1 1 x y 0=2 1=0.03125 2=0.03125\n",
        )
        .unwrap();
        assert_eq!(unit(&layers), 128);
    }
}
