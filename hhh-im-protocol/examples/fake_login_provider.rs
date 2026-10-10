//! 一个带登录流程的假插件：实现 HHH-IM-Protocol，用来端到端联调 HHH Friend 的「安装 + 登录」界面。
//!
//! 像微信那样推送式、要扫码登录的平台：
//! - `session/open` 没凭据时返回「需要登录」；登录过（内存里已登录）或带凭据时返回「在线」。
//! - `login/start` 回一个流程编号，随即推一张二维码步骤；~2 秒后（模拟扫码确认）`credential/save` 存一份不透明凭据、
//!   推 `login/finished`（这时已在线），随即推一个群 + 一条消息、一个还没说过话的私聊（「小伙伴」）；带凭据重开时在
//!   `session/open` 之后推。没声明搜索，宿主在本机替它搜群、搜人。
//!
//! 运行：`cargo run -p hhh-im-protocol --example fake_login_provider --features transport`
//! （通常由 HHH Friend 作为子进程启动）。不联网、全在内存。
//!
//! 验收崩溃重启用的测试开关（契约 provider-protocol v4）：宿主给的工作目录（临时目录）里出现这些文件时照做——
//! `crash`：删掉文件、退出（像崩溃）；`hang`：删掉文件、之后什么请求都不回（像卡死）；
//! `garble`：删掉文件、往标准输出写 3 行不是消息的内容；`crash-on-start`（不删）：一启动就退出（像一直起不来）。
//!
//! `--slow-start`：启动后先等 1.5 秒再回话（像启动慢的插件），验收安装框的“正在检查插件…”用（契约 friend/im-list v1）。
//! `--no-note`：二维码一步不声明登录提示，验收“没声明提示的插件与原来一样”用（契约 provider-protocol v8）。

use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use hhh_im_protocol::rpc::{RpcId, RpcMessage, codec};
use serde_json::{Value, json};

/// 1x1 透明 PNG（固定字节）；二维码步骤与 `resources/read` 都用它，放大后是一块纯色，够验证 base64 渲染通路。
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

fn main() {
    if std::path::Path::new("crash-on-start").exists() {
        eprintln!("fake_login_provider: 测试开关 crash-on-start，一启动就退出");
        std::process::exit(2);
    }
    let stdin = std::io::stdin();
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let hung = Arc::new(AtomicBool::new(false));
    watch_switches(out.clone(), hung.clone());
    // 内存里是否已登录（登录成功后，哪怕 session/open 不带凭据也在线，模拟真实插件的内存会话）。
    let logged_in = Arc::new(AtomicBool::new(false));
    let pushed = Arc::new(AtomicBool::new(false));
    let next_push_id = Arc::new(AtomicI64::new(1_000_000));
    // `--v2`：演示新版本（版本号 0.2.0，多一项“可撤回”），验收更新用（契约 provider-protocol v2）。
    let v2 = std::env::args().any(|arg| arg == "--v2");
    let no_note = std::env::args().any(|arg| arg == "--no-note");
    if std::env::args().any(|arg| arg == "--slow-start") {
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(msg) = codec::decode_line(line) else {
            eprintln!("fake_login_provider: 收到非法 JSON-RPC，跳过");
            continue;
        };
        if msg.is_response() {
            continue; // 宿主对我们推送 / 凭据的确认，忽略。
        }
        let Some(method) = msg.method.as_deref() else {
            continue;
        };
        let params = msg.params.clone().unwrap_or(Value::Null);

        if msg.is_notification() {
            // login/submit、login/cancel、$/cancel：这个假插件用定时器自动完成登录，不处理。
            continue;
        }

        let id = msg.id.clone().expect("请求有编号");
        if hung.load(Ordering::SeqCst) {
            continue; // 测试开关 hang：像卡死，什么都不回。
        }
        let online = logged_in.load(Ordering::SeqCst)
            || params.get("credential").and_then(Value::as_str).is_some();
        if method == "session/open" {
            // 宿主交还的同步位置写进标准错误（宿主并进自己的日志），验收“重启后接着同步”时看它。
            let sync_state = params
                .get("syncState")
                .and_then(Value::as_str)
                .unwrap_or("（没有）");
            eprintln!("fake_login_provider: session/open 收到同步位置 {sync_state}");
        }
        let reply = handle(method, &params, online, v2);
        send(&out, RpcMessage::ok(id, reply));

        // 在线的 session/open 之后推一次会话 + 消息（history=none 的推送）。
        if method == "session/open" && online && !pushed.swap(true, Ordering::SeqCst) {
            push_demo(&out, &next_push_id);
        }

        // 开始登录：推二维码，起一个定时器模拟扫码确认后完成登录。
        if method == "login/start" {
            let flow_id = params
                .get("flowId")
                .and_then(Value::as_str)
                .unwrap_or("flow-1")
                .to_owned();
            let mut step = json!({ "kind": "qrCode", "image": base64_encode(PNG), "expiresAtMs": 1_700_000_120_000i64, "hint": "用微信扫一扫登录（演示：约 2 秒后自动完成）" });
            if !no_note {
                step["note"] = json!({ "text": "演示插件声明的登录提示：扫码后在手机上确认即可。", "tone": "info" });
            }
            notify(
                &out,
                "login/step",
                json!({ "flowId": flow_id, "step": step }),
            );
            let out = out.clone();
            let logged_in = logged_in.clone();
            let pushed = pushed.clone();
            let next_push_id = next_push_id.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(2000));
                logged_in.store(true, Ordering::SeqCst);
                // 存一份不透明凭据（请求，宿主会回；写进系统凭据库供下次冷启动）。
                notify_request(
                    &out,
                    "credential/save",
                    json!({ "credential": "fake-login-token" }),
                );
                // 登录完成：这时已在线（协议 §6.1），宿主用这里给的账号接上 hub，不再问 session/open；随即推会话。
                notify(
                    &out,
                    "login/finished",
                    json!({
                        "flowId": flow_id,
                        "account": { "id": "me", "name": "我（扫码登录）" }
                    }),
                );
                if !pushed.swap(true, Ordering::SeqCst) {
                    push_demo(&out, &next_push_id);
                }
            });
        }
    }
}

/// 每 300 毫秒看一次工作目录里的测试开关文件（见文件头）。
fn watch_switches(out: Arc<Mutex<std::io::Stdout>>, hung: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(300));
            if std::fs::remove_file("crash").is_ok() {
                eprintln!("fake_login_provider: 测试开关 crash，退出");
                std::process::exit(3);
            }
            if std::fs::remove_file("hang").is_ok() {
                eprintln!("fake_login_provider: 测试开关 hang，之后不再回任何请求");
                hung.store(true, Ordering::SeqCst);
            }
            if std::fs::remove_file("garble").is_ok() {
                eprintln!("fake_login_provider: 测试开关 garble，往标准输出写 3 行乱码");
                let mut guard = out.lock().unwrap();
                for line in ["not json 1", "not json 2", "not json 3"] {
                    let _ = writeln!(guard, "{line}");
                }
                let _ = guard.flush();
            }
        }
    });
}

/// 推一个演示群 + 一条消息，再推一个还没说过话的私聊（像通讯录里的好友：只有资料、没有消息）。
fn push_demo(out: &Arc<Mutex<std::io::Stdout>>, next_push_id: &Arc<AtomicI64>) {
    let push = json!({
        "events": [
            { "type": "conversation", "conversation": demo_conversation("c-demo") },
            { "type": "messages", "conversationId": "c-demo", "messages": [ demo_message("m-1", "你好，这是插件推送来的第一条消息") ] },
            { "type": "conversation", "conversation": demo_conversation("c-friend") }
        ],
        "syncState": "cursor-1"
    });
    let id = next_push_id.fetch_add(1, Ordering::SeqCst);
    send(
        out,
        RpcMessage::request(RpcId::Number(id), "events/push", push),
    );
}

fn send(out: &Arc<Mutex<std::io::Stdout>>, msg: RpcMessage) {
    if let Ok(line) = codec::encode_line(&msg) {
        let mut guard = out.lock().unwrap();
        let _ = guard.write_all(line.as_bytes());
        let _ = guard.flush();
    }
}

/// 发一个通知（无需回复）。
fn notify(out: &Arc<Mutex<std::io::Stdout>>, method: &str, params: Value) {
    send(out, RpcMessage::notification(method, params));
}

/// 发一个请求（宿主会回；我们忽略回应）。用于 `credential/save` 这类需要确认的推送。
fn notify_request(out: &Arc<Mutex<std::io::Stdout>>, method: &str, params: Value) {
    send(
        out,
        RpcMessage::request(RpcId::Number(2_000_000), method, params),
    );
}

/// 演示插件的两个会话：群「演示群（插件）」与和「小伙伴」的私聊；按编号给，不认识的编号给群。
fn demo_conversation(id: &str) -> Value {
    if id == "c-friend" {
        json!({ "id": "c-friend", "kind": "direct", "name": "小伙伴", "peer": "u-friend", "private": true })
    } else {
        json!({ "id": "c-demo", "kind": "group", "name": "演示群（插件）" })
    }
}

fn demo_message(id: &str, text: &str) -> Value {
    json!({
        "id": id,
        "conversationId": "c-demo",
        "sender": { "id": "u-friend", "kind": "user", "name": "小伙伴" },
        "createdAtMs": 1_700_000_000_000i64,
        "content": [ { "type": "text", "text": text } ],
        "raw": { "format": "fake-login-v1", "data": text }
    })
}

fn handle(method: &str, params: &Value, online: bool, v2: bool) -> Value {
    match method {
        "initialize" => json!({
            "protocolVersion": 1,
            "provider": { "id": "wxdemo", "name": "微信（演示插件）", "version": if v2 { "0.2.0" } else { "0.1.0" } },
            "platform": {
                "avatarShape": "rounded",
                "history": "none",
                "actions": { "reply": true, "thread": false, "react": false, "recall": if v2 { json!({ "own": true }) } else { Value::Null }, "edit": null }
            },
            "capabilities": {
                "conversations": { "list": false, "search": false, "members": false, "findDirect": false },
                "messages": { "list": false, "get": false },
                "people": { "get": true, "search": false },
                "send": { "text": true, "mentions": false, "mentionAll": false, "image": false, "file": false, "lookup": false },
                "recall": v2,
                "edit": false,
                "reactions": { "list": false, "add": false, "remove": false, "catalog": false },
                "resources": { "read": true, "avatars": false },
                "events": { "messages": true, "deletions": false, "reactions": false, "conversations": true, "people": true },
                "retranslate": true
            },
            "settings": []
        }),
        // 在线（已登录或带凭据）→ online；否则 → 需要登录（扫码）。
        "session/open" => {
            if online {
                json!({ "state": "online", "account": { "id": "me", "name": "我（扫码登录）" } })
            } else {
                json!({ "state": "needsLogin", "reason": "noCredential", "message": "需要登录微信：点“登录”扫码。" })
            }
        }
        "login/start" => json!({ "flowId": "flow-1" }),
        "ping" | "shutdown" | "session/logout" => json!({}),
        "conversations/get" => {
            demo_conversation(params["conversationId"].as_str().unwrap_or("c-demo"))
        }
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
            // 回执发出去的那条：发送者是本人，正文照原文（像真实平台那样回显）。
            let op = params["operationId"].as_str().unwrap_or("op");
            let text: String = params["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|block| block["text"].as_str())
                .collect();
            json!({
                "state": "accepted",
                "message": {
                    "id": format!("sent-{op}"),
                    "conversationId": "c-demo",
                    "sender": { "id": "me", "kind": "user", "name": "我（扫码登录）" },
                    "createdAtMs": now_ms(),
                    "content": [ { "type": "text", "text": text } ],
                    "raw": { "format": "fake-login-v1", "data": text }
                }
            })
        }
        "resources/read" => {
            let offset = params["offset"].as_u64().unwrap_or(0) as usize;
            let length = params["length"].as_u64().unwrap_or(PNG.len() as u64) as usize;
            let end = (offset + length).min(PNG.len());
            let slice = if offset < PNG.len() {
                &PNG[offset..end]
            } else {
                &[]
            };
            json!({ "data": base64_encode(slice), "total": PNG.len(), "eof": end >= PNG.len() })
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

/// 现在的 Unix 毫秒。
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// 最小 base64 编码（标准字母表），不引依赖。
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
