// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 通信のインターフェース。中身は `zwsdk::net`。

pub use zwsdk::net::{Failure, Reply, TIMEOUT, Url, escape, join, parse_url};

use crate::Agent;

#[cfg(not(target_arch = "wasm32"))]
use zwsdk::net::send as send_raw;
#[cfg(not(target_arch = "wasm32"))]
pub use zwsdk::net::{add_root_certificate, has_ip, hold, reach, release, resolve};

#[cfg(target_arch = "wasm32")]
use guest::send_raw;
#[cfg(target_arch = "wasm32")]
pub use guest::{add_root_certificate, has_ip, hold, reach, release, resolve};

pub fn post(uri: &str, form: &[u8]) -> Result<Reply, Failure> {
    send("POST", uri, &[], Some(form))
}

pub fn get(uri: &str) -> Result<Reply, Failure> {
    send("GET", uri, &[], None)
}

pub fn get_as(agent: Agent, uri: &str) -> Result<Reply, Failure> {
    send_as(agent, "GET", uri, &[], None)
}

pub fn send(
    method: &str,
    uri: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Result<Reply, Failure> {
    send_as(Agent::Bml, method, uri, headers, body)
}

/// 任意のメソッド・ヘッダ・本文のリクエスト。User-Agent は文書の物を捨て、
/// `agent` の設定にあるときだけ付ける（TR-B39 12.2.1.3）。
pub fn send_as(
    agent: Agent,
    method: &str,
    uri: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> Result<Reply, Failure> {
    let mut sent: Vec<(String, String)> = headers
        .iter()
        .filter(|(name, _)| !name.eq_ignore_ascii_case("user-agent"))
        .cloned()
        .collect();
    if let Some(agent) = crate::user_agent(agent) {
        sent.push((String::from("User-Agent"), agent));
    }
    send_raw(method, uri, &sent, body)
}

#[cfg(target_arch = "wasm32")]
mod guest {
    use super::{Failure, Reply};
    use std::time::Duration;

    fn failure(code: i32) -> Failure {
        match code {
            -1 => Failure::Argument,
            -3 => Failure::Timeout,
            -300 => Failure::Connect,
            -400 => Failure::Name,
            -500 => Failure::Tls,
            _ => Failure::Disconnected,
        }
    }

    fn convert(reply: zwsdk::plug::host::Reply) -> Reply {
        Reply {
            status: reply.status,
            body: reply.body,
            content_type: reply.content_type,
            location: reply.location,
            headers: reply.headers,
        }
    }

    pub fn has_ip() -> bool {
        zwsdk::plug::host::net::has_ip()
    }

    pub fn resolve(host: &str) -> Option<String> {
        zwsdk::plug::host::net::resolve(host)
    }

    pub fn reach(host: &str, port: u16, timeout: Duration) -> Option<(String, u32)> {
        zwsdk::plug::host::net::reach(
            host,
            port,
            timeout.as_millis().min(u128::from(u32::MAX)) as u32,
        )
    }

    pub fn send_raw(
        method: &str,
        uri: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
    ) -> Result<Reply, Failure> {
        zwsdk::plug::host::net::request(method, uri, headers, body)
            .map(convert)
            .map_err(failure)
    }

    pub fn add_root_certificate(der: &[u8]) -> bool {
        zwsdk::plug::host::net::add_root_certificate(der)
    }

    pub fn hold() {}
    pub fn release() {}
}
