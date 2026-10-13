// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 受像機の内蔵音（`romsound://N`、TR-B14 第二分冊 第三編 第 2 部 3.3.5・表 3-9）。
//! 規格は番号しか決めていないので、全部を正弦波の合成で作り、本体へは AIFF-C で渡す。
//! 0〜4 速報チャイム、5〜12 ボタン操作音、13 アラート音、14・15 空き。

use std::f32::consts::PI;

/// 番号の音を、この速さの一チャンネルのサンプルで。持っていなければ `None`。
pub fn synthesize(number: u32, rate: u32) -> Option<Vec<f32>> {
    let rate = rate.clamp(8000, 192_000) as usize;
    Some(match number {
        0 => bell_strikes(rate),
        1 => jingle(rate),
        2 => rising_arpeggio(rate),
        3 => three_note_chime(rate),
        4 => low_bell(rate),
        5 => select(rate),
        6 => repeat(&select(rate), 3, seconds(rate, 0.13)),
        7 => decide(rate),
        8 => repeat(&decide(rate), 3, seconds(rate, 0.12)),
        9 => tick(rate),
        10 => repeat(&tick(rate), 3, seconds(rate, 0.10)),
        11 => startup_rising(rate),
        12 => startup_falling(rate),
        13 => buzzer(rate),
        _ => return None,
    })
}

pub fn aiff(number: u32) -> Option<Vec<u8>> {
    let samples = synthesize(number, AIFF_RATE)?;
    Some(encode_aiff(&samples, AIFF_RATE))
}

/// 一番高い倍音（5.8 kHz）が入る速さ。
pub const AIFF_RATE: u32 = 24_000;

fn seconds(rate: usize, s: f32) -> usize {
    (rate as f32 * s) as usize
}

fn sine(rate: usize, at: usize, hz: f32) -> f32 {
    (at as f32 * hz * 2.0 * PI / rate as f32).sin()
}

fn tone(
    rate: usize,
    len: usize,
    partials: &[(f32, f32)],
    envelope: impl Fn(f32) -> f32,
) -> Vec<f32> {
    (0..len)
        .map(|i| {
            let t = i as f32 / len as f32;
            partials
                .iter()
                .map(|(hz, gain)| sine(rate, i, *hz) * gain)
                .sum::<f32>()
                * envelope(t)
        })
        .collect()
}

fn decay(floor: f32) -> impl Fn(f32) -> f32 {
    move |t| floor.powf(t)
}

fn mix(into: &mut Vec<f32>, at: usize, sound: &[f32], gain: f32) {
    if into.len() < at + sound.len() {
        into.resize(at + sound.len(), 0.0);
    }
    for (i, sample) in sound.iter().enumerate() {
        into[at + i] += sample * gain;
    }
}

fn repeat(one: &[f32], times: usize, step: usize) -> Vec<f32> {
    let mut out = Vec::new();
    for n in 0..times {
        mix(&mut out, n * step, one, 1.0);
    }
    out
}

/// 一つの音を角を丸めて。`partials` は `(周波数, 大きさ)`。
fn smooth_note(
    rate: usize,
    attack: f32,
    hold: f32,
    tail: f32,
    partials: &[(f32, f32)],
    sway: f32,
) -> Vec<f32> {
    let attack_len = seconds(rate, attack).max(1);
    let hold_len = seconds(rate, hold);
    let tail_len = seconds(rate, tail);
    let tau = (tail_len as f32 / 3.0).max(1.0);
    let soften = seconds(rate, 0.012).max(1);
    (0..hold_len + tail_len)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let envelope = if i < attack_len {
                0.5 - 0.5 * (i as f32 / attack_len as f32 * PI).cos()
            } else if i < hold_len {
                1.0
            } else {
                let since = (i - hold_len) as f32;
                let knee = if since < soften as f32 {
                    0.5 + 0.5 * (since / soften as f32 * PI).cos()
                } else {
                    0.0
                };
                knee + (1.0 - knee) * (-since / tau).exp()
            };
            let mut sample = 0.0;
            for (n, (hz, gain)) in partials.iter().enumerate() {
                if *hz >= rate as f32 / 2.0 {
                    continue;
                }
                let phase = n as f32 * 1.7;
                let wobble = 1.0 + sway * (t * (4.0 + n as f32 * 1.3) * 2.0 * PI + phase).sin();
                sample += (t * hz * 2.0 * PI + phase).sin() * gain * wobble;
            }
            sample * envelope
        })
        .collect()
}

/// 速報チャイム 1：鐘。500 Hz と 550 Hz が唸り、高い部分音が乗る。一秒ごとに四度。
fn bell_strikes(rate: usize) -> Vec<f32> {
    let strike = tone(
        rate,
        seconds(rate, 0.95),
        &[
            (500.0, 0.55),
            (550.0, 0.45),
            (3700.0, 0.18),
            (5250.0, 0.15),
            (5800.0, 0.17),
        ],
        decay(0.08),
    );
    let mut out = Vec::new();
    for at in [0.14, 1.10, 2.18, 3.18] {
        mix(&mut out, seconds(rate, at), &strike, 0.22);
    }
    out.resize(seconds(rate, 4.35), 0.0);
    out
}

/// 速報チャイム 2：G5 → C6 → A6 → C7 と上がって下りる八音の節を三度。
fn jingle(rate: usize) -> Vec<f32> {
    let mut phrase = Vec::new();
    for (at, hz, hold, tail, gain) in [
        (0.00, 788.0, 0.22, 0.20, 0.075),
        (0.26, 1050.0, 0.10, 0.16, 0.08),
        (0.38, 1760.0, 0.14, 0.18, 0.085),
        (0.54, 2100.0, 0.18, 0.20, 0.09),
        (0.74, 1760.0, 0.12, 0.30, 0.07),
        (0.88, 1050.0, 0.10, 0.40, 0.06),
        (1.00, 788.0, 0.16, 0.75, 0.055),
    ] {
        let note = smooth_note(
            rate,
            0.012,
            hold,
            tail,
            &[(hz, gain), (hz * 2.0, gain * 0.15)],
            0.02,
        );
        mix(&mut phrase, seconds(rate, at), &note, 1.0);
    }
    let mut out = Vec::new();
    for start in [0.14, 1.68, 3.20] {
        mix(&mut out, seconds(rate, start), &phrase, 1.0);
    }
    out.resize(seconds(rate, 6.5), 0.0);
    out
}

/// 速報チャイム 3：D5・G5・B5・F6・A6 と駆け上がるレガートを二度ずつ。
fn rising_arpeggio(rate: usize) -> Vec<f32> {
    let steps = [
        (0.0, 589.0, 0.10),
        (0.10, 775.0, 0.04),
        (0.14, 988.0, 0.08),
        (0.22, 1383.0, 0.06),
    ];
    let mut run = Vec::new();
    for (at, hz, hold) in steps {
        let note = smooth_note(rate, 0.008, hold, 0.12, &[(hz, 0.1)], 0.0);
        mix(&mut run, seconds(rate, at), &note, 1.0);
    }
    let last = smooth_note(
        rate,
        0.008,
        0.04,
        0.16,
        &[(1756.0, 0.1), (3512.0, 0.006)],
        0.0,
    );
    mix(&mut run, seconds(rate, 0.28), &last, 1.0);
    let mut out = Vec::new();
    for start in [0.06, 0.58, 1.90, 2.40] {
        mix(&mut out, seconds(rate, start), &run, 1.0);
    }
    out.resize(seconds(rate, 3.27), 0.0);
    out
}

/// 速報チャイム 4：C6・G6・E7 を重ねて和音にし、二巡。三秒置いてもう一度。
fn three_note_chime(rate: usize) -> Vec<f32> {
    let notes = [
        (0.0, 1044.0),
        (0.19, 1567.0),
        (0.38, 2635.0),
        (0.60, 1044.0),
        (0.79, 1567.0),
    ];
    let mut run = Vec::new();
    for (at, hz) in notes {
        let note = smooth_note(
            rate,
            0.01,
            0.17,
            0.30,
            &[(hz, 0.06), (hz * 3.0, 0.004)],
            0.02,
        );
        mix(&mut run, seconds(rate, at), &note, 1.0);
    }
    let last = smooth_note(
        rate,
        0.01,
        0.20,
        0.55,
        &[(2635.0, 0.06), (1567.0, 0.02), (1044.0, 0.008)],
        0.02,
    );
    mix(&mut run, seconds(rate, 0.98), &last, 1.0);
    let mut out = Vec::new();
    for start in [0.10, 2.98] {
        mix(&mut out, seconds(rate, start), &run, 1.0);
    }
    out.resize(seconds(rate, 5.44), 0.0);
    out
}

/// 速報チャイム 5：低い鐘。芯が先に消え、高いきらめきが余韻に残る。四度打つ。
fn low_bell(rate: usize) -> Vec<f32> {
    let body = smooth_note(
        rate,
        0.004,
        0.02,
        0.45,
        &[(150.0, 0.056), (593.0, 0.043)],
        0.0,
    );
    let sparkle = smooth_note(
        rate,
        0.01,
        0.05,
        0.95,
        &[(1330.0, 0.014), (1777.0, 0.02), (2960.0, 0.025)],
        0.2,
    );
    let mut out = Vec::new();
    for start in [0.16, 0.98, 1.82, 2.66] {
        mix(&mut out, seconds(rate, start), &body, 1.7);
        mix(&mut out, seconds(rate, start + 0.04), &sparkle, 1.8);
    }
    out.resize(seconds(rate, 4.35), 0.0);
    out
}

/// 選択音（ボタン 1）：500 Hz に 3.65・4.65 kHz を重ね、0.14 秒で減る。
fn select(rate: usize) -> Vec<f32> {
    tone(
        rate,
        seconds(rate, 0.14),
        &[(500.0, 0.55), (3650.0, 0.25), (4650.0, 0.33)],
        decay(0.12),
    )
    .into_iter()
    .map(|s| s * 0.2)
    .collect()
}

/// 決定音（ボタン 3）：2.1 kHz を 40 ms、続けて 1.4 kHz を 90 ms。位相は繋ぐ（切ると擦れる）。
fn decide(rate: usize) -> Vec<f32> {
    let head_len = seconds(rate, 0.04);
    let body_len = seconds(rate, 0.09);
    let tail_len = seconds(rate, 0.08);
    let total = head_len + body_len + tail_len;
    let attack = seconds(rate, 0.005).max(1);
    let tau = rate as f32 * 0.02;
    let mut phase = 0.0f32;
    (0..total)
        .map(|i| {
            let hz = if i < head_len { 2100.0 } else { 1400.0 };
            phase = (phase + hz * 2.0 * PI / rate as f32) % (2.0 * PI);
            let mut wave = phase.sin();
            if hz * 3.0 < 6000.0 {
                wave += (phase * 3.0).sin() * 0.35;
            }
            let envelope = if i < attack {
                i as f32 / attack as f32
            } else if i < head_len + body_len {
                1.0
            } else {
                (-((i - head_len - body_len) as f32) / tau).exp()
            };
            wave * envelope * 0.145
        })
        .collect()
}

/// 短い音（ボタン 5）：1.05 kHz の矩形波めいた音（三倍音・五倍音）、0.08 秒。
fn tick(rate: usize) -> Vec<f32> {
    tone(
        rate,
        seconds(rate, 0.08),
        &[(1050.0, 0.7), (3150.0, 0.22), (5200.0, 0.13)],
        |t| if t < 0.5 { 1.0 } else { 1.0 - (t - 0.5) * 2.0 },
    )
    .into_iter()
    .map(|s| s * 0.2)
    .collect()
}

/// 起動音（ボタン 7・8）：520 Hz の倍音列の四音が上がるか下がる。
/// `notes` は `(始まり秒, 平らな長さ秒, 余韻秒, 倍音の大きさ)`。
fn arpeggio(rate: usize, notes: &[(f32, f32, f32, [f32; 11])]) -> Vec<f32> {
    let f0 = 520.0;
    let mut out = Vec::new();
    for (n, &(start, hold, tail, table)) in notes.iter().enumerate() {
        let hold_len = seconds(rate, hold);
        let tail_len = seconds(rate, tail);
        let tau = tail_len as f32 / 3.0;
        let attack = seconds(rate, 0.015).max(1);
        let soften = seconds(rate, 0.012).max(1);
        let loudest = table
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map_or(0, |(k, _)| k);
        let note: Vec<f32> = (0..hold_len + tail_len)
            .map(|i| {
                let mut sample = 0.0;
                for (k, gain) in table.iter().enumerate() {
                    let hz = f0 * (k + 1) as f32;
                    if hz >= 6000.0 || *gain == 0.0 {
                        continue;
                    }
                    let phase = (k * 7 + n * 3) as f32 * 0.61;
                    let detune = 1.0 + 0.0015 * ((k % 3) as f32 - 1.0);
                    let t = i as f32 / rate as f32;
                    let wave = (t * hz * detune * 2.0 * PI + phase).sin();
                    let envelope = if i < attack {
                        let x = i as f32 / attack as f32;
                        0.5 - 0.5 * (x * PI).cos()
                    } else if i < hold_len {
                        1.0
                    } else {
                        let faster = 1.0 + 0.4 * k.saturating_sub(loudest) as f32;
                        let since = (i - hold_len) as f32;
                        let knee = if since < soften as f32 {
                            0.5 + 0.5 * (since / soften as f32 * PI).cos()
                        } else {
                            0.0
                        };
                        let decay = (-since * faster / tau).exp();
                        knee + (1.0 - knee) * decay
                    };
                    let sway = 1.0 + 0.06 * (t * (5.0 + k as f32 * 1.7) * 2.0 * PI + phase).sin();
                    let bright = if k > loudest {
                        1.0 + 1.5 * (-t / 0.015).exp()
                    } else {
                        1.0
                    };
                    sample += wave * gain * envelope * sway * bright;
                }
                sample
            })
            .collect();
        mix(&mut out, seconds(rate, start), &note, 1.0);
    }
    out.resize(seconds(rate, 0.43), 0.0);
    out
}

fn startup_rising(rate: usize) -> Vec<f32> {
    arpeggio(
        rate,
        &[
            (
                0.02,
                0.09,
                0.05,
                [
                    0.047, 0.015, 0.011, 0.004, 0.011, 0.002, 0.006, 0.005, 0.003, 0.004, 0.003,
                ],
            ),
            (
                0.115,
                0.04,
                0.03,
                [
                    0.020, 0.025, 0.004, 0.017, 0.006, 0.011, 0.006, 0.005, 0.003, 0.008, 0.001,
                ],
            ),
            (
                0.15,
                0.045,
                0.03,
                [
                    0.009, 0.009, 0.053, 0.003, 0.004, 0.014, 0.000, 0.011, 0.008, 0.013, 0.000,
                ],
            ),
            (
                0.21,
                0.09,
                0.14,
                [
                    0.004, 0.009, 0.011, 0.035, 0.001, 0.012, 0.000, 0.025, 0.005, 0.000, 0.000,
                ],
            ),
        ],
    )
}

fn startup_falling(rate: usize) -> Vec<f32> {
    arpeggio(
        rate,
        &[
            (
                0.03,
                0.035,
                0.03,
                [
                    0.000, 0.000, 0.000, 0.037, 0.000, 0.000, 0.000, 0.024, 0.000, 0.000, 0.000,
                ],
            ),
            (
                0.08,
                0.045,
                0.03,
                [
                    0.000, 0.000, 0.058, 0.030, 0.000, 0.008, 0.000, 0.018, 0.018, 0.000, 0.000,
                ],
            ),
            (
                0.14,
                0.05,
                0.03,
                [
                    0.000, 0.025, 0.022, 0.015, 0.000, 0.025, 0.000, 0.009, 0.003, 0.010, 0.000,
                ],
            ),
            (
                0.21,
                0.09,
                0.14,
                [
                    0.029, 0.012, 0.014, 0.015, 0.003, 0.004, 0.005, 0.005, 0.004, 0.005, 0.002,
                ],
            ),
        ],
    )
}

/// アラート音：88 Hz のブザー。倍音が 1.3 kHz まで並ぶので「ブー」と鳴る。
fn buzzer(rate: usize) -> Vec<f32> {
    let mut partials = vec![
        (88.0, 0.10),
        (176.0, 0.05),
        (264.0, 0.06),
        (352.0, 0.03),
        (440.0, 0.03),
    ];
    for k in 6..=15u32 {
        partials.push((88.0 * k as f32, 0.16 / k as f32));
    }
    smooth_note(rate, 0.02, 0.49, 0.06, &partials, 0.0)
        .into_iter()
        .enumerate()
        .map(|(i, s)| s * (0.75 + 0.5 * i as f32 / rate as f32))
        .collect()
}

fn encode_aiff(samples: &[f32], rate: u32) -> Vec<u8> {
    let frames = samples.len() as u32;
    let mut out = Vec::with_capacity(54 + samples.len() * 2);
    out.extend_from_slice(b"FORM");
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(b"AIFC");
    out.extend_from_slice(b"FVER");
    out.extend_from_slice(&4u32.to_be_bytes());
    out.extend_from_slice(&0xa280_5140u32.to_be_bytes());
    // COMM：一チャンネル、枚数、16 ビット、速さ（80 ビット浮動小数）、NONE。
    out.extend_from_slice(b"COMM");
    out.extend_from_slice(&22u32.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&frames.to_be_bytes());
    out.extend_from_slice(&16u16.to_be_bytes());
    out.extend_from_slice(&extended(rate));
    out.extend_from_slice(b"NONE");
    out.extend_from_slice(b"SSND");
    out.extend_from_slice(&(8 + frames * 2).to_be_bytes());
    out.extend_from_slice(&[0; 8]);
    for sample in samples {
        out.extend_from_slice(&((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_be_bytes());
    }
    if out.len() % 2 == 1 {
        out.push(0);
    }
    let size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&size.to_be_bytes());
    out
}

/// 整数を IEEE 754 の 80 ビット拡張浮動小数に（AIFF のサンプリング周波数）。
fn extended(value: u32) -> [u8; 10] {
    let mut out = [0u8; 10];
    if value == 0 {
        return out;
    }
    let shift = 31 - value.leading_zeros();
    let exponent = 16383 + shift as u16;
    let mantissa = u64::from(value) << (63 - shift);
    out[..2].copy_from_slice(&exponent.to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_numbered_sound_is_there_and_bounded() {
        for number in 0..=13 {
            let samples = synthesize(number, 24_000).expect("a sound");
            assert!(!samples.is_empty(), "{number}");
            assert!(samples.len() <= 24_000 * 7, "{number} は七秒に収まる");
            assert!(samples.iter().all(|s| s.abs() <= 1.0), "{number} が割れる");
            assert!(samples.iter().any(|s| s.abs() > 0.02), "{number} が無音");
        }
        assert!(synthesize(14, 24_000).is_none());
        assert!(synthesize(20, 24_000).is_none());
    }

    #[test]
    fn the_lengths_are_as_written() {
        let rate = 12_000;
        assert_eq!(synthesize(5, rate).unwrap().len(), 1680);
        assert_eq!(synthesize(7, rate).unwrap().len(), 2520);
        assert_eq!(synthesize(0, rate).unwrap().len(), 52_200);
        assert_eq!(synthesize(13, rate).unwrap().len(), 6600);
    }

    #[test]
    fn the_aiff_is_well_formed() {
        let bytes = aiff(9).expect("a sound");
        assert_eq!(&bytes[..4], b"FORM");
        assert_eq!(&bytes[8..12], b"AIFC");
        let size = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        assert_eq!(size + 8, bytes.len());
        assert!(bytes.windows(4).any(|w| w == b"NONE"));
        assert_eq!(extended(24_000), [0x40, 0x0d, 0xbb, 0x80, 0, 0, 0, 0, 0, 0]);
        assert_eq!(extended(48_000), [0x40, 0x0e, 0xbb, 0x80, 0, 0, 0, 0, 0, 0]);
        let sound = mediadecoder::audio::aiff::decode(&bytes).expect("本体が読める");
        assert_eq!(sound.rate, AIFF_RATE);
        assert_eq!(sound.channels.len(), 1);
        assert_eq!(
            sound.channels[0].len(),
            synthesize(9, AIFF_RATE).unwrap().len()
        );
    }
}
