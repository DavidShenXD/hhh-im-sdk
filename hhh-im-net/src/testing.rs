//! 测试用：信任测试根证书的客户端 TLS 设置（feature `test-client`，iPhone 版的测试程序也能用），
//! 以及本机 HTTP / HTTPS 测试服务器与几种“坏服务器”（feature `test-server`，只在电脑上）。
//!
//! 测试根证书与服务器私钥（`certs/`，CN=hhh-im-net test CA，服务器证书给 `localhost` 与 `127.0.0.1`，2126 年到期）
//! 是公开的测试材料，只让本机测试能握 TLS，不保护任何东西。
//!
//! 测试服务器记下收到的每个请求（方法、目标、请求头、请求体、第几条连接），按路径回不同的东西：
//!
//! | 路径 | 回应 |
//! |---|---|
//! | `/json` | `{"ok":true,"method":…,"query":…}` |
//! | `/echo` | 原样回请求体（类型照请求的） |
//! | `/status/{码}` | 那个状态码 |
//! | `/reason` | 499，原因短语 `Odd Thing` |
//! | `/redirect/{码}?to=…` | 那个状态码，`Location` 是 `to` |
//! | `/chain/{n}` | 连跳 n 次（302）再回 200 |
//! | `/gzip`、`/gzip-multi`、`/gzip-empty`、`/gzip-bad` | gzip 压缩的文字、两段 gzip、空体、坏数据 |
//! | `/chunks?n=&size=&gap_ms=` | 分 n 块回，每块 size 字节，块间隔 gap_ms |
//! | `/slow-headers?ms=` | 等 ms 再回 |
//! | `/slow-body?ms=` | 先回 `start`，等 ms 再回 `end` |
//! | `/bytes?size=` | size 字节（带长度） |
//! | `/close` | 回完关连接（`Connection: close`） |
//! | `/empty` | 204 |
//! | 其它 | 200 `ok` |

use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject as _;

#[cfg(any(test, feature = "test-server"))]
mod server;
#[cfg(any(test, feature = "test-server"))]
pub use server::*;

const CA_PEM: &[u8] = include_bytes!("testing/certs/ca.pem");

/// `/gzip` 解压后的文字。
pub const GZIP_TEXT: &str = "hello from gzip, hello from gzip, hello from gzip";

/// 与生产设置相同（抗量子优先、只缓存票据、ALPN http/1.1），只是根证书换成测试根证书。
pub fn client_tls() -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    let ca = CertificateDer::from_pem_slice(CA_PEM).expect("测试根证书是合法 PEM");
    roots.add(ca).expect("测试根证书能加进根证书表");
    crate::handshake::config_with(roots, crate::handshake::tickets_only())
}

/// 客户端发来的 TLS 握手请求（ClientHello）的样子。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    /// 整条记录的字节数（含 5 字节记录头）。
    pub len: usize,
    /// key_share 里带了哪些密钥交换组的份额（IANA 编号，如 X25519MLKEM768 = 0x11EC、X25519 = 0x001D）。
    pub key_shares: Vec<u16>,
    /// SNI 里的主机名（有的话）。
    pub server_name: Option<String>,
    /// 提供的密码套件（按顺序）。
    pub cipher_suites: Vec<u16>,
    /// 扩展的类型（按顺序）。
    pub extensions: Vec<u16>,
    /// ALPN 里报的协议。
    pub alpn: Vec<String>,
}

/// 抗量子混合组 X25519MLKEM768 的编号。
pub const X25519MLKEM768: u16 = 0x11ec;
/// X25519 的编号。
pub const X25519: u16 = 0x001d;
