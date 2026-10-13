// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! BML の ECMAScript を実行する前の書き換え。構文木で見つけて元の文に手を入れる
//! （boa の出力は文字列をエスケープしない）。
//! 割り算・`BinaryTable`・`a.length = n`・宣言し忘れた名前を、受信機と同じに動くよう書き換える。

use std::ops::ControlFlow;

use boa_ast::expression::Expression;
use boa_ast::expression::access::{PropertyAccess, PropertyAccessField};
use boa_ast::expression::operator::assign::{AssignOp, AssignTarget};
use boa_ast::expression::operator::binary::{ArithmeticOp, BinaryOp};
use boa_ast::expression::operator::update::UpdateTarget;
use boa_ast::scope::Scope;
use boa_ast::visitor::{VisitWith, Visitor};
use boa_ast::{Position, Spanned};
use boa_interner::{Interner, Sym};
use boa_parser::{Parser, Source};

pub const DIV: &str = "__bml_div";
pub const SETLEN: &str = "__bml_setlen";

/// 書き換えたスクリプト。読めなければ元のまま（実行してその場でエラーにする）。
pub fn rewrite(source: &str) -> String {
    rewrite_with(source, true, false)
}

pub fn rewrite_declaring(source: &str) -> String {
    rewrite_with(source, true, true)
}

pub fn rewrite_length(source: &str) -> String {
    rewrite_with(source, false, false)
}

fn rewrite_with(source: &str, bml: bool, declare: bool) -> String {
    let mut interner = Interner::default();
    let mut parser = Parser::new(Source::from_bytes(source));
    let Ok(script) = parser.parse_script(&Scope::new_global(), &mut interner) else {
        return source.to_owned();
    };
    let mut finder = Finder {
        tables: [
            interner.get_or_intern("BinaryTable"),
            interner.get_or_intern("CSVTable"),
        ],
        length: interner.get_or_intern("length"),
        bml,
        offsets: Offsets::new(source),
        edits: Vec::new(),
        read: std::collections::BTreeSet::new(),
        declare,
    };
    let _ = script.visit_with(&mut finder);
    let mut out = if finder.edits.is_empty() {
        source.to_owned()
    } else {
        apply(source, finder.edits)
    };
    if declare && !finder.read.is_empty() {
        let mut names: Vec<String> = finder
            .read
            .iter()
            .filter_map(|sym| interner.resolve(*sym).map(|name| name.to_string()))
            .collect();
        names.sort();
        out.push_str("\n;var ");
        out.push_str(&names.join(","));
        out.push(';');
    }
    out
}

/// boa の行と列（列は文字ごと、改行は `\n`・`\r`・`\r\n`・U+2028・U+2029）からバイトの位置へ。
struct Offsets {
    lines: Vec<usize>,
    source: Vec<char>,
    bytes: Vec<usize>,
    line_chars: Vec<usize>,
}

impl Offsets {
    fn new(source: &str) -> Self {
        let mut bytes = Vec::with_capacity(source.len() + 1);
        let mut chars = Vec::with_capacity(source.len());
        for (at, c) in source.char_indices() {
            bytes.push(at);
            chars.push(c);
        }
        bytes.push(source.len());
        let mut line_chars = vec![0];
        let mut lines = vec![0];
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '\r' => {
                    if chars.get(i + 1) == Some(&'\n') {
                        i += 1;
                    }
                    line_chars.push(i + 1);
                    lines.push(bytes[i + 1]);
                }
                '\n' | '\u{2028}' | '\u{2029}' => {
                    line_chars.push(i + 1);
                    lines.push(bytes[i + 1]);
                }
                _ => {}
            }
            i += 1;
        }
        Self {
            lines,
            source: chars,
            bytes,
            line_chars,
        }
    }

    fn byte(&self, at: Position) -> Option<usize> {
        let line = (at.line_number() as usize).checked_sub(1)?;
        let column = (at.column_number() as usize).checked_sub(1)?;
        let first = *self.line_chars.get(line)?;
        let _ = (&self.lines, &self.source);
        self.bytes.get(first + column).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Replace,
    Close,
    Open,
}

#[derive(Debug, Clone)]
struct Edit {
    start: usize,
    end: usize,
    text: String,
    kind: Kind,
    order: usize,
}

struct Finder {
    tables: [Sym; 2],
    length: Sym,
    bml: bool,
    offsets: Offsets,
    edits: Vec<Edit>,
    read: std::collections::BTreeSet<Sym>,
    declare: bool,
}

impl Finder {
    fn push(&mut self, start: usize, end: usize, text: &str, kind: Kind) {
        let order = self.edits.len();
        self.edits.push(Edit {
            start,
            end,
            text: text.to_owned(),
            kind,
            order,
        });
    }
}

impl<'ast> Visitor<'ast> for Finder {
    type BreakTy = ();

    fn visit_expression(&mut self, node: &'ast Expression) -> ControlFlow<()> {
        match node {
            Expression::Binary(binary)
                if self.bml && binary.op() == BinaryOp::Arithmetic(ArithmeticOp::Div) =>
            {
                if let (Some(ls), Some(le), Some(rs), Some(re)) = (
                    self.start(binary.lhs()),
                    self.end(binary.lhs()),
                    self.start(binary.rhs()),
                    self.end(binary.rhs()),
                ) && ls <= le
                    && le <= rs
                    && rs <= re
                {
                    // `return a/b` のように、直前が識別子の文字なら空白を挟む。
                    let glued = self
                        .offsets
                        .source
                        .get(self.char_index(ls).wrapping_sub(1))
                        .is_some_and(|c| c.is_alphanumeric() || *c == '_' || *c == '$');
                    let open = if glued {
                        format!(" {DIV}(")
                    } else {
                        format!("{DIV}(")
                    };
                    self.push(ls, ls, &open, Kind::Open);
                    self.push(le, rs, ", ", Kind::Replace);
                    self.push(re, re, ")", Kind::Close);
                }
            }
            Expression::Assign(assign) if self.bml && assign.op() == AssignOp::Div => {
                let lhs = match assign.lhs() {
                    AssignTarget::Identifier(id) => Some(Expression::Identifier(*id)),
                    AssignTarget::Access(access) => {
                        Some(Expression::PropertyAccess(access.clone()))
                    }
                    AssignTarget::Pattern(_) => None,
                };
                if let Some(lhs) = lhs
                    && let (Some(ls), Some(le), Some(rs), Some(re)) = (
                        self.start(&lhs),
                        self.end(&lhs),
                        self.start(assign.rhs()),
                        self.end(assign.rhs()),
                    )
                    && ls <= le
                    && le <= rs
                    && rs <= re
                {
                    let text: String = self.offsets.source
                        [self.char_index(ls)..self.char_index(le)]
                        .iter()
                        .collect();
                    self.push(le, rs, &format!(" = {DIV}({text}, "), Kind::Replace);
                    self.push(re, re, ")", Kind::Close);
                }
            }
            Expression::Assign(assign) if assign.op() == AssignOp::Assign => {
                if let AssignTarget::Access(PropertyAccess::Simple(access)) = assign.lhs()
                    && let PropertyAccessField::Const(field) = access.field()
                    && field.sym() == self.length
                    && let (Some(os), Some(oe), Some(rs), Some(re)) = (
                        self.start(access.target()),
                        self.end(access.target()),
                        self.start(assign.rhs()),
                        self.end(assign.rhs()),
                    )
                    && os <= oe
                    && oe <= rs
                    && rs <= re
                {
                    let glued = self
                        .offsets
                        .source
                        .get(self.char_index(os).wrapping_sub(1))
                        .is_some_and(|c| c.is_alphanumeric() || *c == '_' || *c == '$');
                    let open = if glued {
                        format!(" {SETLEN}(")
                    } else {
                        format!("{SETLEN}(")
                    };
                    self.push(os, os, &open, Kind::Open);
                    self.push(oe, rs, ", ", Kind::Replace);
                    self.push(re, re, ")", Kind::Close);
                }
            }
            Expression::New(new) if self.bml => {
                if let Expression::Identifier(id) = new.constructor()
                    && self.tables.contains(&id.sym())
                    && let (Some(ns), Some(cs)) = (
                        self.offsets.byte(new.span().start()),
                        self.offsets.byte(id.span().start()),
                    )
                    && ns < cs
                {
                    self.push(ns, cs, "", Kind::Replace);
                }
            }
            Expression::Identifier(id) if self.declare => {
                self.read.insert(id.sym());
            }
            _ => {}
        }
        node.visit_with(self)
    }
}

impl Finder {
    fn char_index(&self, byte: usize) -> usize {
        self.offsets
            .bytes
            .partition_point(|b| *b < byte)
            .min(self.offsets.source.len())
    }

    /// ノードの始まりのバイト位置。boa の span は呼び出しや添字でずれるので、子を辿って求める。
    fn start(&self, e: &Expression) -> Option<usize> {
        match e {
            Expression::Call(call) => self.start(call.function()),
            Expression::PropertyAccess(PropertyAccess::Simple(access)) => {
                self.start(access.target())
            }
            Expression::Binary(binary) => self.start(binary.lhs()),
            Expression::Assign(assign) => match assign.lhs() {
                AssignTarget::Identifier(id) => self.offsets.byte(id.span().start()),
                AssignTarget::Access(access) => {
                    self.start(&Expression::PropertyAccess(access.clone()))
                }
                AssignTarget::Pattern(_) => self.offsets.byte(e.span().start()),
            },
            Expression::Conditional(c) => self.start(c.condition()),
            _ => self.offsets.byte(e.span().start()),
        }
    }

    fn end(&self, e: &Expression) -> Option<usize> {
        match e {
            Expression::PropertyAccess(PropertyAccess::Simple(access)) => match access.field() {
                PropertyAccessField::Expr(index) => {
                    let after = self.end(index)?;
                    let mut i = self.char_index(after);
                    while self
                        .offsets
                        .source
                        .get(i)
                        .is_some_and(|c| c.is_whitespace())
                    {
                        i += 1;
                    }
                    if self.offsets.source.get(i) == Some(&']') {
                        self.offsets.bytes.get(i + 1).copied()
                    } else {
                        Some(after)
                    }
                }
                PropertyAccessField::Const(_) => self.offsets.byte(e.span().end()),
            },
            Expression::Binary(binary) => self.end(binary.rhs()),
            Expression::Assign(assign) => self.end(assign.rhs()),
            Expression::Unary(unary) => self.end(unary.target()),
            Expression::Conditional(c) => self.end(c.if_false()),
            Expression::Update(update) => {
                let own = self.offsets.byte(e.span().end());
                let target = match update.target() {
                    UpdateTarget::Identifier(id) => self.offsets.byte(id.span().end()),
                    UpdateTarget::PropertyAccess(access) => {
                        self.end(&Expression::PropertyAccess(access.clone()))
                    }
                };
                match (own, target) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, b) => a.or(b),
                }
            }
            _ => self.offsets.byte(e.span().end()),
        }
    }
}

/// 後ろから当てる。同じ位置では置き換え → 閉じ（外側が先）→ 開き（内側が先）の順。
fn apply(source: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by(|a, b| {
        b.start.cmp(&a.start).then_with(|| {
            let rank = |e: &Edit| match e.kind {
                Kind::Replace => 0,
                Kind::Close => 1,
                Kind::Open => 2,
            };
            rank(a).cmp(&rank(b)).then_with(|| match a.kind {
                Kind::Close => a.order.cmp(&b.order),
                _ => b.order.cmp(&a.order),
            })
        })
    });
    let mut out = source.to_owned();
    for edit in edits {
        if edit.start > out.len() || edit.end > out.len() || edit.start > edit.end {
            continue;
        }
        out.replace_range(edit.start..edit.end, &edit.text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_assignments_become_calls_even_when_chained() {
        let out = rewrite(
            "wBa.length=wBm.length=wBs.length=0; x[i].length = n + 1; if (a.length == 0) b = a.length;",
        );
        assert_eq!(
            out,
            "__bml_setlen(wBa, __bml_setlen(wBm, __bml_setlen(wBs, 0))); __bml_setlen(x[i], n + 1); if (a.length == 0) b = a.length;"
        );
    }

    #[test]
    fn the_length_only_rewrite_leaves_divisions_alone() {
        let out = rewrite_length("a.length = 0; var x = i / 8; var t = new Foo();");
        assert_eq!(out, "__bml_setlen(a, 0); var x = i / 8; var t = new Foo();");
    }

    #[test]
    fn division_becomes_integer_division_and_new_tables_become_calls() {
        let out = rewrite(
            "var a = aTmp[i / 8]; x /= 2; var bt = new BinaryTable(\"/40/0000/a.btb\", \"1,S:1V\"); var t = new Date(); var s = \"a/b\\n\";",
        );
        assert_eq!(
            out,
            "var a = aTmp[__bml_div(i, 8)]; x = __bml_div(x, 2); var bt = BinaryTable(\"/40/0000/a.btb\", \"1,S:1V\"); var t = new Date(); var s = \"a/b\\n\";"
        );
    }

    #[test]
    fn nested_divisions_are_wrapped_from_the_outside() {
        assert_eq!(
            rewrite("y = a / b / c;"),
            "y = __bml_div(__bml_div(a, b), c);"
        );
        assert_eq!(
            rewrite("function f(){return a/2;}"),
            "function f(){return __bml_div(a, 2);}"
        );
        assert_eq!(
            rewrite("y = (a + b) / (c / d);"),
            "y = __bml_div((a + b), (__bml_div(c, d)));"
        );
    }

    #[test]
    fn lines_and_wide_characters_are_counted_the_way_the_lexer_counts() {
        let out = rewrite("var s = \"あいう\";\r\nvar n = 総 / 2;\n");
        assert_eq!(out, "var s = \"あいう\";\r\nvar n = __bml_div(総, 2);\n");
    }

    #[test]
    fn names_that_are_read_are_declared_at_the_end() {
        let out = rewrite_declaring("if (gRankNv2 != null) { f(gRankNv2.id); }");
        assert_eq!(
            out,
            "if (gRankNv2 != null) { f(gRankNv2.id); }\n;var f,gRankNv2;"
        );
        let mut context = boa_engine::Context::default();
        context
            .eval(boa_engine::Source::from_bytes(&rewrite_declaring(
                "var seen = (gNever != null);",
            )))
            .expect("an undeclared name reads as undefined");
    }

    #[test]
    fn a_script_that_does_not_parse_is_left_alone() {
        assert_eq!(rewrite("var = ;"), "var = ;");
    }
}
