// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 文書が読み込むものをストアから。放送のファイルは `arib-dc://` の名前で、
//! CSS・画像・スクリプト・XMLHttpRequest のどれもここを通る（TR-B39 第 3 編 6.1.3.6）。

use std::sync::{Arc, RwLock};

use blitz_traits::net::{NetHandler, NetProvider, Request};
use blitz_vibey_script::{FetchError, ScriptFetcher};
use url::Url;

use crate::store::Store;

pub const BASE: &str = "arib-dc://localhost/";

#[derive(Debug, Clone)]
pub struct Shared(pub Arc<RwLock<Store>>);

impl Shared {
    pub fn new(store: Store) -> Self {
        Self(Arc::new(RwLock::new(store)))
    }

    pub fn fetch(&self, url: &Url) -> Option<(String, Vec<u8>)> {
        if url.scheme() == "data" {
            let parsed = data_url::DataUrl::process(url.as_str()).ok()?;
            let (body, _) = parsed.decode_to_vec().ok()?;
            let kind = format!(
                "{}/{}",
                parsed.mime_type().type_,
                parsed.mime_type().subtype
            );
            return Some((kind, body));
        }
        if url.scheme() == "http" || url.scheme() == "https" {
            return self.fetch_web(url.as_str());
        }
        if url.scheme() != "arib-dc" {
            return None;
        }
        let path = url.path().trim_start_matches('/');
        let store = self.0.read().unwrap_or_else(|e| e.into_inner());
        store
            .get(path)
            .map(|file| (file.kind.clone(), file.body.clone()))
    }

    fn fetch_web(&self, url: &str) -> Option<(String, Vec<u8>)> {
        let key = url.split('#').next().unwrap_or("").to_owned();
        {
            let store = self.0.read().unwrap_or_else(|e| e.into_inner());
            if let Some(had) = store.web.get(&key) {
                // 失敗は三十秒で取り直す（後から通るようになることがある）。
                let stale = had.is_none()
                    && store
                        .web_failed
                        .get(&key)
                        .is_none_or(|at| zwsdk::plug::host::monotonic_ms() - at > 30_000.0);
                if !stale {
                    let out = had
                        .as_ref()
                        .map(|file| (file.kind.clone(), file.body.clone()));
                    drop(store);
                    self.0
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .touch_web(&key);
                    return out;
                }
            }
        }
        let got = match bml::resource::net::get_as(crate::api::agent(), &key) {
            Ok(reply) if reply.status == 200 => {
                let name = key.split(['?', '#']).next().unwrap_or("");
                let kind = reply
                    .content_type
                    .as_deref()
                    .map(|k| k.split(';').next().unwrap_or("").trim().to_owned())
                    .filter(|k| !k.is_empty() && k != "application/octet-stream")
                    .unwrap_or_else(|| crate::store::mime_of(name).to_owned());
                Some(crate::store::File {
                    kind,
                    body: reply.body,
                    version: 0,
                })
            }
            Ok(reply) => {
                zwsdk::plug::info(&format!("通信  {url} → {}", reply.status));
                None
            }
            Err(failure) => {
                zwsdk::plug::info(&format!("通信  {url} → {failure:?}"));
                None
            }
        };
        let mut store = self.0.write().unwrap_or_else(|e| e.into_inner());
        let out = got
            .as_ref()
            .map(|file| (file.kind.clone(), file.body.clone()));
        if got.is_none() {
            store
                .web_failed
                .insert(key.clone(), zwsdk::plug::host::monotonic_ms());
        }
        store.put_web(key, got);
        out
    }
}

impl NetProvider for Shared {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        if let Some((_, body)) = Shared::fetch(self, &request.url) {
            handler.bytes(request.url.to_string(), body.into());
        }
    }
}

impl ScriptFetcher for Shared {
    fn fetch(&self, url: &Url) -> Result<String, FetchError> {
        match Shared::fetch(self, url) {
            Some((_, body)) => Ok(bml::runtime::script::rewrite_length(
                &String::from_utf8_lossy(&body),
            )),
            None => Err(FetchError::InvalidData(format!("無い: {url}"))),
        }
    }
}
