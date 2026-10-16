# 发版本规范（HHH IM 插件）

HHH 只从插件清单 `source` 写的仓库检查、下载新版本，只读一个文件：每个版本附带的版本索引 `hhhim-release.json`。
本文是发版本的全部规矩；打包工具 `hhh-im-pack` 与模板仓库的 `scripts/pack` 都按它做。

## 1. 清单里的发布字段（都可选）

| 字段 | 写法 |
|---|---|
| `source` | 更新来源，三种之一：`{"type":"github","repo":"<owner>/<name>"}`、`{"type":"gitlab","host":"https://<站点>","project":"<group>/<name>"}`、`{"type":"url","index":"https://…/hhhim-release.json"}`。不写就没有更新来源 |
| `minHost` | 需要的最低 HHH 版本（`主.次.修订`）；HHH 更旧时卡片写“新版本需要先更新 HHH”，不给更新 |
| `target` | 不用自己写：打包工具按 `--target` 写进包里的清单 |

有更新来源的插件，`version` 必须是 `主.次.修订` 三段数字。

## 2. 安装包 `.hhhim`

- 就是一个 zip，最上层是清单 `hhh-im-provider.json`，其余文件按清单里的相对路径放。
- 一个包只给一种设备：`windows-x64`（ARM 的 Windows 也用它）、`macos-arm64`、`macos-x64`、`linux-x64`、`linux-arm64`，
  以及 iPhone / iPad 的 `wasm32-wasip2`（清单 `os` 写 `ios`）。
- 包名 `<插件编号>-<版本>-<目标>.hhhim`，如 `wechat-pad-0.4.0-windows-x64.hhhim`。
- 清单的 `command` 不写扩展名时，打 Windows 包会自动指向 `<command>.exe`、打 `wasm32-wasip2` 包指向 `<command>.wasm`，
  同一份清单能打各种设备的包。
- `wasm32-wasip2` 包里放编成 WASM 组件的程序（`cargo build --release --target wasm32-wasip2`），iPhone / iPad 上的 HHH
  在 App 里运行它：照常经标准输入输出说协议（插件运行时 `serve_stdio_on` 已在 WASI 上接好），能连网络（TCP、域名解析），
  只看得到自己的临时目录，不能起子进程、不能开监听端口、没有组播。
- HHH 解包时拒绝绝对路径、`..`、链接，以及超过 1 万个文件或解压后 1 GB 的包。

## 3. 发一个版本

1. 改清单的 `version`，提交。
2. 在仓库打标签 `v<版本>`（与清单 `version` 完全一致，只能往上加），推上去。
3. 每台要出包的电脑上：`hhh-im-pack <插件文件夹> --target <目标> --notes notes.md --publish`。
   - 新版本要比仓库里最新的正式版本大；同一版本补传别的目标可以。
   - 仓库里同一版本已有的索引会先合进来，几台电脑各打各的目标，索引里就有全部目标。
   - HHH 只读最新正式版本的索引：这一版没出某种设备的包，这种设备上已装的停在旧版、没装的从仓库装不上；
     之后给同一版本补传这个目标就好。
   - 令牌放环境变量 `HHH_IM_PUBLISH_TOKEN`（GitHub 也认 `GITHUB_TOKEN`，GitLab 也认 `GITLAB_TOKEN`），不写进任何文件。
     GitHub 用能写 Contents 的令牌；GitLab 用带 `api` 的个人访问令牌。

正式 Release（草稿、预发布不算）上附：各目标的 `.hhhim` 包，加一个版本索引 `hhhim-release.json`。

## 4. 版本索引 `hhhim-release.json`

```json
{
  "indexVersion": 1,
  "id": "wechat-pad",
  "name": "微信",
  "publisher": "MagicLizi",
  "imType": "wechat",
  "logo": "data:image/svg+xml;base64,…",
  "version": "0.4.0",
  "minHost": "0.1.0",
  "notes": "更新说明（Markdown，给用户看，≤ 2000 字）",
  "packages": [
    { "target": "windows-x64", "file": "wechat-pad-0.4.0-windows-x64.hhhim", "size": 12345678, "sha256": "…" }
  ]
}
```

- `name`、`publisher`、`imType`、`logo` 来自清单，从仓库安装时预览用（`logo` 是清单标志的 data URL，太大就不放）。
- `notes` 在 HHH 里按 Markdown 显示（标题、列表、加粗、代码、链接；HTML 不执行）。
- HHH 下载后核对大小与 `sha256`，对不上不装。

## 5. 包放在哪

| 来源 | 放法 |
|---|---|
| GitHub | Release 的附件 |
| GitLab | 项目的通用包仓库 `packages/generic/<插件编号>/<版本>/<文件名>`，Release 上挂指向它的链接（不放附件上传区：令牌下载不了） |
| 网址 | 包与索引放同一目录，`file` 按索引地址解析 |

## 6. 私有仓库

私有仓库的插件，用户在 HHH 里添加 IM 时填一次这个站点的访问令牌（GitLab 只勾 `read_api`；GitHub 细粒度令牌只读 Contents），
之后安装、检查、更新都用它。公司内部插件推荐放在 GitLab“内部”可见的项目里，每个人用自己账号的令牌，不发共用令牌。

## 7. 插件编号一旦发布不能改

HHH 靠编号认“同一个插件”：改了就是新插件，聊天记录不跟过去。`feishu`、`slack` 等官方插件的编号是保留的，只有带官方签名的版本索引能用。
