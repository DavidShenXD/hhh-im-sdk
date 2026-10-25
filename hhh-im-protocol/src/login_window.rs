//! “登录窗口”一步（契约 provider-protocol v36）的取值规则：起始网址、要截的跳转开头，以及一个地址算不算截到了。
//!
//! 宿主（Friend 桌面版）、自检工具与插件共用这一份，不依赖网址库：只认 `协议://地址/路径` 这一层。比较时协议名与
//! 地址（`://` 之后、第一个 `/` 之前）不分大小写，其余按字面前缀；地址里带用户名、端口的都按字面比，对不上就不截。
//!
//! # Examples
//!
//! ```
//! use hhh_im_protocol::login_window::{capture_ok, captured, start_url_ok};
//! assert!(start_url_ok("https://xd.slack.com/sso/saml/start"));
//! assert!(start_url_ok("http://127.0.0.1:29527/login"));
//! assert!(!start_url_ok("http://example.com/login"));
//! assert!(capture_ok("slack://"));
//! assert!(capture_ok("https://app.slack.com/t/"));
//! assert!(!capture_ok("https://app.slack.com/"));
//! assert!(!capture_ok("javascript:"));
//! let capture = ["slack://".to_owned(), "https://app.slack.com/t/".to_owned()];
//! assert!(captured(&capture, "SLACK://T1/magic-login-sso/code"));
//! assert!(captured(&capture, "https://APP.slack.com/t/T1/magic-login/code"));
//! assert!(!captured(&capture, "https://app.slack.com.evil.test/t/T1/x"));
//! ```

use crate::budget::{LOGIN_WINDOW_CAPTURE_MAX_CHARS, LOGIN_WINDOW_CAPTURES_MAX};

/// 不能当“要截的开头”的协议：普通网页与浏览器内部的协议（截了就等于截走正常翻页或执行脚本）。
const BLOCKED_SCHEMES: &[&str] = &[
    "http",
    "https",
    "file",
    "javascript",
    "data",
    "about",
    "blob",
    "ws",
    "wss",
    "ftp",
    "view-source",
];

/// 把地址拆成（小写的协议名、小写的地址、`://` 地址之后的其余部分）；不是 `协议://…` 写法时为空。
fn split(raw: &str) -> Option<(String, String, &str)> {
    if raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let (scheme, rest) = raw.split_once("://")?;
    let mut chars = scheme.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    if !first_ok || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')) {
        return None;
    }
    let end = rest.find('/').unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    Some((
        scheme.to_ascii_lowercase(),
        authority.to_ascii_lowercase(),
        tail,
    ))
}

/// 起始网址合格：https 且有地址，或本机回环地址（`localhost`、`127.0.0.1`、`[::1]`，可带端口）的 http（示例插件与测试用）。
pub fn start_url_ok(url: &str) -> bool {
    let Some((scheme, authority, _)) = split(url) else {
        return false;
    };
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    match scheme.as_str() {
        "https" => true,
        "http" => {
            let host = if let Some(v6) = authority.strip_prefix('[') {
                v6.split_once(']').map(|(host, _)| format!("[{host}]"))
            } else {
                Some(authority.split(':').next().unwrap_or_default().to_owned())
            };
            matches!(host.as_deref(), Some("localhost" | "127.0.0.1" | "[::1]"))
        }
        _ => false,
    }
}

/// 一个“要截的开头”合格：不超过 [`LOGIN_WINDOW_CAPTURE_MAX_CHARS`] 个字；自定义协议的 `<协议>://…`（协议不在
/// 网页、本机文件、脚本这些常见协议里），或 https 带地址与至少一段路径（`https://app.slack.com/t/` 可以，只到地址的不行）。
pub fn capture_ok(prefix: &str) -> bool {
    if prefix.chars().count() > LOGIN_WINDOW_CAPTURE_MAX_CHARS {
        return false;
    }
    let Some((scheme, authority, tail)) = split(prefix) else {
        return false;
    };
    if scheme == "https" {
        return !authority.is_empty()
            && !authority.contains('@')
            && tail.starts_with('/')
            && !tail.trim_start_matches('/').is_empty();
    }
    !BLOCKED_SCHEMES.contains(&scheme.as_str())
}

/// 这一步的开头清单合格：1 到 [`LOGIN_WINDOW_CAPTURES_MAX`] 项，每项 [`capture_ok`]。
pub fn captures_ok(captures: &[String]) -> bool {
    (1..=LOGIN_WINDOW_CAPTURES_MAX).contains(&captures.len())
        && captures.iter().all(|c| capture_ok(c))
}

/// 地址算不算截到了：和清单里任一合格的开头对上（协议名、地址不分大小写，其余按字面前缀）。
pub fn captured(captures: &[String], url: &str) -> bool {
    let Some(target) = normalized(url) else {
        return false;
    };
    captures
        .iter()
        .filter(|prefix| capture_ok(prefix))
        .filter_map(|prefix| normalized(prefix))
        .any(|prefix| target.starts_with(&prefix))
}

/// 协议名与地址转小写后的整串（比前缀用）。
fn normalized(raw: &str) -> Option<String> {
    let (scheme, authority, tail) = split(raw)?;
    Some(format!("{scheme}://{authority}{tail}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_urls_are_https_or_loopback_http_only() {
        for good in [
            "https://xd.slack.com/sso/saml/start?redir=%2F",
            "https://slack.com",
            "http://localhost:8080/login",
            "http://127.0.0.1/login",
            "http://[::1]:9000/x",
        ] {
            assert!(start_url_ok(good), "{good}");
        }
        for bad in [
            "http://example.com/login",
            "http://127.0.0.1.evil.test/",
            "https://",
            "https://user@slack.com/",
            "slack://T1/login",
            "file:///C:/x.html",
            "javascript:alert(1)",
            "https://slack.com/a b",
            "",
        ] {
            assert!(!start_url_ok(bad), "{bad}");
        }
    }

    #[test]
    fn captures_are_custom_schemes_or_https_with_a_path() {
        for good in [
            "slack://",
            "slack://T1/magic-login",
            "hhh-sample://done",
            "my.app+x://",
            "https://app.slack.com/t/",
        ] {
            assert!(capture_ok(good), "{good}");
        }
        for bad in [
            "",
            "slack",
            "slack:",
            "1slack://",
            "https://app.slack.com",
            "https://app.slack.com/",
            "https:///t/",
            "https://user@app.slack.com/t/",
            "http://app.slack.com/t/",
            "file:///C:/",
            "javascript://x",
            "data://x",
            "about://blank",
            "ws://x/",
            "view-source://x",
            "sla ck://",
        ] {
            assert!(!capture_ok(bad), "{bad}");
        }
        assert!(!capture_ok(&format!(
            "slack://{}",
            "a".repeat(LOGIN_WINDOW_CAPTURE_MAX_CHARS)
        )));
    }

    #[test]
    fn the_list_needs_one_to_four_good_prefixes() {
        let one = |s: &str| s.to_owned();
        assert!(!captures_ok(&[]));
        assert!(captures_ok(&[one("slack://")]));
        assert!(captures_ok(&vec![one("a://"); LOGIN_WINDOW_CAPTURES_MAX]));
        assert!(!captures_ok(&vec![
            one("a://");
            LOGIN_WINDOW_CAPTURES_MAX + 1
        ]));
        assert!(!captures_ok(&[one("slack://"), one("https://slack.com/")]));
    }

    #[test]
    fn only_matching_prefixes_capture_and_case_matters_only_after_the_host() {
        let capture = vec!["slack://".to_owned(), "https://app.slack.com/t/".to_owned()];
        assert!(captured(&capture, "slack://T1/magic-login-sso/abc"));
        assert!(captured(&capture, "Slack://T1/magic-login-sso/abc"));
        assert!(captured(
            &capture,
            "https://APP.SLACK.COM/t/T1/magic-login/abc"
        ));
        assert!(!captured(
            &capture,
            "https://app.slack.com/T/T1/magic-login/abc"
        ));
        assert!(!captured(&capture, "https://app.slack.com.evil.test/t/T1"));
        assert!(!captured(&capture, "https://app.slack.com@evil.test/t/T1"));
        assert!(!captured(&capture, "https://slack.com/signin"));
        assert!(!captured(&capture, "slackx://T1"));
        assert!(!captured(&capture, "not a url"));
        // 不合格的开头即使写进清单也不截（宿主拒绝之前的兜底）。
        assert!(!captured(&["https://".to_owned()], "https://anything/"));
    }
}
