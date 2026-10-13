// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 標準エラーへの一行と、準備の進み具合（`@status`）。

use std::io::Write;

use zwsdk::video::process as wire;

pub const MODEL_STATUS: &str = "RIFE のモデル";

pub fn log_line(line: &str) {
    let mut text = String::with_capacity(line.len() + 1);
    text.push_str(line);
    text.push('\n');
    let _ = std::io::stderr().lock().write_all(text.as_bytes());
}

pub fn report_status(progress: Option<f32>, said: &str) {
    log_line(&wire::status(progress, said));
}
