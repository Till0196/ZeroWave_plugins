// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! 受像機が返す識別情報（メーカー ID、機種、OUI、UA、ブラウザの名前とバージョン）と、対応の問い合わせ。

use super::*;

/// 返すメーカー ID（`%02X`）の既定。局の一覧に載る値を避けた、意味のない値。
/// 変えるのは `[data] maker_id` か `ZW_MAKER_ID`。0x28 は「示さない」の意味。
pub const MAKER_ID: &str = "F7";

/// 返す OUI の既定。IEEE が割り当てない「ローカルに管理する」範囲（SLAP の AAI）。
pub const MAKER_OUI: &str = "02C4E1";

/// 返す機種の既定。意味のない文字列。変えるのは `[data] model_id` か `ZW_MODEL_ID`。
pub const MODEL_ID: &str = "M8Q2";

pub fn model_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        zwsdk::plug::host::cfg("receiver.model_id")
            .filter(|id| {
                !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
            .unwrap_or_else(|| String::from(MODEL_ID))
    })
}

pub fn maker_oui() -> &'static str {
    static OUI: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    OUI.get_or_init(|| {
        zwsdk::plug::host::cfg("receiver.maker_oui")
            .filter(|oui| oui.len() == 6 && oui.chars().all(|c| c.is_ascii_hexdigit()))
            .map(|oui| oui.to_ascii_uppercase())
            .unwrap_or_else(|| String::from(MAKER_OUI))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    /// BML（TS のデータ放送）。設定 `user_agent`。
    Bml,
    /// AIT で起動した Hybridcast（`Hybridcast/1.0`）。設定 `user_agent_hybridcast`。
    Hybridcast,
    /// MH-AIT で起動した MMT のアプリ（`A-PAB/1.0`）。設定 `user_agent_apab`。
    Apab,
}

impl Agent {
    fn key(self) -> &'static str {
        match self {
            Self::Bml => "user_agent",
            Self::Hybridcast => "user_agent_hybridcast",
            Self::Apab => "user_agent_apab",
        }
    }
}

/// 送る UA。`agent` の設定にあるときだけその文字列を使い、空なら送らない。
pub fn user_agent(agent: Agent) -> Option<String> {
    zwsdk::plug::host::cfg(agent.key())
        .map(|agent| agent.trim().to_owned())
        .filter(|agent| !agent.is_empty() && !agent.contains(['\r', '\n']))
}

pub fn maker_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        zwsdk::plug::host::cfg("receiver.maker_id")
            .filter(|id| id.len() == 2 && id.chars().all(|c| c.is_ascii_hexdigit()))
            .map(|id| id.to_ascii_uppercase())
            .unwrap_or_else(|| String::from(MAKER_ID))
    })
}

pub(super) fn browser_version(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (major, minor) = browser_version_parts();
    let items = [js(maker_id()), js(&browser_name()), js(&major), js(&minor)];
    Ok(JsArray::from_iter(items, context).into())
}

pub const BROWSER_NAME: &str = "BRW9K";
pub const BROWSER_VERSION: &str = "1.0";

/// 返すブラウザ名（設定 `browser_name`）。STD-B24 7.6 に合わなければ [`BROWSER_NAME`]。
pub fn browser_name() -> String {
    zwsdk::plug::host::cfg("browser_name")
        .map(|name| name.trim().to_owned())
        .filter(|name| {
            !name.is_empty()
                && name.len() <= 20
                && name
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
        })
        .unwrap_or_else(|| String::from(BROWSER_NAME))
}

/// 返すバージョン（設定 `browser_version`、`主.副`）。999 までの数でなければ [`BROWSER_VERSION`]。
pub fn browser_version_text() -> String {
    zwsdk::plug::host::cfg("browser_version")
        .map(|version| version.trim().to_owned())
        .filter(|version| version_parts(version).is_some())
        .unwrap_or_else(|| String::from(BROWSER_VERSION))
}

fn browser_version_parts() -> (String, String) {
    version_parts(&browser_version_text()).unwrap_or_default()
}

fn version_parts(text: &str) -> Option<(String, String)> {
    let (major, minor) = text.split_once('.').unwrap_or((text, "0"));
    let three = |part: &str| -> Option<String> {
        let number: u32 = part.trim().parse().ok().filter(|n| *n <= 999)?;
        Some(format!("{number:03}"))
    };
    Some((three(major)?, three(minor)?))
}

pub(super) fn browser_support(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let provider = string(args.first(), context);
    let function_name = string(args.get(1), context);
    let extra: Vec<String> = args[2.min(args.len())..]
        .iter()
        .map(|value| string(Some(value), context))
        .collect();
    let supported = match (
        provider.as_str(),
        function_name.as_str(),
        extra.first().map(String::as_str),
    ) {
        // バージョンは "3" とも "3.0" とも問い合わせられる。
        ("ARIB", "BMLversion", Some(version)) => version.split('.').next() == Some("3"),
        // STD-B24 第二編 付属 2 表 11-2 のグループのうち、中身が全部実装されているものだけ対応と返す。EPG.Basic は
        // 番組の情報を返せるので例外。通信のグループはネットに出られるときだけ。
        ("ARIB", "APIGroup", Some(group)) => {
            matches!(
                group,
                "Class.BinaryTable"
                    | "EPG.Basic"
                    | "EPG.Basic2"
                    | "CC.Control"
                    | "Persistent.Basic"
                    | "Ctrl.Basic"
                    | "Ctrl.Basic2"
                    | "Ctrl.Cache"
                    | "Ctrl.Cache.Ext"
                    | "Ctrl.Cache2"
                    | "Ctrl.Com"
                    | "Ctrl.Link"
                    | "Ctrl.Media"
                    | "Ctrl.NPT"
                    | "Ctrl.Quit"
                    | "Ctrl.Screen"
                    | "Ctrl.Time"
                    | "Ctrl.Version"
                    | "Ctrl.AppVersion"
                    | "Ctrl.MobileDisplay"
                    | "RomSound.Basic"
                    | "Timer.Basic"
                    | "Timer.Ext"
                    | "Timer.DateMode"
                    | "Misc.Basic"
                    | "Misc.DRCS"
                    | "Misc.DRCS.unload"
                    | "Misc.Greg"
                    | "Com.IP"
                    | "Com.IP.GetType"
                    | "Com.IP.Transmit"
                    | "Com.IP.confirmIP"
                    | "Com.Certificate"
                    | "AITControlledApp.Start"
            ) && (!(group.starts_with("Com.") || group.starts_with("AITControlled"))
                || crate::resource::net::has_ip())
        }
        ("ARIB", "AudioFile", _) => true,
        ("ARIB", "AITControlledAppEngineFunction", Some("IPTV-F")) => {
            extra.get(1).is_some_and(|f| f.starts_with("HTML5"))
        }
        ("ARIB", "AITTransportMethod", Some("XML")) => {
            extra.get(1).is_none_or(|m| m == "HTTP" || m == "HTTPS")
        }
        ("ARIB", "Storage", _) => false,
        // C プロファイルは `DPACpro` で問い合わせる（TR-B14 第三分冊 8.1.15 (13) 表 8-3）。
        ("DPACpro", "APIGroup", Some(group)) => {
            matches!(
                group,
                "Misc.Basic"
                    | "EPG.Basic"
                    | "Persistent.Basic"
                    | "Ctrl.Basic"
                    | "Ctrl.RAVersion"
                    | "RomSound.Basic"
                    | "Timer.Basic"
                    | "Timer.DateMode"
                    | "Xdpa.ComBrowserUA"
                    | "Xdpa.getIRDID"
            ) || (matches!(group, "Com.IP" | "Com.IP.GetType" | "Com.IP.Transmit")
                && crate::resource::net::has_ip())
        }
        ("DPACpro", "BMLversion", Some(version)) => version.split('.').next() == Some("12"),
        ("DPACpro", "BMLversion", None) => true,
        ("DPACpro", "OSDPixelSize", Some("240x480")) => true,
        ("DPACpro", "ResidentApp", _) => false,
        ("DPACpro", "WriteCproBM", _) => false,
        ("DPACpro", "MediaDecoder", Some(scheme)) => {
            let media = extra.get(1).map(String::as_str).unwrap_or("");
            matches!(scheme, "arib-dc" | "arib" | "http" | "https")
                && matches!(
                    media,
                    "image/gif"
                        | "image/jpeg"
                        | "image/X-arib-png"
                        | "image/png"
                        | "audio/X-arib-mpeg2-aac"
                        | "audio/X-arib-aiff"
                )
        }
        ("nvram", "NVRAM", _) => true,
        ("nvram", "BroadcasterID", _) => true,
        ("nvram", "SecureArea", _) => false,
        ("IPTV", _, _) => false,
        _ => false,
    };
    Ok(JsValue::from(u8::from(supported)))
}

pub(super) fn browser_status(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let provider = string(args.first(), context);
    let name = string(args.get(1), context);
    let state = string(args.get(2), context);
    let web = host(context)
        .page
        .as_ref()
        .is_some_and(|p| p.name.starts_with("http://") || p.name.starts_with("https://"));
    Ok(match (provider.as_str(), name.as_str(), state.as_str()) {
        ("ARIB", "IRD", _) => JsValue::from(1),
        (_, "IRDState", "Link") => JsValue::from(u8::from(web)),
        (_, "IRDState", "Broadcast") => JsValue::from(u8::from(!web)),
        _ => JsValue::from(0),
    })
}

/// `getResidentAppVersion(名前)`。`AITControlledApp` は Hybridcast に対応しているときだけ値を返す。
pub(super) fn resident_app_version(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = string(args.first(), context);
    if name != "AITControlledApp" {
        return Ok(JsValue::null());
    }
    let items = [
        js(maker_id()),
        js("AITControlledApp"),
        js("001"),
        js("000"),
        js("000"),
    ];
    Ok(JsArray::from_iter(items, context).into())
}

/// `getIRDID(type)`：設定の値を十六進 12 桁で（無ければ null）。種別は見ない。
pub(super) fn ird_id(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let cas_id = host(context).cas_id.clone();
    if cas_id.is_empty() {
        return Ok(JsValue::null());
    }
    Ok(JsValue::from(JsString::from(cas_id.as_str())))
}

#[cfg(test)]
mod agent_tests {
    use super::*;

    #[test]
    fn the_browser_name_and_version_fall_back_to_made_up_ones() {
        let made_up = || (String::from("001"), String::from("000"));
        zwsdk::plug::host::set_cfg("browser_name", "");
        zwsdk::plug::host::set_cfg("browser_version", "");
        assert_eq!(browser_name(), BROWSER_NAME);
        assert_eq!(browser_version_parts(), made_up());
        zwsdk::plug::host::set_cfg("browser_name", "MYBROWSER2");
        zwsdk::plug::host::set_cfg("browser_version", "1.5");
        assert_eq!(browser_name(), "MYBROWSER2");
        assert_eq!(
            browser_version_parts(),
            (String::from("001"), String::from("005"))
        );
        zwsdk::plug::host::set_cfg("browser_name", "lower");
        assert_eq!(browser_name(), BROWSER_NAME, "小文字は規格に合わない");
        zwsdk::plug::host::set_cfg("browser_version", "v2");
        assert_eq!(browser_version_text(), BROWSER_VERSION);
        assert_eq!(browser_version_parts(), made_up());
        zwsdk::plug::host::set_cfg("browser_name", "");
        zwsdk::plug::host::set_cfg("browser_version", "");
    }

    #[test]
    fn the_user_agent_is_only_what_was_set() {
        let set = zwsdk::plug::host::set_cfg;
        set("user_agent", "");
        assert_eq!(user_agent(Agent::Bml), None);
        set("user_agent", "  Foo/1.0 (bar)  ");
        assert_eq!(user_agent(Agent::Bml).as_deref(), Some("Foo/1.0 (bar)"));
        set("user_agent", "Foo\r\nX-Evil: 1");
        assert_eq!(user_agent(Agent::Bml), None);
        set("user_agent", "12-X/003.220");
        set("user_agent_hybridcast", "Hybridcast/1.0 (HD,HD;)");
        set("user_agent_apab", "");
        assert_eq!(user_agent(Agent::Bml).as_deref(), Some("12-X/003.220"));
        assert_eq!(
            user_agent(Agent::Hybridcast).as_deref(),
            Some("Hybridcast/1.0 (HD,HD;)")
        );
        assert_eq!(user_agent(Agent::Apab), None);
        set("user_agent", "");
        set("user_agent_hybridcast", "");
    }
}
