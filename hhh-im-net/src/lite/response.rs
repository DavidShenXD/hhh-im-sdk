//! 响应（对应 `reqwest::Response`）：状态、响应头、最终地址；响应体按块读或一次读完。
//!
//! 与 reqwest 一致：
//! - 客户端开着 gzip（缺省开）且响应声明 `Content-Encoding: gzip` 时边收边解，去掉 `Content-Encoding` 与
//!   `Content-Length`（`content_length()` 于是为 `None`）；声明了 gzip 却是空体，当作空体；只解第一段 gzip，
//!   后面再有的数据丢掉。
//! - 请求的整体超时管到读完响应体；读取超时管每两块之间的间隔。
//! - 读响应体的错都是 decode 错误；设了超时的请求外面多套一层 body 错误。

use std::fmt;
use std::io::Write as _;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use http::header::{CONTENT_ENCODING, CONTENT_LENGTH};
use http::{HeaderMap, StatusCode, Version};
use http_body_util::BodyExt as _;
use hyper::body::{Body as _, Incoming};
use serde::de::DeserializeOwned;
use tokio::time::Instant;
use url::Url;

use super::error::{self, BoxError, Result, TimedOut};
use super::pool::PoolReturn;

/// 一个响应。响应体还没读；不读就丢掉时，这条连接不再复用。
pub struct Response {
    status: StatusCode,
    version: Version,
    headers: HeaderMap,
    extensions: http::Extensions,
    url: Box<Url>,
    body: ResponseBody,
}

struct ResponseBody {
    incoming: Incoming,
    gzip: Option<GzipState>,
    deadline: Option<Instant>,
    read_timeout: Option<Duration>,
    finished: bool,
    /// 这条连接：响应体读完时放回连接池。
    connection: PoolReturn,
}

/// 边收边解的 gzip。
struct GzipState {
    decoder: flate2::write::GzDecoder<Vec<u8>>,
    /// 收到过数据没有（空体不算坏数据）。
    fed_any: bool,
    /// 第一段已经解完（后面的数据丢掉）。
    done: bool,
}

impl GzipState {
    /// 喂一块压缩数据；解出来的留在 `decoder` 里的 Vec。
    fn feed(&mut self, mut data: &[u8]) -> std::io::Result<()> {
        self.fed_any |= !data.is_empty();
        while !data.is_empty() && !self.done {
            let used = self.decoder.write(data)?;
            if used == 0 {
                self.done = true;
            }
            data = &data[used..];
        }
        self.decoder.flush()
    }
}

impl Response {
    pub(crate) fn new(
        response: http::Response<Incoming>,
        url: Url,
        accept_gzip: bool,
        deadline: Option<Instant>,
        read_timeout: Option<Duration>,
        connection: PoolReturn,
    ) -> Self {
        let (mut parts, incoming) = response.into_parts();
        let gzip = accept_gzip
            && parts
                .headers
                .get(CONTENT_ENCODING)
                .is_some_and(|value| value.as_bytes() == b"gzip");
        if gzip {
            parts.headers.remove(CONTENT_ENCODING);
            parts.headers.remove(CONTENT_LENGTH);
        }
        let mut connection = connection;
        // 本来就没有体（HEAD、204、304、长度为 0）：不等调用方读，立刻放回。
        if incoming.is_end_stream() {
            connection.release();
        }
        Self {
            status: parts.status,
            version: parts.version,
            headers: parts.headers,
            extensions: parts.extensions,
            url: Box::new(url),
            body: ResponseBody {
                incoming,
                gzip: gzip.then(|| GzipState {
                    decoder: flate2::write::GzDecoder::new(Vec::new()),
                    fed_any: false,
                    done: false,
                }),
                deadline,
                read_timeout,
                finished: false,
                connection,
            },
        }
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn version(&self) -> Version {
        self.version
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    pub fn extensions(&self) -> &http::Extensions {
        &self.extensions
    }

    pub fn extensions_mut(&mut self) -> &mut http::Extensions {
        &mut self.extensions
    }

    /// 响应体还剩多少字节（服务器给了长度、且没在解压时）。
    pub fn content_length(&self) -> Option<u64> {
        if self.body.gzip.is_some() {
            return None;
        }
        self.body.incoming.size_hint().exact()
    }

    /// 最终地址（跟随跳转后的）。
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// 读下一块响应体；读完返回 `None`。
    ///
    /// # Errors
    ///
    /// 连接断了、超时、gzip 坏了时返回 decode 错误。
    pub async fn chunk(&mut self) -> Result<Option<Bytes>> {
        self.body.chunk().await.map_err(error::decode)
    }

    /// 一次读完响应体。
    ///
    /// # Errors
    ///
    /// 同 [`chunk`](Self::chunk)。
    pub async fn bytes(mut self) -> Result<Bytes> {
        let mut first = None;
        let mut rest = BytesMut::new();
        while let Some(chunk) = self.chunk().await? {
            match &first {
                None => first = Some(chunk),
                Some(_) => rest.extend_from_slice(&chunk),
            }
        }
        Ok(match first {
            None => Bytes::new(),
            Some(first) if rest.is_empty() => first,
            Some(first) => {
                let mut all = BytesMut::with_capacity(first.len() + rest.len());
                all.extend_from_slice(&first);
                all.extend_from_slice(&rest);
                all.freeze()
            }
        })
    }

    /// 读完响应体当 UTF-8 文字（不合法的字节换成 U+FFFD）。
    ///
    /// # Errors
    ///
    /// 同 [`chunk`](Self::chunk)。
    pub async fn text(self) -> Result<String> {
        let full = self.bytes().await?;
        Ok(String::from_utf8_lossy(&full).into_owned())
    }

    /// 读完响应体解析成 JSON。
    ///
    /// # Errors
    ///
    /// 同 [`chunk`](Self::chunk)；JSON 解析失败也是 decode 错误。
    pub async fn json<T: DeserializeOwned>(self) -> Result<T> {
        let full = self.bytes().await?;
        serde_json::from_slice(&full).map_err(error::decode)
    }

    /// 4xx / 5xx 时变成错误。
    ///
    /// # Errors
    ///
    /// 状态码是 4xx / 5xx 时返回 status 错误（带地址）。
    pub fn error_for_status(self) -> Result<Self> {
        match self.status_error() {
            Some(error) => Err(error),
            None => Ok(self),
        }
    }

    /// 同 [`error_for_status`](Self::error_for_status)，不拿走响应。
    ///
    /// # Errors
    ///
    /// 同 [`error_for_status`](Self::error_for_status)。
    pub fn error_for_status_ref(&self) -> Result<&Self> {
        match self.status_error() {
            Some(error) => Err(error),
            None => Ok(self),
        }
    }

    fn status_error(&self) -> Option<error::Error> {
        let status = self.status;
        if status.is_client_error() || status.is_server_error() {
            let reason = self.extensions.get::<hyper::ext::ReasonPhrase>().cloned();
            Some(error::status_code((*self.url).clone(), status, reason))
        } else {
            None
        }
    }
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response")
            .field("url", &self.url.as_str())
            .field("status", &self.status)
            .field("headers", &self.headers)
            .finish()
    }
}

impl ResponseBody {
    /// 设了任何超时的请求，读体的错外面多套一层 body 错误（与 reqwest 的超时包装一致）。
    fn wrap(&self, error: BoxError) -> BoxError {
        if self.deadline.is_some() || self.read_timeout.is_some() {
            Box::new(error::body(error))
        } else {
            error
        }
    }

    async fn chunk(&mut self) -> std::result::Result<Option<Bytes>, BoxError> {
        loop {
            if self.finished {
                return Ok(None);
            }
            match self.raw_frame().await? {
                Some(data) => match &mut self.gzip {
                    None => return Ok(Some(data)),
                    Some(gzip) => {
                        if let Err(e) = gzip.feed(&data) {
                            return Err(self.wrap(Box::new(e)));
                        }
                        let decoded = std::mem::take(gzip.decoder.get_mut());
                        if !decoded.is_empty() {
                            return Ok(Some(decoded.into()));
                        }
                    }
                },
                None => {
                    self.finished = true;
                    let Some(gzip) = &mut self.gzip else {
                        return Ok(None);
                    };
                    // 声明了 gzip 却一个字节都没有：当作空体（与 reqwest 一致）。
                    if !gzip.fed_any {
                        return Ok(None);
                    }
                    if let Err(e) = gzip.decoder.try_finish() {
                        return Err(self.wrap(Box::new(e)));
                    }
                    let decoded = std::mem::take(gzip.decoder.get_mut());
                    return Ok((!decoded.is_empty()).then(|| decoded.into()));
                }
            }
        }
    }

    /// 读下一块原始数据（跳过 trailer），套上整体与读取超时。
    async fn raw_frame(&mut self) -> std::result::Result<Option<Bytes>, BoxError> {
        loop {
            let limit = match (self.deadline, self.read_timeout) {
                (Some(deadline), Some(read)) => Some(deadline.min(Instant::now() + read)),
                (Some(deadline), None) => Some(deadline),
                (None, Some(read)) => Some(Instant::now() + read),
                (None, None) => None,
            };
            let frame = match limit {
                Some(limit) => match tokio::time::timeout_at(limit, self.incoming.frame()).await {
                    Ok(frame) => frame,
                    Err(_) => return Err(Box::new(error::body(TimedOut))),
                },
                None => self.incoming.frame().await,
            };
            match frame {
                None => {
                    self.connection.release();
                    return Ok(None);
                }
                Some(Err(e)) => return Err(self.wrap(Box::new(e))),
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        return Ok(Some(data));
                    }
                }
            }
        }
    }
}
