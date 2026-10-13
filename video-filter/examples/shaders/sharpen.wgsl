// 輪郭を少し立てる（アンシャープマスク）。picture のフックに掛ける。
//
// 使い方：プラグインの設定の「掛けるファイル」に sharpen.wgsl。

//!PASS sharpen
//!READS picture
@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let px = 1.0 / zw.input_size;
    let c = textureSample(picture, picture_smp, uv).rgb;
    var around = vec3<f32>(0.0);
    around += textureSample(picture, picture_smp, uv + vec2<f32>(px.x, 0.0)).rgb;
    around += textureSample(picture, picture_smp, uv - vec2<f32>(px.x, 0.0)).rgb;
    around += textureSample(picture, picture_smp, uv + vec2<f32>(0.0, px.y)).rgb;
    around += textureSample(picture, picture_smp, uv - vec2<f32>(0.0, px.y)).rgb;
    let amount = 0.5;
    return vec4<f32>(clamp(c + (c - around * 0.25) * amount, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
