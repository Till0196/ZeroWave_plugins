// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 内蔵音を WAV に書き出す：`romsound 出力先/`。番号ごとに `romsound-N.wav`。

fn main() {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("使い方: romsound 出力先/");
        std::process::exit(2);
    };
    let rate = zerowave_databroadcast::romsound::AIFF_RATE;
    for number in 0..=13u32 {
        let samples = zerowave_databroadcast::romsound::synthesize(number, rate).expect("a sound");
        let mut out = Vec::new();
        let data = samples.len() as u32 * 2;
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data.to_le_bytes());
        for sample in samples {
            out.extend_from_slice(&((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        let path = format!("{dir}/romsound-{number}.wav");
        std::fs::write(&path, out).expect("書ける");
        println!("{path}");
    }
}
