// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 取り出したアプリを開いて PNG に：`snapshot -- フォルダー パス 出力.png [秒]`。

use std::time::{Duration, Instant};

use web::api::Host;
use web::app::App;
use web::net::Shared;
use web::store::Store;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, path, out, rest @ ..] = args.as_slice() else {
        eprintln!("使い方: snapshot <フォルダー> <パス> <出力.png> [秒]");
        std::process::exit(2);
    };
    let seconds: f64 = rest.first().and_then(|s| s.parse().ok()).unwrap_or(3.0);
    let scale: f64 = rest.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.25);
    let store = Store::from_directory(std::path::Path::new(root)).expect("読める");
    eprintln!("フォルダーに {} ファイル", store.len());
    // ZW_NETWORK・ZW_SERVICE で局を差し替える（通信のアプリは局で振る舞いを変える）。
    let network = std::env::var("ZW_NETWORK")
        .ok()
        .and_then(|s| u16::from_str_radix(&s, 16).ok())
        .unwrap_or(0x0004);
    let service = std::env::var("ZW_SERVICE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(101);
    let host = Host {
        network,
        tlv_stream: network,
        service,
        event: Some(1),
        start: Some(1_757_500_000),
        duration: Some(3600),
        name: String::from("試しの番組"),
        description: String::new(),
        zipcode: String::from("1000001"),
        cas_id: std::env::var("ZW_CAS_ID").unwrap_or_default(),
        profile: std::env::var("ZW_PROFILE").unwrap_or_else(|_| String::from("sh4")),
        captions: vec![web::api::Caption {
            tag: 0x30,
            language: String::from("jpn"),
            tmd: 0b0010,
            resolution: 1,
            ..Default::default()
        }],
        next: Some((
            2,
            Some(1_757_503_600),
            Some(1800),
            String::from("次の試しの番組"),
        )),
    };
    // ZW_STORAGE="greg33=0;…": 放送側が残した記憶（Ureg/Greg、localStorage）を先に置く。
    if let Some(pairs) = std::env::var_os("ZW_STORAGE") {
        web::api::kept(|kept| {
            for pair in pairs.to_string_lossy().split(';') {
                if let Some((key, value)) = pair.split_once('=') {
                    kept.storage.insert(key.to_owned(), value.to_owned());
                }
            }
        });
    }
    if let Some(seconds) = std::env::var("ZW_CLOCK").ok().and_then(|s| s.parse().ok()) {
        web::api::date::set_clock(seconds);
    }
    let began = Instant::now();
    let faces = web::fonts::Faces::find();
    eprintln!("フォント {} 本", faces.len());
    let Some(mut app) = App::open(Shared::new(store), path, host, (3840, 2160), scale, &faces)
    else {
        eprintln!("開けない: {path}");
        std::process::exit(1);
    };
    eprintln!("開いた {:?}", began.elapsed());
    // ZW_PRE=式: 開いた直後（タイマーが動く前）に実行する。
    if let Some(code) = std::env::var_os("ZW_PRE") {
        app.eval(&code.to_string_lossy());
    }
    let until = Instant::now() + Duration::from_secs_f64(seconds);
    while Instant::now() < until {
        app.tick();
        for request in app.take_requests() {
            eprintln!("  {request:?}");
            if let web::api::Request::Navigate(url) = request {
                eprintln!("  移る {}", app.navigate(&url));
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if let Some(scale) = std::env::var("ZW_RESCALE")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        app.set_scale(scale);
        for _ in 0..10 {
            app.tick();
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    if let Some(code) = std::env::var_os("ZW_EVAL") {
        app.eval(&code.to_string_lossy());
        for _ in 0..10 {
            app.tick();
            std::thread::sleep(Duration::from_millis(20));
        }
        for request in app.take_requests() {
            println!("{request:?}");
        }
    }
    // ZW_KEYS=40,40,13: 押すキーの番号。0 は一秒待つだけ。
    if let Some(keys) = std::env::var_os("ZW_KEYS") {
        for code in keys
            .to_string_lossy()
            .split(',')
            .filter_map(|k| k.trim().parse().ok())
        {
            if code != 0 {
                app.key(code);
            }
            for _ in 0..20 {
                app.tick();
                std::thread::sleep(Duration::from_millis(50));
                for request in app.take_requests() {
                    eprintln!("  {request:?}");
                    if let web::api::Request::Navigate(url) = request {
                        eprintln!("  移る {}", app.navigate(&url));
                    }
                }
            }
        }
    }
    if let Some(code) = std::env::var_os("ZW_EVAL_AFTER") {
        app.eval(&code.to_string_lossy());
        for _ in 0..10 {
            app.tick();
            std::thread::sleep(Duration::from_millis(20));
        }
        for request in app.take_requests() {
            println!("{request:?}");
        }
    }
    // ZW_CAPTION=字幕.mfu: その MFU（STD-B60 表 9-1 のヘッダー付き）をリスナーへ。
    if let Some(path) = std::env::var_os("ZW_CAPTION")
        && let Ok(mfu) = std::fs::read(path)
    {
        let stream = app_caption();
        app.caption(&stream, &mfu);
        for _ in 0..20 {
            app.tick();
            std::thread::sleep(Duration::from_millis(30));
        }
        for request in app.take_requests() {
            eprintln!("  {request:?}");
        }
    }
    if std::env::var_os("ZW_DUMP").is_some() {
        app.eval("__blitz_send_message(document.body.innerHTML)");
        for request in app.take_requests() {
            if let web::api::Request::Log(text) = request {
                println!("{text}");
            }
        }
    }
    eprintln!("映像 {:?}", app.video_rect());
    if let Some(n) = std::env::var("ZW_PAINTS")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
    {
        let began = Instant::now();
        for _ in 0..n {
            app.tick();
            let _ = app.paint();
            std::thread::sleep(Duration::from_millis(33));
        }
        eprintln!("{n} 回で {:?}（sleep した時間を含む）", began.elapsed());
    }
    // ZW_SHOTS=n: 一秒おきに n 枚撮る。`ZW_PROBE`（式か `@ファイル`）を撮るたびに実行する。
    if let Some(n) = std::env::var("ZW_SHOTS")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
    {
        let stem = out.strip_suffix(".png").unwrap_or(out);
        for i in 0..n {
            for _ in 0..20 {
                app.tick();
                std::thread::sleep(Duration::from_millis(50));
                for request in app.take_requests() {
                    if let web::api::Request::Navigate(url) = request {
                        eprintln!("  移る {}", app.navigate(&url));
                    }
                }
            }
            if let Some(code) = std::env::var_os("ZW_PROBE") {
                let code = code.to_string_lossy().into_owned();
                let code = match code.strip_prefix('@') {
                    Some(file) => std::fs::read_to_string(file).unwrap_or_default(),
                    None => code,
                };
                app.eval(&code);
                for request in app.take_requests() {
                    if let web::api::Request::Log(text) = request {
                        eprintln!("  [{i:02}] {text}");
                    }
                }
            }
            let (buffer, w, h) = app.paint();
            let path = format!("{stem}-{i:02}.png");
            write_png(&path, &buffer, w, h);
            eprintln!("→ {path}");
        }
    }
    let painted = Instant::now();
    let (buffer, w, h) = app.paint();
    eprintln!("レンダリングに {:?}", painted.elapsed());
    let clear = buffer
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[3] == 0)
        .count();
    let opaque = buffer
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[3] == 255)
        .count();
    eprintln!("透明 {clear} 不透明 {opaque} / {}", buffer.len() / 4);
    write_png(out, &buffer, w, h);
    eprintln!("→ {out} ({w}x{h})");
}

fn write_png(path: &str, rgba: &[u8], w: u32, h: u32) {
    let file = std::fs::File::create(path).expect("書ける");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("ヘッダー");
    writer.write_image_data(rgba).expect("中身");
}

fn app_caption() -> web::api::Caption {
    web::api::Caption {
        tag: 0x30,
        language: String::from("jpn"),
        tmd: 0b0010,
        resolution: 1,
        ..Default::default()
    }
}
