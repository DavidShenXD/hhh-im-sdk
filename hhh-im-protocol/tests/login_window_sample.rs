//! 契约 provider-protocol v36：示例插件的“在登录窗口里登录”走一遍——推登录窗口一步（合格的起始网址与 `hhh-sample://`
//! 开头）→ 宿主交回截下的链接 → 插件存凭据 → 登录成功；交回对不上这次登录的链接 → 登录失败并说原因。
//!
//! 起的是 `cargo test` 顺带编好的示例程序（`target/<配置>/examples/sample_provider`）；要 `transport`（宿主一侧的传输）。
#![cfg(feature = "transport")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hhh_im_protocol::method::{LoginStep, step_problem};
use hhh_im_protocol::transport::{Connection, Incoming, StdioChild};
use serde_json::{Value, json};

/// 和本测试同一个配置目录下编好的示例程序。
fn sample_program() -> PathBuf {
    let exe = std::env::current_exe().expect("测试程序的位置");
    let profile = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target/<配置>/deps");
    let program = profile
        .join("examples")
        .join(format!("sample_provider{}", std::env::consts::EXE_SUFFIX));
    assert!(
        program.exists(),
        "没找到示例程序 {}：先 cargo build -p hhh-im-protocol --example sample_provider --features transport",
        program.display()
    );
    program
}

/// 等插件发来下一条指定方法的请求或通知；途中的 `credential/save` 回“存好了”，别的请求回空。
async fn next(conn: &Arc<Connection>, want: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match conn.next_incoming().await.expect("插件连接断了") {
                Incoming::Request { id, method, params } => {
                    let _ = conn.respond(id, json!({})).await;
                    if method == want {
                        return params;
                    }
                }
                Incoming::Notification { method, params } => {
                    if method == want {
                        return params;
                    }
                }
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("10 秒内没等到 {want}"))
}

fn state_of(url: &str) -> String {
    url.split_once("state=")
        .map(|(_, rest)| rest.split('&').next().unwrap_or_default().to_owned())
        .expect("起始网址里带 state")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_sample_logs_in_through_a_login_window_and_refuses_a_wrong_link() {
    let program = sample_program();
    let dir = tempfile::tempdir().unwrap();
    let child = StdioChild::spawn(program.to_str().unwrap(), &[], dir.path(), |_| {}).unwrap();
    let conn = Connection::new(child.clone());
    let init = conn
        .call("initialize", json!({ "protocolVersions": [1], "host": { "name": "测试", "version": "0" }, "locale": "zh-CN", "tempDir": dir.path() }))
        .await
        .unwrap();
    let methods = init["loginMethods"].as_array().unwrap();
    assert!(
        methods
            .iter()
            .any(|m| m["id"] == "window" && m["kind"] == "direct"),
        "声明了在登录窗口里登录：{methods:?}"
    );
    let open = conn
        .call(
            "session/open",
            json!({ "credential": null, "settings": {}, "syncState": null }),
        )
        .await
        .unwrap();
    assert_eq!(open["state"], "needsLogin");

    // 走一遍：登录窗口一步 → 截下的链接 → 存凭据 → 登录成功。
    let flow = conn
        .call("login/start", json!({ "method": "window", "values": {} }))
        .await
        .unwrap()["flowId"]
        .as_str()
        .unwrap()
        .to_owned();
    let step = next(&conn, "login/step").await;
    assert_eq!(step["flowId"], flow.as_str());
    let parsed: LoginStep = serde_json::from_value(step["step"].clone()).unwrap();
    assert_eq!(step_problem(&parsed), None, "宿主认这一步合格");
    let LoginStep::LoginWindow { url, capture, .. } = parsed else {
        panic!("不是登录窗口一步：{step}")
    };
    assert!(url.starts_with("http://127.0.0.1:") && url.contains("/window?state="));
    assert_eq!(capture, vec!["hhh-sample://".to_owned()]);
    let key = state_of(&url);
    let ok = conn
        .call(
            "login/callback",
            json!({ "flowId": flow, "url": format!("hhh-sample://done?state={key}&code=sample-code") }),
        )
        .await;
    assert!(ok.is_ok(), "插件收下截下的链接：{ok:?}");
    let saved = next(&conn, "credential/save").await;
    assert!(saved["credential"].is_string(), "存了凭据：{saved}");
    let finished = next(&conn, "login/finished").await;
    assert_eq!(finished["flowId"], flow.as_str());

    // 交回对不上这次登录的链接：登录失败并说原因。
    let flow = conn
        .call("login/start", json!({ "method": "window", "values": {} }))
        .await
        .unwrap()["flowId"]
        .as_str()
        .unwrap()
        .to_owned();
    let _ = next(&conn, "login/step").await;
    let _ = conn
        .call(
            "login/callback",
            json!({ "flowId": flow, "url": "hhh-sample://done?state=wrong&code=sample-code" }),
        )
        .await;
    let failed = next(&conn, "login/failed").await;
    assert_eq!(failed["flowId"], flow.as_str());
    assert_eq!(failed["message"], "登录窗口交回来的链接对不上这次登录。");

    // 没有登录码：同样失败并说原因。
    let flow = conn
        .call("login/start", json!({ "method": "window", "values": {} }))
        .await
        .unwrap()["flowId"]
        .as_str()
        .unwrap()
        .to_owned();
    let step = next(&conn, "login/step").await;
    let key = state_of(step["step"]["url"].as_str().unwrap());
    let _ = conn
        .call(
            "login/callback",
            json!({ "flowId": flow, "url": format!("hhh-sample://done?state={key}") }),
        )
        .await;
    let failed = next(&conn, "login/failed").await;
    assert_eq!(failed["message"], "登录窗口交回来的链接里没有登录码。");
    child.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_capture_is_a_step_the_host_refuses() {
    let program = sample_program();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("login-window-bad-capture"), "").unwrap();
    let child = StdioChild::spawn(program.to_str().unwrap(), &[], dir.path(), |_| {}).unwrap();
    let conn = Connection::new(child.clone());
    conn.call("initialize", json!({ "protocolVersions": [1], "host": { "name": "测试", "version": "0" }, "locale": "zh-CN", "tempDir": dir.path() }))
        .await
        .unwrap();
    conn.call(
        "session/open",
        json!({ "credential": null, "settings": {}, "syncState": null }),
    )
    .await
    .unwrap();
    conn.call("login/start", json!({ "method": "window", "values": {} }))
        .await
        .unwrap();
    let step = next(&conn, "login/step").await;
    let parsed: LoginStep = serde_json::from_value(step["step"].clone()).unwrap();
    assert!(
        step_problem(&parsed).is_some_and(|why| why.contains("跳转开头")),
        "不合格的开头宿主不认：{step}"
    );
    child.shutdown().await;
}
