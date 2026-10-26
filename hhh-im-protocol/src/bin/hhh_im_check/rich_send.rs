//! 只在显式授权的测试会话里验证插件声明的格式发送；unknown 不能算格式验收通过。
use super::*;
use hhh_im_protocol::content::{RichNode as Node, rich_outgoing};
use hhh_im_protocol::platform::{RichFormat, RichSendCaps};
use hhh_im_protocol::types::Outgoing;

fn content(caps: &RichSendCaps) -> Option<Outgoing> {
    let mut root = Vec::new();
    for format in &caps.formats {
        let text = || {
            vec![Node::Text {
                value: "格式验收".into(),
            }]
        };
        let node = match format {
            RichFormat::Strong => Node::Strong { children: text() },
            RichFormat::Emphasis => Node::Emphasis { children: text() },
            RichFormat::Underline => Node::Underline { children: text() },
            RichFormat::Delete => Node::Delete { children: text() },
            RichFormat::InlineCode => Node::InlineCode {
                value: "code".into(),
            },
            RichFormat::Link => Node::Link {
                url: "https://example.com/".into(),
                children: text(),
            },
            RichFormat::Blockquote => Node::Blockquote {
                children: vec![Node::Paragraph { children: text() }],
            },
            RichFormat::List => Node::List {
                ordered: false,
                start: None,
                children: vec![Node::ListItem {
                    children: vec![Node::Paragraph { children: text() }],
                }],
            },
            RichFormat::Code => Node::Code {
                value: "code".into(),
                lang: None,
            },
            RichFormat::Unknown => continue,
        };
        root.push(
            if matches!(
                format,
                RichFormat::Blockquote | RichFormat::List | RichFormat::Code
            ) {
                node
            } else {
                Node::Paragraph {
                    children: vec![node],
                }
            },
        );
    }
    (!root.is_empty()).then(|| rich_outgoing(root))
}

pub(super) async fn check(
    conn: &Arc<Connection>,
    report: &mut Report,
    conversation: Option<&str>,
    caps: Option<&RichSendCaps>,
) {
    let Some(content) = caps.and_then(content) else {
        report.skip("格式正文发送", "插件未声明可识别的发送格式");
        return;
    };
    let Some(conversation) = conversation else {
        report.skip("格式正文发送", "未给 --allow-send，跳过真实发送");
        return;
    };
    let params = json!({"operationId":format!("check-rich-{}",std::process::id()),"target":{"type":"conversation","conversationId":conversation},"content":[content]});
    match conn.call("messages/send", params).await {
        Ok(value) if value["state"] == "accepted" => report.pass("格式正文发送"),
        Ok(value) if value["state"] == "unknown" => {
            report.skip("格式正文发送", "平台未确认是否发出；保留现场，不自动重发")
        }
        Ok(value) => report.fail("格式正文发送", format!("state={:?}", value["state"])),
        Err(error) => report.fail("格式正文发送", error.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hhh_im_protocol::transport::LinePipe;

    #[tokio::test]
    async fn rich_probe_requires_authorization_and_does_not_pass_an_unknown_outcome() {
        let (host_write, plugin_read) = tokio::io::duplex(4096);
        let (plugin_write, host_read) = tokio::io::duplex(4096);
        let host = Connection::new(Arc::new(LinePipe::new(host_write, host_read, || {})));
        let plugin = Connection::new(Arc::new(LinePipe::new(plugin_write, plugin_read, || {})));
        let caps = RichSendCaps {
            formats: vec![RichFormat::Strong],
            edit: false,
        };
        let mut report = Report::new();
        check(&host, &mut report, None, Some(&caps)).await;
        check(&host, &mut report, Some("test"), None).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(10), plugin.next_incoming())
                .await
                .is_err()
        );
        for outcome in ["unknown", "accepted"] {
            let reply = async {
                let Incoming::Request { id, method, params } =
                    plugin.next_incoming().await.unwrap()
                else {
                    panic!("需要请求")
                };
                assert_eq!(method, "messages/send");
                assert_eq!(params["target"]["conversationId"], "test");
                assert_eq!(params["content"][0]["type"], "rich");
                let body = params.to_string();
                assert!(body.contains("strong") && !body.contains("underline"));
                plugin.respond(id, json!({"state":outcome})).await.unwrap();
            };
            tokio::join!(check(&host, &mut report, Some("test"), Some(&caps)), reply);
        }
        assert!(
            report
                .rows
                .iter()
                .take(3)
                .all(|(_, status, _)| *status == Status::Skip)
        );
        assert!(
            report
                .rows
                .last()
                .is_some_and(|(_, status, _)| *status == Status::Pass)
        );
    }
}
