// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 通知の中身を作る（[`zwsdk::ui::Notice`]）。

use zwsdk::ui::{Notice, Tone};
use zwsdk::video::{At, Env, Event};

use crate::presets::{Chosen, Filter, Notify};

const WAITING: &str = "準備ができるまで、標準のデインターレースで表示します。";
const PREPARING: &str = "初回だけ数分かかります。準備ができるまで、補間せずに表示します。";

fn finish(mut notice: Notice, notify: &Notify) -> Notice {
    notice.details.truncate(usize::from(notify.lines));
    if !notify.extend && notice.progress.is_none() && notice.seconds.is_none() {
        notice.seconds = Some(3.5);
    }
    notice
}

pub fn preset(filter: &Filter, chosen: &Chosen, env: &Env) -> Option<Notice> {
    let notify = &filter.notify;
    if !notify.preset || (chosen.manual && !notify.manual) {
        return None;
    }
    let title = match (&chosen.preset, chosen.manual) {
        (Some(preset), true) => format!("映像フィルター：［{preset}］（手動）"),
        (Some(preset), false) => format!("映像フィルター：［{preset}］"),
        (None, _) => String::from("映像フィルター：なし"),
    };
    let mut details = Vec::new();
    if notify.rule {
        if chosen.manual {
            details.push(String::from("チャンネルを変更するまで有効です。"));
        } else if let Some((auto, why)) = &chosen.auto {
            details.push(if why.is_empty() {
                format!("自動選択［{auto}］")
            } else {
                format!("自動選択［{auto}］：{why}")
            });
        }
    }
    if notify.chain && chosen.preset.is_some() {
        let names: Vec<String> = filter
            .files(chosen, At::Deinterlace)
            .iter()
            .chain(&filter.files(chosen, At::Picture))
            .map(|file| filter.describe(file).0)
            .collect();
        if !names.is_empty() {
            details.push(names.join(" → "));
        }
    }
    if notify.size {
        let scan = if env.format.interlaced {
            " インターレース"
        } else {
            ""
        };
        details.push(format!(
            "{}×{}{scan} → {}×{}",
            env.format.width, env.format.height, env.target[0], env.target[1]
        ));
    }
    Some(finish(
        Notice {
            key: String::from("preset"),
            title,
            details,
            ..Notice::default()
        },
        notify,
    ))
}

pub fn names(filter: &Filter, files: &[String]) -> String {
    files
        .iter()
        .map(|file| filter.describe(file).0)
        .collect::<Vec<_>>()
        .join(" → ")
}

/// 出ている準備中の通知を消す（同じ中身を、出している時間 0 で出し直す）。
pub fn withdrawn(mut compiling: Notice) -> Notice {
    compiling.seconds = Some(0.0);
    compiling
}

pub fn event(filter: &Filter, chosen: Option<&Chosen>, event: &Event) -> Option<Notice> {
    let notify = &filter.notify;
    let what = |at: &At| match at {
        At::Deinterlace => "デインターレース",
        At::Picture => "映像フィルター",
    };
    let notice = match event {
        Event::Building { total } if notify.compile => Notice {
            key: String::from("compile"),
            title: format!("映像フィルターを準備しています（0 / {total}）"),
            details: vec![String::from(WAITING)],
            progress: Some(0.0),
            ..Notice::default()
        },
        Event::Progress { done, total } if notify.compile => Notice {
            key: String::from("compile"),
            title: format!("映像フィルターを準備しています（{done} / {total}）"),
            details: vec![String::from(WAITING)],
            progress: Some(*done as f32 / (*total).max(1) as f32),
            ..Notice::default()
        },
        Event::Built { seconds } if notify.built => Notice {
            key: String::from("compile"),
            title: String::from("映像フィルターの準備ができました。"),
            details: vec![format!("（{seconds:.1} 秒）")],
            tone: Tone::Good,
            ..Notice::default()
        },
        Event::Preparing { progress, said } if notify.compile => Notice {
            key: String::from("compile"),
            title: match (progress, said.is_empty()) {
                (Some(p), false) => format!(
                    "映像フィルターを準備しています（{said} {} %）",
                    (p * 100.0).floor()
                ),
                (Some(p), true) => {
                    format!(
                        "映像フィルターを準備しています（{} %）",
                        (p * 100.0).floor()
                    )
                }
                (None, false) => format!("映像フィルターを準備しています（{said}）"),
                (None, true) => String::from("映像フィルターを準備しています"),
            },
            details: vec![String::from(PREPARING)],
            progress: Some(progress.unwrap_or(0.0)),
            ..Notice::default()
        },
        Event::Prepared if notify.built => Notice {
            key: String::from("compile"),
            title: String::from("映像フィルターの準備ができました。"),
            tone: Tone::Good,
            ..Notice::default()
        },
        Event::Paused { at, ms, budget } if notify.paused => Notice {
            key: String::from("paused"),
            title: format!("{}を一時停止しました。", what(at)),
            details: {
                let mut lines = Vec::new();
                if let Some(chosen) = chosen {
                    let names = names(filter, &filter.files(chosen, *at));
                    if !names.is_empty() {
                        lines.push(names);
                    }
                }
                lines.push(format!(
                    "処理が間に合いません。（{ms:.1} ms／上限 {budget:.1} ms）"
                ));
                lines
            },
            tone: Tone::Warn,
            ..Notice::default()
        },
        Event::Failed { reason, .. } if notify.error => {
            // 理由の改行は行の区切り。案内の行が最後に残るよう、理由は行数の上限より一つ少なく。
            let room = usize::from(notify.lines).saturating_sub(1).max(1);
            let mut details: Vec<String> = reason
                .lines()
                .filter(|line| !line.trim().is_empty())
                .take(room)
                .map(str::to_owned)
                .collect();
            details.push(String::from(
                "詳しくは［右クリック］→［映像フィルター］→［状態］で確認できます。",
            ));
            Notice {
                key: String::from("error"),
                title: String::from("映像フィルターを適用できません。"),
                details,
                tone: Tone::Bad,
                ..Notice::default()
            }
        }
        _ => return None,
    };
    Some(finish(notice, notify))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preparing_says_how_far() {
        let mut filter = Filter::default();
        let preparing = Event::Preparing {
            progress: Some(0.423),
            said: String::from("RIFE のエンジン"),
        };
        let notice = event(&filter, None, &preparing).expect("既定で出す");
        assert_eq!(
            notice.title,
            "映像フィルターを準備しています（RIFE のエンジン 42 %）"
        );
        assert_eq!(notice.details, [PREPARING]);
        assert_eq!(notice.progress, Some(0.423));
        let unknown = Event::Preparing {
            progress: None,
            said: String::from("RIFE のエンジン"),
        };
        let notice = event(&filter, None, &unknown).expect("既定で出す");
        assert_eq!(
            notice.title,
            "映像フィルターを準備しています（RIFE のエンジン）"
        );
        assert!(notice.seconds.is_none(), "済むまで残す");
        filter.notify.compile = false;
        assert!(event(&filter, None, &preparing).is_none());
        filter.notify.built = false;
        assert!(event(&filter, None, &Event::Prepared).is_none());
    }

    #[test]
    fn a_pause_names_the_filters() {
        let mut filter = Filter::default();
        filter.presets.push(crate::presets::Preset {
            name: String::from("地デジ"),
            deinterlace: String::from("deinterlace/tgmc.wgsl"),
            filters: vec![
                crate::presets::Item {
                    file: String::from("a.glsl"),
                    on: true,
                },
                crate::presets::Item {
                    file: String::from("b.hook"),
                    on: true,
                },
            ],
            ..crate::presets::Preset::default()
        });
        let chosen = Chosen {
            auto: None,
            manual: false,
            preset: Some(String::from("地デジ")),
        };
        let paused = Event::Paused {
            at: At::Picture,
            ms: 8.6,
            budget: 8.3,
        };
        let notice = event(&filter, Some(&chosen), &paused).expect("既定で出す");
        assert_eq!(notice.details[0], "a.glsl → b.hook");
        let notice = event(&filter, None, &paused).expect("既定で出す");
        assert!(notice.details[0].starts_with("処理が間に合いません"));
    }

    #[test]
    fn the_settings_choose_what_is_said() {
        let mut filter = Filter::default();
        let failed = Event::Failed {
            at: At::Picture,
            reason: String::from("ファイルが見つかりません：a.glsl"),
        };
        let notice = event(&filter, None, &failed).expect("既定で出す");
        assert_eq!((notice.tone, notice.details.len()), (Tone::Bad, 2));
        let two = Event::Failed {
            at: At::Picture,
            reason: String::from("映像フィルターでエラーが発生しました。\nunreachable"),
        };
        let notice = event(&filter, None, &two).unwrap();
        assert_eq!(notice.details.len(), 3, "理由の改行は行の区切り");
        assert!(notice.details[2].starts_with("詳しくは"));
        filter.notify.lines = 1;
        assert_eq!(
            event(&filter, None, &failed).unwrap().details.len(),
            1,
            "行数の上限"
        );
        filter.notify.error = false;
        assert!(event(&filter, None, &failed).is_none());
        assert!(
            event(&filter, None, &Event::Built { seconds: 1.2 }).is_none(),
            "準備中の通知を消すだけ"
        );
        filter.notify.built = true;
        let built = event(&filter, None, &Event::Built { seconds: 1.2 }).unwrap();
        assert_eq!(built.details, ["（1.2 秒）"]);
        filter.notify.built = false;
        filter.notify.compile = false;
        assert!(event(&filter, None, &Event::Built { seconds: 1.2 }).is_none());
        let progress = Filter::default();
        let half = event(&progress, None, &Event::Progress { done: 1, total: 2 }).unwrap();
        assert_eq!(half.progress, Some(0.5));
    }
}
