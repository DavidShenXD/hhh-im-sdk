//! 方法名，以及每个方法的参数/结果类型（protocol-v1 §4、§6、§8、§9、§10、§11）。
//!
//! 命名约定：宿主→插件的请求与插件→宿主的请求都在这里；通知（`login/step`、`status`、`log/write`、`$/cancel`）
//! 用各自的参数类型，没有结果。

use serde::{Deserialize, Serialize};

use crate::platform::{Capabilities, PlatformDesc, SettingDecl};
use crate::rpc::ReauthReason;
use crate::types::{
    Account, ConversationKind, Emoji, EmojiCategory, EmojiTone, Event, Message, Outgoing, Person,
    Raw, ResourceKind, SendOutcome, SendTarget,
};

/// 协议里的方法名。字符串形式见 [`Method::as_str`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    // —— 生命周期（宿主→插件）——
    /// 协商版本，取回平台说明与能力。
    Initialize,
    /// 恢复账号。
    SessionOpen,
    /// 注销账号。
    SessionLogout,
    /// 关闭。
    Shutdown,
    /// 心跳。
    Ping,
    // —— 登录 ——
    /// 开始登录（宿主→插件）。
    LoginStart,
    /// 提交表单（宿主→插件）。
    LoginSubmit,
    /// 取消登录（宿主→插件）。
    LoginCancel,
    /// 登录步骤（插件→宿主通知）。
    LoginStep,
    /// 登录成功（插件→宿主通知）。
    LoginFinished,
    /// 登录失败（插件→宿主通知）。
    LoginFailed,
    /// 把本机回调交给插件（宿主→插件，契约 provider-protocol v12 缺口 6）。
    LoginCallback,
    // —— 凭据与授权（插件→宿主）——
    /// 保存凭据（插件→宿主）。
    CredentialSave,
    /// 授权失效（插件→宿主通知）。
    SessionExpired,
    /// 这个连接现在有的问题（插件→宿主通知，契约 provider-protocol v12 缺口 5）。
    SessionNotice,
    /// 用存着的凭据续期（宿主→插件，契约 provider-protocol v12 缺口 7）。
    SessionRenew,
    // —— 读取（宿主→插件）——
    /// 列会话。
    ConversationsList,
    /// 取会话。
    ConversationsGet,
    /// 搜会话。
    ConversationsSearch,
    /// 列群成员。
    ConversationsMembers,
    /// 定位已有私聊。
    ConversationsFindDirect,
    /// 一段时间里有人发过消息的会话（会话发现，契约 provider-protocol v12 缺口 1）。
    ConversationsRecentActivity,
    /// 批量读人。
    PeopleGet,
    /// 搜人。
    PeopleSearch,
    /// 列消息。
    MessagesList,
    /// 取单条消息。
    MessagesGet,
    /// 读表情。
    ReactionsList,
    /// 表情目录。
    EmojiCatalog,
    // —— 资源（宿主→插件）——
    /// 读消息资源一段。
    ResourcesRead,
    /// 读头像一段。
    AvatarsRead,
    /// 读表情图一段。
    EmojiImage,
    // —— 上传（宿主→插件）——
    /// 开始上传。
    UploadsBegin,
    /// 传一段。
    UploadsChunk,
    /// 完成上传。
    UploadsFinish,
    /// 放弃上传。
    UploadsAbort,
    // —— 动作（宿主→插件）——
    /// 发送。
    MessagesSend,
    /// 按操作编号查发送结果。
    MessagesLookupSend,
    /// 撤回。
    MessagesRecall,
    /// 编辑。
    MessagesEdit,
    /// 重新翻译。
    MessagesRetranslate,
    /// 点表情。
    ReactionsAdd,
    /// 取消表情。
    ReactionsRemove,
    // —— 推送与状态（插件→宿主）——
    /// 推送事件。
    EventsPush,
    /// 连接状态（通知）。
    Status,
    /// 结构化日志（通知）。
    LogWrite,
    // —— 取消（宿主→插件通知）——
    /// 取消请求。
    Cancel,
}

impl Method {
    /// 线上方法名字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Initialize => "initialize",
            Method::SessionOpen => "session/open",
            Method::SessionLogout => "session/logout",
            Method::Shutdown => "shutdown",
            Method::Ping => "ping",
            Method::LoginStart => "login/start",
            Method::LoginSubmit => "login/submit",
            Method::LoginCancel => "login/cancel",
            Method::LoginStep => "login/step",
            Method::LoginFinished => "login/finished",
            Method::LoginFailed => "login/failed",
            Method::LoginCallback => "login/callback",
            Method::CredentialSave => "credential/save",
            Method::SessionExpired => "session/expired",
            Method::SessionNotice => "session/notice",
            Method::SessionRenew => "session/renew",
            Method::ConversationsList => "conversations/list",
            Method::ConversationsGet => "conversations/get",
            Method::ConversationsSearch => "conversations/search",
            Method::ConversationsMembers => "conversations/members",
            Method::ConversationsFindDirect => "conversations/findDirect",
            Method::ConversationsRecentActivity => "conversations/recentActivity",
            Method::PeopleGet => "people/get",
            Method::PeopleSearch => "people/search",
            Method::MessagesList => "messages/list",
            Method::MessagesGet => "messages/get",
            Method::ReactionsList => "reactions/list",
            Method::EmojiCatalog => "emoji/catalog",
            Method::ResourcesRead => "resources/read",
            Method::AvatarsRead => "avatars/read",
            Method::EmojiImage => "emoji/image",
            Method::UploadsBegin => "uploads/begin",
            Method::UploadsChunk => "uploads/chunk",
            Method::UploadsFinish => "uploads/finish",
            Method::UploadsAbort => "uploads/abort",
            Method::MessagesSend => "messages/send",
            Method::MessagesLookupSend => "messages/lookupSend",
            Method::MessagesRecall => "messages/recall",
            Method::MessagesEdit => "messages/edit",
            Method::MessagesRetranslate => "messages/retranslate",
            Method::ReactionsAdd => "reactions/add",
            Method::ReactionsRemove => "reactions/remove",
            Method::EventsPush => "events/push",
            Method::Status => "status",
            Method::LogWrite => "log/write",
            Method::Cancel => "$/cancel",
        }
    }

    /// 从字符串认方法名。
    ///
    /// # Examples
    ///
    /// ```
    /// use hhh_im_protocol::Method;
    /// assert_eq!(Method::from_name("conversations/list"), Some(Method::ConversationsList));
    /// assert_eq!(Method::from_name("nope"), None);
    /// assert_eq!(Method::ConversationsList.as_str(), "conversations/list");
    /// ```
    pub fn from_name(name: &str) -> Option<Method> {
        // 全集逐一比对；集合小、调用不频繁，直观优先。
        const ALL: &[Method] = &[
            Method::Initialize,
            Method::SessionOpen,
            Method::SessionLogout,
            Method::Shutdown,
            Method::Ping,
            Method::LoginStart,
            Method::LoginSubmit,
            Method::LoginCancel,
            Method::LoginStep,
            Method::LoginFinished,
            Method::LoginFailed,
            Method::LoginCallback,
            Method::CredentialSave,
            Method::SessionExpired,
            Method::SessionNotice,
            Method::SessionRenew,
            Method::ConversationsList,
            Method::ConversationsGet,
            Method::ConversationsSearch,
            Method::ConversationsMembers,
            Method::ConversationsFindDirect,
            Method::ConversationsRecentActivity,
            Method::PeopleGet,
            Method::PeopleSearch,
            Method::MessagesList,
            Method::MessagesGet,
            Method::ReactionsList,
            Method::EmojiCatalog,
            Method::ResourcesRead,
            Method::AvatarsRead,
            Method::EmojiImage,
            Method::UploadsBegin,
            Method::UploadsChunk,
            Method::UploadsFinish,
            Method::UploadsAbort,
            Method::MessagesSend,
            Method::MessagesLookupSend,
            Method::MessagesRecall,
            Method::MessagesEdit,
            Method::MessagesRetranslate,
            Method::ReactionsAdd,
            Method::ReactionsRemove,
            Method::EventsPush,
            Method::Status,
            Method::LogWrite,
            Method::Cancel,
        ];
        ALL.iter().copied().find(|m| m.as_str() == name)
    }
}

// ============================ 生命周期 ============================

/// `initialize` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    /// 宿主支持的协议主版本。
    pub protocol_versions: Vec<u32>,
    /// 宿主信息。
    pub host: HostInfo,
    /// 界面语言（如 zh-CN）。
    pub locale: String,
    /// 宿主给的临时目录（可随时被清空）。
    pub temp_dir: String,
    /// 网络设置。
    pub network: NetworkConfig,
}

/// 宿主信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostInfo {
    /// 宿主名。
    pub name: String,
    /// 宿主版本。
    pub version: String,
}

/// 网络设置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// 出站代理；没有为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
}

/// `initialize` 结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    /// 选定的协议主版本。
    pub protocol_version: u32,
    /// 插件信息。
    pub provider: ProviderInfo,
    /// 平台说明。
    pub platform: PlatformDesc,
    /// 能力清单。
    pub capabilities: Capabilities,
    /// 程序设置项。
    #[serde(default)]
    pub settings: Vec<SettingDecl>,
    /// 支持的登录方式，按显示顺序（契约 provider-protocol v12 缺口 6）；不写就是只有一种、点登录直接走插件推来的步骤。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub login_methods: Vec<LoginMethod>,
}

/// 插件信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    /// 程序编号（同清单 id）。
    pub id: String,
    /// 平台名。
    pub name: String,
    /// 程序版本。
    pub version: String,
}

/// `session/open` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOpenParams {
    /// 宿主保存的不透明凭据；没有为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    /// 程序设置项的值。
    #[serde(default)]
    pub settings: std::collections::BTreeMap<String, String>,
    /// 上次保存的同步游标；没有为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_state: Option<String>,
    /// 这份凭据是借来的、只能用（隔离测试实例借用户的，契约 provider-protocol v12 缺口 7）：插件不得换新凭据、
    /// 不得调 `credential/save`，到期就发 `session/expired`。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub credential_read_only: bool,
    /// 同一编号的内置平台留下的授权与设置，交给插件一次（契约 provider-protocol v12 缺口 14，格式见规范附录 D）；
    /// 只在插件还没有自己的凭据时给。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy: Option<LegacyHandover>,
}

/// 内置平台交给插件的授权与设置（`session/open.legacy`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyHandover {
    /// 格式名；本版只有 [`LEGACY_FORMAT_BUILTIN_V1`]。
    pub format: String,
    /// 内置授权（格式里定义的 JSON 文字）。
    pub credential: String,
    /// 内置平台的连接设置（格式里定义的 JSON 对象）。
    #[serde(default)]
    pub settings: serde_json::Value,
}

/// 内置飞书、Slack 交接用的格式名（规范附录 D）。
pub const LEGACY_FORMAT_BUILTIN_V1: &str = "hhh-builtin-v1";

/// 插件接过交接后，请宿主写进它自己设置里的东西（`session/open` 在线结果的 `adoptedSettings`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptedSettings {
    /// 写进插件参数配置的值（键是插件声明的设置项编号）。
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub parameters: std::collections::BTreeMap<String, String>,
    /// 写成“现在在用”的登录方式与填的内容（键见 [`login_value`]）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<AdoptedLogin>,
}

/// 交接后在用的登录方式。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptedLogin {
    /// 登录方式编号（插件 `loginMethods` 里的 `id`）。
    pub method: String,
    /// 填的内容。
    #[serde(default)]
    pub values: std::collections::BTreeMap<String, String>,
}

/// `session/open` 结果（protocol-v1 §4.2）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SessionOpenResult {
    /// 已在线。
    Online {
        /// 已核验账号。
        account: Account,
        /// 接过交接时请宿主写进插件设置的东西（契约 provider-protocol v12 缺口 14）；没有为空。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        adopted_settings: Option<AdoptedSettings>,
    },
    /// 需要登录。
    NeedsLogin {
        /// 原因。
        reason: NeedsLoginReason,
        /// 给用户看的说明。
        message: String,
    },
}

/// 需要登录的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NeedsLoginReason {
    /// 没有凭据。
    NoCredential,
    /// 凭据过期。
    Expired,
    /// 平台撤销。
    Revoked,
    /// 凭据无效。
    CredentialInvalid,
}

// ============================ 登录 ============================

/// 登录方式的类型：由 IM 层定，各 IM 声明支持哪几种（契约 provider-protocol v12 缺口 6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginKind {
    /// 经公网授权中继：登录前填中继地址（[`login_value::RELAY_URL`]）。
    Relay,
    /// 用户自己的应用：登录前填 App ID 与 App Secret（[`login_value::APP_ID`]、[`login_value::APP_SECRET`]）。
    OwnApp,
    /// 直接登录：不用填，照插件推来的步骤走。
    Direct,
    /// 宿主不认识的类型：这一种方式不登记。
    #[serde(other)]
    Unknown,
}

/// `login/start.values` 里各类型用的键。
pub mod login_value {
    /// 中继地址（`relay`）。
    pub const RELAY_URL: &str = "relayUrl";
    /// App ID（`ownApp`）。
    pub const APP_ID: &str = "appId";
    /// App Secret（`ownApp`）；只在用户这次填了时给，没填表示沿用插件凭据里的。
    pub const APP_SECRET: &str = "appSecret";
}

/// 一种登录方式（`initialize.loginMethods`）：类型由 IM 层定，叫法、说明、灰字、链接由 IM 写。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginMethod {
    /// 方式编号（插件内不重复，宿主据此记“现在在用哪种”）。
    pub id: String,
    /// 类型。
    pub kind: LoginKind,
    /// 叫法（页签上的字）。
    pub label: String,
    /// 一句说明。
    #[serde(default)]
    pub note: String,
    /// `relay`：中继地址框的灰字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// `relay`：没保存过中继地址时框里预填的地址（https，或本机测试的回环 http；v14）。不写就空着让用户填。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    /// `ownApp`：App ID 的叫法（默认 App ID）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_label: Option<String>,
    /// `ownApp`：App ID 框的灰字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_placeholder: Option<String>,
    /// `ownApp`：App Secret 的叫法（默认 App Secret）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_label: Option<String>,
    /// `ownApp`：App Secret 框的灰字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_placeholder: Option<String>,
    /// `ownApp`：App Secret 去哪找、存在哪的说明（画在框旁的 ⓘ 里；v14）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_help: Option<String>,
    /// `ownApp`：要宿主在本机收授权回调时的路径；不写就没有回调。写成相对路径（如 `callback`，v14）时宿主拼在这个 IM
    /// 自己的 `/hhh-friend-im/<平台编号>/` 下面（[`resolve_callback_path`]），插件不必知道宿主怎么给它编号；照 v12 写成
    /// 以 `/` 开头的完整路径时必须已经在这个目录下。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_path: Option<String>,
    /// `ownApp`：回调地址的说明（画在 ⓘ 里）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_help: Option<String>,
    /// 可选的“前往…创建”链接（只收 https）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_link: Option<CreateLink>,
}

/// 登录方式里的一个外部链接。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateLink {
    /// 链接文字。
    pub label: String,
    /// 地址（https）。
    pub url: String,
    /// 链接后面接的一句话。
    #[serde(default)]
    pub after: String,
}

/// 回调路径必须在的目录：`/hhh-friend-im/<平台编号>/`。
///
/// # Examples
///
/// ```
/// use hhh_im_protocol::method::callback_path_ok;
/// assert!(callback_path_ok("/hhh-friend-im/ext-sample/callback", "ext-sample"));
/// assert!(!callback_path_ok("/hhh-friend-im/feishu/callback", "ext-sample"));
/// assert!(!callback_path_ok("/hhh-friend-im/ext-sample/", "ext-sample"));
/// assert!(!callback_path_ok("/hhh-friend-im/ext-sample/../feishu/callback", "ext-sample"));
/// ```
pub fn callback_path_ok(path: &str, platform_id: &str) -> bool {
    let prefix = format!("/hhh-friend-im/{platform_id}/");
    let Some(rest) = path.strip_prefix(&prefix) else {
        return false;
    };
    !rest.is_empty()
        && path.len() <= 256
        && rest.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        })
}

/// 插件声明的回调路径在宿主上的完整路径（契约 provider-protocol v14）：相对路径拼在 `/hhh-friend-im/<平台编号>/` 下面，
/// 以 `/` 开头的照 v12 原样用；不在这个 IM 自己的目录下、或某一段不合规时为空（宿主不登记这种登录方式）。
///
/// # Examples
///
/// ```
/// use hhh_im_protocol::method::resolve_callback_path;
/// assert_eq!(resolve_callback_path("callback", "feishu").as_deref(), Some("/hhh-friend-im/feishu/callback"));
/// assert_eq!(resolve_callback_path("oauth/done", "ext-sample").as_deref(), Some("/hhh-friend-im/ext-sample/oauth/done"));
/// assert_eq!(resolve_callback_path("/hhh-friend-im/ext-sample/callback", "ext-sample").as_deref(), Some("/hhh-friend-im/ext-sample/callback"));
/// assert_eq!(resolve_callback_path("/hhh-friend-im/feishu/callback", "ext-sample"), None);
/// assert_eq!(resolve_callback_path("../feishu/callback", "ext-sample"), None);
/// assert_eq!(resolve_callback_path("", "ext-sample"), None);
/// ```
pub fn resolve_callback_path(path: &str, platform_id: &str) -> Option<String> {
    let full = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/hhh-friend-im/{platform_id}/{path}")
    };
    callback_path_ok(&full, platform_id).then_some(full)
}

/// `login/start` 参数（契约 provider-protocol v12 缺口 6）。旧宿主发 `{}`，旧插件忽略多出的字段。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStartParams {
    /// 这次选的登录方式编号（插件声明了 `loginMethods` 时给）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// 这次填的内容（键见 [`login_value`]）。
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub values: std::collections::BTreeMap<String, String>,
    /// 宿主在本机收回调的完整地址（这种方式声明了 `callbackPath` 时给）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_url: Option<String>,
    /// 一次性的 `state`：插件放进授权地址，回调带回来时宿主核对（给了 `callbackUrl` 时给）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// `login/callback` 参数（宿主→插件）：宿主在本机收到这次登录的回调、`state` 对上了，把整个地址交给插件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginCallbackParams {
    /// 登录流程编号。
    pub flow_id: String,
    /// 回调的完整地址（含查询串）。
    pub url: String,
}

/// `login/start` 结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStartResult {
    /// 本次登录流程编号。
    pub flow_id: String,
}

/// `login/step` 通知参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStepParams {
    /// 登录流程编号。
    pub flow_id: String,
    /// 这一步要显示什么。
    pub step: LoginStep,
}

/// 登录步骤（protocol-v1 §7 登录表）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LoginStep {
    /// 二维码。
    QrCode {
        /// PNG 的 base64。
        image: String,
        /// 过期时间（毫秒）。
        expires_at_ms: i64,
        /// 提示。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
        /// 插件声明的提示（契约 provider-protocol v8）：画在这一步内容下面、按钮上面；不给就没有。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<LoginNote>,
    },
    /// 打开浏览器。
    OpenUrl {
        /// 地址（https 或本机回环 http）。
        url: String,
        /// 按钮文字。
        label: String,
        /// 插件声明的提示（契约 provider-protocol v8）：画在这一步内容下面、按钮上面；不给就没有。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<LoginNote>,
    },
    /// 等待设备确认。
    ConfirmOnDevice {
        /// 提示。
        message: String,
        /// 插件声明的提示（契约 provider-protocol v8）：画在这一步内容下面、按钮上面；不给就没有。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<LoginNote>,
    },
    /// 表单。
    Input {
        /// 字段。
        fields: Vec<LoginField>,
        /// 插件声明的提示（契约 provider-protocol v8）：画在这一步内容下面、按钮上面；不给就没有。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<LoginNote>,
    },
    /// 一段说明（本身就是提示，不再带 `note`）。
    Message {
        /// 文字。
        text: String,
        /// 色调。
        tone: MessageTone,
    },
}

/// 登录表单字段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginField {
    /// 字段编号。
    pub id: String,
    /// 显示名。
    pub label: String,
    /// 密文。
    #[serde(default)]
    pub secret: bool,
    /// 占位符。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// 校验正则。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
}

/// 插件在一步登录里声明的提示（契约 provider-protocol v8）。每个插件自己决定在哪一步、说什么，
/// 宿主不认识提示的含义，只按字段画；文字不超过 [`crate::budget::LOGIN_NOTE_MAX_CHARS`] 个字、原样显示。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginNote {
    /// 文字。
    pub text: String,
    /// 色调（同“一段说明”）。
    pub tone: MessageTone,
}

/// 说明色调。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageTone {
    /// 普通。
    Info,
    /// 警告。
    Warn,
}

/// `login/submit` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginSubmitParams {
    /// 登录流程编号。
    pub flow_id: String,
    /// 表单值。
    pub values: std::collections::BTreeMap<String, String>,
}

/// `login/cancel` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginCancelParams {
    /// 登录流程编号。
    pub flow_id: String,
}

/// `login/finished` 通知参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginFinishedParams {
    /// 登录流程编号。
    pub flow_id: String,
    /// 核验账号。
    pub account: Account,
}

/// `login/failed` 通知参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginFailedParams {
    /// 登录流程编号。
    pub flow_id: String,
    /// 给用户看的说明。
    pub message: String,
    /// 能不能重试。
    #[serde(default)]
    pub retryable: bool,
}

// ============================ 凭据与授权 ============================

/// `credential/save` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialSaveParams {
    /// 不透明凭据（≤64 KiB）。
    pub credential: String,
    /// 下次要续期的时间（毫秒，契约 provider-protocol v12 缺口 7）：到了宿主叫 `session/renew`，插件没在跑也临时起来续。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renew_at_ms: Option<i64>,
}

/// `session/renew` 参数（宿主→插件，需要能力 `session.renew`）：用这份凭据续期，经 `credential/save` 存新的再回空结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRenewParams {
    /// 宿主存着的凭据。
    pub credential: String,
}

/// `session/notice` 通知参数（插件→宿主）：这个连接现在有的问题，每次给全，空数组就是都没有了。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNoticeParams {
    /// 问题。
    #[serde(default)]
    pub notices: Vec<Notice>,
}

/// 连接上的一个问题（画在卡片下面的问题行）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    /// 编号（同一个问题前后不变）。
    pub id: String,
    /// 给用户看的一句话（不超过 [`crate::budget::NOTICE_MAX_CHARS`] 个字）。
    pub text: String,
    /// 要重新登录才能好：卡片右边多一颗“重新登录”。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub relogin: bool,
}

/// `session/expired` 通知参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionExpiredParams {
    /// 原因。
    pub reason: ReauthReason,
    /// 给用户看的说明。
    pub message: String,
}

// ============================ 分页 ============================

/// 一页结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    /// 本页项目。
    pub items: Vec<T>,
    /// 下一页游标；没有为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// 一页搜索结果（带不完整提示）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage<T> {
    /// 本页项目。
    pub items: Vec<T>,
    /// 下一页游标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    /// 范围/完整性提示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

// ============================ 读取参数/结果 ============================

/// `conversations/list` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationsListParams {
    /// 每页 1–100。
    pub limit: u16,
    /// 游标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// `conversations/get` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationsGetParams {
    /// 会话编号。
    pub conversation_id: String,
}

/// 关键词搜索参数（会话与人员共用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchParams {
    /// 关键词 1–50 字。
    pub text: String,
    /// 每页 1–20。
    pub limit: u16,
    /// 游标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// `conversations/members` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MembersParams {
    /// 会话编号。
    pub conversation_id: String,
    /// 每页上限。
    pub limit: u16,
    /// 游标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// `conversations/findDirect` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindDirectParams {
    /// 对方用户编号。
    pub user_id: String,
}

/// `conversations/findDirect` 结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindDirectResult {
    /// 找到的会话编号；没找到为空（不代表从未有过会话）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    /// 提示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

/// `people/get` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeopleGetParams {
    /// 用户编号（≤100 个）。
    pub ids: Vec<String>,
}

/// `people/get` 结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeopleGetResult {
    /// 读到的。
    pub found: Vec<Person>,
    /// 平台明确说不存在的；这次没读到的两边都不放。
    #[serde(default)]
    pub missing: Vec<String>,
}

/// `messages/list` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesListParams {
    /// 会话编号。
    pub conversation_id: String,
    /// 话题编号。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// 只要这个时间之后的（毫秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_ms: Option<i64>,
    /// 只要这个时间之前的（毫秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_ms: Option<i64>,
    /// 从最新往更早翻。
    #[serde(default)]
    pub newest_first: bool,
    /// 每页上限。
    pub limit: u16,
    /// 游标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// `messages/get` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesGetParams {
    /// 会话编号。
    pub conversation_id: String,
    /// 消息编号。
    pub message_id: String,
}

/// `reactions/list` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionsListParams {
    /// 会话编号。
    pub conversation_id: String,
    /// 消息编号。
    pub message_id: String,
    /// 每页 ≤50。
    pub limit: u16,
    /// 游标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// `emoji/catalog` 结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmojiCatalogResult {
    /// 表情。
    pub emojis: Vec<Emoji>,
    /// 分类。
    #[serde(default)]
    pub categories: Vec<EmojiCategory>,
    /// 肤色，按面板里的顺序（v14）；没有肤色选择时不写。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tones: Vec<EmojiTone>,
    /// 目录更新时间（毫秒）。
    pub updated_at_ms: i64,
}

// ============================ 资源 ============================

/// `resources/read` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcesReadParams {
    /// 会话编号。
    pub conversation_id: String,
    /// 消息编号。
    pub message_id: String,
    /// 资源键。
    pub key: String,
    /// 资源种类。
    pub kind: ResourceKind,
    /// 起始字节。
    pub offset: u64,
    /// 最多读多少（≤1 MiB）。
    pub length: u64,
}

/// `avatars/read` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarsReadParams {
    /// 头像键。
    pub key: String,
    /// 起始字节。
    pub offset: u64,
    /// 最多读多少。
    pub length: u64,
}

/// `emoji/image` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmojiImageParams {
    /// 表情类型。
    pub emoji_type: String,
    /// 起始字节。
    pub offset: u64,
    /// 最多读多少。
    pub length: u64,
}

/// 读资源一段的结果（protocol-v1 §9.1）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceChunk {
    /// 这一段的字节（base64）。
    pub data: String,
    /// 整个资源总字节；能给就给。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// 平台给的内容指纹（小写十六进制）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// 是不是读到末尾了。
    pub eof: bool,
}

// ============================ 上传 ============================

/// `uploads/begin` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadsBeginParams {
    /// 资源种类。
    pub kind: ResourceKind,
    /// 文件名。
    pub name: String,
    /// 总字节数。
    pub bytes: u64,
}

/// `uploads/begin` 结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadsBeginResult {
    /// 上传编号。
    pub upload_id: String,
}

/// `uploads/chunk` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadsChunkParams {
    /// 上传编号。
    pub upload_id: String,
    /// 起始字节。
    pub offset: u64,
    /// 这一段的字节（base64，≤1 MiB）。
    pub data: String,
}

/// `uploads/finish` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadsFinishParams {
    /// 上传编号。
    pub upload_id: String,
    /// 全文件 SHA-256（小写十六进制）。
    pub sha256: String,
}

/// `uploads/finish` 结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadsFinishResult {
    /// 资源键（填进发送内容的 image/file）。
    pub key: String,
}

/// `uploads/abort` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadsAbortParams {
    /// 上传编号。
    pub upload_id: String,
}

// ============================ 动作 ============================

/// `messages/send` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesSendParams {
    /// 操作编号（同一个只发一次）。
    pub operation_id: String,
    /// 发送目标。
    pub target: SendTarget,
    /// 内容。
    pub content: Vec<Outgoing>,
}

/// `messages/lookupSend` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesLookupSendParams {
    /// 操作编号。
    pub operation_id: String,
}

/// `messages/recall` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesRecallParams {
    /// 会话编号。
    pub conversation_id: String,
    /// 消息编号。
    pub message_id: String,
}

/// `messages/edit` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesEditParams {
    /// 会话编号。
    pub conversation_id: String,
    /// 消息编号。
    pub message_id: String,
    /// 新内容。
    pub content: Vec<Outgoing>,
}

/// `reactions/add` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionsAddParams {
    /// 会话编号。
    pub conversation_id: String,
    /// 消息编号。
    pub message_id: String,
    /// 表情类型。
    pub emoji_type: String,
}

/// `reactions/remove` 参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionsRemoveParams {
    /// 会话编号。
    pub conversation_id: String,
    /// 消息编号。
    pub message_id: String,
    /// 表情编号。
    pub reaction_id: String,
}

/// `conversations/recentActivity` 参数（会话发现，契约 provider-protocol v12 缺口 1）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentActivityParams {
    /// 从这个时间（毫秒）起，含这一刻。
    pub since_ms: i64,
    /// 一次最多翻几页。
    pub pages_max: u16,
}

/// `conversations/recentActivity` 结果。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentActivityResult {
    /// 这段时间里有人（不算机器人、应用、经接口发的）发过消息的会话，每个会话一项，按最新一条从新到旧。
    #[serde(default)]
    pub items: Vec<ActivityItem>,
    /// 翻到了起点；翻满页数还没到时为假。
    #[serde(default)]
    pub complete: bool,
}

/// 一段时间里有人发过消息的一个会话（只用来认会话，不交出消息内容）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityItem {
    /// 会话编号。
    pub conversation_id: String,
    /// 会话类别。
    pub kind: ConversationKind,
    /// 这段时间里最早一条的时间（毫秒）。
    pub first_ms: i64,
    /// 最新一条的时间（毫秒）。
    pub latest_ms: i64,
    /// 私聊的对方（不是本人的发送者）；群或只有本人发过时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<String>,
}

/// `messages/retranslate` 参数（protocol-v1 §10.1）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetranslateParams {
    /// 规则名。
    pub rule: String,
    /// 要重译的旧消息（带原始数据）。
    pub messages: Vec<RetranslateItem>,
}

/// 要重译的一条旧消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetranslateItem {
    /// 消息编号。
    pub id: String,
    /// 会话编号。
    pub conversation_id: String,
    /// 去凭据化的原始数据。
    pub raw: Raw,
}

/// `messages/retranslate` 结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetranslateResult {
    /// 重译后的消息（宿主按编号覆盖本地）。
    pub messages: Vec<Message>,
}

/// `reactions/list` / `messages/send` 等用到的发送结果别名。
pub type SendResult = SendOutcome;

// ============================ 推送与状态 ============================

/// `events/push` 参数（protocol-v1 §11.1）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsPushParams {
    /// 这一批事件（≤500）。
    pub events: Vec<Event>,
    /// 平台同步位置（history=none 必带）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_state: Option<String>,
}

/// 连接状态（§11.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnState {
    /// 在线。
    Online,
    /// 连接中。
    Connecting,
    /// 离线。
    Offline,
}

/// `status` 通知参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusParams {
    /// 状态。
    pub state: ConnState,
    /// 原因。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// 日志级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    /// 错误。
    Error,
    /// 关注。
    Warn,
    /// 关键节点。
    Info,
    /// 过程。
    Debug,
}

/// `log/write` 通知参数（§11.5）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogWriteParams {
    /// 级别。
    pub level: LogLevel,
    /// 消息（不含凭据、消息正文）。
    pub message: String,
    /// 结构化字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<std::collections::BTreeMap<String, serde_json::Value>>,
}

/// `$/cancel` 通知参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelParams {
    /// 要取消的请求编号。
    pub id: crate::rpc::RpcId,
}

#[cfg(test)]
mod tests {

    // 契约 provider-protocol v14：登录方式的默认中继、App Secret 的说明、相对的回调路径都可选、原样往返。
    #[test]
    fn v14_login_method_fields_are_optional_and_round_trip() {
        let relay = serde_json::json!({ "id": "relay", "kind": "relay", "label": "通过中继登录", "note": "说明",
            "defaultValue": "https://relay.example.com" });
        let own = serde_json::json!({ "id": "own", "kind": "ownApp", "label": "使用自己的应用", "note": "",
            "secretHelp": "开放平台 → 凭证与基础信息。只保存在本机。", "callbackPath": "callback" });
        for declared in [relay, own] {
            let method: LoginMethod = serde_json::from_value(declared.clone()).unwrap();
            assert_eq!(serde_json::to_value(&method).unwrap(), declared);
        }
        let old: LoginMethod = serde_json::from_value(
            serde_json::json!({ "id": "own", "kind": "ownApp", "label": "自己的应用" }),
        )
        .unwrap();
        assert!(old.default_value.is_none() && old.secret_help.is_none());
    }
    use super::*;

    #[test]
    fn method_names_round_trip() {
        for m in [
            Method::Initialize,
            Method::ConversationsFindDirect,
            Method::MessagesRetranslate,
            Method::EventsPush,
            Method::Cancel,
        ] {
            assert_eq!(Method::from_name(m.as_str()), Some(m));
        }
        assert_eq!(
            Method::ConversationsFindDirect.as_str(),
            "conversations/findDirect"
        );
        assert_eq!(Method::Cancel.as_str(), "$/cancel");
        assert!(Method::from_name("does/not-exist").is_none());
        for (m, name) in [
            (
                Method::ConversationsRecentActivity,
                "conversations/recentActivity",
            ),
            (Method::SessionNotice, "session/notice"),
            (Method::SessionRenew, "session/renew"),
            (Method::LoginCallback, "login/callback"),
        ] {
            assert_eq!(m.as_str(), name);
            assert_eq!(Method::from_name(name), Some(m));
        }
    }

    // 契约 provider-protocol v12：登录、续期、交接相关的新字段都是可选的——旧插件、旧宿主的形状照读，写回不多出键。
    #[test]
    fn v12_session_and_login_fields_are_optional_and_round_trip() {
        use serde_json::json;
        // login/start：旧宿主发 {}，读成全空；新宿主给方式、内容、回调地址与 state。
        let old: LoginStartParams = serde_json::from_value(json!({})).unwrap();
        assert_eq!(old, LoginStartParams::default());
        assert_eq!(serde_json::to_value(&old).unwrap(), json!({}));
        let start = json!({ "method": "own", "values": { "appId": "cli_1", "appSecret": "s" },
            "callbackUrl": "http://localhost:29527/hhh-friend-im/ext-sample/callback", "state": "st" });
        let parsed: LoginStartParams = serde_json::from_value(start.clone()).unwrap();
        assert_eq!(parsed.values[login_value::APP_ID], "cli_1");
        assert_eq!(serde_json::to_value(&parsed).unwrap(), start);

        // session/open：不借用、没有交接时不多出键；在线结果不带 adoptedSettings 时照旧。
        let open: SessionOpenParams = serde_json::from_value(json!({ "settings": {} })).unwrap();
        assert!(!open.credential_read_only && open.legacy.is_none());
        let v = serde_json::to_value(&open).unwrap();
        assert!(v.get("credentialReadOnly").is_none() && v.get("legacy").is_none());
        let online: SessionOpenResult = serde_json::from_value(
            json!({ "state": "online", "account": { "id": "u1", "name": "我" } }),
        )
        .unwrap();
        assert!(matches!(
            online,
            SessionOpenResult::Online {
                adopted_settings: None,
                ..
            }
        ));
        let adopted = json!({ "state": "online", "account": { "id": "u1", "name": "我" },
            "adoptedSettings": { "parameters": { "topic": "on" }, "login": { "method": "relay", "values": { "relayUrl": "https://r.example" } } } });
        let back: SessionOpenResult = serde_json::from_value(adopted.clone()).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), adopted);

        // credential/save 的续期时间、session/notice 每次给全。
        let save: CredentialSaveParams =
            serde_json::from_value(json!({ "credential": "c" })).unwrap();
        assert!(save.renew_at_ms.is_none());
        let notice: SessionNoticeParams = serde_json::from_value(
            json!({ "notices": [{ "id": "scope", "text": "缺少权限 xxx", "relogin": true }] }),
        )
        .unwrap();
        assert!(notice.notices[0].relogin);
        assert_eq!(
            serde_json::from_value::<SessionNoticeParams>(json!({})).unwrap(),
            SessionNoticeParams::default()
        );

        // 登录方式：不认识的类型读成 Unknown，不让整个握手失败。
        let method: LoginMethod =
            serde_json::from_value(json!({ "id": "x", "kind": "passkey", "label": "通行密钥" }))
                .unwrap();
        assert_eq!(method.kind, LoginKind::Unknown);

        // 会话发现的结果。
        let activity: RecentActivityResult = serde_json::from_value(json!({
            "items": [{ "conversationId": "c1", "kind": "direct", "firstMs": 1, "latestMs": 2, "peerId": "u2" }],
            "complete": true
        }))
        .unwrap();
        assert_eq!(activity.items[0].peer_id.as_deref(), Some("u2"));
    }

    #[test]
    fn session_open_result_is_tagged_by_state() {
        let needs = SessionOpenResult::NeedsLogin {
            reason: NeedsLoginReason::NoCredential,
            message: "请扫码".into(),
        };
        let v = serde_json::to_value(&needs).unwrap();
        assert_eq!(v["state"], "needsLogin");
        assert_eq!(v["reason"], "noCredential");
    }

    #[test]
    fn login_step_qr_round_trips() {
        let step = LoginStep::QrCode {
            image: "iVBOR".into(),
            expires_at_ms: 1,
            hint: Some("扫码".into()),
            note: None,
        };
        let v = serde_json::to_value(&step).unwrap();
        assert_eq!(v["kind"], "qrCode");
        assert_eq!(v["expiresAtMs"], 1);
        let back: LoginStep = serde_json::from_value(v).unwrap();
        matches!(back, LoginStep::QrCode { .. });
    }

    /// 提示是可选的（契约 provider-protocol v8）：旧插件不带 `note` 照常读、写回时也不多出这个键；
    /// 二维码、打开链接、在手机上确认、表单四种步骤带了就原样往返。
    #[test]
    fn login_notes_are_optional_and_round_trip_on_every_step_that_takes_one() {
        use serde_json::json;
        let old = json!({ "kind": "confirmOnDevice", "message": "请在手机上确认" });
        let step: LoginStep = serde_json::from_value(old.clone()).unwrap();
        assert!(matches!(
            step,
            LoginStep::ConfirmOnDevice { note: None, .. }
        ));
        assert_eq!(
            serde_json::to_value(&step).unwrap(),
            old,
            "没带提示的写回时不多出 note"
        );

        let note = json!({ "text": "扫码后勾选同步", "tone": "info" });
        for mut shape in [
            json!({ "kind": "qrCode", "image": "iVBOR", "expiresAtMs": 1, "hint": "扫码" }),
            json!({ "kind": "openUrl", "url": "https://example.com/login", "label": "打开" }),
            json!({ "kind": "confirmOnDevice", "message": "请在手机上确认" }),
            json!({ "kind": "input", "fields": [{ "id": "code", "label": "验证码", "secret": false }] }),
        ] {
            shape["note"] = note.clone();
            let step: LoginStep = serde_json::from_value(shape.clone()).unwrap();
            assert_eq!(
                serde_json::to_value(&step).unwrap(),
                shape,
                "带提示的原样往返：{shape}"
            );
        }
        let warn: LoginNote =
            serde_json::from_value(json!({ "text": "注意", "tone": "warn" })).unwrap();
        assert_eq!(warn.tone, MessageTone::Warn);
    }
}
