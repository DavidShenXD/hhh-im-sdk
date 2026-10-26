//! 按主机名直接开 TCP 连接：两边同一个入口。
//!
//! tokio 在 WASI 上没有后台线程，按主机名连接（`TcpStream::connect("host:port")`）要用阻塞线程查地址，用不了；
//! iPhone 上先查出地址（IP 写法直接用，主机名走标准库的解析，WASI 下由宿主回答），再按地址逐个连。
//! 电脑上查地址交给 tokio。查地址与连接分成两步，iPhone 版 HTTP 客户端据此分别报“查地址失败”与“连接失败”。

use std::io;
use std::net::{IpAddr, SocketAddr};

use tokio::net::TcpStream;

/// 连上 `host:port`：IP 直接连；主机名查出全部地址，按顺序连，第一个连上的为准。连上后关掉 Nagle（小包立即发出）。
///
/// 自己的协议（不走 HTTP）用它；要 TLS 时在它上面用 [`crate::tls_config`] 握手。
///
/// # Errors
///
/// 查不到地址，或所有地址都连不上时，返回最后一个错误。
///
/// # Examples
///
/// ```no_run
/// # async fn demo() -> std::io::Result<()> {
/// let stream = hhh_im_net::connect_tcp("example.com", 443).await?;
/// # drop(stream);
/// # Ok(())
/// # }
/// ```
pub async fn connect_tcp(host: &str, port: u16) -> io::Result<TcpStream> {
    let addresses = resolve(host, port).await?;
    connect_any(&addresses).await
}

/// 查出 `host:port` 的全部地址；IP 写法（含带方括号的 IPv6）直接用，不查。
pub(crate) async fn resolve(host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let addresses = lookup(host, port).await?;
    if addresses.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("查不到 {host} 的地址"),
        ));
    }
    Ok(addresses)
}

#[cfg(not(target_os = "wasi"))]
async fn lookup(host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
    Ok(tokio::net::lookup_host((host, port)).await?.collect())
}

#[cfg(target_os = "wasi")]
async fn lookup(host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
    use std::net::ToSocketAddrs as _;
    Ok((host, port).to_socket_addrs()?.collect())
}

/// 按顺序连这些地址，第一个连上的为准（关掉 Nagle）；都连不上时返回最后一个错误。
pub(crate) async fn connect_any(addresses: &[SocketAddr]) -> io::Result<TcpStream> {
    let mut last = None;
    for address in addresses {
        match TcpStream::connect(address).await {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "没有可连的地址")))
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    #[tokio::test]
    async fn connects_by_host_name_and_ip_and_moves_bytes() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut buffer = [0u8; 5];
                    socket.read_exact(&mut buffer).await.unwrap();
                    socket.write_all(&buffer).await.unwrap();
                });
            }
        });
        for host in ["localhost", "127.0.0.1"] {
            let mut stream = super::connect_tcp(host, port).await.unwrap();
            assert!(stream.nodelay().unwrap());
            stream.write_all(b"hello").await.unwrap();
            let mut echo = [0u8; 5];
            stream.read_exact(&mut echo).await.unwrap();
            assert_eq!(&echo, b"hello");
        }
    }

    #[tokio::test]
    async fn a_closed_port_is_an_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(super::connect_tcp("127.0.0.1", port).await.is_err());
    }

    #[tokio::test]
    async fn ip_literals_are_not_looked_up() {
        let addresses = super::resolve("[::1]", 8443).await.unwrap();
        assert_eq!(addresses, vec!["[::1]:8443".parse().unwrap()]);
        let addresses = super::resolve("127.0.0.1", 80).await.unwrap();
        assert_eq!(addresses, vec!["127.0.0.1:80".parse().unwrap()]);
    }
}
