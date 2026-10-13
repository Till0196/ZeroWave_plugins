// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! mpv のユーザーシェーダー（hook 形式の GLSL）の単位をパイプラインに。パスはフック先の順に並べ替える。
//! `RGB`〜`PREKERNEL` は現在の画像、`POSTKERNEL`・`SCALED`・`OUTPUT` は画面の大きさへ縮めた画像、
//! `LUMA` は BT.709 で取り出した輝度（済んだら差を RGB に足して戻す）。ほかは受けない。

use std::collections::BTreeMap;

use zwsdk::video::{Kind, Pass, Texture};

use crate::build::Builder;
use crate::glsl::{self, Bound};
use crate::text::{Block, blocks, rpn, rpn_size};

fn rank(stage: &str) -> Option<u32> {
    Some(match stage {
        "LUMA" => 0,
        "RGB" => 1,
        "NATIVE" => 2,
        "MAINPRESUB" => 3,
        "MAIN" => 4,
        "LINEAR" => 5,
        "SIGMOID" => 6,
        "PREKERNEL" => 7,
        "POSTKERNEL" => 8,
        "SCALED" => 9,
        "OUTPUT" => 10,
        _ => return None,
    })
}

fn after_scaling(stage: &str) -> bool {
    rank(stage).is_some_and(|r| r >= 8)
}

#[derive(Debug)]
struct MpvPass<'a> {
    file: &'a str,
    order: usize,
    block: &'a Block,
    stage: &'static str,
}

fn stage_of(block: &Block) -> Option<&'static str> {
    const KNOWN: [&str; 11] = [
        "RGB",
        "NATIVE",
        "MAINPRESUB",
        "MAIN",
        "LINEAR",
        "SIGMOID",
        "PREKERNEL",
        "POSTKERNEL",
        "SCALED",
        "OUTPUT",
        "LUMA",
    ];
    let hooks = block.all("HOOK");
    KNOWN.iter().find(|known| hooks.contains(known)).copied()
}

fn user_texture(file: &str, block: &Block) -> Result<(String, Texture), String> {
    let name = block.header("TEXTURE").unwrap_or_default().to_owned();
    let what = |e: &str| format!("{file}（{name}）：{e}");
    let size: Vec<u32> = block
        .header("SIZE")
        .unwrap_or("1")
        .split_whitespace()
        .map(|n| n.parse().map_err(|_| what("SIZE が数値ではありません。")))
        .collect::<Result<_, _>>()?;
    let format = block.header("FORMAT").unwrap_or("rgba16f").to_owned();
    if zwsdk::video::format_name(&format).is_none() {
        return Err(what(&format!("FORMAT の {format} には対応していません。")));
    }
    let linear = !block
        .header("FILTER")
        .is_some_and(|f| f.eq_ignore_ascii_case("NEAREST"));
    let mut data: String = block.body.split_whitespace().collect();
    // mpv（OpenGL）は `16f` の形式でも中身を float32 で受ける。その長さなら半精度に直す。
    let texels: usize = size.iter().map(|s| *s as usize).product();
    let half = zwsdk::video::texel_bytes(&format).unwrap_or(0);
    if zwsdk::video::format_name(&format).is_some_and(|f| f.ends_with("16float"))
        && data.len() == texels * half * 2 * 2
    {
        data = halves(&data).ok_or_else(|| what("データが 16 進数ではありません。"))?;
    }
    Ok((
        name,
        Texture {
            size,
            format,
            linear,
            keep: false,
            previous: false,
            data: Some(data),
        },
    ))
}

/// 重みのテーブルのサブピクセル位置のグリッド（RAVU-Zoom の `LUTPOS(subpix, vec2(9.0))` の 9）。
fn lut_grid(bodies: &[&str]) -> Option<u32> {
    for body in bodies {
        let mut rest = *body;
        while let Some(at) = rest.find("LUTPOS(") {
            rest = &rest[at + 7..];
            let call = &rest[..rest.find(';').unwrap_or(rest.len())];
            if let Some(v) = call.find("vec2(") {
                let inner = &call[v + 5..];
                let number = inner[..inner.find(')').unwrap_or(0)]
                    .trim()
                    .trim_end_matches(".0");
                if let Ok(n) = number.parse::<u32>()
                    && n > 1
                {
                    return Some(n);
                }
            }
        }
    }
    None
}

/// 重みのテーブルをセルごとに縦へ続けた並びに直す（テクスチャのキャッシュに乗るように）。
/// 中身付き・線形補間・二次元で、幅と高さが n の倍数、`texture(名前, …)` でしか使わないテーブルだけ。
fn regroup(texture: &mut Texture, name: &str, n: u32, bodies: &[&str]) -> Option<[u32; 3]> {
    let [width, height] = texture.size[..] else {
        return None;
    };
    if !texture.linear || width % n != 0 || height % n != 0 || width == n {
        return None;
    }
    let uses: usize = bodies.iter().map(|b| words(b, name)).sum();
    let calls: usize = bodies.iter().map(|b| calls(b, name)).sum();
    if uses == 0 || uses != calls {
        return None;
    }
    let texel = zwsdk::video::texel_bytes(&texture.format)?;
    let bytes = zwsdk::video::from_hex(texture.data.as_deref()?)?;
    if bytes.len() != (width * height) as usize * texel {
        return None;
    }
    let (n, groups, rows) = (n as usize, (width / n) as usize, (height / n) as usize);
    let width = width as usize;
    let mut out = vec![0u8; bytes.len()];
    for row in 0..rows {
        for group in 0..groups {
            for y in 0..n {
                let from = ((row * n + y) * width + group * n) * texel;
                let to = ((row * groups * n + group * n + y) * n) * texel;
                out[to..to + n * texel].copy_from_slice(&bytes[from..from + n * texel]);
            }
        }
    }
    let mut hex = String::with_capacity(out.len() * 2);
    for b in &out {
        hex.push_str(&format!("{b:02x}"));
    }
    texture.data = Some(hex);
    texture.size = vec![n as u32, height * groups as u32];
    Some([n as u32, groups as u32, height])
}

fn words(text: &str, name: &str) -> usize {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut count = 0;
    let mut from = 0;
    while let Some(found) = text[from..].find(name) {
        let at = from + found;
        from = at + name.len();
        let before = text[..at].chars().last();
        let after = text[from..].chars().next();
        if !before.is_some_and(ident) && !after.is_some_and(ident) {
            count += 1;
        }
    }
    count
}

fn calls(text: &str, name: &str) -> usize {
    let mut count = 0;
    let mut rest = text;
    while let Some((_, end)) = glsl::find_texture_call(rest, name) {
        count += 1;
        rest = &rest[end..];
    }
    count
}

fn halves(hex: &str) -> Option<String> {
    let bytes = zwsdk::video::from_hex(hex)?;
    let mut out = String::with_capacity(bytes.len());
    for word in bytes.chunks_exact(4) {
        let half = f16_bits(f32::from_le_bytes([word[0], word[1], word[2], word[3]]));
        for b in half.to_le_bytes() {
            out.push_str(&format!("{b:02x}"));
        }
    }
    Some(out)
}

fn f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x7f_ffff;
    if exponent == 0xff {
        return sign | 0x7c00 | if mantissa != 0 { 0x200 } else { 0 };
    }
    let e = exponent - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = mantissa | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = m >> shift;
        let rest = m & ((1 << shift) - 1);
        let middle = 1 << (shift - 1);
        let round = u32::from(rest > middle || (rest == middle && half & 1 == 1));
        return sign | (half + round) as u16;
    }
    let half = ((e as u32) << 10) | (mantissa >> 13);
    let rest = mantissa & 0x1fff;
    let round = u32::from(rest > 0x1000 || (rest == 0x1000 && half & 1 == 1));
    sign | (half + round) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_become_halves() {
        assert_eq!(f16_bits(1.0), 0x3c00);
        assert_eq!(f16_bits(-2.0), 0xc000);
        assert_eq!(f16_bits(0.5), 0x3800);
        assert_eq!(f16_bits(0.0), 0);
        assert_eq!(f16_bits(65504.0), 0x7bff);
        assert_eq!(f16_bits(1e6), 0x7c00, "大きすぎれば無限");
        assert_eq!(
            f16_bits(f32::from_bits(0x3afba105)),
            0x17dd,
            "ravu の重み（約 0.00192）"
        );
        assert_eq!(f16_bits(6.0e-8), 0x0001, "最小の非正規化数");
        assert_eq!(halves("0000803f"), Some(String::from("003c")));
    }
}

type Named = (String, [u32; 2]);

type Luma = Option<(Named, Named)>;

const LUMA: &str = include_str!("wgsl/luma.wgsl");
const LUMA_MERGE: &str = include_str!("wgsl/luma_merge.wgsl");

pub fn unit(builder: &mut Builder<'_>, files: &[(String, String)]) -> Result<(), String> {
    let parsed: Vec<(&str, Vec<Block>)> = files
        .iter()
        .map(|(name, text)| (name.as_str(), blocks(text)))
        .collect();

    // 添えたテクスチャは、名前がファイルをまたいで同じなら同じ物（mpv と同じ）。
    let bodies: Vec<&str> = parsed
        .iter()
        .flat_map(|(_, blocks)| {
            blocks
                .iter()
                .filter(|b| b.has("HOOK"))
                .map(|b| b.body.as_str())
        })
        .collect();
    let grid = lut_grid(&bodies);
    let mut user: BTreeMap<String, (String, Vec<u32>)> = BTreeMap::new();
    let mut cells: BTreeMap<String, [u32; 3]> = BTreeMap::new();
    for (file, blocks) in &parsed {
        for block in blocks.iter().filter(|b| b.has("TEXTURE")) {
            let (name, mut texture) = user_texture(file, block)?;
            let graph_name = format!("u{}_{}", builder.unit, zwsdk::video::ident(&name));
            user.insert(name.clone(), (graph_name.clone(), texture.size.clone()));
            if let Some(cell) = grid.and_then(|n| regroup(&mut texture, &name, n, &bodies)) {
                cells.insert(name, cell);
            }
            builder.graph.textures.insert(graph_name, texture);
        }
    }

    let mut passes: Vec<MpvPass<'_>> = Vec::new();
    let mut order = 0;
    for (file, blocks) in &parsed {
        for block in blocks.iter().filter(|b| b.has("HOOK")) {
            order += 1;
            match stage_of(block) {
                Some(stage) => passes.push(MpvPass {
                    file,
                    order,
                    block,
                    stage,
                }),
                None => zwsdk::plug::warn(&format!(
                    "{file}: 「{}」は {} に掛かるので飛ばす（受けるのは画像と LUMA）",
                    block.header("DESC").unwrap_or("?"),
                    block.all("HOOK").join(" ")
                )),
            }
        }
    }
    passes.sort_by_key(|pass| (rank(pass.stage).unwrap_or(99), pass.order));

    let target = builder.env.target;
    let input_size = builder.env.input;
    let mut luma: Luma = None;
    let mut saved: BTreeMap<String, (String, [u32; 2])> = BTreeMap::new();
    let mut scaled = false;

    for pass in passes {
        let block = pass.block;
        let desc = block.header("DESC").unwrap_or("pass").to_owned();
        let what = |e: String| format!("{}（{desc}）：{e}", pass.file);
        let expr = |e: String| what(format!("式に誤りがあります。{e}"));
        let stage = pass.stage;

        if stage == "LUMA" {
            if luma.is_none() {
                let (main, size) = builder.main.clone();
                let to = builder.texture_as("luma", size, "r16float");
                builder.simple("luma", &[&main], LUMA.trim_end(), &to);
                luma = Some(((to, size), (main, size)));
            }
        } else {
            merge(builder, &mut luma);
            if after_scaling(stage) && !scaled {
                builder.scale(target);
                scaled = true;
            }
        }
        let hooked = match (&luma, stage) {
            (Some((current, _)), "LUMA") => current.clone(),
            _ => builder.main.clone(),
        };

        let main_size = builder.main.1;
        let mut sizes: BTreeMap<String, [u32; 2]> = BTreeMap::new();
        for name in [
            "MAIN",
            "RGB",
            "MAINPRESUB",
            "LINEAR",
            "SIGMOID",
            "PREKERNEL",
        ] {
            sizes.insert(name.into(), main_size);
        }
        // NATIVE は元の画像（mpv の定義どおり）。Anime4K の AutoDownscalePre はこれで働くかを決める。
        sizes.insert("NATIVE".into(), input_size);
        for name in ["POSTKERNEL", "SCALED"] {
            sizes.insert(name.into(), if scaled { main_size } else { target });
        }
        sizes.insert("OUTPUT".into(), target);
        sizes.insert("NATIVE_CROPPED".into(), input_size);
        sizes.insert(
            "LUMA".into(),
            luma.as_ref().map_or(main_size, |(current, _)| current.1),
        );
        sizes.insert(
            "CHROMA".into(),
            [main_size[0].div_ceil(2), main_size[1].div_ceil(2)],
        );
        sizes.insert("HOOKED".into(), hooked.1);
        sizes.insert(stage.into(), hooked.1);
        for (name, (_, size)) in &saved {
            sizes.insert(name.clone(), *size);
        }
        for (name, (_, size)) in &user {
            sizes.insert(name.clone(), [size[0], size.get(1).copied().unwrap_or(1)]);
        }

        if let Some(when) = block.header("WHEN")
            && rpn(when, &sizes).map_err(expr)? == 0.0
        {
            continue;
        }
        let out = [
            match block.header("WIDTH") {
                Some(text) => rpn_size(text, &sizes).map_err(expr)?,
                None => hooked.1[0],
            },
            match block.header("HEIGHT") {
                Some(text) => rpn_size(text, &sizes).map_err(expr)?,
                None => hooked.1[1],
            },
        ];

        let mut reads = Vec::new();
        let mut binds = Vec::new();
        for name in block.all("BIND") {
            let (graph_name, size, is_user) = if name == "HOOKED" || name == stage {
                (hooked.0.clone(), vec![hooked.1[0], hooked.1[1]], false)
            } else if let Some((graph_name, size)) = user.get(name) {
                (graph_name.clone(), size.clone(), true)
            } else if let Some((graph_name, size)) = saved.get(name) {
                (graph_name.clone(), vec![size[0], size[1]], false)
            } else if name == "NATIVE_CROPPED" || name == "NATIVE" {
                (
                    String::from("picture"),
                    vec![input_size[0], input_size[1]],
                    false,
                )
            } else if rank(name).is_some_and(|r| (1..=7).contains(&r)) {
                (
                    builder.main.0.clone(),
                    vec![main_size[0], main_size[1]],
                    false,
                )
            } else if name == "LUMA"
                && let Some((current, _)) = &luma
            {
                (current.0.clone(), vec![current.1[0], current.1[1]], false)
            } else {
                return Err(what(format!("{name} は BIND に指定できません。")));
            };
            reads.push(graph_name);
            binds.push(Bound {
                name: name.to_owned(),
                size,
                user: is_user,
                cell: if is_user {
                    cells.get(name).copied()
                } else {
                    None
                },
            });
        }
        let aliases: Vec<(String, usize)> = binds
            .iter()
            .position(|b| b.name == "HOOKED")
            .filter(|_| !binds.iter().any(|b| b.name == stage))
            .map(|i| vec![(stage.to_owned(), i)])
            .unwrap_or_default();

        let compute = match block.header("COMPUTE") {
            None => None,
            Some(args) => {
                let numbers: Vec<u32> = args
                    .split_whitespace()
                    .filter_map(|n| n.parse().ok())
                    .collect();
                Some(match numbers.as_slice() {
                    [bw, bh, tw, th, ..] => [*bw, *bh, *tw, *th],
                    [bw, bh] => [*bw, *bh, *bw, *bh],
                    _ => return Err(what(String::from("COMPUTE の値が足りません。"))),
                })
            }
        };
        let wgsl = glsl::translate(&glsl::Pass {
            body: &block.body,
            binds: &binds,
            aliases: &aliases,
            compute,
            target: out,
            input_size,
            target_size: target,
        })
        .map_err(|e| what(format!("シェーダーを変換できません。{e}")))?;

        let save = block
            .header("SAVE")
            .filter(|s| *s != stage && *s != "HOOKED");
        // 形式は残す成分の数（COMPONENTS）。compute は書き先が storage なので四成分のまま
        // （r16float は WebGPU の storage に使えない）。
        let components = block
            .header("COMPONENTS")
            .and_then(|n| n.trim().parse::<u32>().ok())
            .unwrap_or(if stage == "LUMA" { 1 } else { 4 });
        let format = match (compute.is_some(), components) {
            (true, _) => "rgba16float",
            (false, 1) => "r16float",
            (false, 2) => "rg16float",
            _ => "rgba16float",
        };
        let writes = builder.texture_as(save.unwrap_or(stage), out, format);
        match save {
            Some(name) => {
                saved.insert(name.to_owned(), (writes.clone(), out));
            }
            None if stage == "LUMA" => {
                if let Some((current, _)) = luma.as_mut() {
                    *current = (writes.clone(), out);
                }
            }
            None => builder.main = (writes.clone(), out),
        }
        builder.push(Pass {
            name: desc,
            kind: if compute.is_some() {
                Kind::Compute
            } else {
                Kind::Fragment
            },
            source: wgsl,
            entry: String::from("main"),
            reads,
            writes,
            block: compute.map_or([0, 0], |c| [c[0], c[1]]),
            ..Pass::default()
        });
    }
    merge(builder, &mut luma);
    Ok(())
}

fn merge(builder: &mut Builder<'_>, luma: &mut Luma) {
    let Some(((current, size), (base, _))) = luma.take() else {
        return;
    };
    let to = builder.texture("main", size);
    builder.simple("merge luma", &[&current, &base], LUMA_MERGE.trim_end(), &to);
    builder.main = (to, size);
}
