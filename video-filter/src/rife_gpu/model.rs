// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! rife-ncnn-vulkan（MIT）の `flownet.param`・`flownet.bin` を読み、重みを GPU の形に並べる。
//! bin は畳み込みごとに 4 バイトのフラグ（`0x01306B47` なら fp16、0 なら fp32）と重み、その後に
//! バイアス（fp32、フラグ無し）。MemoryData は fp32 のままフラグ無し。

use std::collections::HashMap;

#[derive(Debug, Clone)]
enum Value {
    Int(i64),
    Float(f32),
    Array(Vec<f32>),
}

#[derive(Debug, Clone)]
pub struct Layer {
    pub kind: String,
    pub name: String,
    pub bottoms: Vec<String>,
    pub tops: Vec<String>,
    params: HashMap<i32, Value>,
    pub weights: Option<(Vec<f32>, usize)>,
}

impl Layer {
    pub fn int(&self, key: i32, default: i64) -> i64 {
        match self.params.get(&key) {
            Some(Value::Int(v)) => *v,
            Some(Value::Float(v)) => *v as i64,
            _ => default,
        }
    }

    pub fn float(&self, key: i32, default: f32) -> f32 {
        match self.params.get(&key) {
            Some(Value::Float(v)) => *v,
            Some(Value::Int(v)) => *v as f32,
            _ => default,
        }
    }

    pub fn array(&self, key: i32) -> Vec<f32> {
        match self.params.get(&key) {
            Some(Value::Array(v)) => v.clone(),
            _ => Vec::new(),
        }
    }

    fn channels(&self, k: usize) -> Option<(usize, usize)> {
        let (_, bias_at) = self.weights.as_ref()?;
        let c_out = self.int(0, 0) as usize;
        Some((bias_at / (c_out * k * k).max(1), c_out))
    }
}

pub fn parse_param(text: &str) -> Result<Vec<Layer>, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("7767517") {
        return Err(String::from(
            "ncnn の param ではありません（先頭の番号が違います）。",
        ));
    }
    lines.next();
    let mut layers = Vec::new();
    for line in lines {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }
        let bad = || format!("読めない行：{line}");
        let inputs: usize = parts[2].parse().map_err(|_| bad())?;
        let outputs: usize = parts[3].parse().map_err(|_| bad())?;
        if parts.len() < 4 + inputs + outputs {
            return Err(bad());
        }
        let bottoms = parts[4..4 + inputs]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let tops = parts[4 + inputs..4 + inputs + outputs]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let mut params = HashMap::new();
        for item in &parts[4 + inputs + outputs..] {
            let Some((key, value)) = item.split_once('=') else {
                continue;
            };
            let key: i32 = key.parse().map_err(|_| format!("読めないキー：{item}"))?;
            if key <= -23300 {
                let numbers: Vec<f32> = value
                    .split(',')
                    .skip(1)
                    .filter_map(|v| v.parse().ok())
                    .collect();
                params.insert(-key - 23300, Value::Array(numbers));
            } else if value.contains(['.', 'e', 'E']) {
                let v = value.parse().map_err(|_| format!("読めない値：{item}"))?;
                params.insert(key, Value::Float(v));
            } else {
                let v = value.parse().map_err(|_| format!("読めない値：{item}"))?;
                params.insert(key, Value::Int(v));
            }
        }
        layers.push(Layer {
            kind: parts[0].to_owned(),
            name: parts[1].to_owned(),
            bottoms,
            tops,
            params,
            weights: None,
        });
    }
    let known = [
        "Input",
        "Split",
        "Concat",
        "Convolution",
        "Deconvolution",
        "Interp",
        "PixelShuffle",
        "Crop",
        "BinaryOp",
        "Eltwise",
        "ReLU",
        "Sigmoid",
        "rife.Warp",
        "MemoryData",
    ];
    let unknown: Vec<&str> = layers
        .iter()
        .map(|l| l.kind.as_str())
        .filter(|k| !known.contains(k))
        .collect();
    if !unknown.is_empty() {
        return Err(format!("扱えない層があります：{unknown:?}"));
    }
    Ok(layers)
}

pub fn to_half(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mantissa = bits & 0x7f_ffff;
    if exponent <= 0 {
        if exponent < -10 {
            return sign;
        }
        let m = (mantissa | 0x80_0000) >> (1 - exponent);
        let round = (m >> 12) & 1;
        return sign | ((m >> 13) + round) as u16;
    }
    if exponent >= 31 {
        return sign | 0x7c00;
    }
    let mut half = sign as u32 | ((exponent as u32) << 10) | (mantissa >> 13);
    let rest = mantissa & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && half & 1 == 1) {
        half += 1;
    }
    half as u16
}

fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = (bits >> 10) & 0x1f;
    let fraction = f32::from(bits & 0x3ff);
    match exponent {
        0 => sign * fraction * 2f32.powi(-24),
        31 => sign * f32::INFINITY,
        e => sign * (1.0 + fraction / 1024.0) * 2f32.powi(i32::from(e) - 15),
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn u32(&mut self) -> Result<u32, String> {
        let b = self
            .bytes
            .get(self.at..self.at + 4)
            .ok_or("bin が足りません。")?;
        self.at += 4;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn floats(&mut self, count: usize) -> Result<Vec<f32>, String> {
        (0..count).map(|_| self.u32().map(f32::from_bits)).collect()
    }

    fn tagged(&mut self, count: usize) -> Result<Vec<f32>, String> {
        match self.u32()? {
            0x0130_6B47 => {
                let b = self
                    .bytes
                    .get(self.at..self.at + count * 2)
                    .ok_or("bin が足りません。")?;
                self.at += (count * 2).div_ceil(4) * 4;
                Ok(b.chunks_exact(2)
                    .map(|h| half(u16::from_le_bytes([h[0], h[1]])))
                    .collect())
            }
            0 => self.floats(count),
            other => Err(format!("読めない重みのフラグ {other:#x}")),
        }
    }
}

pub fn load_weights(layers: &mut [Layer], bytes: &[u8]) -> Result<(), String> {
    let mut reader = Reader { bytes, at: 0 };
    for layer in layers.iter_mut() {
        match layer.kind.as_str() {
            "Convolution" | "Deconvolution" => {
                let count = layer.int(6, 0) as usize;
                let mut weights = reader.tagged(count)?;
                let bias_at = weights.len();
                if layer.int(5, 0) == 1 {
                    weights.extend(reader.floats(layer.int(0, 0) as usize)?);
                }
                layer.weights = Some((weights, bias_at));
            }
            "MemoryData" => {
                let (w, h, c) = (
                    layer.int(0, 1).max(1),
                    layer.int(1, 1).max(1),
                    layer.int(2, 1).max(1),
                );
                let weights = reader.floats((w * h * c) as usize)?;
                layer.weights = Some((weights, 0));
            }
            _ => {}
        }
    }
    if reader.at != bytes.len() {
        return Err(format!(
            "bin の大きさが param と合いません（読んだ {}、全部 {}）。",
            reader.at,
            bytes.len()
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Form {
    Raw,
    /// 16 出力が続く形：[16 の組][入力][カーネルの位置][16]（3×3 と 4×4 の逆畳み込み）。
    Transposed,
    /// テンソルコアの 3×3：f16 の [カーネルの位置][入力（16 の倍数）][出力（32 か 64 の倍数）]。
    Tensor,
    /// Winograd F(4,3) の U = G g Gᵀ：[36][入力][出力]、f32。
    Wino,
    Wino16,
}

const G: [[f32; 3]; 6] = [
    [0.25, 0.0, 0.0],
    [-1.0 / 6.0, -1.0 / 6.0, -1.0 / 6.0],
    [-1.0 / 6.0, 1.0 / 6.0, -1.0 / 6.0],
    [1.0 / 24.0, 1.0 / 12.0, 1.0 / 6.0],
    [1.0 / 24.0, -1.0 / 12.0, 1.0 / 6.0],
    [0.0, 0.0, 1.0],
];

fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn halves(values: &[u16]) -> Vec<u8> {
    let mut raw: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    raw.resize(raw.len().div_ceil(4).max(1) * 4, 0);
    raw
}

fn winograd(g: &[f32], mut put: impl FnMut(usize, f32)) {
    let mut gg = [[0f32; 3]; 6];
    for (r, row) in gg.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| G[r][k] * g[k * 3 + c]).sum();
        }
    }
    for (r, row) in gg.iter().enumerate() {
        for (c, other) in G.iter().enumerate() {
            put(r * 6 + c, (0..3).map(|k| row[k] * other[k]).sum());
        }
    }
}

pub fn weights_of(layer: &Layer, form: Form) -> Result<Vec<u8>, String> {
    let (data, _) = layer
        .weights
        .as_ref()
        .ok_or_else(|| format!("{}：重みがありません。", layer.name))?;
    let k = layer.int(1, 1) as usize;
    let kk = k * k;
    let (c_in, c_out) = layer.channels(k).unwrap_or((0, 0));
    Ok(match form {
        Form::Raw if data.is_empty() => vec![0; 4],
        Form::Raw => floats(data),
        Form::Transposed => {
            let groups = c_out.div_ceil(16);
            let mut t = vec![0f32; groups * c_in * kk * 16];
            for oc in 0..c_out {
                for ic in 0..c_in {
                    for kp in 0..kk {
                        t[((oc / 16 * c_in + ic) * kk + kp) * 16 + oc % 16] =
                            data[(oc * c_in + ic) * kk + kp];
                    }
                }
            }
            floats(&t)
        }
        Form::Tensor => {
            // 出力が 64 以上なら 64 ずつのカーネルで処理するので、64 の倍数にパディングする。
            let ocw = if c_out >= 64 { 64 } else { 32 };
            let (cinp, ocp) = (c_in.div_ceil(16) * 16, c_out.div_ceil(ocw) * ocw);
            let mut t = vec![0u16; 9 * cinp * ocp];
            for oc in 0..c_out {
                for ic in 0..c_in {
                    for tap in 0..9 {
                        t[(tap * cinp + ic) * ocp + oc] = to_half(data[(oc * c_in + ic) * 9 + tap]);
                    }
                }
            }
            halves(&t)
        }
        Form::Wino => {
            let mut t = vec![0f32; 36 * c_in * c_out];
            for oc in 0..c_out {
                for ic in 0..c_in {
                    let g = &data[(oc * c_in + ic) * 9..(oc * c_in + ic) * 9 + 9];
                    winograd(g, |b, v| t[(b * c_in + ic) * c_out + oc] = v);
                }
            }
            floats(&t)
        }
        Form::Wino16 => {
            let mut t = vec![0u16; 36 * c_in * c_out];
            for oc in 0..c_out {
                for ic in 0..c_in {
                    let g = &data[(oc * c_in + ic) * 9..(oc * c_in + ic) * 9 + 9];
                    winograd(g, |b, v| t[(b * c_in + ic) * c_out + oc] = to_half(v));
                }
            }
            // 出力の最後の組でサブグループが読み過ぎる分（16 行 × 64 出力）。
            t.resize(t.len() + 16 * 64, 0);
            halves(&t)
        }
    })
}

pub fn has(layer: &Layer, form: Form) -> bool {
    let k = layer.int(1, 1);
    let stride = layer.int(3, 1);
    let pad = layer.int(4, 0);
    let conv3 = layer.kind == "Convolution" && k == 3 && pad == 1;
    let Some((c_in, c_out)) = layer.channels(k as usize) else {
        return false;
    };
    match form {
        Form::Raw => true,
        Form::Transposed => match layer.kind.as_str() {
            "Convolution" => k == 3 && pad == 1,
            "Deconvolution" => k == 4 && stride == 2 && pad == 1,
            _ => false,
        },
        Form::Tensor => conv3 && stride == 1,
        Form::Wino => conv3 && stride == 1 && c_in >= 16 && c_out >= 16,
        Form::Wino16 => {
            conv3 && stride == 1 && c_in >= 16 && c_out >= 16 && c_in % 16 == 0 && c_out % 16 == 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_round_trip() {
        for v in [0.0f32, 1.0, -2.5, 0.333, 65504.0, 1e-5] {
            let back = half(to_half(v));
            assert!((back - v).abs() <= v.abs() * 1e-3 + 1e-7, "{v} → {back}");
        }
    }

    const PARAM: &str = "7767517
4 4
Input in0 0 1 in0
Convolution conv0 1 1 in0 c0 0=16 1=3 4=1 5=1 6=432 9=2 -23310=1,0.2
Convolution conv1 1 1 c0 c1 0=16 1=3 4=1 5=1 6=2304
Sigmoid sig 1 1 c1 out0
";

    fn bin() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(floats(&vec![0.5; 432]));
        bytes.extend(floats(&[0.1; 16]));
        bytes.extend(0x0130_6B47u32.to_le_bytes());
        bytes.extend(halves(&vec![to_half(0.25); 2304]));
        bytes.extend(floats(&[0.0; 16]));
        bytes
    }

    #[test]
    fn a_param_and_bin_are_read() {
        let mut layers = parse_param(PARAM).unwrap();
        load_weights(&mut layers, &bin()).unwrap();
        assert_eq!(layers.len(), 4);
        let conv0 = &layers[1];
        assert_eq!(conv0.int(0, 0), 16);
        assert_eq!(conv0.array(10), vec![0.2]);
        assert_eq!(conv0.weights.as_ref().unwrap().1, 432);
        assert_eq!(layers[2].weights.as_ref().unwrap().0[0], 0.25);
        assert!(!has(conv0, Form::Wino), "入力が 3 では Winograd にしない");
        assert!(has(&layers[2], Form::Wino16));
        assert!(load_weights(&mut layers, &bin()[..100]).is_err());
        assert!(parse_param("1\n").is_err());
        assert!(parse_param("7767517\n1 1\nLSTM a 1 1 x y\n").is_err());
    }

    #[test]
    fn winograd_keeps_the_sum_of_a_flat_kernel() {
        let mut seen = [0f32; 36];
        winograd(&[1.0; 9], |b, v| seen[b] = v);
        assert!((seen[0] - 0.0625).abs() < 1e-6);
        assert!((seen[35] - 1.0).abs() < 1e-6);
    }
}
