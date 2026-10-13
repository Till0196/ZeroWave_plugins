// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 文書のプロファイル：A（地上・BS・CS の 2K）か C（ワンセグ、STD-B24 第二分冊 付属 4）。
//! コンポーネント（`data_component_id` 0x000d）でしか分からず、深い所で要るのでプロセスに一つ持つ。

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    /// 地上・BS・CS の 2K（EUC-JP、960×540 か 720×480）。
    #[default]
    A,
    /// ワンセグ（Shift_JIS、240×480）。
    C,
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(profile: Profile) {
    CURRENT.store(profile as u8, Ordering::SeqCst);
}

pub fn current() -> Profile {
    match CURRENT.load(Ordering::SeqCst) {
        1 => Profile::C,
        _ => Profile::A,
    }
}

pub fn is_c() -> bool {
    current() == Profile::C
}
