// 3×3（縁 1、ストライド 1）の畳み込みを、テンソルコア（cooperative matrix）で。
//
// カーネルの位置ごとに行列の積とみなす：C[画素, 出力] += A_tap[画素, 入力] × B_tap[入力, 出力]。
// ワークグループ（128 スレッド = 32 スレッドのサブグループが 4 つ）が 4 行 × 16 画素 × OCW（32 か 64）出力を受け持ち、サブグループ一つが
// 一行の 16 画素。入力チャンネル 16 本ごとに、入力の切れ端（6 行 × 18 列 × 16 本、
// チャンネルを一番内側）を f16 で共有メモリに置く。するとカーネルの位置 (ky, kx) での
// A（16 画素 × 16 本）は切れ端の中で連続した行列（行の幅 16）になり、並べ直さずに
// そのまま読める。カーネルの 9 か所 × 出力 2 組の積和を、置くたびに回す。
// B（重み）は f16 で [カーネルの位置][入力（16 の倍数）][出力（OCW の倍数）]。
//
// 先に試した「K（入力 × 9）を 16 ずつ進め、そのつど画素 × K に並べ直す」形は、
// 並べ直しの割り算とバリアが重く、テンソルコアの積和が二回しか入らず、共有メモリの
// 畳み込みより遅かった（RTX 2060）。
//
// サブグループが 32 スレッドで、サブグループの中のスレッドが続いた番号であること（NVIDIA）を前提にしている。

enable f16;
enable wgpu_cooperative_matrix;

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

@group(2) @binding(0) var<storage, read> x: array<f32>;
@group(2) @binding(2) var<storage, read_write> out: array<f32>;
@group(2) @binding(3) var<storage, read> wt: array<f32>;
@group(2) @binding(4) var<uniform> p: P;
@group(2) @binding(6) var<storage, read> wtc: array<f16>;

const ROWS: u32 = 4u;
const COLS: u32 = 16u;
const TR: u32 = 6u;    // ROWS + 2
const TC: u32 = 18u;   // COLS + 2
var<workgroup> tile: array<f16, 1728>;   // TR × TC × 16
var<workgroup> c_s: array<f32, {{CS}}>;    // 64 画素 × OCW 出力（行が画素）
const OCW: u32 = {{OCW}}u;   // ワークグループの出力の数（32 か 64）

@compute @workgroup_size(128)
fn conv3tc(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wid: vec3<u32>) {
    let oplane = p.h_out * p.w_out;
    let iplane = p.h_in * p.w_in;
    let x0 = wid.x * COLS;
    let y0 = wid.y * ROWS;
    let oc0 = wid.z * OCW;
    let sg = li / 32u;
    let cinp = (p.c_in + 15u) / 16u * 16u;
    let ocp = (p.c_out + OCW - 1u) / OCW * OCW;
    var acc0 = coop_mat16x16<f32, C>();
    var acc1 = coop_mat16x16<f32, C>();
    var acc2 = coop_mat16x16<f32, C>();
    var acc3 = coop_mat16x16<f32, C>();
    for (var ic0 = 0u; ic0 < cinp; ic0 += 16u) {
        // 切れ端を置く（TR × TC × 16 = 1728 個を 128 スレッドで、14 回）。隣のスレッドは隣の列を読む。
        for (var r = 0u; r < 14u; r++) {
            let e = li + r * 128u;
            if (e < 1728u) {
                let c = e / (TR * TC);
                let rest = e % (TR * TC);
                let ty = rest / TC;
                let tx = rest % TC;
                let iy = i32(y0 + ty) - 1;
                let ix = i32(x0 + tx) - 1;
                var v = 0.0;
                if (ic0 + c < p.c_in && iy >= 0 && iy < i32(p.h_in) && ix >= 0 && ix < i32(p.w_in)) {
                    v = x[p.x_off + (ic0 + c) * iplane + u32(iy) * p.w_in + u32(ix)];
                }
                tile[(ty * TC + tx) * 16u + c] = f16(v);
            }
        }
        workgroupBarrier();
        for (var tap = 0u; tap < 9u; tap++) {
            let ky = tap / 3u;
            let kx = tap % 3u;
            let a = coopLoadT<coop_mat16x16<f16, A>>(&tile[((sg + ky) * TC + kx) * 16u], 16u);
            let wb = (tap * cinp + ic0) * ocp + oc0;
            let b0 = coopLoadT<coop_mat16x16<f16, B>>(&wtc[wb], ocp);
            let b1 = coopLoadT<coop_mat16x16<f16, B>>(&wtc[wb + 16u], ocp);
            acc0 = coopMultiplyAdd(a, b0, acc0);
            acc1 = coopMultiplyAdd(a, b1, acc1);
            if (OCW == 64u) {
                let b2 = coopLoadT<coop_mat16x16<f16, B>>(&wtc[wb + 32u], ocp);
                let b3 = coopLoadT<coop_mat16x16<f16, B>>(&wtc[wb + 48u], ocp);
                acc2 = coopMultiplyAdd(a, b2, acc2);
                acc3 = coopMultiplyAdd(a, b3, acc3);
            }
        }
        workgroupBarrier();
    }
    coopStoreT(acc0, &c_s[sg * 16u * OCW], OCW);
    coopStoreT(acc1, &c_s[sg * 16u * OCW + 16u], OCW);
    if (OCW == 64u) {
        coopStoreT(acc2, &c_s[sg * 16u * OCW + 32u], OCW);
        coopStoreT(acc3, &c_s[sg * 16u * OCW + 48u], OCW);
    }
    workgroupBarrier();
    // 書き出し：64 画素 × OCW 出力を 128 スレッドで。隣のスレッドが隣の画素を書く。
    for (var r = 0u; r < OCW / 2u; r++) {
        let e = li + r * 128u;
        let o = e / 64u;
        let pr = e % 64u;
        let oc = oc0 + o;
        let oy = y0 + pr / 16u;
        let ox = x0 + pr % 16u;
        if (oc < p.c_out && oy < p.h_out && ox < p.w_out) {
            var v = c_s[pr * OCW + o];
            if (p.has_bias == 1u) {
                v += wt[p.bias_at + oc];
            }
            if (p.act == 2u && v < 0.0) {
                v = v * p.slope;
            }
            out[p.out_off + oc * oplane + oy * p.w_out + ox] = v;
        }
    }
}
