{{ENABLE}}
// 3×3・ストライド 1・縁 1 の畳み込みを Winograd F(4,3) で（ncnn と同じ分離型）。
//
//   入力変換：4×4 の出力タイルごとに、6×6 の入力 d を V = Bᵀ d B にして
//             U[36][入力][タイル] へ（隣のスレッドが隣のタイルを読み書きする）。
//   行列積  ：36 個まとめて M[b] = U[b]（タイル × 入力）× W[b]（入力 × 出力）。
//             U と M は [36][チャンネル][タイル]（行列で言えば列の向きに並ぶ）。
//   出力変換：Y = Aᵀ m A で 4×4 に戻し、バイアスと活性化を足して書き出す。
//
// 重み W[b] = G g Gᵀ は手元で計算して [36][入力][出力] に並べてある。掛け算は直接の
// 畳み込みの 1/4（36/(16×9)）。行列積はどの GPU でも動くふつうの WGSL。
//
// 作業バッファ（U と M）の型 S は、f16 が使えれば f16（読み書きが半分。ncnn も fp16 で持つ）、
// 無ければ f32。計算はいつも f32。先頭の `{{ENABLE}}` と `{{S}}` をビルドするときに埋める。

struct P {
    c_in: u32,
    h_in: u32,
    w_in: u32,
    c_out: u32,
    h_out: u32,
    w_out: u32,
    stride: u32,
    pad: u32,
    tiles: u32,    // タイルの数（行列積の行の数）
    act: u32,
    op: u32,
    c_b: u32,
    slope: f32,
    scalar: f32,
    scale_h: f32,
    scale_w: f32,
    coef_a: f32,
    coef_b: f32,
    bias_at: u32,
    has_bias: u32,
    x_off: u32,
    y_off: u32,
    out_off: u32,
    _pad: u32,
}

alias S = {{S}};

@group(2) @binding(0) var<storage, read> x: array<f32>;
@group(2) @binding(2) var<storage, read_write> out: array<f32>;
@group(2) @binding(3) var<storage, read> wt: array<f32>;
@group(2) @binding(4) var<uniform> p: P;
// 作業バッファ：U を書く（入力変換）・読む（行列積）、M を書く（行列積）・読む（出力変換）。
@group(2) @binding(5) var<storage, read> u_in: array<S>;
@group(2) @binding(6) var<storage, read> m_in: array<S>;
@group(2) @binding(7) var<storage, read_write> u_out: array<S>;
@group(2) @binding(8) var<storage, read_write> m_out: array<S>;
// 書き出しの処理に融合した物：残差（足す）とチャンネルごとの係数（掛ける）。使わないときは重みのバッファを仮にバインドする。
// p.op の bit0 が残差（位置 p.y_off）、bit1 が係数（位置 p.c_b）。
@group(2) @binding(1) var<storage, read> res: array<f32>;
@group(2) @binding(9) var<storage, read> beta: array<f32>;

// 入力変換：スレッド一つが一つのタイルの一つの入力チャンネル。id.x がタイル、id.y が入力
// チャンネル（隣のスレッドが隣のタイル：読むのも書くのも続いたアドレスになる）。決まった回数のループは
// `//@unroll` で展開して、6×6 を名前の付いた値に置く。
@compute @workgroup_size(16, 8)
fn input_transform(@builtin(global_invocation_id) id: vec3<u32>) {
    let t = id.x;
    let ic = id.y;
    if (ic >= p.c_in || t >= p.tiles) {
        return;
    }
    let tiles_x = (p.w_out + 3u) / 4u;
    let y0 = i32((t / tiles_x) * 4u) - 1;
    let x0 = i32((t % tiles_x) * 4u) - 1;
    let plane = p.h_in * p.w_in;
    let src = p.x_off + ic * plane;
    //@unroll r 0..6
    let yy{r} = y0 + {r};
    let in_y{r} = yy{r} >= 0 && yy{r} < i32(p.h_in);
    //@end
    //@unroll c 0..6
    let xx{c} = x0 + {c};
    let in_x{c} = xx{c} >= 0 && xx{c} < i32(p.w_in);
    //@end
    //@unroll r 0..6
    //@unroll c 0..6
    var d{r}{c} = 0.0;
    if (in_y{r} && in_x{c}) {
        d{r}{c} = x[src + u32(yy{r}) * p.w_in + u32(xx{c})];
    }
    //@end
    //@end
    // Bᵀ d（行方向）。
    //@unroll c 0..6
    let e0{c} = 4.0 * d0{c} - 5.0 * d2{c} + d4{c};
    let e1{c} = -4.0 * d1{c} - 4.0 * d2{c} + d3{c} + d4{c};
    let e2{c} = 4.0 * d1{c} - 4.0 * d2{c} - d3{c} + d4{c};
    let e3{c} = -2.0 * d1{c} - d2{c} + 2.0 * d3{c} + d4{c};
    let e4{c} = 2.0 * d1{c} - d2{c} - 2.0 * d3{c} + d4{c};
    let e5{c} = 4.0 * d1{c} - 5.0 * d3{c} + d5{c};
    //@end
    // (Bᵀ d) B（列方向）、U[b][ic][t] へ。
    let stride = p.tiles * p.c_in;
    let base = ic * p.tiles + t;
    //@unroll r 0..6
    u_out[({r}u * 6u + 0u) * stride + base] = S(4.0 * e{r}0 - 5.0 * e{r}2 + e{r}4);
    u_out[({r}u * 6u + 1u) * stride + base] = S(-4.0 * e{r}1 - 4.0 * e{r}2 + e{r}3 + e{r}4);
    u_out[({r}u * 6u + 2u) * stride + base] = S(4.0 * e{r}1 - 4.0 * e{r}2 - e{r}3 + e{r}4);
    u_out[({r}u * 6u + 3u) * stride + base] = S(-2.0 * e{r}1 - e{r}2 + 2.0 * e{r}3 + e{r}4);
    u_out[({r}u * 6u + 4u) * stride + base] = S(2.0 * e{r}1 - e{r}2 - 2.0 * e{r}3 + e{r}4);
    u_out[({r}u * 6u + 5u) * stride + base] = S(4.0 * e{r}1 - 5.0 * e{r}3 + e{r}5);
    //@end
}

// 行列積：ワークグループ 16×16 本が 64 行（タイル）× 64 列（出力）、スレッド一つが 4×4。
// K（入力）は 16 ずつ共有メモリに置く。id.z が 36 の位置。
var<workgroup> a_s: array<f32, 1024>;          // [16 k][64 行]
var<workgroup> b_s: array<vec4<f32>, 256>;     // [16 k][16 列の組（4 列ずつ）]

@compute @workgroup_size(16, 16)
fn gemm(@builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>) {
    // lid.x が行（タイル）の 4 の組、lid.y が列（出力）の 4 の組。
    let li = lid.y * 16u + lid.x;
    let row0 = wid.y * 64u;
    let col0 = wid.x * 64u;
    let b = wid.z;
    let k_all = p.c_in;
    let n_all = p.c_out;
    let a_base = b * k_all * p.tiles;
    let w_base = b * k_all * n_all;
    var acc0 = vec4<f32>(0.0);
    var acc1 = vec4<f32>(0.0);
    var acc2 = vec4<f32>(0.0);
    var acc3 = vec4<f32>(0.0);
    for (var k0 = 0u; k0 < k_all; k0 += 16u) {
        // A：16 k × 64 行（U は [k][タイル] なので、隣のスレッドが隣のタイルを読む）。
        //@unroll r 0..4
        {
            let e = li + {r}u * 256u;
            let row = e % 64u;
            let kk = e / 64u;
            var v = 0.0;
            if (row0 + row < p.tiles && k0 + kk < k_all) {
                v = f32(u_in[a_base + (k0 + kk) * p.tiles + row0 + row]);
            }
            a_s[kk * 64u + row] = v;
        }
        //@end
        // B：16 k × 64 列（スレッド一つが 4 列を vec4 で一つ）。
        {
            let kk = li / 16u;
            let cg = li % 16u;
            var v = vec4<f32>(0.0);
            if (k0 + kk < k_all) {
                let base = w_base + (k0 + kk) * n_all + col0 + cg * 4u;
                //@unroll e 0..4
                if (col0 + cg * 4u + {e}u < n_all) {
                    v[{e}] = wt[base + {e}u];
                }
                //@end
            }
            b_s[kk * 16u + cg] = v;
        }
        workgroupBarrier();
        //@unroll kk 0..16
        {
            let bv = b_s[{kk}u * 16u + lid.y];
            let ar = {kk}u * 64u + lid.x * 4u;
            acc0 += a_s[ar] * bv;
            acc1 += a_s[ar + 1u] * bv;
            acc2 += a_s[ar + 2u] * bv;
            acc3 += a_s[ar + 3u] * bv;
        }
        //@end
        workgroupBarrier();
    }
    // M[b][出力][タイル] へ（隣のスレッドは 4 行先：スレッドごとに続いた 4 行を書く）。
    let m_base = b * n_all * p.tiles;
    let col = col0 + lid.y * 4u;
    //@unroll e 0..4
    if (col + {e}u < n_all) {
        //@unroll i 0..4
        if (row0 + lid.x * 4u + {i}u < p.tiles) {
            m_out[m_base + (col + {e}u) * p.tiles + row0 + lid.x * 4u + {i}u] = S(acc{i}[{e}]);
        }
        //@end
    }
    //@end
}

// 出力変換：スレッド一つが一つのタイルの一つの出力。id.x がタイル、id.y が出力（隣のスレッドが
// 隣のタイル：M を続けて読み、出力の行を続けて書く）。
@compute @workgroup_size(16, 8)
fn output_transform(@builtin(global_invocation_id) id: vec3<u32>) {
    let t = id.x;
    let oc = id.y;
    if (oc >= p.c_out || t >= p.tiles) {
        return;
    }
    let stride = p.tiles * p.c_out;
    let base = oc * p.tiles + t;
    // Aᵀ m（行方向）：6×6 → 4×6。
    //@unroll c 0..6
    let m0{c} = f32(m_in[(0u * 6u + {c}u) * stride + base]);
    let m1{c} = f32(m_in[(1u * 6u + {c}u) * stride + base]);
    let m2{c} = f32(m_in[(2u * 6u + {c}u) * stride + base]);
    let m3{c} = f32(m_in[(3u * 6u + {c}u) * stride + base]);
    let m4{c} = f32(m_in[(4u * 6u + {c}u) * stride + base]);
    let m5{c} = f32(m_in[(5u * 6u + {c}u) * stride + base]);
    let e0{c} = m0{c} + m1{c} + m2{c} + m3{c} + m4{c};
    let e1{c} = m1{c} - m2{c} + 2.0 * (m3{c} - m4{c});
    let e2{c} = m1{c} + m2{c} + 4.0 * (m3{c} + m4{c});
    let e3{c} = m1{c} - m2{c} + 8.0 * (m3{c} - m4{c}) + m5{c};
    //@end
    var bias = 0.0;
    if (p.has_bias == 1u) {
        bias = wt[p.bias_at + oc];
    }
    var scale = 1.0;
    if ((p.op & 2u) != 0u) {
        scale = beta[p.c_b + oc];
    }
    let tiles_x = (p.w_out + 3u) / 4u;
    let y0 = (t / tiles_x) * 4u;
    let x0 = (t % tiles_x) * 4u;
    let oplane = p.h_out * p.w_out;
    //@unroll r 0..4
    if (y0 + {r}u < p.h_out) {
        let oy = y0 + {r}u;
        let y{r}0 = e{r}0 + e{r}1 + e{r}2 + e{r}3 + e{r}4;
        let y{r}1 = e{r}1 - e{r}2 + 2.0 * (e{r}3 - e{r}4);
        let y{r}2 = e{r}1 + e{r}2 + 4.0 * (e{r}3 + e{r}4);
        let y{r}3 = e{r}1 - e{r}2 + 8.0 * (e{r}3 - e{r}4) + e{r}5;
        //@unroll c 0..4
        if (x0 + {c}u < p.w_out) {
            let at = oc * oplane + oy * p.w_out + x0 + {c}u;
            var v = (y{r}{c} + bias) * scale;
            if ((p.op & 1u) != 0u) {
                v += res[p.y_off + at];
            }
            if (p.act == 2u && v < 0.0) {
                v = v * p.slope;
            }
            out[p.out_off + at] = v;
        }
        //@end
    }
    //@end
}

{{GEMM16}}
