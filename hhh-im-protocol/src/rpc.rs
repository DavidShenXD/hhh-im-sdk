//! JSON-RPC 2.0 信封与一行一条的编解码（protocol-v1 §3、§12）。

use serde::{Deserialize, Serialize};

/// JSON-RPC 请求/通知的编号：数字或字符串。通知没有编号。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcId {
    /// 数字编号。
    Number(i64),
    /// 字符串编号（插件发起的请求用它，避免与宿主的数字编号撞号）。
    Text(String),
}

impl std::fmt::Display for RpcId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcId::Number(n) => write!(f, "{n}"),
            RpcId::Text(s) => write!(f, "{s}"),
        }
    }
}

/// 一条 JSON-RPC 消息：请求、成功响应、错误响应或通知。
///
/// 用 `jsonrpc` 字段固定为 `"2.0"`；`id` 有无区分请求与通知；`result`/`error` 二选一区分响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcMessage {
    /// 固定 `"2.0"`。
    pub jsonrpc: String,
    /// 请求与响应有编号，通知没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<RpcId>,
    /// 方法名：请求与通知有，响应没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// 参数：请求与通知按方法带。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    /// 成功响应的结果。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// 错误响应。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl RpcMessage {
    /// 组一个请求。
    pub fn request(id: RpcId, method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id: Some(id),
            method: Some(method.into()),
            params: Some(params),
            result: None,
            error: None,
        }
    }

    /// 组一个通知（没有编号，不等回复）。
    pub fn notification(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id: None,
            method: Some(method.into()),
            params: Some(params),
            result: None,
            error: None,
        }
    }

    /// 组一个成功响应。
    pub fn ok(id: RpcId, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id: Some(id),
            method: None,
            params: None,
            result: Some(result),
            error: None,
        }
    }

    /// 组一个错误响应。
    pub fn err(id: RpcId, error: RpcError) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id: Some(id),
            method: None,
            params: None,
            result: None,
            error: Some(error),
        }
    }

    /// 是不是请求（有编号且有方法名）。
    pub fn is_request(&self) -> bool {
        self.id.is_some() && self.method.is_some()
    }

    /// 是不是通知（没编号但有方法名）。
    pub fn is_notification(&self) -> bool {
        self.id.is_none() && self.method.is_some()
    }

    /// 是不是响应（有编号没方法名）。
    pub fn is_response(&self) -> bool {
        self.id.is_some() && self.method.is_none()
    }
}

/// JSON-RPC 错误对象。`code` 取 protocol-v1 §12 的码，`data` 带分类与给用户看的说明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    /// 错误码（§12）。
    pub code: i32,
    /// 简短机读说明（通常等于 `data.kind`）。
    pub message: String,
    /// 结构化错误数据。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<RpcErrorData>,
}

/// 错误的结构化数据（protocol-v1 §12）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcErrorData {
    /// 错误分类。
    pub kind: ErrorKind,
    /// 给用户看的说明（用宿主给的 locale 写，不含令牌）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_message: Option<String>,
    /// 平台原始错误码，原样记成文字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_code: Option<String>,
    /// 需要重新授权的原因。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reauth: Option<ReauthReason>,
    /// 限流还要等多少秒。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
}

/// 错误分类（protocol-v1 §12）：与 hhh-im `ErrorKind` 一一对应，由适配器翻译。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    /// 平台不支持这件事。
    Unsupported,
    /// 没有权限。
    PermissionDenied,
    /// 授权失效，要重新登录。
    AuthExpired,
    /// 限流。
    RateLimited,
    /// 平台明确说没有。
    NotFound,
    /// 参数不合法 / 平台拒绝参数。
    InvalidRequest,
    /// 暂时失败（网络、平台临时故障），按退避重试。
    Disconnected,
    /// 平台给了看不懂的数据。
    ProtocolError,
    /// 请求被取消。
    Cancelled,
    /// 超出平台限制（文件太大等）。
    LimitExceeded,
}

/// 重新授权的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReauthReason {
    /// 授权过期。
    Expired,
    /// 平台撤销了授权。
    Revoked,
    /// 凭据本身无效。
    CredentialInvalid,
}

/// JSON-RPC 标准错误码与本协议的扩展错误码（protocol-v1 §12）。
pub mod code {
    /// 解析错误（收到的不是合法 JSON）。
    pub const PARSE_ERROR: i32 = -32700;
    /// 请求不合法（不是合法的 JSON-RPC）。
    pub const INVALID_REQUEST: i32 = -32600;
    /// 方法不存在。
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// 参数不合法。
    pub const INVALID_PARAMS: i32 = -32602;
    /// 平台不支持。
    pub const UNSUPPORTED: i32 = 1001;
    /// 没有权限。
    pub const PERMISSION_DENIED: i32 = 1002;
    /// 授权失效。
    pub const AUTH_EXPIRED: i32 = 1003;
    /// 限流。
    pub const RATE_LIMITED: i32 = 1004;
    /// 平台说没有。
    pub const NOT_FOUND: i32 = 1005;
    /// 平台拒绝参数。
    pub const INVALID_REQUEST_PLATFORM: i32 = 1006;
    /// 暂时失败。
    pub const DISCONNECTED: i32 = 1007;
    /// 平台数据看不懂。
    pub const PROTOCOL_ERROR: i32 = 1008;
    /// 请求被取消。
    pub const CANCELLED: i32 = 1009;
    /// 超出平台限制。
    pub const LIMIT_EXCEEDED: i32 = 1010;
}

impl RpcError {
    /// 从错误分类组一个错误对象，用对应的扩展错误码。
    pub fn from_kind(kind: ErrorKind, user_message: impl Into<String>) -> Self {
        let c = match kind {
            ErrorKind::Unsupported => code::UNSUPPORTED,
            ErrorKind::PermissionDenied => code::PERMISSION_DENIED,
            ErrorKind::AuthExpired => code::AUTH_EXPIRED,
            ErrorKind::RateLimited => code::RATE_LIMITED,
            ErrorKind::NotFound => code::NOT_FOUND,
            ErrorKind::InvalidRequest => code::INVALID_REQUEST_PLATFORM,
            ErrorKind::Disconnected => code::DISCONNECTED,
            ErrorKind::ProtocolError => code::PROTOCOL_ERROR,
            ErrorKind::Cancelled => code::CANCELLED,
            ErrorKind::LimitExceeded => code::LIMIT_EXCEEDED,
        };
        let kind_name = serde_json::to_value(kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "error".into());
        Self {
            code: c,
            message: kind_name,
            data: Some(RpcErrorData {
                kind,
                user_message: Some(user_message.into()),
                platform_code: None,
                reauth: None,
                retry_after_secs: None,
            }),
        }
    }
}

/// 一行一条的编解码（protocol-v1 §3）：UTF-8、以 `\n` 结尾，消息内部不能有原始换行。
pub mod codec {
    use super::RpcMessage;

    /// 把一条消息编成一行（含结尾换行）。
    ///
    /// # Errors
    /// 序列化失败时返回错误（正常不会发生）。
    ///
    /// # Examples
    ///
    /// ```
    /// use hhh_im_protocol::rpc::{RpcMessage, RpcId, codec};
    /// let msg = RpcMessage::request(RpcId::Number(1), "ping", serde_json::json!({}));
    /// let line = codec::encode_line(&msg).unwrap();
    /// assert!(line.ends_with('\n'));
    /// assert!(!line[..line.len() - 1].contains('\n'));
    /// ```
    pub fn encode_line(msg: &RpcMessage) -> Result<String, serde_json::Error> {
        let mut s = serde_json::to_string(msg)?;
        s.push('\n');
        Ok(s)
    }

    /// 把一行解成一条消息（行里不含结尾换行也可以）。
    ///
    /// # Errors
    /// 不是合法 JSON-RPC 时返回错误。
    pub fn decode_line(line: &str) -> Result<RpcMessage, serde_json::Error> {
        serde_json::from_str(line.trim_end_matches(['\r', '\n']))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_notifications_and_responses_are_told_apart() {
        let req = RpcMessage::request(RpcId::Number(7), "conversations/get", serde_json::json!({}));
        assert!(req.is_request() && !req.is_notification() && !req.is_response());
        let note = RpcMessage::notification("log/write", serde_json::json!({}));
        assert!(note.is_notification() && !note.is_request() && !note.is_response());
        let ok = RpcMessage::ok(RpcId::Number(7), serde_json::json!({}));
        assert!(ok.is_response() && !ok.is_request());
    }

    #[test]
    fn a_line_round_trips_and_carries_no_inner_newline() {
        let msg = RpcMessage::request(
            RpcId::Text("p1".into()),
            "events/push",
            serde_json::json!({"events": [], "syncState": "abc\ndef"}),
        );
        let line = codec::encode_line(&msg).unwrap();
        assert_eq!(line.matches('\n').count(), 1, "只有结尾一个换行");
        let back = codec::decode_line(&line).unwrap();
        assert_eq!(back.method.as_deref(), Some("events/push"));
        assert_eq!(back.id, Some(RpcId::Text("p1".into())));
    }

    #[test]
    fn an_error_from_a_kind_keeps_its_code_and_data() {
        let e = RpcError::from_kind(ErrorKind::AuthExpired, "登录已失效");
        assert_eq!(e.code, code::AUTH_EXPIRED);
        assert_eq!(e.message, "authExpired");
        assert_eq!(e.data.unwrap().kind, ErrorKind::AuthExpired);
    }
}
