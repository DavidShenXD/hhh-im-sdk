//! 跨账号携带的伙伴代发署名。只描述来源的显示快照，不授予执行记录或平台身份。

use serde::{Deserialize, Deserializer, Serialize};

/// 一条消息附带的伙伴显示快照；与消息一起保存和读取，不靠接收方的本机伙伴编号解析。
///
/// # Examples
///
/// ```
/// use hhh_im_protocol::attribution::AgentAttribution;
/// let value = serde_json::json!({"name":"派派","automatic":true});
/// let attribution: AgentAttribution = serde_json::from_value(value.clone()).unwrap();
/// assert!(attribution.problem().is_none());
/// assert_eq!(serde_json::to_value(attribution).unwrap(), value);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAttribution {
    /// 发出时的伙伴名字；不能是空白、控制字符或超过名字预算。
    pub name: String,
    /// 可携带的静态头像 data URL；只收 PNG、JPEG、WebP，不收本机路径、外部地址或 SVG。
    /// 没有自定义头像时省略，由 Friend 用默认头像。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    /// 这一版是否自动发送；false 省略。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub automatic: bool,
}

impl AgentAttribution {
    /// 检查待发送署名；返回可供插件自检的原因，不改正文。
    pub fn problem(&self) -> Option<&'static str> {
        if self.name.trim().is_empty()
            || self.name.len() > crate::budget::AGENT_NAME_MAX_BYTES
            || self.name.chars().any(char::is_control)
        {
            return Some("伙伴署名的名字为空、含控制字符或超过预算");
        }
        if self
            .avatar
            .as_deref()
            .is_some_and(|avatar| !avatar_ok(avatar))
        {
            return Some("伙伴署名的头像必须是预算内的静态图片 data URL");
        }
        None
    }
}

/// 检查无需联网即可携带的头像；浏览器解码失败时使用默认头像。
fn avatar_ok(avatar: &str) -> bool {
    if avatar.len() > crate::budget::AGENT_AVATAR_MAX_BYTES {
        return false;
    }
    let encoded = [
        "data:image/png;base64,",
        "data:image/jpeg;base64,",
        "data:image/webp;base64,",
    ]
    .into_iter()
    .find_map(|prefix| avatar.strip_prefix(prefix));
    encoded.is_some_and(|data| {
        let payload = data.trim_end_matches('=');
        payload.len() >= 2
            && data.len().is_multiple_of(4)
            && data.len() - payload.len() <= 2
            && payload
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/'))
    })
}

/// 署名读不懂时只放弃这一份附注，正文与同批消息照常读取；坏头像不影响有效名字。
pub(crate) fn receive<'de, D>(deserializer: D) -> Result<Option<Box<AgentAttribution>>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(mut raw) = raw else { return Ok(None) };
    if let Some(object) = raw.as_object_mut() {
        let invalid = object
            .get("avatar")
            .is_some_and(|avatar| !avatar.is_null() && !avatar.as_str().is_some_and(avatar_ok));
        if invalid {
            tracing::warn!("收到的伙伴署名头像不可携带，使用默认头像");
            object.remove("avatar");
        }
    }
    let value = match serde_json::from_value::<AgentAttribution>(raw) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(%error, "收到的伙伴署名形状无效，只保留消息正文");
            return Ok(None);
        }
    };
    if let Some(problem) = value.problem() {
        tracing::warn!(problem, "收到的伙伴署名无效，只保留消息正文");
        return Ok(None);
    }
    Ok(Some(Box::new(value)))
}
