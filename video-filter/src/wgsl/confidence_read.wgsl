fn confidence_at(p: vec2<f32>) -> vec2<f32> {
    let dims = vec2<i32>(textureDimensions(confidence));
    let i = clamp(vec2<i32>(floor(p / GRIDf)), vec2<i32>(0), dims - 1);
    return textureLoad(confidence, i, 0).xy;
}
