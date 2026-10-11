//! 子进程 stdio 传输与连接层（protocol-v1 §3、§13）。
//!
//! - [`ProviderTransport`]：原始的「一行一条」收发抽象，便于测试注入进程内假插件。
//! - [`StdioChild`]：按清单启动子进程，走它的 stdin/stdout，把 stderr 收成日志。
//! - [`Connection`]：在传输之上做请求/响应按编号配对，并把插件主动发来的请求与通知交给宿主消费。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::rpc::{RpcError, RpcId, RpcMessage, code, codec};

/// 一个在途请求的回执通道：响应或错误。
type ReplyTx = oneshot::Sender<Result<serde_json::Value, RpcError>>;
/// 在途请求表：编号字符串 → 回执通道。
type Pending = Arc<Mutex<HashMap<String, ReplyTx>>>;

/// 原始的「一行一条」收发抽象。宿主侧用它承载子进程 stdio；测试用它注入进程内假插件。
pub trait ProviderTransport: Send + Sync + 'static {
    /// 发一行（一条 JSON-RPC 消息）。
    fn send(
        &self,
        msg: RpcMessage,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send + '_>>;
    /// 收下一行；传输关闭时返回 `None`。
    fn recv(&self) -> Pin<Box<dyn Future<Output = Option<RpcMessage>> + Send + '_>>;
}

/// 插件主动发来的一件事：请求（要回复）或通知（不回复）。
#[derive(Debug)]
pub enum Incoming {
    /// 插件发来的请求（如 `events/push`、`credential/save`），宿主要用 [`Connection::respond`] 回。
    Request {
        /// 请求编号。
        id: RpcId,
        /// 方法名。
        method: String,
        /// 参数。
        params: serde_json::Value,
    },
    /// 插件发来的通知（如 `login/step`、`status`、`log/write`）。
    Notification {
        /// 方法名。
        method: String,
        /// 参数。
        params: serde_json::Value,
    },
}

/// 连接层：请求/响应按编号配对，插件主动发来的交给 [`Connection::next_incoming`]。
pub struct Connection {
    transport: Arc<dyn ProviderTransport>,
    next_id: Mutex<i64>,
    pending: Pending,
    incoming_rx: Mutex<mpsc::UnboundedReceiver<Incoming>>,
}

impl Connection {
    /// 在一个传输之上建连接，并启动后台读取任务（把响应配对、把插件请求/通知转交）。
    pub fn new(transport: Arc<dyn ProviderTransport>) -> Arc<Self> {
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (incoming_tx, incoming_rx) = mpsc::unbounded_channel();
        let conn = Arc::new(Self {
            transport: transport.clone(),
            next_id: Mutex::new(1),
            pending: pending.clone(),
            incoming_rx: Mutex::new(incoming_rx),
        });
        tokio::spawn(read_loop(transport, pending, incoming_tx));
        conn
    }

    /// 发一个请求，等它的响应（不设超时，超时由调用方包）。
    ///
    /// # Errors
    /// 传输断开、或插件回了错误时返回 [`RpcError`]。
    pub async fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        let id = {
            let mut n = self.next_id.lock().await;
            let id = *n;
            *n += 1;
            id
        };
        let key = RpcId::Number(id).to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(key.clone(), tx);
        let msg = RpcMessage::request(RpcId::Number(id), method, params);
        if let Err(e) = self.transport.send(msg).await {
            self.pending.lock().await.remove(&key);
            return Err(transport_closed(&format!("发送失败：{e}")));
        }
        match rx.await {
            Ok(result) => result,
            Err(_) => Err(transport_closed("连接已关闭")),
        }
    }

    /// 发一个通知（不等回复）。
    ///
    /// # Errors
    /// 传输断开时返回错误。
    pub async fn notify(&self, method: &str, params: serde_json::Value) -> Result<(), RpcError> {
        self.transport
            .send(RpcMessage::notification(method, params))
            .await
            .map_err(|e| transport_closed(&format!("发送失败：{e}")))
    }

    /// 回复插件发来的请求（成功）。
    ///
    /// # Errors
    /// 传输断开时返回错误。
    pub async fn respond(&self, id: RpcId, result: serde_json::Value) -> Result<(), RpcError> {
        self.transport
            .send(RpcMessage::ok(id, result))
            .await
            .map_err(|e| transport_closed(&format!("发送失败：{e}")))
    }

    /// 回复插件发来的请求（错误）。
    ///
    /// # Errors
    /// 传输断开时返回错误。
    pub async fn respond_err(&self, id: RpcId, error: RpcError) -> Result<(), RpcError> {
        self.transport
            .send(RpcMessage::err(id, error))
            .await
            .map_err(|e| transport_closed(&format!("发送失败：{e}")))
    }

    /// 取插件主动发来的下一件事（请求或通知）；连接关闭后返回 `None`。
    pub async fn next_incoming(&self) -> Option<Incoming> {
        self.incoming_rx.lock().await.recv().await
    }
}

/// Windows 的 `CREATE_NO_WINDOW`：起插件进程不带控制台窗口。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn transport_closed(msg: &str) -> RpcError {
    RpcError {
        code: code::DISCONNECTED,
        message: "disconnected".into(),
        data: Some(crate::rpc::RpcErrorData {
            kind: crate::rpc::ErrorKind::Disconnected,
            user_message: Some(msg.to_owned()),
            platform_code: None,
            reauth: None,
            retry_after_secs: None,
        }),
    }
}

async fn read_loop(
    transport: Arc<dyn ProviderTransport>,
    pending: Pending,
    incoming_tx: mpsc::UnboundedSender<Incoming>,
) {
    while let Some(msg) = transport.recv().await {
        // 响应没有方法名；请求与通知有方法名，再按有无编号区分。一次性拆开 msg，避免移动后再借用。
        let RpcMessage {
            id,
            method,
            params,
            result,
            error,
            ..
        } = msg;
        match method {
            None => {
                // 响应：按编号配对唤醒。
                if let Some(id) = id {
                    let key = id.to_string();
                    if let Some(tx) = pending.lock().await.remove(&key) {
                        let outcome = match (result, error) {
                            (_, Some(err)) => Err(err),
                            (Some(v), None) => Ok(v),
                            (None, None) => Ok(serde_json::Value::Null),
                        };
                        let _ = tx.send(outcome);
                    }
                }
            }
            Some(method) => {
                let params = params.unwrap_or(serde_json::Value::Null);
                let event = match id {
                    Some(id) => Incoming::Request { id, method, params },
                    None => Incoming::Notification { method, params },
                };
                let _ = incoming_tx.send(event);
            }
        }
    }
    // 传输关闭：叫醒所有等待中的请求。
    let mut pend = pending.lock().await;
    for (_, tx) in pend.drain() {
        let _ = tx.send(Err(transport_closed("连接已关闭")));
    }
}

/// 标准输出连续这么多行不是合法消息，就当插件坏了：结束它、关掉连接（protocol-v1 §3）。
pub const INVALID_LINES_MAX: usize = 3;

/// 起插件进程的命令：三路管道、设 `HHH_IM_PROTOCOL=1`、工作目录为给定临时目录。
fn plugin_command(
    program: &str,
    args: &[String],
    cwd: &std::path::Path,
) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .env(crate::ENV_FLAG, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // 兜底：句柄真被丢弃时结束子进程。读输出的任务一直拿着句柄，平时靠宿主 `kill_now` / `shutdown` 结束它。
        .kill_on_drop(true);
    // Windows：按控制台程序编的插件（模板默认就是）被桌面宿主拉起时会弹出一个黑色控制台窗口；插件只经管道说话，
    // 不要窗口（CREATE_NO_WINDOW）。
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// 按清单启动子进程、走它 stdin/stdout 的传输（protocol-v1 §3）。stderr 每行交给回调（并入宿主日志）。
pub struct StdioChild {
    stdin: Mutex<tokio::process::ChildStdin>,
    stdout: Mutex<tokio::io::Lines<BufReader<tokio::process::ChildStdout>>>,
    child: Mutex<tokio::process::Child>,
    /// 连续读到的不合法行数（读到一条合法的就清零）。
    invalid: AtomicUsize,
    /// 因为连续乱输出被结束的（宿主据此说明它为什么断了）。
    garbled: AtomicBool,
}

impl StdioChild {
    /// 启动子进程：设 `HHH_IM_PROTOCOL=1`、工作目录为给定临时目录；stderr 每行交给 `on_log`。
    ///
    /// # Errors
    /// 进程起不来时返回 IO 错误。
    pub fn spawn(
        program: &str,
        args: &[String],
        cwd: &std::path::Path,
        on_log: impl Fn(String) + Send + 'static,
    ) -> std::io::Result<Arc<Self>> {
        let mut child = plugin_command(program, args, cwd).spawn()?;
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        // stderr → 日志（每行截断在调用方做）。
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                on_log(line);
            }
        });

        Ok(Arc::new(Self {
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(BufReader::new(stdout).lines()),
            child: Mutex::new(child),
            invalid: AtomicUsize::new(0),
            garbled: AtomicBool::new(false),
        }))
    }

    /// 把子进程借给 `f` 看一眼（起好后紧接着调，比如宿主要把它绑到自己的生命周期上）。
    /// 管道已归这个传输，借出的 `Child` 只用来读进程句柄、编号。
    pub async fn with_child<R>(&self, f: impl FnOnce(&tokio::process::Child) -> R) -> R {
        f(&*self.child.lock().await)
    }

    /// 是不是因为连续乱输出被结束的。
    pub fn garbled(&self) -> bool {
        self.garbled.load(Ordering::SeqCst)
    }

    /// 记一行不合法的输出；连续满 [`INVALID_LINES_MAX`] 行时返回真（该结束它了）。
    fn note_invalid(&self) -> bool {
        self.invalid.fetch_add(1, Ordering::SeqCst) + 1 >= INVALID_LINES_MAX
    }

    /// 结束子进程（关 stdin 后杀）。
    pub async fn shutdown(&self) {
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
    }

    /// 不等地叫子进程结束（同步，可在 `Drop` 里用）：宿主放下一个插件连接时调。读它输出的任务拿着这个句柄，
    /// 光放下句柄等不到 `kill_on_drop`，插件会一直跑着（Windows 上它的程序文件也删不掉）。
    /// 正有别人在 [`Self::shutdown`] 时什么也不做（那边会杀）。
    pub fn kill_now(&self) {
        if let Ok(mut child) = self.child.try_lock() {
            let _ = child.start_kill();
        }
    }
}

impl ProviderTransport for StdioChild {
    fn send(
        &self,
        msg: RpcMessage,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let line = codec::encode_line(&msg)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await
        })
    }

    fn recv(&self) -> Pin<Box<dyn Future<Output = Option<RpcMessage>> + Send + '_>> {
        Box::pin(async move {
            let mut lines = self.stdout.lock().await;
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match codec::decode_line(trimmed) {
                            Ok(msg) => {
                                self.invalid.store(0, Ordering::SeqCst);
                                return Some(msg);
                            }
                            Err(e) => {
                                // 不是合法 JSON-RPC：记日志、跳过这一行；连续满 3 行就结束它、关掉连接。
                                tracing::warn!(error = %e, "插件 stdout 一行不是合法 JSON-RPC");
                                if self.note_invalid() {
                                    tracing::warn!(
                                        lines = INVALID_LINES_MAX,
                                        "插件连续输出不合法的内容，结束它"
                                    );
                                    self.garbled.store(true, Ordering::SeqCst);
                                    self.kill_now();
                                    return None;
                                }
                                continue;
                            }
                        }
                    }
                    Ok(None) => return None,
                    Err(e) => {
                        tracing::warn!(error = %e, "读插件 stdout 失败");
                        return None;
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 进程内假传输：两端用通道接起来，测试连接层。
    struct Loopback {
        to_peer: mpsc::UnboundedSender<RpcMessage>,
        from_peer: Mutex<mpsc::UnboundedReceiver<RpcMessage>>,
    }

    fn pair() -> (Arc<Loopback>, Arc<Loopback>) {
        let (a_tx, a_rx) = mpsc::unbounded_channel();
        let (b_tx, b_rx) = mpsc::unbounded_channel();
        let host = Arc::new(Loopback {
            to_peer: a_tx,
            from_peer: Mutex::new(b_rx),
        });
        let plugin = Arc::new(Loopback {
            to_peer: b_tx,
            from_peer: Mutex::new(a_rx),
        });
        (host, plugin)
    }

    impl ProviderTransport for Loopback {
        fn send(
            &self,
            msg: RpcMessage,
        ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send + '_>> {
            let _ = self.to_peer.send(msg);
            Box::pin(async { Ok(()) })
        }
        fn recv(&self) -> Pin<Box<dyn Future<Output = Option<RpcMessage>> + Send + '_>> {
            Box::pin(async move { self.from_peer.lock().await.recv().await })
        }
    }

    #[tokio::test]
    async fn a_call_gets_its_matching_response() {
        let (host_t, plugin_t) = pair();
        let conn = Connection::new(host_t);

        // 假插件：收到请求就回一个结果。
        let plugin = plugin_t.clone();
        tokio::spawn(async move {
            while let Some(msg) = plugin.recv().await {
                if msg.is_request() {
                    let id = msg.id.clone().unwrap();
                    let _ = plugin
                        .send(RpcMessage::ok(id, serde_json::json!({"pong": true})))
                        .await;
                }
            }
        });

        let out = conn.call("ping", serde_json::json!({})).await.unwrap();
        assert_eq!(out["pong"], true);
    }

    #[tokio::test]
    async fn plugin_requests_and_notifications_reach_incoming() {
        let (host_t, plugin_t) = pair();
        let conn = Connection::new(host_t);

        plugin_t
            .send(RpcMessage::notification(
                "log/write",
                serde_json::json!({"level": "info", "message": "hi"}),
            ))
            .await
            .unwrap();
        plugin_t
            .send(RpcMessage::request(
                RpcId::Text("p1".into()),
                "events/push",
                serde_json::json!({"events": []}),
            ))
            .await
            .unwrap();

        let first = conn.next_incoming().await.unwrap();
        match first {
            Incoming::Notification { method, .. } => assert_eq!(method, "log/write"),
            other => panic!("期望通知，得到 {other:?}"),
        }
        let second = conn.next_incoming().await.unwrap();
        match second {
            Incoming::Request { method, id, .. } => {
                assert_eq!(method, "events/push");
                conn.respond(id, serde_json::json!({})).await.unwrap();
            }
            other => panic!("期望请求，得到 {other:?}"),
        }
    }

    /// 连接的读取任务一直拿着子进程句柄，所以宿主得主动 `kill_now`：叫完插件进程就退出、它的输出关掉，
    /// 等着的请求被叫醒（不会留下一个没人管的插件进程）。
    #[tokio::test]
    async fn killing_the_child_closes_its_connection() {
        let (program, args): (&str, Vec<String>) = if cfg!(windows) {
            ("ping", vec!["-n".into(), "60".into(), "127.0.0.1".into()])
        } else {
            ("sleep", vec!["60".into()])
        };
        let dir = std::env::temp_dir();
        let child = StdioChild::spawn(program, &args, &dir, |_| {}).unwrap();
        let conn = Connection::new(child.clone());
        let waiting = tokio::spawn(async move { conn.call("ping", serde_json::json!({})).await });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        child.kill_now();
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), waiting)
            .await
            .expect("叫停后连接应在几秒内关掉")
            .unwrap();
        assert_eq!(outcome.unwrap_err().code, code::DISCONNECTED);
    }

    /// 插件标准输出连续 3 行不是合法消息：结束它、连接关掉，并记下是乱输出（宿主据此说明原因）。
    #[tokio::test]
    async fn three_invalid_lines_in_a_row_end_the_child() {
        let (program, args): (&str, Vec<String>) = if cfg!(windows) {
            (
                "powershell",
                vec![
                    "-NoProfile".into(),
                    "-Command".into(),
                    "'a'; 'b'; 'c'; Start-Sleep -Seconds 60".into(),
                ],
            )
        } else {
            (
                "sh",
                vec!["-c".into(), "echo a; echo b; echo c; sleep 60".into()],
            )
        };
        let child = StdioChild::spawn(program, &args, &std::env::temp_dir(), |_| {}).unwrap();
        let ended = tokio::time::timeout(std::time::Duration::from_secs(20), child.recv())
            .await
            .expect("连续乱输出后连接应关掉");
        assert!(ended.is_none());
        assert!(child.garbled());
    }

    #[tokio::test]
    async fn a_closed_transport_wakes_pending_calls() {
        let (host_t, plugin_t) = pair();
        let conn = Connection::new(host_t);
        // 丢掉插件端，使读取循环结束。
        drop(plugin_t);
        let err = conn.call("ping", serde_json::json!({})).await.unwrap_err();
        assert_eq!(err.code, code::DISCONNECTED);
    }
}
