//! Taking secrets out of what goes into a bundle: an API key, a proxy's password, any token. What is left says
//! whether one was set, and its first and last two characters when it is long enough for that to give nothing away.

use serde_json::Value;

/// What a secret's name looks like, in any case and with any separator: `apiKey`, `api_key`, `proxyPassword`,
/// `token`, `Authorization`.
pub fn secret_name(name: &str) -> bool {
    let flat: String = name.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase();
    const SIGNS: [&str; 9] = ["apikey", "password", "passwd", "secret", "token", "authorization", "bearer", "credential", "privatekey"];
    (flat.ends_with("key") && !flat.ends_with("hotkey")) || SIGNS.iter().any(|s| flat.contains(s))
}

/// A secret as a bundle shows it: "not set", or "set" with its length, and its first and last two characters when
/// it has at least twelve.
pub fn mask(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    match chars.len() {
        0 => "not set".into(),
        n if n >= 12 => format!("set ({n} characters, {}{}…{}{})", chars[0], chars[1], chars[n - 2], chars[n - 1]),
        n => format!("set ({n} characters)"),
    }
}

/// The password in an address such as `http://user:pass@proxy:8080`, taken out.
pub fn url_login(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        out.push_str(head);
        let end = tail.find(|c: char| c.is_whitespace() || c == '/' || c == '"' || c == '\'').unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rfind('@') {
            Some(login_end) => {
                let login = &authority[..login_end];
                match login.split_once(':') {
                    Some((user, _)) => out.push_str(&format!("{user}:[REDACTED]")),
                    None => out.push_str(login),
                }
                out.push_str(&authority[login_end..]);
            }
            None => out.push_str(authority),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Every value under a secret's name masked ([`mask`]), and logins taken out of addresses, all through `value`.
pub fn json(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (name, inner) in map.iter_mut() {
                match inner {
                    Value::String(s) if secret_name(name) => *s = mask(s),
                    _ => json(inner),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(json),
        Value::String(s) if s.contains("://") && s.contains('@') => *s = url_login(s),
        _ => {}
    }
}

/// Each of `secrets` replaced wherever it appears in `text`. Very short ones are left, as they would match anything.
pub fn scrub(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for secret in secrets.iter().filter(|s| s.chars().count() >= 4) {
        if out.contains(secret.as_str()) {
            out = out.replace(secret.as_str(), "[REDACTED]");
        }
    }
    out
}

/// A settings file as a bundle carries it. JSON keeps its shape with secrets masked; the encrypted secrets file
/// shows only which secrets it holds; any other text has `name = value` lines under a secret's name masked. Every
/// known secret is scrubbed besides.
pub fn config_file(name: &str, text: &str, secrets: &[String]) -> String {
    if name.ends_with("secrets.json") {
        let names: Vec<String> = serde_json::from_str::<Value>(text).ok().and_then(|v| v.as_object().map(|m| m.keys().cloned().collect())).unwrap_or_default();
        return serde_json::json!({ "note": "encrypted secrets, left out", "holds": names }).to_string();
    }
    if let Ok(mut value) = serde_json::from_str::<Value>(text) {
        json(&mut value);
        return scrub(&serde_json::to_string_pretty(&value).unwrap_or_default(), secrets);
    }
    let lines: Vec<String> = text
        .lines()
        .map(|line| match line.split_once(['=', ':']) {
            Some((name, value)) if secret_name(name.trim()) => format!("{}{}{}", name, &line[name.len()..name.len() + 1], mask(value.trim())),
            _ => url_login(line),
        })
        .collect();
    scrub(&lines.join("\n"), secrets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn secret_names_are_known_whatever_their_case() {
        for name in ["apiKey", "api_key", "API-KEY", "proxyPassword", "proxy_password", "token", "accessToken", "Authorization", "clientSecret", "key"] {
            assert!(secret_name(name), "{name}");
        }
        for name in ["model", "keepAudio", "keep_audio", "baseUrl", "proxyUser", "shortcuts", "speechLanguage", "hotkey"] {
            assert!(!secret_name(name), "{name}");
        }
    }

    #[test]
    fn a_secret_says_only_whether_it_is_set() {
        assert_eq!(mask(""), "not set");
        assert_eq!(mask("hunter2"), "set (7 characters)");
        assert_eq!(mask("sk-0123456789abcdef"), "set (19 characters, sk…ef)");
    }

    #[test]
    fn a_login_in_an_address_loses_its_password() {
        assert_eq!(url_login("http://me:s3cret@proxy.example:8080/x"), "http://me:[REDACTED]@proxy.example:8080/x");
        assert_eq!(url_login("socks5://proxy:1080"), "socks5://proxy:1080");
        assert_eq!(url_login("see https://a.example and http://u:p@b.example"), "see https://a.example and http://u:[REDACTED]@b.example");
    }

    #[test]
    fn settings_keep_their_shape_without_their_secrets() {
        let mut v = json!({ "apiKey": "sk-live-abcdefghijklmnop", "model": "m", "proxyPassword": "", "proxyUrl": "http://u:pw@h:1", "nested": [{ "token": "t0k3n-value" }], "tokens_in": 12 });
        json(&mut v);
        assert_eq!(v["apiKey"], "set (24 characters, sk…op)");
        assert_eq!(v["proxyPassword"], "not set");
        assert_eq!(v["proxyUrl"], "http://u:[REDACTED]@h:1");
        assert_eq!(v["nested"][0]["token"], "set (11 characters)");
        assert_eq!(v["model"], "m");
        assert_eq!(v["tokens_in"], 12, "counts are not secrets");
    }

    #[test]
    fn other_text_and_known_secrets_are_scrubbed() {
        let text = "proxy_password = hunter22\nmodel=x\nsaw sk-abcdef in a reply";
        let out = config_file("notes.txt", text, &["sk-abcdef".into()]);
        assert!(!out.contains("hunter22") && !out.contains("sk-abcdef"), "{out}");
        assert!(out.contains("model=x"));
        assert_eq!(config_file("secrets.json", r#"{"api-key":"Zm9v","proxy-password":"YmFy"}"#, &[]), r#"{"note":"encrypted secrets, left out","holds":["api-key","proxy-password"]}"#);
    }
}
