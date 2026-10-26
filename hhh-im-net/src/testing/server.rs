//! 本机测试服务器与几种“坏服务器”（只在电脑上，feature `test-server`）；用法见上一层的说明。

use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::BodyExt as _;
use hyper::body::{Frame, Incoming, SizeHint};
use hyper_util::rt::{TokioIo, TokioTimer};
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::{GZIP_TEXT, Hello};

const SERVER_PEM: &[u8] = include_bytes!("certs/server.pem");
const SERVER_KEY: &[u8] = include_bytes!("certs/server.key");

fn server_tls() -> Arc<rustls::ServerConfig> {
    let chain = vec![CertificateDer::from_pem_slice(SERVER_PEM).expect("测试服务器证书是合法 PEM")];
    let key = PrivateKeyDer::from_pem_slice(SERVER_KEY).expect("测试服务器私钥是合法 PEM");
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(rustls::ALL_VERSIONS)
        .expect("aws-lc-rs 支持 TLS 1.2 与 1.3")
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .expect("测试证书与私钥配对");
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

/// 服务器收到的一个请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    /// 第几条连接（从 1 起）。
    pub connection: usize,
    pub method: String,
    /// 请求目标（路径与查询）。
    pub target: String,
    /// 请求头，按收到的顺序，名字小写。
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Seen {
    /// 某个请求头的全部值。
    pub fn header(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
            .collect()
    }
}

/// 本机测试服务器；丢掉时停止。
pub struct TestServer {
    addr: SocketAddr,
    https: bool,
    seen: Arc<Mutex<Vec<Seen>>>,
    connections: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TestServer {
    /// HTTP 测试服务器（空闲连接一直留着）。
    pub async fn http() -> Self {
        Self::start(false, None).await
    }

    /// HTTPS 测试服务器（测试证书）。
    pub async fn https() -> Self {
        Self::start(true, None).await
    }

    /// HTTP 测试服务器，空闲连接等 `idle` 后由服务器关掉（看客户端会不会拿已断的连接）。
    pub async fn http_closing_idle(idle: Duration) -> Self {
        Self::start(false, Some(idle)).await
    }

    async fn start(https: bool, idle: Option<Duration>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("本机能开端口");
        let addr = listener.local_addr().expect("刚开的端口有地址");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let acceptor = https.then(|| tokio_rustls::TlsAcceptor::from(server_tls()));
        let task = tokio::spawn({
            let seen = seen.clone();
            let connections = connections.clone();
            async move {
                loop {
                    let Ok((tcp, _)) = listener.accept().await else {
                        return;
                    };
                    let connection = connections.fetch_add(1, Ordering::SeqCst) + 1;
                    let seen = seen.clone();
                    let acceptor = acceptor.clone();
                    tokio::spawn(async move {
                        match acceptor {
                            Some(acceptor) => {
                                if let Ok(tls) = acceptor.accept(tcp).await {
                                    serve(TokioIo::new(tls), connection, seen, idle).await;
                                }
                            }
                            None => serve(TokioIo::new(tcp), connection, seen, idle).await,
                        }
                    });
                }
            }
        });
        Self {
            addr,
            https,
            seen,
            connections,
            task,
        }
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// `http(s)://127.0.0.1:端口{path}`。
    pub fn url(&self, path: &str) -> String {
        self.url_on("127.0.0.1", path)
    }

    /// 用别的主机名（如 `localhost`）指向同一个服务器：看“换主机”时的处理。
    pub fn url_on(&self, host: &str, path: &str) -> String {
        let scheme = if self.https { "https" } else { "http" };
        format!("{scheme}://{host}:{}{path}", self.addr.port())
    }

    /// 至今收到的请求。
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// 取走至今收到的请求（之后从空开始记）。
    pub fn take_seen(&self) -> Vec<Seen> {
        std::mem::take(&mut *self.seen.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// 至今接受过几条连接。
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

async fn serve<I>(io: I, connection: usize, seen: Arc<Mutex<Vec<Seen>>>, idle: Option<Duration>)
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let mut builder = hyper::server::conn::http1::Builder::new();
    if let Some(idle) = idle {
        builder.timer(TokioTimer::new()).header_read_timeout(idle);
    }
    let service = hyper::service::service_fn(move |request| {
        let seen = seen.clone();
        async move { Ok::<_, Infallible>(handle(request, connection, seen).await) }
    });
    let _ = builder.serve_connection(io, service).await;
}

async fn handle(
    request: http::Request<Incoming>,
    connection: usize,
    seen: Arc<Mutex<Vec<Seen>>>,
) -> http::Response<TestBody> {
    let (parts, body) = request.into_parts();
    let body = body
        .collect()
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();
    let record = Seen {
        connection,
        method: parts.method.to_string(),
        target: parts
            .uri
            .path_and_query()
            .map_or_else(|| "/".to_owned(), |p| p.as_str().to_owned()),
        headers: parts
            .headers
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_owned(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect(),
        body: body.to_vec(),
    };
    seen.lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(record.clone());
    route(&record).await
}

fn query_value(query: &str, name: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

fn number(query: &str, name: &str, default: u64) -> u64 {
    query_value(query, name)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn gzip(data: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).expect("写进内存");
    encoder.finish().expect("写进内存")
}

fn reply(status: u16, body: impl Into<Bytes>) -> http::Response<TestBody> {
    let mut response = http::Response::new(TestBody::full(body.into()));
    *response.status_mut() = http::StatusCode::from_u16(status).expect("测试用的状态码合法");
    response
}

fn with_header(
    mut response: http::Response<TestBody>,
    name: &'static str,
    value: &str,
) -> http::Response<TestBody> {
    response.headers_mut().append(
        name,
        http::HeaderValue::from_str(value).expect("测试用的请求头合法"),
    );
    response
}

async fn route(seen: &Seen) -> http::Response<TestBody> {
    let (path, query) = seen.target.split_once('?').unwrap_or((&seen.target, ""));
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match segments.as_slice() {
        ["json"] => {
            let body = serde_json::json!({"ok": true, "method": seen.method, "query": query});
            with_header(
                reply(200, body.to_string()),
                "content-type",
                "application/json",
            )
        }
        ["echo"] => {
            let kind = seen
                .header("content-type")
                .first()
                .map_or("application/octet-stream".to_owned(), |v| (*v).to_owned());
            with_header(reply(200, seen.body.clone()), "content-type", &kind)
        }
        ["status", code] => {
            let code: u16 = code.parse().unwrap_or(500);
            reply(code, format!("status {code}"))
        }
        ["reason"] => {
            let mut response = reply(499, "odd");
            response
                .extensions_mut()
                .insert(hyper::ext::ReasonPhrase::from_static(b"Odd Thing"));
            response
        }
        ["redirect", code] => {
            let code: u16 = code.parse().unwrap_or(302);
            let to = query_value(query, "to").unwrap_or_else(|| "/".to_owned());
            with_header(reply(code, "moved"), "location", &to)
        }
        ["chain", n] => {
            let n: u32 = n.parse().unwrap_or(0);
            if n == 0 {
                reply(200, "end")
            } else {
                with_header(
                    reply(302, "moved"),
                    "location",
                    &format!("/chain/{}", n - 1),
                )
            }
        }
        ["gzip"] => with_header(
            reply(200, gzip(GZIP_TEXT.as_bytes())),
            "content-encoding",
            "gzip",
        ),
        ["gzip-multi"] => {
            let mut body = gzip(b"first,");
            body.extend(gzip(b"second"));
            with_header(reply(200, body), "content-encoding", "gzip")
        }
        ["gzip-empty"] => with_header(reply(200, Bytes::new()), "content-encoding", "gzip"),
        ["gzip-bad"] => with_header(
            reply(200, &b"definitely not gzip"[..]),
            "content-encoding",
            "gzip",
        ),
        ["chunks"] => {
            let n = number(query, "n", 3);
            let size = number(query, "size", 4) as usize;
            let gap = Duration::from_millis(number(query, "gap_ms", 5));
            let (tx, rx) = mpsc::channel(4);
            tokio::spawn(async move {
                for index in 0..n {
                    let chunk = Bytes::from(vec![b'a' + (index % 26) as u8; size]);
                    if tx.send(chunk).await.is_err() {
                        return;
                    }
                    tokio::time::sleep(gap).await;
                }
            });
            http::Response::new(TestBody::Channel(rx))
        }
        ["slow-headers"] => {
            tokio::time::sleep(Duration::from_millis(number(query, "ms", 500))).await;
            reply(200, "slow")
        }
        ["slow-body"] => {
            let wait = Duration::from_millis(number(query, "ms", 500));
            let (tx, rx) = mpsc::channel(2);
            tokio::spawn(async move {
                let _ = tx.send(Bytes::from_static(b"start")).await;
                tokio::time::sleep(wait).await;
                let _ = tx.send(Bytes::from_static(b"end")).await;
            });
            http::Response::new(TestBody::Channel(rx))
        }
        ["bytes"] => {
            let size = number(query, "size", 1024) as usize;
            let body: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            reply(200, body)
        }
        ["close"] => with_header(reply(200, "bye"), "connection", "close"),
        ["empty"] => reply(204, Bytes::new()),
        _ => reply(200, "ok"),
    }
}

/// 测试服务器的回应体：整块（带长度）或按块陆续送出（分块传输）。
pub(crate) enum TestBody {
    Full(Option<Bytes>, u64),
    Channel(mpsc::Receiver<Bytes>),
}

impl TestBody {
    fn full(bytes: Bytes) -> Self {
        let len = bytes.len() as u64;
        Self::Full(Some(bytes), len)
    }
}

impl hyper::body::Body for TestBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        match self.get_mut() {
            Self::Full(bytes, _) => Poll::Ready(
                bytes
                    .take()
                    .filter(|b| !b.is_empty())
                    .map(|b| Ok(Frame::data(b))),
            ),
            Self::Channel(rx) => rx
                .poll_recv(cx)
                .map(|chunk| chunk.map(|b| Ok(Frame::data(b)))),
        }
    }

    fn is_end_stream(&self) -> bool {
        matches!(self, Self::Full(None, _) | Self::Full(_, 0))
    }

    fn size_hint(&self) -> SizeHint {
        match self {
            Self::Full(_, len) => SizeHint::with_exact(*len),
            Self::Channel(_) => SizeHint::default(),
        }
    }
}

/// 写一段固定回应的“坏服务器”：读到请求头结束就写 `reply`，然后关连接。
pub struct ScriptedServer {
    port: u16,
    task: JoinHandle<()>,
}

impl Drop for ScriptedServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ScriptedServer {
    /// 每条连接：读完请求头，写 `reply`，关掉。
    pub async fn start(reply: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("本机能开端口");
        let port = listener.local_addr().expect("刚开的端口有地址").port();
        let task = tokio::spawn(async move {
            while let Ok((mut tcp, _)) = listener.accept().await {
                tokio::spawn(async move {
                    read_request_head(&mut tcp).await;
                    let _ = tcp.write_all(reply).await;
                    let _ = tcp.shutdown().await;
                });
            }
        });
        Self { port, task }
    }

    /// 回应头说有 100 字节、只给 10 字节就关连接。
    pub async fn truncated_body() -> Self {
        Self::start(b"HTTP/1.1 200 OK\r\ncontent-length: 100\r\n\r\n0123456789").await
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

async fn read_request_head(tcp: &mut TcpStream) {
    let mut head = Vec::new();
    let mut buffer = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match tcp.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&buffer[..n]),
        }
    }
}

/// 接受连接后一声不吭的服务器：对 https 地址，TLS 握手一直等不到回应（看连接超时）。
pub struct StallingServer {
    port: u16,
    task: JoinHandle<()>,
}

impl Drop for StallingServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl StallingServer {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("本机能开端口");
        let port = listener.local_addr().expect("刚开的端口有地址").port();
        let task = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((tcp, _)) = listener.accept().await {
                held.push(tcp);
            }
        });
        Self { port, task }
    }

    pub fn url(&self, scheme: &str, path: &str) -> String {
        format!("{scheme}://127.0.0.1:{}{path}", self.port)
    }
}

/// 记下每条连接的 TLS 握手请求然后关掉连接的服务器（看客户端“像谁”）。
pub struct HelloProbe {
    port: u16,
    hellos: mpsc::UnboundedReceiver<Hello>,
    task: JoinHandle<()>,
}

impl Drop for HelloProbe {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl HelloProbe {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("本机能开端口");
        let port = listener.local_addr().expect("刚开的端口有地址").port();
        let (tx, hellos) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            while let Ok((mut tcp, _)) = listener.accept().await {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let mut header = [0u8; 5];
                    if tcp.read_exact(&mut header).await.is_err() || header[0] != 0x16 {
                        return;
                    }
                    let len = u16::from_be_bytes([header[3], header[4]]) as usize;
                    let mut record = vec![0u8; len];
                    if tcp.read_exact(&mut record).await.is_ok() {
                        let _ = tx.send(parse_hello(&record, len + 5));
                    }
                });
            }
        });
        Self { port, hellos, task }
    }

    pub fn url(&self, path: &str) -> String {
        format!("https://localhost:{}{path}", self.port)
    }

    /// 下一条连接的握手请求。
    pub async fn next(&mut self) -> Hello {
        tokio::time::timeout(Duration::from_secs(10), self.hellos.recv())
            .await
            .expect("10 秒内应收到握手请求")
            .expect("探针还在")
    }
}

/// 解析 ClientHello 记录体（握手消息），取 key_share 的组与 SNI。
fn parse_hello(record: &[u8], len: usize) -> Hello {
    let mut hello = Hello {
        len,
        key_shares: Vec::new(),
        server_name: None,
        cipher_suites: Vec::new(),
        extensions: Vec::new(),
        alpn: Vec::new(),
    };
    let read_u16 = |at: usize| -> Option<usize> {
        Some(u16::from_be_bytes([*record.get(at)?, *record.get(at + 1)?]) as usize)
    };
    // 握手类型 1 + 长度 3 + 版本 2 + 随机数 32。
    let mut at = 4 + 2 + 32;
    let Some(&session) = record.get(at) else {
        return hello;
    };
    at += 1 + session as usize;
    let Some(suites) = read_u16(at) else {
        return hello;
    };
    let mut suite = at + 2;
    while suite + 2 <= (at + 2 + suites).min(record.len()) {
        hello
            .cipher_suites
            .push(u16::from_be_bytes([record[suite], record[suite + 1]]));
        suite += 2;
    }
    at += 2 + suites;
    let Some(&compression) = record.get(at) else {
        return hello;
    };
    at += 1 + compression as usize;
    let Some(extensions) = read_u16(at) else {
        return hello;
    };
    at += 2;
    let end = (at + extensions).min(record.len());
    while at + 4 <= end {
        let (Some(kind), Some(size)) = (read_u16(at), read_u16(at + 2)) else {
            break;
        };
        let data = &record[(at + 4).min(end)..(at + 4 + size).min(end)];
        hello.extensions.push(kind as u16);
        match kind {
            0x0033 if data.len() >= 2 => {
                let mut pos = 2;
                while pos + 4 <= data.len() {
                    let group = u16::from_be_bytes([data[pos], data[pos + 1]]);
                    let key = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
                    hello.key_shares.push(group);
                    pos += 4 + key;
                }
            }
            0x0010 if data.len() >= 2 => {
                let mut pos = 2;
                while pos < data.len() {
                    let size = data[pos] as usize;
                    if let Some(name) = data.get(pos + 1..pos + 1 + size) {
                        hello.alpn.push(String::from_utf8_lossy(name).into_owned());
                    }
                    pos += 1 + size;
                }
            }
            0x0000 if data.len() >= 5 => {
                let name = u16::from_be_bytes([data[3], data[4]]) as usize;
                if let Some(name) = data.get(5..5 + name) {
                    hello.server_name = Some(String::from_utf8_lossy(name).into_owned());
                }
            }
            _ => {}
        }
        at += 4 + size;
    }
    hello
}
