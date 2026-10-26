//! 与 reqwest 0.12 同形的错误：种类、显示文字、`source` 链与 `is_*` 判断都一致，两个平台的排障日志读起来一样。
//!
//! 原因链也照 reqwest 的样子包：连接阶段的失败外面套一层 `client error (Connect)`（[`is_connect`](Error::is_connect)
//! 认它），再往里是 `tcp connect error` / `dns error` 与系统错误；设了超时的请求，读响应体出的错外面多套一层
//! `request or response body error`。

use std::error::Error as StdError;
use std::fmt;
use std::io;

use http::StatusCode;
use url::Url;

/// 本包请求的结果。
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) type BoxError = Box<dyn StdError + Send + Sync>;

/// 发请求、读响应时的错误（对应 `reqwest::Error`）。
pub struct Error {
    inner: Box<Inner>,
}

struct Inner {
    kind: Kind,
    source: Option<BoxError>,
    url: Option<Url>,
}

#[derive(Debug, Clone)]
pub(crate) enum Kind {
    Builder,
    Request,
    Redirect,
    Status(StatusCode, Option<hyper::ext::ReasonPhrase>),
    Body,
    Decode,
}

impl Error {
    pub(crate) fn new<E: Into<BoxError>>(kind: Kind, source: Option<E>) -> Self {
        Self {
            inner: Box::new(Inner {
                kind,
                source: source.map(Into::into),
                url: None,
            }),
        }
    }

    /// 出错的请求地址（有的话）。
    pub fn url(&self) -> Option<&Url> {
        self.inner.url.as_ref()
    }

    /// 出错的请求地址，可改（例如去掉查询参数再打日志）。
    pub fn url_mut(&mut self) -> Option<&mut Url> {
        self.inner.url.as_mut()
    }

    /// 换上请求地址。
    pub fn with_url(mut self, url: Url) -> Self {
        self.inner.url = Some(url);
        self
    }

    /// 去掉请求地址：地址里可能带会话编号、分页游标、签名参数，打日志前去掉。
    pub fn without_url(mut self) -> Self {
        self.inner.url = None;
        self
    }

    /// 请求没建成（地址、请求头不合法等）。
    pub fn is_builder(&self) -> bool {
        matches!(self.inner.kind, Kind::Builder)
    }

    /// 跟随跳转时出错（跳太多次等）。
    pub fn is_redirect(&self) -> bool {
        matches!(self.inner.kind, Kind::Redirect)
    }

    /// 来自 `error_for_status`：服务器回了 4xx / 5xx。
    pub fn is_status(&self) -> bool {
        matches!(self.inner.kind, Kind::Status(..))
    }

    /// 超时（整体、连接或读取）。
    pub fn is_timeout(&self) -> bool {
        let mut source = self.source();
        while let Some(error) = source {
            if error.is::<TimedOut>() {
                return true;
            }
            if let Some(hyper) = error.downcast_ref::<hyper::Error>()
                && hyper.is_timeout()
            {
                return true;
            }
            if let Some(io) = error.downcast_ref::<io::Error>()
                && io.kind() == io::ErrorKind::TimedOut
            {
                return true;
            }
            source = error.source();
        }
        false
    }

    /// 发请求时出错（连不上、连接断了、超时等）。
    pub fn is_request(&self) -> bool {
        matches!(self.inner.kind, Kind::Request)
    }

    /// 连接阶段出错（查地址、TCP、TLS 握手）。
    pub fn is_connect(&self) -> bool {
        let mut source = self.source();
        while let Some(error) = source {
            if let Some(client) = error.downcast_ref::<ClientError>()
                && client.kind == ClientErrorKind::Connect
            {
                return true;
            }
            source = error.source();
        }
        false
    }

    /// 读请求体或响应体时出错。
    pub fn is_body(&self) -> bool {
        matches!(self.inner.kind, Kind::Body)
    }

    /// 读响应体或解析响应（gzip、JSON）时出错。
    pub fn is_decode(&self) -> bool {
        matches!(self.inner.kind, Kind::Decode)
    }

    /// 连接升级出错；iPhone 版不支持升级，总是 `false`。
    pub fn is_upgrade(&self) -> bool {
        false
    }

    /// `error_for_status` 得到的状态码。
    pub fn status(&self) -> Option<StatusCode> {
        match self.inner.kind {
            Kind::Status(code, _) => Some(code),
            _ => None,
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut builder = f.debug_struct("hhh_im_net::Error");
        builder.field("kind", &self.inner.kind);
        if let Some(url) = &self.inner.url {
            builder.field("url", &url.as_str());
        }
        if let Some(source) = &self.inner.source {
            builder.field("source", source);
        }
        builder.finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.inner.kind {
            Kind::Builder => f.write_str("builder error")?,
            Kind::Request => f.write_str("error sending request")?,
            Kind::Body => f.write_str("request or response body error")?,
            Kind::Decode => f.write_str("error decoding response body")?,
            Kind::Redirect => f.write_str("error following redirect")?,
            Kind::Status(code, reason) => {
                let prefix = if code.is_client_error() {
                    "HTTP status client error"
                } else {
                    "HTTP status server error"
                };
                match reason {
                    Some(reason) => {
                        write!(
                            f,
                            "{prefix} ({} {})",
                            code.as_str(),
                            Escape(reason.as_bytes())
                        )?;
                    }
                    None => write!(f, "{prefix} ({code})")?,
                }
            }
        }
        if let Some(url) = &self.inner.url {
            write!(f, " for url ({url})")?;
        }
        Ok(())
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.inner.source.as_ref().map(|error| &**error as _)
    }
}

pub(crate) fn builder<E: Into<BoxError>>(error: E) -> Error {
    Error::new(Kind::Builder, Some(error))
}

pub(crate) fn request<E: Into<BoxError>>(error: E) -> Error {
    Error::new(Kind::Request, Some(error))
}

pub(crate) fn body<E: Into<BoxError>>(error: E) -> Error {
    Error::new(Kind::Body, Some(error))
}

pub(crate) fn decode<E: Into<BoxError>>(error: E) -> Error {
    Error::new(Kind::Decode, Some(error))
}

pub(crate) fn redirect<E: Into<BoxError>>(error: E, url: Url) -> Error {
    Error::new(Kind::Redirect, Some(error)).with_url(url)
}

pub(crate) fn status_code(
    url: Url,
    status: StatusCode,
    reason: Option<hyper::ext::ReasonPhrase>,
) -> Error {
    Error::new(Kind::Status(status, reason), None::<Error>).with_url(url)
}

pub(crate) fn url_bad_scheme(url: Url) -> Error {
    Error::new(Kind::Builder, Some(BadScheme)).with_url(url)
}

/// 超时（显示文字与 reqwest 一致）。
#[derive(Debug)]
pub(crate) struct TimedOut;

impl fmt::Display for TimedOut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("operation timed out")
    }
}

impl StdError for TimedOut {}

/// 地址的协议不是 http / https（或要求只走 https）。
#[derive(Debug)]
pub(crate) struct BadScheme;

impl fmt::Display for BadScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("URL scheme is not allowed")
    }
}

impl StdError for BadScheme {}

/// 客户端层的错误：连接阶段（`Connect`）或发请求阶段（`SendRequest`），显示为 `client error (…)`。
#[derive(Debug)]
pub(crate) struct ClientError {
    kind: ClientErrorKind,
    source: BoxError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientErrorKind {
    Connect,
    SendRequest,
}

impl ClientError {
    pub(crate) fn connect<E: Into<BoxError>>(source: E) -> Self {
        Self {
            kind: ClientErrorKind::Connect,
            source: source.into(),
        }
    }

    pub(crate) fn send_request<E: Into<BoxError>>(source: E) -> Self {
        Self {
            kind: ClientErrorKind::SendRequest,
            source: source.into(),
        }
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "client error ({:?})", self.kind)
    }
}

impl StdError for ClientError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&*self.source)
    }
}

/// 连接里的某一步（`tcp connect error`、`dns error`）失败，原因是系统错误。
#[derive(Debug)]
pub(crate) struct ConnectStep {
    what: &'static str,
    source: io::Error,
}

impl ConnectStep {
    pub(crate) fn new(what: &'static str, source: io::Error) -> Self {
        Self { what, source }
    }
}

impl fmt::Display for ConnectStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.what)
    }
}

impl StdError for ConnectStep {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&self.source)
    }
}

/// 把服务器给的原因短语按字节转义显示（与 reqwest 一致：可打印 ASCII 原样，其余转义）。
struct Escape<'a>(&'a [u8]);

impl fmt::Display for Escape<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &c in self.0 {
            match c {
                b'\n' => f.write_str("\\n")?,
                b'\r' => f.write_str("\\r")?,
                b'\t' => f.write_str("\\t")?,
                b'\\' | b'"' => write!(f, "\\{}", c as char)?,
                b'\0' => f.write_str("\\0")?,
                0x20..0x7f => write!(f, "{}", c as char)?,
                _ => write!(f, "\\x{c:02x}")?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url() -> Url {
        Url::parse("https://example.com/a?b=c").unwrap()
    }

    #[test]
    fn display_matches_reqwest_wording() {
        assert_eq!(
            request(TimedOut).with_url(url()).to_string(),
            "error sending request for url (https://example.com/a?b=c)"
        );
        assert_eq!(
            decode(body(TimedOut)).to_string(),
            "error decoding response body"
        );
        assert_eq!(
            status_code(url(), StatusCode::NOT_FOUND, None).to_string(),
            "HTTP status client error (404 Not Found) for url (https://example.com/a?b=c)"
        );
        assert_eq!(
            status_code(url(), StatusCode::BAD_GATEWAY, None)
                .without_url()
                .to_string(),
            "HTTP status server error (502 Bad Gateway)"
        );
        let reason = hyper::ext::ReasonPhrase::try_from(&b"Odd \"Thing\""[..]).unwrap();
        assert_eq!(
            status_code(url(), StatusCode::from_u16(499).unwrap(), Some(reason))
                .without_url()
                .to_string(),
            "HTTP status client error (499 Odd \\\"Thing\\\")"
        );
        assert_eq!(
            url_bad_scheme(Url::parse("ftp://x/").unwrap()).to_string(),
            "builder error for url (ftp://x/)"
        );
    }

    #[test]
    fn timeouts_are_found_anywhere_in_the_chain() {
        assert!(request(TimedOut).is_timeout());
        assert!(decode(body(TimedOut)).is_timeout());
        assert!(request(ClientError::connect(TimedOut)).is_timeout());
        assert!(request(io::Error::from(io::ErrorKind::TimedOut)).is_timeout());
        assert!(!request(io::Error::from(io::ErrorKind::ConnectionReset)).is_timeout());
    }

    #[test]
    fn only_connect_stage_errors_are_connect_errors() {
        let refused = ClientError::connect(ConnectStep::new(
            "tcp connect error",
            io::Error::from(io::ErrorKind::ConnectionRefused),
        ));
        let error = request(refused);
        assert!(error.is_connect() && error.is_request());
        assert!(!request(ClientError::send_request(TimedOut)).is_connect());
        let mut chain = Vec::new();
        let mut source = error.source();
        while let Some(cause) = source {
            chain.push(cause.to_string());
            source = cause.source();
        }
        assert_eq!(chain[0], "client error (Connect)");
        assert_eq!(chain[1], "tcp connect error");
    }

    #[test]
    fn kinds_are_exclusive() {
        let error = builder(BadScheme);
        assert!(error.is_builder());
        assert!(!error.is_request() && !error.is_status() && !error.is_decode());
        assert_eq!(error.status(), None);
        let error = status_code(url(), StatusCode::FORBIDDEN, None);
        assert!(error.is_status());
        assert_eq!(error.status(), Some(StatusCode::FORBIDDEN));
    }
}
