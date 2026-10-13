// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 展開した BML を開いて Blitz でレンダリングし PNG
//! に：`bmlsnap -- 展開先/<サービス> <コンポーネント> 出力.png [キー…]`。

use std::collections::BTreeMap;
use std::sync::Arc;

use bml::{Browser, Key, Request};

type Modules = Vec<(u16, Vec<bml::resource::File>)>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, component, out, keys @ ..] = args.as_slice() else {
        eprintln!("使い方: bmlsnap <展開先/サービス> <コンポーネントの十六進> <出力.png> [キー…]");
        std::process::exit(2);
    };
    let component = u16::from_str_radix(component, 16).expect("コンポーネントは十六進");
    // フォントはプラグインと同じに選ぶ。太字のレンダリングのしかたが決まるので、文書を HTML にする前に。
    web::fonts::set_bml(web::fonts::choose::load());
    let mut browser = Browser::new();
    if std::env::var("ZW_PROFILE").is_ok_and(|p| p.eq_ignore_ascii_case("c")) {
        browser.set_profile(bml::runtime::profile::Profile::C);
    }
    // ZW_NETWORK・ZW_TRANSPORT・ZW_SERVICE・ZW_EVENT は十六進（局が自分のネットワークを確かめる）。
    let hex = |name: &str, or: u16| {
        std::env::var(name)
            .ok()
            .and_then(|s| u16::from_str_radix(&s, 16).ok())
            .unwrap_or(or)
    };
    let network = hex("ZW_NETWORK", 0x7fe1);
    browser.set_broadcast(bml::Broadcast {
        network,
        transport: hex("ZW_TRANSPORT", network),
        service: hex("ZW_SERVICE", 1024),
        event: hex("ZW_EVENT", 1),
        start: Some(1_757_500_000),
        duration: Some(3600),
        name: String::from("試しの番組"),
        captions: false,
        next: None,
        audio: None,
        caption_shown: 0,
    });
    let mut components: Vec<(u16, Modules)> = Vec::new();
    for comp in std::fs::read_dir(root).expect("サービスのディレクトリ") {
        let comp = comp.expect("読める");
        let Ok(tag) = u16::from_str_radix(&comp.file_name().to_string_lossy(), 16) else {
            continue;
        };
        let mut modules: Vec<(u16, Vec<bml::resource::File>)> = Vec::new();
        for entry in std::fs::read_dir(comp.path()).expect("コンポーネントのディレクトリ")
        {
            let entry = entry.expect("読める");
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(module) = u16::from_str_radix(&name, 16) else {
                continue;
            };
            let mut files = Vec::new();
            for file in std::fs::read_dir(entry.path()).expect("モジュールのディレクトリ")
            {
                let file = file.expect("読める");
                let location = file.file_name().to_string_lossy().into_owned();
                if location == "module" {
                    continue;
                }
                let body = std::fs::read(file.path()).expect("読める");
                files.push(bml::resource::File {
                    content_type: web::store::mime_of(&location).to_owned(),
                    location,
                    body,
                });
            }
            modules.push((module, files));
        }
        modules.sort_by_key(|(id, _)| *id);
        components.push((tag, modules));
    }
    browser.set_components(components.iter().map(|(tag, _)| *tag).collect());
    for (tag, modules) in &components {
        browser.listed(*tag, modules.iter().map(|(id, _)| (*id, 1u8)).collect());
    }
    // 最初の文書の script が他のモジュールのテーブルを読むので、全部入れてから開く。
    for (tag, modules) in components {
        for (id, files) in modules {
            browser.arrived(tag, id, 1, files);
        }
    }
    browser.set_entry(component, hex("ZW_MODULE", 0));
    browser.set_wanted(true);
    let mut now = 0.0;
    let drain = |browser: &mut Browser| {
        for request in browser.take_requests() {
            match request {
                Request::Log(text) => eprintln!("  {text}"),
                other => eprintln!("  要求: {other:?}"),
            }
        }
    };
    drain(&mut browser);
    let settle_ticks: usize = std::env::var("ZW_SETTLE")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .map_or(30, |s| (s * 10.0) as usize);
    let settle = |browser: &mut Browser, now: &mut f64| {
        for _ in 0..settle_ticks {
            *now += 100.0;
            browser.tick(*now);
        }
    };
    settle(&mut browser, &mut now);
    drain(&mut browser);
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
            other => {
                eprintln!("知らないキー: {other}");
                continue;
            }
        };
        browser.key(key);
        settle(&mut browser, &mut now);
        drain(&mut browser);
    }
    // ZW_BML_EVAL=式: 文書の中で実行して結果を出す。
    if let Ok(code) = std::env::var("ZW_BML_EVAL") {
        browser.eval(&format!("__bml_log(String({code}));"));
        drain(&mut browser);
        settle(&mut browser, &mut now);
        drain(&mut browser);
        if let Ok(code) = std::env::var("ZW_BML_EVAL_AFTER") {
            browser.eval(&format!("__bml_log(String({code}));"));
            drain(&mut browser);
            settle(&mut browser, &mut now);
            drain(&mut browser);
        }
        if let Ok(keys) = std::env::var("ZW_KEYS_AFTER") {
            for key in keys.split(',') {
                let key = match key {
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
                    _ => continue,
                };
                eprintln!("  キー {key:?}");
                browser.key(key);
                settle(&mut browser, &mut now);
                drain(&mut browser);
            }
        }
    }
    let Some(html) = browser.html() else {
        eprintln!("文書が開いていない");
        std::process::exit(1);
    };
    if std::env::var_os("ZW_DUMP").is_some() {
        println!("{}", html.markup);
    }
    let clut = browser.clut();
    let mut files = BTreeMap::new();
    for name in &html.images {
        if let Some(made) = html.made.get(name) {
            files.insert(name.clone(), made.clone());
            continue;
        }
        let file_name = name.split_once('#').map_or(name.as_str(), |(n, _)| n);
        if let Some(file) = browser.file(file_name)
            && let Some(picture) = web::picture::decode(&file.content_type, &file.body, name, &clut)
        {
            files.insert(name.clone(), picture.png());
        } else {
            eprintln!("画像が無い: {name}");
        }
    }
    let mut faces = web::fonts::Faces::find();
    if let Some(mono) = web::fonts::choose::mono() {
        faces.mono = Some(mono);
    }
    eprintln!("フォント {} 本", faces.len());
    let scale: f64 = std::env::var("ZW_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let mut still = web::still::Still::open(
        &html.markup,
        html.size,
        scale,
        web::still::Files(Arc::new(files)),
        &faces,
    );
    eprintln!("映像 {:?}", still.video_rect());
    let (buffer, w, h) = still.paint();
    let file = std::fs::File::create(out).expect("書ける");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("ヘッダー");
    writer.write_image_data(&buffer).expect("中身");
    eprintln!("→ {out} ({w}x{h})");
}
