// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 本体（OS・GPU）にやらせるフレーム補間。[`ENTRIES`] の名前で選ぶと自前の書式のテキストを作る。
//! 本体がその実装を持たなければ [`FALLBACK`] を使う。
//! RIFE は本体の外部プロセスのインターフェースで `zwtrt`（TensorRT）を、mac では `zwcoreml`（CoreML）を起動させる。

use std::collections::BTreeMap;

use zwsdk::video::{Device, HOST_FLOW, HOST_FLOW_GRID, HOST_INTERPOLATE, HOST_PROCESS};

/// 設定（`kind = "program"`）：`zwtrt`（mac は `zwcoreml`）の実行ファイル。空なら RIFE は使えない。
pub const RIFE_PROGRAM: &str = "rife_program";
/// 設定（`kind = "folder"`）：ONNX・エンジン・モデルのフォルダー。空なら実行ファイルと同じフォルダー。
pub const RIFE_DIR: &str = "rife_dir";
pub const RIFE_ONNX: &str = "rife_onnx";
/// 設定（`kind = "folder"`）：`PATH` の先頭に足す TensorRT と CUDA の DLL のフォルダー。
pub const RIFE_PATHS: [&str; 2] = ["rife_tensorrt", "rife_cuda"];
/// 設定：mac の `zwcoreml` のモデル名（空なら [`RIFE_COREML_DEFAULT`]）。
/// 後ろに `:gpu`・`:ane` を付けると計算に使うデバイスを決める。
pub const RIFE_COREML: &str = "rife_coreml";
pub const RIFE_COREML_DEFAULT: &str = "rife-v4.6-s0.5";
/// 設定：受け渡しの方式を絞る（`d3d12`・`vulkan`・`shm` を `,` で。空なら本体に任せる）。
pub const RIFE_WAY: &str = "rife_way";

const ANY: &str = "*";

#[derive(Debug)]
pub struct Entry {
    pub file: &'static str,
    pub name: &'static str,
    /// 本体の処理と実装（`Device::host` の一つ、[`ANY`] なら問わない）。空ならプラグインが自分で処理する。
    pub job: &'static str,
    pub who: &'static str,
}

pub const ENTRIES: [Entry; 7] = [
    Entry {
        file: "interpolate/@videotoolbox.wgsl",
        name: "VideoToolbox（macOS）",
        job: HOST_INTERPOLATE,
        who: "videotoolbox",
    },
    Entry {
        file: "interpolate/@amf.wgsl",
        name: "AMF（AMD）",
        job: HOST_INTERPOLATE,
        who: "amf",
    },
    Entry {
        file: "interpolate/@vpl.wgsl",
        name: "oneVPL（Intel）",
        job: HOST_INTERPOLATE,
        who: "vpl",
    },
    Entry {
        file: "interpolate/@rife.wgsl",
        name: "RIFE（TensorRT・CoreML）",
        job: HOST_INTERPOLATE,
        who: HOST_PROCESS,
    },
    Entry {
        file: crate::rife_gpu::FILE,
        name: "RIFE（GPU 共通）",
        job: "",
        who: ANY,
    },
    Entry {
        file: "interpolate/@flow.wgsl",
        name: "動き探しの補間（自動）",
        job: HOST_FLOW,
        who: ANY,
    },
    Entry {
        file: "interpolate/@nvofa.wgsl",
        name: "NVOFA（NVIDIA）",
        job: HOST_FLOW,
        who: "nvofa",
    },
];

pub fn entry(file: &str) -> Option<&'static Entry> {
    ENTRIES.iter().find(|entry| entry.file == file)
}

pub fn usable(file: &str, host: &[String]) -> bool {
    match entry(file) {
        Some(entry) if entry.job.is_empty() => true,
        Some(entry) => host.iter().any(|h| {
            h.split_once('/').is_some_and(|(job, who)| {
                job == entry.job && (entry.who == ANY || who == entry.who)
            })
        }),
        None => true,
    }
}

pub const FALLBACK: &str = "interpolate/motion.wgsl";

const INTERPOLATE: &str = include_str!("wgsl/interpolate.wgsl");

/// フローを本体が作り、位相ごとにフローに沿って混ぜる。`CONFIDENCE` の所には信頼度を
/// 読むパスか 1 を返す関数が入る（[`text`]）。
const FLOW: &str = include_str!("wgsl/flow.wgsl");

const CONFIDENCE_TEXTURE: &str = include_str!("wgsl/confidence_texture.wgsl");
const CONFIDENCE_PASS: &str = include_str!("wgsl/confidence_pass.wgsl");
const CONFIDENCE_READ_FN: &str = include_str!("wgsl/confidence_read.wgsl");
/// 信頼度の処理に対応しない本体では、いつも信頼する。
const CONFIDENCE_ONE_FN: &str = include_str!("wgsl/confidence_one.wgsl");

fn rife(device: &Device, settings: &BTreeMap<String, String>) -> Option<String> {
    let setting = |key: &str| {
        settings
            .get(key)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
    };
    let Some(program) = setting(RIFE_PROGRAM) else {
        zwsdk::plug::info(&format!(
            "RIFE：設定の {RIFE_PROGRAM} に zwtrt（mac は zwcoreml）が選ばれていない"
        ));
        return None;
    };
    let chosen_dir = setting(RIFE_DIR);
    let dir = match chosen_dir {
        Some(dir) => dir,
        None => {
            // Windows の「パスのコピー」は前後に `"` を付ける。
            let program = program.trim_matches('"');
            program
                .rfind(['/', '\\'])
                .map_or(".", |cut| &program[..cut])
        }
    };
    let dir = dir.trim_end_matches(['/', '\\']);
    let mut lines = if device.backend == "metal" {
        rife_coreml(dir, setting(RIFE_COREML))
    } else {
        rife_trt(dir, setting(RIFE_ONNX))
    };
    if chosen_dir.is_some() {
        lines.push(format!("//!DIR {RIFE_DIR}"));
    }
    if device.backend != "metal" {
        lines.extend(
            RIFE_PATHS
                .iter()
                .filter(|key| setting(key).is_some())
                .map(|key| format!("//!PATH {key}")),
        );
    }
    if let Some(ways) = setting(RIFE_WAY) {
        lines.extend(
            ways.split([',', ';'])
                .map(str::trim)
                .filter(|w| !w.is_empty())
                .map(|w| format!("//!WAY {w}")),
        );
    }
    Some(lines.join("\n"))
}

fn rife_coreml(dir: &str, model: Option<&str>) -> Vec<String> {
    let model = model.unwrap_or(RIFE_COREML_DEFAULT);
    let (name, units) = model.split_once(':').unwrap_or((model, ""));
    let mut lines = vec![
        format!("//!PROGRAM {RIFE_PROGRAM}"),
        String::from("//!ARG serve"),
        format!("//!ARG {dir}"),
        format!("//!ARG {}", name.trim()),
    ];
    if !units.trim().is_empty() {
        lines.push(format!("//!ARG {}", units.trim()));
    }
    lines
}

fn rife_trt(dir: &str, onnx: Option<&str>) -> Vec<String> {
    let onnx = onnx.map_or_else(|| format!("{dir}/rife.onnx"), str::to_owned);
    vec![
        format!("//!PROGRAM {RIFE_PROGRAM}"),
        String::from("//!ARG serve"),
        format!("//!ARG {onnx}"),
        format!("//!ARG {dir}"),
    ]
}

/// 実装のテキスト。`None` なら本体がこの実装を持たない（[`FALLBACK`] を読む）。
pub fn text(entry: &Entry, device: &Device, settings: &BTreeMap<String, String>) -> Option<String> {
    if !usable(entry.file, &device.host) {
        return None;
    }
    if entry.job.is_empty() {
        return Some(String::new());
    }
    let job = if entry.who == ANY {
        entry.job.to_owned()
    } else {
        format!("{}/{}", entry.job, entry.who)
    };
    if entry.job == HOST_INTERPOLATE {
        let mut host = format!("//!HOST {job}\n");
        if entry.who == HOST_PROCESS {
            host.push_str(&rife(device, settings)?);
            host.push('\n');
        }
        return Some(INTERPOLATE.replace("//!HOST interpolate\n", &host));
    }
    // 機能を通知しない本体は既定のグリッドで、信頼度も 1 とみなす。
    let caps = if entry.who == ANY {
        device.flow_caps()
    } else {
        device.flows.iter().find(|caps| caps.who == entry.who)
    };
    let grid = caps
        .and_then(|caps| caps.grids.iter().copied().min())
        .unwrap_or(HOST_FLOW_GRID)
        .max(1);
    let (texture, pass, read, function) = if caps.is_some() {
        (
            CONFIDENCE_TEXTURE.trim_end(),
            CONFIDENCE_PASS.trim_end(),
            "confidence",
            CONFIDENCE_READ_FN.trim_end(),
        )
    } else {
        ("", "", "", CONFIDENCE_ONE_FN.trim_end())
    };
    Some(
        FLOW.replace("//!HOST flow\n", &format!("//!HOST {job}\n"))
            .replace("CONFIDENCE_TEXTURE", texture)
            .replace("CONFIDENCE_PASS", pass)
            .replace("CONFIDENCE_READ", read)
            .replace("CONFIDENCE_FN", function)
            .replace("GRID1", &(grid - 1).to_string())
            .replace("GRIDf", &format!("{grid}.0"))
            .replace("GRID", &grid.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_hosts_the_device_names_are_usable() {
        let arc = vec![String::from("interpolate/vpl")];
        assert!(usable("interpolate/@vpl.wgsl", &arc));
        assert!(!usable("interpolate/@nvofa.wgsl", &arc));
        assert!(!usable("interpolate/@videotoolbox.wgsl", &arc));
        assert!(!usable("interpolate/@amf.wgsl", &[]));
        assert!(usable(
            "interpolate/@flow.wgsl",
            &[String::from("flow/nvofa")]
        ));
        assert!(usable(
            "interpolate/@flow.wgsl",
            &[String::from("flow/intel")]
        ));
        assert!(!usable("interpolate/@flow.wgsl", &arc));
        assert!(usable("interpolate/motion.wgsl", &[]));
        assert!(usable(
            "interpolate/@rife.wgsl",
            &[String::from("interpolate/process")]
        ));
        assert!(!usable("interpolate/@rife.wgsl", &arc));
    }

    #[test]
    fn rife_starts_zwtrt_from_the_settings() {
        let device = Device {
            backend: String::from("dx12"),
            compute: true,
            float32_filterable: true,
            max_texture: 16384,
            host: vec![String::from("interpolate/process")],
            ..Device::default()
        };
        let entry = entry("interpolate/@rife.wgsl").expect("RIFE");
        assert!(
            text(entry, &device, &BTreeMap::new()).is_none(),
            "実行ファイルが選ばれていない"
        );
        let settings: BTreeMap<String, String> = [
            (RIFE_PROGRAM, "\"C:\\trt\\zwtrt.exe\""),
            ("rife_cuda", "C:\\cuda\\bin"),
            (RIFE_WAY, "shm"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let text = text(entry, &device, &settings).expect("使える");
        for line in [
            "//!HOST interpolate/process",
            "//!PROGRAM rife_program",
            "//!ARG serve",
            "//!ARG C:\\trt/rife.onnx",
            "//!ARG C:\\trt",
            "//!PATH rife_cuda",
            "//!WAY shm",
        ] {
            assert!(text.lines().any(|l| l == line), "{line}\n{text}");
        }
        assert!(!text.contains("//!DIR"), "{text}");
        assert!(!text.contains("rife_tensorrt"), "{text}");
    }

    #[test]
    fn rife_starts_zwcoreml_on_metal() {
        let device = Device {
            backend: String::from("metal"),
            compute: true,
            float32_filterable: true,
            max_texture: 16384,
            host: vec![String::from("interpolate/process")],
            ..Device::default()
        };
        let entry = entry("interpolate/@rife.wgsl").expect("RIFE");
        let settings = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect()
        };
        let plain = text(
            entry,
            &device,
            &settings(&[
                (RIFE_PROGRAM, "/Users/a/bin/zwcoreml"),
                (RIFE_DIR, "/Users/a/rife/"),
            ]),
        )
        .expect("使える");
        let lines: Vec<&str> = plain
            .lines()
            .skip_while(|l| !l.starts_with("//!HOST"))
            .take(6)
            .collect();
        assert_eq!(
            lines,
            [
                "//!HOST interpolate/process",
                "//!PROGRAM rife_program",
                "//!ARG serve",
                "//!ARG /Users/a/rife",
                "//!ARG rife-v4.6-s0.5",
                "//!DIR rife_dir",
            ]
        );
        let chosen = text(
            entry,
            &device,
            &settings(&[
                (RIFE_PROGRAM, "/r/zwcoreml"),
                (RIFE_COREML, "rife-v4.22.lite-s1:gpu"),
                (RIFE_WAY, "shm"),
            ]),
        )
        .expect("使える");
        for line in [
            "//!ARG /r",
            "//!ARG rife-v4.22.lite-s1",
            "//!ARG gpu",
            "//!WAY shm",
        ] {
            assert!(chosen.lines().any(|l| l == line), "{line}\n{chosen}");
        }
    }
}
