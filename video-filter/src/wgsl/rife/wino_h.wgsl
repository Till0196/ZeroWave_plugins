// 行列積を fp16 で演算する物（作業バッファが f16 のときだけ組み込む）。積は vec4<f16> で 4 つの k を
// まとめて計算し（Turing・Radeon・Arc は fp16 の積和が fp32 の 2 倍の速さ）、足し込みは 4 つ
// ごとに f32 へ戻して誤差が積もらないようにする。テンソルコアが使えないとき向け。
// 決まった回数のループは `//@unroll` で展開する（wgpu がループごとに足すカウンターを避ける）。
var<workgroup> a_h: array<f16, 1024>;          // [16 k][64 行]
var<workgroup> b_h: array<vec4<f16>, 256>;     // [16 k][16 列の組]

@compute @workgroup_size(16, 16)
fn gemm_h(@builtin(local_invocation_id) lid: vec3<u32>, @builtin(workgroup_id) wid: vec3<u32>) {
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
        //@unroll r 0..4
        {
            let e = li + {r}u * 256u;
            let row = e % 64u;
            let kk = e / 64u;
            var v = f16(0.0);
            if (row0 + row < p.tiles && k0 + kk < k_all) {
                v = u_in[a_base + (k0 + kk) * p.tiles + row0 + row];
            }
            a_h[kk * 64u + row] = v;
        }
        //@end
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
            b_h[kk * 16u + cg] = vec4<f16>(v);
        }
        workgroupBarrier();
        //@unroll kq 0..4
        {
            var p0 = vec4<f16>(0.0);
            var p1 = vec4<f16>(0.0);
            var p2 = vec4<f16>(0.0);
            var p3 = vec4<f16>(0.0);
            //@unroll kk 0..4
            {
                let bv = b_h[({kq}u * 4u + {kk}u) * 16u + lid.y];
                let ar = ({kq}u * 4u + {kk}u) * 64u + lid.x * 4u;
                p0 += a_h[ar] * bv;
                p1 += a_h[ar + 1u] * bv;
                p2 += a_h[ar + 2u] * bv;
                p3 += a_h[ar + 3u] * bv;
            }
            //@end
            acc0 += vec4<f32>(p0);
            acc1 += vec4<f32>(p1);
            acc2 += vec4<f32>(p2);
            acc3 += vec4<f32>(p3);
        }
        //@end
        workgroupBarrier();
    }
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
