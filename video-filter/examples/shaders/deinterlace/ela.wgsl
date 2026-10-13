// デインタレースの見本：ELA（edge-line average）。見せるフィールドの行は
// そのまま、反対の行は上下の行の三方向（左斜め・縦・右斜め）のうち、いちばん
// 差の小さい向きで平均する。前後のフレームは読まない。
//
// 使い方：shaders のフォルダーの deinterlace/ に置き、プリセットの「デインタレース」で選ぶ。

//!FRAMES 0 0

//!PASS ela
fn luma_at(p: vec2<i32>) -> f32 {
    return zw_luma(0, p);
}

fn ela(p: vec2<i32>) -> f32 {
    var best = 1e9;
    var value = 0.0;
    for (var d = -1; d <= 1; d++) {
        let a = luma_at(p + vec2<i32>(d, -1));
        let b = luma_at(p + vec2<i32>(-d, 1));
        let diff = abs(a - b);
        if diff < best {
            best = diff;
            value = (a + b) * 0.5;
        }
    }
    return value;
}

@fragment
fn main(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(at.xy);
    let own = (u32(p.y) & 1u) == zw.first_row;
    let y = select(ela(p), luma_at(p), own);
    let c = p / 2;
    // 色差はその位置の値（見本なので、色差は補間しない）。
    let cb = zw_cb(0, c);
    let cr = zw_cr(0, c);
    return vec4<f32>(zw_rgb(y, cb, cr), 1.0);
}
