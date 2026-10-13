// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! アニメーション画像（MNG、STD-B24 第二分冊 (2/2) 付属 4）のフレームの切り出しと間隔の読み。

pub const SIGNATURE: &[u8; 8] = b"\x8aMNG\r\n\x1a\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub delays_ms: Vec<u32>,
    /// 終わったら最初へ戻るか（`TERM` の 3）。
    pub repeats: bool,
}

pub fn is_mng(bytes: &[u8]) -> bool {
    bytes.starts_with(SIGNATURE)
}

fn chunks(bytes: &[u8]) -> Vec<(&[u8], &[u8])> {
    let mut out = Vec::new();
    let mut at = 8;
    while at + 8 <= bytes.len() {
        let length =
            u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
        let kind = &bytes[at + 4..at + 8];
        // wasm32 では usize が 32 ビットなので、足し算はあふれを確かめる。
        let Some(body) = (at + 8)
            .checked_add(length)
            .and_then(|end| bytes.get(at + 8..end))
        else {
            break;
        };
        out.push((kind, body));
        let Some(next) = (at + 12).checked_add(length) else {
            break;
        };
        at = next;
    }
    out
}

pub fn info(bytes: &[u8]) -> Option<Info> {
    if !is_mng(bytes) {
        return None;
    }
    let mut ticks_per_second = 1000u32;
    let mut repeats = false;
    let mut delay_ticks = 1u32;
    let mut delays = Vec::new();
    for (kind, body) in chunks(bytes) {
        match kind {
            b"MHDR" => {
                if body.len() >= 12 {
                    ticks_per_second =
                        u32::from_be_bytes([body[8], body[9], body[10], body[11]]).max(1);
                }
            }
            b"TERM" => repeats = body.first() == Some(&3),
            // FRAM: framing_mode(1) name… 0, change_delay(1), change_timeout(1),
            // change_clip(1), change_sync(1), delay(4) …
            b"FRAM" => {
                if body.len() >= 10 {
                    let name_end = body[1..].iter().position(|b| *b == 0).map_or(1, |p| p + 1);
                    let change_delay = body.get(name_end + 1).copied().unwrap_or(0);
                    if change_delay != 0
                        && let Some(delay) = body.get(name_end + 5..name_end + 9)
                    {
                        delay_ticks =
                            u32::from_be_bytes([delay[0], delay[1], delay[2], delay[3]]).max(1);
                    }
                }
            }
            b"IEND" => delays.push(delay_ticks * 1000 / ticks_per_second),
            _ => {}
        }
    }
    if delays.is_empty() {
        return None;
    }
    Some(Info {
        delays_ms: delays,
        repeats,
    })
}

pub fn frame(bytes: &[u8], index: usize) -> Option<Vec<u8>> {
    if !is_mng(bytes) {
        return None;
    }
    let mut out = Vec::new();
    let mut current = 0usize;
    let mut inside = false;
    for (kind, body) in chunks(bytes) {
        match kind {
            b"IHDR" => {
                if current == index {
                    inside = true;
                    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
                    push_chunk(&mut out, kind, body);
                }
            }
            b"IEND" => {
                if inside {
                    push_chunk(&mut out, kind, body);
                    return Some(out);
                }
                current += 1;
            }
            _ if inside => push_chunk(&mut out, kind, body),
            _ => {}
        }
    }
    None
}

pub(crate) fn push_chunk(out: &mut Vec<u8>, kind: &[u8], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    let mut crc = Crc::new();
    crc.update(kind);
    crc.update(body);
    out.extend_from_slice(&crc.finish().to_be_bytes());
}

struct Crc(u32);

impl Crc {
    fn new() -> Self {
        Self(0xffff_ffff)
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            let mut c = self.0 ^ u32::from(*byte);
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xedb8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            self.0 = c;
        }
    }

    fn finish(self) -> u32 {
        self.0 ^ 0xffff_ffff
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(kind: &[u8], body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        push_chunk(&mut out, kind, body);
        out
    }

    #[test]
    fn frames_and_delays_are_read() {
        let mut bytes = SIGNATURE.to_vec();
        let mut mhdr = vec![0; 28];
        mhdr[8..12].copy_from_slice(&10u32.to_be_bytes());
        bytes.extend(chunk(b"MHDR", &mhdr));
        bytes.extend(chunk(b"TERM", &[3, 0, 0, 0, 0, 0, 0x7f, 0xff, 0xff, 0xff]));
        bytes.extend(chunk(b"FRAM", &[3, 0, 2, 0, 0, 0, 0, 0, 0, 15]));
        for _ in 0..2 {
            bytes.extend(chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 3, 0, 0, 0]));
            bytes.extend(chunk(b"IDAT", &[0x78, 0x9c, 0x63, 0x60, 0, 0, 0, 2, 0, 1]));
            bytes.extend(chunk(b"IEND", &[]));
        }
        bytes.extend(chunk(b"MEND", &[]));
        let info = info(&bytes).expect("アニメーション画像");
        assert_eq!(info.delays_ms, [1500, 1500]);
        assert!(info.repeats);
        let png = frame(&bytes, 1).expect("フレーム");
        assert!(png.starts_with(b"\x89PNG"));
        assert!(frame(&bytes, 2).is_none());
        assert_eq!(&png[png.len() - 4..], &[0xae, 0x42, 0x60, 0x82]);
    }
}
