// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! `BinaryTable`・`CSVTable` と外字（DRCS）の読み込み。

use super::*;

pub(super) fn cell_value(cell: &table::Cell, _: &mut Context) -> JsValue {
    match cell {
        table::Cell::Boolean(value) => JsValue::from(*value),
        table::Cell::Number(value) => JsValue::from(*value as f64),
        table::Cell::Text(text) => js(text),
        table::Cell::Zip(_) => JsValue::null(),
    }
}

fn table_of(this: &JsValue, context: &mut Context) -> Option<usize> {
    let object = this.as_object()?;
    let index = object.get(js_string!("__t"), context).ok()?;
    index.as_number().map(|n| n as usize)
}

pub(super) fn load_drcs(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let host = host(context);
    let base = host.page.as_ref().map_or(String::new(), |p| p.name.clone());
    let Some(file) = host.resources.get(&base, &name) else {
        if host.wait_for_module(&name) {
            return Err(crate::runtime::leaving());
        }
        host.requests
            .push(Request::Log(format!("loadDRCS: 無い {name}")));
        return nan();
    };
    let glyphs = crate::media::drcs::parse(&file.body);
    if let Some(page) = &mut host.page {
        page.drcs.extend(glyphs);
        page.dirty = true;
    }
    Ok(JsValue::from(1))
}

pub(super) fn unload_drcs(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Some(page) = &mut host(context).page {
        page.drcs.clear();
        page.dirty = true;
    }
    Ok(JsValue::from(1))
}

pub(super) fn binary_table_new(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let structure = string(args.get(1), context);
    let host = host(context);
    let base = host.page.as_ref().map_or(String::new(), |p| p.name.clone());
    // 無ければ例外ではなく null（`if (bt) {…}` と書かれる）。
    let Some(file) = host.resources.get(&base, &name) else {
        // 一覧にあって届いていないだけなら、スクリプトを離れて届いたら文書を開き直す。
        if host.wait_for_module(&name) {
            return Err(crate::runtime::leaving());
        }
        host.requests
            .push(Request::Log(format!("BinaryTable: 無い {name}")));
        return Ok(JsValue::null());
    };
    let Some(parsed) = table::parse(&file.body, &structure) else {
        host.requests.push(Request::Log(format!(
            "BinaryTable: 読めない {name} {structure}"
        )));
        return Ok(JsValue::null());
    };
    host.tables.push(parsed);
    let index = host.tables.len() - 1;
    Ok(table_object(context, index).into())
}

pub(super) fn csv_table_new(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    note(context, format!("CSVTable は未対応: {name}"));
    Err(JsNativeError::error()
        .with_message("CSVTable is not supported")
        .into())
}

fn table_object(context: &mut Context, index: usize) -> JsObject {
    let (rows, columns) = {
        let table = &host(context).tables[index];
        (table.rows.len(), table.fields.len())
    };
    let mut object = ObjectInitializer::new(context);
    object.property(
        js_string!("__t"),
        JsValue::from(index as f64),
        Attribute::all(),
    );
    object.property(
        js_string!("nrow"),
        JsValue::from(rows as f64),
        Attribute::all(),
    );
    object.property(
        js_string!("ncolumn"),
        JsValue::from(columns as f64),
        Attribute::all(),
    );
    for (name, f, length) in [
        (
            "close",
            ok_one as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
            0,
        ),
        ("toNumber", table_number, 2),
        ("toString", table_string, 2),
        ("toArray", table_array, 2),
        ("search", table_search, 7),
    ] {
        object.function(NativeFunction::from_fn_ptr(f), JsString::from(name), length);
    }
    object.build()
}

fn table_number(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(index) = table_of(this, context) else {
        return nan();
    };
    let row = number(args.first(), context) as usize;
    let column = number(args.get(1), context) as usize;
    let value = host(context).tables[index]
        .rows
        .get(row)
        .and_then(|cells| cells.get(column))
        .map_or(f64::NAN, table::Cell::number);
    Ok(JsValue::from(value))
}

fn table_string(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(index) = table_of(this, context) else {
        return Ok(JsValue::null());
    };
    let row = number(args.first(), context) as usize;
    let column = number(args.get(1), context) as usize;
    let text = host(context).tables[index]
        .rows
        .get(row)
        .and_then(|cells| cells.get(column))
        .map(|cell| match cell {
            table::Cell::Boolean(value) => value.to_string(),
            table::Cell::Number(value) => value.to_string(),
            table::Cell::Text(text) => text.clone(),
            table::Cell::Zip(_) => String::new(),
        });
    Ok(text.map_or(JsValue::null(), |t| js(&t)))
}

fn table_array(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(index) = table_of(this, context) else {
        return Ok(JsValue::null());
    };
    let start = number(args.first(), context);
    let count = number(args.get(1), context);
    if !start.is_finite() || start < 0.0 || !count.is_finite() || count < 0.0 {
        return Ok(JsValue::null());
    }
    let rows: Vec<Option<Vec<table::Cell>>> = (0..count as usize)
        .map(|offset| {
            host(context).tables[index]
                .rows
                .get(start as usize + offset)
                .cloned()
        })
        .collect();
    let items: Vec<JsValue> = rows
        .into_iter()
        .map(|row| match row {
            Some(row) => {
                let cells: Vec<JsValue> =
                    row.iter().map(|cell| cell_value(cell, context)).collect();
                JsArray::from_iter(cells, context).into()
            }
            None => JsValue::null(),
        })
        .collect();
    Ok(JsArray::from_iter(items, context).into())
}

fn table_search(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(index) = table_of(this, context) else {
        return nan();
    };
    let start = number(args.first(), context);
    let rest = &args[1.min(args.len())..];
    if !rest.len().is_multiple_of(3) || rest.len() < 6 || !start.is_finite() || start < 0.0 {
        return nan();
    }
    let conditions = rest.len() - 3;
    let any = rest[conditions].to_boolean();
    let limit = number(rest.get(conditions + 1), context);
    let Some(out) = rest.get(conditions + 2).and_then(JsValue::as_object) else {
        return nan();
    };
    if !limit.is_finite() || limit < 1.0 {
        return nan();
    }
    struct Condition {
        column: usize,
        compared_number: f64,
        compared_text: String,
        operator: u32,
    }
    let mut wanted = Vec::new();
    for triple in rest[..conditions].chunks(3) {
        let column = number(Some(&triple[0]), context);
        let operator = number(Some(&triple[2]), context);
        if !column.is_finite() || column < 0.0 || !operator.is_finite() {
            return nan();
        }
        wanted.push(Condition {
            column: column as usize,
            compared_number: number(Some(&triple[1]), context),
            compared_text: string(Some(&triple[1]), context),
            operator: operator as u32,
        });
    }
    let table = host(context).tables[index].clone();
    let mut found: Vec<Vec<table::Cell>> = Vec::new();
    let mut last = -1i64;
    for (at, row) in table.rows.iter().enumerate().skip(start as usize) {
        let results: Vec<bool> = wanted
            .iter()
            .map(|condition| {
                let Some(cell) = row.get(condition.column) else {
                    return false;
                };
                let field = table.fields.get(condition.column);
                let width = field.map_or(32, |f| {
                    if f.unit == table::Unit::Byte {
                        f.length * 8
                    } else {
                        f.length
                    }
                });
                let mask: i64 = if width >= 32 {
                    0xffff_ffff
                } else {
                    (1i64 << width) - 1
                };
                let n = cell.number() as i64;
                let c = condition.compared_number as i64;
                match (cell, condition.operator) {
                    (table::Cell::Number(_) | table::Cell::Boolean(_), 0) => {
                        cell.number() == condition.compared_number
                    }
                    (table::Cell::Number(_) | table::Cell::Boolean(_), 1) => {
                        cell.number() != condition.compared_number
                    }
                    (table::Cell::Number(_), 2) => cell.number() < condition.compared_number,
                    (table::Cell::Number(_), 3) => cell.number() <= condition.compared_number,
                    (table::Cell::Number(_), 4) => cell.number() > condition.compared_number,
                    (table::Cell::Number(_), 5) => cell.number() >= condition.compared_number,
                    (table::Cell::Number(_), 6) => (n & c) & mask != 0,
                    (table::Cell::Number(_), 7) => (n | c) & mask != 0,
                    (table::Cell::Number(_), 8) => (n ^ c) & mask != 0,
                    (table::Cell::Number(_), 9) => (!n & c) & mask != 0,
                    (table::Cell::Number(_), 10) => (!n | c) & mask != 0,
                    (table::Cell::Number(_), 11) => (!n ^ c) & mask != 0,
                    (table::Cell::Text(text), 32) => *text == condition.compared_text,
                    (table::Cell::Text(text), 33) => text.contains(&condition.compared_text),
                    (table::Cell::Text(text), 34) => text.starts_with(&condition.compared_text),
                    (table::Cell::Text(text), 35) => text.ends_with(&condition.compared_text),
                    (table::Cell::Text(text), 36) => *text != condition.compared_text,
                    (table::Cell::Text(text), 37) => !text.contains(&condition.compared_text),
                    (table::Cell::Boolean(value), 64) => {
                        *value == (condition.compared_number != 0.0)
                    }
                    (table::Cell::Boolean(value), 65) => {
                        *value != (condition.compared_number != 0.0)
                    }
                    (table::Cell::Zip(ranges), 96) => {
                        table::zip_includes(ranges, condition.compared_number as u32)
                    }
                    (table::Cell::Zip(ranges), 97) => {
                        !table::zip_includes(ranges, condition.compared_number as u32)
                    }
                    _ => false,
                }
            })
            .collect();
        let hit = if any {
            results.iter().any(|r| *r)
        } else {
            results.iter().all(|r| *r)
        };
        if hit {
            // 郵便番号の欄は、その欄への条件の結果（真偽）に置き換わる。
            let mut kept = row.clone();
            for (column, cell) in kept.iter_mut().enumerate() {
                if matches!(cell, table::Cell::Zip(_)) {
                    let on_this: Vec<bool> = wanted
                        .iter()
                        .zip(&results)
                        .filter(|(c, _)| c.column == column)
                        .map(|(_, r)| *r)
                        .collect();
                    if !on_this.is_empty() {
                        let value = if any {
                            on_this.iter().any(|r| *r)
                        } else {
                            on_this.iter().all(|r| *r)
                        };
                        *cell = table::Cell::Boolean(value);
                    }
                }
            }
            found.push(kept);
            if found.len() >= limit as usize {
                last = at as i64;
                break;
            }
        }
    }
    let rows: Vec<JsValue> = found
        .iter()
        .map(|row| {
            let cells: Vec<JsValue> = row.iter().map(|cell| cell_value(cell, context)).collect();
            JsArray::from_iter(cells, context).into()
        })
        .collect();
    out.set(js_string!("length"), JsValue::from(0), false, context)?;
    for (at, row) in rows.into_iter().enumerate() {
        out.set(at as u32, row, false, context)?;
    }
    Ok(JsValue::from(last as f64))
}
