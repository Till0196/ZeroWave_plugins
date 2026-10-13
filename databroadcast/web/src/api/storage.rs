// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! Web Storage（STD-B62 第二分冊 3.3.5）。`ureg`・`greg` もここに置き、
//! `navigator.bmlCompat.browserPseudo.Ureg[n]` からも同じ物を見る。

use boa_engine::{Context, JsResult, JsValue};

use super::{accessor, js, method, object, set_global, string, with_bridge};

pub(super) fn install(context: &mut Context) {
    for name in ["localStorage", "sessionStorage"] {
        if super::global(context, name).is_some() {
            continue;
        }
        let s = object(context);
        method(&s, context, "getItem", 1, get_item);
        method(&s, context, "setItem", 2, set_item);
        method(&s, context, "removeItem", 1, remove_item);
        method(&s, context, "clear", 0, clear);
        method(&s, context, "key", 1, key);
        accessor(&s, context, "length", Some(length), None);
        set_global(context, name, s.into());
    }
}

fn shared(key: &str) -> bool {
    ["ureg", "greg"].iter().any(|prefix| {
        key.strip_prefix(prefix).is_some_and(|n| {
            (1..=2).contains(&n.len())
                && n.bytes().all(|b| b.is_ascii_digit())
                && n.parse::<u8>().is_ok_and(|n| n < 64)
        })
    })
}

fn station() -> String {
    with_bridge(|bridge, _| {
        let host = &bridge.borrow().host;
        format!("{:04x}.{:04x}/", host.network, host.service)
    })
    .unwrap_or_default()
}

/// ストレージの中のキー。共通のレジスターの外は局ごとに分ける（ある局が置いた物を別の局に読ませない）。
fn scoped(key: &str) -> String {
    if shared(key) {
        key.to_owned()
    } else {
        format!("{}{key}", station())
    }
}

const ITEMS_MOST: usize = 4096;
const BYTES_MOST: usize = 4 << 20;

pub(crate) fn read(key: &str) -> Option<String> {
    // 視聴者の居住地（TR-B39 表 10-4）。都道府県と地域は未設定（0）。
    match key {
        "_zipcode" => {
            return with_bridge(|bridge, _| bridge.borrow().host.zipcode.clone());
        }
        "_prefecture" | "_regioncode" => return Some(String::from("0")),
        _ => {}
    }
    let key = scoped(key);
    super::kept(|kept| kept.storage.get(&key).cloned())
}

pub(crate) fn write(key: &str, value: &str) {
    let key = scoped(key);
    super::kept(|kept| {
        let had = kept.storage.get(&key).map_or(0, |v| key.len() + v.len());
        let total: usize = kept.storage.iter().map(|(k, v)| k.len() + v.len()).sum();
        let count = kept.storage.len() + usize::from(had == 0);
        if count > ITEMS_MOST || total - had + key.len() + value.len() > BYTES_MOST {
            return;
        }
        kept.storage.insert(key, value.to_owned());
    });
}

fn get_item(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let key = string(args.first(), context);
    Ok(read(&key).map_or(JsValue::null(), |v| js(&v)))
}

fn set_item(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let key = string(args.first(), context);
    let value = string(args.get(1), context);
    write(&key, &value);
    Ok(JsValue::undefined())
}

fn remove_item(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let key = scoped(&string(args.first(), context));
    super::kept(|kept| {
        kept.storage.remove(&key);
    });
    Ok(JsValue::undefined())
}

fn clear(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    let station = station();
    super::kept(|kept| {
        kept.storage
            .retain(|key, _| !shared(key) && !key.starts_with(&station));
    });
    Ok(JsValue::undefined())
}

fn keys() -> Vec<String> {
    let station = station();
    super::kept(|kept| {
        kept.storage
            .keys()
            .filter_map(|key| {
                if shared(key) {
                    Some(key.clone())
                } else {
                    key.strip_prefix(&station).map(str::to_owned)
                }
            })
            .collect()
    })
}

fn key(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let index = super::number(args.first(), context);
    let keys = keys();
    Ok(
        if index.is_finite() && index >= 0.0 && (index as usize) < keys.len() {
            js(&keys[index as usize])
        } else {
            JsValue::null()
        },
    )
}

fn length(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(keys().len() as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_registers_are_shared() {
        assert!(shared("ureg0") && shared("greg63"));
        assert!(!shared("ureg64") && !shared("ureg+1") && !shared("ureg") && !shared("x"));
    }
}
