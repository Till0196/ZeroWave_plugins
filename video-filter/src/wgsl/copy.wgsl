// 一つ読んだ画像をそのままコピーする（[`crate::build::Builder::simple`] の本文）。
    return vec4<f32>(textureSampleLevel(r0, r0_smp, uv, 0.0).rgb, 1.0);
