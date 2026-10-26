//! 可选、只读的会话分组协议。游标由插件解释；空页不表示读取结束。

use serde::{Deserialize, Serialize};

/// 分组能力；旧插件缺省全部关闭。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupCaps {
    /// 能分页列出分组。
    #[serde(default)]
    pub list: bool,
    /// 能分页读取组内完整会话。
    #[serde(default)]
    pub members: bool,
    /// 协商后能推送失效通知。
    #[serde(default)]
    pub changes: bool,
}

impl GroupCaps {
    /// 是否没有声明任何能力。
    pub fn is_empty(&self) -> bool {
        !self.list && !self.members && !self.changes
    }
}

/// 原平台的分组种类，不推断特定平台的系统筛选。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GroupKind {
    /// 用户标签。
    Label,
    /// 平台自动分组。
    Smart,
}

/// 一个账号内的分组；排序键按 Unicode 码点比较，相同键按编号排序。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationGroup {
    /// 不透明编号。
    pub id: String,
    /// 原平台名字。
    pub name: String,
    /// 平台翻译好的排序键，缺省排在有键的项目之后。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_key: Option<String>,
    /// 标签或自动分组。
    pub kind: GroupKind,
}

/// 分组目录与成员共用分页信封。只有 nextCursor 缺省才表示结束。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupPage<T> {
    /// 本页项目；允许为空。
    pub items: Vec<T>,
    /// 不透明续页游标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// 存在无法表示为会话的成员时不能把结果冒充完整列表。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub incomplete: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn groups_are_opt_in_and_old_initialization_is_accepted() {
        let caps: crate::Capabilities = serde_json::from_value(json!({})).unwrap();
        assert!(caps.conversation_groups.is_empty());
        assert!(
            serde_json::to_value(caps)
                .unwrap()
                .get("conversationGroups")
                .is_none()
        );
        let params: crate::method::InitializeParams = serde_json::from_value(json!({"protocolVersions":[1],"host":{"name":"old","version":"1"},"locale":"zh-CN","tempDir":"tmp","network":{}})).unwrap();
        assert!(!params.conversation_groups);
        let mut caps = crate::Capabilities::default();
        caps.messages.list = true;
        caps.conversation_groups.members = true;
        assert!(
            caps.check_consistent(crate::platform::HistoryMode::Server)
                .is_err()
        );
        caps.conversation_groups.list = true;
        assert!(
            caps.check_consistent(crate::platform::HistoryMode::Server)
                .is_ok()
        );
    }
    #[test]
    fn groups_empty_page_retains_cursor_and_event_roundtrips() {
        let page: GroupPage<ConversationGroup> =
            serde_json::from_value(json!({"items":[],"nextCursor":"opaque"})).unwrap();
        assert_eq!(page.next_cursor.as_deref(), Some("opaque"));
        assert!(!page.incomplete);
        let event = json!({"type":"conversationGroupsChanged","catalog":true,"groupIds":["a"]});
        let parsed: crate::Event = serde_json::from_value(event.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), event);
    }
}
