// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 録画一本で、データ放送のプラグイン（wasm）を動かしてみる：
//! `dataplugin -- databroadcast.wasm 録画.ts [サービス番号] [出力.png]`。

use std::io::Read;

use zwcore::engine::{Engine, Out};
use zwcore::filters::Container;

fn message_of(out: Out) -> Option<(zwsdk::layer::Message, Vec<u8>)> {
    Some(match out {
        Out::DataStreams(streams) => (
            zwsdk::layer::Message::Streams(
                streams
                    .into_iter()
                    .map(|s| zwsdk::layer::DataStream {
                        pid: s.pid,
                        tag: s.tag,
                        coding: s.coding,
                        entry: s.entry,
                        auto_start: s.auto_start,
                        mmt: s.mmt,
                    })
                    .collect(),
            ),
            Vec::new(),
        ),
        Out::ApplicationService(application) => (
            zwsdk::layer::Message::Application(application.map(|a| {
                zwsdk::layer::ApplicationService {
                    resolution: a.resolution,
                    ait: a.ait,
                    data: a.data,
                    emts: a.emts,
                }
            })),
            Vec::new(),
        ),
        Out::Data { pid, section } => (zwsdk::layer::Message::Section { pid }, section),
        Out::Item {
            pid,
            mpu,
            item,
            fragmentation,
            data,
        } => (
            zwsdk::layer::Message::Item {
                pid,
                mpu,
                item,
                fragmentation,
            },
            data,
        ),
        _ => return None,
    })
}

/// 最後に描いた画面と、アルファで割った RGBA。
type Shown = Option<(zwsdk::layer::Frame, Vec<u8>)>;

enum Driver {
    Wasm(Box<zwsdk::host::LayerInstance>),
    Native(Box<zerowave_databroadcast::Engine>),
}

impl Driver {
    fn message(&mut self, message: &zwsdk::layer::Message, bytes: &[u8]) -> Result<(), String> {
        match self {
            Self::Wasm(data) => data.message(message, bytes),
            Self::Native(engine) => {
                engine.message(message.clone(), bytes);
                Ok(())
            }
        }
    }

    fn tick(&mut self) -> Result<zwsdk::layer::Ticked, String> {
        match self {
            Self::Wasm(data) => data.tick(),
            Self::Native(engine) => Ok(engine.tick()),
        }
    }

    fn frame(&mut self, area: (u32, u32)) -> Result<Option<Shown>, String> {
        match self {
            Self::Wasm(data) => Ok(data.frame(area, false)?.map(|painted| {
                painted.and_then(|(frame, picture)| match picture {
                    zwsdk::host::Picture::Rgba(rgba) => Some((frame, rgba)),
                    zwsdk::host::Picture::Sprites(_) => None,
                })
            })),
            Self::Native(engine) => Ok(engine.take_frame()),
        }
    }

    fn events(&mut self) -> Result<(Vec<zwsdk::layer::Event>, Vec<Vec<u8>>), String> {
        match self {
            Self::Wasm(data) => data.events(),
            Self::Native(engine) => Ok(engine.take_events()),
        }
    }
}

/// MMT のアプリは開いてから六秒後に d（準備中の d は無視される）。
fn press_after_open(
    data: &mut Driver,
    last: &Shown,
    pressed: &mut bool,
    opened_at: &mut Option<std::time::Instant>,
) {
    if last.is_none() {
        return;
    }
    if *pressed {
        opened_at.get_or_insert_with(std::time::Instant::now);
        return;
    }
    if std::env::var_os("ZW_WEB_OPEN").is_some() {
        // Hybridcast を直に開いたときは d を押さない（d は終了の操作）。
        *pressed = true;
        *opened_at = Some(std::time::Instant::now());
        return;
    }
    if opened_at.is_none() {
        *opened_at = Some(std::time::Instant::now());
    }
    if opened_at.is_some_and(|t| t.elapsed() > std::time::Duration::from_secs(8)) {
        *pressed = true;
        eprintln!("d を押す");
        data.message(
            &zwsdk::layer::Message::Key(zwsdk::layer::Key::DataButton),
            &[],
        )
        .expect("key");
    }
}

fn key_of(code: u16) -> Option<zwsdk::layer::Key> {
    Some(match code {
        38 => zwsdk::layer::Key::Up,
        40 => zwsdk::layer::Key::Down,
        37 => zwsdk::layer::Key::Left,
        39 => zwsdk::layer::Key::Right,
        13 => zwsdk::layer::Key::Enter,
        461 => zwsdk::layer::Key::Back,
        1001 => zwsdk::layer::Key::DataButton,
        403 => zwsdk::layer::Key::Red,
        404 => zwsdk::layer::Key::Green,
        405 => zwsdk::layer::Key::Yellow,
        406 => zwsdk::layer::Key::Blue,
        48..=57 => zwsdk::layer::Key::Digit((code - 48) as u8),
        _ => return None,
    })
}

fn write_png(path: &str, frame: &zwsdk::layer::Frame, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("書ける");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("ヘッダー");
    writer.write_image_data(rgba).expect("中身");
    eprintln!("→ {path}");
}

/// ZW_KEYS=39,0,37: 開いて三秒後から ZW_KEY_GAP 秒（既定 1）ごとに押す（0 は待つだけ）。
/// 押す前の画像を `出力-番号.png` に残す。録画を読んでいる途中でも押す（ZW_SPREAD と合わせる）。
struct Keys {
    keys: Vec<u16>,
    gap: f64,
    at: usize,
    stem: String,
}

impl Keys {
    fn press(
        &mut self,
        data: &mut Driver,
        last: &Shown,
        opened_at: Option<std::time::Instant>,
        pressed_at: &std::cell::Cell<Option<std::time::Instant>>,
    ) {
        let Some(opened) = opened_at else {
            return;
        };
        let due = ((opened.elapsed().as_secs_f64() - 3.0) / self.gap).floor();
        if due < 0.0 || due as usize >= self.keys.len() || self.at == due as usize {
            return;
        }
        self.at = due as usize;
        if let Some(key) = key_of(self.keys[self.at]) {
            if let Some((frame, rgba)) = last {
                write_png(&format!("{}-{:02}.png", self.stem, self.at), frame, rgba);
            }
            eprintln!("押す {key:?}");
            pressed_at.set(Some(std::time::Instant::now()));
            data.message(&zwsdk::layer::Message::Key(key), &[])
                .expect("key");
        }
    }
}

/// レンダリングする大きさ：`ZW_AREA=2940x1654`（本体が渡す物理画素）。無ければ `ZW_AREA_W`・`ZW_AREA_H`、既定 1920x1080。
fn area() -> (u32, u32) {
    if let Some((w, h)) = std::env::var("ZW_AREA").ok().and_then(|s| {
        s.split_once('x')
            .map(|(w, h)| (w.parse().ok(), h.parse().ok()))
    }) && let (Some(w), Some(h)) = (w, h)
    {
        return (w, h);
    }
    let get = |name: &str, or: u32| {
        std::env::var(name)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(or)
    };
    (get("ZW_AREA_W", 1920), get("ZW_AREA_H", 1080))
}

/// `ZW_TIMES=1`：画像が来た回ごとの tick・frame・events の ms。終わりにまとめる。
#[derive(Default)]
struct Times {
    on: bool,
    painted: Vec<(f64, f64, f64)>,
    idle: Vec<f64>,
}

impl Times {
    fn summary(&self) {
        if !self.on {
            return;
        }
        let stat = |mut v: Vec<f64>| -> String {
            if v.is_empty() {
                return String::from("-");
            }
            v.sort_by(f64::total_cmp);
            let mean = v.iter().sum::<f64>() / v.len() as f64;
            format!(
                "{} 回 中央 {:.1} 平均 {:.1} 最大 {:.1}",
                v.len(),
                v[v.len() / 2],
                mean,
                v[v.len() - 1]
            )
        };
        eprintln!(
            "まとめ tick（画像あり）{} / frame {} / events {} / tick（画像なし）{}",
            stat(self.painted.iter().map(|t| t.0).collect()),
            stat(self.painted.iter().map(|t| t.1).collect()),
            stat(self.painted.iter().map(|t| t.2).collect()),
            stat(self.idle.clone())
        );
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(plugin), Some(path)) = (args.next(), args.next()) else {
        eprintln!("使い方: dataplugin プラグイン.wasm 録画.ts [サービス番号] [出力.png]");
        std::process::exit(2);
    };
    let service: Option<u16> = args.next().and_then(|s| s.parse().ok());
    let out = args
        .next()
        .unwrap_or_else(|| String::from("dataplugin.png"));
    let network = std::env::var("ZW_NETWORK")
        .ok()
        .and_then(|s| u16::from_str_radix(&s, 16).ok())
        .unwrap_or(0);

    // ZW_NVRAM=nvram.txt: 本体の nvram.txt（`名前<TAB>16進`）を不揮発の記憶として渡す。
    let mut config = zwsdk::layer::Config::default();
    if let Ok(path) = std::env::var("ZW_NVRAM") {
        let text = std::fs::read_to_string(&path).expect("nvram.txt が読める");
        for line in text.lines() {
            let Some((name, hex)) = line.split_once('\t') else {
                continue;
            };
            let bytes = (0..hex.len() / 2)
                .filter_map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok())
                .collect();
            config.nvram.push((name.to_owned(), bytes));
        }
    }
    let began = std::time::Instant::now();
    let mut data = if std::env::var_os("ZW_NATIVE").is_some() {
        Driver::Native(Box::new(zerowave_databroadcast::Engine::new(&config)))
    } else {
        let engine = zwsdk::host::engine().expect("engine");
        let plugin =
            zwsdk::host::Plugin::load(&engine, std::path::Path::new(&plugin)).expect("load");
        eprintln!("ビルドした {:?}", began.elapsed());
        let logger: zwsdk::host::Logger = Box::new(|level, text| eprintln!("[{level:?}] {text}"));
        let mut io = zwsdk::host::Io::default();
        if std::env::var_os("ZW_TRACE").is_some() {
            io.cfg.insert(String::from("trace"), String::from("1"));
        }
        if std::env::var_os("ZW_TIMES").is_some() {
            io.cfg.insert(String::from("times"), String::from("1"));
        }
        if let Ok(code) = std::env::var("ZW_WEB_EVAL") {
            io.cfg.insert(String::from("web_eval"), code);
        }
        if let Ok(url) = std::env::var("ZW_WEB_OPEN") {
            io.cfg.insert(String::from("web_open"), url);
        }
        if let Ok(cas_id) = std::env::var("ZW_CAS_ID") {
            io.cfg.insert(String::from("card_id"), cas_id);
        }
        if let Ok(pairs) = std::env::var("ZW_STORAGE") {
            io.cfg.insert(String::from("web_storage"), pairs);
        }
        // フォント（`kind = "font"`）は本体と同じく設定のキーの名前で渡す。`ZW_CFG` で選ぶ。
        io.font_keys = plugin
            .manifest()
            .config
            .iter()
            .filter(|setting| setting.kind == "font")
            .map(|setting| setting.key.clone())
            .collect();
        // ほかの設定は `ZW_CFG`（`a.b=1|c=…`）で。UA に `;` が入るので区切りは `|`。
        if let Ok(pairs) = std::env::var("ZW_CFG") {
            for pair in pairs.split('|') {
                if let Some((key, value)) = pair.split_once('=') {
                    io.cfg.insert(key.to_owned(), value.to_owned());
                }
            }
        }
        let faces = web::fonts::Faces::find();
        let fonts = zwsdk::draw::font_context(&faces);
        if let Some(sans) = faces.sans {
            io.fonts.insert(String::from("sans"), sans);
        }
        if let Some(mono) = faces.mono {
            io.fonts.insert(String::from("mono"), mono);
        }
        Driver::Wasm(Box::new(
            plugin.layer(logger, io, fonts, &config).expect("layer"),
        ))
    };
    eprintln!("作った {:?}", began.elapsed());

    let mut file = std::fs::File::open(&path).expect("録画が開ける");
    let container = if path.ends_with(".tlv") || path.ends_with(".mmts") {
        Container::Tlv
    } else {
        Container::Ts
    };
    let mut demux = Engine::new(container, 48_000);
    if let Some(service) = service {
        demux.want(service);
    }
    let mut buffer = vec![0u8; 188 * 512];
    let mut pressed = false;
    let mut last: Shown = None;
    let mut elapsed_ms = 0.0;
    let mut told_broadcast = false;
    let mut opened_at: Option<std::time::Instant> = None;
    let mut keys = std::env::var("ZW_KEYS").ok().map(|s| Keys {
        keys: s.split(',').filter_map(|k| k.trim().parse().ok()).collect(),
        gap: std::env::var("ZW_KEY_GAP")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1.0),
        at: usize::MAX,
        stem: out.trim_end_matches(".png").to_owned(),
    });
    // ZW_SPREAD=秒: 録画をこの秒数に引き延ばして読む（モジュールが届く間合いを本物に近づける）。
    let spread: Option<f64> = std::env::var("ZW_SPREAD").ok().and_then(|s| s.parse().ok());
    let total = file.metadata().map_or(1, |m| m.len().max(1)) as f64;
    let mut done = 0f64;
    let reading = std::time::Instant::now();
    // ZW_CLOCK=1: 本体のように放送の時刻を一秒ごとに送る（録画の時刻ではなく、いまの時刻）。
    let clock = std::env::var_os("ZW_CLOCK").is_some();
    let mut clock_said = 0i64;
    let area = area();
    let pressed_at = std::cell::Cell::new(None::<std::time::Instant>);
    let mut times = Times {
        on: std::env::var_os("ZW_TIMES").is_some(),
        ..Default::default()
    };
    let mut pump = |data: &mut Driver, last: &mut Shown| {
        let began = std::time::Instant::now();
        let unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        if clock && unix != clock_said {
            clock_said = unix;
            data.message(
                &zwsdk::layer::Message::Clock {
                    unix,
                    recorded: false,
                },
                &[],
            )
            .expect("clock");
        }
        let ticked = data.tick().expect("tick");
        let ticked_for = began.elapsed();
        if ticked_for > std::time::Duration::from_millis(20) {
            eprintln!("tick {ticked_for:?}");
        }
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
        let mut painted = None;
        if ticked.dirty
            && let Some(frame) = data.frame(area).expect("frame")
        {
            let framed_for = began.elapsed() - ticked_for;
            painted = Some((ms(ticked_for), ms(framed_for)));
            if let Some(at) = pressed_at.take() {
                eprintln!("キーから画像まで {:.1} ms", ms(at.elapsed()));
            }
            eprintln!("frame を受け取るのに {framed_for:?}");
            match &frame {
                Some((frame, _)) => eprintln!(
                    "画像 {}x{} 文書 {:?} 映像だけ {} 待ち {}",
                    frame.width, frame.height, frame.size, frame.invisible, frame.waiting
                ),
                None => eprintln!("画像が消えた"),
            }
            *last = frame;
        }
        let asked = std::time::Instant::now();
        let (events, _audios) = data.events().expect("events");
        if times.on {
            let events_for = ms(asked.elapsed());
            match painted {
                Some((tick, frame)) => {
                    eprintln!("時間 tick {tick:.1} frame {frame:.1} events {events_for:.1} ms");
                    times.painted.push((tick, frame, events_for));
                }
                None => times.idle.push(ms(ticked_for)),
            }
        }
        for event in events {
            match event {
                zwsdk::layer::Event::Log(text) => eprintln!("  {text}"),
                other => eprintln!("  要求: {other:?}"),
            }
        }
    };
    loop {
        let read = file.read(&mut buffer).expect("読める");
        if read == 0 {
            break;
        }
        demux.push(&buffer[..read]);
        while let Some(out) = demux.next() {
            let streams = matches!(out, Out::DataStreams(_));
            if let Some((message, bytes)) = message_of(out) {
                data.message(&message, &bytes).expect("message");
            }
            if streams && !told_broadcast {
                told_broadcast = true;
                data.message(
                    &zwsdk::layer::Message::Broadcast(zwsdk::layer::Broadcast {
                        network,
                        transport: network,
                        // ZW_TOLD_SERVICE=<番号>: 本体が渡す番号を変える（取り違えの再現）。
                        service: std::env::var("ZW_TOLD_SERVICE")
                            .ok()
                            .and_then(|s| s.parse().ok())
                            .or(service)
                            .unwrap_or(0),
                        ..Default::default()
                    }),
                    &[],
                )
                .expect("broadcast");
                data.message(
                    &zwsdk::layer::Message::Area {
                        width: area.0,
                        height: area.1,
                    },
                    &[],
                )
                .expect("area");
            }
            if streams && !pressed && container == Container::Ts {
                pressed = true;
                data.message(
                    &zwsdk::layer::Message::Key(zwsdk::layer::Key::DataButton),
                    &[],
                )
                .expect("key");
            }
        }
        elapsed_ms += read as f64 * 8.0 / 17_000.0;
        pump(&mut data, &mut last);
        press_after_open(&mut data, &last, &mut pressed, &mut opened_at);
        if let Some(keys) = keys.as_mut() {
            keys.press(&mut data, &last, opened_at, &pressed_at);
        }
        done += read as f64;
        if let Some(spread) = spread {
            let due = std::time::Duration::from_secs_f64(spread * done / total);
            while reading.elapsed() < due {
                std::thread::sleep(std::time::Duration::from_millis(20));
                pump(&mut data, &mut last);
                if let Some(keys) = keys.as_mut() {
                    keys.press(&mut data, &last, opened_at, &pressed_at);
                }
            }
        }
    }
    eprintln!("読み終わり（{elapsed_ms:.0} ms ぶん）。");
    if let Some(seconds) = std::env::var("ZW_LINGER")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
    {
        let until = std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds);
        while std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(50));
            pump(&mut data, &mut last);
            press_after_open(&mut data, &last, &mut pressed, &mut opened_at);
            if let Some(keys) = keys.as_mut() {
                keys.press(&mut data, &last, opened_at, &pressed_at);
            }
        }
    }
    drop(pump);
    times.summary();
    if let Some((frame, rgba)) = last {
        write_png(&out, &frame, &rgba);
    } else {
        eprintln!("画像は無い");
    }
}
