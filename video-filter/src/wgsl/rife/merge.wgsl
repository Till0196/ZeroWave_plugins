// 真ん中（t = 0.5）で求めたフロー二本とマスクを等倍へ広げ、位相 t のフレームを作る。フローは t に比例して
// 伸ばし、マスクは近い側へ寄せる（t = 0.5 なら RIFE の出力と同じ）。
//   出力 = (warp(前, 2t·F0)·m(1−t) + warp(今, 2(1−t)·F1)·(1−m)t) ÷ (m(1−t) + (1−m)t)

struct Merge {
    sw: u32,
    sh: u32,
    rx: f32,
    ry: f32,
    f0: u32,
    f1: u32,
    mask: u32,
    _p0: u32,
}

@group(2) @binding(0) var<storage, read> flow0: array<f32>;
@group(2) @binding(1) var<storage, read> flow1: array<f32>;
@group(2) @binding(2) var<storage, read> mask: array<f32>;
@group(2) @binding(3) var<uniform> m: Merge;

fn small(which: u32, c: u32, yy: i32, xx: i32) -> f32 {
    let cy = u32(clamp(yy, 0, i32(m.sh) - 1));
    let cx = u32(clamp(xx, 0, i32(m.sw) - 1));
    let i = c * m.sh * m.sw + cy * m.sw + cx;
    if (which == 0u) {
        return flow0[m.f0 + i];
    } else if (which == 1u) {
        return flow1[m.f1 + i];
    }
    return mask[m.mask + i];
}

fn small_bilinear(which: u32, c: u32, fy: f32, fx: f32) -> f32 {
    let y0 = floor(fy);
    let x0 = floor(fx);
    let iy = i32(y0);
    let ix = i32(x0);
    let a = mix(small(which, c, iy, ix), small(which, c, iy, ix + 1), fx - x0);
    let b = mix(small(which, c, iy + 1, ix), small(which, c, iy + 1, ix + 1), fx - x0);
    return mix(a, b, fy - y0);
}

@compute @workgroup_size(8, 8, 1)
fn merge(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(zw_out);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let t = clamp(zw.phase, 0.0, 1.0);
    let sx = max((f32(id.x) + 0.5) / m.rx - 0.5, 0.0);
    let sy = max((f32(id.y) + 0.5) / m.ry - 0.5, 0.0);
    let f0 = vec2<f32>(small_bilinear(0u, 0u, sy, sx) * m.rx, small_bilinear(0u, 1u, sy, sx) * m.ry);
    let f1 = vec2<f32>(small_bilinear(1u, 0u, sy, sx) * m.rx, small_bilinear(1u, 1u, sy, sx) * m.ry);
    let k = clamp(small_bilinear(2u, 0u, sy, sx), 0.0, 1.0);
    let dims = vec2<f32>(size);
    let here = vec2<f32>(id.xy) + 0.5;
    let v0 = textureSampleLevel(prev, prev_smp, (here + f0 * (2.0 * t)) / dims, 0.0).rgb;
    let v1 = textureSampleLevel(cur, cur_smp, (here + f1 * (2.0 * (1.0 - t))) / dims, 0.0).rgb;
    let w0 = k * (1.0 - t);
    let w1 = (1.0 - k) * t;
    let rgb = (v0 * w0 + v1 * w1) / max(w0 + w1, 1e-6);
    textureStore(zw_out, vec2<i32>(id.xy), vec4<f32>(rgb, 1.0));
}
