use hhh_im_protocol::method::{ActionDecl, ActionFinishedParams, action_problem};
use serde_json::json;

#[test]
fn shortcuts_and_opened_conversations_are_optional_and_round_trip() {
    for value in [
        json!({"id":"add", "label":"加好友", "scope":"account"}),
        json!({"id":"custom", "label":"邀请", "scope":"account", "shortcut":true}),
    ] {
        let declaration: ActionDecl = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(action_problem(&declaration), None);
        assert_eq!(serde_json::to_value(declaration).unwrap(), value);
    }
    for value in [
        json!({"flowId":"x"}),
        json!({"flowId":"x", "openConversationId":"opaque-c:123"}),
    ] {
        let done: ActionFinishedParams = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(done).unwrap(), value);
    }
    let wrong: ActionDecl = serde_json::from_value(
        json!({"id":"x", "label":"成员", "scope":"conversation", "shortcut":true}),
    )
    .unwrap();
    assert!(
        action_problem(&wrong).is_some(),
        "会话操作不能冒充账号快捷入口"
    );
}
