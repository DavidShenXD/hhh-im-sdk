//! 跳转策略（对应 `reqwest::redirect`）：缺省至多跟 10 次；`Policy::none()` 不跟；`Policy::limited(n)`；
//! `Policy::custom(…)` 自己判断。跟随时：换了主机（或端口）就去掉认证类请求头；带上来源页（https 跳到 http 时不带）。

use std::error::Error as StdError;
use std::fmt;

use http::header::{AUTHORIZATION, COOKIE, PROXY_AUTHORIZATION, WWW_AUTHENTICATE};
use http::{HeaderMap, HeaderValue, StatusCode};
use url::Url;

/// 跳转策略。
pub struct Policy {
    inner: PolicyKind,
}

enum PolicyKind {
    Custom(Box<dyn Fn(Attempt<'_>) -> Action + Send + Sync + 'static>),
    Limit(usize),
    None,
}

/// 一次跳转的情况：状态码、要跳去的地址、之前经过的地址（第一个是最初的请求地址）。
#[derive(Debug)]
pub struct Attempt<'a> {
    status: StatusCode,
    next: &'a Url,
    previous: &'a [Url],
}

/// 自定义策略对一次跳转的决定。
#[derive(Debug)]
pub struct Action {
    inner: ActionKind,
}

#[derive(Debug)]
pub(crate) enum ActionKind {
    Follow,
    Stop,
    Error(Box<dyn StdError + Send + Sync>),
}

impl Policy {
    /// 至多跟 `max` 次。
    pub fn limited(max: usize) -> Self {
        Self {
            inner: PolicyKind::Limit(max),
        }
    }

    /// 不跟：跳转回应原样交给调用方。
    pub fn none() -> Self {
        Self {
            inner: PolicyKind::None,
        }
    }

    /// 自己判断每一次跳转。
    pub fn custom<T>(policy: T) -> Self
    where
        T: Fn(Attempt<'_>) -> Action + Send + Sync + 'static,
    {
        Self {
            inner: PolicyKind::Custom(Box::new(policy)),
        }
    }

    /// 按策略对一次跳转做决定（自定义策略里可以把别的策略当默认值用）。
    pub fn redirect(&self, attempt: Attempt<'_>) -> Action {
        match &self.inner {
            PolicyKind::Custom(custom) => custom(attempt),
            // previous 的第一个是最初的地址，不算一次跳转。
            PolicyKind::Limit(max) if attempt.previous.len() > *max => {
                attempt.error(TooManyRedirects)
            }
            PolicyKind::Limit(_) => attempt.follow(),
            PolicyKind::None => attempt.stop(),
        }
    }

    pub(crate) fn check(&self, status: StatusCode, next: &Url, previous: &[Url]) -> ActionKind {
        self.redirect(Attempt {
            status,
            next,
            previous,
        })
        .inner
    }

    pub(crate) fn is_none(&self) -> bool {
        matches!(self.inner, PolicyKind::None)
    }
}

impl Default for Policy {
    fn default() -> Self {
        Self::limited(10)
    }
}

impl fmt::Debug for Policy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inner {
            PolicyKind::Custom(..) => f.write_str("Policy(Custom)"),
            PolicyKind::Limit(max) => write!(f, "Policy(Limit({max}))"),
            PolicyKind::None => f.write_str("Policy(None)"),
        }
    }
}

impl Attempt<'_> {
    /// 跳转回应的状态码。
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// 要跳去的地址。
    pub fn url(&self) -> &Url {
        self.next
    }

    /// 之前经过的地址。
    pub fn previous(&self) -> &[Url] {
        self.previous
    }

    /// 跟。
    pub fn follow(self) -> Action {
        Action {
            inner: ActionKind::Follow,
        }
    }

    /// 不跟，把这次跳转回应交给调用方。
    pub fn stop(self) -> Action {
        Action {
            inner: ActionKind::Stop,
        }
    }

    /// 报错。
    pub fn error<E: Into<Box<dyn StdError + Send + Sync>>>(self, error: E) -> Action {
        Action {
            inner: ActionKind::Error(error.into()),
        }
    }
}

#[derive(Debug)]
pub(crate) struct TooManyRedirects;

impl fmt::Display for TooManyRedirects {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("too many redirects")
    }
}

impl StdError for TooManyRedirects {}

/// 这一跳是否换了主机（或端口）。
pub(crate) fn crosses_host(next: &Url, previous: &[Url]) -> bool {
    previous.last().is_some_and(|last| {
        next.host_str() != last.host_str()
            || next.port_or_known_default() != last.port_or_known_default()
    })
}

/// 去掉认证类请求头（换了主机时用）。
pub(crate) fn remove_credentials(headers: &mut HeaderMap) {
    headers.remove(AUTHORIZATION);
    headers.remove(COOKIE);
    headers.remove("cookie2");
    headers.remove(PROXY_AUTHORIZATION);
    headers.remove(WWW_AUTHENTICATE);
}

/// 来源页：上一个地址去掉账号、密码与 `#` 后面的部分；https 跳到 http 时不带。
pub(crate) fn make_referer(next: &Url, previous: &Url) -> Option<HeaderValue> {
    if next.scheme() == "http" && previous.scheme() == "https" {
        return None;
    }
    let mut referer = previous.clone();
    let _ = referer.set_username("");
    let _ = referer.set_password(None);
    referer.set_fragment(None);
    referer.as_str().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn the_default_follows_ten_redirects() {
        let policy = Policy::default();
        let next = url("http://x.y/z");
        let mut previous: Vec<Url> = (0..=9).map(|i| url(&format!("http://a.b/c/{i}"))).collect();
        assert!(matches!(
            policy.check(StatusCode::FOUND, &next, &previous),
            ActionKind::Follow
        ));
        previous.push(url("http://a.b.d/e/33"));
        match policy.check(StatusCode::FOUND, &next, &previous) {
            ActionKind::Error(error) => assert!(error.is::<TooManyRedirects>()),
            other => panic!("应报跳转太多：{other:?}"),
        }
    }

    #[test]
    fn none_stops_and_custom_decides() {
        let next = url("http://x.y/z");
        let previous = vec![url("http://a.b/c")];
        assert!(matches!(
            Policy::none().check(StatusCode::FOUND, &next, &previous),
            ActionKind::Stop
        ));
        assert!(matches!(
            Policy::limited(0).check(StatusCode::FOUND, &next, &previous),
            ActionKind::Error(_)
        ));
        let custom = Policy::custom(|attempt| {
            if attempt.url().host_str() == Some("x.y") {
                attempt.stop()
            } else {
                attempt.follow()
            }
        });
        assert!(matches!(
            custom.check(StatusCode::FOUND, &next, &previous),
            ActionKind::Stop
        ));
    }

    #[test]
    fn a_different_host_or_port_is_crossing() {
        let previous = vec![url("https://a.example/one")];
        assert!(!crosses_host(&url("https://a.example:443/two"), &previous));
        assert!(crosses_host(&url("https://a.example:8443/two"), &previous));
        assert!(crosses_host(&url("https://b.example/two"), &previous));
        assert!(!crosses_host(&url("https://b.example/two"), &[]));
    }

    #[test]
    fn only_credentials_are_removed() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer t"));
        headers.insert(COOKIE, HeaderValue::from_static("a=b"));
        headers.insert("x-other", HeaderValue::from_static("keep"));
        remove_credentials(&mut headers);
        assert!(!headers.contains_key(AUTHORIZATION) && !headers.contains_key(COOKIE));
        assert!(headers.contains_key("x-other"));
    }

    #[test]
    fn referer_drops_credentials_and_fragment_and_never_downgrades() {
        let previous = url("https://user:pw@a.example/p?q=1#frag");
        assert_eq!(
            make_referer(&url("https://b.example/"), &previous).unwrap(),
            "https://a.example/p?q=1"
        );
        assert!(make_referer(&url("http://b.example/"), &previous).is_none());
    }
}
