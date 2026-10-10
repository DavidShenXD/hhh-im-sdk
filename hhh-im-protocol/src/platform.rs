//! 平台说明、能力清单、设置项、插件清单（protocol-v1 §2、§5、§6.4）。

use serde::{Deserialize, Serialize};

/// 插件包清单 `hhh-im-provider.json`（protocol-v1 §2）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// 清单格式版本，当前 1。
    pub manifest_version: u32,
    /// 程序编号：小写字母开头，`a-z0-9-`，2–32 字符。内部平台编号为 `ext-<id>`。
    pub id: String,
    /// 给用户看的平台名，1–20 字符。
    pub name: String,
    /// 程序版本（semver）。
    pub version: String,
    /// 启动命令（相对清单目录，或 PATH 里的程序）。
    pub command: String,
    /// 启动参数。
    #[serde(default)]
    pub args: Vec<String>,
    /// 支持的系统：windows / macos / linux。
    pub os: Vec<String>,
    /// logo 文件（相对清单目录），SVG/PNG ≤256 KiB；没有就用名字首字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    /// 发布者。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    /// 主页。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// IM 类型（可选）：同一种 IM 在宿主里同一时间只用一个。内置飞书是 `feishu`、内置 Slack 是 `slack`，
    /// 写成同一个值就和它们是同一种；不写时自成一类。写法同编号（见 [`Self::id_ok`]）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub im_type: Option<String>,
    /// 这个安装包给哪种电脑（[`crate::release::Target`] 的名字，契约 provider-protocol v10 §4.2）。从安装包安装时必须有、
    /// 且必须是本机的目标；从文件夹安装（开发用）时可以不写，照旧按 `os` 判断。打包工具写进包里的清单。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// 更新来源：宿主只从这里检查、下载新版本；不写就没有更新来源。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<crate::release::Source>,
    /// 需要的最低宿主（HHH）版本，`主.次.修订`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_host: Option<String>,
}

impl Manifest {
    /// 本轮新加的发布字段是否合规（契约 provider-protocol v10 §4.2）：目标认得出、来源写法对、最低宿主版本是三段数字、
    /// 版本号是三段数字（有来源的插件要比版本大小）。旧清单都不写这些，照常合规。
    ///
    /// # Examples
    ///
    /// ```
    /// let mut manifest: hhh_im_protocol::Manifest = serde_json::from_str(
    ///     r#"{"manifestVersion":1,"id":"wechat-pad","name":"微信","version":"0.4.0","command":"p","os":["windows"],
    ///        "target":"windows-x64","source":{"type":"github","repo":"someone/hhh-wechat"},"minHost":"0.1.0"}"#,
    /// ).unwrap();
    /// assert!(manifest.release_fields_ok().is_ok());
    /// manifest.target = Some("windows-arm64".into());
    /// assert!(manifest.release_fields_ok().is_err());
    /// ```
    pub fn release_fields_ok(&self) -> Result<(), String> {
        if let Some(target) = &self.target {
            let Some(parsed) = crate::release::Target::parse(target) else {
                return Err(format!("插件清单的目标 {target} 认不出。"));
            };
            if !self.os.iter().any(|os| os == parsed.os()) {
                return Err(format!("插件清单的目标 {target} 和支持的系统对不上。"));
            }
        }
        if let Some(source) = &self.source {
            source.check()?;
            if crate::release::Version::parse(&self.version).is_none() {
                return Err(format!(
                    "有更新来源的插件，版本号要写成 主.次.修订（现在是 {}）。",
                    self.version
                ));
            }
        }
        if self
            .min_host
            .as_deref()
            .is_some_and(|min| crate::release::Version::parse(min).is_none())
        {
            return Err("插件清单的最低 HHH 版本（minHost）写法不对。".into());
        }
        Ok(())
    }

    /// 编号是否合法：小写字母开头，只含 `a-z0-9-`，2–32 字符。
    ///
    /// # Examples
    ///
    /// ```
    /// assert!(hhh_im_protocol::platform::Manifest::id_ok("wechat-pad"));
    /// assert!(!hhh_im_protocol::platform::Manifest::id_ok("1bad"));
    /// assert!(!hhh_im_protocol::platform::Manifest::id_ok("a"));
    /// ```
    pub fn id_ok(id: &str) -> bool {
        let bytes = id.as_bytes();
        (2..=32).contains(&id.len())
            && bytes[0].is_ascii_lowercase()
            && bytes
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
    }

    /// 内部平台编号 `ext-<id>`，不会和内置平台或已下线平台撞名。
    pub fn platform_id(&self) -> String {
        format!("ext-{}", self.id)
    }

    /// 清单里的 IM 类型是否合规：没写，或写法同编号（[`Self::id_ok`]）。
    ///
    /// # Examples
    ///
    /// ```
    /// let mut manifest: hhh_im_protocol::Manifest = serde_json::from_str(
    ///     r#"{"manifestVersion":1,"id":"feishu-pro","name":"飞书 Pro","version":"0.1.0","command":"p","os":["windows"],"imType":"feishu"}"#,
    /// ).unwrap();
    /// assert!(manifest.im_type_ok());
    /// manifest.im_type = Some("Feishu".into());
    /// assert!(!manifest.im_type_ok());
    /// ```
    pub fn im_type_ok(&self) -> bool {
        self.im_type.as_deref().is_none_or(Self::id_ok)
    }
}

/// 头像形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AvatarShape {
    /// 圆形。
    Circle,
    /// 圆角方形。
    Rounded,
}

/// 平台历史能力：决定宿主要不要定时拉、能不能往上翻历史（protocol-v1 §5.1、§11.4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistoryMode {
    /// 消息随时能向平台重读（飞书/Slack 这类）。
    Server,
    /// 平台只推一次，不能翻历史（微信这类）：所有会话都存进 HHH，游标由宿主存。
    None,
}

/// 每秒 / 每分钟上限。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateBudget {
    /// 每秒最多几次。
    pub per_second: u32,
    /// 每分钟最多几次。
    pub per_minute: u32,
}

/// 发送附件的上限。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    /// 发送图片最大字节。
    pub image_upload_max_bytes: u64,
    /// 发送文件最大字节。
    pub file_upload_max_bytes: u64,
    /// 附件文件名最长字节（UTF-8）。
    pub attachment_name_max_bytes: u32,
}

/// 会话的叫法。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationNouns {
    /// 群。
    pub group: String,
    /// 私聊。
    pub direct: String,
    /// 频道。
    pub channel: String,
    /// 认不出类别。
    pub unknown: String,
}

/// 读取间隔默认值（秒）；字段同 hhh-im `ReadIntervals`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadIntervalsSecs {
    /// 正在看。
    pub watching: u64,
    /// 1 小时内有动静。
    pub active: u64,
    /// 1 天内有动静。
    pub quiet: u64,
    /// 1 周内有动静。
    pub dormant: u64,
    /// 更早。
    pub stale: u64,
    /// 会话资料与成员刷新。
    pub profile: u64,
    /// 人名与头像过期。
    pub people: u64,
    /// 话题跟踪天数。
    pub thread_days: u32,
}

/// 同时发送到会话的能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Broadcast {
    /// 平台原生支持（一条两处都有）；为假时内核发两次。
    pub native: bool,
    /// 带附件的回复也能同时发送。
    pub attachments: bool,
}

/// 话题相关平台事实。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Threads {
    /// 读会话时根消息带话题现状。
    pub activity_on_root: bool,
    /// 按话题读能只要某时间之后的回复。
    pub reads_after: bool,
    /// 同时发送到会话的能力；不支持为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcast: Option<Broadcast>,
}

/// 每条消息允许哪些操作的声明式规则（protocol-v1 §5.1）。宿主用通用代码执行。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ActionRules {
    /// 能在会话里引用回复。
    #[serde(default)]
    pub reply: bool,
    /// 能回复/新建话题。
    #[serde(default)]
    pub thread: bool,
    /// 能点表情。
    #[serde(default)]
    pub react: bool,
    /// 能撤回；null 表示不能。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recall: Option<OwnWithin>,
    /// 能编辑；null 表示不能。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<OwnWithin>,
}

/// “只对本人发的、在时限内”这类操作约束。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnWithin {
    /// 只能对本人发的消息做。
    #[serde(default)]
    pub own: bool,
    /// 平台时限（秒）；过了宿主不再显示这个操作，最终以平台答复为准。没有时限为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within_secs: Option<u64>,
}

/// 目录里没有的表情怎么显示。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlistedEmoji {
    /// 画成占位图标的表情类型（正则，JavaScript 语法）；为空时都写类型名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder_pattern: Option<String>,
    /// 占位图标的叫法。
    pub placeholder_label: String,
}

/// 表情规则（有表情能力时给）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmojiRules {
    /// 表情编号合法的正则（JavaScript 语法）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_pattern: Option<String>,
    /// 目录外的表情怎么显示。
    pub unlisted: UnlistedEmoji,
}

/// 平台说明（protocol-v1 §5.1）：宿主据此限速、分页、显示界面、算每条消息能做什么。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformDesc {
    /// 头像形状（必填）。
    pub avatar_shape: AvatarShape,
    /// 历史能力（必填）。
    pub history: HistoryMode,
    /// 每条消息允许的操作（必填）。
    pub actions: ActionRules,
    /// 在平台客户端里打开会话的链接模板，`{conversation}` 换成 URL 编码的会话编号。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_link: Option<String>,
    /// 下载图片时文件名的开头；默认用平台名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_name_prefix: Option<String>,
    /// 各类会话的叫法。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_nouns: Option<ConversationNouns>,
    /// 按会话读一页最多几条（history=server 必填）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_max: Option<u16>,
    /// 读取请求合计上限（history=server 必填）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reads: Option<RateBudget>,
    /// 下载资源的上限；平台不限为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<RateBudget>,
    /// 文件按多大一段读（支持续传）；不给就整份读。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_segment_bytes: Option<u64>,
    /// 发送附件上限（能发图片/文件时必填）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
    /// 默认读取间隔。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_defaults: Option<ReadIntervalsSecs>,
    /// 话题平台事实；平台有话题时给。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threads: Option<Threads>,
    /// 撤回的消息直接从历史消失。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted_messages_vanish: bool,
    /// 读消息时会随消息带回完整表情。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reactions_in_reads: bool,
    /// 不同账号表情目录不同。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub emoji_per_account: bool,
    /// 表情规则（有表情能力时给）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<EmojiRules>,
}

/// 一组能力开关（protocol-v1 §5.2）。没写出来的都算不支持。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// 会话能力。
    #[serde(default)]
    pub conversations: ConversationCaps,
    /// 消息读取能力。
    #[serde(default)]
    pub messages: MessageCaps,
    /// 人员能力。
    #[serde(default)]
    pub people: PeopleCaps,
    /// 发送能力；null 或全 false 即只读。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send: Option<SendCaps>,
    /// 能撤回。
    #[serde(default)]
    pub recall: bool,
    /// 能编辑。
    #[serde(default)]
    pub edit: bool,
    /// 表情能力。
    #[serde(default)]
    pub reactions: ReactionCaps,
    /// 资源能力。
    #[serde(default)]
    pub resources: ResourceCaps,
    /// 推送事件能力。
    #[serde(default)]
    pub events: EventCaps,
    /// 支持原始数据与重新翻译。
    #[serde(default)]
    pub retranslate: bool,
}

/// 会话能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConversationCaps {
    /// 能列出会话。
    #[serde(default)]
    pub list: bool,
    /// 能搜索会话。
    #[serde(default)]
    pub search: bool,
    /// 能列群成员。
    #[serde(default)]
    pub members: bool,
    /// 能定位已有私聊。
    #[serde(default)]
    pub find_direct: bool,
}

/// 消息读取能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MessageCaps {
    /// 能分页读消息。
    #[serde(default)]
    pub list: bool,
    /// 能补读单条消息。
    #[serde(default)]
    pub get: bool,
}

/// 人员能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PeopleCaps {
    /// 能按编号批量读人。
    #[serde(default)]
    pub get: bool,
    /// 能搜索人。
    #[serde(default)]
    pub search: bool,
}

/// 发送能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SendCaps {
    /// 能发文字。
    #[serde(default)]
    pub text: bool,
    /// 能带结构化提及。
    #[serde(default)]
    pub mentions: bool,
    /// 能提及所有人。
    #[serde(default)]
    pub mention_all: bool,
    /// 能发图片。
    #[serde(default)]
    pub image: bool,
    /// 能发文件。
    #[serde(default)]
    pub file: bool,
    /// 能按操作编号查询发送结果。
    #[serde(default)]
    pub lookup: bool,
}

impl SendCaps {
    /// 一项都不能发即只读。
    pub fn is_read_only(&self) -> bool {
        !(self.text || self.image || self.file)
    }
}

/// 表情能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReactionCaps {
    /// 能读表情。
    #[serde(default)]
    pub list: bool,
    /// 能点表情。
    #[serde(default)]
    pub add: bool,
    /// 能取消表情。
    #[serde(default)]
    pub remove: bool,
    /// 有表情目录。
    #[serde(default)]
    pub catalog: bool,
}

/// 资源能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResourceCaps {
    /// 能按段读消息资源。
    #[serde(default)]
    pub read: bool,
    /// 能读头像。
    #[serde(default)]
    pub avatars: bool,
}

/// 推送事件能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EventCaps {
    /// 推新消息。
    #[serde(default)]
    pub messages: bool,
    /// 推撤回（消失型平台）。
    #[serde(default)]
    pub deletions: bool,
    /// 推表情变化。
    #[serde(default)]
    pub reactions: bool,
    /// 推会话变化。
    #[serde(default)]
    pub conversations: bool,
    /// 推人员变化。
    #[serde(default)]
    pub people: bool,
}

/// 提供方内编号稳定的参数；缺少类型时沿用文字字段。
///
/// # Examples
/// ```
/// use hhh_im_protocol::platform::SettingDecl;
/// let field = SettingDecl { id: "endpoint".into(), label: "地址".into(), ..Default::default() };
/// assert!(field.field_type.is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SettingDecl {
    /// 字段编号。
    pub id: String,
    /// 显示名称。
    pub label: String,
    /// 值保存在系统凭据库。
    #[serde(default)]
    pub secret: bool,
    /// 是否必填。
    #[serde(default)]
    pub required: bool,
    /// 校验正则。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// 帮助文字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// 控件类型：text、textarea、number、switch、select、secret。
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub field_type: Option<String>,
    /// 字符串默认值；密钥不能声明默认值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// 数值下界。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    /// 数值上界。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    /// 数字旁的单位。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// 单选项。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SettingOption>,
    /// 分组与折叠说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<SettingGroup>,
    /// 折叠摘要中的简短名称。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_label: Option<String>,
    /// 另一个字段等于指定值时显示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_when: Option<SettingCondition>,
    /// 不能编辑时给用户的原因。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
    /// 数值不能小于这个字段的值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_less_than: Option<String>,
    /// 数值不能大于这个字段的值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_greater_than: Option<String>,
}

/// 一个单选项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingOption {
    /// 传给提供方的值。
    pub value: String,
    /// 显示名称。
    pub label: String,
}

/// 字段分组；折叠组默认收起。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingGroup {
    /// 分组编号。
    pub id: String,
    /// 分组名称。
    pub label: String,
    /// 默认折叠。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fold: bool,
    /// 展开时的说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// 不随折叠隐藏的补充说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// 简单相等条件，不能引用密钥。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingCondition {
    /// 另一个字段编号。
    pub field: String,
    /// 满足条件时的值。
    pub equals: String,
}

impl Capabilities {
    /// 校验能力组合是否自洽（protocol-v1 §5.2），不自洽时返回说明哪项不对。
    ///
    /// # Errors
    /// 组合不自洽时返回一句中文说明。
    pub fn check_consistent(&self, history: HistoryMode) -> Result<(), String> {
        match history {
            HistoryMode::Server if !self.messages.list => {
                return Err("history=server 必须声明 messages.list".into());
            }
            HistoryMode::None if !self.events.messages => {
                return Err("history=none 必须声明 events.messages".into());
            }
            HistoryMode::None if !self.conversations.list && !self.events.conversations => {
                return Err("history=none 且不能列会话时，必须声明 events.conversations".into());
            }
            _ => {}
        }
        Ok(())
    }

    /// 是不是只读连接（没有发送能力）。
    pub fn is_read_only(&self) -> bool {
        self.send.as_ref().is_none_or(SendCaps::is_read_only)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_ids_follow_the_rule() {
        assert!(Manifest::id_ok("wechat-pad"));
        assert!(Manifest::id_ok("ab"));
        assert!(!Manifest::id_ok("a"));
        assert!(!Manifest::id_ok("1x"));
        assert!(!Manifest::id_ok("Has-Upper"));
        assert!(!Manifest::id_ok("under_score"));
    }

    // IM 类型可选：写了就以 imType 往返、按编号的写法校验；没写时不输出这个字段，旧清单照常读。
    #[test]
    fn manifest_im_type_is_optional_and_follows_the_id_rule() {
        let base = r#"{"manifestVersion":1,"id":"feishu-pro","name":"飞书 Pro","version":"0.1.0","command":"p","os":["windows"]}"#;
        let old: Manifest = serde_json::from_str(base).unwrap();
        assert_eq!(old.im_type, None);
        assert!(old.im_type_ok());
        assert!(serde_json::to_value(&old).unwrap().get("imType").is_none());

        let typed: Manifest =
            serde_json::from_str(&base.replace(r#""os""#, r#""imType":"feishu","os""#)).unwrap();
        assert_eq!(typed.im_type.as_deref(), Some("feishu"));
        assert!(typed.im_type_ok());
        assert_eq!(serde_json::to_value(&typed).unwrap()["imType"], "feishu");

        for bad in ["Feishu", "f", "1feishu", "fei_shu", ""] {
            let manifest = Manifest {
                im_type: Some(bad.into()),
                ..typed.clone()
            };
            assert!(!manifest.im_type_ok(), "{bad:?} 不该算合规的 IM 类型");
        }
    }

    #[test]
    fn read_only_is_no_send_or_all_false() {
        let mut caps = Capabilities::default();
        assert!(caps.is_read_only(), "没有 send 即只读");
        caps.send = Some(SendCaps::default());
        assert!(caps.is_read_only(), "全 false 也是只读");
        caps.send = Some(SendCaps {
            text: true,
            ..SendCaps::default()
        });
        assert!(!caps.is_read_only());
    }

    #[test]
    fn capability_consistency_follows_history_mode() {
        let mut caps = Capabilities::default();
        assert!(caps.check_consistent(HistoryMode::Server).is_err());
        caps.messages.list = true;
        assert!(caps.check_consistent(HistoryMode::Server).is_ok());

        let mut push = Capabilities::default();
        assert!(push.check_consistent(HistoryMode::None).is_err());
        push.events.messages = true;
        // 不能列会话且不推会话 → 仍不自洽。
        assert!(push.check_consistent(HistoryMode::None).is_err());
        push.events.conversations = true;
        assert!(push.check_consistent(HistoryMode::None).is_ok());
    }

    #[test]
    fn platform_desc_omits_empty_optionals_but_keeps_required() {
        let d = PlatformDesc {
            avatar_shape: AvatarShape::Rounded,
            history: HistoryMode::None,
            actions: ActionRules {
                recall: Some(OwnWithin {
                    own: true,
                    within_secs: Some(120),
                }),
                ..ActionRules::default()
            },
            chat_link: None,
            image_name_prefix: None,
            conversation_nouns: None,
            page_max: None,
            reads: None,
            downloads: None,
            file_segment_bytes: None,
            limits: None,
            read_defaults: None,
            threads: None,
            deleted_messages_vanish: false,
            reactions_in_reads: false,
            emoji_per_account: false,
            emoji: None,
        };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["avatarShape"], "rounded");
        assert_eq!(v["history"], "none");
        assert_eq!(v["actions"]["recall"]["withinSecs"], 120);
        assert!(v.get("chatLink").is_none());
        assert!(v.get("deletedMessagesVanish").is_none(), "false 不序列化");
    }
}
