// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! データ放送の文書を読み、レイアウトして、どこに何をレンダリングするかを返す。

pub mod document;
pub mod media;
pub mod resource;
pub mod runtime;

pub use runtime::bindings::{
    Agent, MAKER_ID, MAKER_OUI, MODEL_ID, browser_name, browser_version_text, maker_id, maker_oui,
    model_id, user_agent,
};
pub use runtime::{Broadcast, Browser, Key, Request};
