// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 録画一本で、データ放送の文書を動かしてみる：`databroadcast -- 録画.ts [サービス番号] [キー…]`。
//! キーは `d`・`u/d/l/r`・`e`（決定）・`b`（戻る）・`1`〜`9`・`R/G/B/Y`。

use std::collections::BTreeMap;
use std::io::Read;

use bml::{Browser, Key, Request};
use mpegts::dsmcc::{Brought, Carousel};
use zwcore::filters::packet::Section;
use zwcore::filters::{Container, Feed, Kind, Program, Sink};

struct Watch {
    service: Option<u16>,
    tags: BTreeMap<u16, u16>,
    carousels: BTreeMap<u16, Carousel>,
    browser: Browser,
    packets: u64,
}

impl Watch {
    fn drain(&mut self) {
        for request in self.browser.take_requests() {
            match request {
                Request::Log(text) => println!(
                    "  [{:>5.1}s] {text}",
                    self.packets as f64 * 188.0 * 8.0 / 17e6
                ),
                other => println!("  要求: {other:?}"),
            }
        }
    }
}

impl Sink for Watch {
    fn on_streams(&mut self, program: &Program) {
        if self.service.is_some_and(|wanted| wanted != program.number) {
            return;
        }
        for stream in &program.streams {
            let (Kind::Data, Some(component), Some(tag)) = (stream.kind, &stream.data, stream.tag)
            else {
                continue;
            };
            if !isdb::bxml::is_bml(component.id) {
                continue;
            }
            if std::env::var_os("ZW_DATA_TRACE").is_some() && !self.tags.contains_key(&stream.id) {
                let info = isdb::bxml::additional_info(&component.additional);
                println!(
                    "コンポーネント pid 0x{:04x} tag 0x{tag:02x} coding 0x{:04x} {:?}",
                    stream.id, component.id, info
                );
            }
            self.tags.insert(stream.id, tag);
            self.browser
                .set_components(self.tags.values().copied().collect());
            if component.id == isdb::bxml::DATA_COMPONENT_C_PROFILE {
                self.browser.set_profile(bml::runtime::profile::Profile::C);
            }
            if let Some(info) = isdb::bxml::additional_info(&component.additional)
                && info.entry_point
            {
                self.browser.set_entry(tag, 0);
                // ZW_NETWORK=7fe6 でネットワークの番号を渡す（局が自分のネットワークを確かめる）。
                let network = std::env::var("ZW_NETWORK")
                    .ok()
                    .and_then(|s| u16::from_str_radix(&s, 16).ok())
                    .unwrap_or(0);
                self.browser.set_broadcast(bml::Broadcast {
                    service: program.number,
                    network,
                    transport: network,
                    ..Default::default()
                });
            }
        }
    }

    fn on_section(&mut self, section: &Section<'_>) {
        let (pid, section) = (section.pid, section.data);
        let Some(&tag) = self.tags.get(&pid) else {
            return;
        };
        let carousel = self.carousels.entry(pid).or_default();
        match carousel.push(section) {
            Some(Brought::Listed(dii)) => {
                self.browser
                    .listed(tag, dii.modules.iter().map(|m| (m.id, m.version)).collect());
                self.browser.data_event(tag, (dii.download_id >> 28) as u8);
            }
            Some(Brought::Module(module)) => {
                let described = isdb::bxml::describe(&module.info);
                let Some(data) = isdb::bxml::unpack(&described, &module.data) else {
                    return;
                };
                let files = isdb::bxml::files(described.content_type.as_deref(), &data);
                // ZW_DUMP=フォルダー: 届いたモジュールの中身を `フォルダー/コンポーネント/モジュール/名前` に書き出す。
                if let Ok(dump) = std::env::var("ZW_DUMP") {
                    let dir = std::path::Path::new(&dump)
                        .join(format!("{tag:02X}"))
                        .join(format!("{:04X}", module.id));
                    let _ = std::fs::create_dir_all(&dir);
                    for file in &files {
                        let name = if file.location.is_empty() {
                            "_"
                        } else {
                            file.location.as_str()
                        };
                        let _ = std::fs::write(dir.join(name), &file.body);
                    }
                }
                self.browser.arrived(tag, module.id, module.version, files);
                self.drain();
            }
            Some(Brought::Stream(section)) => {
                let (data_event, group, events) = isdb::bxml::events(&section);
                for event in events {
                    match event {
                        isdb::bxml::Event::General {
                            id,
                            time,
                            private_data,
                            ..
                        } => {
                            let npt = match time {
                                isdb::bxml::Timing::Immediate => None,
                                isdb::bxml::Timing::Npt(npt) => Some(npt),
                                _ => continue,
                            };
                            self.browser.event_message(
                                tag,
                                data_event,
                                group,
                                id,
                                npt,
                                &private_data,
                            );
                        }
                        isdb::bxml::Event::NptReference { npt, .. } => {
                            self.browser.npt_reference(npt);
                        }
                    }
                }
                self.drain();
            }
            None => {}
        }
    }
}

fn snapshot(browser: &mut Browser, path: &str) {
    let laid = browser.laid().clone();
    let (w, h) = (laid.size.0 as usize, laid.size.1 as usize);
    if w == 0 || h == 0 {
        return;
    }
    let mut rgba = vec![0u8; w * h * 4];
    let mut fill = |x0: f32, y0: f32, x1: f32, y1: f32, c: [u8; 4]| {
        for y in (y0.max(0.0) as usize)..(y1.min(h as f32) as usize) {
            for x in (x0.max(0.0) as usize)..(x1.min(w as f32) as usize) {
                let at = (y * w + x) * 4;
                let a = f32::from(c[3]) / 255.0;
                for k in 0..3 {
                    rgba[at + k] =
                        (f32::from(rgba[at + k]) * (1.0 - a) + f32::from(c[k]) * a) as u8;
                }
                rgba[at + 3] = 255;
            }
        }
    };
    let mut clip: Option<bml::document::layout::Rect> = None;
    for item in &laid.items {
        use bml::document::layout::Item;
        let bounded = |r: &bml::document::layout::Rect| match clip {
            Some(c) => r.intersect(&c),
            None => *r,
        };
        match item {
            Item::Clip(c) => clip = *c,
            Item::Fill { rect, colour } => {
                let r = bounded(rect);
                fill(r.x, r.y, r.right(), r.bottom(), *colour);
            }
            Item::Border {
                rect,
                width,
                colours,
            } => {
                let r = bounded(rect);
                fill(r.x, r.y, r.right(), r.y + width, colours[0]);
                fill(r.x, r.bottom() - width, r.right(), r.bottom(), colours[2]);
                fill(r.x, r.y, r.x + width, r.bottom(), colours[3]);
                fill(r.right() - width, r.y, r.right(), r.bottom(), colours[1]);
            }
            Item::Image { rect, .. } => {
                let r = bounded(rect);
                fill(r.x, r.y, r.right(), r.bottom(), [128, 128, 128, 160]);
            }
            Item::Video { rect } => {
                let r = bounded(rect);
                fill(r.x, r.y, r.right(), r.bottom(), [0, 0, 160, 255]);
            }
            Item::Text(run) => {
                for (dx, letter) in &run.glyphs {
                    let wide = bml::media::text::is_wide(*letter);
                    let width = if wide { run.size } else { run.size / 2.0 };
                    let x = run.x + dx;
                    let r = bounded(&bml::document::layout::Rect {
                        x: x + 1.0,
                        y: run.y + 1.0,
                        w: width - 2.0,
                        h: run.size - 2.0,
                    });
                    fill(r.x, r.y, r.right(), r.bottom(), run.colour);
                }
            }
        }
    }
    let file = std::fs::File::create(path).expect("書ける");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("ヘッダー");
    writer.write_image_data(&rgba).expect("中身");
    println!("  → {path}");
}

fn describe(browser: &mut Browser) {
    let invisible = browser.invisible();
    let laid = browser.laid();
    println!(
        "  置いたもの {} 個、大きさ {:?}、映像だけ {}",
        laid.items.len(),
        laid.size,
        invisible
    );
    for item in &laid.items {
        match item {
            bml::document::layout::Item::Text(run) => {
                let text: String = run.glyphs.iter().map(|(_, c)| *c).collect();
                println!(
                    "    文字 ({:.0},{:.0}) {}px {:?}",
                    run.x, run.y, run.size, text
                );
            }
            bml::document::layout::Item::Video { rect } => println!("    映像 {rect:?}"),
            bml::document::layout::Item::Image { rect, src } => println!("    画像 {rect:?} {src}"),
            _ => {}
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("使い方: databroadcast 録画.ts [サービス番号] [キー…]");
        std::process::exit(2);
    };
    let service = args.next().and_then(|s| s.parse().ok());
    let keys: Vec<String> = args.collect();
    let mut file = std::fs::File::open(&path).expect("録画が開ける");
    let mut feed = Feed::new(Container::Ts);
    let mut watch = Watch {
        service,
        tags: BTreeMap::new(),
        carousels: BTreeMap::new(),
        browser: Browser::new(),
        packets: 0,
    };
    watch.browser.set_wanted(true);
    let mut buffer = vec![0u8; 188 * 512];
    let mut elapsed_ms = 0.0;
    let mut pressed_d = false;
    loop {
        let read = file.read(&mut buffer).expect("読める");
        if read == 0 {
            break;
        }
        watch.packets += (read / 188) as u64;
        feed.push(&buffer[..read], &mut watch);
        elapsed_ms += read as f64 * 8.0 / 17_000.0;
        watch.browser.tick(elapsed_ms);
        watch.drain();
        // ZW_D_AT=秒: 読んでいる途中のこの時刻に d を押す。
        if let Some(at) = std::env::var("ZW_D_AT")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            && elapsed_ms >= at * 1000.0
            && !pressed_d
        {
            pressed_d = true;
            println!("  [{:5.1}s] キー DataButton", elapsed_ms / 1000.0);
            watch.browser.key(Key::DataButton);
            watch.drain();
        }
    }
    println!("読み終わり。");
    // ZW_LINGER=秒: 読み終わった後も時刻だけ進める（起動に一分待つ局のため）。
    if let Some(seconds) = std::env::var("ZW_LINGER")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
    {
        let until = elapsed_ms + seconds * 1000.0;
        while elapsed_ms < until {
            elapsed_ms += 100.0;
            watch.browser.tick(elapsed_ms);
            watch.drain();
        }
    }
    describe(&mut watch.browser);
    for key in keys {
        let key = match key.as_str() {
            "d" => Key::DataButton,
            "u" => Key::Up,
            "n" => Key::Down,
            "l" => Key::Left,
            "r" => Key::Right,
            "e" => Key::Enter,
            "b" => Key::Back,
            "R" => Key::Red,
            "G" => Key::Green,
            "B" => Key::Blue,
            "Y" => Key::Yellow,
            digit if digit.len() == 1 && digit.as_bytes()[0].is_ascii_digit() => {
                Key::Digit(digit.as_bytes()[0] - b'0')
            }
            _ => continue,
        };
        println!("キー {key:?}");
        watch.browser.key(key);
        for _ in 0..20 {
            elapsed_ms += 100.0;
            watch.browser.tick(elapsed_ms);
        }
        watch.drain();
        describe(&mut watch.browser);
        snapshot(&mut watch.browser, &format!("/tmp/bml-{}.png", key.code()));
    }
}
