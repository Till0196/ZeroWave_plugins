// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 受像機のテスト。

use crate::document::layout;
use crate::media::text;
use crate::resource::File;

use super::*;

/// 選局の済んだ受像機。識別子が分かっていないと最初の文書を開かないので、テストはまずそれを渡す。
fn a_tuned_browser() -> Browser {
    let mut browser = Browser::new();
    browser.set_broadcast(Broadcast {
        network: 0x7fe6,
        transport: 0x7fe6,
        service: 0x0400,
        ..Broadcast::default()
    });
    browser
}

pub(super) fn a_browser_with(files: &[(&str, &str)]) -> Browser {
    let mut browser = a_tuned_browser();
    browser.set_wanted(true);
    browser.set_entry(0x40, 0);
    browser.listed(0x40, vec![(0, 1)]);
    let files = files
        .iter()
        .map(|(name, body)| File {
            location: (*name).to_owned(),
            content_type: "text/x-arib-bml".into(),
            body: text::encode(body),
        })
        .collect();
    browser.arrived(0x40, 0, 1, files);
    browser
}

#[test]
fn a_data_button_pressed_before_the_document_arrives_reaches_it() {
    let mut browser = a_tuned_browser();
    browser.set_wanted(true);
    browser.set_entry(0x40, 0);
    browser.listed(0x40, vec![(0, 1)]);
    browser.key(Key::DataButton);
    let source = r#"<bml><head><script><![CDATA[ function shown(){ document.getElementById("p").firstChild.data = "shown"; } ]]></script><bevent><beitem id="d" type="DataButtonPressed" onoccur="shown();" subscribe="subscribe"/></bevent></head><body><p id="p" style="left:10px;top:10px;width:200px;height:30px;font-size:16px;">waiting</p></body></bml>"#;
    browser.arrived(
        0x40,
        0,
        1,
        vec![File {
            location: "startup.bml".into(),
            content_type: "text/x-arib-bml".into(),
            body: text::encode(source),
        }],
    );
    browser.tick(100.0);
    let laid = browser.laid();
    let text = laid.items.iter().find_map(|item| match item {
        layout::Item::Text(run) => Some(run.glyphs.iter().map(|(_, c)| *c).collect::<String>()),
        _ => None,
    });
    assert_eq!(text.as_deref(), Some("shown"));
}

#[test]
fn the_entry_waits_for_the_broadcast_identity() {
    let mut browser = Browser::new();
    browser.set_wanted(true);
    browser.set_entry(0x40, 0);
    browser.listed(0x40, vec![(0, 1)]);
    browser.arrived(
        0x40,
        0,
        1,
        vec![File {
            location: "startup.bml".into(),
            content_type: "text/x-arib-bml".into(),
            body: br#"<bml><head></head><body><p>x</p></body></bml>"#.to_vec(),
        }],
    );
    assert!(!browser.is_open(), "識別子が分かる前は開かない");
    browser.set_broadcast(Broadcast {
        network: 0x7fe6,
        transport: 0x7fe6,
        service: 0x0400,
        ..Broadcast::default()
    });
    assert!(browser.is_open(), "識別子が分かったら開く");
}

#[test]
fn a_single_bml_module_is_its_own_entry() {
    let single = |content_type: &str| {
        let mut browser = a_tuned_browser();
        browser.set_wanted(true);
        browser.set_entry(0x40, 0);
        browser.listed(0x40, vec![(0, 1)]);
        browser.arrived(
            0x40,
            0,
            1,
            vec![File {
                location: String::new(),
                content_type: content_type.into(),
                body: br#"<bml><head></head><body><p>x</p></body></bml>"#.to_vec(),
            }],
        );
        browser
    };
    assert!(single("text/X-arib-bml;charset=\"euc-jp\"").is_open());
    assert!(!single("image/X-arib-png").is_open());
}

#[test]
fn a_table_in_a_module_still_on_its_way_reopens_the_document_when_it_comes() {
    let mut browser = a_tuned_browser();
    browser.set_wanted(true);
    browser.set_entry(0x40, 0);
    browser.listed(0x40, vec![(0, 1), (3, 1)]);
    browser.arrived(
        0x40,
        0,
        1,
        vec![File {
            location: "startup.bml".into(),
            content_type: "text/x-arib-bml".into(),
            body: br#"<bml><head><script><![CDATA[var bt=new BinaryTable("/40/0003/t.btb","0,U:1B");browser.Ureg[1]=(bt==null)?"null":String(bt.nrow);]]></script></head><body><p>x</p></body></bml>"#.to_vec(),
        }],
    );
    assert!(browser.is_open());
    let logs: Vec<String> = browser
        .take_requests()
        .into_iter()
        .filter_map(|r| match r {
            Request::Log(text) => Some(text),
            _ => None,
        })
        .collect();
    assert!(
        logs.iter().any(|l| l.contains("待つ /40/0003/t.btb")),
        "{logs:?}"
    );
    browser.arrived(
        0x40,
        3,
        1,
        vec![File {
            location: "t.btb".into(),
            content_type: "application/octet-stream".into(),
            body: vec![7, 8],
        }],
    );
    assert_eq!(browser.host().ureg[1], "2");
}

#[test]
fn the_data_button_opens_the_entry_document_and_never_closes_it() {
    let mut browser = a_tuned_browser();
    browser.set_entry(0x40, 0);
    browser.listed(0x40, vec![(0, 1)]);
    browser.arrived(
        0x40,
        0,
        1,
        vec![File {
            location: "startup.bml".into(),
            content_type: "text/x-arib-bml".into(),
            body: br#"<bml><head></head><body><p id="p" style="left:10px;top:10px;width:200px;height:30px;font-size:16px;">x</p></body></bml>"#.to_vec(),
        }],
    );
    assert!(!browser.is_open(), "d の前は出さない");
    browser.key(Key::DataButton);
    assert!(browser.is_open(), "d で開く");
    browser.key(Key::DataButton);
    assert!(
        browser.is_open(),
        "文書が d を取らなくても、受像機は閉じない"
    );
}

#[test]
fn the_entry_document_opens_when_its_module_arrives_and_runs_onload() {
    let mut browser = a_browser_with(&[(
        "startup.bml",
        r#"<bml><head><script><![CDATA[ function onInit(){ document.getElementById("p").firstChild.data = "hello"; document.getElementById("p").style.visibility = "visible"; } ]]></script></head><body onload="onInit();"><p id="p" style="left:10px;top:10px;width:200px;height:30px;font-size:16px;visibility:hidden;">x</p></body></bml>"#,
    )]);
    let laid = browser.laid();
    let text = laid.items.iter().find_map(|item| match item {
        layout::Item::Text(run) => Some(run.glyphs.iter().map(|(_, c)| *c).collect::<String>()),
        _ => None,
    });
    assert_eq!(text.as_deref(), Some("hello"));
}

#[test]
fn a_missing_binary_table_is_null_and_the_script_goes_on() {
    let mut browser = a_browser_with(&[(
        "startup.bml",
        r#"<bml><head><script><![CDATA[ function onInit(){ var bt = new BinaryTable("/50/abcd/none.btb", "1,S:1V"); document.getElementById("p").firstChild.data = (bt === null) ? "null" : "other"; } ]]></script></head><body onload="onInit();"><p id="p" style="left:10px;top:10px;width:200px;height:30px;font-size:16px;">x</p></body></bml>"#,
    )]);
    let laid = browser.laid();
    let text = laid.items.iter().find_map(|item| match item {
        layout::Item::Text(run) => Some(run.glyphs.iter().map(|(_, c)| *c).collect::<String>()),
        _ => None,
    });
    assert_eq!(text.as_deref(), Some("null"));
}

#[test]
fn character_codes_are_euc_jp_in_scripts() {
    let mut browser = a_tuned_browser();
    browser.set_wanted(true);
    browser.set_entry(0x40, 0);
    browser.listed(0x40, vec![(0, 1)]);
    let source = r#"<bml><head><script><![CDATA[ function onInit(){ var s = "あ"; document.getElementById("p").firstChild.data = s.charCodeAt(0).toString(16) + String.fromCharCode(0xa4a4) + "A".charCodeAt(0); } ]]></script></head><body onload="onInit();"><p id="p" style="left:10px;top:10px;width:300px;height:30px;font-size:16px;">x</p></body></bml>"#;
    browser.arrived(
        0x40,
        0,
        1,
        vec![File {
            location: "startup.bml".into(),
            content_type: "text/x-arib-bml".into(),
            body: text::encode(source),
        }],
    );
    let laid = browser.laid();
    let text = laid.items.iter().find_map(|item| match item {
        layout::Item::Text(run) => Some(run.glyphs.iter().map(|(_, c)| *c).collect::<String>()),
        _ => None,
    });
    assert_eq!(text.as_deref(), Some("a4a2い65"));
}

#[test]
fn dates_are_japan_standard_time_in_the_bml_form() {
    let mut browser = a_browser_with(&[(
        "startup.bml",
        r#"<bml><head><script><![CDATA[ function onInit(){ var d = new Date(0); var l = new Date(2026, 8, 12, 0, 30, 0); document.getElementById("p").firstChild.data = d.getHours() + "|" + d.toString() + "|" + (l.getTime() / 1000) + "|" + l.getTimezoneOffset() + "|" + (d instanceof Date); } ]]></script></head><body onload="onInit();"><p id="p" style="left:10px;top:10px;width:900px;height:30px;font-size:16px;">x</p></body></bml>"#,
    )]);
    let laid = browser.laid();
    let text = laid.items.iter().find_map(|item| match item {
        layout::Item::Text(run) => Some(run.glyphs.iter().map(|(_, c)| *c).collect::<String>()),
        _ => None,
    });
    // 1970-01-01T00:00Z は日本では 9 時。2026-09-12 00:30 JST は 1789140600。
    assert_eq!(
        text.as_deref(),
        Some("9|1970-01-01T09:00:00|1789140600|-540|true")
    );
}

#[test]
fn the_stream_clock_is_seen_through_date_mode_one() {
    let mut browser = a_browser_with(&[(
        "startup.bml",
        r#"<bml><head><script><![CDATA[ function onInit(){ browser.setCurrentDateMode(1); var s = new Date(); browser.setCurrentDateMode(0); var d = browser.subDate(s, new Date(), 0); document.getElementById("p").firstChild.data = (d > 29000 && d < 31000) + "|" + (browser.subDate(s, new Date(), 2) == 0); } ]]></script></head><body onload="onInit();"><p id="p" style="left:10px;top:10px;width:900px;height:30px;font-size:16px;">x</p></body></bml>"#,
    )]);
    let now = crate::runtime::clock::BmlClock::machine_millis() / 1000;
    browser.set_clock(now + 30);
    browser.eval("onInit();");
    let laid = browser.laid();
    let text = laid.items.iter().find_map(|item| match item {
        layout::Item::Text(run) => Some(run.glyphs.iter().map(|(_, c)| *c).collect::<String>()),
        _ => None,
    });
    assert_eq!(text.as_deref(), Some("true|true"));
}

#[test]
fn a_recording_played_back_shows_its_own_time_as_now() {
    let mut browser = a_browser_with(&[(
        "startup.bml",
        r#"<bml><head><script><![CDATA[ function onInit(){ browser.setCurrentDateMode(1); var s = new Date(); browser.setCurrentDateMode(0); var d = browser.subDate(s, new Date(), 0); document.getElementById("p").firstChild.data = String(d > -1000 && d < 1000); } ]]></script></head><body onload="onInit();"><p id="p" style="left:10px;top:10px;width:900px;height:30px;font-size:16px;">x</p></body></bml>"#,
    )]);
    let now = crate::runtime::clock::BmlClock::machine_millis() / 1000;
    browser.set_clock_as_now(true);
    browser.set_clock(now - 3 * 3600);
    browser.eval("onInit();");
    let laid = browser.laid();
    let text = laid.items.iter().find_map(|item| match item {
        layout::Item::Text(run) => Some(run.glyphs.iter().map(|(_, c)| *c).collect::<String>()),
        _ => None,
    });
    assert_eq!(text.as_deref(), Some("true"));
}

#[test]
fn a_marquee_moves_left_half_a_letter_a_tick() {
    let mut browser = a_browser_with(&[(
        "startup.bml",
        r#"<bml><body><p id="m" style="left:0px;top:0px;width:100px;height:24px;font-size:24px;display:-wap-marquee;-wap-marquee-speed:fast;-wap-marquee-loop:infinite;">ab</p></body></bml>"#,
    )]);
    let at = |browser: &mut Browser| {
        let html = browser.html().unwrap().markup;
        let i = html.find("<p style=\"left:").unwrap() + 15;
        html[i..]
            .split("px")
            .next()
            .unwrap()
            .parse::<f32>()
            .unwrap()
    };
    browser.tick(0.0);
    assert_eq!(at(&mut browser), 100.0);
    browser.tick(100.0);
    assert_eq!(at(&mut browser), 88.0);
    browser.tick(200.0);
    assert_eq!(at(&mut browser), 76.0);
    // 幅 100 + 字幅 24 を超えたら（11 ステップ目、132px）右端へ戻る。
    let mut seen = Vec::new();
    for t in 3..12 {
        browser.tick(t as f64 * 100.0);
        seen.push(at(&mut browser));
    }
    assert_eq!(seen.last().copied(), Some(100.0), "{seen:?}");
}

#[test]
fn launch_document_leaves_the_running_script_and_opens_the_next() {
    let mut browser = a_browser_with(&[
        (
            "startup.bml",
            r#"<bml><head><script><![CDATA[ browser.Ureg[1] = "a"; browser.launchDocument("top.bml"); browser.Ureg[1] = "b"; ]]></script></head><body/></bml>"#,
        ),
        (
            "top.bml",
            r#"<bml><body onload="browser.Ureg[2] = browser.Ureg[1];"><p id="p" style="width:100px;height:20px;">top</p></body></bml>"#,
        ),
    ]);
    assert_eq!(browser.host().ureg[1], "a");
    assert_eq!(browser.host().ureg[2], "a");
    assert_eq!(
        browser.host().page.as_ref().unwrap().name,
        "/40/0000/top.bml"
    );
}

#[test]
fn tuning_clears_ureg_but_keeps_greg() {
    let mut browser = a_browser_with(&[(
        "startup.bml",
        r#"<bml><head><script><![CDATA[ browser.Ureg[10] = "dgtop"; browser.Greg[50] = "kept"; ]]></script></head><body/></bml>"#,
    )]);
    assert_eq!(browser.host().ureg[10], "dgtop");
    browser.retune();
    assert_eq!(browser.host().ureg[10], "");
    assert_eq!(browser.host().greg[50], "kept");
}

#[test]
fn tuning_forgets_the_service_until_the_new_one_is_told() {
    let startup = File {
        location: "startup.bml".into(),
        content_type: "text/x-arib-bml".into(),
        body: text::encode(
            r#"<bml><head><script><![CDATA[ browser.Ureg[1] = browser.getProgramID(2); ]]></script></head><body/></bml>"#,
        ),
    };
    let told = |service: u16| Broadcast {
        network: 4,
        transport: 1,
        service,
        ..Broadcast::default()
    };
    let mut browser = Browser::new();
    browser.set_broadcast(told(1));
    browser.retune();
    browser.set_wanted(true);
    browser.set_entry(0x40, 0);
    browser.listed(0x40, vec![(0, 1)]);
    browser.arrived(0x40, 0, 1, vec![startup]);
    assert!(!browser.is_open(), "新しいサービスの識別子が来る前に開いた");
    browser.set_broadcast(told(20));
    assert!(browser.is_open(), "識別子が来たら開く");
    assert_eq!(
        browser.host().ureg[1],
        "0x0014",
        "前のサービスの番号で開いた"
    );
}

#[test]
fn keys_go_to_the_focused_element_and_the_navigation_follows_nav_index() {
    let mut browser = a_browser_with(&[(
        "startup.bml",
        r#"<bml><body><div id="a" style="width:10px;height:10px;nav-index:0;nav-down:1;" onkeydown="browser.Ureg[3] = String(document.currentEvent.keyCode);" /><div id="b" style="width:10px;height:10px;nav-index:1;" onfocus="browser.Ureg[4]='focused';" /></body></bml>"#,
    )]);
    browser.key(Key::Red);
    let logs = browser.take_requests();
    assert_eq!(browser.host().ureg[3], "22", "{logs:?}");
    browser.key(Key::Down);
    assert_eq!(browser.host().ureg[4], "focused");
    let focus = browser.host().page.as_ref().unwrap().focus;
    let b = browser.host().page.as_ref().unwrap().document.by_id("b");
    assert_eq!(focus, b);
}

#[test]
fn timers_fire_when_their_time_comes() {
    let mut browser = a_browser_with(&[(
        "startup.bml",
        r#"<bml><head><script><![CDATA[ var n = 0; function tickIt(){ n++; browser.Ureg[5] = String(n); } browser.setInterval("tickIt();", 100, 2); ]]></script></head><body/></bml>"#,
    )]);
    browser.tick(50.0);
    assert_eq!(browser.host().ureg[5], "");
    browser.tick(150.0);
    assert_eq!(browser.host().ureg[5], "1");
    browser.tick(500.0);
    assert_eq!(browser.host().ureg[5], "2");
}
