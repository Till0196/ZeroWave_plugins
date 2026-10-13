// 前と今のフレームを RIFE の入力（CHW の f32、縮めて端の値でパディングする）にする。時刻のチャンネルも埋める。
// 縮めるときは元の範囲を二画素ずつ双線形でサンプリングして平均する（1/2 なら 2×2、1/4 なら 4×4 の平均）。

struct Pack {
    pw: u32,
    ph: u32,
    rw: u32,
    rh: u32,
    taps: u32,
    t: f32,
    _p0: u32,
    _p1: u32,
}

@group(2) @binding(0) var<storage, read_write> in0: array<f32>;
@group(2) @binding(1) var<storage, read_write> in1: array<f32>;
@group(2) @binding(2) var<storage, read_write> in2: array<f32>;
@group(2) @binding(3) var<uniform> k: Pack;

@compute @workgroup_size(8, 8, 1)
fn pack(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= k.pw || id.y >= k.ph) {
        return;
    }
    let p = vec2<f32>(f32(min(id.x, k.rw - 1u)), f32(min(id.y, k.rh - 1u)));
    let size = vec2<f32>(textureDimensions(prev));
    let step = size / vec2<f32>(f32(k.rw), f32(k.rh));
    let n = max(k.taps, 1u);
    var a = vec3<f32>(0.0);
    var b = vec3<f32>(0.0);
    for (var j = 0u; j < n; j++) {
        for (var i = 0u; i < n; i++) {
            let at = (p + (vec2<f32>(f32(i), f32(j)) + 0.5) / f32(n)) * step;
            a += textureSampleLevel(prev, prev_smp, at / size, 0.0).rgb;
            b += textureSampleLevel(cur, cur_smp, at / size, 0.0).rgb;
        }
    }
    let count = f32(n * n);
    a /= count;
    b /= count;
    let plane = k.pw * k.ph;
    let i = id.y * k.pw + id.x;
    in0[i] = a.r;
    in0[plane + i] = a.g;
    in0[2u * plane + i] = a.b;
    in1[i] = b.r;
    in1[plane + i] = b.g;
    in1[2u * plane + i] = b.b;
    in2[i] = k.t;
}
