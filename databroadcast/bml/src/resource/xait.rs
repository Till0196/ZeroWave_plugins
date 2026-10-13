// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! XML の AIT（TR-B14 第三分冊 8.3、`isdb:ServiceDiscovery`）。通信のアプリの一覧。

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Application {
    pub org_id: u32,
    pub app_id: u32,
    pub control: String,
    pub url: String,
}

pub fn parse(xml: &str) -> Vec<Application> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some((body, after)) = element(rest, "Application") {
        rest = after;
        let Some(base) = element(body, "URLBase").map(|(b, _)| b.trim()) else {
            continue;
        };
        let location = element(body, "applicationLocation").map_or("", |(b, _)| b.trim());
        let url = if location.starts_with("http://") || location.starts_with("https://") {
            location.to_owned()
        } else {
            format!("{}{}", base, location.trim_start_matches('/'))
        };
        out.push(Application {
            org_id: element(body, "orgId")
                .and_then(|(b, _)| b.trim().parse().ok())
                .unwrap_or(0),
            app_id: element(body, "appId")
                .and_then(|(b, _)| b.trim().parse().ok())
                .unwrap_or(0),
            control: element(body, "controlCode")
                .map_or(String::new(), |(b, _)| b.trim().to_owned()),
            url,
        });
    }
    out
}

fn element<'a>(xml: &'a str, name: &str) -> Option<(&'a str, &'a str)> {
    let mut from = 0;
    loop {
        let at = xml[from..].find('<')? + from;
        let tag_end = xml[at..].find('>')? + at;
        let tag = &xml[at + 1..tag_end];
        let local = tag
            .split_whitespace()
            .next()
            .unwrap_or("")
            .rsplit(':')
            .next()
            .unwrap_or("");
        if local == name && !tag.ends_with('/') {
            let body_start = tag_end + 1;
            let mut search = body_start;
            loop {
                let end = xml[search..].find("</")? + search;
                let end_tag_end = xml[end..].find('>')? + end;
                let end_name = xml[end + 2..end_tag_end]
                    .trim()
                    .rsplit(':')
                    .next()
                    .unwrap_or("");
                if end_name == name {
                    return Some((&xml[body_start..end], &xml[end_tag_end + 1..]));
                }
                search = end_tag_end + 1;
            }
        }
        from = tag_end + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_application_list_is_read() {
        let xml = r#"<?xml version="1.0"?><isdb:ServiceDiscovery xmlns:isdb="urn:arib:isdb:2012">
<isdb:ApplicationDiscovery DomainName="x.jp"><isdb:ApplicationList>
<isdb:Application><isdb:applicationIdentifier><isdb:orgId>22</isdb:orgId><isdb:appId>1</isdb:appId></isdb:applicationIdentifier>
<isdb:applicationDescriptor><isdb:controlCode>AUTOSTART</isdb:controlCode></isdb:applicationDescriptor>
<isdb:applicationTransport xsi:type="isdb:HTTPTransportType"><isdb:URLBase>http://x.jp/hc/</isdb:URLBase></isdb:applicationTransport>
<isdb:applicationLocation>entry/startup.html</isdb:applicationLocation></isdb:Application>
</isdb:ApplicationList></isdb:ApplicationDiscovery></isdb:ServiceDiscovery>"#;
        let apps = parse(xml);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].org_id, 22);
        assert_eq!(apps[0].app_id, 1);
        assert_eq!(apps[0].control, "AUTOSTART");
        assert_eq!(apps[0].url, "http://x.jp/hc/entry/startup.html");
    }
}
