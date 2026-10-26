//! 客户端（对应 `reqwest::Client` / `reqwest::ClientBuilder`）：缺省值与 reqwest 0.12 相同——
//! 请求头带 `Accept: */*`，开 gzip，跳转至多 10 次并带来源页，连接复用、闲置 90 秒作废，关掉 Nagle。
//!
//! 一次请求：补默认请求头 → 取池里的连接或新连（查地址 → TCP → TLS → HTTP/1.1 握手，连接超时管这一整段）→
//! 发出、等响应头 → 按跳转策略跟随。复用的连接刚好被服务器关掉、请求还没发出去时，换一条新连接重发一次。
//! 跟随跳转的请求头、方法与请求体处理照搬 reqwest（tower-http 的跟随规则）；唯一更严的一点：一旦跳到过别的主机，
//! 认证类请求头之后一直不带（reqwest 在“跳到别处再在别处内部跳”时会重新带上最初的认证头）。

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::header::{
    ACCEPT, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, Entry, HOST, LOCATION,
    REFERER, TRANSFER_ENCODING, USER_AGENT,
};
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;
use url::Url;

use super::body::Body;
use super::error::{self, BoxError, ClientError, ConnectStep, Result, TimedOut};
use super::into_url::IntoUrl;
use super::pool::{Key, Pool, PoolReturn, Sender};
use super::redirect::{self, ActionKind};
use super::request::{Request, RequestBuilder};
use super::response::Response;

/// 连接池里空闲连接的缺省闲置时限（与 reqwest 相同）。
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// HTTP 客户端。克隆很便宜（内部是 `Arc`），克隆出的共用连接池。
#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientRef>,
}

struct ClientRef {
    headers: HeaderMap,
    connect_timeout: Option<Duration>,
    timeout: Option<Duration>,
    read_timeout: Option<Duration>,
    redirect: redirect::Policy,
    referer: bool,
    https_only: bool,
    gzip: bool,
    nodelay: bool,
    pool: Pool,
    tls: tokio_rustls::TlsConnector,
}

/// 客户端设置。
#[must_use = "ClientBuilder 要 build() 才得到客户端"]
pub struct ClientBuilder {
    headers: HeaderMap,
    connect_timeout: Option<Duration>,
    timeout: Option<Duration>,
    read_timeout: Option<Duration>,
    redirect: redirect::Policy,
    referer: bool,
    https_only: bool,
    gzip: bool,
    nodelay: bool,
    pool_idle_timeout: Option<Duration>,
    pool_max_idle_per_host: usize,
    tls: Tls,
    error: Option<error::Error>,
}

enum Tls {
    Default,
    Rustls(Box<rustls::ClientConfig>),
    Unknown,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientBuilder {
    /// 缺省设置（与 `reqwest::ClientBuilder::new()` 相同）。
    pub fn new() -> Self {
        let mut headers = HeaderMap::with_capacity(2);
        headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
        Self {
            headers,
            connect_timeout: None,
            timeout: None,
            read_timeout: None,
            redirect: redirect::Policy::default(),
            referer: true,
            https_only: false,
            gzip: true,
            nodelay: true,
            pool_idle_timeout: Some(POOL_IDLE_TIMEOUT),
            pool_max_idle_per_host: usize::MAX,
            tls: Tls::Default,
            error: None,
        }
    }

    /// 建客户端。
    ///
    /// # Errors
    ///
    /// 设置里有不合法的值（如 `user_agent` 写法不对、`use_preconfigured_tls` 传了别的东西）时返回 builder 错误。
    pub fn build(self) -> Result<Client> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let tls = match self.tls {
            Tls::Default => crate::tls_config(),
            Tls::Rustls(config) => Arc::new(*config),
            Tls::Unknown => {
                return Err(error::builder(
                    "Unknown TLS backend passed to `use_preconfigured_tls`",
                ));
            }
        };
        Ok(Client {
            inner: Arc::new(ClientRef {
                headers: self.headers,
                connect_timeout: self.connect_timeout,
                timeout: self.timeout,
                read_timeout: self.read_timeout,
                redirect: self.redirect,
                referer: self.referer,
                https_only: self.https_only,
                gzip: self.gzip,
                nodelay: self.nodelay,
                pool: Pool::new(self.pool_idle_timeout, self.pool_max_idle_per_host),
                tls: tokio_rustls::TlsConnector::from(tls),
            }),
        })
    }

    /// 每个请求都带的 `User-Agent`。
    pub fn user_agent<V>(mut self, value: V) -> Self
    where
        V: TryInto<HeaderValue>,
        V::Error: Into<http::Error>,
    {
        match value.try_into() {
            Ok(value) => {
                self.headers.insert(USER_AGENT, value);
            }
            Err(e) => self.error = Some(error::builder(e.into())),
        }
        self
    }

    /// 每个请求都带的请求头（请求自己设了同名的就用请求的）。
    pub fn default_headers(mut self, headers: HeaderMap) -> Self {
        for (key, value) in &headers {
            self.headers.insert(key, value.clone());
        }
        self
    }

    /// 整体超时：从发出到读完响应体。缺省不限。
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// 读取超时：等响应头、以及响应体每两块之间最多等多久。缺省不限。
    pub fn read_timeout(mut self, timeout: Duration) -> Self {
        self.read_timeout = Some(timeout);
        self
    }

    /// 连接超时：查地址、TCP、TLS 握手一共最多等多久。缺省不限。
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    /// 跳转策略（缺省至多 10 次）。
    pub fn redirect(mut self, policy: redirect::Policy) -> Self {
        self.redirect = policy;
        self
    }

    /// 跟随跳转时带不带来源页（缺省带）。
    pub fn referer(mut self, enable: bool) -> Self {
        self.referer = enable;
        self
    }

    /// 只允许 https（缺省不限）。
    pub fn https_only(mut self, enabled: bool) -> Self {
        self.https_only = enabled;
        self
    }

    /// gzip 自动解压（缺省开）。
    pub fn gzip(mut self, enable: bool) -> Self {
        self.gzip = enable;
        self
    }

    /// 关不关 Nagle（缺省关，小包立即发出）。
    pub fn tcp_nodelay(mut self, enabled: bool) -> Self {
        self.nodelay = enabled;
        self
    }

    /// 空闲连接的闲置时限（缺省 90 秒；`None` 不限）。
    pub fn pool_idle_timeout<D>(mut self, value: D) -> Self
    where
        D: Into<Option<Duration>>,
    {
        self.pool_idle_timeout = value.into();
        self
    }

    /// 每处（协议 + 主机 + 端口）至多留几条空闲连接（缺省不限；0 = 不复用）。
    pub fn pool_max_idle_per_host(mut self, max: usize) -> Self {
        self.pool_max_idle_per_host = max;
        self
    }

    /// 用给定的 rustls 设置握 TLS（传别的类型时，`build()` 报错，与 reqwest 一致）。
    pub fn use_preconfigured_tls(mut self, tls: impl std::any::Any) -> Self {
        let mut tls = Some(tls);
        match (&mut tls as &mut dyn std::any::Any).downcast_mut::<Option<rustls::ClientConfig>>() {
            Some(config) => match config.take() {
                Some(config) => self.tls = Tls::Rustls(Box::new(config)),
                None => self.tls = Tls::Unknown,
            },
            None => self.tls = Tls::Unknown,
        }
        self
    }

    /// 用 rustls（iPhone 版本来就只有 rustls，不起作用）。
    pub fn use_rustls_tls(self) -> Self {
        self
    }

    /// TCP 保活。iPhone 版照收、不起作用（WASI 的 TCP 接口没有这一项）。
    pub fn tcp_keepalive<D>(self, _value: D) -> Self
    where
        D: Into<Option<Duration>>,
    {
        self
    }

    /// 不走系统代理。iPhone 版本来就不走代理，不起作用。
    pub fn no_proxy(self) -> Self {
        self
    }

    /// 只用 HTTP/1。iPhone 版本来就只有 HTTP/1.1，不起作用。
    pub fn http1_only(self) -> Self {
        self
    }
}

impl fmt::Debug for ClientBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientBuilder")
            .field("headers", &self.headers)
            .field("timeout", &self.timeout)
            .field("connect_timeout", &self.connect_timeout)
            .field("redirect", &self.redirect)
            .finish()
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("headers", &self.inner.headers)
            .field("timeout", &self.inner.timeout)
            .field("redirect", &self.inner.redirect)
            .finish()
    }
}

impl Client {
    /// 缺省设置的客户端。
    ///
    /// # Panics
    ///
    /// 与 reqwest 一样，缺省设置建不起来时 panic（只在依赖本身坏掉时发生）。
    pub fn new() -> Self {
        ClientBuilder::new().build().expect("Client::new()")
    }

    /// 从缺省设置开始调。
    pub fn builder() -> ClientBuilder {
        ClientBuilder::new()
    }

    pub fn get<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::GET, url)
    }

    pub fn post<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::POST, url)
    }

    pub fn put<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::PUT, url)
    }

    pub fn patch<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::PATCH, url)
    }

    pub fn delete<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::DELETE, url)
    }

    pub fn head<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::HEAD, url)
    }

    pub fn request<U: IntoUrl>(&self, method: Method, url: U) -> RequestBuilder {
        let request = url.into_url().map(move |url| Request::new(method, url));
        RequestBuilder::new(self.clone(), request)
    }

    /// 发一个已经拼好的请求。
    ///
    /// # Errors
    ///
    /// 地址协议不对、连不上、超时、跳转出错时返回错误。
    pub async fn execute(&self, request: Request) -> Result<Response> {
        let (method, url, mut headers, body, timeout) = request.into_parts();
        if url.scheme() != "http" && url.scheme() != "https" {
            return Err(error::url_bad_scheme(url));
        }
        if self.inner.https_only && url.scheme() != "https" {
            return Err(error::url_bad_scheme(url));
        }
        for (key, value) in &self.inner.headers {
            if let Entry::Vacant(entry) = headers.entry(key) {
                entry.insert(value.clone());
            }
        }
        if self.inner.gzip
            && let Entry::Vacant(entry) = headers.entry(ACCEPT_ENCODING)
        {
            entry.insert(HeaderValue::from_static("gzip"));
        }
        let deadline = timeout
            .or(self.inner.timeout)
            .map(|limit| Instant::now() + limit);
        // 等响应头：整体超时与读取超时谁先到算谁。
        let header_deadline = match (deadline, self.inner.read_timeout) {
            (Some(deadline), Some(read)) => Some(deadline.min(Instant::now() + read)),
            (Some(deadline), None) => Some(deadline),
            (None, Some(read)) => Some(Instant::now() + read),
            (None, None) => None,
        };
        let exchange = self.follow(method, url.clone(), headers, body);
        let (hop, final_url) = match header_deadline {
            Some(at) => match tokio::time::timeout_at(at, exchange).await {
                Ok(result) => result?,
                Err(_) => {
                    tracing::debug!("请求超时（等响应头）");
                    return Err(error::request(TimedOut).with_url(url));
                }
            },
            None => exchange.await?,
        };
        let Hop {
            response,
            key,
            sender,
        } = hop;
        Ok(Response::new(
            response,
            final_url,
            self.inner.gzip,
            deadline,
            self.inner.read_timeout,
            PoolReturn::new(self.inner.pool.clone(), key, sender),
        ))
    }

    /// 发出请求并按策略跟随跳转；返回最后一跳与它的地址。跳转回应的体丢掉（hyper 读掉丢弃），
    /// 下一跳还去同一处时直接接着用这条连接（与 reqwest 一样不多开连接）。
    async fn follow(
        &self,
        mut method: Method,
        mut url: Url,
        mut headers: HeaderMap,
        mut body: Option<Body>,
    ) -> Result<(Hop, Url)> {
        let mut previous: Vec<Url> = Vec::new();
        let mut carried: Option<(Key, Sender)> = None;
        let mut crossed_hosts = false;
        // 每一跳在 `headers` 的副本上去掉认证头、换来源页，`headers` 本身只随方法改变去掉请求体相关的头。
        let mut hop_headers = headers.clone();
        loop {
            let hop = self
                .send(&method, &url, &hop_headers, body.as_ref(), carried.take())
                .await
                .map_err(|e| error::request(e).with_url(url.clone()))?;
            let status = hop.response.status();
            if self.inner.redirect.is_none() {
                return Ok((hop, url));
            }
            match status {
                StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND => {
                    if method == Method::POST {
                        method = Method::GET;
                        body = None;
                        drop_payload_headers(&mut headers);
                    }
                }
                StatusCode::SEE_OTHER => {
                    if method != Method::HEAD {
                        method = Method::GET;
                    }
                    body = None;
                    drop_payload_headers(&mut headers);
                }
                StatusCode::TEMPORARY_REDIRECT | StatusCode::PERMANENT_REDIRECT => {}
                _ => return Ok((hop, url)),
            }
            let next = hop
                .response
                .headers()
                .get(LOCATION)
                .and_then(|location| std::str::from_utf8(location.as_bytes()).ok())
                .and_then(|location| url.join(location).ok());
            let Some(next) = next else {
                return Ok((hop, url));
            };
            previous.push(url.clone());
            match self.inner.redirect.check(status, &next, &previous) {
                ActionKind::Follow => {
                    if next.scheme() != "http" && next.scheme() != "https" {
                        return Err(error::url_bad_scheme(next));
                    }
                    if self.inner.https_only && next.scheme() != "https" {
                        return Err(error::redirect(error::url_bad_scheme(next.clone()), next));
                    }
                    hop_headers = headers.clone();
                    crossed_hosts |= redirect::crosses_host(&next, &previous);
                    if crossed_hosts {
                        redirect::remove_credentials(&mut hop_headers);
                    }
                    if self.inner.referer {
                        let referer = previous
                            .last()
                            .and_then(|last| redirect::make_referer(&next, last));
                        if let Some(referer) = referer {
                            hop_headers.insert(REFERER, referer);
                        }
                    }
                    let Hop {
                        response,
                        key,
                        sender,
                    } = hop;
                    drop(response);
                    carried = Some((key, sender));
                    tracing::debug!(status = status.as_u16(), "跟随跳转");
                    url = next;
                }
                ActionKind::Stop => return Ok((hop, url)),
                ActionKind::Error(e) => {
                    let last = previous.pop().unwrap_or_else(|| url.clone());
                    return Err(error::redirect(e, last));
                }
            }
        }
    }

    /// 在一条连接上发一跳：上一跳带来的连接去同一处就接着用，否则池里有就用、没有就新连；
    /// 复用的连接没发出去就断了时，换新连接重发一次。
    async fn send(
        &self,
        method: &Method,
        url: &Url,
        headers: &HeaderMap,
        body: Option<&Body>,
        carried: Option<(Key, Sender)>,
    ) -> std::result::Result<Hop, BoxError> {
        let host = url.host_str().unwrap_or_default().to_owned();
        let key = Key {
            https: url.scheme() == "https",
            host: host.clone(),
            port: url.port_or_known_default().unwrap_or(80),
        };
        let mut request = build_request(method, url, headers, body)?;
        let (mut sender, reused) = match carried {
            Some((carried_key, mut sender)) if carried_key == key => match sender.ready().await {
                Ok(()) => (sender, true),
                Err(_) => self.checkout(&key).await?,
            },
            Some((carried_key, sender)) => {
                self.inner.pool.put_when_idle(carried_key, sender);
                self.checkout(&key).await?
            }
            None => self.checkout(&key).await?,
        };
        let response = match sender.try_send_request(request).await {
            Ok(response) => response,
            Err(mut failure) => match failure.take_message() {
                Some(unsent) if reused => {
                    tracing::debug!(error = %failure.error(), "复用的连接已断，换新连接重发");
                    request = unsent;
                    sender = self.connect(&key).await?;
                    sender
                        .send_request(request)
                        .await
                        .map_err(ClientError::send_request)?
                }
                _ => return Err(Box::new(ClientError::send_request(failure.into_error()))),
            },
        };
        Ok(Hop {
            response,
            key,
            sender,
        })
    }

    /// 取池里还能用的连接，没有就新连（见 [`Pool::checkout`]）。返回连接与它是否复用来的。
    async fn checkout(&self, key: &Key) -> std::result::Result<(Sender, bool), BoxError> {
        Ok(self.inner.pool.checkout(key, self.connect(key)).await?)
    }

    /// 新连一条：查地址 → TCP → （https）TLS → HTTP/1.1 握手；连接超时管这一整段。
    async fn connect(&self, key: &Key) -> std::result::Result<Sender, ClientError> {
        let open = async {
            let addresses = crate::tcp::resolve(&key.host, key.port)
                .await
                .map_err(|e| ClientError::connect(ConnectStep::new("dns error", e)))?;
            let tcp = crate::tcp::connect_any(&addresses)
                .await
                .map_err(|e| ClientError::connect(ConnectStep::new("tcp connect error", e)))?;
            if !self.inner.nodelay {
                let _ = tcp.set_nodelay(false);
            }
            let io: Box<dyn Io> = if key.https {
                let name = key
                    .host
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_owned();
                let name =
                    rustls::pki_types::ServerName::try_from(name).map_err(ClientError::connect)?;
                Box::new(
                    self.inner
                        .tls
                        .connect(name, tcp)
                        .await
                        .map_err(ClientError::connect)?,
                )
            } else {
                Box::new(tcp)
            };
            let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(io))
                .await
                .map_err(ClientError::connect)?;
            tokio::spawn(async move {
                if let Err(error) = connection.await {
                    tracing::debug!(%error, "HTTP 连接断开");
                }
            });
            Ok(sender)
        };
        let result = match self.inner.connect_timeout {
            Some(limit) => match tokio::time::timeout(limit, open).await {
                Ok(result) => result,
                Err(_) => Err(ClientError::connect(TimedOut)),
            },
            None => open.await,
        };
        if let Err(error) = &result {
            tracing::debug!(%error, host = %key.host, port = key.port, "连接失败");
        }
        result
    }
}

/// 一跳的结果：响应与它用的连接。
struct Hop {
    response: http::Response<Incoming>,
    key: Key,
    sender: Sender,
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// 拼一跳的 HTTP/1.1 请求：目标只写路径与查询，`Host` 没设时按地址补在最后（与 reqwest 一致）。
fn build_request(
    method: &Method,
    url: &Url,
    headers: &HeaderMap,
    body: Option<&Body>,
) -> std::result::Result<http::Request<Full<Bytes>>, BoxError> {
    let target = &url[url::Position::BeforePath..url::Position::AfterQuery];
    let body = body.map(Body::bytes).unwrap_or_default();
    let mut request = http::Request::builder()
        .method(method.clone())
        .uri(target)
        .body(Full::new(body))?;
    *request.headers_mut() = headers.clone();
    if let Entry::Vacant(entry) = request.headers_mut().entry(HOST) {
        let host = &url[url::Position::BeforeHost..url::Position::AfterPort];
        entry.insert(HeaderValue::from_str(host)?);
    }
    Ok(request)
}

fn drop_payload_headers(headers: &mut HeaderMap) {
    for name in [
        CONTENT_TYPE,
        CONTENT_LENGTH,
        CONTENT_ENCODING,
        TRANSFER_ENCODING,
    ] {
        headers.remove(name);
    }
}
