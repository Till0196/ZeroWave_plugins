// Winograd の行列積をテンソルコア（cooperative matrix、f16 → f32）で。U と M は
// [36][チャンネル][タイル] に並ぶので、転置した積として計算する：
//   Mᵀ[b]（出力 × タイル）= Wᵀ[b]（出力 × 入力）× Uᵀ[b]（入力 × タイル）
// 大きい U は行の向きのまま読め（coopLoadT、行の幅 = タイルの数）、列の向きで読むのは小さくて
// 使い回しの効く重み W（[入力][出力]）だけ。結果の向きも M[出力][タイル] にそのまま合う。
// U を列の向きで読む（転置しない coopLoad）形は、NVIDIA で倍ほど遅かった（RTX 2060）。
// ワークグループ（128 スレッド = 32 スレッドのサブグループ 4 つ）が 64 出力 × 64 タイルずつ、サブグループ一つが 64 出力 ×
// 16 タイル（16×16 の積和 4 組）。結果は共有メモリを通して f16 に直し、隣のスレッドが隣の
// タイルを書く。入出力の数は 16 の倍数のときだけ使う。U の作業バッファは、
// 最後の 64 タイルの組で読み過ぎる分をパディングして作ってある。

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
    tiles: u32,
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

@group(2) @binding(4) var<uniform> p: P;
@group(2) @binding(5) var<storage, read> u_in: array<f16>;
@group(2) @binding(8) var<storage, read_write> m_out: array<f16>;
@group(2) @binding(10) var<storage, read> w16: array<f16>;

var<workgroup> c_s: array<f32, 4096>;   // 64 出力 × 64 タイル（行が出力）

// サブグループの分け方はタイル：サブグループ s が 64 出力 × 16 タイル（タイル 16·s から）。大きい U の切れ端は
// ワークグループの中で一度ずつしか読まず、小さい重み（Wᵀ の 4 片）を全員で使い回す。
// サブグループを出力で分けると、4 つのサブグループが同じ U を別々に読み、64ch の層で倍近く遅かった（RTX 2060）。
@compute @workgroup_size(128)
fn gemm_tc(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wid: vec3<u32>) {
    let sg = li / 32u;
    let tile0 = wid.x * 64u;
    let oc0 = wid.y * 64u;
    let b = wid.z;
    let k_all = p.c_in;
    let n_all = p.c_out;
    let u_base = b * k_all * p.tiles + tile0 + sg * 16u;
    let w_base = b * k_all * n_all + oc0;
    var acc0 = coop_mat16x16<f32, C>();
    var acc1 = coop_mat16x16<f32, C>();
    var acc2 = coop_mat16x16<f32, C>();
    var acc3 = coop_mat16x16<f32, C>();
    // 出力が 64 に満たない組でも、テンソルコアの命令は一様な所でしか呼べないので計算はさせ
    // （重みのバッファは読み過ぎる分をパディングしてある）、書き出しで範囲を選ぶ。
    for (var k0 = 0u; k0 < k_all; k0 += 16u) {
        let u = coopLoadT<coop_mat16x16<f16, B>>(&u_in[u_base + k0 * p.tiles], p.tiles);
        let wb = w_base + k0 * n_all;
        acc0 = coopMultiplyAdd(coopLoad<coop_mat16x16<f16, A>>(&w16[wb], n_all), u, acc0);
        acc1 = coopMultiplyAdd(coopLoad<coop_mat16x16<f16, A>>(&w16[wb + 16u], n_all), u, acc1);
        acc2 = coopMultiplyAdd(coopLoad<coop_mat16x16<f16, A>>(&w16[wb + 32u], n_all), u, acc2);
        acc3 = coopMultiplyAdd(coopLoad<coop_mat16x16<f16, A>>(&w16[wb + 48u], n_all), u, acc3);
    }
    // c_s[出力][タイル]：サブグループ s はタイルの列 16·s から。
    coopStoreT(acc0, &c_s[0u * 16u * 64u + sg * 16u], 64u);
    coopStoreT(acc1, &c_s[1u * 16u * 64u + sg * 16u], 64u);
    coopStoreT(acc2, &c_s[2u * 16u * 64u + sg * 16u], 64u);
    coopStoreT(acc3, &c_s[3u * 16u * 64u + sg * 16u], 64u);
    workgroupBarrier();
    let m_base = b * n_all * p.tiles;
    for (var r = 0u; r < 32u; r++) {
        let e = li + r * 128u;
        let row = e / 64u;    // 出力
        let col = e % 64u;    // タイル
        if (oc0 + row < n_all && tile0 + col < p.tiles) {
            m_out[m_base + (oc0 + row) * p.tiles + tile0 + col] = f16(c_s[e]);
        }
    }
}

// 出力が 64 の倍数でない層（96 など）向け：サブグループを出力で分ける（サブグループ s が 16 出力 × 64 タイル）。
// タイルで分けると最後の 64 出力の組で半分を無駄に計算し、96ch の層で 3.0 → 4.0 ms に
// 遅くなった（RTX 2060）。こちらは受け持ちの無いサブグループだけが無駄になる。
@compute @workgroup_size(128)
fn gemm_tc_oc(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wid: vec3<u32>) {
    let sg = li / 32u;
    let tile0 = wid.x * 64u;
    let oc0 = wid.y * 64u;
    let b = wid.z;
    let k_all = p.c_in;
    let n_all = p.c_out;
    let u_base = b * k_all * p.tiles + tile0;
    let w_base = b * k_all * n_all + oc0 + sg * 16u;
    var acc0 = coop_mat16x16<f32, C>();
    var acc1 = coop_mat16x16<f32, C>();
    var acc2 = coop_mat16x16<f32, C>();
    var acc3 = coop_mat16x16<f32, C>();
    for (var k0 = 0u; k0 < k_all; k0 += 16u) {
        let a = coopLoad<coop_mat16x16<f16, A>>(&w16[w_base + k0 * n_all], n_all);
        let ub = u_base + k0 * p.tiles;
        acc0 = coopMultiplyAdd(a, coopLoadT<coop_mat16x16<f16, B>>(&u_in[ub], p.tiles), acc0);
        acc1 = coopMultiplyAdd(a, coopLoadT<coop_mat16x16<f16, B>>(&u_in[ub + 16u], p.tiles), acc1);
        acc2 = coopMultiplyAdd(a, coopLoadT<coop_mat16x16<f16, B>>(&u_in[ub + 32u], p.tiles), acc2);
        acc3 = coopMultiplyAdd(a, coopLoadT<coop_mat16x16<f16, B>>(&u_in[ub + 48u], p.tiles), acc3);
    }
    coopStoreT(acc0, &c_s[sg * 16u * 64u], 64u);
    coopStoreT(acc1, &c_s[sg * 16u * 64u + 16u], 64u);
    coopStoreT(acc2, &c_s[sg * 16u * 64u + 32u], 64u);
    coopStoreT(acc3, &c_s[sg * 16u * 64u + 48u], 64u);
    workgroupBarrier();
    let m_base = b * n_all * p.tiles;
    for (var r = 0u; r < 32u; r++) {
        let e = li + r * 128u;
        let row = e / 64u;
        let col = e % 64u;
        if (oc0 + row < n_all && tile0 + col < p.tiles) {
            m_out[m_base + (oc0 + row) * p.tiles + tile0 + col] = f16(c_s[e]);
        }
    }
}
