// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! プリセット・自動選択・シェーダーの一覧・通知の設定。自動選択は上から当てて最初に合ったもの、
//! どれにも合わなければ [`Filter::fallback`]。設定ファイルが無ければどれも空（何も掛けない）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use zwsdk::video::{At, Format, Program};

pub const PRESETS: &str = "presets";
pub const AUTOS: &str = "autos";
pub const SHADERS: &str = "shaders";
pub const NOTIFY: &str = "notify";
pub const FALLBACK: &str = "fallback";
pub const PLAYBACK: &str = "playback";
pub const MANUAL: &str = "manual";

pub const DEINTERLACE_DIR: &str = "deinterlace";
pub const MOST_BUFFER_MS: u32 = 2000;

/// 「先行レンダリングのバッファ」が自動のとき、本体のオプティカルフローで補間するならバッファする長さ。
pub const AUTO_BUFFER_MS: u32 = 250;

/// 本体が完了を待たずに要求し、フレームを遅らせて出す補間か（AMF・RIFE の外部プロセス）。
pub fn lags(file: &str) -> bool {
    crate::hosted::entry(file).is_some_and(|entry| {
        entry.job == zwsdk::video::HOST_INTERPOLATE
            && ["amf", zwsdk::video::HOST_PROCESS].contains(&entry.who)
    })
}

pub const MOST_HELD: u32 = 2;

/// 新しい入力の一枚だけが重い補間か（本体のオプティカルフローの `@flow`・`@nvofa`、GPU の RIFE）。
fn host_flow(file: &str) -> bool {
    file == crate::rife_gpu::FILE
        || crate::hosted::entry(file).is_some_and(|entry| entry.job == zwsdk::video::HOST_FLOW)
}

pub const INTERPOLATE_DIR: &str = "interpolate";

#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    pub playback: bool,
    pub presets: Vec<Preset>,
    pub autos: Vec<Auto>,
    pub fallback: String,
    pub shaders: Vec<Shader>,
    pub notify: Notify,
    pub manual: Option<Manual>,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            playback: true,
            presets: Vec::new(),
            autos: Vec::new(),
            fallback: String::new(),
            shaders: Vec::new(),
            notify: Notify::default(),
            manual: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Preset {
    pub name: String,
    /// デインタレースのシェーダー（`deinterlace/` の名前）。空なら内蔵（bwdif）。
    pub deinterlace: String,
    /// フィルター。上から順に掛かり、フレーム補間も一つのフィルターとして並ぶ。
    pub filters: Vec<Item>,
    /// 補間の目標：空なら画面のリフレッシュレート、でなければ fps の数（`60`）。
    pub target: String,
    /// 先行レンダリングのバッファの長さ（ミリ秒、`0` なら使わない）。空は自動（[`AUTO_BUFFER_MS`]）。
    pub buffer: String,
    pub keep: bool,
    /// 本体の補間がフレームを何入力遅らせて出すか：空は本体に任せる、`1` か `2`。
    /// 長いほど中間フレームが間に合い、映像と音が遅れる。
    pub held: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Shader {
    pub file: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Notify {
    pub preset: bool,
    pub rule: bool,
    pub chain: bool,
    pub size: bool,
    pub manual: bool,
    pub quiet_same: bool,
    pub compile: bool,
    pub built: bool,
    pub paused: bool,
    pub error: bool,
    pub lines: u8,
    pub extend: bool,
}

impl Default for Notify {
    fn default() -> Self {
        Self {
            preset: true,
            rule: true,
            chain: true,
            size: false,
            manual: true,
            quiet_same: true,
            compile: true,
            built: false,
            paused: true,
            error: true,
            lines: 3,
            extend: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Item {
    pub file: String,
    pub on: bool,
}

impl Default for Item {
    fn default() -> Self {
        Self {
            file: String::new(),
            on: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Auto {
    pub name: String,
    pub any: bool,
    #[serde(rename = "when")]
    pub conditions: Vec<Condition>,
    pub preset: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manual {
    pub preset: Option<String>,
    pub channel: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Condition {
    pub kind: String,
    pub op: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 値は大分類 `"7"` か大分類と中分類 `"7.0"`（十六進）。`any` か `none`。
    Genre,
    /// 値はネットワークの表示名（本体の設定の `[[network]]`）。`any` か `none`。
    Network,
    /// 値は `"ネットワーク:サービス"`（十六進）。`any` か `none`。
    Channel,
    /// 値は [`Size`] の綴り一つ。`le`・`ge`・`eq`。
    Resolution,
    Scan,
    /// ピクチャー構造（MPEG-2）。値は `frame` か `field`。分からなければ合わない。
    Structure,
    Title,
    /// 時間帯。値は `["23:00", "5:00", "every"|"weekday"|"weekend"]`。
    Time,
    Source,
}

impl Kind {
    pub const ALL: [Self; 9] = [
        Self::Genre,
        Self::Network,
        Self::Channel,
        Self::Resolution,
        Self::Scan,
        Self::Structure,
        Self::Title,
        Self::Time,
        Self::Source,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Genre => "genre",
            Self::Network => "network",
            Self::Channel => "channel",
            Self::Resolution => "resolution",
            Self::Scan => "scan",
            Self::Structure => "structure",
            Self::Title => "title",
            Self::Time => "time",
            Self::Source => "source",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Genre => "ジャンル",
            Self::Network => "ネットワーク",
            Self::Channel => "チャンネル",
            Self::Resolution => "解像度",
            Self::Scan => "インタレース／プログレッシブ",
            Self::Structure => "ピクチャー構造",
            Self::Title => "番組名",
            Self::Time => "時間帯",
            Self::Source => "放送・録画",
        }
    }

    pub fn ops(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Genre => &[("any", "含む"), ("none", "含まない")],
            Self::Network | Self::Channel => &[("any", "いずれか"), ("none", "以外")],
            Self::Resolution => &[("le", "以下"), ("ge", "以上"), ("eq", "ちょうど")],
            Self::Scan | Self::Structure | Self::Source => &[("is", "")],
            Self::Title => &[("contains", "含む")],
            Self::Time => &[("between", "")],
        }
    }

    pub fn fresh(self) -> Condition {
        let values: Vec<String> = match self {
            Self::Genre | Self::Network | Self::Channel | Self::Title => Vec::new(),
            Self::Resolution => vec![Size::Hd.name().to_owned()],
            Self::Scan => vec![String::from("interlace")],
            Self::Structure => vec![String::from("field")],
            Self::Time => vec![
                String::from("23:00"),
                String::from("5:00"),
                String::from("every"),
            ],
            Self::Source => vec![String::from("broadcast")],
        };
        Condition {
            kind: self.name().to_owned(),
            op: self.ops()[0].0.to_owned(),
            values,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    OneSeg,
    Sd,
    Hd,
    FullHd,
    Uhd4k,
    Uhd8k,
}

impl Size {
    pub const ALL: [Self; 6] = [
        Self::OneSeg,
        Self::Sd,
        Self::Hd,
        Self::FullHd,
        Self::Uhd4k,
        Self::Uhd8k,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::OneSeg => "1seg",
            Self::Sd => "sd",
            Self::Hd => "hd",
            Self::FullHd => "fhd",
            Self::Uhd4k => "4k",
            Self::Uhd8k => "8k",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|size| size.name() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::OneSeg => "ワンセグ（320×180）",
            Self::Sd => "SD（720×480）",
            Self::Hd => "HD（1440×1080）",
            Self::FullHd => "フル HD（1920×1080）",
            Self::Uhd4k => "4K（3840×2160）",
            Self::Uhd8k => "8K（7680×4320）",
        }
    }

    /// 画素の数。比べるのはこれ（1440×1080 と 1920×1080 を分ける）。
    fn pixels(self) -> u64 {
        match self {
            Self::OneSeg => 320 * 180,
            Self::Sd => 720 * 480,
            Self::Hd => 1440 * 1080,
            Self::FullHd => 1920 * 1080,
            Self::Uhd4k => 3840 * 2160,
            Self::Uhd8k => 7680 * 4320,
        }
    }

    pub fn of(width: u32, height: u32) -> Self {
        let pixels = u64::from(width) * u64::from(height);
        Self::ALL
            .into_iter()
            .min_by_key(|size| size.pixels().abs_diff(pixels))
            .unwrap_or(Self::Hd)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    pub program: &'a Program,
    pub format: Option<&'a Format>,
}

pub fn channel_key(network: u16, service: u16) -> String {
    format!("{network:04x}:{service:04x}")
}

impl Condition {
    pub fn matches(&self, target: Target<'_>) -> bool {
        let Some(kind) = Kind::parse(&self.kind) else {
            return false;
        };
        let program = target.program;
        let negate = self.op == "none";
        match kind {
            Kind::Genre => {
                let hit = self.values.iter().any(|value| {
                    let (large, middle) = genre_parse(value);
                    program
                        .genres
                        .iter()
                        .any(|&(l, m)| Some(l) == large && middle.is_none_or(|middle| middle == m))
                });
                hit != negate
            }
            Kind::Network => {
                let hit = !program.band.is_empty() && self.values.contains(&program.band);
                hit != negate
            }
            Kind::Channel => {
                let here = channel_key(program.network, program.service);
                let hit = self.values.contains(&here);
                hit != negate
            }
            Kind::Resolution => {
                let Some(format) = target.format.filter(|f| f.width > 0 && f.height > 0) else {
                    return false;
                };
                let Some(size) = self.values.first().and_then(|v| Size::parse(v)) else {
                    return false;
                };
                let have = Size::of(format.width, format.height).pixels();
                match self.op.as_str() {
                    "ge" => have >= size.pixels(),
                    "eq" => have == size.pixels(),
                    _ => have <= size.pixels(),
                }
            }
            Kind::Scan => {
                let Some(format) = target.format else {
                    return false;
                };
                let want = self.values.first().map(String::as_str) == Some("progressive");
                !format.interlaced == want
            }
            Kind::Structure => {
                let want = match self.values.first().map(String::as_str) {
                    Some("frame") => zwsdk::video::Structure::Frame,
                    _ => zwsdk::video::Structure::Field,
                };
                program.structure == want
            }
            Kind::Title => self
                .values
                .iter()
                .any(|word| !word.is_empty() && program.title.contains(word.as_str())),
            Kind::Time => {
                let (Some(from), Some(to)) = (
                    self.values.first().and_then(|v| clock(v)),
                    self.values.get(1).and_then(|v| clock(v)),
                ) else {
                    return false;
                };
                let weekday = program.weekday();
                let day_ok = match self.values.get(2).map(String::as_str) {
                    Some("weekday") => weekday < 5,
                    Some("weekend") => weekday >= 5,
                    _ => true,
                };
                let now = program.minute_of_day();
                let inside = if from <= to {
                    (from..to).contains(&now)
                } else {
                    now >= from || now < to
                };
                day_ok && inside
            }
            Kind::Source => {
                let want = self.values.first().map(String::as_str) == Some("recording");
                program.recording == want
            }
        }
    }

    pub fn summary(&self, channel_name: &dyn Fn(&str) -> String) -> String {
        let Some(kind) = Kind::parse(&self.kind) else {
            return self.kind.clone();
        };
        let op = kind
            .ops()
            .iter()
            .find(|(name, _)| *name == self.op)
            .map_or("", |(_, label)| *label);
        let values = match kind {
            Kind::Genre => self
                .values
                .iter()
                .map(|value| genre_label(value))
                .collect::<Vec<_>>()
                .join("・"),
            Kind::Network | Kind::Title => self.values.join("・"),
            Kind::Channel => self
                .values
                .iter()
                .map(|value| channel_name(value))
                .collect::<Vec<_>>()
                .join("・"),
            Kind::Resolution => self
                .values
                .first()
                .and_then(|v| Size::parse(v))
                .map_or_else(String::new, |s| s.label().to_owned()),
            Kind::Scan => match self.values.first().map(String::as_str) {
                Some("progressive") => String::from("プログレッシブ"),
                _ => String::from("インターレース"),
            },
            Kind::Source => match self.values.first().map(String::as_str) {
                Some("recording") => String::from("録画"),
                _ => String::from("放送"),
            },
            Kind::Structure => {
                structure_label(self.values.first().map_or("field", String::as_str)).to_owned()
            }
            Kind::Time => format!(
                "{}〜{}（{}）",
                self.values.first().map_or("", String::as_str),
                self.values.get(1).map_or("", String::as_str),
                day_label(self.values.get(2).map_or("every", String::as_str))
            ),
        };
        if values.is_empty() {
            return format!("{}：未選択", kind.label());
        }
        match kind {
            Kind::Genre | Kind::Network | Kind::Channel => {
                if self.op == "none" {
                    format!("{}：{values} 以外", kind.label())
                } else {
                    format!("{}：{values}", kind.label())
                }
            }
            Kind::Resolution => format!("{}：{values} {op}", kind.label()),
            Kind::Title => format!("{}：「{values}」を含む", kind.label()),
            Kind::Source => values,
            _ => format!("{}：{values}", kind.label()),
        }
    }
}

pub fn structure_label(value: &str) -> &'static str {
    match value {
        "frame" => "フレームピクチャー",
        _ => "フィールドピクチャー",
    }
}

pub fn day_label(day: &str) -> &'static str {
    match day {
        "weekday" => "平日",
        "weekend" => "土日",
        _ => "毎日",
    }
}

pub fn clock(text: &str) -> Option<u16> {
    let (h, m) = text.split_once(':')?;
    let (h, m) = (h.trim().parse::<u16>().ok()?, m.trim().parse::<u16>().ok()?);
    (h <= 24 && m < 60).then_some(h * 60 + m)
}

pub fn genre_parse(value: &str) -> (Option<u8>, Option<u8>) {
    let mut parts = value.split('.');
    let large = parts.next().and_then(|t| u8::from_str_radix(t, 16).ok());
    let middle = parts.next().and_then(|t| u8::from_str_radix(t, 16).ok());
    (large, middle)
}

pub fn genre_label(value: &str) -> String {
    let (large, middle) = genre_parse(value);
    let Some(large) = large else {
        return value.to_owned();
    };
    let name = crate::genre::large_name(large);
    match middle.and_then(|middle| crate::genre::middle_name(large, middle)) {
        Some(middle) => format!("{name} › {middle}"),
        None => name.to_owned(),
    }
}

impl Auto {
    pub fn matches(&self, target: Target<'_>) -> bool {
        if self.conditions.is_empty() {
            return true;
        }
        if self.any {
            self.conditions.iter().any(|c| c.matches(target))
        } else {
            self.conditions.iter().all(|c| c.matches(target))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Chosen {
    pub auto: Option<(String, String)>,
    pub manual: bool,
    pub preset: Option<String>,
}

impl Filter {
    pub fn load(settings: &BTreeMap<String, String>) -> Self {
        fn json<T: serde::de::DeserializeOwned + Default>(
            settings: &BTreeMap<String, String>,
            key: &str,
        ) -> T {
            settings
                .get(key)
                .and_then(|text| serde_json::from_str(text).ok())
                .unwrap_or_default()
        }
        Self {
            playback: settings
                .get(PLAYBACK)
                .is_none_or(|on| zwsdk::abi::truthy(on)),
            presets: json(settings, PRESETS),
            autos: json(settings, AUTOS),
            fallback: settings.get(FALLBACK).cloned().unwrap_or_default(),
            shaders: json(settings, SHADERS),
            notify: json(settings, NOTIFY),
            manual: settings.get(MANUAL).and_then(|text| {
                let (preset, channel) = text.split_once('\t')?;
                Some(Manual {
                    preset: (!preset.is_empty()).then(|| preset.to_owned()),
                    channel: channel.to_owned(),
                })
            }),
        }
    }

    pub fn values(&self) -> Vec<(&'static str, String)> {
        let json = |value: Result<String, serde_json::Error>| value.unwrap_or_default();
        vec![
            (PLAYBACK, self.playback.to_string()),
            (PRESETS, json(serde_json::to_string(&self.presets))),
            (AUTOS, json(serde_json::to_string(&self.autos))),
            (FALLBACK, self.fallback.clone()),
            (SHADERS, json(serde_json::to_string(&self.shaders))),
            (NOTIFY, json(serde_json::to_string(&self.notify))),
        ]
    }

    pub fn manual_value(&self) -> String {
        self.manual
            .as_ref()
            .map(|manual| {
                format!(
                    "{}\t{}",
                    manual.preset.as_deref().unwrap_or(""),
                    manual.channel
                )
            })
            .unwrap_or_default()
    }

    pub fn preset(&self, name: &str) -> Option<&Preset> {
        self.presets.iter().find(|preset| preset.name == name)
    }

    /// 合った自動選択とそのプリセット（`None` は「適用しない」）。消えたプリセットは飛ばす。
    pub fn auto_for(&self, target: Target<'_>) -> Option<(&Auto, Option<&Preset>)> {
        self.autos
            .iter()
            .filter(|auto| auto.matches(target))
            .find_map(|auto| {
                if auto.preset.is_empty() {
                    return Some((auto, None));
                }
                Some((auto, Some(self.preset(&auto.preset)?)))
            })
    }

    pub fn manual_here(&self, program: &Program) -> Option<Option<&Preset>> {
        let manual = self.manual.as_ref()?;
        if manual.channel != channel_key(program.network, program.service) {
            return None;
        }
        Some(manual.preset.as_deref().and_then(|name| self.preset(name)))
    }

    pub fn choose(&self, target: Option<Target<'_>>) -> Chosen {
        let Some(target) = target else {
            return Chosen::default();
        };
        if target.program.recording && !self.playback {
            return Chosen::default();
        }
        if let Some(manual) = self.manual_here(target.program) {
            return Chosen {
                auto: None,
                manual: true,
                preset: manual.map(|preset| preset.name.clone()),
            };
        }
        if let Some((auto, preset)) = self.auto_for(target) {
            let names = |key: &str| {
                if channel_key(target.program.network, target.program.service) == key {
                    target.program.channel.clone()
                } else {
                    key.to_owned()
                }
            };
            let why = auto
                .conditions
                .iter()
                .filter(|c| c.matches(target))
                .map(|c| c.summary(&names))
                .collect::<Vec<_>>()
                .join(if auto.any { "・" } else { "、" });
            return Chosen {
                auto: Some((auto.name.clone(), why)),
                manual: false,
                preset: preset.map(|preset| preset.name.clone()),
            };
        }
        Chosen {
            auto: None,
            manual: false,
            preset: self
                .preset(&self.fallback)
                .map(|preset| preset.name.clone()),
        }
    }

    /// フック `at` に掛けるファイルの並び（`|` がグループの区切り）。mpv のシェーダーはグループの中で
    /// 並べ替わるので、フィルターは一つずつグループに分ける。
    pub fn chain(&self, chosen: &Chosen, at: At) -> String {
        self.files(chosen, at).join(" | ")
    }

    pub fn files(&self, chosen: &Chosen, at: At) -> Vec<String> {
        let Some(preset) = chosen.preset.as_deref().and_then(|name| self.preset(name)) else {
            return Vec::new();
        };
        match at {
            At::Deinterlace => {
                let file = preset.deinterlace.trim();
                if file.is_empty() {
                    Vec::new()
                } else {
                    vec![format!("{DEINTERLACE_DIR}/{file}")]
                }
            }
            At::Picture => preset
                .filters
                .iter()
                .filter(|item| item.on && !item.file.trim().is_empty())
                .map(|item| item.file.trim().to_owned())
                .collect(),
        }
    }

    pub fn buffer_ms(&self, chosen: &Chosen) -> u32 {
        chosen
            .preset
            .as_deref()
            .and_then(|name| self.preset(name))
            .map_or(0, |preset| match preset.buffer.trim() {
                "" if preset
                    .filters
                    .iter()
                    .any(|item| item.on && host_flow(&item.file)) =>
                {
                    AUTO_BUFFER_MS
                }
                text => text.parse::<u32>().unwrap_or(0),
            })
            .min(MOST_BUFFER_MS)
    }

    pub fn held(&self, chosen: &Chosen) -> u32 {
        chosen
            .preset
            .as_deref()
            .and_then(|name| self.preset(name))
            .and_then(|preset| preset.held.trim().parse::<u32>().ok())
            .map_or(0, |held| held.min(MOST_HELD))
    }

    pub fn keeps(&self, chosen: &Chosen) -> bool {
        chosen
            .preset
            .as_deref()
            .and_then(|name| self.preset(name))
            .is_some_and(|preset| preset.keep)
    }

    pub fn retime(&self, chosen: &Chosen, display: f64) -> f64 {
        let fallback = if display > 1.0 { display } else { 60.0 };
        chosen
            .preset
            .as_deref()
            .and_then(|name| self.preset(name))
            .and_then(|preset| preset.target.parse::<f64>().ok())
            .filter(|fps| *fps > 1.0)
            .unwrap_or(fallback)
    }

    pub fn describe(&self, file: &str) -> (String, String) {
        match self.shaders.iter().find(|shader| shader.file == file) {
            Some(shader) => (
                if shader.name.is_empty() {
                    file.rsplit('/').next().unwrap_or(file).to_owned()
                } else {
                    shader.name.clone()
                },
                if shader.kind.is_empty() {
                    UNSET.to_owned()
                } else {
                    shader.kind.clone()
                },
            ),
            None => match crate::hosted::entry(file) {
                Some(entry) => (entry.name.to_owned(), String::from("補間")),
                None => (
                    file.rsplit('/').next().unwrap_or(file).to_owned(),
                    UNSET.to_owned(),
                ),
            },
        }
    }

    pub fn uses_of_preset(&self, name: &str) -> usize {
        self.autos.iter().filter(|auto| auto.preset == name).count()
            + usize::from(self.fallback == name)
    }

    pub fn uses_of_shader(&self, file: &str) -> usize {
        let deinterlace = file.strip_prefix(&format!("{DEINTERLACE_DIR}/"));
        self.presets
            .iter()
            .filter(|preset| match deinterlace {
                Some(name) => preset.deinterlace == name,
                None => preset.filters.iter().any(|item| item.file == file),
            })
            .count()
    }

    pub fn interpolates(&self, chosen: &Chosen) -> bool {
        chosen
            .preset
            .as_deref()
            .and_then(|name| self.preset(name))
            .is_some_and(|preset| {
                preset
                    .filters
                    .iter()
                    .any(|item| item.on && is_interpolation(&item.file))
            })
    }

    /// フレーム補間を足す既定の位置：最後の「ノイズ除去」「レストア」の後、最初の「拡大」の前。
    pub fn interpolation_slot(&self, preset: &Preset) -> usize {
        let kinds: Vec<String> = preset
            .filters
            .iter()
            .map(|item| self.describe(&item.file).1)
            .collect();
        let first_upscale = kinds.iter().position(|kind| kind == "拡大");
        let last_clean = kinds
            .iter()
            .rposition(|kind| kind == "ノイズ除去" || kind == "レストア")
            .map(|at| at + 1);
        match (last_clean, first_upscale) {
            (Some(clean), Some(up)) => clean.min(up),
            (Some(clean), None) => clean,
            (None, Some(up)) => up,
            (None, None) => preset.filters.len(),
        }
    }
}

pub fn is_interpolation(file: &str) -> bool {
    file.starts_with(&format!("{INTERPOLATE_DIR}/"))
}

pub const UNSET: &str = "未設定";

pub const KINDS: [&str; 7] = [
    "拡大",
    "ノイズ除去",
    "レストア",
    "シャープ",
    "補間",
    "デインタレース",
    "その他",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn program(large: u8) -> Program {
        Program {
            genres: vec![(large, 0)],
            network: 0x7fef,
            service: 0x0400,
            band: String::from("GR"),
            channel: String::from("[局]"),
            title: String::from("[番組名]"),
            recording: false,
            structure: zwsdk::video::Structure::Unknown,
            minute: 6 * 1440 + 20 * 60,
        }
    }

    fn format(width: u32, height: u32, interlaced: bool) -> Format {
        Format {
            width,
            height,
            depth: 8,
            interlaced,
            aspect: [16, 9],
            transfer: String::from("sdr"),
        }
    }

    fn when(kind: Kind, op: &str, values: &[&str]) -> Condition {
        Condition {
            kind: kind.name().to_owned(),
            op: op.to_owned(),
            values: values.iter().map(|v| (*v).to_owned()).collect(),
        }
    }

    fn preset(name: &str, deinterlace: &str, files: &[&str]) -> Preset {
        Preset {
            name: name.to_owned(),
            deinterlace: deinterlace.to_owned(),
            filters: files
                .iter()
                .map(|file| Item {
                    file: (*file).to_owned(),
                    on: true,
                })
                .collect(),
            ..Preset::default()
        }
    }

    fn filter() -> Filter {
        Filter {
            presets: vec![
                preset("アニメ", "tgmc.wgsl", &["a.glsl", "b.hook"]),
                preset("実写", "", &["b.hook"]),
            ],
            autos: vec![
                Auto {
                    name: String::from("4K"),
                    any: false,
                    conditions: vec![when(Kind::Resolution, "ge", &["4k"])],
                    preset: String::from("無い物"),
                },
                Auto {
                    name: String::from("アニメ全般"),
                    any: false,
                    conditions: vec![when(Kind::Genre, "any", &["7"])],
                    preset: String::from("アニメ"),
                },
            ],
            fallback: String::from("実写"),
            ..Filter::default()
        }
    }

    #[test]
    fn an_empty_buffer_buffers_only_for_host_flow() {
        let mut f = Filter {
            presets: vec![
                preset("流れ", "", &["interpolate/@flow.wgsl"]),
                preset("NVOFA 止め", "", &["interpolate/@nvofa.wgsl"]),
                preset("oneVPL", "", &["interpolate/@vpl.wgsl"]),
                preset("シェーダー", "", &["b.hook"]),
            ],
            ..Filter::default()
        };
        f.presets[1].filters[0].on = false;
        let ms = |f: &Filter, name: &str| {
            f.buffer_ms(&Chosen {
                preset: Some(name.to_owned()),
                ..Chosen::default()
            })
        };
        assert_eq!(ms(&f, "流れ"), AUTO_BUFFER_MS);
        assert_eq!(ms(&f, "NVOFA 止め"), 0);
        assert_eq!(ms(&f, "oneVPL"), 0);
        assert_eq!(ms(&f, "シェーダー"), 0);
        f.presets[0].buffer = String::from("0");
        assert_eq!(ms(&f, "流れ"), 0);
        f.presets[0].buffer = String::from("500");
        assert_eq!(ms(&f, "流れ"), 500);
    }

    #[test]
    fn the_held_inputs_come_from_the_preset() {
        let mut f = Filter {
            presets: vec![preset("RIFE", "", &["interpolate/@rife.wgsl"])],
            ..Filter::default()
        };
        let held = |f: &Filter| {
            f.held(&Chosen {
                preset: Some(String::from("RIFE")),
                ..Chosen::default()
            })
        };
        assert_eq!(held(&f), 0, "空は本体に任せる");
        f.presets[0].held = String::from("2");
        assert_eq!(held(&f), 2);
        f.presets[0].held = String::from("5");
        assert_eq!(held(&f), MOST_HELD);
        assert!(lags("interpolate/@rife.wgsl") && lags("interpolate/@amf.wgsl"));
        assert!(!lags("interpolate/@vpl.wgsl") && !lags("interpolate/@flow.wgsl"));
    }

    #[test]
    fn nothing_is_applied_without_a_settings_file() {
        let empty = Filter::load(&BTreeMap::new());
        assert_eq!(empty, Filter::default());
        assert!(empty.presets.is_empty() && empty.autos.is_empty());
        let p = program(7);
        let hd = format(1440, 1080, true);
        let chosen = empty.choose(Some(Target {
            program: &p,
            format: Some(&hd),
        }));
        assert_eq!(chosen.preset, None);
    }

    #[test]
    fn an_auto_that_applies_nothing_stops_there() {
        let mut f = filter();
        f.autos[0].preset = String::new();
        let uhd = format(3840, 2160, false);
        let anime = program(7);
        let chosen = f.choose(Some(Target {
            program: &anime,
            format: Some(&uhd),
        }));
        assert_eq!(chosen.preset, None, "4K では掛けない");
        assert_eq!(
            chosen.auto.as_ref().map(|(name, _)| name.as_str()),
            Some("4K")
        );
        let hd = format(1440, 1080, true);
        let chosen = f.choose(Some(Target {
            program: &anime,
            format: Some(&hd),
        }));
        assert_eq!(chosen.preset.as_deref(), Some("アニメ"));
    }

    #[test]
    fn autos_then_the_fallback_then_manual() {
        let mut f = filter();
        let hd = format(1440, 1080, true);
        let anime = program(7);
        let t = |p| Target {
            program: p,
            format: Some(&hd),
        };
        let chosen = f.choose(Some(t(&anime)));
        assert_eq!(chosen.preset.as_deref(), Some("アニメ"));
        assert_eq!(
            chosen.auto,
            Some((
                String::from("アニメ全般"),
                String::from("ジャンル：アニメ／特撮")
            ))
        );
        let news = program(0);
        assert_eq!(f.choose(Some(t(&news))).preset.as_deref(), Some("実写"));
        assert_eq!(
            f.choose(None),
            Chosen::default(),
            "番組が分からなければ掛けない"
        );
        f.manual = Some(Manual {
            preset: None,
            channel: channel_key(0x7fef, 0x0400),
        });
        let manual = f.choose(Some(t(&anime)));
        assert!(manual.manual && manual.preset.is_none());
        let other = Program {
            service: 0x0401,
            ..program(7)
        };
        assert_eq!(f.choose(Some(t(&other))).preset.as_deref(), Some("アニメ"));
    }

    #[test]
    fn conditions_read_their_values() {
        let p = program(5);
        let hd = format(1440, 1080, true);
        let t = Target {
            program: &p,
            format: Some(&hd),
        };
        for (c, want) in [
            (when(Kind::Genre, "any", &["5"]), true),
            (when(Kind::Genre, "any", &["5.1"]), false),
            (when(Kind::Genre, "none", &["7"]), true),
            (when(Kind::Network, "any", &["BS", "GR"]), true),
            (when(Kind::Network, "none", &["GR"]), false),
            (when(Kind::Channel, "any", &["7fef:0400"]), true),
            (when(Kind::Resolution, "le", &["hd"]), true),
            (when(Kind::Resolution, "eq", &["fhd"]), false),
            (when(Kind::Scan, "is", &["interlace"]), true),
            (when(Kind::Title, "contains", &["番組"]), true),
            (
                when(Kind::Time, "between", &["19:00", "21:00", "weekday"]),
                true,
            ),
            (
                when(Kind::Time, "between", &["23:00", "5:00", "every"]),
                false,
            ),
            (when(Kind::Source, "is", &["broadcast"]), true),
            (when(Kind::Structure, "is", &["field"]), false),
        ] {
            assert_eq!(c.matches(t), want, "{c:?}");
        }
        let field = Program {
            structure: zwsdk::video::Structure::Field,
            ..program(5)
        };
        assert!(when(Kind::Structure, "is", &["field"]).matches(Target {
            program: &field,
            format: None
        }));
        assert!(
            !when(Kind::Structure, "is", &["frame"]).matches(Target {
                program: &p,
                format: None
            }),
            "分からなければ合わない"
        );
        let blank = Target {
            program: &p,
            format: None,
        };
        assert!(
            !when(Kind::Resolution, "le", &["8k"]).matches(blank),
            "画像が無ければ合わない"
        );
    }

    #[test]
    fn a_preset_becomes_one_unit_per_filter_and_the_settings_read_back() {
        let mut f = filter();
        f.presets[0].filters[1].on = false;
        f.shaders.push(Shader {
            file: String::from("a.glsl"),
            name: String::from("拡大（A）"),
            kind: String::from("拡大"),
        });
        let chosen = Chosen {
            preset: Some(String::from("アニメ")),
            ..Chosen::default()
        };
        assert_eq!(f.chain(&chosen, At::Deinterlace), "deinterlace/tgmc.wgsl");
        assert_eq!(
            f.chain(&chosen, At::Picture),
            "a.glsl",
            "切った物は掛けない"
        );
        f.shaders.push(Shader {
            file: String::from("r.glsl"),
            name: String::from("レストア"),
            kind: String::from("レストア"),
        });
        let mut p = preset("p", "", &["r.glsl", "a.glsl"]);
        assert_eq!(f.interpolation_slot(&p), 1);
        p.filters.insert(
            1,
            Item {
                file: String::from("interpolate/motion.wgsl"),
                on: true,
            },
        );
        f.presets.push(p);
        let p = Chosen {
            preset: Some(String::from("p")),
            ..Chosen::default()
        };
        assert_eq!(
            f.chain(&p, At::Picture),
            "r.glsl | interpolate/motion.wgsl | a.glsl"
        );
        assert!(f.interpolates(&p) && !f.interpolates(&chosen));
        assert_eq!(f.retime(&p, 144.0), 144.0, "既定は画面に合わせる");
        f.presets.last_mut().unwrap().target = String::from("60");
        assert_eq!(f.retime(&p, 144.0), 60.0);
        f.presets.pop();
        assert_eq!(
            f.describe("a.glsl"),
            (String::from("拡大（A）"), String::from("拡大"))
        );
        assert_eq!(f.describe("b.hook").1, UNSET);
        assert_eq!(f.uses_of_preset("実写"), 1, "どれにも合わないときも使用中");
        assert_eq!(f.uses_of_shader("deinterlace/tgmc.wgsl"), 1);
        f.manual = Some(Manual {
            preset: Some(String::from("実写")),
            channel: String::from("0004:0065"),
        });
        let mut settings: BTreeMap<String, String> = f
            .values()
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect();
        settings.insert(MANUAL.to_owned(), f.manual_value());
        assert_eq!(Filter::load(&settings), f);
        assert!(settings.values().all(|v| !v.contains('\n')), "値は一行");
    }

    #[test]
    fn conditions_have_a_line_to_show() {
        let names = |key: &str| {
            if key == "7fef:0400" {
                String::from("ゼロウェーブ")
            } else {
                key.to_owned()
            }
        };
        assert_eq!(
            when(Kind::Genre, "any", &["7"]).summary(&names),
            "ジャンル：アニメ／特撮"
        );
        assert_eq!(
            when(Kind::Resolution, "le", &["sd"]).summary(&names),
            "解像度：SD（720×480） 以下"
        );
        assert_eq!(
            when(Kind::Network, "none", &["BS4K", "CS4K"]).summary(&names),
            "ネットワーク：BS4K・CS4K 以外"
        );
        assert_eq!(
            when(Kind::Channel, "any", &["7fef:0400"]).summary(&names),
            "チャンネル：ゼロウェーブ"
        );
        assert_eq!(genre_label("7.0"), "アニメ／特撮 › 国内アニメ");
    }
}
