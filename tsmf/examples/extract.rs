// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! Takes one stream out of a recording of a cable carrier (`extract -- carrier.ts out.ts`).
//! `--stream` names the TSID, `--relative` the slot, `--service` the service to find;
//! without any, the first stream the frame names is taken.

use zerowave_tsmf::{Multiplex, SLOTS, Survey, Take};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Container {
    Ts,
    Tlv,
}

fn main() {
    let mut relative: Option<u8> = None;
    let mut stream_id: Option<u16> = None;
    let mut service: Option<u16> = None;
    let mut paths = Vec::new();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--relative" => {
                relative = arguments
                    .next()
                    .and_then(|number| number.parse::<u8>().ok());
            }
            "--stream" => stream_id = arguments.next().and_then(|id| parse(&id)),
            "--service" => service = arguments.next().and_then(|id| parse(&id)),
            other => paths.push(other.to_owned()),
        }
    }
    let [path, target] = paths.as_slice() else {
        eprintln!(
            "usage: extract [--relative <n>] [--stream <tsid>] [--service <sid>] <carrier> <out>"
        );
        std::process::exit(2);
    };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("{path}: {error}");
            std::process::exit(1);
        }
    };
    let (container, taken) = demultiplex(Container::Ts, bytes, relative, stream_id, service);
    if let Err(error) = std::fs::write(target, &taken) {
        eprintln!("{target}: {error}");
        std::process::exit(1);
    }
    eprintln!(
        "{} バイトを {target} へ（{}）",
        taken.len(),
        match container {
            Container::Ts => "TS",
            Container::Tlv => "TLV",
        }
    );
}

/// A slot may hold TLV rather than transport packets, which the file name does not say.
fn demultiplex(
    container: Container,
    bytes: Vec<u8>,
    relative: Option<u8>,
    stream_id: Option<u16>,
    service: Option<u16>,
) -> (Container, Vec<u8>) {
    let mut survey = Survey::new();
    survey.feed(&bytes[..bytes.len().min(8 << 20)]);
    let streams = survey.streams();
    if streams.is_empty() {
        if relative.is_some() || stream_id.is_some() {
            eprintln!("多重フレームが見当たらないので、そのまま再生する");
        }
        return (container, bytes);
    }
    // A service says nothing about which stream carries it, so the streams are read until one does.
    if let Some(service) = service {
        return by_service(container, bytes, service);
    }
    let chosen = streams
        .iter()
        .find(|stream| Some(stream.stream_id) == stream_id)
        .or_else(|| {
            streams
                .iter()
                .find(|stream| Some(stream.number) == relative)
        })
        .or_else(|| streams.first());
    for stream in &streams {
        let kind = if stream.tlv { "TLV" } else { "TS" };
        let mark = if chosen.map(|chosen| chosen.number) == Some(stream.number) {
            " ←"
        } else {
            ""
        };
        eprintln!(
            "多重 {} {kind}  TSID 0x{:04x}  ONID 0x{:04x}  スロット {}/{}{mark}",
            stream.number, stream.stream_id, stream.original_network_id, stream.slots, SLOTS,
        );
    }
    let Some(chosen) = chosen else {
        return (container, bytes);
    };
    if stream_id.is_some_and(|wanted| wanted != chosen.stream_id) {
        eprintln!(
            "TSID 0x{:04x} はこのキャリアに無い",
            stream_id.unwrap_or_default()
        );
    }
    let container = if chosen.tlv {
        Container::Tlv
    } else {
        Container::Ts
    };
    let mut multiplex = Multiplex::new(Take::Stream(chosen.stream_id));
    let mut kept = Vec::with_capacity(bytes.len());
    for chunk in bytes.chunks(65_536) {
        kept.extend_from_slice(&multiplex.feed(0, chunk));
    }
    (container, kept)
}

fn by_service(container: Container, bytes: Vec<u8>, service: u16) -> (Container, Vec<u8>) {
    let mut multiplex = Multiplex::new(Take::Service(service));
    let mut kept = Vec::with_capacity(bytes.len());
    for chunk in bytes.chunks(65_536) {
        kept.extend_from_slice(&multiplex.feed(0, chunk));
    }
    let Some(chosen) = multiplex.chosen() else {
        eprintln!("SID 0x{service:04x} を含むストリームが無い");
        for (number, services) in multiplex.services() {
            let named: Vec<String> = services
                .iter()
                .map(|service| format!("0x{service:04x}"))
                .collect();
            eprintln!("多重 {number}  SID {}", named.join(" "));
        }
        return (container, bytes);
    };
    eprintln!(
        "多重 {} {}  TSID 0x{:04x}  スロット {}/{}  ← SID 0x{service:04x}",
        chosen.number,
        if chosen.tlv { "TLV" } else { "TS" },
        chosen.stream_id,
        chosen.slots,
        SLOTS,
    );
    let container = if chosen.tlv {
        Container::Tlv
    } else {
        Container::Ts
    };
    (container, kept)
}

fn parse(text: &str) -> Option<u16> {
    let text = text.trim();
    match text.strip_prefix("0x") {
        Some(hex) => u16::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}
