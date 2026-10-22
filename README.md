# hhh-im-sdk

给 [HHH](https://github.com/MagicLizi) 写 IM 插件用的协议包与工具。HHH 的 IM 层只做统一的界面和一个统一的插座
（HHH-IM-Protocol）：每种 IM 都是插在这个插座上的插件，负责和自己的平台说话、把平台的格式翻成统一格式、声明自己能做什么
和要用户填什么；聊天记录、凭据、配置都交给 HHH。

本仓库从 HHH 仓库按版本同步过来（HHH 仓库是唯一正本），不要在这里直接改协议。

## 里面有什么

| 内容 | 说明 |
|---|---|
| `hhh-im-protocol/` | 协议的 Rust 包：线上类型（含格式正文与卡片的 `content`）、JSON-RPC 编解码、写插件程序用的运行时（feature `runtime`）、子进程 stdio 传输（feature `transport`）、安装包 `.hhhim` 的打包与解包（feature `package`） |
| `hhh-im-check` | 自检工具：拿你的插件把协议用例跑一遍 |
| `hhh-im-pack` | 打包工具：把插件文件夹打成 `.hhhim`、算 sha256、写版本索引；`--publish` 发到 GitHub Release 或 GitLab |
| `docs/protocol-v1.md` | 协议规范 |
| `docs/release.md` | 发版本规范：标签、Release、版本索引、包放在哪 |

## 用法

插件里依赖协议包（按标签取一个固定版本）：

```toml
[dependencies]
hhh-im-protocol = { git = "https://github.com/MagicLizi/hhh-im-sdk", tag = "v0.9.0", features = ["runtime"] }
```

运行时（`hhh_im_protocol::runtime`）管每个插件都要的、和平台无关的那一半：读写标准输入输出、宿主的请求各开一个任务并发处理、
`$/cancel`、向宿主发请求并等回复（`credential/save`、`events/push`）、日志转 `log/write`、资源分段读、上传攒齐与核对、
向平台发了几次请求。插件实现 `Plugin`（收到某个方法怎么办），交给 `serve_on` / `serve_stdio` / `serve_stdio_on`；模板仓库就是这样写的。
声明 `readRequestGating` 后，在每条平台读取出口等待 `metering::read_request().await?`，宿主放行后再发 HTTP；运行时自动确认支持回执的许可，避免通信恢复时积攒许可集中生效。旧宿主没有给额度凭据时仍按结果计数。详见协议 §8。
编成 `wasm32-wasip2` 给 iPhone / iPad 用时，`serve_stdio*` 在 WASI 上自己接好标准输入输出（见 `docs/release.md` §2）。

账号级操作可声明 shortcut:true，让宿主在会话区显示快捷入口；操作完成可返回 openConversationId，支持宿主核对并打开结果会话（旧宿主可忽略）。详见协议 §6.7。

装两个工具：

```sh
cargo install --git https://github.com/MagicLizi/hhh-im-sdk --tag v0.9.0 hhh-im-protocol --features transport,package
```

- 自检：`hhh-im-check <插件文件夹>/hhh-im-provider.json`
- 打包：`hhh-im-pack <插件文件夹> --target windows-x64`（不写 `--target` 用这台电脑的目标）
- 发布：`hhh-im-pack <插件文件夹> --notes notes.md --publish`，令牌放环境变量 `HHH_IM_PUBLISH_TOKEN`（GitHub 也认 `GITHUB_TOKEN`，GitLab 也认 `GITLAB_TOKEN`），不写进任何文件
- GitLab 发布会同时更新包仓库的最新版本索引；旧版本可用 `hhh-im-pack <插件文件夹> --publish --latest-only` 只补索引，清单版本必须已发布

私有插件的拥有者可在支持分享码的 HHH 中点「管理 → 分享…」，朋友粘贴分享码安装和更新。GitHub 的清单 `source` 应指向单独的发布仓库，只放安装包与版本索引；GitLab 分享使用只读包仓库的部署令牌。发布令牌留在发布者电脑，不能当分享码发给朋友。详见 [发版本规范](docs/release.md)。

从零起一个插件，用模板仓库 [hhh-im-plugin-template](https://github.com/MagicLizi/hhh-im-plugin-template)：清单、标志、插件骨架、打包脚本、
给 agent 的规范都在里面。

## 许可证

[MIT](LICENSE)
