use hhh_im_protocol::{
    AgentAttribution, Message,
    method::MessagesSendParams,
    platform::{Capabilities, HistoryMode},
};
use serde_json::{Value, json};

fn message(attribution: Option<Value>) -> Value {
    let mut value = json!({
        "id":"m1","conversationId":"c1","sender":{"id":"u1","kind":"user"},
        "createdAtMs":1,"content":[{"type":"text","text":"保留正文"}]
    });
    if let Some(attribution) = attribution {
        value["agentAttribution"] = attribution;
    }
    value
}

#[test]
fn attribution_is_optional_and_new_messages_and_send_requests_round_trip() {
    let old = message(None);
    let parsed: Message = serde_json::from_value(old.clone()).unwrap();
    assert!(parsed.agent_attribution.is_none());
    assert_eq!(serde_json::to_value(parsed).unwrap(), old);
    let snapshot =
        json!({"name":"派派","automatic":true,"avatar":"data:image/png;base64,aW1hZ2U="});
    let new = message(Some(snapshot.clone()));
    let parsed: Message = serde_json::from_value(new.clone()).unwrap();
    assert_eq!(parsed.agent_attribution.unwrap().name, "派派");
    let parsed: Message = serde_json::from_value(new.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), new);
    for attribution in [None, Some(snapshot)] {
        let mut expected = json!({"operationId":"op","target":{"type":"conversation","conversationId":"c1"},"content":[{"type":"text","text":"正文"}]});
        if let Some(value) = attribution {
            expected["agentAttribution"] = value;
        }
        let parsed: MessagesSendParams = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), expected);
    }
}

#[test]
fn invalid_attribution_never_discards_the_message_body() {
    for bad in [
        json!("bad"),
        json!({"name":42}),
        json!({"name":""}),
        json!({"name":"\n派派"}),
        json!({"name":"x".repeat(hhh_im_protocol::budget::AGENT_NAME_MAX_BYTES + 1)}),
        json!({"name":"派派","automatic":"future"}),
    ] {
        let parsed: Message = serde_json::from_value(message(Some(bad))).unwrap();
        assert!(parsed.agent_attribution.is_none());
        assert_eq!(
            serde_json::to_value(parsed).unwrap()["content"][0]["text"],
            "保留正文"
        );
    }
}

#[test]
fn bad_avatars_keep_the_name_and_never_reference_a_senders_machine_or_remote_server() {
    for avatar in [
        json!("file:///private/avatar.png"),
        json!("/api/friends/1/avatar"),
        json!("https://example.test/avatar.png"),
        json!("data:image/svg+xml;base64,PHN2Zz4="),
        json!("data:image/png;base64,!bad"),
        json!("x".repeat(hhh_im_protocol::budget::AGENT_AVATAR_MAX_BYTES + 1)),
        json!(42),
        json!({}),
        json!("data:image/png;base64,===="),
        json!("data:image/png;base64,a=b="),
    ] {
        let parsed: Message =
            serde_json::from_value(message(Some(json!({"name":"派派","avatar":avatar})))).unwrap();
        let attribution = parsed.agent_attribution.unwrap();
        assert_eq!(attribution.name, "派派");
        assert!(attribution.avatar.is_none());
    }
    let manual: AgentAttribution = serde_json::from_value(json!({"name":"派派"})).unwrap();
    assert!(!manual.automatic);
    assert_eq!(
        serde_json::to_value(manual).unwrap(),
        json!({"name":"派派"})
    );
}

#[test]
fn attribution_capabilities_are_opt_in_and_require_real_message_operations() {
    let old: Capabilities = serde_json::from_value(json!({"messages":{"list":true}})).unwrap();
    assert!(!old.agent_attribution.send && !old.agent_attribution.receive);
    assert!(
        serde_json::to_value(old)
            .unwrap()
            .get("agentAttribution")
            .is_none()
    );
    let mut caps: Capabilities = serde_json::from_value(json!({
        "messages":{"list":true},"agentAttribution":{"send":true,"receive":true}
    }))
    .unwrap();
    assert!(caps.check_consistent(HistoryMode::Server).is_err());
    caps.send = Some(serde_json::from_value(json!({"text":true})).unwrap());
    assert!(caps.check_consistent(HistoryMode::Server).is_ok());
    assert_eq!(
        serde_json::to_value(caps).unwrap()["agentAttribution"],
        json!({"send":true,"receive":true})
    );
}
