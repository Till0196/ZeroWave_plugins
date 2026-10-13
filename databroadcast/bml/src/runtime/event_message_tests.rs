// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 汎用イベントメッセージのテスト。

use super::tests::a_browser_with;
use super::*;

fn a_listener(beitem: &str) -> Browser {
    let body = format!(
        r#"<bml><head><script><![CDATA[function got(){{var e=document.currentEvent;browser.Ureg[1]+=e.messageId+":"+e.messageVersion+":"+e.messageGroupId+":"+e.esRef+":"+e.privateData+";";}}]]></script><bevent>{beitem}</bevent></head><body><p>x</p></body></bml>"#
    );
    a_browser_with(&[("startup.bml", &body)])
}

fn ureg1(browser: &mut Browser) -> String {
    browser.registers().0[1].clone()
}

#[test]
fn the_message_id_is_the_high_byte_and_the_version_the_low_byte() {
    let mut browser = a_listener(
        r#"<beitem id="em" type="EventMessageFired" message_id="176" es_ref="/50" onoccur="got();" subscribe="subscribe"/>"#,
    );
    browser.event_message(0x50, 0, 0, 0xB007, None, b"ten_blk_open_2");
    assert_eq!(ureg1(&mut browser), "176:7:0:/50:ten_blk_open_2;");
}

#[test]
fn the_same_version_fires_once_until_it_changes() {
    let mut browser = a_listener(
        r#"<beitem id="em" type="EventMessageFired" message_id="176" es_ref="/50" onoccur="got();" subscribe="subscribe"/>"#,
    );
    browser.event_message(0x50, 0, 0, 0xB007, None, b"a");
    browser.event_message(0x50, 0, 0, 0xB007, None, b"a");
    browser.event_message(0x50, 0, 0, 0xB008, None, b"b");
    browser.event_message(0x50, 0, 0, 0xB008, None, b"b");
    assert_eq!(ureg1(&mut browser), "176:7:0:/50:a;176:8:0:/50:b;");
}

#[test]
fn only_the_component_the_beitem_names_reaches_it() {
    let mut browser = a_listener(
        r#"<beitem id="a" type="EventMessageFired" message_id="146" es_ref="/60" onoccur="got();" subscribe="subscribe"/><beitem id="b" type="EventMessageFired" message_id="146" onoccur="got();" subscribe="subscribe"/>"#,
    );
    // /50 のものはどちらにも来ない（a は /60、b は文書の /40）。
    browser.event_message(0x50, 0, 0, 0x9201, None, b"x");
    assert_eq!(ureg1(&mut browser), "");
    browser.event_message(0x60, 0, 0, 0x9201, None, b"y");
    assert_eq!(ureg1(&mut browser), "146:1:0:/60:y;");
    browser.event_message(0x40, 0, 0, 0x9201, None, b"z");
    assert_eq!(ureg1(&mut browser), "146:1:0:/60:y;146:1:0:/40:z;");
}

#[test]
fn the_group_defaults_to_zero() {
    let mut browser = a_listener(
        r#"<beitem id="a" type="EventMessageFired" message_id="146" onoccur="got();" subscribe="subscribe"/><beitem id="b" type="EventMessageFired" message_group_id="1" message_id="146" onoccur="got();" subscribe="subscribe"/>"#,
    );
    browser.event_message(0x40, 0, 1, 0x9201, None, b"g1");
    assert_eq!(ureg1(&mut browser), "146:1:1:/40:g1;");
    browser.event_message(0x40, 0, 0, 0x9202, None, b"g0");
    assert_eq!(ureg1(&mut browser), "146:1:1:/40:g1;146:2:0:/40:g0;");
}

#[test]
fn group_zero_is_dropped_when_its_data_event_is_not_the_current_one() {
    let mut browser = a_listener(
        r#"<beitem id="a" type="EventMessageFired" message_id="176" onoccur="got();" subscribe="subscribe"/><beitem id="b" type="EventMessageFired" message_group_id="1" message_id="176" onoccur="got();" subscribe="subscribe"/>"#,
    );
    browser.data_event(0x40, 3);
    browser.event_message(0x40, 14, 0, 0xB001, None, b"old");
    assert_eq!(ureg1(&mut browser), "");
    browser.event_message(0x40, 14, 1, 0xB001, None, b"g1");
    assert_eq!(ureg1(&mut browser), "176:1:1:/40:g1;");
    browser.event_message(0x40, 3, 0, 0xB001, None, b"now");
    assert_eq!(ureg1(&mut browser), "176:1:1:/40:g1;176:1:0:/40:now;");
}

#[test]
fn two_five_five_and_omission_match_anything() {
    let mut browser = a_listener(
        r#"<beitem id="a" type="EventMessageFired" onoccur="got();" subscribe="subscribe"/><beitem id="b" type="EventMessageFired" message_id="255" message_version="3" onoccur="got();" subscribe="subscribe"/>"#,
    );
    browser.event_message(0x40, 0, 0, 0x0102, None, b"v2");
    assert_eq!(ureg1(&mut browser), "1:2:0:/40:v2;");
    browser.event_message(0x40, 0, 0, 0x0103, None, b"v3");
    assert_eq!(
        ureg1(&mut browser),
        "1:2:0:/40:v2;1:3:0:/40:v3;1:3:0:/40:v3;"
    );
}

#[test]
fn resubscribing_forgets_the_versions_it_fired() {
    let mut browser = a_listener(
        r#"<beitem id="em" type="EventMessageFired" message_id="176" onoccur="got();" subscribe="subscribe"/>"#,
    );
    browser.event_message(0x40, 0, 0, 0xB007, None, b"a");
    browser.event_message(0x40, 0, 0, 0xB007, None, b"a");
    assert_eq!(ureg1(&mut browser), "176:7:0:/40:a;");
    browser.eval(
        "document.getElementById('em').subscribe=false;document.getElementById('em').subscribe=true;",
    );
    browser.event_message(0x40, 0, 0, 0xB007, None, b"a");
    assert_eq!(ureg1(&mut browser), "176:7:0:/40:a;176:7:0:/40:a;");
}

#[test]
fn an_event_timed_by_npt_waits_for_that_npt() {
    let mut browser = a_listener(
        r#"<beitem id="em" type="EventMessageFired" message_id="1" onoccur="got();" subscribe="subscribe"/>"#,
    );
    browser.tick(0.0);
    browser.event_message(0x40, 0, 0, 0x0101, Some(90_000 * 10), b"at10s");
    browser.event_message(0x40, 0, 0, 0x0101, Some(90_000 * 10), b"at10s");
    assert_eq!(ureg1(&mut browser), "");
    browser.npt_reference(90_000 * 5);
    browser.tick(1000.0);
    assert_eq!(ureg1(&mut browser), "");
    browser.tick(6000.0);
    assert_eq!(ureg1(&mut browser), "1:1:0:/40:at10s;");
}
