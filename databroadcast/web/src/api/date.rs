// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! `Date` の解析の補完。boa は ISO 8601 しか読まないが、通信のアプリは
//! `2026/09/12 05:00:00` などブラウザ流の書き方を使うので、NaN になる文字列だけ自分で解析する。

use std::cell::Cell;

use boa_engine::{Context, JsObject, JsResult, JsValue};

use bml::runtime::clock::days_from_civil;

use super::{get, global, js, set, set_global, string, value};

thread_local! {
    /// 放送の時刻と PC の時計の差（ミリ秒）。録画の再生では `new Date()` は録画の時刻。
    static OFFSET_MS: Cell<i64> = const { Cell::new(0) };
}

pub fn set_clock(unix_seconds: i64) {
    OFFSET_MS.with(|offset| offset.set(unix_seconds * 1000 - machine_millis()));
}

pub fn clear_clock() {
    OFFSET_MS.with(|offset| offset.set(0));
}

fn machine_millis() -> i64 {
    zwsdk::plug::host::unix_ms() as i64
}

pub(crate) fn now_millis() -> f64 {
    (machine_millis() + OFFSET_MS.with(Cell::get)) as f64
}

pub(super) fn install(context: &mut Context) {
    let Some(original) = global(context, "Date") else {
        return;
    };
    set_global(context, "__zw_date", original.clone().into());
    let wrapper = boa_engine::object::FunctionObjectBuilder::new(
        context.realm(),
        boa_engine::NativeFunction::from_fn_ptr(construct),
    )
    .name(boa_engine::JsString::from("Date"))
    .length(7)
    .constructor(true)
    .build();
    let wrapper: JsObject = wrapper.into();
    let proto = get(&original, "prototype", context);
    set(&wrapper, "prototype", proto.clone(), context);
    if let Some(proto) = proto.as_object() {
        set(&proto, "constructor", wrapper.clone().into(), context);
    }
    let utc = get(&original, "UTC", context);
    value(&wrapper, context, "UTC", utc);
    super::method(&wrapper, context, "now", 0, now);
    super::method(&wrapper, context, "parse", 1, parse);
    set_global(context, "Date", wrapper.into());
}

fn original(context: &mut Context) -> Option<JsObject> {
    global(context, "__zw_date")
}

fn construct(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(date) = original(context) else {
        return Ok(JsValue::undefined());
    };
    let constructing = this.as_object().is_some_and(|o| o.is_constructor());
    if !constructing {
        let made = date.construct(&[JsValue::from(now_millis())], None, context)?;
        return JsValue::from(made).to_string(context).map(JsValue::from);
    }
    if args.is_empty() {
        return date
            .construct(&[JsValue::from(now_millis())], None, context)
            .map(JsValue::from);
    }
    if let [only] = args
        && only.is_string()
    {
        let text = string(Some(only), context);
        let made = date.construct(&[js(&text)], None, context)?;
        let millis = get(&made, "getTime", context)
            .as_object()
            .map_or(Ok(f64::NAN), |f| {
                f.call(&made.clone().into(), &[], context)
                    .and_then(|v| v.to_number(context))
            })?;
        if millis.is_nan()
            && let Some(parts) = Parts::read(&text)
        {
            return construct_parts(&date, &parts, context).map(JsValue::from);
        }
        return Ok(made.into());
    }
    date.construct(args, None, context).map(JsValue::from)
}

/// もう過ぎたか（`document.cookie` の `expires`）。読めない形は「過ぎていない」。
pub(super) fn is_past(text: &str) -> bool {
    let Some(parts) = Parts::read(text) else {
        return false;
    };
    let days = days_from_civil(i64::from(parts.year), parts.month as u32, parts.day as u32);
    let seconds = days * 86_400 + i64::from((parts.hour * 60 + parts.minute) * 60 + parts.second);
    let millis = seconds * 1000 - i64::from(parts.offset_seconds.unwrap_or(0)) * 1000;
    (millis as f64) < now_millis()
}

fn now(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(now_millis()))
}

fn parse(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(date) = original(context) else {
        return Ok(JsValue::nan());
    };
    let text = string(args.first(), context);
    let f = get(&date, "parse", context);
    let got = match f.as_object() {
        Some(f) => f
            .call(&date.clone().into(), &[js(&text)], context)?
            .to_number(context)?,
        None => f64::NAN,
    };
    if got.is_nan()
        && let Some(parts) = Parts::read(&text)
    {
        let made = construct_parts(&date, &parts, context)?;
        return get(&made, "getTime", context)
            .as_object()
            .map_or(Ok(JsValue::nan()), |f| f.call(&made.into(), &[], context));
    }
    Ok(JsValue::from(got))
}

fn construct_parts(date: &JsObject, parts: &Parts, context: &mut Context) -> JsResult<JsObject> {
    let numbers = [
        JsValue::from(parts.year),
        JsValue::from(parts.month - 1),
        JsValue::from(parts.day),
        JsValue::from(parts.hour),
        JsValue::from(parts.minute),
        JsValue::from(parts.second),
    ];
    match parts.offset_seconds {
        None => date.construct(&numbers, None, context),
        Some(offset) => {
            let utc = get(date, "UTC", context);
            let millis = match utc.as_object() {
                Some(f) => f
                    .call(&date.clone().into(), &numbers, context)?
                    .to_number(context)?,
                None => f64::NAN,
            };
            let millis = millis - f64::from(offset) * 1000.0;
            date.construct(&[JsValue::from(millis)], None, context)
        }
    }
}

#[derive(Debug, PartialEq)]
struct Parts {
    year: i32,
    month: i32,
    day: i32,
    hour: i32,
    minute: i32,
    second: i32,
    offset_seconds: Option<i32>,
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

impl Parts {
    fn read(text: &str) -> Option<Self> {
        let mut numbers: Vec<i32> = Vec::new();
        let mut month_name: Option<i32> = None;
        let mut time: Option<(i32, i32, i32)> = None;
        let mut offset: Option<i32> = None;
        let mut pm: Option<bool> = None;
        let mut dashed_ymd = false;
        let cleaned = text.replace(',', " ");
        for raw in cleaned.split_whitespace() {
            let token = raw.trim_matches(|c: char| c == '(' || c == ')');
            if token.is_empty() {
                continue;
            }
            let lower = token.to_ascii_lowercase();
            if token.contains(':') && token.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                let mut it = token.split(':');
                let h = it.next()?.parse().ok()?;
                let m = it.next()?.parse().ok()?;
                let s = it
                    .next()
                    .map(|s| s.split('.').next().unwrap_or("0").parse().unwrap_or(0))
                    .unwrap_or(0);
                time = Some((h, m, s));
                continue;
            }
            if let Some(zone) = lower
                .strip_prefix("gmt")
                .or_else(|| lower.strip_prefix("utc"))
                .or_else(|| lower.strip_prefix("z"))
            {
                offset = Some(if zone.is_empty() {
                    0
                } else {
                    zone_offset(zone)?
                });
                continue;
            }
            if lower.starts_with('+') || (lower.starts_with('-') && lower.len() >= 5) {
                offset = Some(zone_offset(&lower)?);
                continue;
            }
            if lower == "am" || lower == "pm" || lower == "a.m." || lower == "p.m." {
                pm = Some(lower.starts_with('p'));
                continue;
            }
            if let Some(index) = MONTHS.iter().position(|m| lower.starts_with(m)) {
                month_name = Some(index as i32 + 1);
                continue;
            }
            if token.contains('/') || token.contains('-') || token.contains('.') {
                let fields: Vec<&str> = token
                    .split(['/', '-', '.'])
                    .filter(|s| !s.is_empty())
                    .collect();
                if fields.iter().all(|f| f.chars().all(|c| c.is_ascii_digit())) {
                    if fields.len() == 3 && fields[0].len() == 4 {
                        dashed_ymd = true;
                    }
                    for f in fields {
                        numbers.push(f.parse().ok()?);
                    }
                    continue;
                }
            }
            if let Ok(n) = token
                .trim_end_matches(|c: char| c.is_ascii_alphabetic())
                .parse::<i32>()
            {
                numbers.push(n);
                continue;
            }
            if lower.chars().all(|c| c.is_ascii_alphabetic()) {
                continue;
            }
            return None;
        }
        let (year, month, day) = match (month_name, numbers.as_slice()) {
            (Some(m), [d, y]) if *y > 31 => (*y, m, *d),
            (Some(m), [y, d]) if *y > 31 => (*y, m, *d),
            (Some(m), [d]) => (2001, m, *d),
            (None, [a, b, c]) => {
                if dashed_ymd || *a > 31 {
                    (*a, *b, *c)
                } else if *c > 31 {
                    (*c, *a, *b)
                } else {
                    (2000 + *c, *a, *b)
                }
            }
            (None, [a, b]) if *a > 31 => (*a, *b, 1),
            _ => return None,
        };
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        let (mut hour, minute, second) = time.unwrap_or((0, 0, 0));
        match pm {
            Some(true) if hour < 12 => hour += 12,
            Some(false) if hour == 12 => hour = 0,
            _ => {}
        }
        Some(Parts {
            year,
            month,
            day,
            hour,
            minute,
            second,
            offset_seconds: offset,
        })
    }
}

fn zone_offset(text: &str) -> Option<i32> {
    let (sign, rest) = match text.as_bytes().first()? {
        b'+' => (1, &text[1..]),
        b'-' => (-1, &text[1..]),
        _ => (1, text),
    };
    let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
    let (h, m) = match digits.len() {
        1 | 2 => (digits.parse::<i32>().ok()?, 0),
        3 => (
            digits[..1].parse::<i32>().ok()?,
            digits[1..].parse::<i32>().ok()?,
        ),
        4 => (
            digits[..2].parse::<i32>().ok()?,
            digits[2..].parse::<i32>().ok()?,
        ),
        _ => return None,
    };
    Some(sign * (h * 3600 + m * 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> (i32, i32, i32, i32, i32, i32, Option<i32>) {
        let p = Parts::read(text).unwrap_or_else(|| panic!("読めない: {text}"));
        (
            p.year,
            p.month,
            p.day,
            p.hour,
            p.minute,
            p.second,
            p.offset_seconds,
        )
    }

    #[test]
    fn a_past_expiry_is_seen() {
        clear_clock();
        assert!(is_past("Thu, 01 Jan 1970 00:00:00 GMT"));
        assert!(!is_past("Fri, 01 Jan 2100 00:00:00 GMT"));
        assert!(!is_past("nonsense"));
    }

    #[test]
    fn the_clock_follows_the_broadcast() {
        set_clock(1_000_000);
        assert!((now_millis() - 1_000_000_000.0).abs() < 1_000.0);
        clear_clock();
        assert!((now_millis() - machine_millis() as f64).abs() < 1_000.0);
    }

    #[test]
    fn browser_style_dates_are_read() {
        assert_eq!(at("2026/09/12 05:00:00"), (2026, 9, 12, 5, 0, 0, None));
        assert_eq!(at("2026/9/12"), (2026, 9, 12, 0, 0, 0, None));
        assert_eq!(at("2026-09-12 05:10"), (2026, 9, 12, 5, 10, 0, None));
        assert_eq!(at("Sep 12, 2026 05:00:00"), (2026, 9, 12, 5, 0, 0, None));
        assert_eq!(
            at("Sat, 12 Sep 2026 05:00:00 GMT"),
            (2026, 9, 12, 5, 0, 0, Some(0))
        );
        assert_eq!(
            at("Sat Sep 12 2026 05:00:00 GMT+0900 (JST)"),
            (2026, 9, 12, 5, 0, 0, Some(32_400))
        );
        assert_eq!(at("9/12/2026 5:00 PM"), (2026, 9, 12, 17, 0, 0, None));
        assert_eq!(at("12 September 2026"), (2026, 9, 12, 0, 0, 0, None));
        assert_eq!(
            at("2026/09/12 05:00:00 +09:00"),
            (2026, 9, 12, 5, 0, 0, Some(32_400))
        );
        assert!(Parts::read("nonsense").is_none());
        assert!(Parts::read("2026/13/40").is_none());
    }
}
