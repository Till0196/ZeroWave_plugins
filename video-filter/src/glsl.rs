// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! mpv のユーザーシェーダーの一つのパス（GLSL）を、本体のバインディングに合わせた WGSL に。
//! mpv が足す名前（`HOOKED_tex` など）を前置きで作り、naga で読んで WGSL に書く。
//! naga が扱えない物（結合サンプラー、`barrier()`、暗黙の LOD など）はここで補う。大きさは定数で埋め込む。

use std::fmt::Write as _;

use naga::ShaderStage;
use naga::valid::{Capabilities, ValidationFlags, Validator};

#[derive(Debug, Clone)]
pub struct Bound {
    pub name: String,
    pub size: Vec<u32>,
    pub user: bool,
    pub cell: Option<[u32; 3]>,
}

#[derive(Debug)]
pub struct Pass<'a> {
    pub body: &'a str,
    pub binds: &'a [Bound],
    pub aliases: &'a [(String, usize)],
    pub compute: Option<[u32; 4]>,
    pub target: [u32; 2],
    pub input_size: [u32; 2],
    pub target_size: [u32; 2],
}

fn vec2(size: [u32; 2]) -> String {
    format!("vec2({:.1}, {:.1})", f64::from(size[0]), f64::from(size[1]))
}

fn polyfills() -> String {
    let mut s = String::new();
    for c in 2..=4 {
        for r in 2..=4 {
            let m = format!("mat{c}x{r}");
            let cols: Vec<String> = (0..c).map(|k| format!("a[{k}] * b[{k}]")).collect();
            writeln!(
                s,
                "{m} zw_mcm({m} a, {m} b) {{ return {m}({}); }}",
                cols.join(", ")
            )
            .unwrap();
            let cols: Vec<String> = ["x", "y", "z", "w"][..c]
                .iter()
                .map(|k| format!("c * r.{k}"))
                .collect();
            writeln!(
                s,
                "{m} zw_outer(vec{r} c, vec{c} r) {{ return {m}({}); }}",
                cols.join(", ")
            )
            .unwrap();
        }
    }
    // textureGather の並びは GLSL の決まり：(0,1) (1,1) (1,0) (0,0)。縁は CLAMP_TO_EDGE。
    s.push_str(concat!(
        "#define zw_base(s, p) ivec2(floor((p) * vec2(textureSize(s, 0)) - 0.5))\n",
        "#define zw_at(s, p, dx, dy) texelFetch(s, clamp(zw_base(s, p) + ivec2(dx, dy), ivec2(0), textureSize(s, 0) - 1), 0)\n",
        "#define zw_gather(s, p, c) vec4(zw_at(s, p, 0, 1)[c], zw_at(s, p, 1, 1)[c], zw_at(s, p, 1, 0)[c], zw_at(s, p, 0, 0)[c])\n",
        "#define zw_tgo(s, p, o, c) zw_gather(s, (p) + vec2(o) / vec2(textureSize(s, 0)), c)\n",
        "#define zw_ofs(s, p, o) texelFetch(s, clamp(zw_base(s, p) + (o), ivec2(0), textureSize(s, 0) - 1), 0).x\n",
        "#define zw_tgos(s, p, o) vec4(zw_ofs(s, p, o[0]), zw_ofs(s, p, o[1]), zw_ofs(s, p, o[2]), zw_ofs(s, p, o[3]))\n",
    ));
    s
}

/// シェーダーが `defined(NAME_gather)` で gather の有無を問い合わせているか。ここの gather は
/// 速くなく naga が読めないコードパスも通るので、問い合わせには無いと返す。
fn asks_for_gather(body: &str) -> bool {
    body.match_indices("defined(").any(|(at, _)| {
        let inner = &body[at + "defined(".len()..];
        inner
            .find(')')
            .is_some_and(|end| inner[..end].trim().ends_with("_gather"))
    })
}

fn names(
    s: &mut String,
    name: &str,
    i: usize,
    size: &[u32],
    pos: &str,
    map_pt: &str,
    gather: bool,
) {
    let size2 = [size[0], size.get(1).copied().unwrap_or(1)];
    let raw = format!("sampler2D(zw_t_{i}, zw_s_{i})");
    let pt = format!(
        "vec2({:.10}, {:.10})",
        1.0 / f64::from(size2[0].max(1)),
        1.0 / f64::from(size2[1].max(1))
    );
    for (suffix, value) in [
        ("raw", raw),
        ("pos", pos.to_owned()),
        ("size", vec2(size2)),
        ("rot", String::from("mat2(1.0, 0.0, 0.0, 1.0)")),
        ("off", String::from("vec2(0.0)")),
        ("pt", pt),
        ("mul", String::from("1.0")),
    ] {
        writeln!(s, "#define {name}_{suffix} {value}").unwrap();
    }
    writeln!(
        s,
        "#define {name}_map(id) ((vec2(id) + vec2(0.5)) * {map_pt})"
    )
    .unwrap();
    writeln!(
        s,
        "#define {name}_tex(pos) ({name}_mul * vec4(texture({name}_raw, pos)))"
    )
    .unwrap();
    writeln!(
        s,
        "#define {name}_texOff(off) {name}_tex({name}_pos + {name}_pt * vec2(off))"
    )
    .unwrap();
    if !gather {
        return;
    }
    writeln!(
        s,
        "#define {name}_gather(pos, c) ({name}_mul * vec4(zw_gather({name}_raw, pos, c)))"
    )
    .unwrap();
    writeln!(
        s,
        "#define {name}_gatherOff(off, c) {name}_gather({name}_pos + vec2(off) * {name}_pt, c)"
    )
    .unwrap();
}

pub fn source(pass: &Pass<'_>) -> (String, ShaderStage) {
    let stage = if pass.compute.is_some() {
        ShaderStage::Compute
    } else {
        ShaderStage::Fragment
    };
    let mut s = String::from("#version 450\n");
    s.push_str(concat!(
        "layout(set = 0, binding = 0) uniform ZwBlock {\n",
        "    vec2 zw_input_size; vec2 zw_output_size; uint zw_frame; float zw_phase;\n",
        "    uint zw_field; uint zw_first_row; uint zw_mate_row; uint zw_matched;\n",
        "    float zw_random; uint zw_outputs; uint zw_second;\n",
        "    uint zw_reserved0; uint zw_reserved1; uint zw_reserved2;\n",
        "};\n",
        "#define frame int(zw_frame)\n",
        "#define random zw_random\n",
        "#define tex_offset vec2(0.0)\n",
    ));
    writeln!(s, "#define input_size {}", vec2(pass.input_size)).unwrap();
    writeln!(s, "#define target_size {}", vec2(pass.target_size)).unwrap();
    s.push_str(&polyfills());
    let map_pt = format!(
        "vec2({:.10}, {:.10})",
        1.0 / f64::from(pass.target[0].max(1)),
        1.0 / f64::from(pass.target[1].max(1))
    );
    let pos = match stage {
        ShaderStage::Compute => {
            format!("((vec2(gl_GlobalInvocationID.xy) + vec2(0.5)) * {map_pt})")
        }
        _ => String::from("zw_uv"),
    };
    if stage == ShaderStage::Fragment {
        s.push_str("layout(location = 0) in vec2 zw_uv;\n");
    }
    let gather = !asks_for_gather(pass.body);
    for (i, bound) in pass.binds.iter().enumerate() {
        let dims = bound.size.len().clamp(1, 3);
        writeln!(
            s,
            "layout(set = 0, binding = {}) uniform texture{dims}D zw_t_{i};\nlayout(set = 0, binding = {}) uniform sampler zw_s_{i};",
            1 + 2 * i,
            2 + 2 * i
        )
        .unwrap();
        if bound.user {
            writeln!(
                s,
                "#define {} sampler{dims}D(zw_t_{i}, zw_s_{i})",
                bound.name
            )
            .unwrap();
            if let Some([n, groups, height]) = bound.cell {
                // セルを縦へ続けた幅 n のテーブルの座標へ（線形補間はセルの中で閉じる）。
                writeln!(
                    s,
                    "vec4 zw_cell_{i}(vec2 c) {{\n    float u = c.x * {w}.0;\n    float k = floor(u / {n}.0);\n    float y = c.y * {height}.0;\n    float r = floor(y / {n}.0);\n    return texture(sampler2D(zw_t_{i}, zw_s_{i}), vec2((u - {n}.0 * k) / {n}.0, (r * {w}.0 + {n}.0 * k + (y - {n}.0 * r)) / {tall}.0));\n}}",
                    w = n * groups,
                    tall = height * groups,
                )
                .unwrap();
            }
            let size2 = [bound.size[0], bound.size.get(1).copied().unwrap_or(1)];
            writeln!(s, "#define {}_size {}", bound.name, vec2(size2)).unwrap();
            writeln!(
                s,
                "#define {}_pt vec2({:.10}, {:.10})",
                bound.name,
                1.0 / f64::from(size2[0].max(1)),
                1.0 / f64::from(size2[1].max(1))
            )
            .unwrap();
        } else {
            names(&mut s, &bound.name, i, &bound.size, &pos, &map_pt, gather);
        }
    }
    for (alias, i) in pass.aliases {
        if let Some(bound) = pass.binds.get(*i) {
            names(&mut s, alias, *i, &bound.size, &pos, &map_pt, gather);
        }
    }
    match pass.compute {
        None => s.push_str("layout(location = 0) out vec4 zw_out;\n"),
        Some([_, _, x, y]) => {
            writeln!(s, "layout(local_size_x = {x}, local_size_y = {y}) in;").unwrap();
            writeln!(s, "const uvec3 zw_workgroup_size = uvec3({x}u, {y}u, 1u);").unwrap();
            writeln!(
                s,
                "layout(set = 0, binding = {}, rgba16f) uniform writeonly image2D out_image;",
                1 + 2 * pass.binds.len()
            )
            .unwrap();
        }
    }
    let mut body = pass.body.to_owned();
    for (i, bound) in pass.binds.iter().enumerate() {
        if bound.user && bound.cell.is_some() {
            body = replace_texture_call(&body, &bound.name, &format!("zw_cell_{i}("));
        }
    }
    for (from, to) in [
        ("gl_WorkGroupSize", "zw_workgroup_size"),
        ("matrixCompMult", "zw_mcm"),
        ("textureGatherOffsets", "zw_tgos"),
        ("textureGatherOffset", "zw_tgo"),
        ("outerProduct", "zw_outer"),
        ("textureGather", "zw_gather"),
    ] {
        body = replace_word(&body, from, to);
    }
    s.push_str("#line 1\n");
    s.push_str(&body);
    match stage {
        ShaderStage::Compute => s.push_str("\nvoid main() { hook(); }\n"),
        _ => s.push_str("\nvoid main() { zw_out = hook(); }\n"),
    }
    (s, stage)
}

pub fn replace_texture_call(text: &str, name: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = find_texture_call(rest, name) {
        out.push_str(&rest[..at.0]);
        out.push_str(to);
        rest = &rest[at.1..];
    }
    out.push_str(rest);
    out
}

pub fn find_texture_call(text: &str, name: &str) -> Option<(usize, usize)> {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut from = 0;
    while let Some(found) = text[from..].find("texture") {
        let start = from + found;
        from = start + 7;
        if text[..start].chars().last().is_some_and(ident) {
            continue;
        }
        let tail = text[start + 7..].trim_start();
        let Some(tail) = tail.strip_prefix('(') else {
            continue;
        };
        let tail = tail.trim_start();
        let Some(tail) = tail.strip_prefix(name) else {
            continue;
        };
        if tail.chars().next().is_some_and(ident) {
            continue;
        }
        let tail = tail.trim_start();
        let Some(tail) = tail.strip_prefix(',') else {
            continue;
        };
        return Some((start, text.len() - tail.len()));
    }
    None
}

fn replace_word(text: &str, from: &str, to: &str) -> String {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(from) {
        let before = rest[..at].chars().last().or_else(|| out.chars().last());
        let after = rest[at + from.len()..].chars().next();
        out.push_str(&rest[..at]);
        if before.is_some_and(ident) || after.is_some_and(ident) {
            out.push_str(from);
        } else {
            out.push_str(to);
        }
        rest = &rest[at + from.len()..];
    }
    out.push_str(rest);
    out
}

/// `barrier()` から naga 30 が入れるサブグループの同期を抜く（capability を要求しないように）。
fn unsubgroup(block: &mut naga::Block) {
    use naga::{Barrier, Statement};
    for statement in block.iter_mut() {
        match statement {
            Statement::ControlBarrier(b) | Statement::MemoryBarrier(b) => {
                b.remove(Barrier::SUB_GROUP);
            }
            Statement::Block(inner) => unsubgroup(inner),
            Statement::If { accept, reject, .. } => {
                unsubgroup(accept);
                unsubgroup(reject);
            }
            Statement::Switch { cases, .. } => {
                for case in cases.iter_mut() {
                    unsubgroup(&mut case.body);
                }
            }
            Statement::Loop {
                body, continuing, ..
            } => {
                unsubgroup(body);
                unsubgroup(continuing);
            }
            _ => {}
        }
    }
}

fn zero_lod(exprs: &mut naga::Arena<naga::Expression>) {
    for (_, expr) in exprs.iter_mut() {
        if let naga::Expression::ImageSample { level, .. } = expr
            && matches!(level, naga::SampleLevel::Auto)
        {
            *level = naga::SampleLevel::Zero;
        }
    }
}

fn short(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn translate(pass: &Pass<'_>) -> Result<String, String> {
    let (glsl, stage) = source(pass);
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(path) = std::env::var_os("ZW_DUMP_GLSL") {
        let _ = std::fs::write(path, &glsl);
    }
    let mut frontend = naga::front::glsl::Frontend::default();
    let mut module = frontend
        .parse(&naga::front::glsl::Options::from(stage), &glsl)
        .map_err(|e| short(&e.emit_to_string(&glsl)))?;
    // fragment でも LOD 0 に（中間の画像にミップマップは無く、微分の手間が省ける）。
    for (_, function) in module.functions.iter_mut() {
        unsubgroup(&mut function.body);
        zero_lod(&mut function.expressions);
    }
    for entry in module.entry_points.iter_mut() {
        unsubgroup(&mut entry.function.body);
        zero_lod(&mut entry.function.expressions);
    }
    let info = Validator::new(ValidationFlags::all(), Capabilities::default())
        .validate(&module)
        .map_err(|e| short(&e.emit_to_string(&glsl)))?;
    naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mpv_pass_becomes_wgsl_bound_to_the_contract() {
        let body = "
vec4 hook() {
    vec4 c = HOOKED_texOff(vec2(0.0));
    vec4 n = MAIN_tex(MAIN_pos + vec2(0.0, MAIN_pt.y));
    return mix(c, n, 0.5) + vec4(float(frame) * 0.0 + random * 0.0 + input_size.x * 0.0);
}
";
        let binds = vec![Bound {
            cell: None,
            name: "HOOKED".into(),
            size: vec![1440, 1080],
            user: false,
        }];
        let aliases = vec![("MAIN".to_string(), 0)];
        let pass = Pass {
            body,
            binds: &binds,
            aliases: &aliases,
            compute: None,
            target: [1440, 1080],
            input_size: [1440, 1080],
            target_size: [1920, 1080],
        };
        let wgsl = translate(&pass).expect("訳せる");
        assert!(wgsl.contains("@group(0) @binding(0)"));
        assert!(wgsl.contains("@group(0) @binding(1)"));
        assert!(wgsl.contains("@group(0) @binding(2)"));
        assert!(wgsl.contains("@fragment"));
        assert!(wgsl.contains("fn main("));
    }

    #[test]
    fn a_shader_that_asks_for_gather_is_told_there_is_none() {
        assert!(asks_for_gather("#if defined(LUMA_gather) && X\n"));
        assert!(asks_for_gather("#if (defined( HOOKED_gather ) || D1W)"));
        assert!(!asks_for_gather("vec4 g = HOOKED_gather(HOOKED_pos, 0);"));
        assert!(!asks_for_gather("#if defined(LUMA_raw)"));
    }

    #[test]
    fn only_whole_words_are_replaced() {
        assert_eq!(
            replace_word(
                "textureGatherOffsets(a) textureGatherOffset(b) mytextureGather(c)",
                "textureGatherOffset",
                "X"
            ),
            "textureGatherOffsets(a) X(b) mytextureGather(c)"
        );
        assert_eq!(
            replace_word("textureGather(a)", "textureGather", "zw_gather"),
            "zw_gather(a)"
        );
    }

    #[test]
    fn a_compute_pass_with_a_barrier_becomes_wgsl() {
        let body = "
shared float cache[64];
void hook() {
    uint i = gl_LocalInvocationIndex;
    cache[i] = HOOKED_tex(HOOKED_pos).x;
    barrier();
    float v = cache[(i + 1u) % (gl_WorkGroupSize.x * gl_WorkGroupSize.y)];
    imageStore(out_image, ivec2(gl_GlobalInvocationID.xy), vec4(v));
}
";
        let binds = vec![Bound {
            cell: None,
            name: "HOOKED".into(),
            size: vec![960, 540],
            user: false,
        }];
        let pass = Pass {
            body,
            binds: &binds,
            aliases: &[],
            compute: Some([8, 8, 8, 8]),
            target: [1920, 1080],
            input_size: [960, 540],
            target_size: [1920, 1080],
        };
        let wgsl = translate(&pass).expect("訳せる");
        assert!(wgsl.contains("@compute"));
        assert!(
            wgsl.contains("@group(0) @binding(3)"),
            "書く先は reads の次"
        );
        assert!(wgsl.contains("workgroupBarrier"));
    }
}
