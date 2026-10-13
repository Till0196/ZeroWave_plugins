// フレーム補間の見本：一つの入力から二枚出す。一枚目は前のフレームと今のフレームを
// 半分ずつ混ぜ、二枚目は今のフレーム。動きは追わないので、
// 仕組みを確かめるための物。
//
// 画面が出す枚数の倍で動く（59.94 の放送なら 119.88）。

//!OUTPUTS 2
//!HISTORY 1

//!PASS blend
//!READS picture picture:1
@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let now = textureSample(picture, picture_smp, uv);
    let before = textureSample(picture_1, picture_1_smp, uv);
    // phase は 0（一枚目）と 0.5（二枚目）。
    let t = 0.5 + zw.phase;
    return vec4<f32>(mix(before.rgb, now.rgb, t), 1.0);
}
