//! 运行时的协议服务：用 `transport` 一侧的写法当宿主，经内存管道和一个用运行时写的小插件对话。

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

use super::*;
use crate::rpc::RpcId;

/// 小插件：`slow` 睡很久（等着被取消）；`fast` 马上回；`save` 先向宿主发 `credential/save` 等回复，把回复交回；
/// `count` 数三次平台请求；`self` 自己回复后再推一条通知；`fail` 回错误。
struct Small;

impl Plugin for Small {
    async fn request(self: Arc<Self>, call: Call) -> Reply {
        match call.method.as_str() {
            "slow" => {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Reply::from(Ok(json!({ "slow": true })))
            }
            "fast" => Reply::from(Ok(json!({ "fast": true }))),
            "save" => Reply::from(
                call.host
                    .request("credential/save", json!({ "credential": { "token": "t" } }))
                    .await
                    .map(|saved| json!({ "saved": saved })),
            ),
            "count" => {
                for _ in 0..3 {
                    metering::request();
                }
                Reply::from(Ok(json!({ "items": [] })))
            }
            "self" => {
                call.host
                    .respond(call.id.clone(), Ok(json!({ "started": true })));
                call.host.notify("login/step", json!({ "flowId": "f1" }));
                Reply::Responded
            }
            other => Reply::from(Err(method_not_found(other))),
        }
    }

    async fn notification(self: Arc<Self>, method: String, _params: Value, host: Host) {
        if method == "login/cancel" {
            host.notify("status", json!({ "state": "online", "seen": method }));
        }
    }
}

/// 宿主这一头：往插件写一行、读插件写出来的下一行。
struct FakeHost {
    to_plugin: DuplexStream,
    from_plugin: tokio::io::Lines<BufReader<DuplexStream>>,
}

impl FakeHost {
    async fn send(&mut self, message: Value) {
        let line = format!("{message}\n");
        self.to_plugin.write_all(line.as_bytes()).await.unwrap();
    }

    async fn next(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(5), self.from_plugin.next_line())
            .await
            .expect("插件 5 秒内该有输出")
            .unwrap()
            .expect("插件的输出不该已经关了");
        serde_json::from_str(&line).unwrap()
    }
}

fn start() -> (FakeHost, tokio::task::JoinHandle<()>) {
    let (host_side, plugin_in) = tokio::io::duplex(64 * 1024);
    let (plugin_out, host_read) = tokio::io::duplex(64 * 1024);
    let server = tokio::spawn(serve(Small, plugin_in, plugin_out));
    (
        FakeHost {
            to_plugin: host_side,
            from_plugin: BufReader::new(host_read).lines(),
        },
        server,
    )
}

fn request(id: i64, method: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": {} })
}

// 请求并发：慢的不挡快的；`$/cancel` 停掉慢的并回“已取消”。
#[tokio::test]
async fn requests_run_concurrently_and_can_be_cancelled() {
    let (mut host, _server) = start();
    host.send(request(1, "slow")).await;
    host.send(request(2, "fast")).await;
    let fast = host.next().await;
    assert_eq!(
        (fast["id"].clone(), fast["result"]["fast"].clone()),
        (json!(2), json!(true))
    );
    host.send(json!({ "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": 1 } }))
        .await;
    let cancelled = host.next().await;
    assert_eq!(cancelled["id"], 1);
    assert_eq!(cancelled["error"]["data"]["kind"], "cancelled");
}

// 插件向宿主发请求并等回复；宿主回了才把结果交回原请求。
#[tokio::test]
async fn plugin_requests_wait_for_the_host_reply() {
    let (mut host, _server) = start();
    host.send(request(7, "save")).await;
    let asked = host.next().await;
    assert_eq!(asked["method"], "credential/save");
    assert_eq!(asked["params"]["credential"]["token"], "t");
    let id = asked["id"].clone();
    assert!(
        id.as_str().is_some_and(|id| id.starts_with('p')),
        "插件的编号是字符串，不和宿主撞号"
    );
    host.send(json!({ "jsonrpc": "2.0", "id": id, "result": { "ok": true } }))
        .await;
    let reply = host.next().await;
    assert_eq!(reply["id"], 7);
    assert_eq!(reply["result"]["saved"]["ok"], true);
}

// 数到的平台请求多于一次时补上 platformRequests；自己回复的不再多回；不认识的方法回 -32601；通知交给插件。
#[tokio::test]
async fn counts_self_replies_unknown_methods_and_notifications() {
    let (mut host, _server) = start();
    host.send(request(3, "count")).await;
    let counted = host.next().await;
    assert_eq!(counted["result"]["platformRequests"], 3);
    host.send(request(4, "self")).await;
    let started = host.next().await;
    assert_eq!(
        (started["id"].clone(), started["result"]["started"].clone()),
        (json!(4), json!(true))
    );
    let step = host.next().await;
    assert_eq!(step["method"], "login/step");
    host.send(request(5, "nope")).await;
    let unknown = host.next().await;
    assert_eq!(unknown["error"]["code"], crate::rpc::code::METHOD_NOT_FOUND);
    host.send(json!({ "jsonrpc": "2.0", "method": "login/cancel", "params": { "flowId": "f1" } }))
        .await;
    let status = host.next().await;
    assert_eq!(status["params"]["seen"], "login/cancel");
}

// 输入读完：还在跑的请求给一小会儿做完，写完就退出（不会因为有人还拿着宿主那一头而一直等）。
#[tokio::test]
async fn ends_when_the_input_closes() {
    let (mut host, server) = start();
    host.send(request(9, "fast")).await;
    host.send(request(10, "slow")).await;
    drop(host.to_plugin);
    let fast = tokio::time::timeout(Duration::from_secs(5), host.from_plugin.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(fast.contains("\"fast\":true"));
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("输入关了，服务该退出")
        .unwrap();
}

#[test]
fn platform_requests_are_added_only_when_more_than_one() {
    assert_eq!(with_platform_requests(Ok(json!({})), 1).unwrap(), json!({}));
    assert_eq!(
        with_platform_requests(Ok(json!({})), 2).unwrap(),
        json!({ "platformRequests": 2 })
    );
    assert_eq!(
        with_platform_requests(Ok(json!({ "platformRequests": 9 })), 2).unwrap(),
        json!({ "platformRequests": 9 }),
        "插件自己写了就不改"
    );
    assert_eq!(
        with_platform_requests(Ok(json!([1])), 3).unwrap(),
        json!([1])
    );
    let _ = RpcId::Number(1);
}
