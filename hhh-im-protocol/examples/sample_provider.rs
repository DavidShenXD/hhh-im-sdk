//! 示例 IM：把契约 provider-protocol v12 的新声明与新方法都用上的演示插件，验收与插件作者参照用。
//!
//! - 两种登录方式：通过中继登录、使用自己的应用（声明本机回调路径）。插件自带一个本机回环的假授权页：登录时推“打开登录页”
//!   一步，页上点“同意”——中继方式由插件自己完成登录；自己的应用方式带着 `code`、`state` 跳回宿主给的回调地址，宿主经
//!   `login/callback` 交回来，核对 `state` 后完成。存凭据时带 `renewAtMs`（60 秒后），宿主到时叫 `session/renew`。
//! - 会话发现（`conversations/recentActivity`）、开放读取频率与“每分钟最多读几次”（默认 120）、按内容认编辑、正文表情写法
//!   `{名字}` 与白底表情图、每条消息自己说能不能编辑（`editDraft`）、一对话题副本、按规则名重译、续期、账号的工作区。
//! - 平台是 `history=server`：宿主按读取频率来读（`messages/list`），每次读都在标准错误里写一行“距上次 N 秒”，看读取间隔用。
//!
//! 工作目录（宿主给的临时目录）里出现这些文件时照做：`expire` 说登录失效；`notice` 报一条要重新登录的问题、`notice-clear`
//! 清掉；`new-message` 在没关注的“示例公告”里来一条新消息（会话发现用）；`edit` 改掉第一条的文字（按内容认编辑用）——以上
//! 做完删掉文件。`deny-reactions` 在时读表情一律回没有权限；`rule-v2` 在时多声明一条重译规则 `sample-v2`（下次打开生效）。
//!
//! 凭据是一段 JSON：签发时间与账号。交接时沿用内置平台核验过的账号（同编号共用一份本机数据，按账号分库），之后每次打开、
//! 续期都从凭据里取回这个账号。
//!
//! 参数：`--id <编号>` 换插件编号（同一台电脑装两份时用）；`--reserved feishu` 用保留编号 `feishu`（交接验收用：
//! `session/open` 收到 `legacy` 时先存自己的凭据，再回在线与 `adoptedSettings`）；`--plain` 什么都不开放（参数配置里没有它）；
//! `--callback-path <路径>` 换掉“使用自己的应用”声明的回调路径（演示 `hhh-im-check` 报出不在本 IM 目录下的路径）。
//! 第一条消息上有一个目录外的表情 `wave`：声明了猜图，宿主来要图时给，别的目录外类型回没有。表情图都是 32×32 的白底
//! 色块圆（声明了 `whiteBackground`，宿主显示前抠掉白底）。
//!
//! 运行：`cargo run -p hhh-im-protocol --example sample_provider --features transport`（通常由 HHH Friend 作为子进程启动）。
//! 不联网、全在内存。

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use hhh_im_protocol::rpc::{RpcError, RpcId, RpcMessage, codec};
use serde_json::{Value, json};

/// “赞”的表情图：白底上蓝色圆（32×32，白底，宿主按 `whiteBackground` 抠掉）。
const THUMBSUP_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x20, 0x08, 0x02, 0x00, 0x00, 0x00, 0xFC, 0x18, 0xED,
    0xA3, 0x00, 0x00, 0x00, 0x66, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0xED, 0xD6, 0xB9, 0x0D, 0x00,
    0x20, 0x0C, 0x03, 0x40, 0xEF, 0xC4, 0x62, 0xEC, 0xDF, 0x84, 0x9E, 0x4F, 0x41, 0x18, 0x4B, 0x40,
    0x10, 0xB5, 0xAF, 0x09, 0xC4, 0xB0, 0xC3, 0x07, 0x7F, 0x00, 0x29, 0xF7, 0x2F, 0x01, 0x18, 0x45,
    0xFB, 0x19, 0xEC, 0x44, 0x7B, 0x18, 0x50, 0xD2, 0x27, 0x06, 0x58, 0xE9, 0x23, 0x03, 0xC4, 0xF4,
    0xAE, 0xA1, 0x05, 0xF6, 0xD3, 0x5B, 0x43, 0x08, 0xB0, 0xD2, 0x2B, 0xC3, 0x0B, 0xAC, 0x0E, 0x5B,
    0x00, 0x01, 0x1C, 0x19, 0xD3, 0xFB, 0x5F, 0xF2, 0x13, 0xBF, 0xA9, 0x62, 0xE1, 0x28, 0x56, 0xA6,
    0x62, 0xE9, 0x2B, 0x6A, 0x8B, 0xA8, 0x78, 0x45, 0xF9, 0x35, 0x2B, 0x64, 0x65, 0x49, 0x60, 0xC8,
    0x25, 0xFE, 0x0F, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// “微笑”的表情图：白底上黄色圆。
const SMILE_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x20, 0x08, 0x02, 0x00, 0x00, 0x00, 0xFC, 0x18, 0xED,
    0xA3, 0x00, 0x00, 0x00, 0x6B, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0xED, 0xD6, 0xC1, 0x0D, 0xC0,
    0x20, 0x0C, 0x03, 0x40, 0x0F, 0xD6, 0x49, 0x3A, 0x27, 0xAB, 0xF5, 0x9D, 0xFE, 0x81, 0x54, 0x91,
    0xE2, 0xBA, 0x82, 0x06, 0xE5, 0xED, 0xFB, 0x00, 0x31, 0xEC, 0xE5, 0x83, 0x7F, 0x00, 0x57, 0xC3,
    0x74, 0x08, 0x80, 0x17, 0x1D, 0x67, 0x90, 0x89, 0x8E, 0x30, 0xA0, 0xA4, 0x3F, 0x18, 0x60, 0xA5,
    0x7B, 0x06, 0x88, 0xE9, 0x53, 0x43, 0x0B, 0xE4, 0xD3, 0x47, 0x43, 0x08, 0xB0, 0xD2, 0x3B, 0x23,
    0x0A, 0x9C, 0x47, 0x3F, 0x05, 0x14, 0xF0, 0xC1, 0x35, 0x5D, 0xFF, 0x25, 0x6F, 0xF1, 0x9B, 0x2A,
    0x16, 0x8E, 0x62, 0x65, 0x2A, 0x96, 0xBE, 0xA2, 0xB6, 0x88, 0x8A, 0x57, 0x95, 0x5F, 0xB3, 0x1B,
    0x08, 0x2A, 0x00, 0xAC, 0xD3, 0x60, 0x42, 0x37, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
    0xAE, 0x42, 0x60, 0x82,
];

/// 目录外的 `wave`：白底上橙色圆（猜图给的）。
const WAVE_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x20, 0x08, 0x02, 0x00, 0x00, 0x00, 0xFC, 0x18, 0xED,
    0xA3, 0x00, 0x00, 0x00, 0x63, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0xED, 0xD6, 0xC1, 0x0A, 0x00,
    0x20, 0x08, 0x03, 0xD0, 0xFD, 0x78, 0xDF, 0x6E, 0xE7, 0x48, 0xC1, 0x68, 0x0D, 0x2A, 0xA3, 0xF3,
    0xDE, 0xC5, 0x72, 0xB0, 0xC3, 0x07, 0x9F, 0x00, 0x0D, 0xFE, 0x25, 0x00, 0x51, 0x74, 0x9A, 0xC1,
    0x56, 0x74, 0x82, 0x01, 0x27, 0x3D, 0x36, 0x40, 0x4B, 0x0F, 0x0C, 0x30, 0xD3, 0x3D, 0x43, 0x0C,
    0xEC, 0xA7, 0x4F, 0x86, 0x12, 0x60, 0xA5, 0x8F, 0x46, 0x1A, 0x58, 0x1D, 0xB6, 0x02, 0x0A, 0x38,
    0x32, 0xA6, 0xF7, 0xBF, 0xE4, 0x27, 0x7E, 0x53, 0xC5, 0xC2, 0x51, 0xAC, 0x4C, 0xC5, 0xD2, 0x57,
    0xD4, 0x16, 0x51, 0xF1, 0xAA, 0xF2, 0x6B, 0x1D, 0x44, 0xAF, 0xFE, 0x81, 0xF2, 0x7D, 0x87, 0x2C,
    0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// 自己的应用方式里，假授权页交回的授权码。
const CODE: &str = "sample-code";
/// 续期间隔。
const RENEW_AFTER_MS: i64 = 60_000;

type Out = Arc<Mutex<std::io::Stdout>>;
/// 插件发给宿主、正在等回复的请求：编号 → 收回复的一头（续期要等凭据存好再回宿主）。
type Waiting = Arc<Mutex<BTreeMap<i64, std::sync::mpsc::Sender<RpcMessage>>>>;

/// 一次正在进行的登录：宿主给的 `state` → 流程编号与回调地址（自己的应用方式才有）。
struct Flow {
    flow_id: String,
    callback: Option<String>,
}

struct State {
    /// 插件编号（`--reserved` 时就是那个保留编号）。
    id: String,
    plain: bool,
    /// `--callback-path` 给的回调路径；不给就写相对路径 `callback`（v14：宿主拼在这个 IM 自己的目录下，插件不用知道平台编号）。
    callback_path: Option<String>,
    logged_in: bool,
    /// 当前账号（`session/open`、续期时从凭据里取回；交接时换成内置平台核验过的账号）。
    account: Value,
    flows: BTreeMap<String, Flow>,
    next_flow: u64,
    messages: Vec<Value>,
    /// 消息上的表情回应（第一条上有一个目录里的“赞”、一个目录外的 `wave`；宿主点的加在后面）。
    reactions: Vec<Value>,
    /// 每个会话上次被读的时间（标准错误里写读取间隔用）。
    last_read: BTreeMap<String, i64>,
    /// 刚开始的登录要推的“打开登录页”一步（回复 `login/start` 之后推）。
    pending_step: Option<(String, String)>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let value_of = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let id = value_of("--reserved")
        .or_else(|| value_of("--id"))
        .unwrap_or_else(|| "sample".into());
    let state = Arc::new(Mutex::new(State {
        id,
        plain: args.iter().any(|a| a == "--plain"),
        callback_path: value_of("--callback-path"),
        logged_in: false,
        account: default_account(),
        flows: BTreeMap::new(),
        next_flow: 1,
        messages: demo_messages(),
        reactions: demo_reactions(),
        last_read: BTreeMap::new(),
        pending_step: None,
    }));
    let out: Out = Arc::new(Mutex::new(std::io::stdout()));
    let next_id = Arc::new(AtomicI64::new(1_000_000));
    let waiting: Waiting = Arc::new(Mutex::new(BTreeMap::new()));
    let port = serve_auth_pages(state.clone(), out.clone(), next_id.clone(), waiting.clone());
    watch_switches(state.clone(), out.clone());

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(msg) = codec::decode_line(line) else {
            eprintln!("sample_provider: 收到非法 JSON-RPC，跳过");
            continue;
        };
        if msg.is_response() {
            // 有人在等这个回复（续期等凭据存好）就交给它，别的回复不看。
            if let Some(RpcId::Number(n)) = &msg.id
                && let Some(tx) = waiting.lock().unwrap().remove(n)
            {
                let _ = tx.send(msg);
            }
            continue;
        }
        if msg.is_notification() {
            continue;
        }
        let (Some(method), Some(id)) = (msg.method.clone(), msg.id.clone()) else {
            continue;
        };
        let params = msg.params.clone().unwrap_or(Value::Null);
        if method == "session/renew" {
            // 续期：经 credential/save 存新的、宿主回了“存好了”再回续期成功（协议 §6.3）；在另一条线程里等，
            // 这条线程接着读标准输入（宿主的回复也从这里来）。
            eprintln!("sample_provider: 宿主叫续期");
            adopt_account(&state, params["credential"].as_str());
            let (state, out, next_id, waiting) =
                (state.clone(), out.clone(), next_id.clone(), waiting.clone());
            std::thread::spawn(move || {
                let reply = match save_credential_and_wait(&state, &out, &next_id, &waiting) {
                    Ok(()) => RpcMessage::ok(id, json!({})),
                    Err(message) => RpcMessage::err(id, error("unavailable", &message)),
                };
                send(&out, reply);
            });
            continue;
        }
        if method == "session/open" && params.get("legacy").is_some_and(Value::is_object) {
            // 交接：先经 credential/save 存好自己的凭据、宿主回了“存好了”再回在线（协议附录 D，同 §6.3 先存后用；
            // 没等回复就回在线时，宿主可能还没存下，内置那份就留到下次再交）。同续期在另一条线程里等。
            let (state, out, next_id, waiting) =
                (state.clone(), out.clone(), next_id.clone(), waiting.clone());
            std::thread::spawn(move || {
                let reply = match adopt_legacy(&params, &state, &out, &next_id, &waiting) {
                    Ok(value) => RpcMessage::ok(id, value),
                    Err(message) => RpcMessage::err(id, error("unavailable", &message)),
                };
                send(&out, reply);
            });
            continue;
        }
        let reply = handle(&method, &params, &state, &out, &next_id, &waiting, port);
        send(
            &out,
            match reply {
                Ok(value) => RpcMessage::ok(id, value),
                Err(error) => RpcMessage::err(id, error),
            },
        );
        let pending = state.lock().unwrap().pending_step.take();
        if let Some((flow_id, url)) = pending {
            notify(
                &out,
                "login/step",
                json!({ "flowId": flow_id, "step": { "kind": "openUrl", "url": url, "label": "再打开一次",
                    "note": { "text": "演示插件的假授权页：点“同意”就算登录成功。", "tone": "info" } } }),
            );
        }
    }
}

/// 处理一个请求；错误按协议的错误对象回。
fn handle(
    method: &str,
    params: &Value,
    state: &Arc<Mutex<State>>,
    out: &Out,
    next_id: &Arc<AtomicI64>,
    waiting: &Waiting,
    port: u16,
) -> Result<Value, RpcError> {
    match method {
        "initialize" => Ok(initialize(&state.lock().unwrap())),
        "session/open" => Ok(open(params, state, out, next_id)),
        "session/logout" => {
            state.lock().unwrap().logged_in = false;
            Ok(json!({}))
        }
        "ping" | "shutdown" => Ok(json!({})),
        "login/start" => start_login(params, state, port),
        "login/callback" => finish_callback(params, state, out, next_id, waiting),
        "conversations/get" => conversation(params["conversationId"].as_str().unwrap_or(""))
            .ok_or_else(|| error("notFound", "没有这个会话。")),
        "conversations/recentActivity" => Ok(recent_activity(params, state)),
        "messages/list" => Ok(list_messages(params, state)),
        "messages/get" => {
            let id = params["messageId"].as_str().unwrap_or("");
            let state = state.lock().unwrap();
            state
                .messages
                .iter()
                .find(|m| m["id"] == id)
                .map(translated)
                .ok_or_else(|| error("notFound", "没有这条消息。"))
        }
        "people/get" => {
            let found: Vec<Value> = params["ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|id| json!({ "id": id, "kind": "user", "name": person(id) }))
                .collect();
            Ok(json!({ "found": found, "missing": [] }))
        }
        "reactions/list" => {
            if std::path::Path::new("deny-reactions").exists() {
                return Err(error(
                    "permissionDenied",
                    "示例 IM 没给读表情的权限：在示例开放平台打开“读表情”后重新登录。",
                ));
            }
            let message = params["messageId"].as_str().unwrap_or("");
            let state = state.lock().unwrap();
            let items: Vec<Value> = state
                .reactions
                .iter()
                .filter(|r| r["messageId"] == message)
                .cloned()
                .collect();
            Ok(json!({ "items": items }))
        }
        "reactions/add" => {
            let mut state = state.lock().unwrap();
            let n = state.reactions.len() + 1;
            let reaction = json!({
                "id": format!("r-{n}"), "conversationId": params["conversationId"], "messageId": params["messageId"],
                "operator": { "id": "me", "kind": "user", "name": "示例用户" },
                "emojiType": params["emojiType"], "atMs": now_ms()
            });
            state.reactions.push(reaction.clone());
            Ok(reaction)
        }
        "reactions/remove" => {
            let id = params["reactionId"].as_str().unwrap_or("").to_owned();
            state
                .lock()
                .unwrap()
                .reactions
                .retain(|r| r["id"] != id.as_str());
            Ok(json!({}))
        }
        "emoji/catalog" => Ok(json!({
            "emojis": [ { "emojiType": "thumbsup", "name": "赞" }, { "emojiType": "smile", "name": "微笑" } ],
            "categories": [], "updatedAtMs": 1
        })),
        "emoji/image" => match params["emojiType"].as_str() {
            Some("thumbsup") => Ok(chunk(params, THUMBSUP_PNG)),
            Some("smile") => Ok(chunk(params, SMILE_PNG)),
            Some("wave") => Ok(chunk(params, WAVE_PNG)),
            _ => Err(error("notFound", "示例 IM 没有这个表情的图。")),
        },
        "resources/read" => Ok(chunk(params, SMILE_PNG)),
        "messages/send" => Ok(send_message(params, state)),
        "messages/edit" => edit_message(params, state),
        "messages/retranslate" => Ok(retranslate(params, state)),
        _ => Err(error("unsupported", "示例 IM 不支持这件事。")),
    }
}

fn initialize(state: &State) -> Value {
    let mut rules = vec![json!({ "name": "sample-v1" })];
    if std::path::Path::new("rule-v2").exists() {
        rules.push(json!({ "name": "sample-v2", "formats": ["sample-msg-v1"] }));
    }
    let mut platform = json!({
        "avatarShape": "rounded",
        "history": "server",
        "actions": { "reply": true, "thread": true, "react": true, "recall": null, "edit": { "own": true, "perMessage": true } },
        "pageMax": 50,
        "reads": { "perSecond": 5, "perMinute": 120 },
        "emoji": { "unlisted": { "placeholderLabel": "表情", "guess": true }, "textPattern": "\\{([^{}\\r\\n]{1,12})\\}", "whiteBackground": true },
        "discovery": { "fastestSecs": 15, "pages": 2, "coldPages": 4 },
        "editsByContent": ["text"],
        "retranslateRules": rules,
    });
    if !state.plain {
        platform["userAdjustable"] = json!(["readIntervals", "readsPerMinute"]);
    }
    let settings = if state.plain {
        json!([])
    } else {
        json!([{ "id": "signature", "label": "消息签名", "type": "text", "default": "",
                 "help": "发消息时附在末尾；留空不加。" }])
    };
    let callback = state
        .callback_path
        .clone()
        .unwrap_or_else(|| "callback".to_owned());
    json!({
        "protocolVersion": 1,
        "provider": { "id": state.id, "name": "示例 IM", "version": "0.1.0" },
        "platform": platform,
        "capabilities": {
            "conversations": { "list": false, "search": false, "members": false, "findDirect": false, "recentActivity": true },
            "messages": { "list": true, "get": true },
            "people": { "get": true, "search": false },
            "send": { "text": true, "mentions": false, "mentionAll": false, "image": false, "file": false, "lookup": false },
            "recall": false,
            "edit": true,
            "reactions": { "list": true, "add": true, "remove": true, "catalog": true },
            "resources": { "read": true, "avatars": false },
            "events": { "messages": true, "deletions": false, "reactions": false, "conversations": true, "people": true },
            "retranslate": true,
            "session": { "renew": true }
        },
        "settings": settings,
        "loginMethods": [
            { "id": "relay", "kind": "relay", "label": "通过中继登录", "note": "经公网授权中继登录示例 IM，不用自己创建应用。",
              "placeholder": "https://relay.example.com" },
            { "id": "own", "kind": "ownApp", "label": "使用自己的应用", "note": "用你在示例开放平台创建的应用登录。",
              "idPlaceholder": "cli_ 开头", "secretPlaceholder": "应用的 App Secret",
              "secretHelp": "示例开放平台 → 应用的凭证页。只保存在本机。", "callbackPath": callback,
              "callbackHelp": "在示例开放平台的应用后台，把这个地址登记为重定向地址。",
              "createLink": { "label": "前往示例开放平台创建", "url": "https://open.example.com/app",
                              "after": "，创建后把 App ID 与 App Secret 填在下面。" } }
        ]
    })
}

/// 交接（`session/open` 带 `legacy`）：沿用内置授权里上次核验的账号，先存自己的凭据并等宿主回复，再回在线与要写进
/// 设置的东西；宿主没存上时回错误（内置那份留着，下次再交）。
fn adopt_legacy(
    params: &Value,
    state: &Arc<Mutex<State>>,
    out: &Out,
    next_id: &Arc<AtomicI64>,
    waiting: &Waiting,
) -> Result<Value, String> {
    let legacy = &params["legacy"];
    eprintln!(
        "sample_provider: 收到内置平台的交接（格式 {}）",
        legacy["format"].as_str().unwrap_or("?")
    );
    // 内置授权里有上次核验的账号：沿用它（同编号按账号分库，账号对上本机记录与关注才在）。
    let builtin: Value = legacy["credential"]
        .as_str()
        .and_then(|c| serde_json::from_str(c).ok())
        .unwrap_or(Value::Null);
    if let Some(id) = builtin["account"]["id"]
        .as_str()
        .filter(|id| !id.is_empty())
    {
        let name = builtin["account"]["name"].as_str().unwrap_or(id);
        state.lock().unwrap().account = json!({ "id": id, "name": name });
    }
    save_credential_and_wait(state, out, next_id, waiting)?;
    let account = state.lock().unwrap().account.clone();
    state.lock().unwrap().logged_in = true;
    push_conversations(out, next_id);
    let app_id = legacy["settings"]["appId"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    let mut adopted = json!({ "parameters": { "signature": "" } });
    if !app_id.is_empty() {
        adopted["login"] = json!({ "method": "own", "values": { "appId": app_id } });
    }
    Ok(json!({ "state": "online", "account": account, "adoptedSettings": adopted }))
}

/// `session/open`：有凭据在线；都没有要登录（带 `legacy` 的交接见 [`adopt_legacy`]）。
fn open(params: &Value, state: &Arc<Mutex<State>>, out: &Out, next_id: &Arc<AtomicI64>) -> Value {
    let credential = params
        .get("credential")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty());
    adopt_account(state, credential);
    if credential.is_some() || state.lock().unwrap().logged_in {
        state.lock().unwrap().logged_in = true;
        push_conversations(out, next_id);
        json!({ "state": "online", "account": state.lock().unwrap().account.clone() })
    } else {
        json!({ "state": "needsLogin", "reason": "noCredential", "message": "需要登录示例 IM：点“登录”。" })
    }
}

/// `login/start`：记下这次的 state 与回调地址，回流程编号；“打开登录页”一步在回复之后推。
fn start_login(params: &Value, state: &Arc<Mutex<State>>, port: u16) -> Result<Value, RpcError> {
    let method = params["method"].as_str().unwrap_or("relay");
    if method == "own" && params["values"]["appId"].as_str().unwrap_or("").trim() == "bad" {
        return Err(error(
            "invalidRequest",
            "示例开放平台说这个 App ID 不存在。",
        ));
    }
    let mut state = state.lock().unwrap();
    let flow_id = format!("flow-{}", state.next_flow);
    state.next_flow += 1;
    // 旧宿主不给 state：插件自己编一个。
    let key = params["state"]
        .as_str()
        .map_or_else(|| format!("sample-{flow_id}"), str::to_owned);
    let callback = (method == "own")
        .then(|| params["callbackUrl"].as_str().map(str::to_owned))
        .flatten();
    state.pending_step = Some((
        flow_id.clone(),
        format!("http://127.0.0.1:{port}/authorize?state={}", encode(&key)),
    ));
    state.flows.insert(
        key,
        Flow {
            flow_id: flow_id.clone(),
            callback,
        },
    );
    Ok(json!({ "flowId": flow_id }))
}

/// `login/callback`：宿主把回调地址交回来；`state` 与授权码对得上才算登录成功。
fn finish_callback(
    params: &Value,
    state: &Arc<Mutex<State>>,
    out: &Out,
    next_id: &Arc<AtomicI64>,
    waiting: &Waiting,
) -> Result<Value, RpcError> {
    let url = params["url"].as_str().unwrap_or("");
    let query = query_of(url);
    let given = query.get("state").cloned().unwrap_or_default();
    let flow = {
        let mut state = state.lock().unwrap();
        match state.flows.get(&given) {
            Some(flow) if Some(flow.flow_id.as_str()) == params["flowId"].as_str() => {
                state.flows.remove(&given)
            }
            _ => None,
        }
    };
    let Some(flow) = flow else {
        return Err(error("invalidRequest", "回调对不上这次登录。"));
    };
    if query.get("code").map(String::as_str) != Some(CODE) {
        notify(
            out,
            "login/failed",
            json!({ "flowId": flow.flow_id, "message": "示例开放平台没给授权码。", "retryable": true }),
        );
        return Ok(json!({}));
    }
    complete_login(&flow.flow_id, state, out, next_id, waiting);
    Ok(json!({}))
}

/// 登录成功：先存凭据（带续期时间）、等宿主回了“存好了”再说登录完成、推会话（协议 §6.3：`login/finished` 只能在
/// `credential/save` 成功之后发）。在另一条线程里等，调用方（读标准输入的那条、假授权页）照常往下走；宿主没存上时说登录失败。
fn complete_login(
    flow_id: &str,
    state: &Arc<Mutex<State>>,
    out: &Out,
    next_id: &Arc<AtomicI64>,
    waiting: &Waiting,
) {
    let (flow_id, state, out, next_id, waiting) = (
        flow_id.to_owned(),
        state.clone(),
        out.clone(),
        next_id.clone(),
        waiting.clone(),
    );
    std::thread::spawn(move || {
        if let Err(message) = save_credential_and_wait(&state, &out, &next_id, &waiting) {
            notify(
                &out,
                "login/failed",
                json!({ "flowId": flow_id, "message": message, "retryable": true }),
            );
            return;
        }
        let account = {
            let mut state = state.lock().unwrap();
            state.logged_in = true;
            state.account.clone()
        };
        notify(
            &out,
            "login/finished",
            json!({ "flowId": flow_id, "account": account }),
        );
        push_conversations(&out, &next_id);
    });
}

/// 一份新凭据（签发时间与账号）的 `credential/save` 参数，60 秒后续期。
fn credential_params(state: &Arc<Mutex<State>>) -> Value {
    let issued = now_ms();
    let credential =
        json!({ "issuedAtMs": issued, "account": state.lock().unwrap().account.clone() });
    eprintln!("sample_provider: 存新凭据（签发于 {issued}）");
    json!({ "credential": credential.to_string(), "renewAtMs": issued + RENEW_AFTER_MS })
}

/// 存一份新凭据并等宿主回复（登录、交接、续期都是存好了才往下走，协议 §6.3 先存后用）。宿主 10 秒不回、回了错误都算没存上。
fn save_credential_and_wait(
    state: &Arc<Mutex<State>>,
    out: &Out,
    next_id: &Arc<AtomicI64>,
    waiting: &Waiting,
) -> Result<(), String> {
    let id = next_id.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = std::sync::mpsc::channel();
    waiting.lock().unwrap().insert(id, tx);
    send(
        out,
        RpcMessage::request(
            RpcId::Number(id),
            "credential/save",
            credential_params(state),
        ),
    );
    let reply = rx.recv_timeout(std::time::Duration::from_secs(10));
    waiting.lock().unwrap().remove(&id);
    match reply {
        Ok(reply) if reply.error.is_none() => Ok(()),
        Ok(reply) => Err(format!(
            "宿主没存上新凭据：{}",
            reply.error.map(|e| e.message).unwrap_or_default()
        )),
        Err(_) => Err("宿主 10 秒没回存凭据".into()),
    }
}

fn default_account() -> Value {
    json!({ "id": "me", "name": "示例用户", "tenant": "示例团队" })
}

/// 从凭据里取回账号；凭据不是这个插件写的 JSON（旧版写的、空的）就用默认账号。
fn adopt_account(state: &Arc<Mutex<State>>, credential: Option<&str>) {
    let account = credential
        .and_then(|c| serde_json::from_str::<Value>(c).ok())
        .map(|c| c["account"].clone())
        .filter(|a| a["id"].as_str().is_some_and(|id| !id.is_empty()))
        .unwrap_or_else(default_account);
    state.lock().unwrap().account = account;
}

/// 按现在的规则翻译一条消息：`rule-v2` 在时文字前加“【新版翻译】”（与 `messages/retranslate` 按 `sample-v2` 重译的结果一样）。
fn translated(message: &Value) -> Value {
    let mut m = message.clone();
    if std::path::Path::new("rule-v2").exists()
        && let Some(data) = m["raw"]["data"].as_str()
    {
        m["content"] = json!([{ "type": "text", "text": format!("【新版翻译】{data}") }]);
    }
    m
}

/// 会话与人推给宿主（宿主在本机替插件搜群、搜人）。
fn push_conversations(out: &Out, next_id: &Arc<AtomicI64>) {
    let events: Vec<Value> = ["c-team", "c-direct"]
        .iter()
        .filter_map(|id| conversation(id))
        .map(|c| json!({ "type": "conversation", "conversation": c }))
        .collect();
    request(
        out,
        next_id,
        "events/push",
        json!({ "events": events, "syncState": "sample-1" }),
    );
}

fn conversation(id: &str) -> Option<Value> {
    Some(match id {
        "c-team" => json!({ "id": "c-team", "kind": "group", "name": "示例团队群" }),
        "c-news" => json!({ "id": "c-news", "kind": "group", "name": "示例公告" }),
        "c-direct" => {
            json!({ "id": "c-direct", "kind": "direct", "name": "阿美", "peer": "u-amy", "private": true })
        }
        _ => return None,
    })
}

fn person(id: &str) -> &'static str {
    match id {
        "me" => "示例用户",
        "u-amy" => "阿美",
        "u-bo" => "阿博",
        _ => "示例成员",
    }
}

/// 一条消息：文字、原始数据；本人发的、能编辑的带编辑框原文。
fn message(
    id: &str,
    conversation: &str,
    sender: &str,
    at_ms: i64,
    text: &str,
    editable: bool,
) -> Value {
    let mut m = json!({
        "id": id, "conversationId": conversation,
        "sender": { "id": sender, "kind": "user", "name": person(sender) },
        "createdAtMs": at_ms,
        "content": [ { "type": "text", "text": text } ],
        "raw": { "format": "sample-msg-v1", "data": text }
    });
    if editable {
        m["editDraft"] = json!([{ "type": "text", "text": text }]);
    }
    m
}

fn demo_messages() -> Vec<Value> {
    let base = now_ms() - 3_600_000;
    let mut thread_reply = message(
        "m-4",
        "c-team",
        "u-bo",
        base + 40_000,
        "话题里的回复，也发到了群里",
        false,
    );
    thread_reply["thread"] = json!({ "id": "t-1", "rootId": "m-1", "inThread": true,
        "copy": { "role": "thread", "peerId": "m-4c", "threadId": "t-1" } });
    let mut copy = message(
        "m-4c",
        "c-team",
        "u-bo",
        base + 40_000,
        "话题里的回复，也发到了群里",
        false,
    );
    copy["thread"] =
        json!({ "copy": { "role": "conversation", "peerId": "m-4", "threadId": "t-1" } });
    // 第一条是话题 t-1 的第一条（带话题编号与现状，宿主据此按话题读回复、给会话里那份副本找到话题）。
    let mut root = message(
        "m-1",
        "c-team",
        "u-amy",
        base,
        "欢迎来到示例 IM {赞}",
        false,
    );
    root["thread"] = json!({ "id": "t-1", "inThread": false,
        "activity": { "latestReplyId": "m-4", "latestReplyAtMs": base + 40_000, "replyCount": 1 } });
    vec![
        root,
        message("m-2", "c-team", "me", base + 10_000, "这条可以编辑", true),
        message(
            "m-3",
            "c-team",
            "me",
            base + 20_000,
            "这条平台说不能编辑",
            false,
        ),
        thread_reply,
        copy,
        message(
            "m-5",
            "c-direct",
            "u-amy",
            base + 50_000,
            "私聊里的一条",
            false,
        ),
        message(
            "m-6",
            "c-news",
            "u-bo",
            base - 86_400_000,
            "很早以前的公告",
            false,
        ),
    ]
}

/// 第一条消息上的表情回应：目录里的“赞”与目录外的 `wave`（宿主按 `guess` 来要图）。
fn demo_reactions() -> Vec<Value> {
    vec![
        json!({ "id": "r-1", "conversationId": "c-team", "messageId": "m-1",
            "operator": { "id": "u-amy", "kind": "user", "name": "阿美" }, "emojiType": "thumbsup", "atMs": 1_700_000_100_000i64 }),
        json!({ "id": "r-2", "conversationId": "c-team", "messageId": "m-1",
            "operator": { "id": "u-bo", "kind": "user", "name": "阿博" }, "emojiType": "wave", "atMs": 1_700_000_200_000i64 }),
    ]
}

/// 会话发现：这段时间里别人发过消息的会话。
fn recent_activity(params: &Value, state: &Arc<Mutex<State>>) -> Value {
    let since = params["sinceMs"].as_i64().unwrap_or(0);
    let state = state.lock().unwrap();
    let mut items: BTreeMap<String, Value> = BTreeMap::new();
    for m in &state.messages {
        let at = m["createdAtMs"].as_i64().unwrap_or(0);
        let conversation = m["conversationId"].as_str().unwrap_or("");
        if at < since || m["sender"]["id"] == "me" || m["thread"]["inThread"] == true {
            continue;
        }
        let kind = if conversation == "c-direct" {
            "direct"
        } else {
            "group"
        };
        let entry = items.entry(conversation.to_owned()).or_insert_with(|| {
            json!({ "conversationId": conversation, "kind": kind, "firstMs": at, "latestMs": at })
        });
        entry["firstMs"] = json!(entry["firstMs"].as_i64().unwrap_or(at).min(at));
        entry["latestMs"] = json!(entry["latestMs"].as_i64().unwrap_or(at).max(at));
        if kind == "direct" {
            entry["peerId"] = m["sender"]["id"].clone();
        }
    }
    let mut items: Vec<Value> = items.into_values().collect();
    items.sort_by_key(|i| std::cmp::Reverse(i["latestMs"].as_i64().unwrap_or(0)));
    json!({ "items": items, "complete": true })
}

/// 读一页消息（会话里的或一个话题里的），并在标准错误里写距上次读这个会话多久。
fn list_messages(params: &Value, state: &Arc<Mutex<State>>) -> Value {
    let conversation = params["conversationId"].as_str().unwrap_or("");
    let thread = params["threadId"].as_str();
    let after = params["afterMs"].as_i64();
    let before = params["beforeMs"].as_i64();
    let limit = params["limit"].as_u64().unwrap_or(50) as usize;
    let mut state = state.lock().unwrap();
    let now = now_ms();
    let key = format!(
        "{conversation}{}",
        thread.map(|t| format!("/{t}")).unwrap_or_default()
    );
    match state.last_read.insert(key.clone(), now) {
        Some(last) => eprintln!(
            "sample_provider: messages/list {key}（距上次 {:.1} 秒）",
            (now - last) as f64 / 1000.0
        ),
        None => eprintln!("sample_provider: messages/list {key}（第一次）"),
    }
    let mut items: Vec<Value> = state
        .messages
        .iter()
        .filter(|m| m["conversationId"] == conversation)
        .filter(|m| match thread {
            Some(t) => m["thread"]["id"] == t || m["id"] == "m-1",
            None => m["thread"]["inThread"] != true,
        })
        .filter(|m| {
            let at = m["createdAtMs"].as_i64().unwrap_or(0);
            after.is_none_or(|a| at > a) && before.is_none_or(|b| at < b)
        })
        .map(translated)
        .collect();
    items.sort_by_key(|m| m["createdAtMs"].as_i64().unwrap_or(0));
    if params["newestFirst"].as_bool().unwrap_or(false) {
        items.reverse();
    }
    items.truncate(limit.max(1));
    json!({ "items": items })
}

fn send_message(params: &Value, state: &Arc<Mutex<State>>) -> Value {
    let op = params["operationId"].as_str().unwrap_or("op");
    let conversation = params["conversationId"].as_str().unwrap_or("c-team");
    let text: String = params["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| block["text"].as_str())
        .collect();
    let sent = message(
        &format!("sent-{op}"),
        conversation,
        "me",
        now_ms(),
        &text,
        true,
    );
    state.lock().unwrap().messages.push(sent.clone());
    json!({ "state": "accepted", "message": sent })
}

fn edit_message(params: &Value, state: &Arc<Mutex<State>>) -> Result<Value, RpcError> {
    let id = params["messageId"].as_str().unwrap_or("");
    let text: String = params["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| block["text"].as_str())
        .collect();
    let mut state = state.lock().unwrap();
    let Some(m) = state.messages.iter_mut().find(|m| m["id"] == id) else {
        return Err(error("notFound", "没有这条消息。"));
    };
    if m.get("editDraft").is_none() {
        return Err(error("permissionDenied", "示例 IM 说这条不能编辑。"));
    }
    m["content"] = json!([{ "type": "text", "text": text }]);
    m["raw"] = json!({ "format": "sample-msg-v1", "data": text });
    m["editDraft"] = json!([{ "type": "text", "text": text }]);
    m["editedAtMs"] = json!(now_ms());
    Ok(m.clone())
}

/// 按规则名重译：`sample-v2` 在文字前加“【新版翻译】”，`sample-v1` 照原文。
fn retranslate(params: &Value, state: &Arc<Mutex<State>>) -> Value {
    let rule = params["rule"].as_str().unwrap_or("");
    eprintln!(
        "sample_provider: 按 {rule} 重译 {} 条",
        params["messages"].as_array().map_or(0, Vec::len)
    );
    let state = state.lock().unwrap();
    let messages: Vec<Value> = params["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let mut m = state
                .messages
                .iter()
                .find(|m| m["id"] == item["id"])?
                .clone();
            let data = item["raw"]["data"].as_str().unwrap_or("");
            let text = if rule == "sample-v2" {
                format!("【新版翻译】{data}")
            } else {
                data.to_owned()
            };
            m["content"] = json!([{ "type": "text", "text": text }]);
            Some(m)
        })
        .collect();
    json!({ "messages": messages })
}

fn chunk(params: &Value, png: &[u8]) -> Value {
    let offset = params["offset"].as_u64().unwrap_or(0) as usize;
    let length = params["length"].as_u64().unwrap_or(png.len() as u64) as usize;
    let end = (offset + length).min(png.len());
    let slice = if offset < png.len() {
        &png[offset..end]
    } else {
        &[]
    };
    json!({ "data": base64_encode(slice), "total": png.len(), "eof": end >= png.len() })
}

/// 每 300 毫秒看一次工作目录里的开关文件（见文件头）。
fn watch_switches(state: Arc<Mutex<State>>, out: Out) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(300));
            if std::fs::remove_file("expire").is_ok() {
                eprintln!("sample_provider: 开关 expire，说登录失效");
                state.lock().unwrap().logged_in = false;
                notify(
                    &out,
                    "session/expired",
                    json!({ "reason": "expired", "message": "示例 IM 的登录过期了，请重新登录。" }),
                );
            }
            if std::fs::remove_file("notice").is_ok() {
                notify(
                    &out,
                    "session/notice",
                    json!({ "notices": [{ "id": "scope", "text": "示例 IM 缺少读消息记录的权限：重新登录补上。", "relogin": true }] }),
                );
            }
            if std::fs::remove_file("notice-clear").is_ok() {
                notify(&out, "session/notice", json!({ "notices": [] }));
            }
            if std::fs::remove_file("new-message").is_ok() {
                let mut state = state.lock().unwrap();
                let n = state.messages.len();
                let m = message(
                    &format!("m-new-{n}"),
                    "c-news",
                    "u-bo",
                    now_ms(),
                    "示例公告：今天下午三点开会",
                    false,
                );
                state.messages.push(m);
                eprintln!("sample_provider: 开关 new-message，示例公告来了一条新消息");
            }
            if std::fs::remove_file("edit").is_ok() {
                let mut state = state.lock().unwrap();
                if let Some(m) = state.messages.iter_mut().find(|m| m["id"] == "m-1") {
                    let text = "欢迎来到示例 IM（已改过）{赞}";
                    m["content"] = json!([{ "type": "text", "text": text }]);
                    m["raw"] = json!({ "format": "sample-msg-v1", "data": text });
                    m["updatedAtMs"] = json!(now_ms());
                }
                eprintln!("sample_provider: 开关 edit，改了第一条");
            }
        }
    });
}

/// 本机回环的假授权页：`/authorize` 给“同意 / 拒绝”，`/approve` 完成（自己的应用方式跳回宿主的回调地址），`/deny` 失败。
fn serve_auth_pages(
    state: Arc<Mutex<State>>,
    out: Out,
    next_id: Arc<AtomicI64>,
    waiting: Waiting,
) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("假授权页监听本机端口");
    let port = listener.local_addr().map_or(0, |a| a.port());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (state, out, next_id, waiting) =
                (state.clone(), out.clone(), next_id.clone(), waiting.clone());
            std::thread::spawn(move || serve(stream, &state, &out, &next_id, &waiting));
        }
    });
    port
}

fn serve(
    mut stream: TcpStream,
    state: &Arc<Mutex<State>>,
    out: &Out,
    next_id: &Arc<AtomicI64>,
    waiting: &Waiting,
) {
    let mut first = String::new();
    let mut reader = BufReader::new(stream.try_clone().expect("复制连接"));
    if reader.read_line(&mut first).is_err() {
        return;
    }
    // 读完请求头（假授权页只收 GET）。
    let mut line = String::new();
    while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
        line.clear();
    }
    let target = first.split_whitespace().nth(1).unwrap_or("/");
    let path = target.split('?').next().unwrap_or("/");
    let query = query_of(target);
    let key = query.get("state").cloned().unwrap_or_default();
    let flow = {
        let state = state.lock().unwrap();
        state
            .flows
            .get(&key)
            .map(|f| (f.flow_id.clone(), f.callback.clone()))
    };
    let (status, headers, body) = match (path, flow) {
        ("/authorize", Some(_)) => (
            "200 OK",
            String::new(),
            format!(
                "<!doctype html><meta charset=utf-8><title>示例 IM 授权</title><body style=\"font:16px system-ui;padding:40px\">\
                 <h1>示例 IM 授权</h1><p>HHH Friend 想读取你在示例 IM 里的消息。</p>\
                 <p><a href=\"/approve?state={0}\" style=\"padding:8px 16px;background:#3370ff;color:#fff;border-radius:6px;text-decoration:none\">同意</a>\
                 &nbsp;<a href=\"/deny?state={0}\">拒绝</a></p></body>",
                encode(&key)
            ),
        ),
        ("/approve", Some((_, Some(callback)))) => {
            let sep = if callback.contains('?') { '&' } else { '?' };
            (
                "302 Found",
                format!(
                    "Location: {callback}{sep}code={CODE}&state={}\r\n",
                    encode(&key)
                ),
                String::new(),
            )
        }
        ("/approve", Some((flow_id, None))) => {
            state.lock().unwrap().flows.remove(&key);
            complete_login(&flow_id, state, out, next_id, waiting);
            (
                "200 OK",
                String::new(),
                "<!doctype html><meta charset=utf-8><body style=\"font:16px system-ui;padding:40px\">示例 IM 授权已完成，可以关掉这个页面。</body>".to_owned(),
            )
        }
        ("/deny", Some((flow_id, _))) => {
            state.lock().unwrap().flows.remove(&key);
            notify(
                out,
                "login/failed",
                json!({ "flowId": flow_id, "message": "你在示例 IM 里拒绝了授权。", "retryable": true }),
            );
            (
                "200 OK",
                String::new(),
                "<!doctype html><meta charset=utf-8><body style=\"font:16px system-ui;padding:40px\">已拒绝。</body>".to_owned(),
            )
        }
        _ => (
            "404 Not Found",
            String::new(),
            "<!doctype html><meta charset=utf-8>这次登录已经结束或不存在。".to_owned(),
        ),
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
}

/// 地址查询串里的各项（百分号解码）。
fn query_of(url: &str) -> BTreeMap<String, String> {
    url.split_once('?')
        .map(|(_, q)| q)
        .unwrap_or("")
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (decode(k), decode(v)))
        .collect()
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 协议的错误对象。
fn error(kind: &str, message: &str) -> RpcError {
    let code = match kind {
        "permissionDenied" => 1002,
        "notFound" => 1005,
        "invalidRequest" => 1006,
        _ => 1001,
    };
    serde_json::from_value(json!({ "code": code, "message": message, "data": { "kind": kind, "userMessage": message } }))
        .expect("错误对象")
}

fn send(out: &Out, msg: RpcMessage) {
    if let Ok(line) = codec::encode_line(&msg) {
        let mut guard = out.lock().unwrap();
        let _ = guard.write_all(line.as_bytes());
        let _ = guard.flush();
    }
}

fn notify(out: &Out, method: &str, params: Value) {
    send(out, RpcMessage::notification(method, params));
}

/// 发一个要宿主回的请求（回复不看）。
fn request(out: &Out, next_id: &Arc<AtomicI64>, method: &str, params: Value) {
    let id = next_id.fetch_add(1, Ordering::SeqCst);
    send(out, RpcMessage::request(RpcId::Number(id), method, params));
}

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
