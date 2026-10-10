//! 一个最小但完整的假插件：实现 HHH-IM-Protocol，用来自测 `hhh-im-check` 工具，
//! 也给 hhh-im 的 provider 适配器做端到端联调。
//!
//! 它是一个 `history=none`（只推一次）的平台：有一个会话，`session/open` 后推一条消息；
//! 支持发文字、按段读一张小图、重新翻译。不联网、全在内存，带 `--mock` 时行为一致（本就不联网）。
//!
//! 运行：`cargo run -p hhh-im-protocol --example fake_provider --features transport`（通常由
//! `hhh-im-check` 作为子进程启动）。

use std::io::{BufRead, Write};

use hhh_im_protocol::rpc::{RpcId, RpcMessage, codec};
use serde_json::{Value, json};

fn main() {
    // 一行一条：从 stdin 读、往 stdout 写。日志走 stderr。
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let mut pushed = false;
    let mut next_push_id = 1_000_000i64;

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(msg) = codec::decode_line(line) else {
            eprintln!("fake_provider: 收到非法 JSON-RPC，跳过");
            continue;
        };
        if msg.is_response() {
            // 宿主对我们推送/凭据的确认，忽略即可。
            continue;
        }
        let Some(method) = msg.method.as_deref() else {
            continue;
        };
        let params = msg.params.clone().unwrap_or(Value::Null);

        // 通知：无需回复。
        if msg.is_notification() {
            if method == "$/cancel" {
                // 这个假插件的请求都即时返回，不用处理取消。
            }
            continue;
        }

        let id = msg.id.clone().expect("请求有编号");
        let reply = handle(method, &params);
        send(&mut out, RpcMessage::ok(id, reply));

        // session/open 成功后，推一条消息 + 一个会话，演示 history=none 的推送。
        if method == "session/open" && !pushed {
            pushed = true;
            let push = json!({
                "events": [
                    { "type": "conversation", "conversation": {
                        "id": "c-demo", "kind": "group", "name": "演示群" } },
                    { "type": "messages", "conversationId": "c-demo", "messages": [
                        demo_message("m-1", "你好，这是推送来的第一条消息") ] }
                ],
                "syncState": "cursor-1"
            });
            next_push_id += 1;
            send(
                &mut out,
                RpcMessage::request(RpcId::Number(next_push_id), "events/push", push),
            );
        }
    }
}

fn send(out: &mut std::io::Stdout, msg: RpcMessage) {
    if let Ok(line) = codec::encode_line(&msg) {
        let _ = out.write_all(line.as_bytes());
        let _ = out.flush();
    }
}

fn demo_message(id: &str, text: &str) -> Value {
    json!({
        "id": id,
        "conversationId": "c-demo",
        "sender": { "id": "u-friend", "kind": "user", "name": "小伙伴" },
        "createdAtMs": 1_700_000_000_000i64,
        "content": [ { "type": "text", "text": text } ],
        "raw": { "format": "fake-v1", "data": text }
    })
}

fn handle(method: &str, params: &Value) -> Value {
    match method {
        "initialize" => json!({
            "protocolVersion": 1,
            "provider": { "id": "fake", "name": "假插件", "version": "0.1.0" },
            "platform": {
                "avatarShape": "rounded",
                "history": "none",
                "actions": { "reply": false, "thread": false, "react": false, "recall": null, "edit": null }
            },
            "capabilities": {
                "conversations": { "list": false, "search": false, "members": false, "findDirect": false },
                "messages": { "list": false, "get": false },
                "people": { "get": true, "search": false },
                "send": { "text": true, "mentions": false, "mentionAll": false, "image": false, "file": false, "lookup": false },
                "recall": false,
                "edit": false,
                "reactions": { "list": false, "add": false, "remove": false, "catalog": false },
                "resources": { "read": true, "avatars": false },
                "events": { "messages": true, "deletions": false, "reactions": false, "conversations": true, "people": true },
                "retranslate": true
            },
            "settings": []
        }),
        "session/open" => json!({
            "state": "online",
            "account": { "id": "me", "name": "我" }
        }),
        "ping" | "shutdown" | "session/logout" => json!({}),
        "conversations/get" => json!({
            "id": "c-demo", "kind": "group", "name": "演示群"
        }),
        "people/get" => {
            let ids = params["ids"].as_array().cloned().unwrap_or_default();
            let found: Vec<Value> = ids
                .iter()
                .filter_map(|v| v.as_str())
                .map(|id| json!({ "id": id, "kind": "user", "name": format!("人-{id}") }))
                .collect();
            json!({ "found": found, "missing": [] })
        }
        "messages/send" => {
            let op = params["operationId"].as_str().unwrap_or("op");
            json!({
                "state": "accepted",
                "message": demo_message(&format!("sent-{op}"), "（已发送）")
            })
        }
        "resources/read" => {
            // 一张 1x1 透明 PNG（固定字节），按整段返回。
            const PNG: &[u8] = &[
                0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
                0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
                0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78,
                0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
                0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
            ];
            let offset = params["offset"].as_u64().unwrap_or(0) as usize;
            let length = params["length"].as_u64().unwrap_or(PNG.len() as u64) as usize;
            let end = (offset + length).min(PNG.len());
            let slice = if offset < PNG.len() {
                &PNG[offset..end]
            } else {
                &[]
            };
            json!({
                "data": base64_encode(slice),
                "total": PNG.len(),
                "eof": end >= PNG.len()
            })
        }
        "messages/retranslate" => {
            let items = params["messages"].as_array().cloned().unwrap_or_default();
            let msgs: Vec<Value> = items
                .iter()
                .map(|it| {
                    let id = it["id"].as_str().unwrap_or("m");
                    let data = it["raw"]["data"].as_str().unwrap_or("");
                    demo_message(id, &format!("（重译）{data}"))
                })
                .collect();
            json!({ "messages": msgs })
        }
        _ => json!({}),
    }
}

/// 最小 base64 编码（标准字母表），避免给示例引依赖。
fn base64_encode(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
