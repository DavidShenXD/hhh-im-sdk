//! 对照测试：同一段请求分别用电脑版（reqwest）与 iPhone 版（`lite`）对同一个本机测试服务器跑一遍，
//! 服务器收到的字节（方法、目标、请求头及顺序、请求体、用了第几条连接）与调用方拿到的结果
//! （状态、最终地址、长度、响应头、响应体，或错误的文字、种类、地址与原因链）必须一致。

use std::time::Duration;

use crate::testing::{self, HelloProbe, ScriptedServer, Seen, StallingServer, TestServer};

/// 同一段测试分别以 `net = reqwest`（电脑版）与 `net = lite`（iPhone 版）展开，返回两份结果。
macro_rules! both {
    (|$net:ident| $body:expr) => {{
        let desktop = {
            #[allow(unused_imports)]
            use crate::native as $net;
            $body.await
        };
        let iphone = {
            #[allow(unused_imports)]
            use crate::lite as $net;
            $body.await
        };
        (desktop, iphone)
    }};
}

/// 错误的可观察部分。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Failure {
    text: String,
    builder: bool,
    request: bool,
    connect: bool,
    timeout: bool,
    redirect: bool,
    body: bool,
    decode: bool,
    status: Option<u16>,
    url: Option<String>,
    chain: Vec<String>,
}

macro_rules! failure {
    ($error:expr) => {{
        let error = &$error;
        Failure {
            text: error.to_string(),
            builder: error.is_builder(),
            request: error.is_request(),
            connect: error.is_connect(),
            timeout: error.is_timeout(),
            redirect: error.is_redirect(),
            body: error.is_body(),
            decode: error.is_decode(),
            status: error.status().map(|s| s.as_u16()),
            url: error.url().map(ToString::to_string),
            chain: chain(&error),
        }
    }};
}

/// 响应的可观察部分（响应头去掉 `date`）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Reply {
    status: u16,
    url: String,
    content_length: Option<u64>,
    headers: Vec<(String, String)>,
    body: Result<Vec<u8>, Failure>,
}

macro_rules! reply {
    ($result:expr) => {{
        match $result {
            Err(error) => Err(failure!(error)),
            Ok(response) => {
                let status = response.status().as_u16();
                let url = response.url().to_string();
                let content_length = response.content_length();
                let headers = response_headers(response.headers());
                let body = match response.bytes().await {
                    Ok(bytes) => Ok(bytes.to_vec()),
                    Err(error) => Err(failure!(error)),
                };
                Ok::<Reply, Failure>(Reply {
                    status,
                    url,
                    content_length,
                    headers,
                    body,
                })
            }
        }
    }};
}

/// 信任测试根证书的客户端设置起点（两边同一份 TLS 设置）。
macro_rules! test_client {
    ($net:ident) => {
        $net::Client::builder().use_preconfigured_tls(testing::client_tls())
    };
}

fn chain(error: &dyn std::error::Error) -> Vec<String> {
    let mut parts = Vec::new();
    let mut source = error.source();
    while let Some(cause) = source {
        parts.push(cause.to_string());
        source = cause.source();
    }
    parts
}

fn response_headers(headers: &http::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, _)| *name != http::header::DATE)
        .map(|(k, v)| {
            (
                k.as_str().to_owned(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect()
}

/// 服务器记录里的连接按第一次用到的先后编号（两次运行各自的第几条），multipart 分界串换成固定值。
fn normalized(mut seen: Vec<Seen>) -> Vec<Seen> {
    // 连接按第一次用到的先后重新编号：只看哪些请求共用了连接（连上又没用的连接不算，hyper-util 与
    // iPhone 版在“边等空闲连接边新连”时都可能多连一条）。
    let mut order: Vec<usize> = Vec::new();
    for record in &mut seen {
        let index = match order.iter().position(|c| *c == record.connection) {
            Some(index) => index,
            None => {
                order.push(record.connection);
                order.len() - 1
            }
        };
        record.connection = index + 1;
        let boundary = record
            .header("content-type")
            .first()
            .and_then(|kind| kind.split_once("boundary="))
            .map(|(_, boundary)| boundary.to_owned());
        if let Some(boundary) = boundary {
            for (_, value) in &mut record.headers {
                *value = value.replace(&boundary, "BOUNDARY");
            }
            let body = String::from_utf8_lossy(&record.body).replace(&boundary, "BOUNDARY");
            record.body = body.into_bytes();
        }
    }
    seen
}

macro_rules! compare {
    ($server:expr, $what:expr, |$net:ident| $body:expr) => {{
        let server: &TestServer = $server;
        server.take_seen();
        let desktop = {
            #[allow(unused_imports)]
            use crate::native as $net;
            $body.await
        };
        let desktop_seen = normalized(server.take_seen());
        let iphone = {
            #[allow(unused_imports)]
            use crate::lite as $net;
            $body.await
        };
        let iphone_seen = normalized(server.take_seen());
        assert_eq!(desktop_seen, iphone_seen, "{}：服务器收到的不同", $what);
        assert_eq!(desktop, iphone, "{}：两边结果不同", $what);
        (desktop, desktop_seen)
    }};
}

#[tokio::test]
async fn common_calls_send_and_receive_the_same_bytes() {
    for server in [TestServer::http().await, TestServer::https().await] {
        let base = server.url("");
        let (reply, seen) = compare!(&server, "GET 带查询与认证", |net| async {
            let client = test_client!(net).build().unwrap();
            reply!(
                client
                    .get(format!("{base}/json?x=1"))
                    .query(&[("a", "1"), ("b", "x y")])
                    .bearer_auth("tok")
                    .header("x-custom", "v")
                    .send()
                    .await
            )
        });
        let reply = reply.unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(seen[0].target, "/json?x=1&a=1&b=x+y");
        assert_eq!(seen[0].header("authorization"), ["Bearer tok"]);

        let (reply, seen) = compare!(&server, "POST JSON", |net| async {
            let client = test_client!(net).build().unwrap();
            reply!(
                client
                    .post(format!("{base}/echo"))
                    .json(&serde_json::json!({"text": "你好", "n": 1}))
                    .send()
                    .await
            )
        });
        let expected = serde_json::to_vec(&serde_json::json!({"text": "你好", "n": 1})).unwrap();
        assert_eq!(reply.unwrap().body.unwrap(), expected);
        assert_eq!(seen[0].header("content-type"), ["application/json"]);

        for (method, label) in [
            ("POST", "表单"),
            ("PUT", "字节"),
            ("PATCH", "文字"),
            ("DELETE", "无体"),
            ("HEAD", "HEAD"),
        ] {
            let _ = compare!(&server, label, |net| async {
                let client = test_client!(net).build().unwrap();
                let url = format!("{base}/echo");
                let builder = match method {
                    "POST" => client.post(&url).form(&[("a", "b c"), ("d", "中")]),
                    "PUT" => client.put(&url).body(vec![0u8, 1, 2, 255]),
                    "PATCH" => client.patch(&url).body("text body"),
                    "DELETE" => client.delete(&url),
                    _ => client.head(&url),
                };
                reply!(builder.send().await)
            });
        }
    }
}

#[tokio::test]
async fn multipart_uploads_are_identical_byte_for_byte() {
    let server = TestServer::https().await;
    let url = server.url("/json");
    let (_, seen) = compare!(&server, "multipart", |net| async {
        let client = test_client!(net).build().unwrap();
        let form = net::multipart::Form::new()
            .text("plain", "hello")
            .part(
                "file",
                net::multipart::Part::bytes(vec![1u8, 2, 3, 0, 255])
                    .file_name("图 \"1\".png")
                    .mime_str("image/png")
                    .unwrap(),
            )
            .text("名字", "值");
        reply!(client.post(&url).multipart(form).send().await)
    });
    assert!(String::from_utf8_lossy(&seen[0].body).ends_with("--BOUNDARY--\r\n"));
}

#[tokio::test]
async fn redirects_follow_the_same_rules() {
    let server = TestServer::http().await;
    let other = TestServer::http().await;
    let secure = TestServer::https().await;
    let to =
        |target: &str| url::form_urlencoded::byte_serialize(target.as_bytes()).collect::<String>();
    let cases = [
        ("POST 302 改 GET 去体", "POST", 302, "/echo".to_owned()),
        ("PUT 303 改 GET 去体", "PUT", 303, "/echo".to_owned()),
        ("HEAD 303 仍是 HEAD", "HEAD", 303, "/echo".to_owned()),
        ("POST 307 原样重发", "POST", 307, "/echo".to_owned()),
        ("PUT 308 原样重发", "PUT", 308, "/echo".to_owned()),
        ("PUT 301 原样重发", "PUT", 301, "/echo".to_owned()),
        ("相对地址", "GET", 302, "echo?from=relative".to_owned()),
        (
            "换主机去认证头",
            "GET",
            302,
            server.url_on("localhost", "/echo"),
        ),
        ("换端口去认证头", "GET", 302, other.url("/echo")),
        ("跳到 ftp", "GET", 302, "ftp://example.com/file".to_owned()),
        ("没有可用的 Location", "GET", 302, "http://[bad".to_owned()),
    ];
    for (label, method, code, target) in cases {
        let start = format!(
            "{}?to={}",
            server.url(&format!("/redirect/{code}")),
            to(&target)
        );
        let _ = compare!(&server, label, |net| async {
            let client = test_client!(net).build().unwrap();
            let builder = match method {
                "POST" => client.post(&start).json(&serde_json::json!({"k": "v"})),
                "PUT" => client.put(&start).body("put body"),
                "HEAD" => client.head(&start),
                _ => client.get(&start),
            };
            reply!(
                builder
                    .bearer_auth("secret")
                    .header("x-keep", "1")
                    .send()
                    .await
            )
        });
    }
    // 换主机那一跳：认证头去掉、其它头与来源页带上（另一个端口的服务器上看）。
    let start = format!(
        "{}?to={}",
        server.url("/redirect/302"),
        to(&other.url("/echo"))
    );
    other.take_seen();
    let (desktop, iphone) = both!(|net| async {
        let client = test_client!(net).build().unwrap();
        client
            .get(&start)
            .bearer_auth("secret")
            .header("x-keep", "1")
            .send()
            .await
            .unwrap();
        other.take_seen()
    });
    assert_eq!(normalized(desktop.clone()), normalized(iphone));
    assert!(desktop[0].header("authorization").is_empty());
    assert_eq!(desktop[0].header("x-keep"), ["1"]);
    assert_eq!(desktop[0].header("referer"), [start.as_str()]);

    // https 跳到 http：不带来源页。
    let start = format!(
        "{}?to={}",
        secure.url("/redirect/302"),
        to(&server.url("/echo"))
    );
    server.take_seen();
    let (desktop, iphone) = both!(|net| async {
        let client = test_client!(net).build().unwrap();
        client.get(&start).send().await.unwrap();
        server.take_seen()
    });
    assert_eq!(normalized(desktop.clone()), normalized(iphone));
    assert!(desktop[0].header("referer").is_empty());

    // 次数限制、不跟、自定义。
    for (label, policy, path) in [
        ("缺省跟 10 次", "default", "/chain/10"),
        ("缺省第 11 次报错", "default", "/chain/11"),
        ("不跟", "none", "/chain/1"),
        ("至多 2 次", "limited", "/chain/3"),
        ("自定义停在第 2 跳", "custom", "/chain/5"),
    ] {
        let url = server.url(path);
        let _ = compare!(&server, label, |net| async {
            let policy = match policy {
                "none" => net::redirect::Policy::none(),
                "limited" => net::redirect::Policy::limited(2),
                "custom" => net::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() >= 2 {
                        attempt.stop()
                    } else {
                        attempt.follow()
                    }
                }),
                _ => net::redirect::Policy::default(),
            };
            let client = test_client!(net).redirect(policy).build().unwrap();
            reply!(client.get(&url).send().await)
        });
    }
    // 只许 https 时跳到 http。
    let start = format!(
        "{}?to={}",
        secure.url("/redirect/302"),
        to(&server.url("/echo"))
    );
    let _ = compare!(&secure, "只许 https", |net| async {
        let client = test_client!(net).https_only(true).build().unwrap();
        reply!(client.get(&start).send().await)
    });
}

/// 坏 gzip 的最深一层原因来自不同的解压库（reqwest 用 async-compression，iPhone 版用 flate2），只差大小写，
/// 对照时统一成小写；外层的错误种类与文字仍逐字比较。
fn decoder_causes_lowercased(reply: Result<Reply, Failure>) -> Result<Reply, Failure> {
    reply.map(|mut reply| {
        if let Err(failure) = &mut reply.body {
            for cause in &mut failure.chain {
                *cause = cause.to_lowercase();
            }
        }
        reply
    })
}

#[tokio::test]
async fn gzip_is_handled_the_same_way() {
    let server = TestServer::http().await;
    for path in ["/gzip", "/gzip-multi", "/gzip-empty", "/gzip-bad"] {
        let url = server.url(path);
        let (reply, _) = compare!(&server, path, |net| async {
            let client = test_client!(net).build().unwrap();
            decoder_causes_lowercased(reply!(client.get(&url).send().await))
        });
        if path == "/gzip" {
            let reply = reply.unwrap();
            assert_eq!(reply.body.unwrap(), testing::GZIP_TEXT.as_bytes());
            assert_eq!(reply.content_length, None);
            assert!(reply.headers.iter().all(|(k, _)| k != "content-encoding"));
        }
        let _ = compare!(&server, "关掉 gzip", |net| async {
            let client = test_client!(net).gzip(false).build().unwrap();
            decoder_causes_lowercased(reply!(client.get(&url).send().await))
        });
        let _ = compare!(&server, "自己声明 identity", |net| async {
            let client = test_client!(net).build().unwrap();
            decoder_causes_lowercased(reply!(
                client
                    .get(&url)
                    .header("accept-encoding", "identity")
                    .send()
                    .await
            ))
        });
        let _ = compare!(&server, "设了超时", |net| async {
            let client = test_client!(net)
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap();
            decoder_causes_lowercased(reply!(client.get(&url).send().await))
        });
    }
}

#[tokio::test]
async fn chunked_bodies_read_the_same() {
    let server = TestServer::https().await;
    let url = server.url("/chunks?n=6&size=5&gap_ms=10");
    let (body, _) = compare!(&server, "按块读", |net| async {
        let client = test_client!(net).build().unwrap();
        let mut response = client.get(&url).send().await.unwrap();
        assert_eq!(response.content_length(), None);
        let mut all = Vec::new();
        let mut chunks = 0;
        while let Some(chunk) = response.chunk().await.unwrap() {
            all.extend_from_slice(&chunk);
            chunks += 1;
        }
        assert!(chunks >= 2, "应分多块到达");
        assert_eq!(response.chunk().await.unwrap(), None);
        all
    });
    assert_eq!(body.len(), 30);
    let url = server.url("/bytes?size=300000");
    let _ = compare!(&server, "大响应体", |net| async {
        let client = test_client!(net).build().unwrap();
        reply!(client.get(&url).send().await)
    });
}

#[tokio::test]
async fn timeouts_fail_the_same_way() {
    let server = TestServer::http().await;
    let slow_headers = server.url("/slow-headers?ms=1500");
    let slow_body = server.url("/slow-body?ms=1500");
    let (reply, _) = compare!(&server, "整体超时（等响应头）", |net| async {
        let client = test_client!(net)
            .timeout(Duration::from_millis(300))
            .build()
            .unwrap();
        reply!(client.get(&slow_headers).send().await)
    });
    assert!(reply.unwrap_err().timeout);
    let _ = compare!(&server, "请求自己的超时盖过客户端的", |net| async {
        let client = test_client!(net)
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();
        reply!(
            client
                .get(&slow_headers)
                .timeout(Duration::from_millis(300))
                .send()
                .await
        )
    });
    let (reply, _) = compare!(&server, "整体超时（读响应体）", |net| async {
        let client = test_client!(net)
            .timeout(Duration::from_millis(500))
            .build()
            .unwrap();
        reply!(client.get(&slow_body).send().await)
    });
    let failure = reply.unwrap().body.unwrap_err();
    assert!(failure.timeout && failure.decode);
    let _ = compare!(&server, "读取超时（等响应头）", |net| async {
        let client = test_client!(net)
            .read_timeout(Duration::from_millis(300))
            .build()
            .unwrap();
        reply!(client.get(&slow_headers).send().await)
    });
    let _ = compare!(&server, "读取超时（两块之间）", |net| async {
        let client = test_client!(net)
            .read_timeout(Duration::from_millis(500))
            .build()
            .unwrap();
        reply!(client.get(&slow_body).send().await)
    });
    let _ = compare!(&server, "读取超时之内正常", |net| async {
        let client = test_client!(net)
            .read_timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        reply!(client.get(slow_body.replace("1500", "100")).send().await)
    });
}

#[tokio::test]
async fn connection_failures_look_the_same() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let closed = format!(
        "http://127.0.0.1:{}/x?cursor=secret",
        listener.local_addr().unwrap().port()
    );
    drop(listener);
    let (desktop, iphone) = both!(|net| async {
        let error = test_client!(net)
            .build()
            .unwrap()
            .get(&closed)
            .send()
            .await
            .unwrap_err();
        (failure!(error), net::network_cause(error))
    });
    assert_eq!(desktop, iphone, "连接被拒绝");
    assert!(desktop.0.connect && desktop.0.request);
    assert!(!desktop.1.contains("secret"));

    let https = TestServer::https().await;
    let url = https.url("/json");
    let (desktop, iphone) = both!(|net| async {
        // 生产设置不认测试根证书：握手失败。
        let error = net::client_builder()
            .build()
            .unwrap()
            .get(&url)
            .send()
            .await
            .unwrap_err();
        (failure!(error), net::network_cause(error))
    });
    assert_eq!(desktop, iphone, "证书不受信任");
    assert!(desktop.0.connect);

    let stalling = StallingServer::start().await;
    let url = stalling.url("https", "/x");
    let (desktop, iphone) = both!(|net| async {
        let client = test_client!(net)
            .connect_timeout(Duration::from_millis(300))
            .build()
            .unwrap();
        let error = client.get(&url).send().await.unwrap_err();
        (failure!(error), net::network_cause(error))
    });
    assert_eq!(desktop, iphone, "连接超时（TLS 握手等不到回应）");
    assert!(desktop.0.connect && desktop.0.timeout);

    let truncated = ScriptedServer::truncated_body().await;
    let url = truncated.url("/x");
    for timeout in [None, Some(Duration::from_secs(5))] {
        let (desktop, iphone) = both!(|net| async {
            let mut builder = test_client!(net);
            if let Some(timeout) = timeout {
                builder = builder.timeout(timeout);
            }
            reply!(builder.build().unwrap().get(&url).send().await)
        });
        assert_eq!(desktop, iphone, "响应体不完整（超时 {timeout:?}）");
        assert!(desktop.unwrap().body.unwrap_err().decode);
    }
}

#[tokio::test]
async fn status_errors_look_the_same() {
    let server = TestServer::http().await;
    for path in [
        "/status/404",
        "/status/503",
        "/reason",
        "/status/200",
        "/status/302",
    ] {
        let url = server.url(path);
        let _ = compare!(&server, path, |net| async {
            let client = test_client!(net)
                .redirect(net::redirect::Policy::none())
                .build()
                .unwrap();
            let response = client.get(&url).send().await.unwrap();
            let by_ref = response.error_for_status_ref().err().map(|e| failure!(e));
            let owned = response.error_for_status().err().map(|e| failure!(e));
            (by_ref, owned)
        });
    }
}

#[tokio::test]
async fn bad_input_fails_the_same_way() {
    let server = TestServer::http().await;
    let http_url = server.url("/json");
    let (desktop, iphone) = both!(|net| async {
        let client = test_client!(net).build().unwrap();
        let mut failures = Vec::new();
        for url in [
            "not a url",
            "ftp://example.com/x",
            "mailto:someone@example.com",
            "http://",
            "https://exa mple.com/",
        ] {
            failures.push(failure!(client.get(url).send().await.unwrap_err()));
        }
        failures.push(failure!(
            client
                .get(&http_url)
                .header("bad header", "v")
                .send()
                .await
                .unwrap_err()
        ));
        failures.push(failure!(
            client
                .get(&http_url)
                .header("x-ok", "bad\nvalue")
                .send()
                .await
                .unwrap_err()
        ));
        let https_only = test_client!(net).https_only(true).build().unwrap();
        failures.push(failure!(
            https_only.get(&http_url).send().await.unwrap_err()
        ));
        failures.push(failure!(
            net::Client::builder()
                .use_preconfigured_tls(42u8)
                .build()
                .unwrap_err()
        ));
        failures.push(failure!(
            net::Client::builder()
                .user_agent("bad\nagent")
                .build()
                .unwrap_err()
        ));
        failures.push(failure!(
            net::multipart::Part::text("x")
                .mime_str("not a mime")
                .unwrap_err()
        ));
        failures
    });
    assert_eq!(desktop, iphone);
    assert!(desktop.iter().all(|f| f.builder));
}

#[tokio::test]
async fn client_settings_reach_the_server_the_same_way() {
    let server = TestServer::http().await;
    let url = server.url("/json");
    let mut defaults = http::HeaderMap::new();
    defaults.insert("x-default", "client".parse().unwrap());
    defaults.insert("x-shared", "client".parse().unwrap());
    let (_, seen) = compare!(&server, "User-Agent 与默认请求头", |net| async {
        let client = test_client!(net)
            .user_agent("hhh-test/1")
            .default_headers(defaults.clone())
            .build()
            .unwrap();
        reply!(client.get(&url).header("x-shared", "request").send().await)
    });
    assert_eq!(seen[0].header("x-shared"), ["request"]);
    assert_eq!(seen[0].header("user-agent"), ["hhh-test/1"]);

    let with_credentials = server
        .url("/json")
        .replace("127.0.0.1", "us%20er:p%40ss@127.0.0.1");
    let (reply, seen) = compare!(&server, "地址里的账号密码", |net| async {
        let client = test_client!(net).build().unwrap();
        reply!(client.get(&with_credentials).send().await)
    });
    assert!(!reply.unwrap().url.contains("ss@"));
    assert_eq!(seen[0].header("authorization"), ["Basic dXMgZXI6cEBzcw=="]);

    let _ = compare!(&server, "build 后 execute、try_clone", |net| async {
        let client = test_client!(net).build().unwrap();
        let builder = client.post(&url).body("again").basic_auth("u", Some("p"));
        let copy = builder.try_clone().unwrap();
        let request = builder.build().unwrap();
        let first = reply!(client.execute(request).await);
        let second = reply!(copy.send().await);
        (first, second)
    });
}

#[tokio::test]
async fn connections_are_reused_the_same_way() {
    let (desktop, iphone) = both!(|net| async {
        let server = TestServer::https().await;
        let client = test_client!(net).build().unwrap();
        let mut counts = Vec::new();
        for _ in 0..3 {
            client
                .get(server.url("/json"))
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
        }
        counts.push(server.connections());
        // 回完就关的连接不再用。
        client
            .get(server.url("/close"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        client
            .get(server.url("/json"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        counts.push(server.connections());
        // 没读完就丢掉的响应：连接跟着丢。
        drop(
            client
                .get(server.url("/bytes?size=200000"))
                .send()
                .await
                .unwrap(),
        );
        client
            .get(server.url("/json"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        counts.push(server.connections());
        // 本来就没有体的响应（HEAD、204）不读也能复用。
        let _ = client.head(server.url("/json")).send().await.unwrap();
        let _ = client.get(server.url("/empty")).send().await.unwrap();
        client
            .get(server.url("/json"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        counts.push(server.connections());

        let no_pool = test_client!(net).pool_max_idle_per_host(0).build().unwrap();
        let before = server.connections();
        for _ in 0..2 {
            no_pool
                .get(server.url("/json"))
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
        }
        counts.push(server.connections() - before);

        let short_idle = test_client!(net)
            .pool_idle_timeout(Duration::from_millis(100))
            .build()
            .unwrap();
        let before = server.connections();
        short_idle
            .get(server.url("/json"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        short_idle
            .get(server.url("/json"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        counts.push(server.connections() - before);
        counts
    });
    assert_eq!(desktop, iphone);
    assert_eq!(desktop[0], 1, "连续请求应复用一条连接");
}

#[tokio::test]
async fn connections_closed_by_the_server_are_replaced() {
    let (desktop, iphone) = both!(|net| async {
        let server = TestServer::http_closing_idle(Duration::from_millis(100)).await;
        let client = test_client!(net).build().unwrap();
        let mut statuses = Vec::new();
        for _ in 0..3 {
            let response = client.get(server.url("/json")).send().await.unwrap();
            statuses.push(response.status().as_u16());
            response.bytes().await.unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        (statuses, server.connections())
    });
    assert_eq!(desktop, iphone);
    assert_eq!(desktop, (vec![200, 200, 200], 3));
}

#[tokio::test]
async fn both_send_the_post_quantum_hello_every_time() {
    let (desktop, iphone) = both!(|net| async {
        let mut probe = HelloProbe::start().await;
        let client = net::client_builder().build().unwrap();
        let mut hellos = Vec::new();
        for _ in 0..2 {
            let _ = client.get(probe.url("/")).send().await;
            hellos.push(probe.next().await);
        }
        let downloads = net::download_client_builder().build().unwrap();
        let _ = downloads.get(probe.url("/")).send().await;
        hellos.push(probe.next().await);
        hellos
    });
    for hello in desktop.iter().chain(&iphone) {
        assert!(hello.len > 1200, "握手请求应超过 1200 字节：{hello:?}");
        assert_eq!(
            hello.key_shares,
            vec![testing::X25519MLKEM768, testing::X25519]
        );
        assert_eq!(hello.server_name.as_deref(), Some("localhost"));
    }
}

/// 电脑版的握手与飞书、Slack 现有 `src/net.rs` 那份逐项相同：同一把设置（下面照抄它们的组装方式）
/// 与本包 `client_builder()` 各连一次探针，握手请求的长度、密码套件、扩展（rustls 会打乱顺序，按集合比）、
/// 密钥份额、SNI、ALPN 都一样。
#[tokio::test]
async fn desktop_handshake_is_identical_to_the_copy_plugins_carry_today() {
    use std::sync::Arc;
    // 照抄飞书 / Slack `src/net.rs` 的 `tls_config()` + `config_with()`。
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut copy = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(rustls::ALL_VERSIONS)
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    copy.alpn_protocols = vec![b"http/1.1".to_vec()];
    copy.resumption = rustls::client::Resumption::store(crate::handshake::tickets_only());

    let mut probe = HelloProbe::start().await;
    let theirs = reqwest::Client::builder()
        .use_preconfigured_tls(copy)
        .build()
        .unwrap();
    let _ = theirs.get(probe.url("/")).send().await;
    let theirs = probe.next().await;
    let ours = crate::native::client_builder().build().unwrap();
    let _ = ours.get(probe.url("/")).send().await;
    let ours = probe.next().await;
    // rustls 每次随机打乱扩展的顺序（防僵化），按集合比。
    let sorted = |mut hello: testing::Hello| {
        hello.extensions.sort_unstable();
        hello
    };
    assert_eq!(sorted(ours.clone()), sorted(theirs));
    assert_eq!(ours.alpn, ["http/1.1"]);
}

#[test]
fn the_shared_helpers_behave_the_same() {
    assert!(crate::native::client_builder().build().is_ok());
    assert!(crate::lite::client_builder().build().is_ok());
    assert!(crate::lite::download_client_builder().build().is_ok());
    let native = crate::native::client_builder()
        .build()
        .unwrap()
        .get("http://exa mple.com/?page_token=secret")
        .build()
        .unwrap_err();
    let lite = crate::lite::client_builder()
        .build()
        .unwrap()
        .get("http://exa mple.com/?page_token=secret")
        .build()
        .unwrap_err();
    assert_eq!(
        crate::native::network_cause(native),
        crate::lite::network_cause(lite)
    );
}

#[tokio::test]
async fn free_get_works_on_both() {
    let server = TestServer::http().await;
    let url = server.url("/json");
    let _ = compare!(&server, "get()", |net| async {
        reply!(net::get(&url).await)
    });
}
