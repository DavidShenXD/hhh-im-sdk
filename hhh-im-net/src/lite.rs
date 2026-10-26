//! iPhone / iPad（`wasm32-wasip2`）上的 HTTP 客户端：reqwest 0.12 接口的同名子集，名字、参数与语义一致，
//! 插件代码不分平台。底层是 hyper（HTTP/1.1）跑在 tokio 上，TLS 用本包统一的设置（[`crate::tls_config`]）。
//!
//! 电脑上也编进来：本包的测试拿它与 reqwest 逐项对照（同一个本机服务器、同样的请求，看到的字节与结果一致）。

mod body;
mod client;
mod error;
mod into_url;
pub mod multipart;
mod pool;
pub mod redirect;
mod request;
mod response;

pub use body::Body;
pub use client::{Client, ClientBuilder};
pub use error::{Error, Result};
pub use http::header;
pub use http::{Method, StatusCode, Version};
pub use into_url::IntoUrl;
pub use request::{Request, RequestBuilder};
pub use response::Response;
pub use url::Url;

/// 用缺省设置发一个 GET（与 `reqwest::get` 相同；要多次请求时建一个 [`Client`] 复用）。
///
/// # Errors
///
/// 同 [`RequestBuilder::send`]。
pub async fn get<T: IntoUrl>(url: T) -> Result<Response> {
    Client::builder().build()?.get(url).send().await
}

/// 建一个出站 HTTP 客户端的起点（与电脑上的 `hhh_im_net::client_builder` 同名同义；iPhone 上缺省就是统一握手设置）。
pub fn client_builder() -> ClientBuilder {
    Client::builder()
}

/// 下载用 HTTP 客户端的起点：连接超时 10 秒，不设总超时（与电脑上的同名函数一致；TCP 保活在 iPhone 上不起作用）。
pub fn download_client_builder() -> ClientBuilder {
    client_builder()
        .connect_timeout(crate::DOWNLOAD_CONNECT_TIMEOUT)
        .tcp_keepalive(crate::DOWNLOAD_TCP_KEEPALIVE)
}

/// 把网络错误连同底层原因拼成一行，不带请求地址（与电脑上的同名函数一致）。
pub fn network_cause(error: Error) -> String {
    use std::error::Error as _;
    let error = error.without_url();
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(cause) = source {
        parts.push(cause.to_string());
        source = cause.source();
    }
    parts.join(" ← ")
}
