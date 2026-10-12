//! 资源分段（protocol-v1 §9.1）：宿主每次要一段（≤1 MiB），按顺序一段接一段来。
//!
//! 一个资源开一条读流（插件给：从某个位置读到末尾），下一段正好接着上一段时接着读这条流，不再向平台重开请求；
//! 位置对不上（宿主重试、换了位置）就重开。读流按资源记着，至多 [`STREAMS_MAX`] 条、闲置 [`STREAM_IDLE`] 就丢掉。
//! 平台只能整份读的（图片常这样），打开时从头读，运行时替你跳到要的位置。

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::budget::RESOURCE_SEGMENT_MAX_BYTES;
use crate::method::ResourceChunk;
use crate::rpc::RpcError;

/// 同时记着几条读流。
const STREAMS_MAX: usize = 8;
/// 读流闲置多久丢掉（连接闲久了可能已被中间设备断掉）。
const STREAM_IDLE: Duration = Duration::from_secs(60);

/// [`ByteStream::next_chunk`] 交回的：下一块字节（读完为 `None`）。
pub type NextChunk<'a> =
    Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, RpcError>> + Send + 'a>>;

/// 一条读流：一块一块地交出字节，读完交 `None`。平台的下载响应包一层就是。
pub trait ByteStream: Send {
    /// 下一块字节；读完为 `None`。
    fn next_chunk(&mut self) -> NextChunk<'_>;
}

/// 刚打开的一条读流。
pub struct Opened {
    /// 读流本身。
    pub stream: Box<dyn ByteStream>,
    /// 读流的第一个字节在整个资源里的位置：平台能从中间读时是要的位置，只能整份读时是 0（运行时跳过前面的）。
    pub start: u64,
    /// 整个资源的字节数；不知道为空。
    pub total: Option<u64>,
    /// 整个资源的 SHA-256（十六进制）；平台给了就给。
    pub sha256: Option<String>,
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opened")
            .field("start", &self.start)
            .field("total", &self.total)
            .finish_non_exhaustive()
    }
}

/// 一条正在读的资源流。
struct Stream {
    /// 是哪个资源（插件给的键）。
    key: String,
    /// 下一个字节在整个资源里的位置。
    position: u64,
    stream: Box<dyn ByteStream>,
    /// 读出来还没交出去的字节。
    buffer: Vec<u8>,
    total: Option<u64>,
    sha256: Option<String>,
    ended: bool,
    used: Instant,
}

impl Stream {
    /// 再读一块进缓冲；读完了记下。
    async fn fill(&mut self) -> Result<(), RpcError> {
        match self.stream.next_chunk().await? {
            Some(chunk) => self.buffer.extend_from_slice(&chunk),
            None => self.ended = true,
        }
        Ok(())
    }

    /// 跳过前面的字节，直到 `offset`（刚打开、只能整份读时）。
    async fn skip_to(&mut self, offset: u64) -> Result<(), RpcError> {
        let mut left = usize::try_from(offset.saturating_sub(self.position)).unwrap_or(usize::MAX);
        while left > 0 && !self.ended {
            if self.buffer.is_empty() {
                self.fill().await?;
            }
            let drop = left.min(self.buffer.len());
            self.buffer.drain(..drop);
            left -= drop;
        }
        self.position = offset;
        Ok(())
    }

    /// 交出一段：攒够 `length` 字节或读到末尾；能判断是不是末尾就判断。
    async fn segment(&mut self, length: u64) -> Result<ResourceChunk, RpcError> {
        let length = usize::try_from(length.min(RESOURCE_SEGMENT_MAX_BYTES)).unwrap_or(usize::MAX);
        while self.buffer.len() < length && !self.ended {
            self.fill().await?;
        }
        let take = length.min(self.buffer.len());
        let data: Vec<u8> = self.buffer.drain(..take).collect();
        self.position += data.len() as u64;
        self.used = Instant::now();
        // 不知道总长、这一段刚好读满时，再看一眼后面还有没有（有就留着下一段用）。
        if self.buffer.is_empty() && !self.ended && self.total.is_none() {
            self.fill().await?;
        }
        let eof = (self.ended && self.buffer.is_empty())
            || self.total.is_some_and(|total| self.position >= total);
        Ok(ResourceChunk {
            data: BASE64.encode(&data),
            total: self.total,
            sha256: self.sha256.clone(),
            eof,
        })
    }
}

/// 记着的读流（一个插件进程一份，放在插件的状态里）。
#[derive(Default)]
pub struct Segments {
    streams: Mutex<Vec<Stream>>,
}

impl std::fmt::Debug for Segments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Segments")
    }
}

impl Segments {
    /// 交出资源 `key` 从 `offset` 起的一段（`resources/read`、`avatars/read` 都用它）：有接着这个位置的读流就接着读，
    /// 否则用 `open(offset)` 开一条新的。`key` 由插件定，同一个资源要一样（如会话、消息、资源键拼起来）。
    ///
    /// # Errors
    /// 打开或读的时候平台出错，原样交回（这条读流丢掉，宿主重试时重开）。
    ///
    /// # Examples
    ///
    /// ```
    /// use hhh_im_protocol::runtime::{ByteStream, NextChunk, Opened, Segments};
    ///
    /// // 一个从内存里一次交出全部字节的读流。
    /// struct Once(Option<Vec<u8>>);
    /// impl ByteStream for Once {
    ///     fn next_chunk(&mut self) -> NextChunk<'_> {
    ///         let next = self.0.take();
    ///         Box::pin(async move { Ok(next) })
    ///     }
    /// }
    ///
    /// # #[tokio::main(flavor = "current_thread")]
    /// # async fn main() {
    /// let segments = Segments::default();
    /// let open = |_offset| async { Ok(Opened { stream: Box::new(Once(Some(b"hello".to_vec()))), start: 0, total: Some(5), sha256: None }) };
    /// let first = segments.read("doc", 0, 3, open).await.unwrap();
    /// assert!(!first.eof);
    /// // 下一段接着读同一条流：这次的打开不会被调用。
    /// let second = segments.read("doc", 3, 3, |_| async { unreachable!() }).await.unwrap();
    /// assert!(second.eof);
    /// # }
    /// ```
    pub async fn read<F, Fut>(
        &self,
        key: impl Into<String>,
        offset: u64,
        length: u64,
        open: F,
    ) -> Result<ResourceChunk, RpcError>
    where
        F: FnOnce(u64) -> Fut,
        Fut: Future<Output = Result<Opened, RpcError>>,
    {
        let key = key.into();
        let mut stream = match self.take(&key, offset) {
            Some(stream) => stream,
            None => {
                let opened = open(offset).await?;
                let mut stream = Stream {
                    key,
                    position: opened.start.min(offset),
                    stream: opened.stream,
                    buffer: Vec::new(),
                    total: opened.total,
                    sha256: opened.sha256,
                    ended: false,
                    used: Instant::now(),
                };
                stream.skip_to(offset).await?;
                stream
            }
        };
        let chunk = stream.segment(length).await?;
        if !chunk.eof {
            self.keep(stream);
        }
        Ok(chunk)
    }

    /// 取出接着这个位置的读流（取出来读，读完再放回）。
    fn take(&self, key: &str, offset: u64) -> Option<Stream> {
        let mut streams = self.streams.lock().expect("读流表锁中毒");
        let index = streams
            .iter()
            .position(|stream| stream.key == key && stream.position == offset)?;
        Some(streams.swap_remove(index))
    }

    /// 放回还没读完的读流；丢掉闲置太久的、同一个资源的旧流，超过上限时丢最旧的。
    fn keep(&self, stream: Stream) {
        let mut streams = self.streams.lock().expect("读流表锁中毒");
        streams.retain(|old| old.used.elapsed() < STREAM_IDLE && old.key != stream.key);
        if streams.len() >= STREAMS_MAX
            && let Some(oldest) = streams
                .iter()
                .enumerate()
                .min_by_key(|(_, stream)| stream.used)
                .map(|(index, _)| index)
        {
            streams.swap_remove(oldest);
        }
        streams.push(stream);
    }
}

/// 整份已经在内存里的字节（表情图这类小东西）交出从 `offset` 起的一段。
///
/// # Examples
///
/// ```
/// let chunk = hhh_im_protocol::runtime::chunk_of_bytes(b"abcdef", 4, 10);
/// assert!(chunk.eof);
/// assert_eq!(chunk.total, Some(6));
/// ```
pub fn chunk_of_bytes(bytes: &[u8], offset: u64, length: u64) -> ResourceChunk {
    let start = usize::try_from(offset)
        .unwrap_or(usize::MAX)
        .min(bytes.len());
    let length = usize::try_from(length.min(RESOURCE_SEGMENT_MAX_BYTES)).unwrap_or(usize::MAX);
    let end = start.saturating_add(length).min(bytes.len());
    ResourceChunk {
        data: BASE64.encode(&bytes[start..end]),
        total: Some(bytes.len() as u64),
        sha256: None,
        eof: end >= bytes.len(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// 从内存里按块交出的读流（每块 `piece` 字节），可以中途出错一次。
    struct Pieces {
        data: Vec<u8>,
        at: usize,
        piece: usize,
        fail_at: Option<usize>,
    }

    impl ByteStream for Pieces {
        fn next_chunk(&mut self) -> NextChunk<'_> {
            Box::pin(async move {
                if self.fail_at.is_some_and(|at| self.at >= at) {
                    self.fail_at = None;
                    return Err(RpcError::from_kind(crate::ErrorKind::Disconnected, "断了"));
                }
                if self.at >= self.data.len() {
                    return Ok(None);
                }
                let end = (self.at + self.piece).min(self.data.len());
                let chunk = self.data[self.at..end].to_vec();
                self.at = end;
                Ok(Some(chunk))
            })
        }
    }

    fn data(size: usize) -> Vec<u8> {
        (0..size).map(|i| (i % 251) as u8).collect()
    }

    fn decode(chunk: &ResourceChunk) -> Vec<u8> {
        BASE64.decode(&chunk.data).unwrap()
    }

    // 顺着读：只开一次读流；位置对不上（宿主重试前一段）时重开；平台能从中间读时从要的位置开。
    #[tokio::test]
    async fn sequential_segments_reuse_one_stream_and_mismatches_reopen() {
        let all = data(2_500);
        let opens = Arc::new(AtomicUsize::new(0));
        let segments = Segments::default();
        let open = |offset: u64| {
            let (all, opens) = (all.clone(), opens.clone());
            async move {
                opens.fetch_add(1, Ordering::SeqCst);
                let at = usize::try_from(offset).unwrap();
                Ok(Opened {
                    stream: Box::new(Pieces {
                        data: all,
                        at,
                        piece: 300,
                        fail_at: None,
                    }),
                    start: offset,
                    total: Some(2_500),
                    sha256: None,
                })
            }
        };
        let mut got = Vec::new();
        let mut offset = 0;
        loop {
            let chunk = segments.read("r", offset, 1_000, open).await.unwrap();
            got.extend(decode(&chunk));
            offset = got.len() as u64;
            if chunk.eof {
                break;
            }
        }
        assert_eq!(got, all);
        assert_eq!(opens.load(Ordering::SeqCst), 1, "顺着读只开一次");
        let again = segments.read("r", 1_000, 500, open).await.unwrap();
        assert_eq!(decode(&again), all[1_000..1_500]);
        assert_eq!(
            opens.load(Ordering::SeqCst),
            2,
            "读完的流不留，从中间要时重开"
        );
    }

    // 只能整份读的：从头开，跳到要的位置交出。
    #[tokio::test]
    async fn whole_only_streams_skip_to_the_offset() {
        let all = data(1_000);
        let segments = Segments::default();
        let chunk = segments
            .read("img", 600, 1_000, |_| {
                let all = all.clone();
                async move {
                    Ok(Opened {
                        stream: Box::new(Pieces {
                            data: all,
                            at: 0,
                            piece: 128,
                            fail_at: None,
                        }),
                        start: 0,
                        total: None,
                        sha256: Some("abc".into()),
                    })
                }
            })
            .await
            .unwrap();
        assert_eq!(decode(&chunk), all[600..]);
        assert!(chunk.eof, "不知道总长也认得出末尾");
        assert_eq!(chunk.sha256.as_deref(), Some("abc"));
    }

    // 读的时候平台出错：错误交回，这条流不留，下次从同一位置要时重开。
    #[tokio::test]
    async fn errors_drop_the_stream() {
        let all = data(3_000);
        let segments = Segments::default();
        let first = segments
            .read("f", 0, 1_000, |_| {
                let all = all.clone();
                async move {
                    Ok(Opened {
                        stream: Box::new(Pieces {
                            data: all,
                            at: 0,
                            piece: 500,
                            fail_at: Some(1_500),
                        }),
                        start: 0,
                        total: Some(3_000),
                        sha256: None,
                    })
                }
            })
            .await
            .unwrap();
        assert_eq!(decode(&first).len(), 1_000);
        let failed = segments
            .read("f", 1_000, 1_000, |_| async { unreachable!() })
            .await;
        assert!(failed.is_err());
        let reopened = segments
            .read("f", 1_000, 1_000, |offset| {
                let all = all.clone();
                async move {
                    Ok(Opened {
                        stream: Box::new(Pieces {
                            data: all,
                            at: usize::try_from(offset).unwrap(),
                            piece: 500,
                            fail_at: None,
                        }),
                        start: offset,
                        total: Some(3_000),
                        sha256: None,
                    })
                }
            })
            .await
            .unwrap();
        assert_eq!(decode(&reopened), all[1_000..2_000]);
    }

    #[test]
    fn bytes_in_memory_are_cut_into_segments() {
        let chunk = chunk_of_bytes(b"abcdef", 2, 2);
        assert_eq!(BASE64.decode(&chunk.data).unwrap(), b"cd");
        assert!(!chunk.eof);
        let past = chunk_of_bytes(b"abc", 10, 2);
        assert!(past.eof);
        assert!(BASE64.decode(&past.data).unwrap().is_empty());
    }
}
