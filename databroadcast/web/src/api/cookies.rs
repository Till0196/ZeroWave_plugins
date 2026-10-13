// Copyright 2026 Till0196
// SPDX-License-Identifier: Apache-2.0

//! cookie の保存先。名前と値は設定したホストの名前ごとに持ち、同じホストか、`Domain` で
//! 許されたサブドメインにだけ付ける。ある局のサーバーが置いたセッションの cookie を、別の局の
//! サーバーや転送先へ持って行かない。

use std::collections::BTreeMap;

/// 一つのホスト名で持てる cookie の数、名前の数、一つの cookie の長さの上限。
const PER_DOMAIN: usize = 64;
const DOMAINS: usize = 64;
const PAIR_MOST: usize = 4096;

#[derive(Debug, Default)]
pub struct Jar {
    /// (ホスト名, サブドメインにも付けるか) → cookie の名前 → 値。
    by_domain: BTreeMap<(String, bool), BTreeMap<String, String>>,
}

impl Jar {
    pub const fn new() -> Self {
        Self {
            by_domain: BTreeMap::new(),
        }
    }

    pub fn header_for(&self, host: &str) -> String {
        let host = host.to_ascii_lowercase();
        self.by_domain
            .iter()
            .filter(|((domain, below), _)| {
                *domain == host || (*below && host.ends_with(&format!(".{domain}")))
            })
            .flat_map(|(_, pairs)| pairs.iter())
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// `host` から届いた `Set-Cookie` の一つ。`Domain` は `host` 自身かその親ドメインだけを受け付ける。
    pub fn set(&mut self, host: &str, line: &str, is_past: impl Fn(&str) -> bool) {
        let host = host.to_ascii_lowercase();
        let mut parts = line.split(';');
        let Some((name, value)) = parts.next().and_then(|first| first.split_once('=')) else {
            return;
        };
        let (name, value) = (name.trim(), value.trim());
        if name.is_empty() || name.len() + value.len() > PAIR_MOST {
            return;
        }
        let mut gone = value.is_empty();
        let mut domain = (host.clone(), false);
        for part in parts {
            let Some((key, said)) = part.split_once('=') else {
                continue;
            };
            let said = said.trim();
            match key.trim().to_ascii_lowercase().as_str() {
                "expires" => gone |= is_past(said),
                "max-age" => gone |= said.parse::<i64>().is_ok_and(|age| age <= 0),
                "domain" => {
                    let wanted = said.trim_start_matches('.').to_ascii_lowercase();
                    let matches = wanted == host || host.ends_with(&format!(".{wanted}"));
                    if !matches || !wanted.contains('.') {
                        return;
                    }
                    domain = (wanted, true);
                }
                _ => {}
            }
        }
        if gone {
            if let Some(pairs) = self.by_domain.get_mut(&domain) {
                pairs.remove(name);
                if pairs.is_empty() {
                    self.by_domain.remove(&domain);
                }
            }
            return;
        }
        if !self.by_domain.contains_key(&domain) && self.by_domain.len() >= DOMAINS {
            return;
        }
        let pairs = self.by_domain.entry(domain).or_default();
        if !pairs.contains_key(name) && pairs.len() >= PER_DOMAIN {
            return;
        }
        pairs.insert(name.to_owned(), value.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cookie_goes_back_only_to_where_it_came_from() {
        let mut jar = Jar::new();
        jar.set("api.a.example.jp", "session=1; Path=/", |_| false);
        assert_eq!(jar.header_for("api.a.example.jp"), "session=1");
        assert_eq!(jar.header_for("b.example.jp"), "", "別の局へは付けない");
        jar.set("api.a.example.jp", "shared=2; Domain=.a.example.jp", |_| {
            false
        });
        assert_eq!(jar.header_for("www.a.example.jp"), "shared=2");
        jar.set("api.a.example.jp", "evil=3; Domain=b.example.jp", |_| false);
        jar.set("api.a.example.jp", "tld=4; Domain=jp", |_| false);
        assert_eq!(jar.header_for("b.example.jp"), "");
        jar.set("api.a.example.jp", "session=; Max-Age=0", |_| false);
        assert_eq!(jar.header_for("api.a.example.jp"), "shared=2");
    }
}
