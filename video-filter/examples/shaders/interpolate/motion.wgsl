// 動き補償のフレーム補間。前のフレームと今のフレームの間を、目標のコマ数（画面のリフレッシュ
// レートか、設定した fps）で埋める。ブロックの動きを探して画像をずらして混ぜる方式
// （学習したネットワークは使わない）。
//
// 1. pyr0〜pyr3：今と前の輝度を 1・1/2・1/4・1/8 に（r が今、g が前）。
// 2. me3：1/8 のレベルの 4×4 画素のブロックごとに、周り 8×8 のウィンドウで対称の動き（今は +d、前は −d）を
//    広く全部試す（まず半分の画素で比べ、最良の周りを全部で）。
// 3. me2・me1：粗いレベルから細かいレベルへ。親と親の隣・0 を候補に選び、±1 で絞り込む。コストは差の
//    和に、手がかり（親）から離れるほど足すペナルティ（動きを周りと揃える）。me1 は半画素・1/4 画素
//    まで（元の大きさで 1 画素刻み）。ウィンドウが隣と重なるので、ブロックの境目で動きが飛びにくい。
//    読み込みは四画素ずつ（textureGather）。
// 4. med：3×3 のブロックの動きの中央値（外れた動きを周りに合わせる）と、動きベクトル場の荒れ具合。
// 5. conf：ブロックごとの信頼度（動きを探したときの差と荒れ具合）を、周りの 3×3 で平均する。
// 6. blend：画素ごとに近い 4 ブロックの動きを試し、前と今が合う動きほど重く混ぜる（ブロックの
//    継ぎ目が出ず、物の境目では合う方の動きが勝つ）。合わない画素は、動きベクトル場が寄り集まる
//    所（隠れていく物）なら前のフレーム、離れていく所（現れる物）なら今のフレームの側を使う。信頼度の
//    ない所は、滑らかに混ぜるだけへ寄せる（画像がちぎれるより二重に見える方がまし）。
//
// 動きの値は元の大きさの画素で、前から今への動き M。1920×1080 で、Apple M2 が約 7 ms、
// RTX 5060 Ti が約 1.3 ms。

//!HISTORY 1
//!RETIME
//!TEXTURE l0 input.w input.h rg16float
//!TEXTURE l1 input.w 2 / input.h 2 / rg16float
//!TEXTURE l2 input.w 4 / input.h 4 / rg16float
//!TEXTURE l3 input.w 8 / input.h 8 / rg16float
//!TEXTURE v3 input.w 31 + 32 / input.h 31 + 32 / rgba16float nearest
//!TEXTURE v2 input.w 15 + 16 / input.h 15 + 16 / rgba16float nearest
//!TEXTURE v1 input.w 7 + 8 / input.h 7 + 8 / rgba16float nearest
//!TEXTURE vm input.w 7 + 8 / input.h 7 + 8 / rgba16float nearest
//!TEXTURE cf input.w 7 + 8 / input.h 7 + 8 / r16float

//!PASS pyr0
//!READS picture picture:1
//!WRITES l0
@fragment
fn main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let w = vec3<f32>(0.299, 0.587, 0.114);
    let p = vec2<i32>(floor(pos.xy));
    return vec4<f32>(dot(textureLoad(picture, p, 0).rgb, w), dot(textureLoad(picture_1, p, 0).rgb, w), 0.0, 1.0);
}

//!PASS pyr1
//!READS l0
//!WRITES l1
@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    // 線形のサンプラーで 2×2 の真ん中をサンプリングすると、その四画素の平均。
    return vec4<f32>(textureSampleLevel(l0, l0_smp, uv, 0.0).rg, 0.0, 1.0);
}

//!PASS pyr2
//!READS l1
//!WRITES l2
@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSampleLevel(l1, l1_smp, uv, 0.0).rg, 0.0, 1.0);
}

//!PASS pyr3
//!READS l2
//!WRITES l3
@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSampleLevel(l2, l2_smp, uv, 0.0).rg, 0.0, 1.0);
}

//!PASS me3
//!READS l3
//!WRITES v3
fn cost(b: vec2<i32>, d: vec2<i32>) -> f32 {
    // ブロック b（4×4 画素）の周り 8×8 のウィンドウ。四画素ずつ一度に読む（textureGather）。
    let size = vec2<f32>(textureDimensions(l3));
    let o = b * 4 - 2;
    var s = 0.0;
    for (var j = 0; j < 4; j++) {
        for (var i = 0; i < 4; i++) {
            let c = vec2<f32>(o + vec2<i32>(2 * i + 1, 2 * j + 1));
            let now = textureGather(0, l3, l3_smp, (c + vec2<f32>(d)) / size);
            let before = textureGather(1, l3, l3_smp, (c - vec2<f32>(d)) / size);
            let e = abs(now - before);
            s += e.x + e.y + e.z + e.w;
        }
    }
    return s;
}

// 半分の画素だけの差（総当たり用、二倍して比べる）。
fn cost_coarse(b: vec2<i32>, d: vec2<i32>) -> f32 {
    let size = vec2<f32>(textureDimensions(l3));
    let o = b * 4 - 2;
    var s = 0.0;
    for (var j = 0; j < 4; j++) {
        for (var i = (j % 2); i < 4; i += 2) {
            let c = vec2<f32>(o + vec2<i32>(2 * i + 1, 2 * j + 1));
            let now = textureGather(0, l3, l3_smp, (c + vec2<f32>(d)) / size);
            let before = textureGather(1, l3, l3_smp, (c - vec2<f32>(d)) / size);
            let e = abs(now - before);
            s += e.x + e.y + e.z + e.w;
        }
    }
    return s * 2.0;
}

// 端数の動きの差（線形でサンプリングする。四画素の真ん中をサンプリングすると 2×2 の平均）。
fn cost_fine(b: vec2<i32>, d: vec2<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(l3));
    let o = b * 4 - 2;
    var s = 0.0;
    for (var j = 0; j < 4; j++) {
        for (var i = 0; i < 4; i++) {
            let c = vec2<f32>(o + vec2<i32>(2 * i + 1, 2 * j + 1));
            let now = textureSampleLevel(l3, l3_smp, (c + d) / size, 0.0).r;
            let before = textureSampleLevel(l3, l3_smp, (c - d) / size, 0.0).g;
            s += abs(now - before) * 4.0;
        }
    }
    return s;
}

@fragment
fn main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let b = vec2<i32>(floor(pos.xy));
    var best = vec2<i32>(0);
    var best_cost = cost_coarse(b, best);
    for (var dy = -4; dy <= 4; dy++) {
        for (var dx = -4; dx <= 4; dx++) {
            let d = vec2<i32>(dx, dy);
            let c = cost_coarse(b, d) + 1.0 * f32(abs(dx) + abs(dy));
            if (c < best_cost) {
                best_cost = c;
                best = d;
            }
        }
    }
    let centre = best;
    best_cost = 1e9;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let d = centre + vec2<i32>(dx, dy);
            let c = cost(b, d) + 1.0 * f32(abs(d.x) + abs(d.y));
            if (c < best_cost) {
                best_cost = c;
                best = d;
            }
        }
    }
    return vec4<f32>(vec2<f32>(best) * 16.0, best_cost, 1.0);
}

//!PASS me2
//!READS l2 v3
//!WRITES v2
fn cost(b: vec2<i32>, d: vec2<i32>) -> f32 {
    // ブロック b（4×4 画素）の周り 8×8 のウィンドウ。四画素ずつ一度に読む（textureGather）。
    let size = vec2<f32>(textureDimensions(l2));
    let o = b * 4 - 2;
    var s = 0.0;
    for (var j = 0; j < 4; j++) {
        for (var i = 0; i < 4; i++) {
            let c = vec2<f32>(o + vec2<i32>(2 * i + 1, 2 * j + 1));
            let now = textureGather(0, l2, l2_smp, (c + vec2<f32>(d)) / size);
            let before = textureGather(1, l2, l2_smp, (c - vec2<f32>(d)) / size);
            let e = abs(now - before);
            s += e.x + e.y + e.z + e.w;
        }
    }
    return s;
}

// 半分の画素だけの差（総当たり用、二倍して比べる）。
fn cost_coarse(b: vec2<i32>, d: vec2<i32>) -> f32 {
    let size = vec2<f32>(textureDimensions(l2));
    let o = b * 4 - 2;
    var s = 0.0;
    for (var j = 0; j < 4; j++) {
        for (var i = (j % 2); i < 4; i += 2) {
            let c = vec2<f32>(o + vec2<i32>(2 * i + 1, 2 * j + 1));
            let now = textureGather(0, l2, l2_smp, (c + vec2<f32>(d)) / size);
            let before = textureGather(1, l2, l2_smp, (c - vec2<f32>(d)) / size);
            let e = abs(now - before);
            s += e.x + e.y + e.z + e.w;
        }
    }
    return s * 2.0;
}

// 端数の動きの差（線形でサンプリングする。四画素の真ん中をサンプリングすると 2×2 の平均）。
fn cost_fine(b: vec2<i32>, d: vec2<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(l2));
    let o = b * 4 - 2;
    var s = 0.0;
    for (var j = 0; j < 4; j++) {
        for (var i = 0; i < 4; i++) {
            let c = vec2<f32>(o + vec2<i32>(2 * i + 1, 2 * j + 1));
            let now = textureSampleLevel(l2, l2_smp, (c + d) / size, 0.0).r;
            let before = textureSampleLevel(l2, l2_smp, (c - d) / size, 0.0).g;
            s += abs(now - before) * 4.0;
        }
    }
    return s;
}

fn parent_at(b: vec2<i32>) -> vec2<f32> {
    let size = vec2<i32>(textureDimensions(v3));
    return textureLoad(v3, clamp(b, vec2<i32>(0), size - 1), 0).xy;
}

fn here(m: vec2<f32>) -> vec2<i32> {
    return vec2<i32>(round(m / 8.0));
}

@fragment
fn main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let b = vec2<i32>(floor(pos.xy));
    let up = b / 2;
    let guide = here(parent_at(up));
    var cands = array<vec2<i32>, 6>(
        guide,
        here(parent_at(up + vec2<i32>(-1, 0))),
        here(parent_at(up + vec2<i32>(1, 0))),
        here(parent_at(up + vec2<i32>(0, -1))),
        here(parent_at(up + vec2<i32>(0, 1))),
        vec2<i32>(0)
    );
    var best = guide;
    var best_cost = 1e9;
    for (var k = 0; k < 6; k++) {
        let d = cands[k];
        let off = d - guide;
        let c = cost(b, d) + 1.0 * f32(abs(off.x) + abs(off.y));
        if (c < best_cost) {
            best_cost = c;
            best = d;
        }
    }
    let centre = best;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            if (dx == 0 && dy == 0) {
                continue;
            }
            let d = centre + vec2<i32>(dx, dy);
            let off = d - guide;
            let c = cost(b, d) + 1.0 * f32(abs(off.x) + abs(off.y));
            if (c < best_cost) {
                best_cost = c;
                best = d;
            }
        }
    }
    return vec4<f32>(vec2<f32>(best) * 8.0, best_cost, 1.0);
}

//!PASS me1
//!READS l1 v2
//!WRITES v1
fn cost(b: vec2<i32>, d: vec2<i32>) -> f32 {
    // ブロック b（4×4 画素）の周り 8×8 のウィンドウ。四画素ずつ一度に読む（textureGather）。
    let size = vec2<f32>(textureDimensions(l1));
    let o = b * 4 - 2;
    var s = 0.0;
    for (var j = 0; j < 4; j++) {
        for (var i = 0; i < 4; i++) {
            let c = vec2<f32>(o + vec2<i32>(2 * i + 1, 2 * j + 1));
            let now = textureGather(0, l1, l1_smp, (c + vec2<f32>(d)) / size);
            let before = textureGather(1, l1, l1_smp, (c - vec2<f32>(d)) / size);
            let e = abs(now - before);
            s += e.x + e.y + e.z + e.w;
        }
    }
    return s;
}

// 半分の画素だけの差（総当たり用、二倍して比べる）。
fn cost_coarse(b: vec2<i32>, d: vec2<i32>) -> f32 {
    let size = vec2<f32>(textureDimensions(l1));
    let o = b * 4 - 2;
    var s = 0.0;
    for (var j = 0; j < 4; j++) {
        for (var i = (j % 2); i < 4; i += 2) {
            let c = vec2<f32>(o + vec2<i32>(2 * i + 1, 2 * j + 1));
            let now = textureGather(0, l1, l1_smp, (c + vec2<f32>(d)) / size);
            let before = textureGather(1, l1, l1_smp, (c - vec2<f32>(d)) / size);
            let e = abs(now - before);
            s += e.x + e.y + e.z + e.w;
        }
    }
    return s * 2.0;
}

// 端数の動きの差（線形でサンプリングする。四画素の真ん中をサンプリングすると 2×2 の平均）。
fn cost_fine(b: vec2<i32>, d: vec2<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(l1));
    let o = b * 4 - 2;
    var s = 0.0;
    for (var j = 0; j < 4; j++) {
        for (var i = 0; i < 4; i++) {
            let c = vec2<f32>(o + vec2<i32>(2 * i + 1, 2 * j + 1));
            let now = textureSampleLevel(l1, l1_smp, (c + d) / size, 0.0).r;
            let before = textureSampleLevel(l1, l1_smp, (c - d) / size, 0.0).g;
            s += abs(now - before) * 4.0;
        }
    }
    return s;
}

fn parent_at(b: vec2<i32>) -> vec2<f32> {
    let size = vec2<i32>(textureDimensions(v2));
    return textureLoad(v2, clamp(b, vec2<i32>(0), size - 1), 0).xy;
}

fn here(m: vec2<f32>) -> vec2<i32> {
    return vec2<i32>(round(m / 4.0));
}

@fragment
fn main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let b = vec2<i32>(floor(pos.xy));
    let up = b / 2;
    let guide = here(parent_at(up));
    var cands = array<vec2<i32>, 6>(
        guide,
        here(parent_at(up + vec2<i32>(-1, 0))),
        here(parent_at(up + vec2<i32>(1, 0))),
        here(parent_at(up + vec2<i32>(0, -1))),
        here(parent_at(up + vec2<i32>(0, 1))),
        vec2<i32>(0)
    );
    var best = guide;
    var best_cost = 1e9;
    for (var k = 0; k < 6; k++) {
        let d = cands[k];
        let off = d - guide;
        let c = cost(b, d) + 1.0 * f32(abs(off.x) + abs(off.y));
        if (c < best_cost) {
            best_cost = c;
            best = d;
        }
    }
    let centre = best;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            if (dx == 0 && dy == 0) {
                continue;
            }
            let d = centre + vec2<i32>(dx, dy);
            let off = d - guide;
            let c = cost(b, d) + 1.0 * f32(abs(off.x) + abs(off.y));
            if (c < best_cost) {
                best_cost = c;
                best = d;
            }
        }
    }
    // 半画素、続いて 1/4 画素。
    var f = vec2<f32>(best);
    var f_cost = cost_fine(b, f);
    for (var r = 0; r < 2; r++) {
        let step = select(0.25, 0.5, r == 0);
        let centre_f = f;
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                if (dx == 0 && dy == 0) {
                    continue;
                }
                let d = centre_f + vec2<f32>(f32(dx), f32(dy)) * step;
                let off = d - vec2<f32>(guide);
                let c = cost_fine(b, d) + 1.0 * (abs(off.x) + abs(off.y));
                if (c < f_cost) {
                    f_cost = c;
                    f = d;
                }
            }
        }
    }
    // z はペナルティを除いた差（ウィンドウの 64 画素の和）。混ぜるときの信頼度に使う。
    return vec4<f32>(f * 4.0, cost_fine(b, f), 1.0);
}

//!PASS med
//!READS v1
//!WRITES vm
fn mv(b: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(v1));
    return textureLoad(v1, clamp(b, vec2<i32>(0), size - 1), 0);
}

@fragment
fn main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let b = vec2<i32>(floor(pos.xy));
    // 3×3 の動きのうち、ほかの八つとの距離の和が一番小さい物（ベクトルの中央値）。
    var vs: array<vec2<f32>, 9>;
    var n = 0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            vs[n] = mv(b + vec2<i32>(dx, dy)).xy;
            n++;
        }
    }
    var best = vs[4];
    var best_sum = 1e9;
    for (var a = 0; a < 9; a++) {
        var sum = 0.0;
        for (var c = 0; c < 9; c++) {
            let e = vs[a] - vs[c];
            sum += abs(e.x) + abs(e.y);
        }
        if (sum < best_sum) {
            best_sum = sum;
            best = vs[a];
        }
    }
    // w：周りの八つが中央値からどれだけずれるか（平均、画素）。動きベクトル場の荒れ具合。
    var rough = 0.0;
    for (var a = 0; a < 9; a++) {
        let e = vs[a] - best;
        rough += length(e);
    }
    return vec4<f32>(best, mv(b).z, rough / 8.0);
}

//!PASS conf
//!READS vm
//!WRITES cf
// ブロックごとの信頼度（動きを探したときの差と、動きベクトル場の荒れ具合）を、周りの 3×3 で平均する。
// 混ぜるときは線形でつなぐので、信頼度の低い所へは広く滑らかに寄る（ブロックの形が出ない）。
fn conf_at(b: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(vm));
    let v = textureLoad(vm, clamp(b, vec2<i32>(0), size - 1), 0);
    return (1.0 - smoothstep(0.04, 0.12, v.z / 64.0)) * (1.0 - smoothstep(8.0, 32.0, v.w));
}

@fragment
fn main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let b = vec2<i32>(floor(pos.xy));
    var sum = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            sum += conf_at(b + vec2<i32>(dx, dy));
        }
    }
    return vec4<f32>(sum / 9.0, 0.0, 0.0, 1.0);
}

//!PASS blend
//!READS picture picture:1 vm cf
fn mv(b: vec2<i32>) -> vec2<f32> {
    let size = vec2<i32>(textureDimensions(vm));
    return textureLoad(vm, clamp(b, vec2<i32>(0), size - 1), 0).xy;
}

fn rough_at(b: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(vm));
    return textureLoad(vm, clamp(b, vec2<i32>(0), size - 1), 0).w;
}

fn cost_at(b: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(vm));
    return textureLoad(vm, clamp(b, vec2<i32>(0), size - 1), 0).z;
}

// ブロックの座標 g（ブロックの真ん中が整数）での動き（双線形）。
fn field(g: vec2<f32>) -> vec2<f32> {
    let i = vec2<i32>(floor(g));
    let f = g - floor(g);
    return mix(mix(mv(i), mv(i + vec2<i32>(1, 0)), f.x), mix(mv(i + vec2<i32>(0, 1)), mv(i + vec2<i32>(1, 1)), f.x), f.y);
}

@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(picture));
    let px = 1.0 / size;
    let t = clamp(zw.phase, 0.0, 1.0);
    let w = vec3<f32>(0.299, 0.587, 0.114);
    // 近い 4 ブロック（ブロックの真ん中で 1、隣へ行くほど下がる重み）。
    let q = uv * size / 8.0 - 0.5;
    let b0 = vec2<i32>(floor(q));
    let f = q - floor(q);
    var off = array<vec2<f32>, 4>(vec2<f32>(1.5, 0.0), vec2<f32>(-1.5, 0.0), vec2<f32>(0.0, 1.5), vec2<f32>(0.0, -1.5));
    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    var pix_sure = 0.0;
    for (var k = 0; k < 4; k++) {
        let o = vec2<i32>(k % 2, k / 2);
        let m = mv(b0 + o) * px;
        let bw = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
        let a = textureSampleLevel(picture_1, picture_1_smp, uv - m * t, 0.0).rgb;
        let c = textureSampleLevel(picture, picture_smp, uv + m * (1.0 - t), 0.0).rgb;
        // 合い方：真ん中と ±1.5 画素の四か所の輝度の差。
        var err = abs(dot(a - c, w));
        for (var j = 0; j < 4; j++) {
            let s = off[j] * px;
            let a2 = textureSampleLevel(picture_1, picture_1_smp, uv - m * t + s, 0.0).rgb;
            let c2 = textureSampleLevel(picture, picture_smp, uv + m * (1.0 - t) + s, 0.0).rgb;
            err += abs(dot(a2 - c2, w));
        }
        err = err * 0.2;
        // 信頼度はブロックの重みで足す（一番合う物だけを見ると、ブロックの境目で候補の組が入れ替わって飛ぶ）。
        pix_sure += bw * exp(-err * err / 0.1);
        let weight = bw * exp(-err * err / 0.0064) + 1e-4 * bw;
        sum += mix(a, c, t) * weight;
        wsum += weight;
    }
    let moved = sum / wsum;
    let still = mix(textureSampleLevel(picture_1, picture_1_smp, uv, 0.0).rgb, textureSampleLevel(picture, picture_smp, uv, 0.0).rgb, t);
    // 画素の合い方と、ブロックの信頼度（conf のパスで滑らかにした物）。
    let block_conf = textureSampleLevel(cf, cf_smp, uv, 0.0).r;
    let agree = block_conf;
    let sure = pix_sure * block_conf;
    var fallback = still;
    if (true) {
        // 隠れる・現れる所：動きベクトル場の発散（±K 画素のブロックの動きの差）。寄り集まる（負）なら
        // 前のフレームにだけある物が隠れていく所なので前の側、離れていく（正）なら今の側。
        let g = uv * size / 8.0 - 0.5;
        let k = 1.5;
        let vxp = field(g + vec2<f32>(k, 0.0));
        let vxm = field(g - vec2<f32>(k, 0.0));
        let vyp = field(g + vec2<f32>(0.0, k));
        let vym = field(g - vec2<f32>(0.0, k));
        let div = (vxp.x - vxm.x) + (vyp.y - vym.y);
        let m = field(g) * px;
        let a = textureSampleLevel(picture_1, picture_1_smp, uv - m * t, 0.0).rgb;
        let c = textureSampleLevel(picture, picture_smp, uv + m * (1.0 - t), 0.0).rgb;
        let side = smoothstep(-6.0, 6.0, div);
        let one = mix(a, c, side);
        let strength = smoothstep(6.0 * 0.5, 6.0 * 2.0, abs(div));
        fallback = mix(still, one, strength);
    }
    // 動きが一致しない所は、片側を選ばずに混ぜるだけ（選ぶと画像がちぎれる）。
    fallback = mix(still, fallback, agree);
    return vec4<f32>(mix(fallback, moved, sure), 1.0);
}
