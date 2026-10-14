// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! スタイルシートの字句解析と構文。BML の CSS は CSS 2 の部分集合に放送の特性を足したもの
//! （STD-B24 第二分冊 (2/2) 付属 2）。どの名前がどう効くかは [`crate::document::style`]。

use cssparser::{Delimiter, Parser, Token};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Px(f32),
    Number(f32),
    Keyword(String),
    Text(String),
    Url(String),
    Size(u32, u32),
    Aspect(u32, u32),
    /// 空白で分けた値の並び。`grayscale-color-index: 15 17`。
    List(Vec<Value>),
}

impl Value {
    pub fn px(&self) -> Option<f32> {
        match self {
            Self::Px(value) | Self::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub fn number(&self) -> Option<f32> {
        match self {
            Self::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub fn keyword(&self) -> Option<&str> {
        match self {
            Self::Keyword(word) => Some(word),
            _ => None,
        }
    }

    pub fn items(&self) -> &[Value] {
        match self {
            Self::List(items) => items,
            _ => std::slice::from_ref(self),
        }
    }

    /// 書かれた形に戻す（スクリプトが `style.left` を読むときの文字列）。
    pub fn to_css(&self) -> String {
        match self {
            Self::Px(value) => format!("{}px", trim(*value)),
            Self::Number(value) => trim(*value),
            Self::Keyword(word) => word.clone(),
            Self::Text(text) => format!("\"{text}\""),
            Self::Url(url) => format!("url({url})"),
            Self::Size(w, h) => format!("{w}x{h}"),
            Self::Aspect(w, h) => format!("{w}v{h}"),
            Self::List(items) => items
                .iter()
                .map(Value::to_css)
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

fn trim(value: f32) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub name: String,
    pub value: Value,
    pub important: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Simple {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub pseudo: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selector {
    pub chain: Vec<Simple>,
}

impl Selector {
    pub fn specificity(&self) -> (u32, u32, u32) {
        self.chain.iter().fold((0, 0, 0), |(a, b, c), simple| {
            (
                a + u32::from(simple.id.is_some()),
                b + simple.classes.len() as u32,
                c + u32::from(simple.tag.is_some()),
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
}

pub fn parse_stylesheet(text: &str) -> Vec<Rule> {
    let mut parser = Parser::new(text);
    let mut rules = Vec::new();
    rules_into(&mut parser, &mut rules);
    rules
}

/// `@media` の中は同じ規則として読み（TR-B14 の C プロファイル）、ほかの `@` は飛ばす。
fn rules_into(parser: &mut Parser<'_>, rules: &mut Vec<Rule>) {
    loop {
        let start = parser.state();
        let Ok(token) = parser.next() else {
            break;
        };
        match token {
            Token::AtKeyword(name) => {
                let media = name.eq_ignore_ascii_case("media");
                // 見出し（`tv` など）は読まない。受像機は `tv` しか無い。
                let _ = parser.parse_until_before(
                    Delimiter::CurlyBracketBlock | Delimiter::Semicolon,
                    |p| {
                        while p.next().is_ok() {}
                        Ok::<(), cssparser::ParseError<()>>(())
                    },
                );
                if let Ok(Token::CurlyBracketBlock) = parser.next() {
                    let _ = parser.parse_nested_block(|p| {
                        if media {
                            rules_into(p, rules);
                        } else {
                            while p.next().is_ok() {}
                        }
                        Ok::<(), cssparser::ParseError<()>>(())
                    });
                }
            }
            _ => {
                parser.reset(&start);
                let selectors = parser
                    .parse_until_before(Delimiter::CurlyBracketBlock, |p| {
                        Ok::<_, cssparser::ParseError<()>>(selectors(p))
                    })
                    .unwrap_or_default();
                match parser.next() {
                    Ok(Token::CurlyBracketBlock) => {
                        let declarations = parser
                            .parse_nested_block(|p| {
                                Ok::<_, cssparser::ParseError<()>>(declarations(p))
                            })
                            .unwrap_or_default();
                        if !selectors.is_empty() {
                            rules.push(Rule {
                                selectors,
                                declarations,
                            });
                        }
                    }
                    _ => break,
                }
            }
        }
    }
}

pub fn parse_declarations(text: &str) -> Vec<Declaration> {
    let mut parser = Parser::new(text);
    declarations(&mut parser)
}

fn selectors(parser: &mut Parser<'_>) -> Vec<Selector> {
    let mut out = Vec::new();
    let mut current = Selector::default();
    let mut simple: Option<Simple> = None;
    let mut after_space = false;
    while let Ok(token) = parser.next_including_whitespace() {
        match token {
            Token::WhiteSpace(_) => {
                if simple.is_some() {
                    after_space = true;
                }
                continue;
            }
            Token::Comma => {
                if let Some(simple) = simple.take() {
                    current.chain.push(simple);
                }
                if !current.chain.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                after_space = false;
                continue;
            }
            _ => {}
        }
        if after_space && let Some(done) = simple.take() {
            current.chain.push(done);
        }
        after_space = false;
        let slot = simple.get_or_insert_with(Simple::default);
        match token {
            Token::Ident(name) => slot.tag = Some(name.to_string()),
            Token::Delim('*') => slot.tag = None,
            Token::IDHash(name) | Token::Hash(name) => slot.id = Some(name.to_string()),
            Token::Delim('.') => {
                if let Ok(Token::Ident(name)) = parser.next_including_whitespace() {
                    slot.classes.push(name.to_string());
                }
            }
            Token::Colon => {
                if let Ok(Token::Ident(name)) = parser.next_including_whitespace()
                    && (name.eq_ignore_ascii_case("focus") || name.eq_ignore_ascii_case("active"))
                {
                    slot.pseudo = Some(name.to_ascii_lowercase());
                } else {
                    simple = None;
                    current = Selector::default();
                    let _ = parser.parse_until_before(Delimiter::Comma, |p| {
                        while p.next().is_ok() {}
                        Ok::<(), cssparser::ParseError<()>>(())
                    });
                    let _ = parser.next();
                }
            }
            // 運用されない形（`:where(...)`、`[attr]` など）は選択子ごと捨てる。
            _ => {
                simple = None;
                current = Selector::default();
                let _ = parser.parse_until_before(Delimiter::Comma, |p| {
                    while p.next().is_ok() {}
                    Ok::<(), cssparser::ParseError<()>>(())
                });
                let _ = parser.next();
            }
        }
    }
    if let Some(simple) = simple.take() {
        current.chain.push(simple);
    }
    if !current.chain.is_empty() {
        out.push(current);
    }
    out
}

fn declarations(parser: &mut Parser<'_>) -> Vec<Declaration> {
    let mut out = Vec::new();
    while let Ok(token) = parser.next() {
        let name = match token {
            Token::Ident(name) => name.to_string().to_ascii_lowercase(),
            Token::Semicolon => continue,
            _ => {
                let _ = parser.parse_until_before(Delimiter::Semicolon, |p| {
                    while p.next().is_ok() {}
                    Ok::<(), cssparser::ParseError<()>>(())
                });
                let _ = parser.next();
                continue;
            }
        };
        if parser.expect_colon().is_err() {
            let _ = parser.parse_until_before(Delimiter::Semicolon, |p| {
                while p.next().is_ok() {}
                Ok::<(), cssparser::ParseError<()>>(())
            });
            let _ = parser.next();
            continue;
        }
        let (values, important) = parser
            .parse_until_before(Delimiter::Semicolon, |p| {
                Ok::<_, cssparser::ParseError<()>>(values(p))
            })
            .unwrap_or_default();
        let _ = parser.next();
        let value = match values.len() {
            0 => continue,
            1 => values.into_iter().next().unwrap(),
            _ => Value::List(values),
        };
        out.push(Declaration {
            name,
            value,
            important,
        });
    }
    out
}

fn values(parser: &mut Parser<'_>) -> (Vec<Value>, bool) {
    let mut out = Vec::new();
    let mut important = false;
    while let Ok(token) = parser.next() {
        let value = match token {
            Token::Dimension { value, unit, .. } => match unit.to_ascii_lowercase().as_str() {
                "px" => Value::Px(*value),
                // `960x540` は 960 に単位 `x540`、`16v9` は 16 に単位 `v9` として来る。
                other => match other.split_once(['x', 'v']) {
                    Some(("", rest)) if !rest.is_empty() => match rest.parse::<u32>() {
                        Ok(second) if other.starts_with('x') => Value::Size(*value as u32, second),
                        Ok(second) => Value::Aspect(*value as u32, second),
                        Err(_) => Value::Keyword(format!("{value}{other}")),
                    },
                    _ => Value::Keyword(format!("{value}{other}")),
                },
            },
            Token::Number { value, .. } => Value::Number(*value),
            Token::Percentage { unit_value, .. } => Value::Number(*unit_value * 100.0),
            Token::Ident(word) => Value::Keyword(word.to_string()),
            Token::Hash(hex) | Token::IDHash(hex) => Value::Keyword(format!("#{hex}")),
            Token::QuotedString(text) => Value::Text(text.to_string()),
            Token::UnquotedUrl(url) => Value::Url(url.to_string()),
            Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                let url = parser
                    .parse_nested_block(|p| {
                        Ok::<_, cssparser::ParseError<()>>(
                            p.expect_string().map(|s| s.to_string()).unwrap_or_default(),
                        )
                    })
                    .unwrap_or_default();
                Value::Url(url)
            }
            Token::Delim('!') => {
                if let Ok(Token::Ident(word)) = parser.next()
                    && word.eq_ignore_ascii_case("important")
                {
                    important = true;
                }
                continue;
            }
            Token::Comma => continue,
            _ => continue,
        };
        out.push(value);
    }
    (out, important)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stylesheet_is_rules_of_selectors_and_declarations() {
        let rules = parse_stylesheet(
            "body{clut:url(top.clt);resolution:960x540;display-aspect-ratio:16v9;used-key-list:basic data-button;}\np{font-size:16px;color-index:138;grayscale-color-index:153 154;font-family:\"丸ゴシック\";}\n#Vstream, div p.x{top:93px;}",
        );
        assert_eq!(rules.len(), 3);
        let body = &rules[0];
        assert_eq!(body.selectors[0].chain[0].tag.as_deref(), Some("body"));
        assert_eq!(body.declarations[0].value, Value::Url("top.clt".into()));
        assert_eq!(body.declarations[1].value, Value::Size(960, 540));
        assert_eq!(body.declarations[2].value, Value::Aspect(16, 9));
        assert_eq!(
            body.declarations[3].value,
            Value::List(vec![
                Value::Keyword("basic".into()),
                Value::Keyword("data-button".into())
            ])
        );
        let p = &rules[1];
        assert_eq!(p.declarations[0].value, Value::Px(16.0));
        assert_eq!(p.declarations[1].value, Value::Number(138.0));
        assert_eq!(p.declarations[3].value, Value::Text("丸ゴシック".into()));
        let two = &rules[2];
        assert_eq!(two.selectors.len(), 2);
        assert_eq!(two.selectors[0].chain[0].id.as_deref(), Some("Vstream"));
        assert_eq!(two.selectors[1].chain.len(), 2);
        assert_eq!(two.selectors[1].chain[1].classes, ["x"]);
        assert_eq!(two.selectors[1].specificity(), (0, 1, 2));
    }

    #[test]
    fn a_style_attribute_is_declarations() {
        let declarations = parse_declarations(
            "left:0px;top:0px; width:960px;height:540px;visibility:hidden !important",
        );
        assert_eq!(declarations.len(), 5);
        assert_eq!(declarations[4].value, Value::Keyword("hidden".into()));
        assert!(declarations[4].important);
        assert_eq!(declarations[0].value.to_css(), "0px");
    }

    #[test]
    fn a_rule_that_cannot_be_read_does_not_take_the_next_with_it() {
        let rules = parse_stylesheet("@import url(x.css); p:hover{color-index:1} div{left:1px}");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selectors[0].chain[0].tag.as_deref(), Some("div"));
    }
}
