// ncnn の層を WGSL で（RIFE を動かすのに要る物だけ）。値はいつも f32、
// 並びは CHW（チャンネル、行、列）で隙間なく置く。

struct P {
    c_in: u32,
    h_in: u32,
    w_in: u32,
    c_out: u32,
    h_out: u32,
    w_out: u32,
    stride: u32,
    pad: u32,
    kernel: u32,
    act: u32,      // 0：無し、2：leaky
    op: u32,       // BinaryOp の種類・Crop の始まりなど
    c_b: u32,      // 二つ目の入力のチャンネル数（1 なら全チャンネルへ）
    slope: f32,
    scalar: f32,
    scale_h: f32,
    scale_w: f32,
    coef_a: f32,
    coef_b: f32,
    bias_at: u32,  // 重みの中のバイアスの位置
    has_bias: u32,
    // バッファの中の始まり（要素）。切り出しはコピーせずにここをずらし、結合は作る側が
    // 結合先のこの位置へ直接書く。
    x_off: u32,
    y_off: u32,
    out_off: u32,
    _pad: u32,
}

@group(2) @binding(0) var<storage, read> x: array<f32>;
@group(2) @binding(1) var<storage, read> y: array<f32>;
@group(2) @binding(2) var<storage, read_write> out: array<f32>;
@group(2) @binding(3) var<storage, read> wt: array<f32>;
@group(2) @binding(4) var<uniform> p: P;
// 並べ替えた重み：[出力の 16 の組][入力][カーネルの位置] ごとに vec4 四つ（16 チャンネル）。
@group(2) @binding(5) var<storage, read> wt4: array<vec4<f32>>;

fn activate(v: f32) -> f32 {
    if (p.act == 2u && v < 0.0) {
        return v * p.slope;
    }
    if (p.act == 1u) {
        return max(v, 0.0);
    }
    return v;
}

// 畳み込み（正方形のカーネル、同じ幅の縁）。一つのスレッドが一画素の 8 チャンネルを出す。
// id.z が出力チャンネルの 8 の組。
@compute @workgroup_size(8, 8, 1)
fn conv(@builtin(global_invocation_id) id: vec3<u32>) {
    let ox = id.x;
    let oy = id.y;
    let oc0 = id.z * 8u;
    if (ox >= p.w_out || oy >= p.h_out || oc0 >= p.c_out) {
        return;
    }
    let k = p.kernel;
    let kk = k * k;
    var acc: array<f32, 8>;
    for (var j = 0u; j < 8u; j++) {
        acc[j] = 0.0;
    }
    let plane = p.h_in * p.w_in;
    let per_out = p.c_in * kk;
    for (var ic = 0u; ic < p.c_in; ic++) {
        for (var ky = 0u; ky < k; ky++) {
            let iy = i32(oy * p.stride + ky) - i32(p.pad);
            if (iy < 0 || iy >= i32(p.h_in)) {
                continue;
            }
            for (var kx = 0u; kx < k; kx++) {
                let ix = i32(ox * p.stride + kx) - i32(p.pad);
                if (ix < 0 || ix >= i32(p.w_in)) {
                    continue;
                }
                let v = x[p.x_off + ic * plane + u32(iy) * p.w_in + u32(ix)];
                let wi = ic * kk + ky * k + kx;
                for (var j = 0u; j < 8u; j++) {
                    if (oc0 + j < p.c_out) {
                        acc[j] += v * wt[(oc0 + j) * per_out + wi];
                    }
                }
            }
        }
    }
    let oplane = p.h_out * p.w_out;
    for (var j = 0u; j < 8u; j++) {
        let oc = oc0 + j;
        if (oc < p.c_out) {
            var v = acc[j];
            if (p.has_bias == 1u) {
                v += wt[p.bias_at + oc];
            }
            out[p.out_off + oc * oplane + oy * p.w_out + ox] = activate(v);
        }
    }
}

// 逆畳み込み（集める形）：出力 (oy, ox) に、iy·stride + ky − pad = oy となる入力を足す。
// ncnn の重みは [出力][入力][ky][kx]。
@compute @workgroup_size(8, 8, 1)
fn deconv(@builtin(global_invocation_id) id: vec3<u32>) {
    let ox = id.x;
    let oy = id.y;
    let oc0 = id.z * 8u;
    if (ox >= p.w_out || oy >= p.h_out || oc0 >= p.c_out) {
        return;
    }
    let k = p.kernel;
    let kk = k * k;
    var acc: array<f32, 8>;
    for (var j = 0u; j < 8u; j++) {
        acc[j] = 0.0;
    }
    let plane = p.h_in * p.w_in;
    let per_out = p.c_in * kk;
    for (var ky = 0u; ky < k; ky++) {
        let ty = i32(oy) + i32(p.pad) - i32(ky);
        if (ty < 0 || ty % i32(p.stride) != 0) {
            continue;
        }
        let iy = ty / i32(p.stride);
        if (iy >= i32(p.h_in)) {
            continue;
        }
        for (var kx = 0u; kx < k; kx++) {
            let tx = i32(ox) + i32(p.pad) - i32(kx);
            if (tx < 0 || tx % i32(p.stride) != 0) {
                continue;
            }
            let ix = tx / i32(p.stride);
            if (ix >= i32(p.w_in)) {
                continue;
            }
            for (var ic = 0u; ic < p.c_in; ic++) {
                let v = x[p.x_off + ic * plane + u32(iy) * p.w_in + u32(ix)];
                let wi = ic * kk + ky * k + kx;
                for (var j = 0u; j < 8u; j++) {
                    if (oc0 + j < p.c_out) {
                        acc[j] += v * wt[(oc0 + j) * per_out + wi];
                    }
                }
            }
        }
    }
    let oplane = p.h_out * p.w_out;
    for (var j = 0u; j < 8u; j++) {
        let oc = oc0 + j;
        if (oc < p.c_out) {
            var v = acc[j];
            if (p.has_bias == 1u) {
                v += wt[p.bias_at + oc];
            }
            out[p.out_off + oc * oplane + oy * p.w_out + ox] = activate(v);
        }
    }
}

fn at_in(c: u32, yy: i32, xx: i32) -> f32 {
    let cy = u32(clamp(yy, 0, i32(p.h_in) - 1));
    let cx = u32(clamp(xx, 0, i32(p.w_in) - 1));
    return x[p.x_off + c * p.h_in * p.w_in + cy * p.w_in + cx];
}

fn bilinear(c: u32, fy: f32, fx: f32) -> f32 {
    let y0 = floor(fy);
    let x0 = floor(fx);
    let wy = fy - y0;
    let wx = fx - x0;
    let iy = i32(y0);
    let ix = i32(x0);
    let a = mix(at_in(c, iy, ix), at_in(c, iy, ix + 1), wx);
    let b = mix(at_in(c, iy + 1, ix), at_in(c, iy + 1, ix + 1), wx);
    return mix(a, b, wy);
}

// 一画素ずつの層。id.x が CHW の通し番号。
fn index_of(id: vec3<u32>) -> u32 {
    return id.x + id.y * 65535u * 64u;
}

// 双線形の大きさ直し（align_corners = false）。一つのスレッドが一画素の全チャンネル
// （位置と重みは一度だけ求める）。
@compute @workgroup_size(64)
fn interp(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = index_of(id);
    let oplane = p.h_out * p.w_out;
    if (i >= oplane) {
        return;
    }
    let ox = i % p.w_out;
    let oy = i / p.w_out;
    let fy = max((f32(oy) + 0.5) / p.scale_h - 0.5, 0.0);
    let fx = max((f32(ox) + 0.5) / p.scale_w - 0.5, 0.0);
    let y0 = floor(fy);
    let x0 = floor(fx);
    let wy = fy - y0;
    let wx = fx - x0;
    let ya = u32(clamp(i32(y0), 0, i32(p.h_in) - 1)) * p.w_in;
    let yb = u32(clamp(i32(y0) + 1, 0, i32(p.h_in) - 1)) * p.w_in;
    let xa = u32(clamp(i32(x0), 0, i32(p.w_in) - 1));
    let xb = u32(clamp(i32(x0) + 1, 0, i32(p.w_in) - 1));
    let plane = p.h_in * p.w_in;
    for (var c = 0u; c < p.c_out; c++) {
        let base = p.x_off + c * plane;
        let top = mix(x[base + ya + xa], x[base + ya + xb], wx);
        let bottom = mix(x[base + yb + xa], x[base + yb + xb], wx);
        out[p.out_off + c * oplane + i] = mix(top, bottom, wy);
    }
}

// PixelShuffle（r = kernel）：出力 (c, y, x) ← 入力 (c·r² + (y%r)·r + x%r, y/r, x/r)。
@compute @workgroup_size(64)
fn shuffle(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = index_of(id);
    let total = p.c_out * p.h_out * p.w_out;
    if (i >= total) {
        return;
    }
    let r = p.kernel;
    let ox = i % p.w_out;
    let oy = (i / p.w_out) % p.h_out;
    let c = i / (p.w_out * p.h_out);
    let ic = c * r * r + (oy % r) * r + (ox % r);
    out[p.out_off + i] = x[p.x_off + ic * p.h_in * p.w_in + (oy / r) * p.w_in + (ox / r)];
}

// コピーする（結合先へ直接書けなかった入力を、結合先の位置へ）。
@compute @workgroup_size(64)
fn copy(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = index_of(id);
    if (i >= p.c_out * p.h_in * p.w_in) {
        return;
    }
    out[p.out_off + i] = x[p.x_off + i];
}

// 四則（ncnn の BinaryOp）。scalar が 1 なら二つ目は定数。
fn binary(a: f32, b: f32) -> f32 {
    switch (p.op) {
        case 0u: { return a + b; }
        case 1u: { return a - b; }
        case 2u: { return a * b; }
        case 3u: { return a / b; }
        case 4u: { return max(a, b); }
        case 5u: { return min(a, b); }
        case 7u: { return b - a; }
        case 8u: { return b / a; }
        default: { return a; }
    }
}

@compute @workgroup_size(64)
fn binary_scalar(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = index_of(id);
    if (i >= p.c_out * p.h_out * p.w_out) {
        return;
    }
    out[p.out_off + i] = binary(x[p.x_off + i], p.scalar);
}

@compute @workgroup_size(64)
fn binary_pair(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = index_of(id);
    let plane = p.h_out * p.w_out;
    if (i >= p.c_out * plane) {
        return;
    }
    var j = i;
    if (p.c_b == 1u) {
        j = i % plane;
    } else if (p.has_bias == 2u) {
        // 二つ目が C×1×1（チャンネルごとの係数、lite モデルの MemoryData）。
        j = i / plane;
    }
    out[p.out_off + i] = binary(x[p.x_off + i], y[p.y_off + j]);
}

// 重み付きの和（Eltwise SUM）。
@compute @workgroup_size(64)
fn weighted(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = index_of(id);
    if (i >= p.c_out * p.h_out * p.w_out) {
        return;
    }
    out[p.out_off + i] = p.coef_a * x[p.x_off + i] + p.coef_b * y[p.y_off + i];
}

// leaky ReLU（slope）と Sigmoid（op = 1）。
@compute @workgroup_size(64)
fn unary(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = index_of(id);
    if (i >= p.c_out * p.h_out * p.w_out) {
        return;
    }
    let v = x[p.x_off + i];
    if (p.op == 1u) {
        out[p.out_off + i] = 1.0 / (1.0 + exp(-v));
    } else {
        out[p.out_off + i] = select(v * p.slope, v, v >= 0.0);
    }
}

// rife.Warp：画像 x をフロー y（dx, dy、画素）でワープする。縁は端の値（grid_sample の border）。
// 一つのスレッドが一画素の全チャンネル（フローと位置は一度だけ読む）。
@compute @workgroup_size(64)
fn warp(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = index_of(id);
    let plane = p.h_in * p.w_in;
    if (i >= plane) {
        return;
    }
    let ox = i % p.w_in;
    let oy = i / p.w_in;
    let fx = clamp(f32(ox) + y[p.y_off + i], 0.0, f32(p.w_in - 1u));
    let fy = clamp(f32(oy) + y[p.y_off + plane + i], 0.0, f32(p.h_in - 1u));
    let x0 = floor(fx);
    let y0 = floor(fy);
    let wx = fx - x0;
    let wy = fy - y0;
    let xa = u32(x0);
    let ya = u32(y0) * p.w_in;
    let xb = min(xa + 1u, p.w_in - 1u);
    let yb = min(u32(y0) + 1u, p.h_in - 1u) * p.w_in;
    for (var c = 0u; c < p.c_out; c++) {
        let base = p.x_off + c * plane;
        let top = mix(x[base + ya + xa], x[base + ya + xb], wx);
        let bottom = mix(x[base + yb + xa], x[base + yb + xb], wx);
        out[p.out_off + c * plane + i] = mix(top, bottom, wy);
    }
}

// 3×3・縁 1 の畳み込みの、画素の少ない層向け。ワークグループ一つが一画素の 4 チャンネルを
// 受け持ち、64 本のスレッドが入力チャンネルを分けて持って、共有メモリで足し合わせる。
// 重みは並べ替えた物（16 の組の中の 4 の組を vec4 で）。
var<workgroup> partial: array<vec4<f32>, 64>;

@compute @workgroup_size(64)
fn conv3r(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wid: vec3<u32>) {
    let pixel = wid.x;
    let ox = pixel % p.w_out;
    let oy = pixel / p.w_out;
    let oc0 = wid.y * 4u;
    let og = oc0 / 16u;
    let jj = (oc0 % 16u) / 4u;
    let plane = p.h_in * p.w_in;
    var acc = vec4<f32>(0.0);
    for (var ic = li; ic < p.c_in; ic += 64u) {
        let wbase = ((og * p.c_in + ic) * 9u) * 4u + jj;
        for (var ky = 0u; ky < 3u; ky++) {
            let iy = i32(oy * p.stride + ky) - 1;
            if (iy < 0 || iy >= i32(p.h_in)) {
                continue;
            }
            for (var kx = 0u; kx < 3u; kx++) {
                let ix = i32(ox * p.stride + kx) - 1;
                if (ix < 0 || ix >= i32(p.w_in)) {
                    continue;
                }
                acc += x[p.x_off + ic * plane + u32(iy) * p.w_in + u32(ix)] * wt4[wbase + (ky * 3u + kx) * 4u];
            }
        }
    }
    partial[li] = acc;
    workgroupBarrier();
    for (var step = 32u; step > 0u; step = step / 2u) {
        if (li < step) {
            partial[li] = partial[li] + partial[li + step];
        }
        workgroupBarrier();
    }
    if (li == 0u) {
        let total = partial[0];
        let oplane = p.h_out * p.w_out;
        for (var e = 0u; e < 4u; e++) {
            let oc = oc0 + e;
            if (oc < p.c_out) {
                var v = total[e];
                if (p.has_bias == 1u) {
                    v += wt[p.bias_at + oc];
                }
                out[p.out_off + oc * oplane + pixel] = activate(v);
            }
        }
    }
}

// 3×3・ストライド 1・縁 1 の畳み込みを、共有メモリで。ワークグループが 64×8 画素
// ×16 チャンネルを受け持ち、スレッド一つが横に並んだ 4 画素 ×16 チャンネルを出す。
// 入力チャンネルは 4 枚ずつ、その切れ端（10×66）と重み（[c][k] ごとに 16 チャンネル
// = vec4 四つ）を置く。足し込む先は名前の付いた vec4。添字で参照する配列にすると
// レジスターに収まらずローカルメモリに追い出され、倍遅くなった（RTX 5060 Ti）。
// 下の積和は生成した物（展開して書いてある）。
const TW: u32 = 64u;
const TH: u32 = 8u;
const OC: u32 = 16u;
const CI: u32 = 4u;
const IW: u32 = 66u;
const IH: u32 = 10u;
var<workgroup> tile: array<f32, 2640>;
var<workgroup> wtile: array<vec4<f32>, 144>;

@compute @workgroup_size(16, 8, 1)
fn conv3(@builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>) {
    let x0 = wid.x * TW;
    let y0 = wid.y * TH;
    let oc0 = wid.z * OC;
    let li = lid.y * 16u + lid.x;
    let plane = p.h_in * p.w_in;
    var a00 = vec4<f32>(0.0);
    var a01 = vec4<f32>(0.0);
    var a02 = vec4<f32>(0.0);
    var a03 = vec4<f32>(0.0);
    var a10 = vec4<f32>(0.0);
    var a11 = vec4<f32>(0.0);
    var a12 = vec4<f32>(0.0);
    var a13 = vec4<f32>(0.0);
    var a20 = vec4<f32>(0.0);
    var a21 = vec4<f32>(0.0);
    var a22 = vec4<f32>(0.0);
    var a23 = vec4<f32>(0.0);
    var a30 = vec4<f32>(0.0);
    var a31 = vec4<f32>(0.0);
    var a32 = vec4<f32>(0.0);
    var a33 = vec4<f32>(0.0);
    for (var c0 = 0u; c0 < p.c_in; c0 += CI) {
        // どのスレッドも同じ回数だけ回す（FXC はバリアの前に回数の違うループがあるとコンパイルを拒否する）。
        for (var r = 0u; r < (CI * IH * IW + 127u) / 128u; r++) {
            let i = li + r * 128u;
            if (i < CI * IH * IW) {
            let c = i / (IH * IW);
            let r = (i / IW) % IH;
            let col = i % IW;
            let ic = c0 + c;
            let iy = i32(y0 + r) - 1;
            let ix = i32(x0 + col) - 1;
            var v = 0.0;
            if (ic < p.c_in && iy >= 0 && iy < i32(p.h_in) && ix >= 0 && ix < i32(p.w_in)) {
                v = x[p.x_off + ic * plane + u32(iy) * p.w_in + u32(ix)];
            }
            tile[i] = v;
            }
        }
        // 重み：wtile[(c·9 + k)·4 + j] の成分 e が出力チャンネル oc0 + 4j + e。
        for (var r = 0u; r < (CI * 9u * 4u + 127u) / 128u; r++) {
            let i = li + r * 128u;
            if (i < CI * 9u * 4u) {
            let c = i / 36u;
            var w4 = vec4<f32>(0.0);
            if (c0 + c < p.c_in) {
                w4 = wt4[((wid.z * p.c_in + c0 + c) * 9u) * 4u + i % 36u];
            }
            wtile[i] = w4;
            }
        }
        workgroupBarrier();
        for (var c = 0u; c < CI; c++) {
            for (var ky = 0u; ky < 3u; ky++) {
                let base = c * IH * IW + (lid.y + ky) * IW + lid.x * 4u;
                let v0 = tile[base + 0u];
                let v1 = tile[base + 1u];
                let v2 = tile[base + 2u];
                let v3 = tile[base + 3u];
                let v4 = tile[base + 4u];
                let v5 = tile[base + 5u];
                let wb = (c * 9u + ky * 3u) * 4u;
                let w00 = wtile[wb + 0u];
                let w01 = wtile[wb + 1u];
                let w02 = wtile[wb + 2u];
                let w03 = wtile[wb + 3u];
                let w10 = wtile[wb + 4u];
                let w11 = wtile[wb + 5u];
                let w12 = wtile[wb + 6u];
                let w13 = wtile[wb + 7u];
                let w20 = wtile[wb + 8u];
                let w21 = wtile[wb + 9u];
                let w22 = wtile[wb + 10u];
                let w23 = wtile[wb + 11u];
                a00 += v0 * w00 + v1 * w10 + v2 * w20;
                a01 += v0 * w01 + v1 * w11 + v2 * w21;
                a02 += v0 * w02 + v1 * w12 + v2 * w22;
                a03 += v0 * w03 + v1 * w13 + v2 * w23;
                a10 += v1 * w00 + v2 * w10 + v3 * w20;
                a11 += v1 * w01 + v2 * w11 + v3 * w21;
                a12 += v1 * w02 + v2 * w12 + v3 * w22;
                a13 += v1 * w03 + v2 * w13 + v3 * w23;
                a20 += v2 * w00 + v3 * w10 + v4 * w20;
                a21 += v2 * w01 + v3 * w11 + v4 * w21;
                a22 += v2 * w02 + v3 * w12 + v4 * w22;
                a23 += v2 * w03 + v3 * w13 + v4 * w23;
                a30 += v3 * w00 + v4 * w10 + v5 * w20;
                a31 += v3 * w01 + v4 * w11 + v5 * w21;
                a32 += v3 * w02 + v4 * w12 + v5 * w22;
                a33 += v3 * w03 + v4 * w13 + v5 * w23;
            }
        }
        workgroupBarrier();
    }
    let oy = y0 + lid.y;
    if (oy >= p.h_out) {
        return;
    }
    let oplane = p.h_out * p.w_out;
    store4(x0 + lid.x * 4u + 0u, oy, oc0, oplane, array<vec4<f32>, 4>(a00, a01, a02, a03));
    store4(x0 + lid.x * 4u + 1u, oy, oc0, oplane, array<vec4<f32>, 4>(a10, a11, a12, a13));
    store4(x0 + lid.x * 4u + 2u, oy, oc0, oplane, array<vec4<f32>, 4>(a20, a21, a22, a23));
    store4(x0 + lid.x * 4u + 3u, oy, oc0, oplane, array<vec4<f32>, 4>(a30, a31, a32, a33));
}

fn store4(ox: u32, oy: u32, oc0: u32, oplane: u32, acc: array<vec4<f32>, 4>) {
    if (ox >= p.w_out) {
        return;
    }
    for (var jj = 0u; jj < 4u; jj++) {
        for (var e = 0u; e < 4u; e++) {
            let oc = oc0 + jj * 4u + e;
            if (oc < p.c_out) {
                var v = acc[jj][e];
                if (p.has_bias == 1u) {
                    v += wt[p.bias_at + oc];
                }
                out[p.out_off + oc * oplane + oy * p.w_out + ox] = activate(v);
            }
        }
    }
}

// 3×3・縁 1 の畳み込み（ストライド 1 か 2、Winograd を使わない層）。スレッド一つが横に並んだ 4 画素
// ×16 チャンネル（名前付きの vec4 十六）。重みは入力 16 本分ずつ共有メモリに置いて、
// ワークグループの全員がそこから読む。1 画素ずつでは読み込み 5 回に積和 16 回で読み込みが勝ち、
// 12→32 で 1 TFLOPS 前後だった（RTX 2060）。範囲の外のスレッドも、バリアのために最後まで回す。
var<workgroup> w3s: array<vec4<f32>, 576>;   // 入力 16 × カーネルの位置 9 × vec4 四つ

@compute @workgroup_size(8, 8, 1)
fn conv3s(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let ox0 = id.x * 4u;
    let oy = id.y;
    let valid = ox0 < p.w_out && oy < p.h_out;
    let plane = p.h_in * p.w_in;
    var a00 = vec4<f32>(0.0);
    var a01 = vec4<f32>(0.0);
    var a02 = vec4<f32>(0.0);
    var a03 = vec4<f32>(0.0);
    var a10 = vec4<f32>(0.0);
    var a11 = vec4<f32>(0.0);
    var a12 = vec4<f32>(0.0);
    var a13 = vec4<f32>(0.0);
    var a20 = vec4<f32>(0.0);
    var a21 = vec4<f32>(0.0);
    var a22 = vec4<f32>(0.0);
    var a23 = vec4<f32>(0.0);
    var a30 = vec4<f32>(0.0);
    var a31 = vec4<f32>(0.0);
    var a32 = vec4<f32>(0.0);
    var a33 = vec4<f32>(0.0);
    for (var ic0 = 0u; ic0 < p.c_in; ic0 += 16u) {
        let n = min(16u, p.c_in - ic0);
        let wsrc = ((id.z * p.c_in + ic0) * 9u) * 4u;
        //@unroll r 0..9
        {
            let e = li + {r}u * 64u;
            if (e < n * 36u) {
                w3s[e] = wt4[wsrc + e];
            }
        }
        //@end
        workgroupBarrier();
        if (valid) {
            for (var icl = 0u; icl < n; icl++) {
                let base = p.x_off + (ic0 + icl) * plane;
                for (var ky = 0u; ky < 3u; ky++) {
                    let iy = i32(oy * p.stride + ky) - 1;
                    if (iy < 0 || iy >= i32(p.h_in)) {
                        continue;
                    }
                    let row = base + u32(iy) * p.w_in;
                    for (var kx = 0u; kx < 3u; kx++) {
                        let wb = (icl * 9u + ky * 3u + kx) * 4u;
                        let w0 = w3s[wb];
                        let w1 = w3s[wb + 1u];
                        let w2 = w3s[wb + 2u];
                        let w3 = w3s[wb + 3u];
                        //@unroll j 0..4
                        let ix{j} = i32((ox0 + {j}u) * p.stride + kx) - 1;
                        var v{j} = 0.0;
                        if (ix{j} >= 0 && ix{j} < i32(p.w_in)) {
                            v{j} = x[row + u32(ix{j})];
                        }
                        a{j}0 += v{j} * w0;
                        a{j}1 += v{j} * w1;
                        a{j}2 += v{j} * w2;
                        a{j}3 += v{j} * w3;
                        //@end
                    }
                }
            }
        }
        workgroupBarrier();
    }
    if (valid) {
        let oplane = p.h_out * p.w_out;
        store4(ox0 + 0u, oy, id.z * 16u, oplane, array<vec4<f32>, 4>(a00, a01, a02, a03));
        store4(ox0 + 1u, oy, id.z * 16u, oplane, array<vec4<f32>, 4>(a10, a11, a12, a13));
        store4(ox0 + 2u, oy, id.z * 16u, oplane, array<vec4<f32>, 4>(a20, a21, a22, a23));
        store4(ox0 + 3u, oy, id.z * 16u, oplane, array<vec4<f32>, 4>(a30, a31, a32, a33));
    }
}

// 4×4・ストライド 2・縁 1 の逆畳み込みを、並べ替えた重みで（16 チャンネルずつ）。
// 出力 (2m+a, 2n+b) が使うカーネルの位置は偶奇で決まる：a = 0 なら (ky=1, iy=m) と
// (ky=3, iy=m−1)、a = 1 なら (ky=0, iy=m+1) と (ky=2, iy=m)。スレッド一つが入力の
// 一点 (m, n) から出力の 2×2 を出す。重みは [16 の組][入力][16 のカーネルの位置] の vec4 四つで、
// 入力 8 本分ずつ共有メモリに置いて全員で読む。
var<workgroup> w4s: array<vec4<f32>, 512>;   // 入力 8 × カーネルの位置 16 × vec4 四つ
fn tap(ic: u32, iy: i32, ix: i32) -> f32 {
    if (iy < 0 || iy >= i32(p.h_in) || ix < 0 || ix >= i32(p.w_in)) {
        return 0.0;
    }
    return x[p.x_off + ic * p.h_in * p.w_in + u32(iy) * p.w_in + u32(ix)];
}

@compute @workgroup_size(8, 8, 1)
fn deconv4(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let n = i32(id.x);
    let m = i32(id.y);
    let valid = id.x < p.w_in && id.y < p.h_in;
    var o00 = vec4<f32>(0.0);
    var o01 = vec4<f32>(0.0);
    var o02 = vec4<f32>(0.0);
    var o03 = vec4<f32>(0.0);
    var o10 = vec4<f32>(0.0);
    var o11 = vec4<f32>(0.0);
    var o12 = vec4<f32>(0.0);
    var o13 = vec4<f32>(0.0);
    var o20 = vec4<f32>(0.0);
    var o21 = vec4<f32>(0.0);
    var o22 = vec4<f32>(0.0);
    var o23 = vec4<f32>(0.0);
    var o30 = vec4<f32>(0.0);
    var o31 = vec4<f32>(0.0);
    var o32 = vec4<f32>(0.0);
    var o33 = vec4<f32>(0.0);
    for (var ic0 = 0u; ic0 < p.c_in; ic0 += 8u) {
      let nch = min(8u, p.c_in - ic0);
      let wsrc = ((id.z * p.c_in + ic0) * 16u) * 4u;
      //@unroll r 0..8
      {
          let e = li + {r}u * 64u;
          if (e < nch * 64u) {
              w4s[e] = wt4[wsrc + e];
          }
      }
      //@end
      workgroupBarrier();
      for (var icl = 0u; icl < nch && valid; icl++) {
        let ic = ic0 + icl;
        let wbase = icl * 64u;
        let t00 = tap(ic, m + (-1), n + (-1));
        let t01 = tap(ic, m + (-1), n + (0));
        let t02 = tap(ic, m + (-1), n + (1));
        let t10 = tap(ic, m + (0), n + (-1));
        let t11 = tap(ic, m + (0), n + (0));
        let t12 = tap(ic, m + (0), n + (1));
        let t20 = tap(ic, m + (1), n + (-1));
        let t21 = tap(ic, m + (1), n + (0));
        let t22 = tap(ic, m + (1), n + (1));
        o00 += t11 * w4s[wbase + 20u];
        o01 += t11 * w4s[wbase + 21u];
        o02 += t11 * w4s[wbase + 22u];
        o03 += t11 * w4s[wbase + 23u];
        o00 += t10 * w4s[wbase + 28u];
        o01 += t10 * w4s[wbase + 29u];
        o02 += t10 * w4s[wbase + 30u];
        o03 += t10 * w4s[wbase + 31u];
        o00 += t01 * w4s[wbase + 52u];
        o01 += t01 * w4s[wbase + 53u];
        o02 += t01 * w4s[wbase + 54u];
        o03 += t01 * w4s[wbase + 55u];
        o00 += t00 * w4s[wbase + 60u];
        o01 += t00 * w4s[wbase + 61u];
        o02 += t00 * w4s[wbase + 62u];
        o03 += t00 * w4s[wbase + 63u];
        o10 += t12 * w4s[wbase + 16u];
        o11 += t12 * w4s[wbase + 17u];
        o12 += t12 * w4s[wbase + 18u];
        o13 += t12 * w4s[wbase + 19u];
        o10 += t11 * w4s[wbase + 24u];
        o11 += t11 * w4s[wbase + 25u];
        o12 += t11 * w4s[wbase + 26u];
        o13 += t11 * w4s[wbase + 27u];
        o10 += t02 * w4s[wbase + 48u];
        o11 += t02 * w4s[wbase + 49u];
        o12 += t02 * w4s[wbase + 50u];
        o13 += t02 * w4s[wbase + 51u];
        o10 += t01 * w4s[wbase + 56u];
        o11 += t01 * w4s[wbase + 57u];
        o12 += t01 * w4s[wbase + 58u];
        o13 += t01 * w4s[wbase + 59u];
        o20 += t21 * w4s[wbase + 4u];
        o21 += t21 * w4s[wbase + 5u];
        o22 += t21 * w4s[wbase + 6u];
        o23 += t21 * w4s[wbase + 7u];
        o20 += t20 * w4s[wbase + 12u];
        o21 += t20 * w4s[wbase + 13u];
        o22 += t20 * w4s[wbase + 14u];
        o23 += t20 * w4s[wbase + 15u];
        o20 += t11 * w4s[wbase + 36u];
        o21 += t11 * w4s[wbase + 37u];
        o22 += t11 * w4s[wbase + 38u];
        o23 += t11 * w4s[wbase + 39u];
        o20 += t10 * w4s[wbase + 44u];
        o21 += t10 * w4s[wbase + 45u];
        o22 += t10 * w4s[wbase + 46u];
        o23 += t10 * w4s[wbase + 47u];
        o30 += t22 * w4s[wbase + 0u];
        o31 += t22 * w4s[wbase + 1u];
        o32 += t22 * w4s[wbase + 2u];
        o33 += t22 * w4s[wbase + 3u];
        o30 += t21 * w4s[wbase + 8u];
        o31 += t21 * w4s[wbase + 9u];
        o32 += t21 * w4s[wbase + 10u];
        o33 += t21 * w4s[wbase + 11u];
        o30 += t12 * w4s[wbase + 32u];
        o31 += t12 * w4s[wbase + 33u];
        o32 += t12 * w4s[wbase + 34u];
        o33 += t12 * w4s[wbase + 35u];
        o30 += t11 * w4s[wbase + 40u];
        o31 += t11 * w4s[wbase + 41u];
        o32 += t11 * w4s[wbase + 42u];
        o33 += t11 * w4s[wbase + 43u];
    }
      workgroupBarrier();
    }
    if (!valid) {
        return;
    }
    let oplane = p.h_out * p.w_out;
    store4(u32(2 * n + 0), u32(2 * m + 0), id.z * 16u, oplane, array<vec4<f32>, 4>(o00, o01, o02, o03));
    store4(u32(2 * n + 1), u32(2 * m + 0), id.z * 16u, oplane, array<vec4<f32>, 4>(o10, o11, o12, o13));
    store4(u32(2 * n + 0), u32(2 * m + 1), id.z * 16u, oplane, array<vec4<f32>, 4>(o20, o21, o22, o23));
    store4(u32(2 * n + 1), u32(2 * m + 1), id.z * 16u, oplane, array<vec4<f32>, 4>(o30, o31, o32, o33));
}


// 出力が 8 以下の 4×4・ストライド 2・縁 1 の逆畳み込み。スレッド一つが入力の一点から出力の 2×2 を
// 4 チャンネル分（vec4 四つ）。16 ずつの物では出力 4 の層で 4 分の 3 が無駄だった。
// 重みは deconv4 と同じ並べ替え（16 の組の中の 4 の組を vec4 で読む）。
@compute @workgroup_size(8, 8, 1)
fn deconv4s(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = i32(id.x);
    let m = i32(id.y);
    if (id.x >= p.w_in || id.y >= p.h_in) {
        return;
    }
    let oc0 = id.z * 4u;
    let og = oc0 / 16u;
    let jj = (oc0 % 16u) / 4u;
    var o0 = vec4<f32>(0.0);
    var o1 = vec4<f32>(0.0);
    var o2 = vec4<f32>(0.0);
    var o3 = vec4<f32>(0.0);
    for (var ic = 0u; ic < p.c_in; ic++) {
        let wbase = ((og * p.c_in + ic) * 16u) * 4u + jj;
        let t00 = tap(ic, m + (-1), n + (-1));
        let t01 = tap(ic, m + (-1), n + (0));
        let t02 = tap(ic, m + (-1), n + (1));
        let t10 = tap(ic, m + (0), n + (-1));
        let t11 = tap(ic, m + (0), n + (0));
        let t12 = tap(ic, m + (0), n + (1));
        let t20 = tap(ic, m + (1), n + (-1));
        let t21 = tap(ic, m + (1), n + (0));
        let t22 = tap(ic, m + (1), n + (1));
        o0 += t11 * wt4[wbase + 20u];
        o0 += t10 * wt4[wbase + 28u];
        o0 += t01 * wt4[wbase + 52u];
        o0 += t00 * wt4[wbase + 60u];
        o1 += t12 * wt4[wbase + 16u];
        o1 += t11 * wt4[wbase + 24u];
        o1 += t02 * wt4[wbase + 48u];
        o1 += t01 * wt4[wbase + 56u];
        o2 += t21 * wt4[wbase + 4u];
        o2 += t20 * wt4[wbase + 12u];
        o2 += t11 * wt4[wbase + 36u];
        o2 += t10 * wt4[wbase + 44u];
        o3 += t22 * wt4[wbase + 0u];
        o3 += t21 * wt4[wbase + 8u];
        o3 += t12 * wt4[wbase + 32u];
        o3 += t11 * wt4[wbase + 40u];
    }
    let oplane = p.h_out * p.w_out;
    store_quad(u32(2 * n + 0), u32(2 * m + 0), oc0, oplane, o0);
    store_quad(u32(2 * n + 1), u32(2 * m + 0), oc0, oplane, o1);
    store_quad(u32(2 * n + 0), u32(2 * m + 1), oc0, oplane, o2);
    store_quad(u32(2 * n + 1), u32(2 * m + 1), oc0, oplane, o3);
}

fn store_quad(ox: u32, oy: u32, oc0: u32, oplane: u32, acc: vec4<f32>) {
    if (ox >= p.w_out || oy >= p.h_out) {
        return;
    }
    for (var e = 0u; e < 4u; e++) {
        let oc = oc0 + e;
        if (oc < p.c_out) {
            var v = acc[e];
            if (p.has_bias == 1u) {
                v += wt[p.bias_at + oc];
            }
            out[p.out_off + oc * oplane + oy * p.w_out + ox] = activate(v);
        }
    }
}
