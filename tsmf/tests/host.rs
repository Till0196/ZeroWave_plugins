// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! ビルドした TSMF のプラグイン（wasm）を本体の側から通す。wasm が無ければ飛ばす。

use std::path::PathBuf;

fn tsmf_wasm() -> Option<PathBuf> {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target"));
    for profile in ["release", "debug"] {
        let path = target
            .join("wasm32-unknown-unknown")
            .join(profile)
            .join("zerowave_tsmf.wasm");
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn null_packets(count: usize) -> Vec<u8> {
    let mut packet = [0u8; 188];
    packet[0] = 0x47;
    packet[1] = 0x1f;
    packet[2] = 0xff;
    packet[3] = 0x10;
    packet.iter().copied().cycle().take(188 * count).collect()
}

#[test]
fn the_tsmf_plugin_loads_and_passes_a_plain_stream_through() {
    let Some(path) = tsmf_wasm() else {
        eprintln!("zerowave_tsmf.wasm が無いので飛ばす");
        return;
    };
    let engine = zwsdk::host::engine().expect("engine");
    let plugin = zwsdk::host::Plugin::load(&engine, &path).expect("loads");
    assert_eq!(plugin.name(), "多重フレーム（TSMF）");
    assert!(plugin.has("stream"));
    let said = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&said);
    let mut chain = zwsdk::host::Chain::new(vec![
        plugin
            .stream(
                Box::new(move |_, text| sink.lock().unwrap().push(text.to_owned())),
                zwsdk::host::Io::default(),
            )
            .expect("an instance"),
    ]);
    assert!(
        chain
            .start(zwsdk::host::Container::Tlv, Some(1024))
            .is_empty()
    );
    let bytes = null_packets(64);
    let mut failed = Vec::new();
    let out = chain.run(&bytes, &mut failed).to_vec();
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(out, bytes);
    assert!(said.lock().unwrap().is_empty());
}

#[test]
fn a_split_frame_borrows_the_other_carriers() {
    use std::io::Read;
    use std::sync::{Arc, Mutex};
    let Some(path) = tsmf_wasm() else {
        eprintln!("zerowave_tsmf.wasm が無いので飛ばす");
        return;
    };

    #[derive(Default)]
    struct Lending {
        asked: Mutex<Vec<String>>,
        read: Arc<Mutex<usize>>,
        stopped: Arc<Mutex<usize>>,
    }
    struct Counting(Arc<Mutex<usize>>, usize);
    impl Read for Counting {
        fn read(&mut self, into: &mut [u8]) -> std::io::Result<usize> {
            if self.1 == 0 {
                return Ok(0);
            }
            let n = into.len().min(self.1).min(188 * 10);
            into[..n].fill(0x47);
            self.1 -= n;
            *self.0.lock().unwrap() += n;
            Ok(n)
        }
    }
    impl zwsdk::host::tuner::Lender for Lending {
        fn words(&self) -> String {
            String::from(
                "DELIVERY_SYSTEM=ISDBC&FREQUENCY=93000000&multiplex.part=tlv:1&multiplex.carriers=93000000,99000000,105000000",
            )
        }
        fn open(&self, words: &str) -> Result<zwsdk::host::tuner::Lent, String> {
            self.asked.lock().unwrap().push(words.to_owned());
            let stopped = Arc::clone(&self.stopped);
            Ok(zwsdk::host::tuner::Lent {
                bytes: Box::new(Counting(Arc::clone(&self.read), 188 * 100)),
                stop: Box::new(move || *stopped.lock().unwrap() += 1),
            })
        }
    }

    let engine = zwsdk::host::engine().expect("engine");
    let plugin = zwsdk::host::Plugin::load(&engine, &path).expect("loads");
    assert!(plugin.has("tuners"), "チューナーの確保を宣言している");
    let lending = Arc::new(Lending::default());
    let said = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&said);
    let io = zwsdk::host::Io {
        lender: Some(lending.clone() as zwsdk::host::tuner::LenderOf),
        ..zwsdk::host::Io::default()
    };
    let mut chain = zwsdk::host::Chain::new(vec![
        plugin
            .stream(
                Box::new(move |_, text| sink.lock().unwrap().push(text.to_owned())),
                io,
            )
            .expect("an instance"),
    ]);
    assert!(chain.start(zwsdk::host::Container::Tlv, None).is_empty());
    let asked = lending.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 2, "残りの二本: {asked:?}");
    assert!(asked[0].contains("FREQUENCY=99000000"), "{asked:?}");
    assert!(asked[1].contains("FREQUENCY=105000000"), "{asked:?}");
    let mut failed = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while *lending.read.lock().unwrap() < 2 * 188 * 100 && std::time::Instant::now() < deadline {
        let _ = chain.run(&null_packets(10), &mut failed);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(
        *lending.read.lock().unwrap(),
        2 * 188 * 100,
        "確保したチューナーの分を読み切った"
    );
    assert!(
        said.lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("確保して結合")),
        "{:?}",
        said.lock().unwrap()
    );
    drop(chain);
    assert_eq!(*lending.stopped.lock().unwrap(), 2, "落とせば返す");

    let plugin = zwsdk::host::Plugin::load(&engine, &path).expect("loads");
    let mut chain = zwsdk::host::Chain::new(vec![
        plugin
            .stream(Box::new(|_, _| {}), zwsdk::host::Io::default())
            .expect("an instance"),
    ]);
    assert!(chain.start(zwsdk::host::Container::Tlv, None).is_empty());
}
