// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 文書のツリーと、XML からそれを作る処理。DTD は読まず、実体参照は定義済みの五つと数値だけ。
//! スクリプトが要素を掴んで書き換えるので、ノードは配列の番号（[`NodeId`]）で指す。

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub usize);

#[derive(Debug, Clone)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub kind: Kind,
    /// CDATA 区間として書かれた文字か。CDATA の空白はまとめない（空白の数で欄を揃える文書がある）。
    pub cdata: bool,
}

#[derive(Debug, Clone)]
pub enum Kind {
    Element(Element),
    Text(String),
}

#[derive(Debug, Clone, Default)]
pub struct Element {
    pub name: String,
    /// 書かれた順のまま。同じ名前が二度あれば先のほうが勝つ。
    pub attributes: Vec<(String, String)>,
    pub children: Vec<NodeId>,
}

impl Element {
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn set_attribute(&mut self, name: &str, value: &str) {
        if let Some((_, slot)) = self.attributes.iter_mut().find(|(key, _)| key == name) {
            *slot = value.to_owned();
        } else {
            self.attributes.push((name.to_owned(), value.to_owned()));
        }
    }

    pub fn remove_attribute(&mut self, name: &str) {
        self.attributes.retain(|(key, _)| key != name);
    }
}

#[derive(Debug, Clone, Default)]
pub struct Document {
    pub nodes: Vec<Node>,
    pub root: Option<NodeId>,
    ids: BTreeMap<String, NodeId>,
}

impl Document {
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.0]
    }

    pub fn element(&self, id: NodeId) -> Option<&Element> {
        match &self.node(id).kind {
            Kind::Element(element) => Some(element),
            Kind::Text(_) => None,
        }
    }

    pub fn element_mut(&mut self, id: NodeId) -> Option<&mut Element> {
        match &mut self.node_mut(id).kind {
            Kind::Element(element) => Some(element),
            Kind::Text(_) => None,
        }
    }

    pub fn text(&self, id: NodeId) -> Option<&str> {
        match &self.node(id).kind {
            Kind::Text(text) => Some(text),
            Kind::Element(_) => None,
        }
    }

    pub fn by_id(&self, id: &str) -> Option<NodeId> {
        self.ids.get(id).copied()
    }

    pub fn push(&mut self, parent: Option<NodeId>, kind: Kind) -> NodeId {
        let id = NodeId(self.nodes.len());
        if let Kind::Element(element) = &kind
            && let Some(name) = element.attribute("id")
        {
            self.ids.entry(name.to_owned()).or_insert(id);
        }
        self.nodes.push(Node {
            parent,
            kind,
            cdata: false,
        });
        if let Some(parent) = parent
            && let Some(element) = self.element_mut(parent)
        {
            element.children.push(id);
        }
        id
    }

    pub fn reindex(&mut self) {
        self.ids.clear();
        for (at, node) in self.nodes.iter().enumerate() {
            if let Kind::Element(element) = &node.kind
                && let Some(name) = element.attribute("id")
            {
                self.ids.entry(name.to_owned()).or_insert(NodeId(at));
            }
        }
    }

    pub fn elements(&self) -> impl Iterator<Item = NodeId> + '_ {
        (0..self.nodes.len())
            .map(NodeId)
            .filter(|id| matches!(self.node(*id).kind, Kind::Element(_)))
    }

    pub fn text_of(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.gather_text(id, &mut out);
        out
    }

    fn gather_text(&self, id: NodeId, out: &mut String) {
        match &self.node(id).kind {
            Kind::Text(text) => out.push_str(text),
            Kind::Element(element) => {
                for child in &element.children {
                    self.gather_text(*child, out);
                }
            }
        }
    }

    pub fn set_text_of(&mut self, id: NodeId, text: &str) {
        let Some(element) = self.element_mut(id) else {
            return;
        };
        element.children.clear();
        self.push(Some(id), Kind::Text(text.to_owned()));
    }

    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        let mut at = self.node(id).parent;
        std::iter::from_fn(move || {
            let here = at?;
            at = self.node(here).parent;
            Some(here)
        })
    }

    pub fn first_named(&self, name: &str) -> Option<NodeId> {
        self.elements().find(|id| {
            self.element(*id)
                .is_some_and(|element| element.name == name)
        })
    }
}

/// XML を読んでツリーにする。壊れていても止まらない（閉じ忘れは親を閉じ、読めない物は飛ばす）。
pub fn parse(text: &str) -> Document {
    let mut document = Document::default();
    let mut open: Vec<NodeId> = Vec::new();
    let mut at = 0;
    let bytes = text.as_bytes();
    while at < bytes.len() {
        if bytes[at] != b'<' {
            let end = text[at..].find('<').map_or(bytes.len(), |found| at + found);
            let raw = &text[at..end];
            if let Some(&parent) = open.last()
                && !raw.trim().is_empty()
            {
                document.push(Some(parent), Kind::Text(unescape(raw)));
            }
            at = end;
            continue;
        }
        let rest = &text[at..];
        if let Some(inner) = rest.strip_prefix("<![CDATA[") {
            let end = inner.find("]]>").unwrap_or(inner.len());
            if let Some(&parent) = open.last() {
                let id = document.push(Some(parent), Kind::Text(inner[..end].to_owned()));
                document.node_mut(id).cdata = true;
            }
            at += 9 + end + 3;
            continue;
        }
        if rest.starts_with("<!--") {
            at += rest.find("-->").map_or(rest.len(), |found| found + 3);
            continue;
        }
        if rest.starts_with("<?") {
            at += rest.find("?>").map_or(rest.len(), |found| found + 2);
            continue;
        }
        if rest.starts_with("<!") {
            at += rest.find('>').map_or(rest.len(), |found| found + 1);
            continue;
        }
        let Some(end) = tag_end(rest) else {
            break;
        };
        let tag = &rest[1..end - 1];
        at += end;
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim();
            if let Some(found) = open.iter().rposition(|id| {
                document
                    .element(*id)
                    .is_some_and(|element| element.name == name)
            }) {
                open.truncate(found);
            }
            continue;
        }
        let (tag, closes) = match tag.strip_suffix('/') {
            Some(inner) => (inner, true),
            None => (tag, false),
        };
        let element = element(tag);
        let parent = open.last().copied();
        let id = document.push(parent, Kind::Element(element));
        if document.root.is_none() {
            document.root = Some(id);
        }
        if !closes {
            open.push(id);
        }
    }
    // 空の `p`・`span`・`a` にもテキストノードを置く（`firstChild.data = …` で入れ替える文書がある）。
    let empty: Vec<NodeId> = document
        .elements()
        .filter(|id| {
            document.element(*id).is_some_and(|e| {
                matches!(e.name.as_str(), "p" | "span" | "a" | "pre") && e.children.is_empty()
            })
        })
        .collect();
    for id in empty {
        document.push(Some(id), Kind::Text(String::new()));
    }
    document
}

fn tag_end(rest: &str) -> Option<usize> {
    let mut quote = None;
    for (at, byte) in rest.bytes().enumerate() {
        match (quote, byte) {
            (None, b'"' | b'\'') => quote = Some(byte),
            (Some(open), _) if open == byte => quote = None,
            (None, b'>') => return Some(at + 1),
            _ => {}
        }
    }
    None
}

fn element(tag: &str) -> Element {
    let tag = tag.trim();
    let name_end = tag
        .find(|letter: char| letter.is_whitespace())
        .unwrap_or(tag.len());
    // `bml:bevent` の接頭辞は名前に含めない（受像機も `bevent` と同じに扱う）。
    let name = tag[..name_end]
        .strip_prefix("bml:")
        .unwrap_or(&tag[..name_end]);
    let mut element = Element {
        name: name.to_owned(),
        ..Default::default()
    };
    let mut rest = tag[name_end..].trim_start();
    while !rest.is_empty() {
        let Some(equals) = rest.find('=') else {
            break;
        };
        let name = rest[..equals].trim();
        let after = rest[equals + 1..].trim_start();
        let Some(quote) = after.chars().next() else {
            break;
        };
        if quote != '"' && quote != '\'' {
            break;
        }
        let Some(close) = after[1..].find(quote) else {
            break;
        };
        element
            .attributes
            .push((name.to_owned(), unescape(&after[1..1 + close])));
        rest = after[1 + close + 1..].trim_start();
    }
    element
}

pub fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let Some(semi) = after.find(';') else {
            out.push('&');
            rest = after;
            continue;
        };
        let name = &after[..semi];
        let letter = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => name.strip_prefix('#').and_then(|number| {
                let value = match number.strip_prefix('x') {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => number.parse().ok(),
                }?;
                char::from_u32(value)
            }),
        };
        match letter {
            Some(letter) => {
                out.push(letter);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="EUC-JP" ?>
<!DOCTYPE bml PUBLIC "-//ARIB//DTD BML 1.0//JA" "http://www.arib.or.jp/B24/DTD/bml_1_0.dtd">
<?bml bml-version="3.0" ?>
<bml><head><title>t</title><script><![CDATA[ if (a < b) {} ]]></script></head>
<body id="body" style="clut:url(common.clt);" invisible="invisible">
<div id="d" style="left:0px;top:0px;width:960px;height:540px;">
<object id="Vstream" type="video/X-arib-mpeg2" data="/-1" remain="remain"/>
</div>
<p id="p">a &amp; b<br/>&#x3042;</p>
</body></bml>"#;

    #[test]
    fn a_document_is_a_tree_with_its_ids_indexed() {
        let document = parse(SAMPLE);
        let root = document.root.expect("a root");
        assert_eq!(document.element(root).unwrap().name, "bml");
        let body = document.by_id("body").expect("body");
        assert_eq!(
            document.element(body).unwrap().attribute("invisible"),
            Some("invisible")
        );
        let video = document.by_id("Vstream").expect("the video");
        assert_eq!(document.node(video).parent, document.by_id("d"));
        let paragraph = document.by_id("p").expect("p");
        assert_eq!(document.text_of(paragraph), "a & bあ");
        let children = &document.element(paragraph).unwrap().children;
        assert_eq!(children.len(), 3);
        assert_eq!(document.element(children[1]).unwrap().name, "br");
    }

    #[test]
    fn a_script_keeps_its_cdata_as_written() {
        let document = parse(SAMPLE);
        let script = document.first_named("script").expect("script");
        assert_eq!(document.text_of(script), " if (a < b) {} ");
    }

    #[test]
    fn an_unclosed_element_is_closed_by_its_parent() {
        let document = parse("<bml><body><div><p>x</body></bml>");
        let body = document.first_named("body").unwrap();
        assert_eq!(document.node(body).parent, document.root);
        let p = document.first_named("p").unwrap();
        assert_eq!(document.text_of(p), "x");
    }

    #[test]
    fn attribute_values_may_hold_a_closing_bracket() {
        let document = parse(r#"<bml><p onclick="a > b" id="x">t</p></bml>"#);
        let p = document.by_id("x").unwrap();
        assert_eq!(
            document.element(p).unwrap().attribute("onclick"),
            Some("a > b")
        );
    }
}

#[cfg(test)]
mod cdata_tests {
    use super::*;

    #[test]
    fn plain_text_and_a_cdata_section_are_two_nodes() {
        let document = parse(r#"<bml><body><p id="p">◇<![CDATA[-]]></p></body></bml>"#);
        let p = document.first_named("p").unwrap();
        let children = &document.element(p).unwrap().children;
        assert_eq!(children.len(), 2, "{children:?}");
    }
}
