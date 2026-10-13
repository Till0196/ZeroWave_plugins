// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 通信（IP の接続、テキストの送信、ネットワークの確認、AIT）。

use super::*;

pub(super) fn is_ip_connected(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(u32::from(crate::resource::net::has_ip())))
}

/// 回線の種類。受像機は Ethernet の「その他」（403）を返す。ネットに出られなければ NaN。
pub(super) fn connection_type(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    if !crate::resource::net::has_ip() {
        return nan();
    }
    Ok(JsValue::from(403))
}

/// `transmitTextDataOverIP(uri, text, charset)`：`Denbun=<テキスト>` を POST する（TR-B14 第三分冊 図 8-4）。
/// 受像機は視聴者に確かめるが、ここは記録に残して送る。
pub(super) fn transmit_text_data(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let uri = string(args.first(), context);
    let text = string(args.get(1), context);
    let charset = string(args.get(2), context);
    let encoding = match charset.to_ascii_uppercase().as_str() {
        "EUC-JP" | "EUC_JP" => encoding_rs::EUC_JP,
        "SHIFT_JIS" | "SHIFT-JIS" | "SJIS" => encoding_rs::SHIFT_JIS,
        _ => return reply_array(context, -1, "", ""),
    };
    let (encoded, _, _) = encoding.encode(&text);
    if encoded.len() > 4096 {
        return reply_array(context, -1, "", "");
    }
    let form = format!("Denbun={}", crate::resource::net::escape(&encoded));
    host(context).requests.push(Request::Log(format!(
        "通信: POST {uri} ({} バイト)",
        form.len()
    )));
    match crate::resource::net::post(&uri, form.as_bytes()) {
        Ok(reply) => {
            let (body, _, _) = encoding.decode(&reply.body);
            reply_array(context, 1, &reply.status.to_string(), &body)
        }
        Err(failure) => reply_array(context, failure.code(), "", ""),
    }
}

fn reply_array(context: &mut Context, code: i32, status: &str, body: &str) -> JsResult<JsValue> {
    let items = [JsValue::from(code), js(status), js(body)];
    Ok(JsArray::from_iter(items, context).into())
}

/// `confirmIPNetwork(destination, type, timeout)` → [届くか, アドレス, 往復のミリ秒]。
/// type 1 は受像機では ping だが、特権が要るので TCP の 80 番で試す。
pub(super) fn confirm_ip_network(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let destination = string(args.first(), context);
    let kind = number(args.get(1), context);
    let timeout = match args.get(2) {
        Some(v) if !v.is_undefined() => number(Some(v), context),
        _ => 4000.0,
    };
    if destination.is_empty() || destination.len() > 255 {
        return nan();
    }
    let host_name =
        crate::resource::net::parse_url(&destination).map_or(destination.clone(), |u| u.host);
    let (ok, address, millis): (bool, String, i32) = if kind == 0.0 {
        match crate::resource::net::resolve(&host_name) {
            Some(address) => (true, address, -1),
            None => (false, String::new(), -1),
        }
    } else if kind == 1.0 {
        let timeout = std::time::Duration::from_millis(timeout.max(1.0) as u64);
        match crate::resource::net::reach(&host_name, 80, timeout) {
            Some((address, millis)) => (true, address, millis as i32),
            None => (false, String::new(), 0),
        }
    } else {
        return nan();
    };
    let items = [
        JsValue::from(ok),
        if address.is_empty() {
            JsValue::null()
        } else {
            js(&address)
        },
        JsValue::from(millis),
    ];
    Ok(JsArray::from_iter(items, context).into())
}

/// `startAITControlledApp(mode, app_id, ait_uri)`: XML の AIT のその番号（無ければ AUTOSTART）を起動させる。
pub(super) fn start_ait_controlled_app(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let app_id = number(args.get(1), context);
    let mut ait_uri = string(args.get(2), context);
    if ait_uri.is_empty() {
        ait_uri = host(context).ait_url.clone().unwrap_or_default();
    }
    if ait_uri.is_empty() {
        return nan();
    }
    let host = host(context);
    let base = host.page.as_ref().map_or(String::new(), |p| p.name.clone());
    let Some(file) = host.resources.get(&base, &ait_uri) else {
        host.requests.push(Request::Log(format!(
            "startAITControlledApp: AIT が取れない {ait_uri}"
        )));
        return nan();
    };
    let xml = String::from_utf8_lossy(&file.body);
    let apps = crate::resource::xait::parse(&xml);
    let wanted = app_id.is_finite().then_some(app_id as u32);
    let Some(app) = apps
        .iter()
        .find(|a| wanted == Some(a.app_id))
        .or_else(|| apps.iter().find(|a| a.control == "AUTOSTART"))
        .or_else(|| apps.first())
    else {
        host.requests.push(Request::Log(format!(
            "startAITControlledApp: AIT が空 {ait_uri}"
        )));
        return nan();
    };
    host.requests.push(Request::LaunchApp {
        org_id: app.org_id,
        app_id: app.app_id,
        url: app.url.clone(),
    });
    Ok(JsValue::from(1))
}

pub(super) fn is_root_certificate_existing(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let kind = number(args.first(), context);
    let id = number(args.get(1), context);
    let version = args
        .get(2)
        .filter(|v| !v.is_undefined())
        .map(|v| number(Some(v), context));
    if !kind.is_finite() || !id.is_finite() {
        return nan();
    }
    let has = host(context).root_certificates.iter().any(|(k, i, v)| {
        f64::from(*k) == kind
            && f64::from(*i) == id
            && version.is_none_or(|wanted| f64::from(*v) >= wanted)
    });
    Ok(JsValue::from(u32::from(has)))
}

pub(super) fn root_certificate_info(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let held = host(context).root_certificates.clone();
    let items: Vec<JsValue> = held
        .iter()
        .map(|(k, i, v)| {
            JsArray::from_iter(
                [JsValue::from(*k), JsValue::from(*i), JsValue::from(*v)],
                context,
            )
            .into()
        })
        .collect();
    Ok(JsArray::from_iter(items, context).into())
}

/// 監視している AIT の中身。まだなら 0、駄目なら −1。先頭は AIT、続く行がアプリ。
pub(super) fn received_ait(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let mut uri = string(args.first(), context);
    if uri.is_empty() {
        uri = host(context).ait_url.clone().unwrap_or_default();
    }
    if uri.is_empty() {
        return Ok(JsValue::from(-1));
    }
    let host = host(context);
    let base = host.page.as_ref().map_or(String::new(), |p| p.name.clone());
    let Some(file) = host.resources.get(&base, &uri) else {
        return Ok(JsValue::from(0));
    };
    let apps = crate::resource::xait::parse(&String::from_utf8_lossy(&file.body));
    if apps.is_empty() {
        return Ok(JsValue::from(-1));
    }
    let mut rows: Vec<JsValue> = Vec::with_capacity(apps.len() + 1);
    rows.push(
        JsArray::from_iter(
            [JsValue::from(0), JsValue::from(apps.len() as u32)],
            context,
        )
        .into(),
    );
    for app in &apps {
        let control = match app.control.as_str() {
            "AUTOSTART" => 1,
            "PRESENT" => 2,
            "DESTROY" => 3,
            "KILL" => 4,
            _ => 0,
        };
        rows.push(
            JsArray::from_iter(
                [
                    JsValue::from(app.org_id),
                    JsValue::from(app.app_id),
                    JsValue::from(control),
                    js(&app.url),
                ],
                context,
            )
            .into(),
        );
    }
    Ok(JsArray::from_iter(rows, context).into())
}

pub(super) fn start_receiving_ait(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let uri = string(args.first(), context);
    if uri.is_empty() {
        return nan();
    }
    host(context).ait_url = Some(uri);
    Ok(JsValue::from(1))
}

pub(super) fn stop_receiving_ait(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    host(context).ait_url = None;
    Ok(JsValue::from(1))
}
