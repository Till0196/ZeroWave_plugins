// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! MMT のデータ放送（ARIB-HTML5、STD-B62）を動かす。
//! 文書は Blitz がレイアウトし、スクリプトは boa が実行し、
//! 受像機の API は [`api`] と `js/prelude.js` で足す。

pub mod api;
pub mod app;
pub mod draw;
pub mod fonts;
pub mod net;
pub mod paint;
pub mod picture;
pub mod script;
pub mod still;
pub mod store;
