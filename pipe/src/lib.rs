// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 受けたストリームを、利用者が設定ページで選んだプログラム（TS と MMT/TLV で別）に通す stream hook。
//! 多重化フレームを分離するプラグインより後（`order = 10`）。ヘッダーの制御のビットが立ったパケットがあるときだけ通し、
//! 止まったら起動し直す（その間の物は捨てる）。状態は `ts_state`・`tlv_state` に書く。

use zwsdk::plug::host::proc::Process;
use zwsdk::plug::{Buf, Container, Stream, StreamInfo, host, info, output, output_buf, warn};
use zwsdk::ui::{Badge, Tone};

mod page;

/// 最初に書いてから、出力が無いまま待つ長さ。
const PATIENCE_MS: f64 = 1500.0;

/// 止まってから起動し直すまで。
const AGAIN_MS: f64 = 1500.0;

/// 起動し直す回数。これを超えて止まったらそのまま通す。
const MOST_DEATHS: u32 = 3;

/// ビットの立っていない映像・音声のパケットがこれだけ続いたら、そのまま通す。
const PLAIN_EVIDENCE: u32 = 16;

/// 決まらないまま抱えるのはここまで。超えたらプログラムに通す。
const DECIDE_MS: f64 = 1500.0;
const DECIDE_BYTES: usize = 8 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Marked,
    Plain,
    Unsure,
}

#[derive(Debug)]
enum Look {
    Ts {
        carry: Vec<u8>,
        plain: u32,
    },
    /// MMT/TLV。MMTP の拡張ヘッダーの制御のビットを見る。
    Tlv {
        feed: mmttlv::tlv::Feed,
        plain: u32,
    },
}

impl Look {
    fn new(container: Container) -> Option<Self> {
        match container {
            Container::Ts => Some(Self::Ts {
                carry: Vec::new(),
                plain: 0,
            }),
            Container::Tlv => Some(Self::Tlv {
                feed: mmttlv::tlv::Feed::new(),
                plain: 0,
            }),
            Container::Unknown => None,
        }
    }

    fn look(&mut self, bytes: &[u8]) -> Verdict {
        match self {
            Self::Ts { carry, plain } => {
                carry.extend_from_slice(bytes);
                let (verdict, used) = ts_marks(carry, plain);
                carry.drain(..used);
                verdict
            }
            Self::Tlv { feed, plain } => {
                for piece in feed.feed(bytes) {
                    if let mmttlv::tlv::Piece::Mmtp {
                        scrambled: marked,
                        payload_type,
                        ..
                    } = piece
                    {
                        if marked {
                            return Verdict::Marked;
                        }
                        // MPU（映像・音声）だけを数える。制御の物はビットが立たない。
                        if payload_type == 0 {
                            *plain += 1;
                        }
                    }
                }
                if *plain >= PLAIN_EVIDENCE {
                    Verdict::Plain
                } else {
                    Verdict::Unsure
                }
            }
        }
    }
}

/// TS のパケットを見る。4 バイト目の上の 2 ビットが立っていれば [`Verdict::Marked`]、立っていない
/// PES のヘッダーを [`PLAIN_EVIDENCE`] 個数えたら [`Verdict::Plain`]。返すのは判定と、読み終えた長さ。
fn ts_marks(bytes: &[u8], plain: &mut u32) -> (Verdict, usize) {
    const SIZE: usize = 188;
    let mut at = 0;
    while at + SIZE <= bytes.len() {
        if bytes[at] != 0x47 || bytes.get(at + SIZE).is_some_and(|next| *next != 0x47) {
            at += 1;
            continue;
        }
        let packet = &bytes[at..at + SIZE];
        at += SIZE;
        if packet[3] & 0xc0 != 0 {
            return (Verdict::Marked, at);
        }
        let start = packet[1] & 0x40 != 0;
        let payload = match (packet[3] >> 4) & 0x3 {
            1 => 4,
            3 => 5 + usize::from(packet[4]),
            _ => continue,
        };
        if start && packet.get(payload..payload + 3) == Some(&[0, 0, 1]) {
            *plain += 1;
            if *plain >= PLAIN_EVIDENCE {
                return (Verdict::Plain, at);
            }
        }
    }
    (Verdict::Unsure, at)
}

#[derive(Debug, Default)]
pub struct Pipe {
    state: State,
    what: &'static str,
    key: &'static str,
    env: &'static str,
    shown: &'static str,
    args: Vec<String>,
    deaths: u32,
}

#[derive(Debug)]
struct Deciding {
    look: Look,
    held: Vec<u8>,
    since: f64,
}

#[derive(Debug, Default)]
enum State {
    #[default]
    Through,
    /// 動いている。`deadline` は最初に書いたときに付き、出力が来たら消える。
    Running {
        process: Process,
        written: bool,
        deadline: Option<f64>,
    },
    /// 止まって、この時刻に起動し直す。それまでは捨てる。
    Waiting {
        until: f64,
    },
    Deciding(Box<Deciding>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Keys {
    program: &'static str,
    args: &'static str,
    env: &'static str,
    state: &'static str,
    what: &'static str,
}

const TS: Keys = Keys {
    program: "ts",
    args: "ts_args",
    env: "ts_env",
    state: "ts_state",
    what: "TS",
};

const TLV: Keys = Keys {
    program: "tlv",
    args: "tlv_args",
    env: "tlv_env",
    state: "tlv_state",
    what: "MMT/TLV",
};

fn keys(container: Container) -> Option<Keys> {
    match container {
        Container::Ts => Some(TS),
        Container::Tlv => Some(TLV),
        Container::Unknown => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shown {
    Idle,
    Running,
    Restarting,
    /// 通さなくてよいので、そのまま通している。
    Unused,
    /// 起動できない・止まり続けたので、そのまま通している。
    Stopped,
}

impl Shown {
    fn word(self) -> &'static str {
        match self {
            Self::Idle => "",
            Self::Running => "running",
            Self::Restarting => "restarting",
            Self::Unused => "unused",
            Self::Stopped => "stopped",
        }
    }

    fn of(word: &str) -> Self {
        match word {
            "running" => Self::Running,
            "restarting" => Self::Restarting,
            "unused" => Self::Unused,
            "stopped" => Self::Stopped,
            _ => Self::Idle,
        }
    }

    fn badge(self) -> Option<Badge> {
        let (text, tone) = match self {
            Self::Idle => return None,
            Self::Running => ("動いています", Tone::Good),
            Self::Restarting => ("起動し直しています", Tone::Warn),
            Self::Unused => ("使っていません", Tone::Muted),
            Self::Stopped => ("止まりました", Tone::Bad),
        };
        Some(Badge::new(text, tone))
    }
}

impl Pipe {
    fn show(&self, shown: Shown) {
        if !self.shown.is_empty() {
            host::set_cfg(self.shown, shown.word());
        }
    }

    fn running(process: Process) -> State {
        State::Running {
            process,
            written: false,
            deadline: None,
        }
    }

    fn dead(&mut self, why: &str) {
        self.deaths += 1;
        if self.deaths > MOST_DEATHS {
            warn(&format!(
                "{} のプログラムが{why}（{} 回目）。この選局では {} をそのまま通します",
                self.what, self.deaths, self.what
            ));
            self.state = State::Through;
            self.show(Shown::Stopped);
            return;
        }
        warn(&format!(
            "{} のプログラムが{why}（{} 回目）。起動し直します",
            self.what, self.deaths
        ));
        self.state = State::Waiting {
            until: host::monotonic_ms() + AGAIN_MS,
        };
        self.show(Shown::Restarting);
    }

    fn pass(&mut self, bytes: &[u8]) {
        let State::Running {
            process,
            written,
            deadline,
        } = &mut self.state
        else {
            return;
        };
        if !*written {
            *written = true;
            *deadline = Some(host::monotonic_ms() + PATIENCE_MS);
        }
        if !process.write(bytes) {
            return self.dead("受け取らなくなりました");
        }
        match process.read() {
            Some(out) if !out.is_empty() => {
                *deadline = None;
                output(&out);
            }
            Some(_) => {
                if deadline.is_some_and(|at| host::monotonic_ms() >= at) {
                    self.dead("応答しません");
                }
            }
            None => self.dead("終わりました"),
        }
    }
}

impl Pipe {
    fn begin(&mut self, why: &str) {
        let held = match std::mem::take(&mut self.state) {
            State::Deciding(deciding) => deciding.held,
            _ => Vec::new(),
        };
        match Process::spawn_with(self.key, &self.args, self.env) {
            Some(process) => {
                info(&format!("{} を選んだプログラムに通します{why}", self.what));
                self.state = Self::running(process);
                self.show(Shown::Running);
                self.pass(&held);
            }
            None => {
                info(&format!(
                    "{} のプログラムを起動できないので、そのまま通します",
                    self.what
                ));
                self.state = State::Through;
                self.show(Shown::Stopped);
                output(&held);
            }
        }
    }
}

impl Stream for Pipe {
    fn new() -> Self {
        Self::default()
    }

    fn start(&mut self, stream: StreamInfo) -> Result<(), String> {
        self.state = State::Through;
        self.deaths = 0;
        let Some(Keys {
            program: key,
            args,
            env,
            state,
            what,
        }) = keys(stream.container)
        else {
            return Ok(());
        };
        self.what = what;
        self.key = key;
        self.env = env;
        self.shown = state;
        self.args = zwsdk::config::list(args);
        self.show(Shown::Idle);
        if host::cfg(key).unwrap_or_default().trim().is_empty() {
            info(&format!(
                "{what} のプログラムが選ばれていないので、そのまま通します"
            ));
            return Ok(());
        }
        if let Some(look) = Look::new(stream.container) {
            self.state = State::Deciding(Box::new(Deciding {
                look,
                held: Vec::new(),
                since: host::monotonic_ms(),
            }));
        }
        Ok(())
    }

    fn input(&mut self, buf: Buf) {
        if let State::Deciding(deciding) = &mut self.state {
            let bytes = buf.to_vec();
            let verdict = deciding.look.look(&bytes);
            deciding.held.extend_from_slice(&bytes);
            let late = host::monotonic_ms() - deciding.since >= DECIDE_MS
                || deciding.held.len() >= DECIDE_BYTES;
            match verdict {
                Verdict::Plain => {
                    info(&format!(
                        "{} はプログラムに通さなくてよいので、そのまま通します",
                        self.what
                    ));
                    let held = std::mem::take(&mut deciding.held);
                    self.state = State::Through;
                    self.show(Shown::Unused);
                    output(&held);
                }
                Verdict::Marked => self.begin(""),
                Verdict::Unsure if late => self.begin("（見分けが付かないまま時間が来たので）"),
                Verdict::Unsure => {}
            }
            return;
        }
        if let State::Waiting { until } = self.state {
            if host::monotonic_ms() < until {
                return;
            }
            match Process::spawn_with(self.key, &self.args, self.env) {
                Some(process) => {
                    info(&format!("{} のプログラムを起動し直しました", self.what));
                    self.state = Self::running(process);
                    self.show(Shown::Running);
                }
                None => return self.dead("起動できません"),
            }
        }
        match self.state {
            State::Through => output_buf(&buf),
            State::Running { .. } => self.pass(&buf.to_vec()),
            State::Waiting { .. } | State::Deciding(_) => {}
        }
    }

    fn stop(&mut self) {
        self.state = State::Through;
        self.show(Shown::Idle);
    }
}

zwsdk::export_stream!(
    Pipe,
    name = "パイプ",
    version = "0.1.0",
    order = 10,
    ui = page::Settings,
    ui_manifest = r#"{"title":"パイプ","settings":"main"}"#,
    config = r#"[
        {"key":"ts","kind":"program","default":"",
         "label":"TS のプログラム",
         "hint":"標準入力で TS を受け、標準出力へ出す実行ファイル。空ならそのまま"},
        {"key":"ts_args","kind":"list","item":"text","default":[],
         "label":"その引数"},
        {"key":"ts_env","kind":"map","item":"env","default":{},
         "label":"その環境変数","hint":"値の $TMPDIR・${HOME} などは展開する"},
        {"key":"ts_state","kind":"state","default":""},
        {"key":"tlv","kind":"program","default":"",
         "label":"MMT/TLV のプログラム","section":"MMT/TLV",
         "hint":"標準入力で MMT/TLV を受け、標準出力へ出す実行ファイル。空ならそのまま"},
        {"key":"tlv_args","kind":"list","item":"text","default":[],
         "label":"その引数"},
        {"key":"tlv_env","kind":"map","item":"env","default":{},
         "label":"その環境変数","hint":"値の $TMPDIR・${HOME} などは展開する"},
        {"key":"tlv_state","kind":"state","default":""}
    ]"#
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_container_has_its_own_program() {
        assert_eq!(keys(Container::Ts).map(|k| k.state), Some("ts_state"));
        assert_eq!(
            keys(Container::Ts).map(|k| (k.program, k.env)),
            Some(("ts", "ts_env"))
        );
        assert_eq!(
            keys(Container::Tlv).map(|k| (k.program, k.env)),
            Some(("tlv", "tlv_env"))
        );
        assert_eq!(keys(Container::Unknown), None);
    }

    #[test]
    fn without_a_program_the_stream_passes_through() {
        let mut pipe = Pipe::new();
        pipe.start(StreamInfo {
            container: Container::Ts,
            service: None,
        })
        .unwrap();
        let packet = [0x47u8; 188];
        pipe.input(Buf::of(&packet));
        assert_eq!(zwsdk::plug::abi::native::take_output(), packet);
    }
}

#[cfg(test)]
mod looking {
    use super::*;

    fn packet(pid: u16, start: bool, marked: bool, pes: bool) -> [u8; 188] {
        let mut packet = [0xffu8; 188];
        packet[0] = 0x47;
        packet[1] = (if start { 0x40 } else { 0 }) | (pid >> 8) as u8;
        packet[2] = pid as u8;
        packet[3] = 0x10 | if marked { 0x80 } else { 0 };
        if pes {
            packet[4..7].copy_from_slice(&[0, 0, 1]);
        }
        packet
    }

    #[test]
    fn a_marked_packet_decides_at_once() {
        let mut look = Look::new(Container::Ts).unwrap();
        let mut bytes = packet(0, true, false, false).to_vec();
        bytes.extend_from_slice(&packet(0x111, false, true, false));
        assert_eq!(look.look(&bytes), Verdict::Marked);
    }

    #[test]
    fn plain_pes_starts_decide_across_chunks() {
        let mut look = Look::new(Container::Ts).unwrap();
        let mut bytes = Vec::new();
        for _ in 0..PLAIN_EVIDENCE + 1 {
            bytes.extend_from_slice(&packet(0x111, true, false, true));
        }
        let (head, tail) = bytes.split_at(1000);
        assert_eq!(look.look(head), Verdict::Unsure);
        assert_eq!(look.look(tail), Verdict::Plain);
    }

    #[test]
    fn a_sample_file() {
        let Ok(path) = std::env::var("ZW_PIPE_SAMPLE") else {
            return;
        };
        let container = if path.ends_with(".ts") {
            Container::Ts
        } else {
            Container::Tlv
        };
        let mut bytes = vec![0u8; 8 << 20];
        let got =
            std::io::Read::read(&mut std::fs::File::open(&path).unwrap(), &mut bytes).unwrap();
        bytes.truncate(got);
        let mut look = Look::new(container).unwrap();
        let mut verdict = Verdict::Unsure;
        let mut read = 0;
        for chunk in bytes.chunks(64 * 1024).take(128) {
            read += chunk.len();
            verdict = look.look(chunk);
            if verdict != Verdict::Unsure {
                break;
            }
        }
        zwsdk::plug::info(&format!("{path}: {verdict:?} at {read} bytes"));
    }
}
