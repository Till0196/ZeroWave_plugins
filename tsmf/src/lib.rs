// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! ケーブルの多重化フレーム（TSMF）を分離するプラグイン。指定されたサービスのストリームだけを
//! 取り出し、多重化フレームでないストリームはそのまま通す。多重化フレームの読み方は `isdb::tsmf`。
//! 複数の搬送波に分かれた多重化フレームは、`multiplex.carriers` の本数だけ本体からチューナーを確保して流し込む。

pub mod multiplex;

pub use multiplex::{Carriage, Group, Multiplex, Reception, Relative, SLOTS, Survey, Take, Tuner};

use zwsdk::plug::host::tuner::Tuner as Borrowed;
use zwsdk::plug::{Buf, Container, Stream, StreamInfo};

#[derive(Debug)]
pub struct Tsmf {
    multiplex: Multiplex,
    borrowed: Vec<Borrowed>,
    said_multiplexed: bool,
    said_lapsed: bool,
    said_collision: bool,
}

/// 多重化フレームを分離するか（`separate`、既定は真）。選局のたびに読むので設定ウィンドウで切ればその場で効く。
fn separate() -> bool {
    zwsdk::plug::host::cfg("separate").is_none_or(|value| zwsdk::abi::truthy(&value))
}

fn trace() -> bool {
    zwsdk::plug::host::on("trace")
}

impl Stream for Tsmf {
    fn new() -> Self {
        Self {
            multiplex: Multiplex::new(Take::Whole),
            borrowed: Vec::new(),
            said_multiplexed: false,
            said_lapsed: false,
            said_collision: false,
        }
    }

    /// TS で指定されたサービスがあれば、それを含むストリームを取る。TLV はそのまま。
    fn start(&mut self, info: StreamInfo) -> Result<(), String> {
        self.borrowed.clear();
        self.said_multiplexed = false;
        self.said_lapsed = false;
        self.said_collision = false;
        // チャンネルスキャンの予約値のときはチューナーを確保しない。
        let probing = info.service.and_then(zwsdk::abi::part_of).is_some();
        if separate()
            && !probing
            && let Some(words) = zwsdk::plug::host::tuner::words()
            && let Some(slot) = slot_of(&words)
        {
            let carriers = carriers_of(&words);
            for frequency in carriers.iter().skip(1) {
                match Borrowed::open(&words_for(&words, frequency)) {
                    Some(tuner) => self.borrowed.push(tuner),
                    None => {
                        zwsdk::plug::warn(&format!(
                            "搬送波 {frequency} のチューナーを確保できない。複数の搬送波に分かれた多重化フレームは全部の搬送波が要るので、この局は映らない"
                        ));
                        break;
                    }
                }
            }
            let take = Take::Relative(slot.0, slot.1);
            if trace() || carriers.len() > 1 {
                zwsdk::plug::info(&format!(
                    "{take:?} を取る（搬送波 {} 本{}）",
                    1 + self.borrowed.len(),
                    if carriers.len() > 1 {
                        "、確保して結合"
                    } else {
                        ""
                    }
                ));
            }
            self.multiplex = Multiplex::over(take, 1 + self.borrowed.len());
            return Ok(());
        }
        let take = match (info.container, info.service) {
            // 止めるのは、多重化フレームを読めない放送に当たったときの回避策。
            _ if !separate() => Take::Whole,
            // チャンネルスキャンが相対ストリームを一つずつ読むための予約値（`zwsdk::abi::PART`）。
            (Container::Ts, Some(service)) => match zwsdk::abi::part_of(service) {
                Some(relative) => Take::Relative(relative, Carriage::Transport),
                None => Take::Service(service),
            },
            (Container::Tlv, Some(service)) => match zwsdk::abi::part_of(service) {
                Some(relative) => Take::Relative(relative, Carriage::Tlv),
                None => Take::Whole,
            },
            _ => Take::Whole,
        };
        if trace() {
            zwsdk::plug::info(&format!("{:?} を取る（{:?}）", take, info.container));
        }
        self.multiplex = Multiplex::new(take);
        Ok(())
    }

    fn stop(&mut self) {
        self.borrowed.clear();
    }

    fn input(&mut self, buf: Buf) {
        if matches!(self.multiplex.taking(), Take::Whole) {
            zwsdk::plug::output_buf(&buf);
            return;
        }
        let mut taken = self.multiplex.feed(0, &buf.to_vec());
        for (n, tuner) in self.borrowed.iter_mut().enumerate() {
            if let Some(bytes) = tuner.read()
                && !bytes.is_empty()
            {
                taken.extend(self.multiplex.feed(n + 1, &bytes));
            }
        }
        if let Some(pid) = self.multiplex.collision()
            && !self.said_collision
        {
            self.said_collision = true;
            zwsdk::plug::warn(&format!(
                "多重化フレームの二本のストリームが同じ PID {pid:#06x} を使っていて、null を落とされたストリームでは分けられない。\
                 Mirakurun の側で tsmfRelTs で分けてください"
            ));
        }
        if self.multiplex.lapsed() && !self.said_lapsed {
            self.said_lapsed = true;
            zwsdk::plug::info("多重化フレームが途切れた。一本のストリームとしてそのまま通す");
        }
        if self.multiplex.single() && !self.said_multiplexed {
            self.said_multiplexed = true;
            zwsdk::plug::info(
                "多重化フレームにストリームが 1 本だけなので、分離せずにそのまま通す",
            );
        }
        if self.multiplex.multiplexed() && !self.said_multiplexed {
            self.said_multiplexed = true;
            zwsdk::plug::info(&format!(
                "多重化フレーム: {} 本のストリーム、{:?} を取る",
                self.multiplex.streams().len(),
                self.multiplex.taking()
            ));
        }
        zwsdk::plug::output(&taken);
    }
}

fn word<'a>(words: &'a str, name: &str) -> Option<&'a str> {
    words
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

fn slot_of(words: &str) -> Option<(u8, Carriage)> {
    let (kind, number) = word(words, zwsdk::abi::word::PART)?.split_once(':')?;
    let carriage = match kind.trim() {
        "ts" => Carriage::Transport,
        "tlv" => Carriage::Tlv,
        _ => return None,
    };
    Some((number.trim().parse().ok()?, carriage))
}

fn carriers_of(words: &str) -> Vec<String> {
    let own = word(words, "FREQUENCY").unwrap_or_default().to_owned();
    let mut list: Vec<String> = word(words, zwsdk::abi::word::CARRIERS)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|one| !one.is_empty() && *one != own)
        .map(str::to_owned)
        .collect();
    list.insert(0, own);
    list
}

/// 別の搬送波のチューナーを確保するための指定。周波数だけ替え、`tuner` の指定は外す（同時に二本は受けられない）。
fn words_for(words: &str, frequency: &str) -> String {
    words
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            match key {
                "tuner" => None,
                "FREQUENCY" => Some(format!("FREQUENCY={frequency}")),
                _ => Some(format!("{key}={value}")),
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

zwsdk::export_stream!(
    Tsmf,
    name = "多重フレーム（TSMF）",
    version = "0.1.0",
    order = 0,
    hooks = "[\"stream\",\"tuners\"]",
    config = r#"[
        {"key":"separate","kind":"bool","default":"true",
         "label":"TSMF の多重を分離する"},
        {"key":"trace","kind":"bool","default":"false",
         "label":"TSMF ヘッダの中身をログに出す（既定はオフ）","section":"ログ"}
    ]"#
);

#[cfg(test)]
mod tests {
    use super::*;

    fn null_packets(count: usize) -> Vec<u8> {
        let mut packet = [0u8; 188];
        packet[0] = 0x47;
        packet[1] = 0x1f;
        packet[2] = 0xff;
        packet[3] = 0x10;
        packet.iter().copied().cycle().take(188 * count).collect()
    }

    #[test]
    fn the_tune_words_say_the_slot_and_the_carriers() {
        let words = "DELIVERY_SYSTEM=ISDBC&FREQUENCY=93000000&tuner=dvb:adapter4*&multiplex.part=tlv:1&multiplex.carriers=93000000,99000000,105000000";
        assert_eq!(slot_of(words), Some((1, Carriage::Tlv)));
        assert_eq!(
            slot_of("multiplex.part=ts:2"),
            Some((2, Carriage::Transport))
        );
        assert_eq!(slot_of("multiplex.part=x:2"), None);
        assert_eq!(slot_of("FREQUENCY=1"), None);
        assert_eq!(
            carriers_of(words),
            ["93000000", "99000000", "105000000"],
            "自分が一本目"
        );
        assert_eq!(
            carriers_of("FREQUENCY=99000000&multiplex.carriers=93000000,105000000"),
            ["99000000", "93000000", "105000000"],
            "一覧に無ければ先頭に足す"
        );
        assert_eq!(
            carriers_of("FREQUENCY=93000000"),
            ["93000000"],
            "分かれていない"
        );
        assert_eq!(
            words_for(words, "99000000"),
            "DELIVERY_SYSTEM=ISDBC&FREQUENCY=99000000&multiplex.part=tlv:1&multiplex.carriers=93000000,99000000,105000000",
            "違うのは周波数だけ、チューナーの指定は外す"
        );
    }

    #[test]
    fn a_stream_not_taken_apart_passes_through_untouched() {
        let mut tsmf = Tsmf::new();
        tsmf.start(StreamInfo {
            container: Container::Tlv,
            service: Some(1024),
        })
        .expect("starts");
        let bytes = null_packets(4);
        tsmf.input(Buf::of(&bytes));
        assert_eq!(zwsdk::plug::abi::native::take_output(), bytes);
    }

    #[test]
    fn the_reserved_service_asks_for_one_relative_stream() {
        let mut tsmf = Tsmf::new();
        tsmf.start(StreamInfo {
            container: Container::Ts,
            service: Some(zwsdk::abi::PART | 3),
        })
        .expect("starts");
        assert_eq!(
            tsmf.multiplex.taking(),
            Take::Relative(3, Carriage::Transport),
            "相対ストリーム 3 を取る"
        );
    }

    #[test]
    fn a_broadcast_service_is_still_a_service() {
        let mut tsmf = Tsmf::new();
        tsmf.start(StreamInfo {
            container: Container::Ts,
            service: Some(1024),
        })
        .expect("starts");
        assert_eq!(tsmf.multiplex.taking(), Take::Service(1024));
    }

    #[test]
    fn a_transport_stream_without_a_frame_is_passed_whole() {
        let mut tsmf = Tsmf::new();
        tsmf.start(StreamInfo {
            container: Container::Ts,
            service: Some(1024),
        })
        .expect("starts");
        let bytes = null_packets(64);
        let mut fed = 0;
        while fed < (5 << 20) {
            tsmf.input(Buf::of(&bytes));
            fed += bytes.len();
        }
        let out = zwsdk::plug::abi::native::take_output();
        assert_eq!(out.len(), fed);
        assert!(zwsdk::plug::abi::native::take_log().is_empty());
    }
}
