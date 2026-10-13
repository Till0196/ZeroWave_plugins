// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 文書の起動・読み直し・終わり、モジュールのロック、放送中かの問い合わせ。

use super::*;

pub(super) fn launch_document(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let host = host(context);
    // 文書の中の場所（`#body`）へは読み直さずフォーカスを移す（読み直すと onload が繰り返す文書がある）。
    if let Some(fragment) = name.strip_prefix('#') {
        host.fragment = Some(fragment.to_owned());
        return Ok(JsValue::from(0));
    }
    let base = host.page.as_ref().map_or(String::new(), |p| p.name.clone());
    if Resources::locate(&base, &name).is_none() {
        return nan();
    }
    // 届いていなくても離れる: 開くほうが待つ（`Browser::launch`）。
    host.launch = Some(name);
    Err(leaving())
}

pub(super) fn reload_document(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let host = host(context);
    let Some(name) = host.page.as_ref().map(|p| p.name.clone()) else {
        return nan();
    };
    host.launch = Some(name);
    Err(leaving())
}

pub(super) fn quit_document(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let host = host(context);
    host.quit = true;
    Err(leaving())
}

/// `getActiveDocument()`。通信の文書はスキームとホストを除いたパス（TR-B14 第三分冊）。
pub(super) fn active_document(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = host(context).page.as_ref().map(|p| p.name.clone());
    Ok(name.map_or(JsValue::null(), |name| {
        if let Some(url) = crate::resource::net::parse_url(&name) {
            js(&url.path)
        } else {
            js(&name)
        }
    }))
}

pub(super) fn lock_module(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    lock_module_with(args, context, -1)
}

pub(super) fn lock_module_ex(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    lock_module_with(args, context, -2)
}

/// `lockModuleOnMemory` と `Ex`。一覧（DII）に無いモジュールは、無印なら status −1、
/// Ex なら −2 の `ModuleLocked` を発生させる。一覧がまだなら来てから決める。
fn lock_module_with(args: &[JsValue], context: &mut Context, missing: i32) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let host = host(context);
    let base = host.page.as_ref().map_or(String::new(), |p| p.name.clone());
    let Some(located) = Resources::locate(&base, &name) else {
        return nan();
    };
    let key = (located.component, located.module);
    let ex = missing == -2;
    match host.locked.iter_mut().find(|lock| lock.key == key) {
        Some(lock) => lock.ex |= ex,
        None => host.locked.push(crate::runtime::Lock { key, ex }),
    }
    if host.resources.has_module(key.0, key.1) {
        host.pending_locked.push((name, 0));
    } else if host.resources.has_listing(key.0) && !host.resources.is_listed(key.0, key.1) {
        host.pending_locked.push((name, missing));
    }
    Ok(JsValue::from(1))
}

pub(super) fn unlock_module(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let host = host(context);
    let base = host.page.as_ref().map_or(String::new(), |p| p.name.clone());
    let Some(located) = Resources::locate(&base, &name) else {
        return nan();
    };
    host.locked
        .retain(|lock| lock.key != (located.component, located.module));
    Ok(JsValue::from(1))
}

pub(super) fn unlock_all_modules(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    host(context).locked.clear();
    Ok(JsValue::from(1))
}

/// `getLockedModuleInfo()`：`[名前, ロックの種類（1 無印、2 Ex）, 状態（1 揃った、2 まだ）]` の並び。
pub(super) fn locked_module_info(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let host = host(context);
    let locked: Vec<(crate::runtime::Lock, bool)> = host
        .locked
        .iter()
        .map(|lock| (*lock, host.resources.has_module(lock.key.0, lock.key.1)))
        .collect();
    let rows: Vec<JsValue> = locked
        .into_iter()
        .map(|(lock, arrived)| {
            let (component, module) = lock.key;
            let name = format!("/{component:02X}/{module:04X}");
            let items = [
                js(&name),
                JsValue::from(if lock.ex { 2 } else { 1 }),
                JsValue::from(if arrived { 1 } else { 2 }),
            ];
            JsArray::from_iter(items, context).into()
        })
        .collect();
    Ok(JsArray::from_iter(rows, context).into())
}

pub(super) fn is_being_broadcast(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    let host = host(context);
    let base = host.page.as_ref().map_or(String::new(), |p| p.name.clone());
    let Some(located) = Resources::locate(&base, &name) else {
        return nan();
    };
    Ok(JsValue::from(
        host.resources.is_listed(located.component, located.module),
    ))
}

pub(super) fn free_memory(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(10 * 1024 * 1024))
}
