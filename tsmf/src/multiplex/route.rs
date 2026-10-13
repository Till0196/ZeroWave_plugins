// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! Taking one stream out of a frame whose slots have been thinned (Mirakurun strips
//! null packets, so slots can no longer be counted). The stream is read by packet
//! identifier instead, learning its identifiers from its own PAT and PMTs.

use std::collections::{BTreeMap, BTreeSet};

use isdb::tstables;

const PACKET: usize = 188;
const SYNC: u8 = 0x47;
/// Identifiers every stream shares: the MPEG-2 and ARIB tables and the frame header.
const SHARED_BELOW: u16 = 0x30;
const PID_PAT: u16 = 0x0000;

/// Far fewer than 53 packets between headers means the null packets were stripped.
#[derive(Debug, Default)]
pub(super) struct Gauge {
    held: Vec<u8>,
    headers: u32,
    since_header: u32,
    packets_between: u64,
}

const DECIDE_HEADERS: u32 = 8;
const THINNED_BELOW: u64 = 45;

impl Gauge {
    pub(super) fn feed(&mut self, arrived: &[u8]) {
        if self.decided() {
            return;
        }
        self.held.extend_from_slice(arrived);
        let mut at = 0;
        while at + PACKET <= self.held.len() {
            if self.held[at] != SYNC {
                at += 1;
                continue;
            }
            let packet = &self.held[at..at + PACKET];
            let pid = pid_of(packet);
            if pid == isdb::tsmf::HEADER_PID && isdb::tsmf::info(packet).is_some() {
                if self.headers > 0 {
                    self.packets_between += u64::from(self.since_header);
                }
                self.headers += 1;
                self.since_header = 0;
            } else if self.headers > 0 {
                self.since_header += 1;
            }
            at += PACKET;
        }
        self.held.drain(..at);
    }

    pub(super) fn decided(&self) -> bool {
        self.headers > DECIDE_HEADERS
    }

    pub(super) fn thinned(&self) -> bool {
        self.decided() && self.packets_between / u64::from(self.headers - 1) < THINNED_BELOW
    }
}

/// Which stream carries which services, read from the association tables as they go past.
#[derive(Debug, Default)]
pub(super) struct Associations {
    held: Vec<u8>,
    services: BTreeMap<u16, Vec<u16>>,
}

impl Associations {
    pub(super) fn feed(&mut self, arrived: &[u8]) {
        self.held.extend_from_slice(arrived);
        let mut at = 0;
        while at + PACKET <= self.held.len() {
            if self.held[at] != SYNC {
                at += 1;
                continue;
            }
            let packet = &self.held[at..at + PACKET];
            if pid_of(packet) == PID_PAT
                && let Some((stream_id, programs)) = association_in(packet)
            {
                let known = self.services.entry(stream_id).or_default();
                for (number, _) in programs {
                    if number != 0 && !known.contains(&number) {
                        known.push(number);
                    }
                }
            }
            at += PACKET;
        }
        self.held.drain(..at);
    }

    pub(super) fn stream_of(&self, service: u16) -> Option<u16> {
        self.services
            .iter()
            .find(|(_, services)| services.contains(&service))
            .map(|(stream_id, _)| *stream_id)
    }
}

#[derive(Debug)]
pub(super) struct Router {
    stream_id: u16,
    held: Vec<u8>,
    keep: BTreeSet<u16>,
    drop: BTreeSet<u16>,
    /// This stream's programme numbers, which say whose a map is when two streams share its PID.
    programs_mine: BTreeSet<u16>,
    maps: BTreeSet<u16>,
    other_maps: BTreeSet<u16>,
    /// A PID both this stream and another use; those packets cannot be told apart.
    collision: Option<u16>,
    sections: BTreeMap<u16, Vec<u8>>,
}

impl Router {
    pub(super) fn new(stream_id: u16) -> Self {
        Self {
            stream_id,
            held: Vec::new(),
            keep: BTreeSet::new(),
            drop: BTreeSet::new(),
            programs_mine: BTreeSet::new(),
            maps: BTreeSet::new(),
            other_maps: BTreeSet::new(),
            collision: None,
            sections: BTreeMap::new(),
        }
    }

    pub(super) fn collision(&self) -> Option<u16> {
        self.collision
    }

    pub(super) fn feed(&mut self, arrived: &[u8]) -> Vec<u8> {
        self.held.extend_from_slice(arrived);
        let mut kept = Vec::with_capacity(arrived.len());
        let mut at = 0;
        while at + PACKET <= self.held.len() {
            if self.held[at] != SYNC {
                at += 1;
                continue;
            }
            let packet = self.held[at..at + PACKET].to_vec();
            if self.wanted(&packet) {
                kept.extend_from_slice(&packet);
            }
            at += PACKET;
        }
        self.held.drain(..at);
        kept
    }

    fn wanted(&mut self, packet: &[u8]) -> bool {
        let pid = pid_of(packet);
        if pid == PID_PAT {
            let Some((stream_id, programs)) = association_in(packet) else {
                return false;
            };
            let mine = stream_id == self.stream_id;
            for (number, map) in programs {
                if mine {
                    self.programs_mine.insert(number);
                    self.keep.insert(map);
                    if number != 0 {
                        self.maps.insert(map);
                    }
                } else {
                    if number != 0 {
                        self.other_maps.insert(map);
                    }
                    if !self.keep.contains(&map) {
                        self.drop.insert(map);
                    }
                }
            }
            return mine;
        }
        // A map may sit on a PID both streams use; its programme number says whose it is.
        if (self.maps.contains(&pid) || self.other_maps.contains(&pid))
            && let Some(section) = self.assemble(pid, packet)
            && let Some(map) = tstables::program_map(&section)
        {
            let mut named = vec![map.pcr_pid];
            for descriptor in &map.descriptors {
                if let Some(ecm) = ca_pid(descriptor) {
                    named.push(ecm);
                }
            }
            for stream in &map.streams {
                named.push(stream.pid);
                for descriptor in &stream.descriptors {
                    if let Some(ecm) = ca_pid(descriptor) {
                        named.push(ecm);
                    }
                }
            }
            let mine = self.maps.contains(&pid) && !self.other_maps.contains(&pid)
                || self.programs_mine.contains(&map.program_number);
            if mine {
                self.keep.extend(named);
            } else if let Some(shared) = named.iter().find(|pid| self.keep.contains(pid)) {
                self.collision.get_or_insert(*shared);
            }
        }
        if self.keep.contains(&pid) {
            return true;
        }
        if self.drop.contains(&pid) {
            return false;
        }
        pid < SHARED_BELOW
    }

    fn assemble(&mut self, pid: u16, packet: &[u8]) -> Option<Vec<u8>> {
        let starts = packet[1] & 0x40 != 0;
        let payload = payload_of(packet)?;
        if starts {
            let pointer = usize::from(*payload.first()?);
            let section = payload.get(1 + pointer..)?;
            self.sections.insert(pid, section.to_vec());
        } else {
            let section = self.sections.get_mut(&pid)?;
            section.extend_from_slice(payload);
        }
        let section = self.sections.get(&pid)?;
        let length = 3 + (usize::from(section.get(1)? & 0x0f) << 8 | usize::from(*section.get(2)?));
        if section.len() < length {
            return None;
        }
        let whole = section[..length].to_vec();
        self.sections.remove(&pid);
        Some(whole)
    }
}

fn pid_of(packet: &[u8]) -> u16 {
    (u16::from(packet[1] & 0x1f) << 8) | u16::from(packet[2])
}

fn payload_of(packet: &[u8]) -> Option<&[u8]> {
    let control = (packet[3] >> 4) & 0x03;
    match control {
        0x01 => Some(&packet[4..]),
        0x03 => {
            let adaptation = usize::from(packet[4]);
            packet.get(5 + adaptation..)
        }
        _ => None,
    }
}

/// The association table in a packet: the stream it names and (programme, map PID).
fn association_in(packet: &[u8]) -> Option<(u16, Vec<(u16, u16)>)> {
    if packet[1] & 0x40 == 0 {
        return None;
    }
    let payload = payload_of(packet)?;
    let pointer = usize::from(*payload.first()?);
    let section = payload.get(1 + pointer..)?;
    if *section.first()? != 0x00 {
        return None;
    }
    let length = (usize::from(section[1] & 0x0f) << 8) | usize::from(*section.get(2)?);
    let section = section.get(..3 + length)?;
    if isdb::tsmf::crc32(section) != 0 {
        return None;
    }
    let stream_id = (u16::from(section[3]) << 8) | u16::from(section[4]);
    let mut programs = Vec::new();
    let body = &section[8..section.len() - 4];
    for entry in body.as_chunks::<4>().0 {
        let number = (u16::from(entry[0]) << 8) | u16::from(entry[1]);
        let map = (u16::from(entry[2] & 0x1f) << 8) | u16::from(entry[3]);
        programs.push((number, map));
    }
    Some((stream_id, programs))
}

fn ca_pid(descriptor: &tstables::Descriptor) -> Option<u16> {
    if descriptor.tag != 0x09 || descriptor.body.len() < 4 {
        return None;
    }
    Some((u16::from(descriptor.body[2] & 0x1f) << 8) | u16::from(descriptor.body[3]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(pid: u16, start: bool, payload: &[u8]) -> Vec<u8> {
        let mut packet = vec![0xffu8; PACKET];
        packet[0] = SYNC;
        packet[1] = (if start { 0x40 } else { 0 }) | (pid >> 8) as u8;
        packet[2] = pid as u8;
        packet[3] = 0x10;
        packet[4..4 + payload.len()].copy_from_slice(payload);
        packet
    }

    fn with_crc(mut section: Vec<u8>) -> Vec<u8> {
        let mut register: u32 = 0xFFFF_FFFF;
        for byte in &section {
            register ^= u32::from(*byte) << 24;
            for _ in 0..8 {
                register = if register & 0x8000_0000 != 0 {
                    (register << 1) ^ 0x04C1_1DB7
                } else {
                    register << 1
                };
            }
        }
        section.extend_from_slice(&register.to_be_bytes());
        section
    }

    fn pat(stream_id: u16, programs: &[(u16, u16)]) -> Vec<u8> {
        let length = 5 + programs.len() * 4 + 4;
        let mut section = vec![0x00, 0xb0 | (length >> 8) as u8, length as u8];
        section.extend_from_slice(&stream_id.to_be_bytes());
        section.extend_from_slice(&[0xc1, 0x00, 0x00]);
        for (number, map) in programs {
            section.extend_from_slice(&number.to_be_bytes());
            section.extend_from_slice(&(0xe000 | map).to_be_bytes());
        }
        let section = with_crc(section);
        let mut payload = vec![0u8];
        payload.extend_from_slice(&section);
        packet(PID_PAT, true, &payload)
    }

    fn pmt(map: u16, number: u16, pcr: u16, es: &[u16]) -> Vec<u8> {
        let length = 5 + 4 + es.len() * 5 + 4;
        let mut section = vec![0x02, 0xb0 | (length >> 8) as u8, length as u8];
        section.extend_from_slice(&number.to_be_bytes());
        section.extend_from_slice(&[0xc1, 0x00, 0x00]);
        section.extend_from_slice(&(0xe000 | pcr).to_be_bytes());
        section.extend_from_slice(&[0xf0, 0x00]);
        for pid in es {
            section.push(0x02);
            section.extend_from_slice(&(0xe000 | pid).to_be_bytes());
            section.extend_from_slice(&[0xf0, 0x00]);
        }
        let section = with_crc(section);
        let mut payload = vec![0u8];
        payload.extend_from_slice(&section);
        packet(map, true, &payload)
    }

    #[test]
    fn a_thinned_frame_is_read_by_identifier() {
        let mut router = Router::new(0x40f2);
        let mut carrier = Vec::new();
        carrier.extend(pat(0x40f2, &[(0, 0x10), (201, 0x101)]));
        carrier.extend(pat(0x48f3, &[(0, 0x10), (200, 0x101)]));
        carrier.extend(pmt(0x101, 201, 0x1001, &[0x1001, 0x1041]));
        carrier.extend(packet(0x1001, false, b"mine"));
        carrier.extend(packet(0x1042, false, b"theirs"));
        carrier.extend(packet(0x11, false, b"sdt"));
        let kept = router.feed(&carrier);
        let pids: Vec<u16> = kept
            .as_chunks::<PACKET>()
            .0
            .iter()
            .map(|p| pid_of(p))
            .collect();
        // 自分の PAT、PMT、符号、共有のテーブルが残り、他のストリームの PAT は落ちる。
        assert_eq!(pids, vec![PID_PAT, 0x101, 0x1001, 0x11]);
        let mut router = Router::new(0x40f2);
        router.feed(&pat(0x48f3, &[(200, 0x101)]));
        assert!(router.feed(&packet(0x101, false, b"x")).is_empty());
    }

    #[test]
    fn a_gauge_tells_a_thinned_frame_from_a_whole_one() {
        let header = {
            let mut p = vec![0u8; PACKET];
            p[0] = SYNC;
            p[1] = 0x00;
            p[2] = 0x2f;
            p[3] = 0x10;
            p
        };
        assert!(isdb::tsmf::info(&header).is_none());
        let mut gauge = Gauge {
            headers: 10,
            packets_between: 9 * 23,
            ..Gauge::default()
        };
        assert!(gauge.thinned());
        gauge.packets_between = 9 * 52;
        assert!(!gauge.thinned());
    }
}
