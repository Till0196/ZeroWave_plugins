// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! CDATA の兄弟ノードのテスト。

use crate::document::layout;

use super::tests::a_browser_with;

#[test]
fn a_cdata_section_is_its_own_node_beside_plain_text() {
    let mut browser = a_browser_with(&[(
        "startup.bml",
        r#"<bml><head><script><![CDATA[ function onInit(){ var p = document.getElementById("p"); p.firstChild.nextSibling.data = "week"; } ]]></script></head><body onload="onInit();"><p id="p" style="left:10px;top:10px;width:200px;height:30px;font-size:16px;">◇<![CDATA[-]]></p></body></bml>"#,
    )]);
    let laid = browser.laid();
    let text: String = laid
        .items
        .iter()
        .filter_map(|item| match item {
            layout::Item::Text(run) => Some(run.glyphs.iter().map(|(_, c)| *c).collect::<String>()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "◇week");
}
