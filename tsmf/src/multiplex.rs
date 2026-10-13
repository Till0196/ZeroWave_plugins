// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! Taking one stream out of a carrier that holds several (TSMF), and out of several
//! carriers that hold one (a split frame). [`Multiplex`] is a state machine with no
//! threads or sockets; [`Reception`] drives it from [`Tuner`]s on one thread.

mod route;

use std::collections::BTreeMap;
use std::io::{self, Read};

use isdb::bond::Bond;
use isdb::tsmf::{Frames, Splitter};

pub use isdb::tsmf::{Carriage, Group, Relative, SLOTS, Survey};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Take {
    #[default]
    Whole,
    Relative(u8, Carriage),
    /// The stream carrying a TSID. Channel files name this: relative numbers move.
    Stream(u16),
    /// The stream carrying a service. On a split frame only the first carrier is read,
    /// which is not enough for TLV: use [`Take::Stream`].
    Service(u16),
}

impl Take {
    pub fn of(stream: &Relative) -> Self {
        Self::Relative(stream.number, carriage_of(stream))
    }
}

fn carriage_of(stream: &Relative) -> Carriage {
    if stream.tlv {
        Carriage::Tlv
    } else {
        Carriage::Transport
    }
}

/// One stream out of one or more carriers. A split frame gives nothing until lined up.
#[derive(Debug)]
pub struct Multiplex {
    take: Take,
    carriers: usize,
    reading: Reading,
    /// Not kept for [`Take::Whole`]: it would walk every packet of a fast carrier for nothing.
    survey: Option<Survey>,
    chosen: Option<Relative>,
    multiplexed: bool,
    /// Held while deciding whether the carrier is multiplexed at all (most are not).
    held: Vec<u8>,
    gauge: route::Gauge,
    associations: route::Associations,
    since_header: usize,
    headers_seen: usize,
    lapsed: bool,
    single: bool,
}

/// Bytes without a header, once headers were seen, before the frames count as stopped.
/// DMirakurun can hand a few frames and then plain transport packets.
const LAPSED_AFTER: usize = 4 << 20;

/// Held before deciding there is no frame. Larger would delay low-rate radio by seconds.
const DECIDE_AFTER: usize = 64 * 1024;

#[derive(Debug)]
enum Reading {
    Whole,
    Waiting(u16),
    Seeking {
        service: u16,
        probes: BTreeMap<u8, Probe>,
    },
    One(Box<Splitter>),
    Routed(Box<route::Router>),
    Bonded {
        frames: Vec<Frames>,
        bond: Box<Bond>,
    },
}

fn reading_of(number: u8, carriage: Carriage, carriers: usize) -> Reading {
    if carriers == 1 {
        // One carrier has nothing to be lined up against; the bond would only add seconds.
        return Reading::One(Box::new(Splitter::of(number, carriage)));
    }
    Reading::Bonded {
        frames: (0..carriers).map(|_| Frames::new(number)).collect(),
        bond: Box::new(Bond::new(carriage)),
    }
}

impl Multiplex {
    pub fn new(take: Take) -> Self {
        Self::over(take, 1)
    }

    /// Reads a frame split across `carriers`. The order fed does not matter.
    pub fn over(take: Take, carriers: usize) -> Self {
        let carriers = carriers.max(1);
        let reading = match take {
            Take::Whole => Reading::Whole,
            Take::Relative(number, carriage) => reading_of(number, carriage, carriers),
            Take::Stream(stream_id) => Reading::Waiting(stream_id),
            Take::Service(service) => Reading::Seeking {
                service,
                probes: BTreeMap::new(),
            },
        };
        Self {
            take,
            carriers,
            reading,
            survey: (!matches!(take, Take::Whole)).then(Survey::new),
            chosen: None,
            multiplexed: false,
            held: Vec::new(),
            gauge: route::Gauge::default(),
            associations: route::Associations::default(),
            since_header: 0,
            headers_seen: 0,
            lapsed: false,
            single: false,
        }
    }

    /// Whether the frame carries one transport stream only; the carrier is passed whole.
    /// Taking such a frame apart waited on the PAT and sometimes gave no picture.
    pub fn single(&self) -> bool {
        self.single
    }

    fn settle_single(&mut self, arrived: &[u8]) -> Option<Vec<u8>> {
        if self.carriers != 1 || matches!(self.reading, Reading::Whole | Reading::Bonded { .. }) {
            return None;
        }
        let survey = self.survey.as_ref()?;
        // グループの指定があっても搬送波が一本なら分割ではない（ケーブルの多重化フレームはそう指定する）。
        if survey.frames() == 0 || survey.group().is_some_and(|group| group.carriers > 1) {
            return None;
        }
        let streams = survey.streams();
        let [only] = streams.as_slice() else {
            return None;
        };
        if only.tlv {
            return None;
        }
        self.single = true;
        self.chosen = Some(only.clone());
        self.reading = Reading::Whole;
        self.survey = None;
        let mut out = std::mem::take(&mut self.held);
        out.extend_from_slice(arrived);
        Some(out)
    }

    pub fn lapsed(&self) -> bool {
        self.lapsed
    }

    fn watch_lapse(&mut self, arrived: usize) {
        let Some(survey) = &self.survey else {
            return;
        };
        let frames = survey.frames();
        if frames != self.headers_seen {
            self.headers_seen = frames;
            self.since_header = 0;
            return;
        }
        if frames == 0 {
            return;
        }
        self.since_header += arrived;
        if self.since_header < LAPSED_AFTER {
            return;
        }
        self.lapsed = true;
        self.multiplexed = false;
        self.chosen = None;
        self.survey = None;
        self.held = Vec::new();
        self.reading = Reading::Whole;
    }

    pub fn taking(&self) -> Take {
        self.take
    }

    /// A PID two streams of a thinned frame both use. Those packets cannot be told apart;
    /// the way out is a Mirakurun that takes the frame apart first (`tsmfRelTs`).
    pub fn collision(&self) -> Option<u16> {
        match &self.reading {
            Reading::Routed(router) => router.collision(),
            _ => None,
        }
    }

    fn taking_number(&self) -> Option<u8> {
        match self.take {
            Take::Relative(number, _) => Some(number),
            _ => None,
        }
    }

    pub fn chosen(&self) -> Option<&Relative> {
        self.chosen.as_ref()
    }

    pub fn carriers(&self) -> usize {
        self.carriers
    }

    pub fn streams(&self) -> Vec<Relative> {
        self.survey
            .as_ref()
            .map(Survey::streams)
            .unwrap_or_default()
    }

    pub fn group(&self) -> Option<Group> {
        self.survey.as_ref().and_then(Survey::group)
    }

    /// Takes another stream of the same frame. The caller must restart its own reader.
    pub fn take(&mut self, take: Take) {
        let survey = self.survey.take();
        let carriers = self.carriers;
        let multiplexed = self.multiplexed;
        let gauge = std::mem::take(&mut self.gauge);
        let associations = std::mem::take(&mut self.associations);
        *self = Self::over(take, carriers);
        self.multiplexed = multiplexed;
        self.gauge = gauge;
        self.associations = associations;
        if let Some(survey) = survey {
            self.survey = Some(survey);
        }
        self.settle();
    }

    pub fn take_stream(&mut self, stream_id: u16) -> bool {
        if !self
            .streams()
            .iter()
            .any(|stream| stream.stream_id == stream_id)
        {
            // Refused rather than waited for: waiting would blank the stream that was playing.
            return false;
        }
        self.take(Take::Stream(stream_id));
        true
    }

    /// Cannot be refused until every stream has been read; nothing comes out until found.
    pub fn take_service(&mut self, service: u16) {
        self.take(Take::Service(service));
    }

    fn settle(&mut self) {
        let found = match &mut self.reading {
            Reading::Waiting(stream_id) => {
                let stream_id = *stream_id;
                self.survey
                    .as_ref()
                    .map(Survey::streams)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|stream| stream.stream_id == stream_id)
            }
            Reading::Seeking { service, probes } => {
                let streams = self
                    .survey
                    .as_ref()
                    .map(Survey::streams)
                    .unwrap_or_default();
                for stream in &streams {
                    probes
                        .entry(stream.number)
                        .or_insert_with(|| Probe::new(stream));
                }
                let service = *service;
                // The stream's own PAT says which it is, even when thinned. Probes stay for TLV.
                let by_table = self.associations.stream_of(service);
                streams.into_iter().find(|stream| {
                    if !stream.tlv {
                        return by_table == Some(stream.stream_id);
                    }
                    probes
                        .get(&stream.number)
                        .is_some_and(|probe| probe.carries(service))
                })
            }
            _ => return,
        };
        let Some(found) = found else {
            return;
        };
        self.reading = if !found.tlv && self.carriers == 1 && self.gauge.thinned() {
            Reading::Routed(Box::new(route::Router::new(found.stream_id)))
        } else {
            reading_of(found.number, carriage_of(&found), self.carriers)
        };
        self.chosen = Some(found);
    }

    fn reroute(&mut self) {
        if self.carriers != 1 || !self.gauge.thinned() {
            return;
        }
        if let Reading::One(_) = self.reading
            && let Some(chosen) = &self.chosen
            && !chosen.tlv
        {
            self.reading = Reading::Routed(Box::new(route::Router::new(chosen.stream_id)));
        }
    }

    pub fn multiplexed(&self) -> bool {
        self.multiplexed
    }

    fn unmultiplexed(&mut self, arrived: &[u8]) -> Option<Vec<u8>> {
        if self.carriers > 1 {
            return None;
        }
        if self.multiplexed {
            self.held = Vec::new();
            return None;
        }
        self.held.extend_from_slice(arrived);
        if self.held.len() < DECIDE_AFTER {
            return Some(Vec::new());
        }
        self.reading = Reading::Whole;
        self.survey = None;
        Some(std::mem::take(&mut self.held))
    }

    /// Services per stream. Only filled while a service is being looked for (for scans).
    pub fn services(&self) -> BTreeMap<u8, Vec<u16>> {
        match &self.reading {
            Reading::Seeking { probes, .. } => probes
                .iter()
                .map(|(number, probe)| (*number, probe.services()))
                .collect(),
            _ => BTreeMap::new(),
        }
    }

    /// Gives back the stream's share. Bytes from a carrier outside the group are dropped.
    pub fn feed(&mut self, carrier: usize, arrived: &[u8]) -> Vec<u8> {
        if carrier == 0 && self.survey.is_some() {
            let seen = {
                let survey = self.survey.as_mut().expect("just checked");
                survey.feed(arrived);
                survey.frames() > 0
            };
            self.multiplexed |= seen;
            self.watch_lapse(arrived.len());
            if self.lapsed {
                return arrived.to_vec();
            }
            if let Some(whole) = self.settle_single(arrived) {
                return whole;
            }
            self.gauge.feed(arrived);
            if matches!(self.reading, Reading::Seeking { .. }) {
                self.associations.feed(arrived);
            }
            self.reroute();
            // Looked for before taking apart, so the frame the header belongs to is not lost.
            self.settle();
            if self.chosen.is_none()
                && let Some(number) = self.taking_number()
            {
                self.chosen = self
                    .streams()
                    .into_iter()
                    .find(|stream| stream.number == number);
            }
        }
        if let Reading::Seeking { probes, .. } = &mut self.reading
            && carrier == 0
        {
            for probe in probes.values_mut() {
                probe.feed(arrived);
            }
            // These same bytes may be what settled it; they are handed to the stream below.
            self.settle();
        }
        if carrier == 0
            && matches!(self.reading, Reading::Waiting(_) | Reading::Seeking { .. })
            && let Some(loose) = self.unmultiplexed(arrived)
        {
            return loose;
        }
        match &mut self.reading {
            Reading::Whole => arrived.to_vec(),
            Reading::Waiting(_) | Reading::Seeking { .. } => Vec::new(),
            Reading::One(splitter) => splitter.feed(arrived),
            Reading::Routed(router) => router.feed(arrived),
            Reading::Bonded { frames, bond } => {
                let Some(frames) = frames.get_mut(carrier) else {
                    return Vec::new();
                };
                let whole = frames.feed(arrived);
                if whole.is_empty() {
                    return Vec::new();
                }
                bond.feed(whole)
            }
        }
    }
}

#[derive(Debug)]
struct Probe {
    splitter: Splitter,
    tables: Tables,
}

#[derive(Debug)]
enum Tables {
    Ts(Box<mpegts::pat::Reader>),
    Tlv(Box<TlvServices>),
}

/// The services an MMT/TLV stream names in its package tables. Reads the signalling only.
#[derive(Debug, Default)]
struct TlvServices {
    packets: mmttlv::tlv::Feed,
    signalling: mmttlv::signalling::Signalling,
    found: Vec<u16>,
}

impl TlvServices {
    fn feed(&mut self, bytes: &[u8]) {
        use isdb::mmttables::{TABLE_MPT_COMPLETE, TABLE_MPT_SUBSET_FIRST, package_table};
        use mmttlv::tlv::Piece;
        for piece in self.packets.feed(bytes) {
            let Piece::Mmtp {
                packet_id,
                payload_type,
                payload,
                scrambled: false,
                ..
            } = piece
            else {
                continue;
            };
            if payload_type != mmttlv::mmtp::SIGNALLING {
                continue;
            }
            for message in self.signalling.push(packet_id, &payload) {
                if message.id != isdb::mh::MESSAGE_PA {
                    continue;
                }
                let payload = &message.payload;
                let Some(&tables) = payload.first() else {
                    continue;
                };
                let mut at = 1 + 4 * usize::from(tables);
                while at + 4 <= payload.len() {
                    let length = usize::from(payload[at + 2]) << 8 | usize::from(payload[at + 3]);
                    if at + 4 + length > payload.len() {
                        break;
                    }
                    if (TABLE_MPT_SUBSET_FIRST..=TABLE_MPT_COMPLETE).contains(&payload[at])
                        && let Some(table) = package_table(&payload[at..at + 4 + length])
                    {
                        let service = table.service_id();
                        if !self.found.contains(&service) {
                            self.found.push(service);
                        }
                    }
                    at += 4 + length;
                }
            }
        }
    }
}

impl Probe {
    fn new(stream: &Relative) -> Self {
        Self {
            splitter: Splitter::of(stream.number, carriage_of(stream)),
            tables: if stream.tlv {
                Tables::Tlv(Box::default())
            } else {
                Tables::Ts(Box::new(mpegts::pat::Reader::new()))
            },
        }
    }

    fn feed(&mut self, arrived: &[u8]) {
        let mine = self.splitter.feed(arrived);
        if mine.is_empty() {
            return;
        }
        match &mut self.tables {
            Tables::Ts(reader) => reader.feed(&mine),
            Tables::Tlv(services) => services.feed(&mine),
        }
    }

    fn services(&self) -> Vec<u16> {
        match &self.tables {
            Tables::Ts(reader) => reader
                .programs()
                .iter()
                .filter(|program| program.number != 0)
                .map(|program| program.number)
                .collect(),
            Tables::Tlv(services) => services.found.clone(),
        }
    }

    fn carries(&self, service: u16) -> bool {
        self.services().contains(&service)
    }
}

/// Where a carrier's bytes come from. [`Reception::open`] gives up on the first that fails.
pub trait Tuner {
    type Carrier: Read;
    type Error;

    fn carriers(&self) -> usize {
        1
    }

    /// Opens one, from nought. One tuner cannot serve two carriers at once.
    fn open(&mut self, carrier: usize) -> Result<Self::Carrier, Self::Error>;
}

const READ_SIZE: usize = 64 * 1024;

/// Receives a stream from tuners on one thread. Per-carrier threads use [`Multiplex::feed`].
pub struct Reception<T: Tuner> {
    tuned: Vec<T::Carrier>,
    multiplex: Multiplex,
    buffer: Vec<u8>,
    ended: bool,
}

impl<T: Tuner> std::fmt::Debug for Reception<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Reception")
            .field("carriers", &self.tuned.len())
            .field("multiplex", &self.multiplex)
            .finish_non_exhaustive()
    }
}

impl<T: Tuner> Reception<T> {
    pub fn open(tuner: &mut T, take: Take) -> Result<Self, T::Error> {
        let carriers = tuner.carriers().max(1);
        let mut tuned = Vec::with_capacity(carriers);
        for carrier in 0..carriers {
            tuned.push(tuner.open(carrier)?);
        }
        Ok(Self {
            tuned,
            multiplex: Multiplex::over(take, carriers),
            buffer: vec![0u8; READ_SIZE],
            ended: false,
        })
    }

    pub fn multiplex(&mut self) -> &mut Multiplex {
        &mut self.multiplex
    }

    pub fn ended(&self) -> bool {
        self.ended
    }

    /// Asks every carrier once (the bond needs them to move together).
    /// An empty answer is not the end; [`Self::ended`] is.
    pub fn read(&mut self) -> io::Result<Vec<u8>> {
        let mut kept = Vec::new();
        let mut sending = false;
        for carrier in 0..self.tuned.len() {
            let read = match self.tuned[carrier].read(&mut self.buffer) {
                Ok(0) => continue,
                Ok(read) => read,
                // A read that timed out is a carrier gone quiet, not a stream that has ended.
                Err(reason)
                    if matches!(
                        reason.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    sending = true;
                    continue;
                }
                Err(reason) => return Err(reason),
            };
            sending = true;
            let arrived = self.multiplex.feed(carrier, &self.buffer[..read]);
            kept.extend_from_slice(&arrived);
        }
        self.ended = !sending;
        Ok(kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKET: usize = 188;
    const SYNC: u8 = 0x47;

    fn crc32(bytes: &[u8]) -> u32 {
        let mut register: u32 = 0xFFFF_FFFF;
        for byte in bytes {
            register ^= u32::from(*byte) << 24;
            for _ in 0..8 {
                register = if register & 0x8000_0000 != 0 {
                    (register << 1) ^ 0x04C1_1DB7
                } else {
                    register << 1
                };
            }
        }
        register
    }

    fn slot_packet(mark: u8) -> Vec<u8> {
        let mut packet = vec![0u8; PACKET];
        packet[0] = SYNC;
        packet[1] = 0x01;
        packet[3] = 0x10;
        packet[4] = mark;
        packet
    }

    fn header(slots: &[u8; SLOTS]) -> Vec<u8> {
        let mut packet = vec![0u8; PACKET];
        packet[0] = SYNC;
        packet[2] = 0x2f;
        packet[3] = 0x10;
        let payload = &mut packet[4..];
        payload[0] = 0x1a;
        payload[1] = 0x86;
        for (slot, relative) in slots.iter().enumerate() {
            let at = 69 + slot / 2;
            if slot % 2 == 0 {
                payload[at] = (payload[at] & 0x0f) | (relative << 4);
            } else {
                payload[at] = (payload[at] & 0xf0) | (relative & 0x0f);
            }
        }
        for number in 1..=15usize {
            let at = 5 + (number - 1) * 4;
            payload[at] = 0x40;
            payload[at + 1] = number as u8;
            payload[at + 2] = 0x00;
            payload[at + 3] = 0x04;
        }
        payload[121] = 0xff;
        payload[122] = 0xfe;
        let check = crc32(&payload[..180]);
        payload[180..184].copy_from_slice(&check.to_be_bytes());
        packet
    }

    fn tlv_header(slots: &[u8; SLOTS]) -> Vec<u8> {
        let mut packet = header(slots);
        let payload = &mut packet[4..];
        payload[121] = 0x00;
        payload[122] = 0x00;
        let check = crc32(&payload[..180]);
        payload[180..184].copy_from_slice(&check.to_be_bytes());
        packet
    }

    fn tlv_packet(mark: u8, starts: bool) -> Vec<u8> {
        let mut packet = vec![mark; PACKET];
        packet[0] = SYNC;
        packet[1] = if starts { 0x40 } else { 0x00 };
        packet[2] = 0x2d;
        packet[3] = 0x10;
        packet
    }

    fn frame(slots: &[u8; SLOTS]) -> Vec<u8> {
        let mut bytes = header(slots);
        for slot in slots {
            bytes.extend_from_slice(&slot_packet(*slot));
        }
        bytes
    }

    fn shared() -> [u8; SLOTS] {
        let mut slots = [0u8; SLOTS];
        for (at, slot) in slots.iter_mut().enumerate() {
            *slot = if at % 2 == 0 { 1 } else { 2 };
        }
        slots
    }

    #[derive(Debug)]
    struct Canned {
        carriers: Vec<Vec<u8>>,
        opened: Vec<usize>,
    }

    impl Tuner for Canned {
        type Carrier = io::Cursor<Vec<u8>>;
        type Error = io::Error;

        fn carriers(&self) -> usize {
            self.carriers.len()
        }

        fn open(&mut self, carrier: usize) -> io::Result<Self::Carrier> {
            self.opened.push(carrier);
            Ok(io::Cursor::new(self.carriers[carrier].clone()))
        }
    }

    #[test]
    fn a_frame_of_one_stream_is_passed_whole() {
        let one = [1u8; SLOTS];
        let mut bytes = Vec::new();
        for _ in 0..4 {
            bytes.extend_from_slice(&frame(&one));
        }
        for take in [
            Take::Service(1024),
            Take::Stream(1),
            Take::Relative(1, Carriage::Transport),
        ] {
            let mut multiplex = Multiplex::new(take);
            let mut out = Vec::new();
            for chunk in bytes.chunks(PACKET * 20) {
                out.extend(multiplex.feed(0, chunk));
            }
            assert!(multiplex.single(), "{take:?}");
            assert_eq!(out, bytes, "{take:?}: そのまま、先頭から");
        }
        let mut multiplex = Multiplex::new(Take::Relative(2, Carriage::Transport));
        let _ = multiplex.feed(0, &frame(&shared()));
        assert!(!multiplex.single());
    }

    #[test]
    fn only_the_chosen_streams_slots_come_out() {
        let mut multiplex = Multiplex::new(Take::Relative(2, Carriage::Transport));
        let kept = multiplex.feed(0, &frame(&shared()));
        assert_eq!(kept.len(), 26 * PACKET);
        for packet in kept.chunks(PACKET) {
            assert_eq!(packet[4], 2, "a slot of the other stream came out");
        }
    }

    #[test]
    fn the_streams_of_the_carrier_are_named_while_one_of_them_plays() {
        let mut multiplex = Multiplex::new(Take::Relative(1, Carriage::Transport));
        multiplex.feed(0, &frame(&shared()));
        let streams = multiplex.streams();
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].number, 1);
        assert_eq!(streams[0].stream_id, 0x4001);
        assert_eq!(streams[0].slots, 26);
        assert_eq!(streams[1].stream_id, 0x4002);
        assert!(!streams[1].tlv);
    }

    #[test]
    fn another_stream_of_the_frame_is_taken_without_retuning() {
        let mut multiplex = Multiplex::new(Take::Relative(1, Carriage::Transport));
        assert_eq!(multiplex.feed(0, &frame(&shared())).len(), 26 * PACKET);
        assert!(multiplex.take_stream(0x4002), "the frame holds that stream");
        assert_eq!(multiplex.taking(), Take::Stream(0x4002));
        assert_eq!(
            multiplex.chosen().map(|stream| stream.number),
            Some(2),
            "the frame had already named it, so nothing is waited for"
        );
        assert_eq!(
            multiplex.streams().len(),
            2,
            "the list survived the change of stream"
        );
        let kept = multiplex.feed(0, &frame(&shared()));
        assert_eq!(kept.len(), 26 * PACKET);
        assert!(kept.chunks(PACKET).all(|packet| packet[4] == 2));
    }

    #[test]
    fn a_stream_the_frame_does_not_hold_is_not_taken() {
        let mut multiplex = Multiplex::new(Take::Relative(1, Carriage::Transport));
        multiplex.feed(0, &frame(&shared()));
        assert!(!multiplex.take_stream(0x4009));
        assert_eq!(multiplex.taking(), Take::Relative(1, Carriage::Transport));
    }

    #[test]
    fn a_stream_is_taken_by_its_identifier() {
        let mut multiplex = Multiplex::new(Take::Stream(0x4002));
        assert!(multiplex.chosen().is_none());
        assert!(multiplex.feed(0, &slot_packet(2)).is_empty());
        let kept = multiplex.feed(0, &frame(&shared()));
        assert_eq!(
            multiplex.chosen().map(|stream| stream.number),
            Some(2),
            "the header named it"
        );
        assert_eq!(kept.len(), 26 * PACKET, "the frame that named it is kept");
        assert!(kept.chunks(PACKET).all(|packet| packet[4] == 2));
    }

    #[test]
    fn an_identifier_the_carrier_does_not_hold_gives_nothing() {
        let mut multiplex = Multiplex::new(Take::Stream(0x4009));
        assert!(multiplex.feed(0, &frame(&shared())).is_empty());
        assert!(multiplex.chosen().is_none());
        assert_eq!(multiplex.streams().len(), 2, "what is here was still read");
    }

    #[test]
    fn the_carriage_of_a_stream_taken_by_identifier_comes_from_the_frame() {
        let mut slots = [0u8; SLOTS];
        slots[0] = 1;
        slots[1] = 1;
        let mut bytes = tlv_header(&slots);
        bytes.extend_from_slice(&tlv_packet(0xa1, true));
        bytes.extend_from_slice(&tlv_packet(0xa2, false));
        for _ in 2..SLOTS {
            bytes.extend_from_slice(&slot_packet(0));
        }
        let mut multiplex = Multiplex::new(Take::Stream(0x4001));
        let kept = multiplex.feed(0, &bytes);
        assert!(multiplex.chosen().is_some_and(|stream| stream.tlv));
        assert_eq!(kept.len(), 184 + 185);
    }

    fn pat_packet(stream_id: u16, service: u16) -> Vec<u8> {
        let mut section = vec![0x00u8, 0xb0, 0x0d];
        section.extend_from_slice(&stream_id.to_be_bytes());
        section.extend_from_slice(&[0xc1, 0x00, 0x00]);
        section.extend_from_slice(&service.to_be_bytes());
        section.extend_from_slice(&[0xe1, 0x00]);
        let check = crc32(&section);
        section.extend_from_slice(&check.to_be_bytes());

        let mut packet = vec![0xffu8; PACKET];
        packet[0] = SYNC;
        packet[1] = 0x40;
        packet[2] = 0x00;
        packet[3] = 0x10;
        packet[4] = 0x00;
        packet[5..5 + section.len()].copy_from_slice(&section);
        packet
    }

    fn frame_of_tables(slots: &[u8; SLOTS]) -> Vec<u8> {
        let mut bytes = header(slots);
        for slot in slots {
            bytes.extend_from_slice(&if *slot == 0 {
                slot_packet(0)
            } else {
                pat_packet(0x4000 + u16::from(*slot), 0x0400 + u16::from(*slot))
            });
        }
        bytes
    }

    #[test]
    fn a_stream_is_taken_by_a_service_on_it() {
        let mut multiplex = Multiplex::new(Take::Service(0x0402));
        let bytes = frame_of_tables(&shared());
        assert_eq!(multiplex.feed(0, &bytes).len(), 26 * PACKET);
        assert_eq!(
            multiplex.chosen().map(|stream| stream.number),
            Some(2),
            "the second stream's table named the service"
        );
        assert_eq!(
            multiplex.chosen().map(|stream| stream.stream_id),
            Some(0x4002)
        );
        assert_eq!(multiplex.feed(0, &bytes).len(), 26 * PACKET);
    }

    #[test]
    fn the_services_of_every_stream_are_read_while_one_is_looked_for() {
        let mut multiplex = Multiplex::new(Take::Service(0x0409));
        multiplex.feed(0, &frame_of_tables(&shared()));
        assert!(multiplex.chosen().is_none());
        let services = multiplex.services();
        assert_eq!(services.get(&1), Some(&vec![0x0401]));
        assert_eq!(services.get(&2), Some(&vec![0x0402]));
    }

    #[test]
    fn the_probes_are_dropped_once_a_stream_is_chosen() {
        let mut multiplex = Multiplex::new(Take::Service(0x0401));
        multiplex.feed(0, &frame_of_tables(&shared()));
        assert_eq!(multiplex.chosen().map(|stream| stream.number), Some(1));
        assert!(multiplex.services().is_empty());
    }

    #[test]
    fn a_carrier_with_no_frame_is_handed_on_even_when_a_stream_was_asked_for() {
        let mut multiplex = Multiplex::new(Take::Stream(0x4002));
        let mut fed = Vec::new();
        let mut kept = Vec::new();
        while fed.len() < DECIDE_AFTER + PACKET {
            let packet = slot_packet(7);
            fed.extend_from_slice(&packet);
            kept.extend(multiplex.feed(0, &packet));
        }
        assert!(!multiplex.multiplexed());
        assert_eq!(kept, fed, "the carrier is handed on from its beginning");
        let more = slot_packet(7);
        assert_eq!(multiplex.feed(0, &more), more);
    }

    #[test]
    fn a_carrier_whose_frames_stop_is_handed_on_whole_from_there() {
        let mut multiplex = Multiplex::new(Take::Service(0x0999));
        let bytes = frame(&shared());
        assert!(multiplex.feed(0, &bytes).is_empty());
        assert!(multiplex.multiplexed());
        let mut fed = 0;
        let mut out = 0;
        let packet = slot_packet(7);
        while fed < LAPSED_AFTER + PACKET {
            fed += PACKET;
            out += multiplex.feed(0, &packet).len();
        }
        assert!(multiplex.lapsed());
        assert!(!multiplex.multiplexed());
        assert!(out > 0, "what arrived after giving up comes out");
        assert_eq!(multiplex.feed(0, &packet), packet);
    }

    #[test]
    fn a_service_on_a_carrier_with_no_frame_is_left_to_what_reads_the_stream() {
        let mut multiplex = Multiplex::new(Take::Service(0x0400));
        let mut kept = Vec::new();
        let mut fed = 0;
        while fed < DECIDE_AFTER + PACKET {
            let packet = pat_packet(0x4001, 0x0401);
            fed += packet.len();
            kept.extend(multiplex.feed(0, &packet));
        }
        assert_eq!(kept.len(), fed);
        assert!(multiplex.chosen().is_none(), "no frame named anything");
    }

    #[test]
    fn a_frame_arriving_while_waiting_settles_it() {
        let mut multiplex = Multiplex::new(Take::Stream(0x4002));
        assert!(multiplex.feed(0, &slot_packet(2)).is_empty());
        let kept = multiplex.feed(0, &frame(&shared()));
        assert!(multiplex.multiplexed());
        assert_eq!(kept.len(), 26 * PACKET);
    }

    #[test]
    fn a_carrier_with_no_frame_is_handed_on_as_it_is() {
        let mut multiplex = Multiplex::new(Take::Whole);
        let bytes = slot_packet(9);
        assert_eq!(multiplex.feed(0, &bytes), bytes);
        assert!(multiplex.streams().is_empty());
    }

    #[test]
    fn every_carrier_of_a_split_frame_is_tuned() {
        let mut tuner = Canned {
            carriers: vec![frame(&shared()), frame(&shared()), frame(&shared())],
            opened: Vec::new(),
        };
        let mut reception =
            Reception::open(&mut tuner, Take::Relative(1, Carriage::Transport)).expect("opens");
        assert_eq!(reception.multiplex().carriers(), 3);
        let _ = reception.read().expect("reads");
        assert_eq!(tuner.opened, vec![0, 1, 2]);
    }

    #[test]
    fn one_carrier_comes_straight_back() {
        let mut tuner = Canned {
            carriers: vec![frame(&shared())],
            opened: Vec::new(),
        };
        let mut reception =
            Reception::open(&mut tuner, Take::Relative(2, Carriage::Transport)).expect("opens");
        assert_eq!(reception.read().expect("reads").len(), 26 * PACKET);
        assert!(!reception.ended());
        assert!(reception.read().expect("reads").is_empty());
        assert!(reception.ended());
    }
}
