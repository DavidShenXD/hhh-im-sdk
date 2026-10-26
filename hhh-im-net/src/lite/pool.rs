//! 连接池：按“协议 + 主机 + 端口”留着读完响应的空闲连接，下次同一处的请求直接用。
//!
//! 与 reqwest（hyper-util 的连接池）一致：响应体读完（或本来就没有体）就放回池子；调用方没读完就丢掉响应时，
//! 在后台等 hyper 把剩下的体读掉丢弃、连接空下来再放回，读不完的 hyper 关掉连接，不放回。
//! 取连接时池里没有、但有连接正在空下来，就边等它边新连，谁先好用谁（与 hyper-util 一样，少开连接）。
//! 空闲超过闲置时限（缺省 90 秒）的不再用；每处至多留 `max_idle_per_host` 条（缺省不限，0 = 不复用）。

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::Poll;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::Full;
use hyper::client::conn::http1::SendRequest;
use tokio::sync::Notify;
use tokio::time::Instant;

pub(crate) type Sender = SendRequest<Full<Bytes>>;

/// 连接去处：https 与否、主机、端口。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    pub(crate) https: bool,
    pub(crate) host: String,
    pub(crate) port: u16,
}

#[derive(Clone)]
pub(crate) struct Pool {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<State>,
    /// 有连接放回或不再等它时通知。
    returned: Notify,
    idle_timeout: Option<Duration>,
    max_idle_per_host: usize,
}

#[derive(Default)]
struct State {
    idle: HashMap<Key, Vec<Idle>>,
    /// 各处正在空下来（后台等它放回）的连接数。
    pending: HashMap<Key, usize>,
}

struct Idle {
    sender: Sender,
    since: Instant,
}

impl Pool {
    pub(crate) fn new(idle_timeout: Option<Duration>, max_idle_per_host: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State::default()),
                returned: Notify::new(),
                idle_timeout,
                max_idle_per_host,
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// 取一条还活着、没闲置过久的空闲连接（后放回的先用）。
    fn take(&self, key: &Key) -> Option<Sender> {
        let mut state = self.state();
        let list = state.idle.get_mut(key)?;
        list.retain(|entry| self.alive(entry));
        let entry = list.pop();
        if list.is_empty() {
            state.idle.remove(key);
        }
        entry.map(|entry| entry.sender)
    }

    fn has_pending(&self, key: &Key) -> bool {
        self.state()
            .pending
            .get(key)
            .is_some_and(|count| *count > 0)
    }

    /// 取一条能发请求的连接：池里有就用（等它就绪）；没有、但有连接正在空下来时，边等它边用 `connect`
    /// 新连，谁先好用谁；都没有就新连。返回连接与它是否复用来的。
    pub(crate) async fn checkout<F, E>(&self, key: &Key, connect: F) -> Result<(Sender, bool), E>
    where
        F: Future<Output = Result<Sender, E>>,
    {
        let mut connect = std::pin::pin!(connect);
        loop {
            let notified = self.inner.returned.notified();
            let mut notified = std::pin::pin!(notified);
            notified.as_mut().enable();
            while let Some(mut sender) = self.take(key) {
                if sender.ready().await.is_ok() {
                    return Ok((sender, true));
                }
            }
            if !self.has_pending(key) {
                return connect.as_mut().await.map(|sender| (sender, false));
            }
            let connected = std::future::poll_fn(|cx| {
                if notified.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(None);
                }
                connect.as_mut().poll(cx).map(Some)
            })
            .await;
            if let Some(result) = connected {
                return result.map(|sender| (sender, false));
            }
        }
    }

    /// 放回一条读完响应的连接。
    pub(crate) fn put(&self, key: Key, sender: Sender) {
        if self.inner.max_idle_per_host == 0 || sender.is_closed() {
            return;
        }
        {
            let mut state = self.state();
            let list = state.idle.entry(key).or_default();
            list.retain(|entry| self.alive(entry));
            if list.len() < self.inner.max_idle_per_host {
                list.push(Idle {
                    sender,
                    since: Instant::now(),
                });
            }
        }
        self.inner.returned.notify_waiters();
    }

    fn alive(&self, entry: &Idle) -> bool {
        !entry.sender.is_closed()
            && self
                .inner
                .idle_timeout
                .is_none_or(|limit| entry.since.elapsed() < limit)
    }
}

impl Pool {
    /// 还在收响应的连接：后台等它空下来再放回（等不到就丢掉；不在异步运行时里时直接丢掉）。
    pub(crate) fn put_when_idle(&self, key: Key, mut sender: Sender) {
        if self.inner.max_idle_per_host == 0 {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        *self.state().pending.entry(key.clone()).or_default() += 1;
        let pool = self.clone();
        runtime.spawn(async move {
            let ready = sender.ready().await.is_ok();
            {
                let mut state = pool.state();
                if let Some(count) = state.pending.get_mut(&key) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        state.pending.remove(&key);
                    }
                }
            }
            if ready {
                pool.put(key, sender);
            } else {
                pool.inner.returned.notify_waiters();
            }
        });
    }
}

/// 响应用着的连接：读完响应体时放回池子；响应没读完就丢掉时，等连接空下来再放回。
pub(crate) struct PoolReturn {
    slot: Option<(Pool, Key, Sender)>,
}

impl PoolReturn {
    pub(crate) fn new(pool: Pool, key: Key, sender: Sender) -> Self {
        Self {
            slot: Some((pool, key, sender)),
        }
    }

    /// 响应体读完了：放回。
    pub(crate) fn release(&mut self) {
        if let Some((pool, key, sender)) = self.slot.take() {
            pool.put(key, sender);
        }
    }
}

impl Drop for PoolReturn {
    fn drop(&mut self) {
        if let Some((pool, key, sender)) = self.slot.take() {
            pool.put_when_idle(key, sender);
        }
    }
}
