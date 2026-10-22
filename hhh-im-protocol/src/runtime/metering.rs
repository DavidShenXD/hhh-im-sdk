//! 平台请求计数（protocol-v1 §8 的 `platformRequests`，v14）：插件替宿主办一件事（一次宿主调用）时，数一共向平台
//! 发了几次请求；运行时办完随结果报给宿主，宿主按实际次数记进这个平台的读取额度。
//!
//! 声明 `readRequestGating` 的插件在读取出口等 [`read_request`]，得到宿主明确放行后再发 HTTP。
//! 额度与优先级仍由宿主决定，运行时只传递等待和取消。旧宿主没给读取凭据时仍按结果计数。
//! 旧的 [`request`] 只计数、不等待，供未声明此能力的插件使用。

use super::Host;
use crate::rpc::{ErrorKind, RpcError};
use serde_json::Value;
use std::cell::Cell;
use std::future::Future;

tokio::task_local! {
    // task_local（任务局部变量）：每个 tokio 任务各有一份，同时进行的几次调用各数各的、互不串。
    // `Cell` 是可以在只读借用里改的小格子；它只在这个任务里用，不跨线程共享。
    static SENT: Cell<u32>;
    static PERMIT: Option<(Host, Value)>;
}

/// 每次向平台发送读取请求前等待宿主额度，放行后才计数；没有协商额度时沿用计数模式。
///
/// # Examples
///
/// ```
/// use hhh_im_protocol::runtime::metering;
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() {
/// let (result, sent) = metering::counted(async {
///     metering::read_request().await?;
///     // 放行后再发送一条平台 HTTP 请求。
///     Ok::<_, hhh_im_protocol::RpcError>(())
/// }).await;
/// assert!(result.is_ok());
/// assert_eq!(sent, 1);
/// # }
/// ```
///
/// # Errors
/// 宿主取消了读取、连接断开、或没有明确放行时返回错误；调用方必须停止这一次平台请求。
pub async fn read_request() -> Result<(), RpcError> {
    if let Some((host, permit)) = PERMIT.try_with(Clone::clone).ok().flatten() {
        let permit = permit
            .as_str()
            .filter(|p| !p.is_empty() && p.len() <= 128)
            .ok_or_else(|| {
                RpcError::from_kind(ErrorKind::ProtocolError, "宿主的读取额度凭据无效。")
            })?;
        let granted = host.request_read_permit(permit).await?;
        if granted.get("granted").and_then(Value::as_bool) != Some(true) {
            return Err(RpcError::from_kind(
                ErrorKind::ProtocolError,
                "宿主没有明确放行这次平台读取。",
            ));
        }
    }
    request();
    Ok(())
}

pub(super) async fn counted_with_permit<F: Future>(
    future: F,
    host: Host,
    permit: Option<Value>,
) -> (F::Output, u32) {
    PERMIT
        .scope(permit.map(|permit| (host, permit)), counted(future))
        .await
}

/// 向平台发一次请求前调一下：在 [`counted`] 里时数一次，否则什么也不做。
pub fn request() {
    // `try_with`：这个任务里有计数器就用它；没有（不在数的任务里）时返回错误，这里不在乎。
    let _ = SENT.try_with(|sent| sent.set(sent.get().saturating_add(1)));
}

/// 跑一段调用，同时数它向平台发了几次请求。运行时已经替每个宿主请求包好，插件一般不用自己调。
///
/// # Examples
///
/// ```
/// use hhh_im_protocol::runtime::metering;
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() {
/// let (answer, sent) = metering::counted(async {
///     metering::request();
///     metering::request();
///     42
/// })
/// .await;
/// assert_eq!((answer, sent), (42, 2));
/// # }
/// ```
pub async fn counted<F: Future>(future: F) -> (F::Output, u32) {
    SENT.scope(Cell::new(0), async move {
        let output = future.await;
        (output, SENT.with(Cell::get))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    // 同时进行的两段调用各数各的；不在数的任务里发的请求不算进任何一段。
    #[tokio::test]
    async fn each_call_counts_only_its_own_requests() {
        request();
        let one = counted(async {
            request();
            tokio::task::yield_now().await;
            request();
        });
        let two = counted(async {
            request();
        });
        let ((_, a), (_, b)) = tokio::join!(one, two);
        assert_eq!((a, b), (2, 1));
    }
}
