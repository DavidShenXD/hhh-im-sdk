//! 统一的 TLS 握手设置：电脑与 iPhone 两边、本包建的所有 HTTPS 连接都用这一份（与 HHH 自己的 `hhh-net` 同一策略）。
//!
//! 握手请求（ClientHello）长什么样，决定了客户端在网络上“像谁”。主流浏览器与各语言运行时都已优先抗量子密钥交换
//! X25519MLKEM768，并同时带传统 X25519：请求约 1.5 KB，超过一个 TCP 包。只看第一个包的 SNI 过滤设备读不全这种请求，
//! 会放行；而老式的小握手（约 240 字节）整包可见，会被单独掐断。所以：
//!
//! - 加密后端 aws-lc-rs，默认顺序优先 X25519MLKEM768；服务器不支持时直接用同时带上的 X25519，不多一次往返。
//! - **不记住服务器选过的密钥交换组**。rustls 默认会记住（下次只带那一个组的份额），对不支持抗量子的服务器，
//!   第二次起握手就退回小请求、又被拦；这里换成只缓存会话票据的会话缓存，每次新建连接都带抗量子份额。
//! - 其余与 reqwest 自带的 rustls 设置一致：TLS 1.3 与 1.2、内置 webpki 根证书、SNI、ALPN 只报 `http/1.1`。

use std::sync::{Arc, OnceLock};

use rustls::NamedGroup;
use rustls::client::{
    ClientSessionMemoryCache, ClientSessionStore, Resumption, Tls12ClientSessionValue,
    Tls13ClientSessionValue,
};
use rustls::pki_types::ServerName;

/// 会话缓存记多少个服务器（与 rustls 默认 `Resumption::in_memory_sessions(256)` 相同）。
const SESSION_CACHE_SERVERS: usize = 256;

/// 全进程共用的 TLS 设置；第一次用时建好。
///
/// 在 TCP 上自己握 TLS（如 WebSocket）时用它；发 HTTP 请求用 [`crate::client_builder`]，它已经带上这份设置。
/// 拷贝共享同一个会话缓存（内部是 `Arc`），不同客户端连同一服务器也能复用票据。
///
/// # Examples
///
/// ```
/// let config = hhh_im_net::tls_config();
/// assert_eq!(config.alpn_protocols, vec![b"http/1.1".to_vec()]);
/// ```
pub fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(config_with(roots, tickets_only()))
        })
        .clone()
}

/// 只缓存会话票据的会话缓存（新的一份）。
pub(crate) fn tickets_only() -> Arc<dyn ClientSessionStore> {
    Arc::new(TicketsOnly(ClientSessionMemoryCache::new(
        SESSION_CACHE_SERVERS,
    )))
}

/// 按给定根证书与会话缓存组装设置；生产设置与测试走同一条路。
pub(crate) fn config_with(
    roots: rustls::RootCertStore,
    sessions: Arc<dyn ClientSessionStore>,
) -> rustls::ClientConfig {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(rustls::ALL_VERSIONS)
        // 不变量：aws-lc-rs 默认 provider 自带 TLS 1.2 与 1.3 的密码套件，这一步只会在依赖本身坏掉时失败——
        // 属于“程序自身已坏”，由单测 `production_config_*` 锁住。
        .expect("aws-lc-rs 默认 provider 应支持 TLS 1.2 与 1.3")
        .with_root_certificates(roots)
        .with_no_client_auth();
    // 与 reqwest 在未开 HTTP/2 时的做法一致。
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config.resumption = Resumption::store(sessions);
    config
}

/// 只缓存会话票据、不记住服务器选过的密钥交换组的会话缓存。
///
/// rustls 用“记住的组”决定下次握手的初始密钥份额；不记，就总按默认顺序先带抗量子份额。
#[derive(Debug)]
struct TicketsOnly(ClientSessionMemoryCache);

impl ClientSessionStore for TicketsOnly {
    fn set_kx_hint(&self, _: ServerName<'static>, _: NamedGroup) {}

    fn kx_hint(&self, _: &ServerName<'_>) -> Option<NamedGroup> {
        None
    }

    fn set_tls12_session(&self, server_name: ServerName<'static>, value: Tls12ClientSessionValue) {
        self.0.set_tls12_session(server_name, value);
    }

    fn tls12_session(&self, server_name: &ServerName<'_>) -> Option<Tls12ClientSessionValue> {
        self.0.tls12_session(server_name)
    }

    fn remove_tls12_session(&self, server_name: &ServerName<'static>) {
        self.0.remove_tls12_session(server_name);
    }

    fn insert_tls13_ticket(
        &self,
        server_name: ServerName<'static>,
        value: Tls13ClientSessionValue,
    ) {
        self.0.insert_tls13_ticket(server_name, value);
    }

    fn take_tls13_ticket(
        &self,
        server_name: &ServerName<'static>,
    ) -> Option<Tls13ClientSessionValue> {
        self.0.take_tls13_ticket(server_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 抗量子份额 1216 字节；带上它的 ClientHello 一定超过这个长度，不带的远小于它。
    pub(crate) const PQ_CLIENT_HELLO_MIN: usize = 1200;

    fn server() -> ServerName<'static> {
        ServerName::try_from("example.com").expect("合法域名")
    }

    /// 按给定设置开一个连接，取它发出的第一个握手请求（ClientHello）的字节数。
    fn client_hello_len(config: &rustls::ClientConfig) -> usize {
        let mut connection =
            rustls::ClientConnection::new(Arc::new(config.clone()), server()).expect("设置可用");
        let mut bytes = Vec::new();
        connection.write_tls(&mut bytes).expect("写入内存");
        bytes.len()
    }

    #[test]
    fn production_config_prefers_post_quantum_and_keeps_x25519() {
        let config = tls_config();
        let groups: Vec<NamedGroup> = config
            .crypto_provider()
            .kx_groups
            .iter()
            .map(|group| group.name())
            .collect();
        assert_eq!(groups.first(), Some(&NamedGroup::X25519MLKEM768));
        assert!(groups.contains(&NamedGroup::X25519));
        assert!(client_hello_len(&config) > PQ_CLIENT_HELLO_MIN);
    }

    #[test]
    fn production_config_matches_reqwest_defaults() {
        let config = tls_config();
        assert_eq!(config.alpn_protocols, vec![b"http/1.1".to_vec()]);
        assert!(config.enable_sni);
        assert!(!config.client_auth_cert_resolver.has_certs());
    }

    /// 对照组：rustls 默认缓存记下服务器选了 X25519 后，下次握手只带 X25519，请求变小——这就是要换缓存的原因。
    #[test]
    fn default_rustls_cache_shrinks_the_next_hello_after_remembering_x25519() {
        let cache = Arc::new(ClientSessionMemoryCache::new(SESSION_CACHE_SERVERS));
        let config = config_with(rustls::RootCertStore::empty(), cache.clone());
        assert!(client_hello_len(&config) > PQ_CLIENT_HELLO_MIN);
        cache.set_kx_hint(server(), NamedGroup::X25519);
        assert!(client_hello_len(&config) < PQ_CLIENT_HELLO_MIN);
    }

    #[test]
    fn tickets_only_cache_keeps_every_hello_post_quantum() {
        let cache = tickets_only();
        let config = config_with(rustls::RootCertStore::empty(), cache.clone());
        cache.set_kx_hint(server(), NamedGroup::X25519);
        assert_eq!(cache.kx_hint(&server()), None);
        for _ in 0..3 {
            assert!(client_hello_len(&config) > PQ_CLIENT_HELLO_MIN);
        }
    }

    #[test]
    fn the_shared_config_is_built_once() {
        assert!(Arc::ptr_eq(&tls_config(), &tls_config()));
    }
}
