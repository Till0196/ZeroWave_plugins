// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! TLV / MMTS の録画で、データ放送が起動に至る経路（MPT・MH-AIT・DATA_SI・
//! CACHE・FILE・COMPONENT）を一段ずつ、届いた順に出す。

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use isdb::mh::{MESSAGE_M2_SECTION, MESSAGE_M2_SHORT_SECTION, MESSAGE_PA};
use isdb::mhdata::MESSAGE_DATA_TRANSMISSION;
use isdb::mmttables::{self, TABLE_MH_AIT, TABLE_MPT_COMPLETE, TABLE_MPT_SUBSET_FIRST};
use mmttlv::signalling::Signalling;
use mmttlv::tlv::{Feed, Piece};
use zwcore::filters::packet::Section;
use zwcore::filters::{Container, Feed as Broadcast, Sink};

/// MH-アプリケーションサービス記述子（ARIB STD-B60 表 7-87）。
const DESCRIPTOR_APPLICATION_SERVICE: u16 = 0x8034;
/// MH-伝送プロトコル記述子・MH-簡易アプリケーション位置記述子（10.3.3.2、10.3.3.3）。
const DESCRIPTOR_TRANSPORT_PROTOCOL: u16 = 0x802a;
const DESCRIPTOR_SIMPLE_LOCATION: u16 = 0x802b;

#[derive(Default)]
struct Seen {
    mpt: BTreeMap<(u16, u8), String>,
    ait: BTreeMap<(u16, u16, u8), String>,
    data: BTreeMap<u16, BTreeSet<u8>>,
    tables: BTreeMap<u8, u32>,
    packets: u64,
    entries: BTreeSet<String>,
    emts: Vec<(u16, u16, u8, u8, String, u8)>,
}

fn emt(packet_id: u16, section: &[u8], seen: &mut Seen) {
    let Some(table) = mmttables::event_message_table(section) else {
        return;
    };
    for found in &table.descriptors {
        if found.tag != mmttables::DESCRIPTOR_EVENT_MESSAGE {
            continue;
        }
        let Some(message) = mmttables::event_message(&found.body) else {
            continue;
        };
        let key = (
            packet_id,
            table.group_id,
            message.message_id(),
            message.message_version(),
        );
        if seen
            .emts
            .last()
            .is_some_and(|last| (last.0, last.1, last.2, last.3) == key)
        {
            continue;
        }
        if seen.emts.iter().any(|e| (e.0, e.1, e.2, e.3) == key) {
            continue;
        }
        seen.emts.push((
            key.0,
            key.1,
            key.2,
            key.3,
            String::from_utf8_lossy(&message.private).into_owned(),
            message.time_mode,
        ));
    }
}

#[derive(Default)]
struct Files {
    store: web::store::Store,
    items: u64,
    sections: u64,
}

impl Sink for Files {
    fn on_section(&mut self, section: &Section<'_>) {
        let section = section.data;
        if matches!(section.first(), Some(0xa3..=0xa5)) {
            self.sections += 1;
            self.store.table(section);
        }
    }

    fn on_item(&mut self, packet_id: u16, mpu: u32, item: u32, fragmentation: u8, data: &[u8]) {
        self.items += 1;
        self.store.item(packet_id, mpu, item, fragmentation, data);
    }
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("使い方: mmtapp 録画.tlv");
        std::process::exit(2);
    };
    let mut file = std::fs::File::open(&path).expect("録画が開ける");
    let mut feed = Feed::new();
    let mut signalling = Signalling::new();
    let mut seen = Seen::default();
    let mut broadcast = Broadcast::new(Container::Tlv);
    let mut files = Files::default();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer).expect("読める");
        if read == 0 {
            break;
        }
        broadcast.push(&buffer[..read], &mut files);
        for piece in feed.feed(&buffer[..read]) {
            let Piece::Mmtp {
                packet_id,
                payload_type,
                payload,
                scrambled,
                ..
            } = piece
            else {
                continue;
            };
            seen.packets += 1;
            if scrambled || payload_type != mmttlv::mmtp::SIGNALLING {
                continue;
            }
            for message in signalling.push(packet_id, &payload) {
                match message.id {
                    MESSAGE_PA => package_tables(&message.payload, &mut seen),
                    MESSAGE_M2_SECTION | MESSAGE_M2_SHORT_SECTION => {
                        for section in sections(&message.payload) {
                            *seen.tables.entry(section[0]).or_default() += 1;
                            if section[0] == TABLE_MH_AIT {
                                ait(packet_id, section, &mut seen);
                            }
                            if section[0] == mmttables::TABLE_EMT {
                                emt(packet_id, section, &mut seen);
                            }
                        }
                    }
                    MESSAGE_DATA_TRANSMISSION => {
                        for section in sections(&message.payload) {
                            seen.data.entry(packet_id).or_default().insert(section[0]);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    println!("MMTP {} パケット", seen.packets);
    println!("M2 セクションのテーブル: {:?}", seen.tables);
    println!("\n[M1] MPT のアプリケーションサービス記述子（0x8034）");
    if seen.mpt.is_empty() {
        println!(
            "  無し——受像機はここで「データ放送のサービスではない」と見て MH-AIT を見に行かない"
        );
    }
    for ((service, version), said) in &seen.mpt {
        println!("  サービス {service} バージョン {version}: {said}");
    }
    println!("\n[M2] MH-AIT");
    if seen.ait.is_empty() {
        println!("  無し");
    }
    for ((packet_id, kind, version), said) in &seen.ait {
        println!("  packet 0x{packet_id:04x} type 0x{kind:04x} バージョン {version}: {said}");
    }
    println!("\n[M3] データ伝送のテーブル（DDMT 0xa3 / DAMT 0xa4 / DCCT 0xa5）");
    if seen.data.is_empty() {
        println!("  無し");
    }
    for (packet_id, tables) in &seen.data {
        let names: Vec<&str> = tables
            .iter()
            .map(|t| match t {
                0xa3 => "DDMT",
                0xa4 => "DAMT",
                0xa5 => "DCCT",
                _ => "?",
            })
            .collect();
        println!("  packet 0x{packet_id:04x}: {}", names.join(" "));
    }
    println!("\n[EMT] イベントメッセージ（packet、グループ、id、バージョン、時、中身）");
    for (packet_id, group, id, version, private, mode) in &seen.emts {
        println!(
            "  packet 0x{packet_id:04x} グループ 0x{group:03x} id {id} バージョン {version} mode {mode} 「{private}」"
        );
    }
    println!(
        "\n[M4-M5] ストア（Feed → Store）: テーブル {} 枚、断片 {} 個、ファイル {} 本、揃った {}",
        files.sections,
        files.items,
        files.store.len(),
        files.store.is_complete()
    );
    for entry in &seen.entries {
        println!(
            "  エントリー {entry}: {}",
            if files.store.get(entry).is_some() {
                "ある——開ける"
            } else {
                "無い——開けない"
            }
        );
    }
    let mut paths: Vec<&str> = files.store.paths().collect();
    paths.sort_unstable();
    for path in paths.iter().take(40) {
        println!("    {path}");
    }
    if paths.len() > 40 {
        println!("    …あと {} 本", paths.len() - 40);
    }
}

fn package_tables(payload: &[u8], seen: &mut Seen) {
    let Some(&tables) = payload.first() else {
        return;
    };
    let mut at = 1 + 4 * usize::from(tables);
    while at + 4 <= payload.len() {
        let length = usize::from(payload[at + 2]) << 8 | usize::from(payload[at + 3]);
        if at + 4 + length > payload.len() {
            break;
        }
        if (TABLE_MPT_SUBSET_FIRST..=TABLE_MPT_COMPLETE).contains(&payload[at])
            && let Some(table) = mmttables::package_table(&payload[at..at + 4 + length])
        {
            let service = table.service_id();
            let key = (service, table.version);
            if let std::collections::btree_map::Entry::Vacant(slot) = seen.mpt.entry(key) {
                let mut said = Vec::new();
                for found in &table.descriptors {
                    if found.tag == DESCRIPTOR_APPLICATION_SERVICE {
                        said.push(application_service(&found.body));
                    }
                }
                let assets: Vec<String> = table
                    .assets
                    .iter()
                    .map(|asset| {
                        format!(
                            "{}@{}",
                            asset.kind,
                            asset
                                .packet_id()
                                .map_or(String::from("-"), |id| format!("0x{id:04x}"))
                        )
                    })
                    .collect();
                let summary = if said.is_empty() {
                    format!("記述子 0x8034 無し。アセット {}", assets.join(" "))
                } else {
                    format!("{}。アセット {}", said.join("; "), assets.join(" "))
                };
                println!(
                    "MPT サービス {service} バージョン {}: {summary}",
                    table.version
                );
                slot.insert(summary);
            }
        }
        at += 4 + length;
    }
}

fn application_service(body: &[u8]) -> String {
    if body.len() < 3 {
        return String::from("0x8034（短い）");
    }
    let format = body[0] >> 4;
    let resolution = body[1] >> 4;
    let default_ait = body[2] & 0x80 != 0;
    let dt_message = body[2] & 0x40 != 0;
    let emt_num = body[2] & 0x0f;
    let ait_location = general_location(&body[3..]);
    // EMT の名前と位置（AIT の場所 3 バイト、フラグが立てばデータ伝送の場所 3 バイトの後）。
    let mut emts = Vec::new();
    let mut at = 3 + 3 + if dt_message { 3 } else { 0 };
    for _ in 0..emt_num {
        if at + 4 > body.len() || body[at + 1] != 0 {
            break;
        }
        emts.push(format!(
            "0x{:02x}→0x{:04x}",
            body[at],
            u16::from_be_bytes([body[at + 2], body[at + 3]])
        ));
        at += 4;
    }
    format!(
        "0x8034 format {format}（1=ARIB-HTML5） 解像度 {} default_AIT {} DT_message {} EMT {emt_num} [{}] AIT の場所 {ait_location}",
        match resolution {
            0 => "1080",
            1 => "2160",
            2 => "4320",
            _ => "?",
        },
        u8::from(default_ait),
        u8::from(dt_message),
        emts.join(" ")
    )
}

fn general_location(body: &[u8]) -> String {
    match body.first() {
        Some(0x00) if body.len() >= 3 => {
            format!("packet 0x{:04x}", u16::from_be_bytes([body[1], body[2]]))
        }
        Some(kind) => format!("種類 0x{kind:02x}"),
        None => String::from("無し"),
    }
}

fn ait(packet_id: u16, section: &[u8], seen: &mut Seen) {
    let Some(ait) = mmttables::application_information(section) else {
        return;
    };
    let key = (packet_id, ait.application_type, ait.version);
    if seen.ait.contains_key(&key) {
        return;
    }
    let mut apps = Vec::new();
    for app in &ait.applications {
        let base = app
            .descriptors
            .iter()
            .find(|d| d.tag == DESCRIPTOR_TRANSPORT_PROTOCOL)
            .map(|d| format!("{:02x?}", &d.body[..d.body.len().min(6)]));
        let location = app
            .descriptors
            .iter()
            .find(|d| d.tag == DESCRIPTOR_SIMPLE_LOCATION)
            .map(|d| String::from_utf8_lossy(&d.body).into_owned());
        if app.control_code == 0x01
            && let (Some(base), Some(location)) = (
                app.descriptors
                    .iter()
                    .find(|d| d.tag == DESCRIPTOR_TRANSPORT_PROTOCOL)
                    .and_then(|d| transport_base(&d.body)),
                location.as_deref(),
            )
        {
            seen.entries.insert(format!("{base}{location}"));
        }
        apps.push(format!(
            "app 0x{:08x}/0x{:04x} control {} ({}) 伝送 {} 場所 {}",
            app.organization_id,
            app.application_id,
            app.control_code,
            match app.control_code {
                0x01 => "AUTOSTART",
                0x02 => "PRESENT",
                0x04 => "KILL",
                _ => "?",
            },
            base.unwrap_or_else(|| String::from("無し")),
            location.unwrap_or_else(|| String::from("無し")),
        ));
    }
    let summary = if apps.is_empty() {
        String::from("アプリ無し")
    } else {
        apps.join("; ")
    };
    println!(
        "MH-AIT packet 0x{packet_id:04x} type 0x{:04x} バージョン {}: {summary}",
        ait.application_type, ait.version
    );
    seen.ait.insert(key, summary);
}

fn sections(payload: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 3 <= payload.len() {
        if payload[at] == 0xff {
            break;
        }
        let length = usize::from(u16::from_be_bytes([payload[at + 1], payload[at + 2]])) & 0x0fff;
        let next = at + 3 + length;
        if next > payload.len() {
            break;
        }
        out.push(&payload[at..next]);
        at = next;
    }
    out
}

fn transport_base(body: &[u8]) -> Option<String> {
    if body.len() < 4 {
        return None;
    }
    let protocol = u16::from_be_bytes([body[0], body[1]]);
    if protocol != 0x0005 && protocol != 0x0003 {
        return None;
    }
    let length = usize::from(body[3]);
    let base = body.get(4..4 + length)?;
    Some(String::from_utf8_lossy(base).into_owned())
}
