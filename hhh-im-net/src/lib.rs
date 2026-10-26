//! IM 插件联网的唯一入口：电脑与 iPhone / iPad 同一套写法。
//!
//! - **电脑**（Windows / macOS / Linux）：本包就是 [reqwest](https://docs.rs/reqwest/0.12) 本身（原样重新导出），
//!   另加 [`client_builder`]（统一的 TLS 握手设置）、[`download_client_builder`]（下载用）、[`network_cause`]
//!   （不带地址的网络错误原因）。
//! - **iPhone / iPad**（`wasm32-wasip2`）：reqwest 在 wasm32 上只有浏览器版本，WASI 里用不了；这里是接口同名的
//!   一套实现（hyper HTTP/1.1 跑在 tokio 上），名字、参数与语义都跟 reqwest 0.12 一致，只是少了一些不常用的用法
//!   （见下）。少了的用法在 iPhone 版编译时报错，不会运行时假装成功。
//! - **两边**：[`connect_tcp`] 按主机名直接开 TCP 连接（自己的协议用）；[`tls_config`] 是统一的 TLS 设置，
//!   在 TCP 上自己握 TLS（如 WebSocket）时用它。
//!
//! # 怎么接
//!
//! 插件的 `Cargo.toml` 依赖本包（电脑与 iPhone 都是这一行），不再直接依赖 reqwest；代码里把 `reqwest::` 换成
//! `hhh_im_net::`。只想先接 iPhone、电脑版暂时不动的老插件，可以把 reqwest 留在
//! `[target.'cfg(not(target_os = "wasi"))'.dependencies]`、把本包放进 `[target.'cfg(target_os = "wasi")'.dependencies]`，
//! 再在 crate 根写一行 `#[cfg(target_os = "wasi")] extern crate hhh_im_net as reqwest;`——子模块里的 `reqwest::…`
//! 在 iPhone 版就解析到本包。
//!
//! 建客户端用 [`client_builder`]（或下载用的 [`download_client_builder`]），不要直接 `Client::new()` /
//! `Client::builder()`：电脑上只有前者装好了统一的握手设置，后者没有加密后端、一发 https 就 panic；
//! iPhone 上两者一样。
//!
//! 编 iPhone 版要 wasi-sdk 的 clang（aws-lc-rs 有 C 代码，设 `CC_wasm32_wasip2` 等）和
//! `RUSTFLAGS="--cfg tokio_unstable"`（tokio 在 WASI 上的网络要它）；HHH 的统一 CI 已经配好。
//!
//! # iPhone 版有、没有的用法
//!
//! 有：`get` / `post` / `put` / `patch` / `delete` / `head` / `request`、请求头、`query`、`json`、`form`、
//! `multipart`、`body`、`bearer_auth` / `basic_auth`、整体与读取与连接超时、跳转（缺省至多 10 次、`Policy::none()`、
//! `Policy::limited(n)`，换主机时去掉认证头）、gzip 自动解压、连接复用与闲置超时、`status` / `headers` /
//! `content_length` / `url` / `json` / `text` / `bytes` / `chunk` / `error_for_status`，`Error` 的
//! `is_timeout` / `is_connect` / `status` / `without_url` 等。
//!
//! 没有：阻塞接口（`blocking`，WASI 上没有线程）、HTTP/2、代理（`no_proxy` 照收、不起作用）、Cookie、
//! 流式请求体、`bytes_stream`、TCP 保活（`tcp_keepalive` 照收、不起作用）。
//!
//! # Examples
//!
//! ```no_run
//! # async fn demo() -> Result<(), hhh_im_net::Error> {
//! let client = hhh_im_net::client_builder()
//!     .timeout(std::time::Duration::from_secs(30))
//!     .build()?;
//! let reply: serde_json::Value = client
//!     .get("https://example.com/api")
//!     .query(&[("page", "1")])
//!     .bearer_auth("token")
//!     .send()
//!     .await?
//!     .error_for_status()?
//!     .json()
//!     .await?;
//! # drop(reply);
//! # Ok(())
//! # }
//! ```

use std::time::Duration;

mod handshake;
mod tcp;

/// 下载用客户端的连接超时：10 秒。
pub const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 下载用客户端的 TCP 保活间隔：15 秒，让系统及早发现对端已经不在的连接（iPhone 上不起作用）。
pub const DOWNLOAD_TCP_KEEPALIVE: Duration = Duration::from_secs(15);

/// iPhone / iPad 那套实现。iPhone 版本包的根就是它；电脑上也编进来，只给本包与 HHH 的对照测试用。
#[doc(hidden)]
pub mod lite;

#[cfg(not(target_os = "wasi"))]
mod native;

#[cfg(not(target_os = "wasi"))]
pub use native::*;

#[cfg(target_os = "wasi")]
pub use lite::*;

pub use handshake::tls_config;
pub use tcp::connect_tcp;

/// 测试用：信任测试根证书的 TLS 设置（feature `test-client`），本机测试服务器（feature `test-server`）。
#[cfg(any(test, feature = "test-client"))]
#[doc(hidden)]
pub mod testing;

#[cfg(all(test, not(target_os = "wasi")))]
mod equivalence_tests;
