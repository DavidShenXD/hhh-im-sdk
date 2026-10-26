//! 电脑上：reqwest 本身，加上统一握手的建客户端入口与网络错误原因。
//!
//! reqwest 这里没带加密后端（TLS 设置由本包整份交给它），所以它自己的缺省客户端（`Client::new()`、
//! `Client::builder().build()`、`reqwest::get`）一发 https 就 panic（“No provider set”）。本包的 [`get`] 换成走
//! [`client_builder`]；建客户端也一律从 [`client_builder`] 开始。

pub use reqwest::*;

/// 建一个出站 HTTP 客户端的起点：已装好统一的 TLS 握手设置（[`crate::tls_config`]）。
///
/// 调用方在它上面继续设超时、跳转、请求头等，再 `build()`。不要在它上面再调 TLS 调节方法（最低版本、额外根证书、
/// 忽略证书等）：预置设置下它们不生效。
///
/// # Examples
///
/// ```
/// let client = hhh_im_net::client_builder()
///     .timeout(std::time::Duration::from_secs(30))
///     .build()
///     .expect("统一入口建出的客户端总能建成");
/// # drop(client);
/// ```
pub fn client_builder() -> ClientBuilder {
    Client::builder().use_preconfigured_tls((*crate::tls_config()).clone())
}

/// 用统一入口建的客户端发一个 GET（盖过 `reqwest::get`，理由见模块说明）；要多次请求时建一个客户端复用。
///
/// # Errors
///
/// 同 `RequestBuilder::send`。
pub async fn get<T: IntoUrl>(url: T) -> Result<Response> {
    client_builder().build()?.get(url).send().await
}

/// 下载用 HTTP 客户端的起点：统一握手（[`client_builder`]）加连接超时与 TCP 保活。
///
/// **不设总超时**：大文件慢慢下不会被一刀切断；等首个响应与读停滞的超时归调用方。
///
/// # Examples
///
/// ```
/// let client = hhh_im_net::download_client_builder()
///     .redirect(hhh_im_net::redirect::Policy::none())
///     .build()
///     .expect("统一入口建出的客户端总能建成");
/// # drop(client);
/// ```
pub fn download_client_builder() -> ClientBuilder {
    client_builder()
        .connect_timeout(crate::DOWNLOAD_CONNECT_TIMEOUT)
        .tcp_keepalive(crate::DOWNLOAD_TCP_KEEPALIVE)
}

/// 把网络错误连同底层原因拼成一行，例如
/// `error sending request ← client error (Connect) ← Connection refused (os error 61)`。
///
/// 只打最外层常常是句废话（“发送失败”），真正的原因（连接被重置、握手失败、系统错误码）埋在 `source` 链深处，
/// 排障要全链。先去掉请求地址：地址里可能带会话编号、分页游标、签名参数；令牌只在请求头里，不会出现。
///
/// # Examples
///
/// ```
/// let error = hhh_im_net::client_builder()
///     .build()
///     .unwrap()
///     .get("http://exa mple.com/?page_token=secret")
///     .build()
///     .unwrap_err();
/// let cause = hhh_im_net::network_cause(error);
/// assert!(cause.starts_with("builder error ← "), "{cause}");
/// assert!(!cause.contains("secret"));
/// ```
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

#[cfg(test)]
mod tests {
    #[test]
    fn client_builder_builds_clients_repeatedly() {
        for _ in 0..3 {
            super::client_builder()
                .build()
                .expect("统一入口建出的客户端总能建成");
            super::download_client_builder()
                .build()
                .expect("下载入口建出的客户端总能建成");
        }
    }

    /// 本机一个刚关掉的端口：得到真实的“连接被拒绝”，看原因链是否完整、是否带出地址。
    #[tokio::test]
    async fn network_cause_keeps_the_chain_but_not_the_address() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let error = super::client_builder()
            .build()
            .unwrap()
            .get(format!(
                "http://127.0.0.1:{port}/open-apis/im/v1/messages?page_token=secret-cursor"
            ))
            .send()
            .await
            .expect_err("端口已关闭");
        assert!(error.is_connect());
        let cause = super::network_cause(error);
        assert!(cause.contains(" ← "), "应带底层原因：{cause}");
        assert!(
            !cause.contains("secret-cursor") && !cause.contains("open-apis"),
            "不应带请求地址：{cause}"
        );
    }
}
