//! HHH-IM-Protocol：宿主（HHH）与外部 IM 插件之间的线上类型、JSON-RPC 编解码与传输。
//!
//! 规范正本：`doc/product-contracts/hhh-im/provider-protocol/protocol-v1.md`。
//!
//! 这些线上类型**独立于 hhh-im 的内部类型**定义、独立定版、独立冻结：hhh-im 的内部模型
//! 怎么改，都不破坏已发布的插件；两边之间的翻译由 hhh-im 的 provider 适配器负责。
//!
//! 分层：
//! - [`rpc`]：JSON-RPC 2.0 信封与一行一条的编解码。
//! - [`types`]：账号、会话、人员、消息、内容块、资源、表情等领域类型。
//! - [`platform`]：平台说明、能力清单、设置项、插件清单。
//! - [`method`]：方法名常量，以及每个方法的参数/结果类型。
//! - [`release`]：安装包的目标、更新来源、版本号与版本索引（发布与检查更新）。
//! - [`transport`]（feature `transport`）：子进程 stdio 传输与 [`transport::ProviderTransport`] trait。
//! - [`package`]（feature `package`）：安装包 `.hhhim` 的打包、读清单与解包。

#![forbid(unsafe_code)]

pub mod method;
pub mod platform;
pub mod release;
pub mod rpc;
pub mod types;

#[cfg(feature = "transport")]
pub mod transport;

#[cfg(feature = "package")]
pub mod package;

pub use method::Method;
pub use platform::{Capabilities, Manifest, PlatformDesc, SettingDecl};
pub use rpc::{ErrorKind, RpcError, RpcErrorData, RpcId, RpcMessage, codec};
pub use types::{
    Account, Block, Conversation, ConversationKind, Event, Message, Outgoing, Person, Reaction,
    Resource, ResourceKind, SendOutcome, SendTarget,
};

/// 本库实现的协议主版本号。只有不兼容的改动才升主版本。
pub const PROTOCOL_VERSION: u32 = 1;

/// 清单文件固定文件名。
pub const MANIFEST_FILE: &str = "hhh-im-provider.json";

/// 宿主启动插件时设置的环境变量（值固定为 `"1"`）。
pub const ENV_FLAG: &str = "HHH_IM_PROTOCOL";

/// 宿主强制执行的预算（protocol-v1 §14）。超出的内容宿主按协议错误处理，不写进本地库。
pub mod budget {
    /// 单条 JSON-RPC 消息最大字节数。
    pub const MESSAGE_MAX_BYTES: usize = 4 * 1024 * 1024;
    /// 每批推送事件最多几个。
    pub const EVENTS_PER_PUSH_MAX: usize = 500;
    /// 每条消息的内容块最多几个。
    pub const BLOCKS_PER_MESSAGE_MAX: usize = 200;
    /// 单块文字最大字节数。
    pub const TEXT_BLOCK_MAX_BYTES: usize = 64 * 1024;
    /// 编号最大字节数（不含控制字符）。
    pub const ID_MAX_BYTES: usize = 256;
    /// 资源一段最大字节数。
    pub const RESOURCE_SEGMENT_MAX_BYTES: u64 = 1024 * 1024;
    /// 登录二维码图片最大字节数。
    pub const QR_IMAGE_MAX_BYTES: usize = 512 * 1024;
    /// 登录步骤里插件声明的提示最多几个字（按字符数，契约 provider-protocol v8）；超出的提示不显示。
    pub const LOGIN_NOTE_MAX_CHARS: usize = 200;
    /// logo 最大字节数。
    pub const LOGO_MAX_BYTES: usize = 256 * 1024;
    /// 凭据最大字节数。
    pub const CREDENTIAL_MAX_BYTES: usize = 64 * 1024;
    /// 单条消息的 `raw` 原始数据最大字节数。
    pub const RAW_MAX_BYTES: usize = 256 * 1024;
    /// 一次 `messages/retranslate` 最多几条。
    pub const RETRANSLATE_BATCH_MAX: usize = 100;
    /// 日志行、`log/write` 的 message 截断到多少字节。
    pub const LOG_LINE_MAX_BYTES: usize = 4 * 1024;
}
