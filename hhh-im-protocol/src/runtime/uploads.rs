//! 上传攒齐（protocol-v1 §9.2）：宿主发附件前按 `uploads/begin` → `uploads/chunk`（一段接一段）→ `uploads/finish`
//! 交来字节。运行时在内存里攒齐、核对长度与 SHA-256，交给插件一整份（[`Upload`]），插件再整份交给平台。

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};
use sha2::Digest as _;

use crate::budget::RESOURCE_SEGMENT_MAX_BYTES;
use crate::method::{
    UploadsAbortParams, UploadsBeginParams, UploadsChunkParams, UploadsFinishParams,
};
use crate::rpc::{ErrorKind, RpcError};
use crate::types::ResourceKind;

/// 同时进行的上传最多几个（每个都在内存里攒字节）。
const UPLOADS_MAX: usize = 4;

/// 攒齐、核对过的一个上传。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    /// 种类（图片、文件）。
    pub kind: ResourceKind,
    /// 文件名。
    pub name: String,
    /// 全部字节。
    pub data: Vec<u8>,
}

struct Pending {
    kind: ResourceKind,
    name: String,
    bytes: u64,
    data: Vec<u8>,
}

/// 每种附件最多多少字节；`None` 是这种不能上传。
type Limit = Box<dyn Fn(ResourceKind) -> Option<u64> + Send + Sync>;

/// 正在攒的上传（一个插件进程一份，放在插件的状态里）。
pub struct Uploads {
    pending: Mutex<HashMap<String, Pending>>,
    next: AtomicU64,
    limit: Limit,
    too_big: String,
}

impl std::fmt::Debug for Uploads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Uploads")
    }
}

fn error(kind: ErrorKind, message: impl Into<String>) -> RpcError {
    RpcError::from_kind(kind, message)
}

impl Uploads {
    /// `limit` 给每种附件的上限（字节，同平台的发送上限；`None` 是这种不能上传），`too_big` 是超过上限时给用户看的话。
    ///
    /// # Examples
    ///
    /// ```
    /// use hhh_im_protocol::method::{UploadsBeginParams, UploadsChunkParams, UploadsFinishParams};
    /// use hhh_im_protocol::runtime::Uploads;
    /// use hhh_im_protocol::types::ResourceKind;
    ///
    /// let uploads = Uploads::new(|kind| (kind != ResourceKind::Sticker).then_some(10), "附件须在 10 字节以内。");
    /// let begun = uploads.begin(UploadsBeginParams { kind: ResourceKind::File, name: "a.txt".into(), bytes: 2 }).unwrap();
    /// let id = begun["uploadId"].as_str().unwrap().to_owned();
    /// uploads.chunk(UploadsChunkParams { upload_id: id.clone(), offset: 0, data: "aGk=".into() }).unwrap();
    /// // "hi" 的 SHA-256。
    /// let sha256 = "8f434346648f6b96df89dda901c5176b10a6d83961dd3c1ac88b59b2dc327aa4".into();
    /// let upload = uploads.finish(UploadsFinishParams { upload_id: id, sha256 }).unwrap();
    /// assert_eq!(upload.data, b"hi");
    /// ```
    pub fn new(
        limit: impl Fn(ResourceKind) -> Option<u64> + Send + Sync + 'static,
        too_big: impl Into<String>,
    ) -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            limit: Box::new(limit),
            too_big: too_big.into(),
        }
    }

    /// `uploads/begin`：核对种类与大小，开一个上传，回 `{uploadId}`。
    ///
    /// # Errors
    /// 这种不能上传（`unsupported`）、为空或超过上限（`limit_exceeded`）、同时上传的太多（`rate_limited`）。
    pub fn begin(&self, params: UploadsBeginParams) -> Result<Value, RpcError> {
        let Some(max) = (self.limit)(params.kind) else {
            return Err(error(ErrorKind::Unsupported, "这种附件不能上传。"));
        };
        if params.bytes == 0 || params.bytes > max {
            return Err(error(ErrorKind::LimitExceeded, self.too_big.clone()));
        }
        let mut pending = self.pending.lock().expect("上传表锁中毒");
        if pending.len() >= UPLOADS_MAX {
            return Err(error(
                ErrorKind::RateLimited,
                "同时上传的附件太多，请稍后再试。",
            ));
        }
        let id = format!("upload-{}", self.next.fetch_add(1, Ordering::SeqCst));
        pending.insert(
            id.clone(),
            Pending {
                kind: params.kind,
                name: params.name,
                bytes: params.bytes,
                data: Vec::new(),
            },
        );
        Ok(json!({ "uploadId": id }))
    }

    /// `uploads/chunk`：按顺序接上一段，回 `{}`。
    ///
    /// # Errors
    /// 不是合法的 base64、没有这个上传、没接着上一段、比说好的长。
    pub fn chunk(&self, params: UploadsChunkParams) -> Result<Value, RpcError> {
        let bytes = BASE64
            .decode(params.data.as_bytes())
            .map_err(|_| error(ErrorKind::InvalidRequest, "上传的这一段不是合法的 base64。"))?;
        let mut pending = self.pending.lock().expect("上传表锁中毒");
        let upload = pending
            .get_mut(&params.upload_id)
            .ok_or_else(|| error(ErrorKind::NotFound, "没有这个上传，请重新上传。"))?;
        if params.offset != upload.data.len() as u64 {
            return Err(error(
                ErrorKind::InvalidRequest,
                "上传的这一段没有接着上一段。",
            ));
        }
        if bytes.len() as u64 > RESOURCE_SEGMENT_MAX_BYTES
            || upload.data.len() as u64 + bytes.len() as u64 > upload.bytes
        {
            return Err(error(ErrorKind::LimitExceeded, "上传的内容比说好的长。"));
        }
        upload.data.extend_from_slice(&bytes);
        // 剖析开着时数收到了多少字节（契约 provider-protocol v32 §4）。
        super::perf::count_current(super::perf::UPLOAD_BYTES, bytes.len() as u64);
        Ok(json!({}))
    }

    /// `uploads/finish`：核对长度与 SHA-256，交出整份（插件交给平台后回 `{key}`）。
    ///
    /// # Errors
    /// 没有这个上传、比说好的短、摘要对不上。
    pub fn finish(&self, params: UploadsFinishParams) -> Result<Upload, RpcError> {
        let upload = self
            .pending
            .lock()
            .expect("上传表锁中毒")
            .remove(&params.upload_id)
            .ok_or_else(|| error(ErrorKind::NotFound, "没有这个上传，请重新上传。"))?;
        if upload.data.len() as u64 != upload.bytes {
            return Err(error(ErrorKind::InvalidRequest, "上传的内容比说好的短。"));
        }
        let digest: String = sha2::Sha256::digest(&upload.data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if !digest.eq_ignore_ascii_case(params.sha256.trim()) {
            return Err(error(ErrorKind::InvalidRequest, "上传的内容与摘要对不上。"));
        }
        Ok(Upload {
            kind: upload.kind,
            name: upload.name,
            data: upload.data,
        })
    }

    /// `uploads/abort`：丢掉这个上传，回 `{}`（没有也算成功）。
    pub fn abort(&self, params: UploadsAbortParams) -> Value {
        self.pending
            .lock()
            .expect("上传表锁中毒")
            .remove(&params.upload_id);
        json!({})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uploads() -> Uploads {
        Uploads::new(
            |kind| match kind {
                ResourceKind::Image => Some(4),
                ResourceKind::File => Some(8),
                ResourceKind::Sticker => None,
            },
            "图片须在 4 字节以内、文件须在 8 字节以内。",
        )
    }

    fn begin(uploads: &Uploads, kind: ResourceKind, bytes: u64) -> Result<String, RpcError> {
        uploads
            .begin(UploadsBeginParams {
                kind,
                name: "a".into(),
                bytes,
            })
            .map(|v| v["uploadId"].as_str().unwrap().to_owned())
    }

    fn kind_of(error: &RpcError) -> ErrorKind {
        error.data.as_ref().unwrap().kind
    }

    #[test]
    fn begin_checks_kind_size_and_concurrency() {
        let uploads = uploads();
        assert_eq!(
            kind_of(&begin(&uploads, ResourceKind::Sticker, 1).unwrap_err()),
            ErrorKind::Unsupported
        );
        let too_big = begin(&uploads, ResourceKind::Image, 5).unwrap_err();
        assert_eq!(kind_of(&too_big), ErrorKind::LimitExceeded);
        assert_eq!(
            too_big.data.as_ref().unwrap().user_message.as_deref(),
            Some("图片须在 4 字节以内、文件须在 8 字节以内。")
        );
        assert_eq!(
            kind_of(&begin(&uploads, ResourceKind::File, 0).unwrap_err()),
            ErrorKind::LimitExceeded
        );
        for _ in 0..UPLOADS_MAX {
            begin(&uploads, ResourceKind::File, 8).unwrap();
        }
        assert_eq!(
            kind_of(&begin(&uploads, ResourceKind::File, 8).unwrap_err()),
            ErrorKind::RateLimited
        );
    }

    #[test]
    fn chunks_must_follow_on_and_finish_checks_length_and_digest() {
        let uploads = uploads();
        let id = begin(&uploads, ResourceKind::File, 4).unwrap();
        let chunk = |offset, data: &[u8]| {
            uploads.chunk(UploadsChunkParams {
                upload_id: id.clone(),
                offset,
                data: BASE64.encode(data),
            })
        };
        chunk(0, b"ab").unwrap();
        assert!(chunk(1, b"c").is_err(), "没接着上一段");
        assert!(chunk(2, b"cde").is_err(), "比说好的长");
        let short = uploads.finish(UploadsFinishParams {
            upload_id: id.clone(),
            sha256: String::new(),
        });
        assert!(short.is_err(), "比说好的短");
        // 短了的已经丢掉，要重新开始。
        let id = begin(&uploads, ResourceKind::File, 2).unwrap();
        uploads
            .chunk(UploadsChunkParams {
                upload_id: id.clone(),
                offset: 0,
                data: BASE64.encode(b"hi"),
            })
            .unwrap();
        let wrong = uploads.finish(UploadsFinishParams {
            upload_id: id,
            sha256: "00".into(),
        });
        assert!(wrong.is_err(), "摘要对不上");
    }
}
