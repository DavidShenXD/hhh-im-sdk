//! 声明式参数端到端示例：不联网，六种字段经真实子进程接到 Friend。
//!
//! 默认先登录；登录完成和重连时均核对 session/open 的参数。普通参数及密钥是否收到
//! 写到临时目录的 applied.json，密钥本身从不写文件或日志。--v2 加字段、--changed 改类型，
//! --no-settings 无声明、--unknown 声明未知控件，供安装/升级兼容性验收。

use hhh_im_protocol::rpc::{RpcId, RpcMessage, codec};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn declarations(args: &[String]) -> Value {
    if args.iter().any(|a| a == "--no-settings") {
        return json!([]);
    }
    let mut fields = vec![
        json!({"id":"endpoint","label":"服务地址","type":"text","required":true,"default":"https://im.example.com","pattern":"^https://","help":"以 https:// 开头。","group":{"id":"connection","label":"连接"}}),
        json!({"id":"token","label":"连接密钥","type":"secret","help":"留空保留已保存的密钥。","group":{"id":"connection","label":"连接"}}),
        json!({"id":"retry","label":"断线后自动重连","type":"switch","default":"true","group":{"id":"recovery","label":"连接恢复"}}),
        json!({"id":"delay","label":"重连等待时间","type":"number","default":"5","min":1,"max":60,"unit":"秒","showWhen":{"field":"retry","equals":"true"},"group":{"id":"recovery","label":"连接恢复"}}),
        json!({"id":"mode","label":"重连方式","type":"select","default":"backoff","options":[{"value":"backoff","label":"逐次延长"},{"value":"fixed","label":"固定等待"}],"showWhen":{"field":"retry","equals":"true"},"group":{"id":"recovery","label":"连接恢复"}}),
        json!({"id":"channels","label":"接收范围","type":"textarea","default":"","help":"每行一个频道名；留空接收全部频道。"}),
    ];
    if args.iter().any(|a| a == "--v2") {
        fields.push(json!({"id":"tag","label":"备注","type":"text","default":""}));
    }
    if args.iter().any(|a| a == "--changed") {
        fields[0] = json!({"id":"endpoint","label":"服务编号","type":"number","default":"1"});
    }
    if args.iter().any(|a| a == "--unknown") {
        fields[0]["type"] = json!("html");
    }
    json!(fields)
}

fn send(out: &mut impl Write, msg: RpcMessage) {
    writeln!(out, "{}", codec::encode_line(&msg).expect("协议编码")).expect("标准输出");
    out.flush().expect("标准输出刷新");
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let fields = declarations(&args);
    let version = if args.iter().any(|a| a == "--v2" || a == "--changed") {
        "0.2.0"
    } else {
        "0.1.0"
    };
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    let mut logged_in = false;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = codec::decode_line(&line) else {
            continue;
        };
        let Some(method) = msg.method.as_deref() else {
            continue;
        };
        let Some(id) = msg.id.clone() else { continue };
        let params = msg.params.as_ref().cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => {
                json!({"protocolVersion":1,"provider":{"id":"parameter-demo","name":"参数演示","version":version},"platform":{"avatarShape":"rounded","history":"none","actions":{}},"capabilities":{"conversations":{"list":false},"messages":{"list":false,"get":false},"events":{"messages":true,"conversations":true},"resources":{"read":false,"avatars":false}},"settings":fields})
            }
            "session/open" => {
                let values = params["settings"].as_object().cloned().unwrap_or_default();
                let mut public = values.clone();
                public.remove("token");
                public.insert(
                    "tokenReceived".into(),
                    json!(
                        values
                            .get("token")
                            .and_then(Value::as_str)
                            .is_some_and(|s| !s.is_empty())
                    ),
                );
                // 示例的证据只含普通值；实际插件自行处理 session/open 的参数。
                std::fs::write("applied.json", serde_json::to_vec_pretty(&public).unwrap())
                    .expect("参数证据落盘");
                if logged_in || params["credential"].as_str().is_some() {
                    json!({"state":"online","account":{"id":"parameter-me","name":format!("参数演示 · {} 秒",values.get("delay").and_then(Value::as_str).unwrap_or("5"))}})
                } else {
                    json!({"state":"needsLogin","reason":"noCredential","message":"点登录完成本机演示。"})
                }
            }
            "login/start" => json!({"flowId":"parameter-login"}),
            _ => json!({}),
        };
        send(&mut out, RpcMessage::ok(id, result));
        if method == "login/start" {
            logged_in = true;
            send(
                &mut out,
                RpcMessage::request(
                    RpcId::Number(1000000),
                    "credential/save",
                    json!({"credential":"parameter-demo-login"}),
                ),
            );
            send(
                &mut out,
                RpcMessage::notification(
                    "login/finished",
                    json!({"flowId":"parameter-login","account":{"id":"parameter-me","name":"参数演示"}}),
                ),
            );
        }
        if method == "shutdown" {
            break;
        }
    }
}
