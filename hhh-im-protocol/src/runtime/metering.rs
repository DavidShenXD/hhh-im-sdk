//! 平台请求计数（protocol-v1 §8 的 `platformRequests`，v14）：插件替宿主办一件事（一次宿主调用）时，数一共向平台
//! 发了几次请求；运行时办完随结果报给宿主，宿主按实际次数记进这个平台的读取额度。
//!
//! 插件在向平台发请求的出口每次调一下 [`request`] 就行；运行时把每个宿主请求都包在 [`counted`] 里。插件不排队、
//! 也不等额度——额度由宿主管——所以 [`request`] 永不阻塞、永不报错；不在数的任务里（插件自己开的后台任务）时什么也
//! 不做。

use std::cell::Cell;
use std::future::Future;

tokio::task_local! {
    // task_local（任务局部变量）：每个 tokio 任务各有一份，同时进行的几次调用各数各的、互不串。
    // `Cell` 是可以在只读借用里改的小格子；它只在这个任务里用，不跨线程共享。
    static SENT: Cell<u32>;
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
