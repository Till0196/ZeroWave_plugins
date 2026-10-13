// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 放送で届いたファイルのストア。DDMT・DAMT・MPU の索引（[`isdb::mhdata`]）を合わせて、
//! `sh4/60/001/top/source/top.js` のようなパスで読めるようにする。

use std::collections::BTreeMap;

use isdb::mhdata::{IndexedItem, asset_table, directory_table, index_item};

pub(crate) const INFLATED_MOST: usize = 32 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub kind: String,
    pub body: Vec<u8>,
    pub version: u8,
}

#[derive(Debug, Default)]
struct Gathering {
    pieces: Vec<Vec<u8>>,
    runs: Vec<Vec<u8>>,
}

impl Gathering {
    fn best(&self, size: u32) -> Option<&Vec<u8>> {
        self.runs
            .iter()
            .find(|run| size != 0 && run.len() == size as usize)
            .or_else(|| self.runs.iter().max_by_key(|run| run.len()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryState {
    pub complete: bool,
    pub mpus: Vec<u32>,
}

#[derive(Debug, Default)]
pub struct Store {
    files: BTreeMap<String, File>,
    directories: BTreeMap<u16, String>,
    nodes: BTreeMap<u32, u16>,
    items: BTreeMap<(u16, u32, u32), Gathering>,
    indexed: BTreeMap<(u16, u32), Vec<IndexedItem>>,
    pub generation: u64,
    pub web: BTreeMap<String, Option<File>>,
    pub web_failed: BTreeMap<String, f64>,
    pub web_used: BTreeMap<String, u64>,
    pub web_clock: u64,
}

/// 通信で取ったものの上限。溢れたら古い順に捨てる。
pub const WEB_BUDGET: usize = 32 << 20;
/// 通信のストアの件数の上限。取れなかった物は 0 バイトなので、大きさだけでは増え続ける。
pub const WEB_MOST: usize = 512;

pub const ITEM_BUDGET: usize = 64 << 20;

const INDEXED_MOST: usize = 1024;
const NODES_MOST: usize = 4096;

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_directory(root: &std::path::Path) -> std::io::Result<Self> {
        let mut store = Self::new();
        fn walk(
            store: &mut Store,
            root: &std::path::Path,
            dir: &std::path::Path,
        ) -> std::io::Result<()> {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    walk(store, root, &path)?;
                } else if let Ok(relative) = path.strip_prefix(root) {
                    let name = relative.to_string_lossy().replace('\\', "/");
                    let kind = mime_of(&name).to_owned();
                    store.files.insert(
                        name,
                        File {
                            kind,
                            body: std::fs::read(&path)?,
                            version: 0,
                        },
                    );
                }
            }
            Ok(())
        }
        walk(&mut store, root, root)?;
        store.generation += 1;
        Ok(store)
    }

    /// DAMT が示す MPU が全部、索引ごと揃ったか。最初の文書は別のコンポーネントを読むので、開くのはそれから。
    pub fn is_complete(&self) -> bool {
        if self.nodes.is_empty() {
            return false;
        }
        self.nodes.iter().all(|(mpu, node)| {
            let Some(directory) = self.directories.get(node) else {
                return false;
            };
            let Some(index) = self
                .indexed
                .iter()
                .find(|((_, seq), _)| seq == mpu)
                .map(|(_, index)| index)
            else {
                return false;
            };
            index.iter().all(|entry| {
                self.files
                    .contains_key(&format!("{directory}{}", entry.name))
            })
        })
    }

    /// このパスの下の状態（`storeDataResource`・`addCacheEventListener` の結果、TR-B39 11.2.3）。
    /// テーブルがパスを知らなければ `None`（知っていたのに `None` になれば `deleted`）。
    pub fn directory_state(&self, path: &str) -> Option<DirectoryState> {
        let path = path.trim_start_matches('/');
        if !path.ends_with('/') && self.files.contains_key(path) {
            let version = self.files.get(path).map_or(0, |f| f.version);
            return Some(DirectoryState {
                complete: true,
                mpus: vec![u32::from(version)],
            });
        }
        let prefix = if path.ends_with('/') {
            path.to_owned()
        } else {
            format!("{path}/")
        };
        let nodes: Vec<u16> = self
            .directories
            .iter()
            .filter(|(_, directory)| directory.starts_with(&prefix))
            .map(|(node, _)| *node)
            .collect();
        if nodes.is_empty() {
            return None;
        }
        let mpus: Vec<u32> = self
            .nodes
            .iter()
            .filter(|(_, node)| nodes.contains(node))
            .map(|(mpu, _)| *mpu)
            .collect();
        if mpus.is_empty() {
            return None;
        }
        let complete = mpus.iter().all(|mpu| {
            let Some(directory) = self
                .nodes
                .get(mpu)
                .and_then(|node| self.directories.get(node))
            else {
                return false;
            };
            let Some(index) = self
                .indexed
                .iter()
                .find(|((_, seq), _)| seq == mpu)
                .map(|(_, index)| index)
            else {
                return false;
            };
            index.iter().all(|entry| {
                self.files
                    .contains_key(&format!("{directory}{}", entry.name))
            })
        });
        Some(DirectoryState { complete, mpus })
    }

    pub fn is_stored(&self, path: &str) -> Option<bool> {
        self.directory_state(path).map(|state| state.complete)
    }

    pub fn get(&self, path: &str) -> Option<&File> {
        self.files.get(path.trim_start_matches('/'))
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    pub fn table(&mut self, section: &[u8]) {
        match section.first() {
            Some(&isdb::mhdata::TABLE_DDMT) => {
                if let Some(table) = directory_table(section) {
                    for node in table.nodes {
                        self.directories
                            .insert(node.tag, format!("{}{}", table.base, node.path));
                    }
                    self.place();
                }
            }
            Some(&isdb::mhdata::TABLE_DAMT) => {
                if let Some(table) = asset_table(section) {
                    for mpu in table.mpus {
                        if let Some(node) = mpu.node {
                            self.nodes.insert(mpu.sequence, node);
                        }
                    }
                    // 選局までため続けないよう、古い（番号の小さい）MPU から忘れる。
                    while self.nodes.len() > NODES_MOST {
                        self.nodes.pop_first();
                    }
                    self.place();
                }
            }
            _ => {}
        }
    }

    pub fn item(&mut self, packet_id: u16, mpu: u32, item_id: u32, fragmentation: u8, data: &[u8]) {
        let gathering = self.items.entry((packet_id, mpu, item_id)).or_default();
        let mut done = false;
        match fragmentation {
            0 => {
                gathering.runs.push(data.to_vec());
                done = true;
            }
            1 => gathering.pieces = vec![data.to_vec()],
            2 => gathering.pieces.push(data.to_vec()),
            _ => {
                gathering.pieces.push(data.to_vec());
                gathering.runs.push(gathering.pieces.concat());
                gathering.pieces.clear();
                done = true;
            }
        }
        if gathering.runs.len() > 3 {
            gathering.runs.remove(0);
        }
        if !done {
            return;
        }
        if item_id == 0
            && let Some(index) = gathering.best(0).and_then(|b| index_item(b))
        {
            self.indexed.insert((packet_id, mpu), index);
            self.trim_indexed();
        }
        self.trim_items();
        self.place();
    }

    /// 索引の数が上限を超えたら、DAMT に載っていない MPU の物から、無ければ古い物から忘れる。
    fn trim_indexed(&mut self) {
        while self.indexed.len() > INDEXED_MOST {
            let key = self
                .indexed
                .keys()
                .find(|(_, mpu)| !self.nodes.contains_key(mpu))
                .or_else(|| self.indexed.keys().next())
                .copied();
            let Some(key) = key else { break };
            self.indexed.remove(&key);
        }
    }

    fn trim_items(&mut self) {
        let size = |g: &Gathering| -> usize {
            g.runs.iter().map(Vec::len).sum::<usize>()
                + g.pieces.iter().map(Vec::len).sum::<usize>()
        };
        let mut total: usize = self.items.values().map(size).sum();
        while total > ITEM_BUDGET {
            let Some(&key) = self
                .items
                .keys()
                .find(|(packet_id, mpu, _)| !self.indexed.contains_key(&(*packet_id, *mpu)))
                .or_else(|| self.items.keys().next())
            else {
                break;
            };
            if let Some(gone) = self.items.remove(&key) {
                total -= size(&gone);
            }
        }
    }

    pub fn touch_web(&mut self, key: &str) {
        self.web_clock += 1;
        self.web_used.insert(key.to_owned(), self.web_clock);
    }

    pub fn put_web(&mut self, key: String, file: Option<File>) {
        self.touch_web(&key);
        self.web.insert(key.clone(), file);
        let size = |f: &Option<File>| f.as_ref().map_or(0, |f| f.body.len());
        let mut total: usize = self.web.values().map(size).sum();
        while total > WEB_BUDGET || self.web.len() > WEB_MOST {
            let Some(victim) = self
                .web
                .keys()
                .filter(|k| **k != key)
                .min_by_key(|k| self.web_used.get(*k).copied().unwrap_or(0))
                .cloned()
            else {
                break;
            };
            if let Some(gone) = self.web.remove(&victim) {
                total -= size(&gone);
            }
            self.web_used.remove(&victim);
            self.web_failed.remove(&victim);
        }
    }

    fn place(&mut self) {
        let mut placed = Vec::new();
        for ((packet_id, mpu), index) in &self.indexed {
            let Some(directory) = self
                .nodes
                .get(mpu)
                .and_then(|node| self.directories.get(node))
            else {
                continue;
            };
            for entry in index {
                let path = format!("{directory}{}", entry.name);
                if self
                    .files
                    .get(&path)
                    .is_some_and(|f| f.version == entry.version)
                {
                    continue;
                }
                let Some(raw) = self
                    .items
                    .get(&(*packet_id, *mpu, entry.id))
                    .and_then(|g| g.best(entry.size))
                else {
                    continue;
                };
                if entry.size != 0 && raw.len() != entry.size as usize {
                    continue;
                }
                let body = if entry.compressed() {
                    // 展開した大きさにも上限（小さな圧縮データが数 GB に伸びないように）。
                    match miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(
                        raw,
                        INFLATED_MOST,
                    ) {
                        Ok(body) => body,
                        Err(_) => continue,
                    }
                } else {
                    raw.clone()
                };
                placed.push((
                    path,
                    File {
                        kind: entry.kind.clone(),
                        body,
                        version: entry.version,
                    },
                ));
            }
        }
        if !placed.is_empty() {
            self.generation += 1;
            for (path, file) in placed {
                self.files.insert(path, file);
            }
        }
    }
}

pub fn mime_of(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    match lower.rsplit('.').next().unwrap_or("") {
        "html" | "htm" => "text/html",
        "js" => "application/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "gif" => "image/gif",
        "ttf" | "otf" | "woff" | "woff2" => "font/*",
        _ => "application/octet-stream",
    }
}
