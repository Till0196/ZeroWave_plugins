// 取り出したときの画像（r1）に、輝度（r0）の差を足して RGB に戻す。
    let rgb = textureSampleLevel(r1, r1_smp, uv, 0.0).rgb;
    let y = textureSampleLevel(r0, r0_smp, uv, 0.0).x;
    return vec4<f32>(clamp(rgb + vec3<f32>(y - dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722))), vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
