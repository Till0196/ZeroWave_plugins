//!HISTORY 1
//!RETIME
//!TEXTURE flow input.w GRID1 + GRID / input.h GRID1 + GRID / rgba16float nearest keep
CONFIDENCE_TEXTURE
//!PASS flow
//!READS picture:1 picture
//!WRITES flow
//!HOST flow
CONFIDENCE_PASS
//!PASS warp
//!READS picture:1 picture flow CONFIDENCE_READ
// フローは「その画素が相手のフレームのどこへ動いたか」（前→今が xy、今→前が zw）。
// t の画素 p は、前では p − t·F(前→今)(p)、今では p − (1−t)·F(今→前)(p) に
// あったとみなす。行って戻ったフローの不一致が大きい側は隠れた所とみて
// 重みを下げ、実装が信頼できないとした側（信頼度）も下げる。両側とも
// 当てにならなければフローを使わずに混ぜる。
fn flow_at(p: vec2<f32>) -> vec4<f32> {
    let dims = vec2<i32>(textureDimensions(flow));
    let q = p / GRIDf - 0.5;
    let i = vec2<i32>(floor(q));
    let w = q - floor(q);
    let lo = clamp(i, vec2<i32>(0), dims - 1);
    let hi = clamp(i + 1, vec2<i32>(0), dims - 1);
    let v00 = textureLoad(flow, vec2<i32>(lo.x, lo.y), 0);
    let v10 = textureLoad(flow, vec2<i32>(hi.x, lo.y), 0);
    let v01 = textureLoad(flow, vec2<i32>(lo.x, hi.y), 0);
    let v11 = textureLoad(flow, vec2<i32>(hi.x, hi.y), 0);
    return mix(mix(v00, v10, w.x), mix(v01, v11, w.x), w.y);
}

CONFIDENCE_FN

@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(picture));
    let p = uv * size;
    let t = clamp(zw.phase, 0.0, 1.0);
    let f = flow_at(p);
    let fab = f.xy;
    let fba = f.zw;
    let c = confidence_at(p);
    let a = textureSampleLevel(picture_1, picture_1_smp, (p - t * fab) / size, 0.0);
    let b = textureSampleLevel(picture, picture_smp, (p - (1.0 - t) * fba) / size, 0.0);
    let ea = length(fab + flow_at(p + fab).zw);
    let eb = length(fba + flow_at(p + fba).xy);
    let sigma = 4.0;
    let wa = (1.0 - t) * exp(-ea * ea / sigma) * c.x;
    let wb = t * exp(-eb * eb / sigma) * c.y;
    let w0 = 0.02;
    let still = mix(
        textureSampleLevel(picture_1, picture_1_smp, uv, 0.0),
        textureSampleLevel(picture, picture_smp, uv, 0.0),
        t
    );
    let color = (a * wa + b * wb + still * w0) / (wa + wb + w0);
    return vec4<f32>(color.rgb, 1.0);
}
