// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! ビルドしたパイプのプラグイン（wasm）を本体の側から通す。プログラムは `cat`。wasm が無ければ飛ばす。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

fn pipe_wasm() -> Option<PathBuf> {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target"));
    ["plugin", "release", "debug"]
        .into_iter()
        .map(|profile| {
            target
                .join("wasm32-unknown-unknown")
                .join(profile)
                .join("zerowave_pipe.wasm")
        })
        .find(|path| path.exists())
}

fn io_with(plugin: &zwsdk::host::Plugin, chosen: &[(&str, &str)]) -> zwsdk::host::Io {
    let table: BTreeMap<String, String> = chosen
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    zwsdk::host::Io {
        live: Some(Arc::new(RwLock::new(table))),
        programs: plugin
            .manifest()
            .config
            .iter()
            .filter(|setting| setting.kind == "program")
            .map(|setting| setting.key.clone())
            .collect(),
        envs: plugin
            .manifest()
            .config
            .iter()
            .filter(|setting| setting.is_env())
            .map(|setting| setting.key.clone())
            .collect(),
        states: plugin
            .manifest()
            .config
            .iter()
            .filter(|setting| setting.kind == "state")
            .map(|setting| setting.key.clone())
            .collect(),
        ..zwsdk::host::Io::default()
    }
}

type Table = Arc<RwLock<BTreeMap<String, String>>>;

fn chain(path: &Path, chosen: &[(&str, &str)]) -> (zwsdk::host::Chain, Arc<Mutex<Vec<String>>>) {
    let (chain, said, _) = chain_with_table(path, chosen);
    (chain, said)
}

fn chain_with_table(
    path: &Path,
    chosen: &[(&str, &str)],
) -> (zwsdk::host::Chain, Arc<Mutex<Vec<String>>>, Table) {
    let engine = zwsdk::host::engine().expect("engine");
    let plugin = zwsdk::host::Plugin::load(&engine, path).expect("loads");
    assert!(plugin.has("stream"));
    assert!(plugin.has("ui"), "自分の設定ページを持つ");
    assert_eq!(plugin.manifest().ui.settings.as_deref(), Some("main"));
    assert_eq!(
        plugin.manifest().order,
        10,
        "多重化フレームを分離するプラグインより後"
    );
    let said = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&said);
    let io = io_with(&plugin, chosen);
    let table = io.live.clone().expect("共有のテーブル");
    let chain = zwsdk::host::Chain::new(vec![
        plugin
            .stream(
                Box::new(move |_, text| sink.lock().unwrap().push(text.to_owned())),
                io,
            )
            .expect("an instance"),
    ]);
    (chain, said, table)
}

/// 子の出力は遅れて出るので、流し終えた後も空の入力で読み出しを回す。
#[cfg(unix)]
#[test]
fn a_ts_stream_goes_through_the_chosen_program() {
    let Some(path) = pipe_wasm() else {
        eprintln!("zerowave_pipe.wasm が無いので飛ばす");
        return;
    };
    let (mut chain, said, table) = chain_with_table(&path, &[("ts", "cat")]);
    assert!(chain.start(zwsdk::host::Container::Ts, None).is_empty());
    let sent: Vec<u8> = (0..188 * 2000).map(|n| (n % 251) as u8).collect();
    let mut got = Vec::new();
    let mut failed = Vec::new();
    for chunk in sent.chunks(188 * 64) {
        got.extend_from_slice(chain.run(chunk, &mut failed));
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while got.len() < sent.len() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
        got.extend_from_slice(chain.run(&[], &mut failed));
    }
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(got.len(), sent.len());
    assert_eq!(got, sent);
    assert!(
        said.lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("TS を選んだプログラムに通します")),
        "{:?}",
        said.lock().unwrap()
    );
    assert_eq!(
        table.read().unwrap().get("ts_state").map(String::as_str),
        Some("running"),
        "ページが読む状態"
    );
}

#[test]
fn an_unchosen_container_passes_through() {
    let Some(path) = pipe_wasm() else {
        eprintln!("zerowave_pipe.wasm が無いので飛ばす");
        return;
    };
    let (mut chain, said) = chain(&path, &[("ts", "cat")]);
    assert!(chain.start(zwsdk::host::Container::Tlv, None).is_empty());
    let bytes = [0x7fu8; 1000];
    let mut failed = Vec::new();
    assert_eq!(chain.run(&bytes, &mut failed), bytes);
    assert!(failed.is_empty());
    assert!(
        said.lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("そのまま通します")),
        "{:?}",
        said.lock().unwrap()
    );
}

#[cfg(unix)]
fn feed_until(
    chain: &mut zwsdk::host::Chain,
    said: &Arc<Mutex<Vec<String>>>,
    wanted: &str,
    within: std::time::Duration,
) -> Vec<u8> {
    let chunk = [0x47u8; 188 * 8];
    let mut got = Vec::new();
    let mut failed = Vec::new();
    let deadline = std::time::Instant::now() + within;
    while !said
        .lock()
        .unwrap()
        .iter()
        .any(|line| line.contains(wanted))
    {
        assert!(
            std::time::Instant::now() < deadline,
            "「{wanted}」が出ない: {:?}",
            said.lock().unwrap()
        );
        got.extend_from_slice(chain.run(&chunk, &mut failed));
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(failed.is_empty(), "{failed:?}");
    got
}

/// すぐ終わるプログラムは三度まで起動し直し、四度目で通す。
#[cfg(unix)]
#[test]
fn a_program_that_keeps_dying_is_retried_then_bypassed() {
    let Some(path) = pipe_wasm() else {
        eprintln!("zerowave_pipe.wasm が無いので飛ばす");
        return;
    };
    let (mut chain, said) = chain(&path, &[("ts", "true")]);
    assert!(chain.start(zwsdk::host::Container::Ts, None).is_empty());
    let before = feed_until(
        &mut chain,
        &said,
        "この選局では TS をそのまま通します",
        std::time::Duration::from_secs(15),
    );
    assert!(before.is_empty(), "止まっている間は何も出さない");
    let lines = said.lock().unwrap().clone();
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("起動し直しました"))
            .count(),
        3,
        "{lines:?}"
    );
    let chunk = [0x47u8; 188];
    let mut failed = Vec::new();
    assert_eq!(chain.run(&chunk, &mut failed), chunk, "諦めた後はそのまま");

    assert!(chain.start(zwsdk::host::Container::Ts, None).is_empty());
    said.lock().unwrap().clear();
    feed_until(
        &mut chain,
        &said,
        "（1 回目）",
        std::time::Duration::from_secs(5),
    );
}

#[cfg(unix)]
#[test]
fn a_program_that_never_answers_is_restarted() {
    let Some(path) = pipe_wasm() else {
        eprintln!("zerowave_pipe.wasm が無いので飛ばす");
        return;
    };
    let (mut chain, said) = chain(
        &path,
        &[("ts", "sh"), ("ts_args", r#"["-c","cat >/dev/null"]"#)],
    );
    assert!(chain.start(zwsdk::host::Container::Ts, None).is_empty());
    let started = std::time::Instant::now();
    feed_until(
        &mut chain,
        &said,
        "応答しません",
        std::time::Duration::from_secs(5),
    );
    assert!(started.elapsed() >= std::time::Duration::from_millis(1400));
    feed_until(
        &mut chain,
        &said,
        "起動し直しました",
        std::time::Duration::from_secs(5),
    );
}

#[cfg(unix)]
#[test]
fn the_chosen_environment_reaches_the_program() {
    let Some(path) = pipe_wasm() else {
        eprintln!("zerowave_pipe.wasm が無いので飛ばす");
        return;
    };
    let (mut chain, said) = chain(
        &path,
        &[
            ("ts", "sh"),
            ("ts_args", r#"["-c","printf %s $ZW_PIPE; cat >/dev/null"]"#),
            ("ts_env", r#"{"ZW_PIPE":"${HOME}:ok"}"#),
        ],
    );
    assert!(chain.start(zwsdk::host::Container::Ts, None).is_empty());
    let wanted = format!("{}:ok", std::env::var("HOME").unwrap_or_default());
    let mut got = Vec::new();
    let mut failed = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while got.len() < wanted.len() && std::time::Instant::now() < deadline {
        got.extend_from_slice(chain.run(&[0x47u8; 188], &mut failed));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        String::from_utf8_lossy(&got),
        wanted,
        "{:?}",
        said.lock().unwrap()
    );
}
