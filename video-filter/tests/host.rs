// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! ビルドした shaders のプラグイン（wasm）を本体の側から載せてビルドさせる。wasm が無ければ飛ばす。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use zwsdk::ui::{Ctx, Event, MenuItem, Notice};
use zwsdk::video::{At, Device, Env, Format, Program};

fn wasm(sub: &str) -> Option<PathBuf> {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target"));
    let path = target
        .join(sub)
        .join("wasm32-unknown-unknown/release/zerowave_video_filter.wasm");
    path.exists().then_some(path)
}

fn env(at: At) -> Env {
    Env {
        at,
        format: Format {
            width: 1440,
            height: 1080,
            depth: 8,
            interlaced: true,
            aspect: [16, 9],
            transfer: "sdr".into(),
        },
        input: [1440, 1080],
        target: [1920, 1080],
        rate: [30000, 1001],
        display: 60.0,
        device: Device {
            backend: "vulkan".into(),
            compute: true,
            float32_filterable: false,
            max_texture: 16384,
            host: Vec::new(),
            ..Device::default()
        },
        settings: BTreeMap::new(),
        program: Some(program(0x7)),
    }
}

fn program(large: u8) -> Program {
    Program {
        genres: vec![(large, 0)],
        network: 0x7fef,
        service: 0x0400,
        band: "GR".into(),
        channel: "[局]".into(),
        title: "[番組]".into(),
        recording: false,
        structure: zwsdk::video::Structure::Unknown,
        minute: 20 * 60,
    }
}

const PRESETS: &str = r#"[{"name":"見本","deinterlace":"ela.wgsl","filters":[{"file":"sharpen.wgsl","on":true},{"file":"blend2x.wgsl","on":true}]},{"name":"無い物","filters":[{"file":"nothing.wgsl","on":true}]}]"#;
const AUTOS: &str =
    r#"[{"name":"アニメ","when":[{"kind":"genre","op":"any","values":["7"]}],"preset":"見本"}]"#;

fn io(
    plugin: &zwsdk::host::Plugin,
    live: &zwsdk::host::IoLive,
    notices: &Arc<Mutex<Vec<Notice>>>,
) -> zwsdk::host::Io {
    let config = &plugin.manifest().config;
    let sink = Arc::clone(notices);
    zwsdk::host::Io {
        folders: config
            .iter()
            .filter(|s| s.kind == "folder")
            .map(|s| s.key.clone())
            .collect(),
        home: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/shaders")),
        defaults: config
            .iter()
            .map(|s| (s.key.clone(), s.default.clone()))
            .collect(),
        live: Some(Arc::clone(live)),
        store: Some(Arc::new(|_: &BTreeMap<String, String>| {})),
        notify: Some(Arc::new(move |notice: Notice| {
            sink.lock().unwrap().push(notice)
        })),
        ..zwsdk::host::Io::default()
    }
}

/// wasm をビルドするのは重いので一本に。
#[test]
fn the_plugin_works_end_to_end() {
    let Some(path) = wasm("") else {
        eprintln!("zerowave_video_filter.wasm が無いので飛ばす");
        return;
    };
    let engine = zwsdk::host::engine().expect("engine");
    let plugin = zwsdk::host::Plugin::load(&engine, &path).expect("載る");
    assert_eq!(plugin.name(), "映像フィルター");
    assert!(plugin.has("video") && plugin.has("ui"));
    assert_eq!(
        plugin.video_manifest().at,
        vec![At::Deinterlace, At::Picture]
    );
    assert_eq!(plugin.manifest().ui.title, "映像フィルター");
    assert_eq!(plugin.manifest().ui.panels.len(), 1);
    assert_eq!(plugin.manifest().ui.panels[0].id, "filter");
    let live: zwsdk::host::IoLive = Arc::new(RwLock::new(
        [
            ("presets", PRESETS),
            ("autos", AUTOS),
            ("fallback", "無い物"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect(),
    ));
    let notices = Arc::new(Mutex::new(Vec::new()));
    let mut picture = plugin
        .video(Box::new(|_, _| {}), io(&plugin, &live, &notices))
        .expect("インスタンス");

    let graph = picture.build(&env(At::Picture)).expect("作れる");
    graph.check(At::Picture).expect("本体の検査");
    assert_eq!(graph.passes.len(), 2);
    assert!(
        graph.passes[1].name.starts_with("1|"),
        "{}",
        graph.passes[1].name
    );
    assert_eq!(
        picture.key(&env(At::Picture)).as_deref(),
        Some("sharpen.wgsl | blend2x.wgsl")
    );
    {
        let said = notices.lock().unwrap();
        assert_eq!(said.len(), 1, "同じなら通知しない");
        assert_eq!(said[0].title, "映像フィルター：［見本］");
        assert_eq!(
            said[0].details[0],
            "自動選択［アニメ］：ジャンル：アニメ／特撮"
        );
    }
    // 準備中の通知はトーストを差し替えるので、済んだらプリセットの通知を出し直す。
    picture.event(&zwsdk::video::Event::Building { total: 2 });
    picture.event(&zwsdk::video::Event::Built { seconds: 0.0 });
    {
        let said = notices.lock().unwrap();
        assert_eq!(said.len(), 3);
        assert!(said[1].title.starts_with("映像フィルターを準備しています"));
        assert_eq!(said[2].title, "映像フィルター：［見本］");
    }
    notices.lock().unwrap().clear();
    picture.event(&zwsdk::video::Event::Building { total: 2 });
    picture.event(&zwsdk::video::Event::Built { seconds: 0.0 });
    {
        let said = notices.lock().unwrap();
        assert_eq!(said.len(), 2);
        assert_eq!(said[1].seconds, Some(0.0));
    }
    notices.lock().unwrap().clear();
    let mut deinterlace = plugin
        .video(Box::new(|_, _| {}), io(&plugin, &live, &notices))
        .expect("インスタンス");
    deinterlace
        .build(&env(At::Deinterlace))
        .expect("作れる")
        .check(At::Deinterlace)
        .expect("本体の検査");
    let mut other = env(At::Picture);
    other.program = Some(program(0x0));
    assert_eq!(picture.key(&other).as_deref(), Some("nothing.wgsl"));
    let why = picture.build(&other).unwrap_err();
    assert!(
        why.contains("ファイルが見つかりません：nothing.wgsl"),
        "{why}"
    );
    notices.lock().unwrap().clear();
    picture.event(&zwsdk::video::Event::Paused {
        at: At::Picture,
        ms: 9.5,
        budget: 8.3,
    });
    assert_eq!(notices.lock().unwrap()[0].tone, zwsdk::ui::Tone::Warn);

    let mut ui = plugin
        .ui(Box::new(|_, _| {}), io(&plugin, &live, &notices))
        .expect("ui");
    let ctx = Ctx {
        program: Some(program(0x7)),
        format: Some(env(At::Picture).format),
        ..Ctx::default()
    };
    let items = ui.menu(&ctx).expect("メニュー");
    assert!(
        matches!(&items[0], MenuItem::Item { label, .. } if label == "自動選択（アニメ → 見本）"),
        "{items:?}"
    );
    ui.pick(&ctx, "none").expect("押せる");
    assert_eq!(
        picture.key(&env(At::Picture)).as_deref(),
        Some(""),
        "適用しない"
    );
    ui.pick(&ctx, "auto").expect("押せる");
    assert_eq!(
        picture.key(&env(At::Picture)).as_deref(),
        Some("sharpen.wgsl | blend2x.wgsl")
    );

    let panel = ui.page("filter", &ctx).expect("パネル");
    assert_eq!(panel.nodes.len(), 3, "{panel:?}");
    let page = ui.page("main", &ctx).expect("ページ");
    assert_eq!(page.tabs.len(), 4);
    let event = |id: &str, value: &str| Event {
        id: id.to_owned(),
        value: value.to_owned(),
    };
    ui.event("main", &ctx, &event("tab", "autos"))
        .expect("応答");
    assert_eq!(
        ui.event("main", &ctx, &event("edit:0", ""))
            .expect("応答")
            .open
            .as_deref(),
        Some("rule")
    );
    ui.event("rule", &ctx, &event("add", "network"))
        .expect("応答");
    ui.event("rule", &ctx, &event("net:1", "BS")).expect("応答");
    assert!(
        ui.event("rule", &ctx, &event("save", ""))
            .expect("応答")
            .close
    );
    assert!(
        !live.read().unwrap()["autos"].contains("network"),
        "OK まで書かない"
    );
    ui.event("main", &ctx, &event("ok", "")).expect("応答");
    let autos = live.read().unwrap()["autos"].clone();
    assert!(
        autos.contains(r#""kind":"network","op":"any","values":["BS"]"#),
        "{autos}"
    );
    assert_eq!(
        ui.page("status", &ctx).expect("ページ").title,
        "映像フィルターの状態"
    );
}
