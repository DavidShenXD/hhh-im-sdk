//! 宿主那一头（protocol-v1 §3）：所有发往标准输出的消息都排进一条队列、由一个写手按顺序写，一行一条、不会两条
//! 搅在一起；插件发给宿主的请求（`credential/save`、`events/push`…）按编号等回复。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::rpc::{ErrorKind, RpcError, RpcId, RpcMessage};

/// 等宿主回一个请求最多等多久（宿主写系统凭据库、落库一批推送通常不到一秒）。
const REPLY_WAIT: Duration = Duration::from_secs(30);

/// 写手队列里的一项。
#[derive(Debug)]
pub(super) enum Out {
    /// 写这一条。
    Message(Box<RpcMessage>),
    /// 前面的写完就停（输入读完了）。
    Close,
}

/// 写手要写的消息（[`Host::new`] 给的另一头，交给 [`super::serve_on`]）。
#[derive(Debug)]
pub struct Writes(pub(super) mpsc::UnboundedReceiver<Out>);

impl Writes {
    /// 取下一条要写的消息；写手停了（输入读完）或队列空着没人再写时为空。测试里当宿主读插件写出来的东西用。
    pub async fn next(&mut self) -> Option<RpcMessage> {
        match self.0.recv().await? {
            Out::Message(message) => Some(*message),
            Out::Close => None,
        }
    }
}

struct Inner {
    out: mpsc::UnboundedSender<Out>,
    /// 插件发出的请求的编号：字符串 `p1`、`p2`…，不会和宿主的数字编号撞号（rpc 模块的约定）。
    next: AtomicU64,
    waiting: Mutex<HashMap<String, oneshot::Sender<Result<Value, RpcError>>>>,
}

/// 宿主那一头：回请求、发通知、发请求并等回复。可以随意复制，复制出来的是同一个。
#[derive(Clone)]
pub struct Host(Arc<Inner>);

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Host")
    }
}

impl Host {
    /// 一个新的宿主那一头；写手从返回的 [`Writes`] 取消息写出去。
    pub fn new() -> (Self, Writes) {
        let (out, rx) = mpsc::unbounded_channel();
        (
            Self(Arc::new(Inner {
                out,
                next: AtomicU64::new(1),
                waiting: Mutex::new(HashMap::new()),
            })),
            Writes(rx),
        )
    }

    fn send(&self, message: RpcMessage) {
        // 写手已经停了（标准输出关了）：宿主那头已经不在，丢掉就是。
        let _ = self.0.out.send(Out::Message(Box::new(message)));
    }

    /// 输入读完：排着的写完后写手停下，之后再发的丢掉。
    pub(super) fn close(&self) {
        let _ = self.0.out.send(Out::Close);
    }

    /// 回宿主的一个请求（[`super::Reply::Responded`] 时自己回）。
    pub fn respond(&self, id: RpcId, result: Result<Value, RpcError>) {
        self.send(match result {
            Ok(value) => RpcMessage::ok(id, value),
            Err(error) => RpcMessage::err(id, error),
        });
    }

    /// 发一个通知（不等回复），如 `login/step`、`status`、`session/expired`。
    pub fn notify(&self, method: &str, params: impl Serialize) {
        match serde_json::to_value(params) {
            Ok(params) => self.send(RpcMessage::notification(method, params)),
            Err(error) => tracing::error!(method, %error, "通知的参数写不出来，没发"),
        }
    }

    /// 发一个请求给宿主，等它回复（至多 30 秒），如 `credential/save`、`events/push`。
    ///
    /// # Errors
    /// 宿主回了错误、连接断了或等太久时返回错误（等太久与断了是 `disconnected`）。
    pub async fn request(&self, method: &str, params: impl Serialize) -> Result<Value, RpcError> {
        let params = serde_json::to_value(params).map_err(|error| {
            RpcError::from_kind(
                ErrorKind::ProtocolError,
                format!("发给宿主的参数写不出来：{error}"),
            )
        })?;
        let id = format!("p{}", self.0.next.fetch_add(1, Ordering::SeqCst));
        let (tx, rx) = oneshot::channel();
        self.0
            .waiting
            .lock()
            .expect("在等的请求表锁中毒")
            .insert(id.clone(), tx);
        self.send(RpcMessage::request(RpcId::Text(id.clone()), method, params));
        let reply = tokio::time::timeout(REPLY_WAIT, rx).await;
        self.0
            .waiting
            .lock()
            .expect("在等的请求表锁中毒")
            .remove(&id);
        match reply {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(RpcError::from_kind(
                ErrorKind::Disconnected,
                "和宿主的连接已经断了。",
            )),
            Err(_) => Err(RpcError::from_kind(
                ErrorKind::Disconnected,
                format!("宿主没有在 {} 秒内回复 {method}。", REPLY_WAIT.as_secs()),
            )),
        }
    }

    /// 宿主回了插件的某个请求：交给在等它的那一个；没人在等（等超时了）就丢掉。
    pub(super) fn answer(&self, message: RpcMessage) {
        let Some(id) = message.id.as_ref().map(ToString::to_string) else {
            return;
        };
        let waiting = self
            .0
            .waiting
            .lock()
            .expect("在等的请求表锁中毒")
            .remove(&id);
        let Some(waiting) = waiting else {
            tracing::debug!(%id, "宿主回了一个没人在等的请求，丢掉");
            return;
        };
        let result = match (message.result, message.error) {
            (_, Some(error)) => Err(error),
            (Some(result), None) => Ok(result),
            (None, None) => Ok(Value::Null),
        };
        let _ = waiting.send(result);
    }
}
