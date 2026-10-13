// 輝度を取り出す（mpv の LUMA のパスの最初）。
    let rgb = textureSampleLevel(r0, r0_smp, uv, 0.0).rgb;
    return vec4<f32>(dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0, 0.0, 1.0);
