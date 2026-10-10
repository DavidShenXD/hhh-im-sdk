# HHH-IM-Protocol v1 规范（草案）

> 状态：draft，随 [contract-v1](contract-v1.md) 一起确认。日期：2026-10-04。
> 本文规定宿主（HHH）与外部 IM 程序之间交换什么。程序内部怎么写，见[通用实现范式](provider-guide-v1.md)。

## 0. 术语

- **宿主**：HHH。具体由 hhh-im 的“外部程序适配器”实现，Friend 负责装配。
- **IM 程序**（简称程序）：实现本协议的一个独立可执行程序，用什么语言写都行。它在 HHH 里的位置和飞书、Slack 适配器相同，只做三件事：**拿数据、做动作、报变化**。
- **连接**：HHH 里的一个 IM 账号。一个程序进程只服务一个连接。

## 1. 原则（小版本里不改）

1. **数据和配置全部在 HHH。** 宿主用 HHH 现有的本地库、文件库和系统凭据库保存所有东西：聊天记录、话题、表情、已读未读、发送记录、人名册、会话资料、文件、凭据、同步位置和程序设置。程序不落盘任何业务数据，只能用内存缓存，以及宿主给的临时目录（宿主随时可能清空它）。
2. **一个进程一个账号。** 宿主给每个连接启动一个程序进程。协议里的编号都限定在这个账号里，所以不带账号编号或连接编号。
3. **按能力声明办事。** 握手时程序交出平台说明和能力清单。宿主只调用声明过的方法，界面只显示声明过的操作。没有声明发送能力的连接是只读的。
4. **宿主校验是为了保数据、防 bug，不是安全隔离。** 大小、数量、频率、超时和编号格式都由宿主检查；程序声明的上限只能比宿主的更严，不能更松；程序给的网址界面一律不直接打开。程序以用户自己的权限运行，真正的信任边界在“安装”这一步——用户看清楚程序路径、来源和能力后才启用。
5. **推送至少送达一次。** 程序推来的变化，宿主写进本地库之后才确认；没得到确认的，程序要重发。重复到达的内容按编号去重。
6. **平台私有的东西留在程序里。** 私有编码、私有格式、加密和签名，都由程序转换成本协议的通用形式。宿主看不到平台的原始数据。程序可以随消息附上**去凭据化**的原始数据，供以后重新翻译（§7.4、§10.1），这份数据里不能含令牌、密钥或别的凭据。
7. **向前兼容。** 不认识的字段，双方都忽略；不认识的内容块，显示为“暂不支持”；新功能只以可选能力的方式加入。

## 2. 程序包

每个程序带一个清单文件 `hhh-im-provider.json`，放在程序文件夹的最上层。用户在 HHH「设置 → IM → 添加 IM」里选安装包 `.hhhim`、填仓库地址或选程序文件夹（开发用）；HHH 读清单、试启动一次，通过后把程序复制（安装包是解开）进自己的 IM 插件目录再运行，原文件之后可以删或挪。更新来自清单 `source` 写的仓库（点“更新”），也可以「从文件更新」选新版本的安装包或文件夹（编号必须相同）。

安装包、版本索引与发版本的规矩见[发版本规范](release.md)（公开仓库 `hhh-im-sdk` 的 `docs/release.md` 是它的副本），要点（契约 v10 §4.1–§4.3）：安装包就是 zip，最上层是清单；一个包只给一种电脑（`windows-x64`、`macos-arm64`、`macos-x64`、`linux-x64`、`linux-arm64`）；`command` 不写扩展名时，Windows 包里指向 `<command>.exe`，宿主在 Windows 上找不到 `command` 时也试 `<command>.exe`。

```json
{
  "manifestVersion": 1,
  "id": "wechat-pad",
  "name": "微信",
  "version": "0.3.0",
  "command": "bin/wechat-provider.exe",
  "args": ["--hhh"],
  "os": ["windows"],
  "logo": "logo.svg",
  "publisher": "Lizi",
  "homepage": "https://example.com",
  "imType": "wechat"
}
```

| 字段 | 规则 |
|---|---|
| `manifestVersion` | 清单格式的版本，现在是 1 |
| `id` | 程序编号：以小写字母开头，只含 `a-z0-9-`，长 2–32 个字符。宿主内部使用的平台编号是 `ext-<id>`，因此永远不会和内置平台（`feishu`、`slack`）或已下线的平台（`wechat`）撞名。同一个编号只能装一个程序 |
| `name` | 给用户看的平台名，1–20 个字符 |
| `version` | 程序版本，semver 写法 |
| `command` / `args` | 启动命令。`command` 按清单所在目录解析相对路径，也可以是 PATH 里能找到的程序（如 `node`、`python`）；`args` 原样传入 |
| `os` | 支持的系统：`windows` / `macos` / `linux`。当前系统不在列表里时不能添加 |
| `logo` | 可选。SVG、PNG、JPG 或 WebP，不超过 256 KiB，路径相对清单所在的文件夹、不能走出这个文件夹。没有 logo 时，宿主用名字的第一个字画一个 |
| `publisher` / `homepage` | 可选。添加时展示给用户 |
| `imType` | 可选（v5）。IM 类型，规则同 `id`。内置飞书是 `feishu`、内置 Slack 是 `slack`；写成同一个值就和它们是同一种，宿主里同一种同一时间只用一个。不写时自成一类、不和任何 IM 互斥。只认这个字段，插件编号碰巧同名不算同一种 |
| `source` | 可选（v10）。更新来源：`{"type":"github","repo":"<owner>/<name>"}`、`{"type":"gitlab","host":"https://<站点>","project":"<group>/<name>"}`、`{"type":"url","index":"https://…/hhhim-release.json"}` 之一。不写就没有更新来源。写了时 `version` 必须是 `主.次.修订` 三段数字。宿主只接受同一来源、同一编号的更新 |
| `minHost` | 可选（v10）。需要的最低 HHH 版本（`主.次.修订`）；HHH 更旧时不给装、不给更新 |
| `target` | 可选（v10）。这个包给哪种电脑，打包工具按 `--target` 写进包里的清单，不用手写。从安装包装时必须有且是本机；文件夹装时可以没有，照旧按 `os` 判断 |

清单里只放启动前就必须知道的信息。平台说明和能力在握手时给出（§5），可以随程序版本变化。

## 3. 传输

- 消息格式是 **JSON-RPC 2.0**。宿主发给程序的消息走程序的标准输入，程序发给宿主的走标准输出。编码 UTF-8，**一行一条消息**：以 `\n` 结尾，消息内部不能出现原始换行。
- 双方都可以发请求和通知。程序发给宿主的请求只有本文列出的几种（§6.3、§11.1）。
- **标准输出只能输出协议消息。** 宿主每读到一行不合法的 JSON-RPC，记一次协议错误；同一个进程连续 3 次，宿主就重启它。
- **标准错误输出是日志**：每行一条。宿主把每行截到 4 KiB，标上程序编号，写进 HHH 日志。日志里不要写凭据和消息正文。
- **单条消息最大 4 MiB**。超过的消息宿主直接丢掉，按协议错误处理。大文件要分段传（§9）。
- 宿主启动程序时，设置环境变量 `HHH_IM_PROTOCOL=1`，工作目录是一个空的临时目录。凭据不通过环境变量传。
- 第一版只有这一种传输方式。方法和类型都不依赖传输，以后可以再加本机 WebSocket 或远程连接。
- **结构化日志**：除了标准错误输出，程序还可以发 `log/write` 通知（§11.5），把一条带级别和字段的日志交给宿主，宿主并进自己的日志系统。两者都不能写凭据或消息正文。

## 4. 生命周期

```
启动进程 → initialize → session/open ─┬→ 在线：读取 / 动作 / 推送
                                      └→ 需要登录：login/start … credential/save → login/finished → 在线
在线时程序发 session/expired（授权失效）→ 宿主提示重新登录
任何时候 → shutdown → 宿主关闭标准输入 → 程序 5 秒内退出（超时宿主就结束进程）
```

### 4.1 initialize（宿主→程序，必需）

参数：

```json
{
  "protocolVersions": [1],
  "host": { "name": "HHH", "version": "0.9.0" },
  "locale": "zh-CN",
  "tempDir": "<宿主给的临时目录，可随时被清空>",
  "network": { "proxy": null }
}
```

结果：

```json
{
  "protocolVersion": 1,
  "provider": { "id": "wechat-pad", "name": "微信", "version": "0.3.0" },
  "platform": { "…": "平台说明，见 §5.1" },
  "capabilities": { "…": "能力，见 §5.2" },
  "settings": [ "…程序设置项，见 §6.4" ]
}
```

- `protocolVersion` 必须是宿主列出的版本之一，否则宿主拒绝连接，并提示“程序版本与 HHH 不兼容”。
- `provider.id` 必须和清单里的一致。
- 宿主保存最近一次收到的平台说明。没连上程序时，宿主按这份说明只读打开本地库。
- `network.proxy` 是 HHH 的出站代理设置。程序访问平台时应当照这个设置走，这样用户在 HHH 里设的网络规则对程序也生效。
- 超时 10 秒。

### 4.2 session/open（宿主→程序，必需）

用宿主保存的凭据、设置和同步位置恢复账号。

参数：

```json
{ "credential": "<不透明字符串，或 null>", "settings": { "<键>": "<值>" }, "syncState": "<不透明字符串，或 null>" }
```

结果是下面两种之一：

- `{ "state": "online", "account": Account }`
- `{ "state": "needsLogin", "reason": "noCredential|expired|revoked|credentialInvalid", "message": "<给用户看的说明>" }`

规则：

- `account.id` 和宿主上次核验的账号不同时，宿主按现有规则处理（hhh-im 的 `AccountChanged`）：停止读写旧账号的库，提示用户换了账号。
- 超时 30 秒。

### 4.3 shutdown（宿主→程序）

没有参数。程序停掉所有平台请求和推送，回复空结果，然后等标准输入关闭后退出。

### 4.4 心跳与重启

- 宿主每 30 秒发一次 `ping`（没有参数，结果为空）。连续两次在 10 秒内没有回复，就当作程序卡死。
- 程序退出或卡死时，宿主按 1、2、4……最长 60 秒的间隔退避重启。10 分钟内重启 5 次还不行，宿主就停下来，在界面上把这个连接标成“程序出错”，并附上最后几行日志。
- 每次重启都从 `initialize` 开始，再用保存的凭据和同步位置接着上次的进度。

## 5. 平台说明与能力

### 5.1 平台说明（`platform`）

宿主根据平台说明来限速、分页、显示界面，并计算每条消息允许哪些操作。它对应 hhh-im 的 `Platform` / `PlatformInfo`。

| 字段 | 类型 | 必需 | 说明 |
|---|---|---|---|
| `avatarShape` | `circle` \| `rounded` | 是 | 头像在平台自己的客户端里是什么形状 |
| `chatLink` | 字符串 | 否 | 在平台客户端里打开会话的链接模板，`{conversation}` 会换成 URL 编码后的会话编号。不给就不显示“在 xx 里查看” |
| `imageNamePrefix` | 字符串 | 否 | 用户下载图片时文件名的开头，默认用平台名 |
| `conversationNouns` | `{group, direct, channel, unknown}` | 否 | 各类会话怎么称呼，默认依次是“群 / 私聊 / 频道 / 会话” |
| `history` | `server` \| `none` | 是 | `server`：消息随时可以向平台重新读取（飞书、Slack 这类）。`none`：平台只推一次，不能翻历史（微信这类）。见 §11.4 |
| `pageMax` | 整数 1–100 | `history=server` 时必需 | 按会话读消息时，一页最多几条 |
| `reads` | `{perSecond, perMinute}` | `history=server` 时必需 | 读取请求的合计上限。宿主在它和自己的上限（每秒 20 次、每分钟 1200 次）里取更严的 |
| `downloads` | `{perSecond, perMinute}` | 否 | 下载消息资源的上限，平台不限就省略 |
| `fileSegmentBytes` | 整数，64 KiB–8 MiB | 否 | 文件按多大一段读，支持续传。不给就整份读，断了从头重下 |
| `limits` | `{imageUploadMaxBytes, fileUploadMaxBytes, attachmentNameMaxBytes}` | 能发图片或文件时必需 | 发送附件的上限 |
| `readDefaults` | 读取间隔（秒） | 否 | 默认读取间隔，字段同 hhh-im 的 `ReadIntervals`。超出宿主允许范围的值会被拒绝 |
| `threads` | 对象或 null | 否 | 平台有话题时给：`{activityOnRoot, readsAfter, broadcast: {native, attachments} 或 null}`，含义同 hhh-im `Platform` 里的同名项 |
| `deletedMessagesVanish` | 布尔 | 否 | 撤回的消息直接从历史里消失（而不是留下“已撤回”标记）时为真。默认假 |
| `reactionsInReads` | 布尔 | 否 | 读消息时会随消息带回完整的表情。默认假 |
| `emojiPerAccount` | 布尔 | 否 | 不同账号的表情目录不一样。默认假 |
| `actions` | 见下文 | 是 | 每条消息允许哪些操作 |
| `emoji` | `{namePattern, unlisted: {placeholderPattern, placeholderLabel}}` | 有表情能力时必需 | 表情编号的规则（正则，JavaScript 语法），以及目录里没有的表情怎么显示 |

`actions` 用声明式规则写，由宿主用通用代码执行。内置平台的这部分是 Rust 函数（`actions` / `edit_draft`），外部程序改用下面这种规则：

```json
{
  "reply": true,
  "thread": false,
  "react": false,
  "recall": { "own": true, "withinSecs": 120 },
  "edit": null
}
```

- `recall` 或 `edit` 为 null，表示不能做。`own: true` 表示只能对本人发的消息做。`withinSecs` 是平台的时限：过了时限宿主就不再显示这个操作，但最终能不能做，仍以平台的答复为准。
- 已撤回的消息和系统通知，一律没有任何操作。
- “编辑”只对内容是文字和提及的消息显示。编辑框里的原文由宿主按通用规则从消息里拼出来，文字和提及分开成几段。

内置平台的平台说明里还有几项，外部程序的平台说明**刻意不设**。原因如下，以免以后误加：

- **不猜表情图，也不补全表情目录**：表情目录由程序在 `emoji/catalog` 里一次给全。
- **不靠比较内容推断“编辑过”**：程序必须自己给出 `editedAtMs`。比较内容是飞书独有的特例。
- **没有头像身份规则**：头像用不透明的键（§7.3），同一个头像的键必须保持不变。
- **没有页面可以直接加载的图片来源**：所有图片都经宿主的文件库读取。

### 5.2 能力（`capabilities`）

没写出来的都算不支持。宿主只调用声明过的方法；如果宿主调用了未声明的方法，那是宿主的错误。

```json
{
  "conversations": { "list": true, "search": false, "members": false, "findDirect": false },
  "messages":      { "list": false, "get": false },
  "people":        { "get": true, "search": false },
  "send":          { "text": true, "mentions": false, "mentionAll": false, "image": true, "file": false, "lookup": false },
  "recall": false,
  "edit": false,
  "reactions":     { "list": false, "add": false, "remove": false, "catalog": false },
  "resources":     { "read": true, "avatars": true },
  "events":        { "messages": true, "deletions": true, "reactions": false, "conversations": true, "people": true },
  "retranslate":   true
}
```

- `retranslate` 为真时，程序随消息附带去凭据化的原始数据（§7.4 的 `raw`），宿主存一张原始数据表；程序翻译规则升级后，宿主调 `messages/retranslate`（§10.1）让它把本地旧消息重新翻译一遍。为假时不附带原始数据、也不会被调用重译。

- **只读连接**：`send` 为 null，或者其中各项都是 false。宿主不显示输入框，伙伴不在这个连接上回复，所有发送请求都在本地直接拒绝。
- `history=server` 时，`messages.list` 必须为真。`history=none` 时，`events.messages` 必须为真。
- `history=none` 而且 `conversations.list` 为假时，`events.conversations` 必须为真，因为会话清单只能靠推送得到（§11.4）。
- 能力组合不自洽时，宿主拒绝连接，并说明是哪一项。

## 6. 登录、凭据与设置

### 6.1 登录流程

宿主调用 `login/start`（参数 `{}`），程序返回 `{ "flowId": "<编号>" }`。之后程序发 `login/step` 通知，告诉宿主当前该显示什么，宿主用通用组件显示出来：

| `step.kind` | 字段 | 宿主显示什么 |
|---|---|---|
| `qrCode` | `image`（PNG 的 base64，不超过 512 KiB）、`expiresAtMs`、`hint` | 二维码和倒计时。码过期后由程序自己换新码，再发一次这个步骤 |
| `openUrl` | `url`（https，或本机回环地址的 http）、`label` | 一个“打开浏览器”按钮，不在页面里嵌入网页 |
| `confirmOnDevice` | `message` | 等待提示，例如“请在手机上确认” |
| `input` | `fields: [{id, label, secret, placeholder, pattern}]` | 一个表单。用户提交后，宿主调用 `login/submit {flowId, values}` |
| `message` | `text`、`tone`（`info` / `warn`） | 一段说明文字 |

除 `message` 外，每种步骤都可以带可选的 `note: { "text", "tone" }`（`tone` 是 `info` / `warn`，与 `message` 同色调）：程序想在这一步提醒用户什么（例如“扫码后在手机上勾选同步最近的消息”），就在这一步带上，宿主把它画在这一步内容下面、按钮上面；不带就没有。宿主不解析提示的含义，文字原样显示、不支持格式；空文字或超过 §14 上限的提示不显示（这一步照常），不认识的色调按 `info`。旧宿主忽略这个字段。

登录流程以程序发出的两个通知之一结束：`login/finished {flowId, account}` 或 `login/failed {flowId, message, retryable}`。用户中途取消时，宿主发 `login/cancel {flowId}`。

- 同一时间只有一个登录流程。新的 `login/start` 会取消还没结束的旧流程。
- 需要浏览器授权回调（OAuth）的，回调由程序自己接收：程序在本机回环地址上监听，并把这个地址写进 `openUrl` 步骤里。宿主不为程序开回调路由。
- 程序必须在收到 `credential/save` 的成功回复之后，才能发 `login/finished`。

### 6.2 凭据由宿主保存

凭据对宿主来说是一串不透明的字符串，宿主只负责存和还，不解读内容。程序要把重新上线所需的全部材料都放进凭据里，例如登录密钥、设备编号、续期材料。

### 6.3 credential/save（程序→宿主请求）

参数：`{ "credential": "<不透明字符串，不超过 64 KiB>" }`。宿主把它写进系统凭据库（沿用 Friend 现在的分片保存），写好后回复空结果。写失败时宿主回复错误，这时程序不能丢掉旧凭据。

- 这些时候都要调用：登录成功、续期后换了密钥、平台轮换了会话材料。
- **先存后用**：要换凭据时，程序先调用 `credential/save`，等宿主回复成功之后，才能丢掉旧凭据。
- 宿主不把凭据写进日志、wire 记录或 HHH 的配置文件。

### 6.4 程序设置项

`initialize` 结果里的 `settings`，声明程序可配置的参数：

```json
[{ "id": "clientId", "label": "Client ID", "secret": false, "required": true, "pattern": "^[0-9.]+$", "help": "在 Slack 应用后台复制" }]
```

宿主在该 IM 的“管理 → 参数配置”中通用生成表单；无声明时不显示入口。缺少 `type` 时沿用文字项，`secret:true` 始终使用密钥处理；协议主版本仍为 1。

| 可选字段 | 形状与规则 |
|---|---|
| `type` | `text / textarea / number / switch / select / secret`；未知类型明确拒绝 |
| `default` | 字符串；数字以字符串传值，开关只用 `true / false`；密钥不能给默认值 |
| `min / max / unit` | 数字上下界（整数）与显示单位 |
| `options` | 单选的 `[{value,label}]`；值唯一，最多128项 |
| `group` | `{id,label,fold?,help?,note?}`；fold 默认折叠，help 为展开摘要，note 始终显示；同组说明一致 |
| `summaryLabel` | 折叠摘要使用的短标题 |
| `showWhen` | `{field,equals}` 简单字符串相等条件；不能引用密钥或自己；隐藏不丢值 |
| `disabledReason` | 不能编辑时显示的原因，宿主也拒绝变更 |
| `notLessThan / notGreaterThan` | 其它数字字段 id 的顺序关系，可引用数字型单选 |
| `pattern` | regex-lite 支持的便携正则（不支持回溯引用／环视）；服务端校验，前端不执行插件表达式 |

- 参数声明最多128项、整体最多1MiB，字段值与长说明最多8192个UTF-8字节，名称最多512字节；声明只含数据，不执行 HTML、JavaScript 或任意表达式。
- 普通值保存在用户参数配置，密钥只存系统凭据库，读取配置只返回已保存标记。空密钥输入保留原值，清除必须显式指定。
- 插件参数显式保存；开着的插件保存后重新连接，关着的只保存。登录完成后的 `session/open` 同样传入当前参数。
- `session/open.settings` 保持字符串映射，只包含当前声明的项；宿主补默认值并校验，秘密到此才物化为明文，不写日志或普通文件。
- 程序升级后，`id` 与类型兼容的项保留原值；删除或类型变化保留旧存储、明确提示校验问题，用户检查后保存。

### 6.5 授权失效与断开

- 程序发现授权失效时，发通知 `session/expired {reason, message}`，`reason` 是 `expired`、`revoked`、`credentialInvalid` 之一。在重新登录之前，程序对之后的请求一律回授权失效错误（§12）。宿主据此停止读取，并提示用户重新登录，走 hhh-im 现有的 `AuthExpired` 流程。
- 用户在 HHH 里断开连接时：
  1. 宿主调用 `session/logout`。程序尽量在平台上登出，然后丢掉内存里的会话。
  2. 宿主删除凭据，停止进程。
  3. 本地聊天记录和文件按 HHH 现有规则保留，断开不删数据。

## 7. 数据模型

所有字段用 camelCase。时间一律是 Unix **毫秒**。编号都是不透明字符串：1–256 字节，不含控制字符，在这个账号内稳定不变。

### 7.1 Account

`{ "id", "name", "avatar"?: AvatarKey, "tenant"? }`

### 7.2 Conversation

`{ "id", "kind": "group|direct|channel|unknown", "name", "avatar"?, "peer"?: "<私聊对方的用户编号>", "private"?: bool }`

`private` 只在私有频道时为真。

### 7.3 Person 与 AvatarKey

- Person：`{ "id", "kind": "user|bot", "name"?, "avatar"? }`。
- AvatarKey 是不透明字符串，宿主拿它调用 `avatars/read`。同一个头像的键不能变；头像换了，键也必须跟着换。

### 7.4 Message

```json
{
  "id": "…", "conversationId": "…",
  "sender": { "id": "…", "kind": "user|bot|system", "name": "…" },
  "createdAtMs": 0, "updatedAtMs": 0, "editedAtMs": 0,
  "deleted": false, "incomplete": false,
  "content": [ "Block…" ],
  "replyTo": "<被回复的消息编号>",
  "thread": { "id": "…", "rootId": "…", "inThread": true, "broadcast": false,
              "activity": { "latestReplyId": "…", "latestReplyAtMs": 0, "replyCount": 0 } },
  "reactions": [ "Reaction…" ],
  "raw": { "format": "<程序自定的格式名>", "data": "<去凭据化的原始数据，字符串或 base64>" }
}
```

- 除系统通知（`sender.kind` 为 `system`）以外，`sender.id` 必须有值。群消息要给出真实的发送者，不能用群编号代替。
- 撤回的消息：`deleted` 为真，`content` 为空。撤回后仍留在历史里的平台，照常返回这条消息；`deletedMessagesVanish` 为真的平台，改用 `deleted` 事件（§11.2）。
- `reactions`：只要给出，就必须是这条消息**此刻的全部**表情。不知道全部就省略，不能只给一部分。
- `thread` 的含义同 hhh-im 的话题规则：`inThread` 为假的一律不算话题回复。平台没有话题功能时省略。
- `editedAtMs` 只在平台明确表示“改过”时给出。
- `raw` 只在声明了 `retranslate` 能力时给（§10.1）：`format` 是程序自己定的格式名（例如 `wechat-addmsg-v1`），`data` 是去凭据化的原始数据。宿主把它和消息一起存进原始数据表，供以后重新翻译；**不含任何令牌、密钥、会话材料**。单条上限见 §14。

### 7.5 内容块（Block）

| `type` | 字段 | 说明 |
|---|---|---|
| `text` | `text` | 纯文字，原样给出，不含任何标记 |
| `mention` | `userId`?、`name`?、`everyone` | 经平台结构确认的提及。提及所有人时 `everyone` 为真 |
| `resource` | `resource: Resource`、`name`?、`width`?、`height`? | 图片、文件、表情包 |
| `rich` | `root` | 格式正文，即 mdast 的一个子集，节点见 §7.6 |
| `markdown` | `text` | 平台明确标为 Markdown 的原文，由宿主解析 |
| `card` | 结构见附录 B | 卡片：标题、色调和若干块 |
| `unsupported` | `kind`、`label`? | 暂时翻译不了的内容。出现它时，整条消息要标 `incomplete: true` |

宿主遇到不认识的 `type` 时，按 `unsupported` 显示，消息照常保存。

**Resource**：`{ "key", "kind": "image|file|sticker", "mime"?, "bytes"?, "durationMs"? }`

- `key` 只在它所在的那条消息里有意义。宿主读资源时，总会带上会话编号和消息编号。
- v1 里语音、视频、音频都按 `file` 给。`name` 要写成用户看得懂的文件名，例如“语音 0:12.wav”。`mime` 和 `durationMs` 宿主先存着，等 HHH 有了音视频专用的内容块再用上。
- **程序必须交出通用格式的数据。** 平台私有的编码（例如微信语音的 SILK），由程序转成 WAV、MP3、OGG 这类通用格式。

### 7.6 格式正文的节点（v1 固定下来）

`paragraph`、`heading{depth}`、`thematicBreak`、`blockquote`、`list{ordered,start}`、`listItem`、`code{lang,value}`、`table{align,pageSize}`、`tableRow`、`tableCell`、`text{value}`、`strong`、`emphasis`、`delete`、`underline`、`inlineCode{value}`、`break`、`link{url}`、`mention{userId,name,everyone}`、`emoji{emojiType}`、`resource{resource,name,width,height}`、`unsupported{kind,label}`、`color{tone}`、`tag{text,tone}`、`person{userId,name}`、`time{ms,format}`。

- 节点之间的嵌套关系遵守 mdast。宿主对节点数、嵌套深度和文字长度有预算（§14）。
- 不认识的节点按 `unsupported` 处理。

### 7.7 Reaction

`{ "id", "conversationId", "messageId", "operator": Person, "emojiType", "atMs"? }`

### 7.8 发出的内容（Outgoing）

`text{text}`、`mention{userId,name}`、`mentionAll`、`image{key}`、`file{key}`。其中的 `key` 来自 `uploads/finish`。

## 8. 读取方法（宿主→程序）

分页游标都是程序给出的不透明字符串。宿主把游标和这次查询的条件绑在一起，不会拿去别的查询里用。

| 方法 | 参数 | 结果 | 需要的能力 |
|---|---|---|---|
| `conversations/list` | `limit`（1–100）、`cursor`? | `{items: Conversation[], next?}` | `conversations.list` |
| `conversations/get` | `conversationId` | `Conversation` | 必需 |
| `conversations/search` | `text`（1–50 个字）、`limit`（1–20）、`cursor`? | `{items, next?, notice?}` | `conversations.search` |
| `conversations/members` | `conversationId`、`limit`、`cursor`? | `{items: Person[], next?}` | `conversations.members` |
| `conversations/findDirect` | `userId` | `{conversationId?, notice?}`。只定位已有的私聊，不新建会话 | `conversations.findDirect` |
| `people/get` | `ids[]`（不超过 100 个） | `{found: Person[], missing: string[]}`。平台明确说不存在的放进 `missing`；这次没读到的，两边都不放 | `people.get` |
| `people/search` | 同会话搜索 | `{items: Person[], next?, notice?}` | `people.search` |
| `messages/list` | `conversationId`、`threadId`?、`afterMs`?、`beforeMs`?、`newestFirst`、`limit`、`cursor`? | `{items: Message[], next?}`，每页里按从旧到新排 | `messages.list` |
| `messages/get` | `conversationId`、`messageId` | `Message` | `messages.get` |
| `reactions/list` | `conversationId`、`messageId`、`limit`（不超过 50）、`cursor`? | `{items: Reaction[], next?}` | `reactions.list` |
| `emoji/catalog` | 无 | `{emojis: Emoji[], categories?, tones?, updatedAtMs}` | `reactions.catalog` |

- `messages/list` 的翻页规则同 hhh-im 的 `MessageQuery`：`newestFirst` 为真时，从最新往更早翻。
- **读取不能有副作用**：不能借读取在平台上标已读，也不能改变在线状态。已读未读只在 HHH 本地计算。
- Emoji 的字段同 hhh-im 的 `EmojiInfo`，只是图片地址换成 `emoji/image` 用的编号。

## 9. 资源

### 9.1 读取

| 方法 | 参数 | 结果 |
|---|---|---|
| `resources/read` | `conversationId`、`messageId`、`key`、`kind`、`offset`、`length`（不超过 1 MiB） | `{data: base64, total?, sha256?, eof}` |
| `avatars/read` | `key`、`offset`、`length` | 同上 |
| `emoji/image` | `emojiType`、`offset`、`length` | 同上 |

- 排队、并发、超时、重试和落盘都由宿主的下载引擎（hhh-download）负责。程序每次只返回请求的那一段，不缓存整份文件。
- `total` 是整个资源的字节数，不是这一段的长度，能给就尽早给。`sha256` 是平台给的内容指纹（小写十六进制），平台不给就省略。
- 程序声明了 `fileSegmentBytes` 时，宿主按段读，断了可以续传。没有声明时，宿主从 0 开始连续读到 `eof`，中途断了就从头再读。
- 这条消息已经没有这个资源时，回 `notFound`，宿主会换别的消息去取，或者标成“读不到”。暂时读不到时，回 `disconnected`。

### 9.2 上传（发送附件之前）

1. `uploads/begin {kind, name, bytes}` → `{uploadId}`
2. `uploads/chunk {uploadId, offset, data}`：每段不超过 1 MiB，按顺序发
3. `uploads/finish {uploadId, sha256}` → `{key}`
4. `uploads/abort {uploadId}`：中途放弃

程序负责校验长度和摘要。宿主的发送记录会保存 `key`，重试发送时不再重新上传。`key` 不要求在程序重启后仍然有效：失效了就回 `notFound`，宿主会重新上传。

有些平台的上传接口本身就会生成一条消息（例如微信的图片、视频、语音），没有“先上传拿编号、再单独发送”这一步。这类程序在 `uploads/*` 阶段只把字节暂存进临时目录、返回一个本地 `key`，等 `messages/send` 带着这个 `key` 来时才真正上传并生成消息。对宿主来说流程不变：先 `uploads/*` 再 `messages/send`。

## 10. 动作（宿主→程序）

| 方法 | 参数 | 结果 | 需要的能力 |
|---|---|---|---|
| `messages/send` | `operationId`、`target`、`content: Outgoing[]` | `{state: "accepted", message}` 或 `{state: "unknown", reason}` | `send` 里对应的项 |
| `messages/lookupSend` | `operationId` | 同上 | `send.lookup` |
| `messages/recall` | `conversationId`、`messageId` | `{}` | `recall` |
| `messages/edit` | `conversationId`、`messageId`、`content` | 编辑后的 `Message` | `edit` |
| `reactions/add` | `conversationId`、`messageId`、`emojiType` | `Reaction` | `reactions.add` |
| `reactions/remove` | `conversationId`、`messageId`、`reactionId` | `{}` | `reactions.remove` |

`target` 是下面两种之一：

- 发到会话：`{type: "conversation", conversationId}`
- 回复某条消息：`{type: "reply", conversationId, messageId, inThread, threadRoot?, alsoToConversation}`

规则：

- **同一个 `operationId` 在一个进程的生命周期里只发一次。** 重复调用时返回同一个结果，不能再发一遍。
- 拿不准平台有没有收到时，回 `unknown`，不要自己重发。宿主不会自动重发结果为 `unknown` 的消息，这是 hhh-im 的现有规则。
- 撤回、编辑和表情的时限与权限由平台决定。平台拒绝时，程序把拒绝原样回给宿主，不要重试。
- 超出能力的请求，宿主在本地就先拦下：比如不能撤回别人的消息，只读连接不能发送。

### 10.1 messages/retranslate（宿主→程序，需要 `retranslate` 能力）

程序的翻译规则升级后，用它把本地库里存着原始数据的旧消息按新规则重新翻译一遍，**不联网**。

- 参数：`{ "rule": "<规则名>", "messages": [ { "id", "conversationId", "raw": { "format", "data" } } ] }`。一批不超过 100 条，宿主按 §14 的预算分批。
- 结果：`{ "messages": [ Message… ] }`，每条是重译后的完整消息；程序不想改的那条可以原样返回，宿主按消息编号覆盖本地记录。
- `rule` 由程序自己定名（例如 `wechat-miniapp-card-v2`）。宿主按规则名记“这条已经用这个规则重译过”，同一规则不重复重译；程序换了新规则名才会再被调。这对应 hhh-im 现有的 `content_rereads` 机制，区别是数据来自程序的 `raw` 而不是平台重读。
- 程序声明哪些规则、以及规则适用于哪些消息（靠 `raw.format` 或 `data` 里的特征），由程序在实现文档里说明；宿主只负责按规则名调一次、存结果、记已重译。

## 11. 推送

### 11.1 events/push（程序→宿主请求）

```json
{ "jsonrpc": "2.0", "id": 41, "method": "events/push",
  "params": { "events": [ "Event…" ], "syncState": "<不透明字符串，可选>" } }
```

- 宿主把这一批事件和 `syncState` **在同一个事务里**写进本地库，写完才回复空结果。这个回复就是确认。
- 程序**同一时间只能有一批在途**，收到上一批的确认后才发下一批。这样顺序是确定的，也天然起到背压作用。
- 收到错误回复，或者 30 秒都没有回复时，程序隔一会儿用同样的内容重发。宿主按编号去重。
- 一批最多 500 个事件。

### 11.2 事件种类

| `type` | 字段 | 说明 |
|---|---|---|
| `messages` | `conversationId`、`messages[]` | 新到的或有变化的消息，包括编辑、撤回成标记、话题根消息的变化。同一条消息以最新收到的为准 |
| `deleted` | `conversationId`、`messageId` | 消息从平台上消失了（`deletedMessagesVanish` 为真的平台） |
| `reactions` | `conversationId`、`messageId`、`reactions[]` | 这条消息此刻的全部表情 |
| `reactionAdded` / `reactionRemoved` | 前者 `reaction`；后者 `conversationId`、`messageId`、`reactionId` | 单个表情的增加和取消 |
| `conversation` | `conversation` | 新出现的会话，或者会话资料变了 |
| `conversationRemoved` | `conversationId` | 会话在平台上没有了（比如删了好友、退了群）。宿主只记下这个状态，不删本地记录 |
| `people` | `people[]` | 有人的名字或头像变了 |

### 11.3 连接状态（程序→宿主通知，不需要确认）

`status {state: "online|connecting|offline", reason?}`：程序和平台之间的连接状况，宿主用它显示推送状态。`history=server` 的平台在推送断开时，宿主按 hhh-im 现有规则（`PUSH_FALLBACK_AFTER`）回到定时读取。

### 11.4 两类平台

**`history=server`：消息随时能重读**

- 推送的作用只是更早知道有变化。宿主照 hhh-im 现有规则工作：连上、重连、打开会话时补读一次；推送断了就回到定时读取。
- `syncState` 可以不用。

**`history=none`：平台只推一次**

- 推送是消息的唯一来源。宿主不会定时去读消息，也不会往上翻页补读断档，本地最早的那条就是尽头。
- 程序要把平台的同步位置（例如微信的同步游标）放进每一批的 `syncState`。宿主把它和这一批消息一起存进本地库，下次 `session/open` 时交还给程序，程序从那里接着拉。宿主保证：本地库里保存的 `syncState` 之前的所有事件，都已经写进了本地库。
- 这类平台**所有会话的消息都存进 HHH**，不按同步范围丢弃（HHH 已经不再有“先关注再接收”这一步）。宿主收到某个还没在侧栏出现的会话的推送时，存库并走“发现会话”的路径让它出现在侧栏——和内置平台里会话因为来了新消息而出现是同一套表现。否则这条消息只推一次，丢了就再也拿不到。
- 会话清单靠 `conversation` 事件推送过来，存进本地库，按名字搜索会话在 HHH 本地完成。

### 11.5 log/write（程序→宿主通知，不需要确认）

程序把一条结构化日志交给宿主，宿主并进自己的日志系统（带上程序编号）。

```json
{ "jsonrpc": "2.0", "method": "log/write",
  "params": { "level": "error|warn|info|debug", "message": "…", "fields": { "<键>": "<值>" } } }
```

- `message` 和 `fields` 里**不能含凭据、消息正文、令牌**。宿主按 §14 截断过长的内容。
- 这条通知只是把程序内部的日志透出来，便于排查；程序照常也可以写标准错误输出（§3），两者宿主都收。

## 12. 错误

JSON-RPC 的 `error.code` 按下表取值；`error.data` 里写明错误分类和给用户看的说明：

```json
{ "code": 1003, "message": "authExpired",
  "data": { "kind": "authExpired", "userMessage": "微信登录已失效，请重新扫码。", "platformCode": "-13",
            "reauth": "expired", "retryAfterSecs": null } }
```

| code | kind | 宿主怎么处理（括号里是对应的 hhh-im `ErrorKind`） |
|---|---|---|
| -32700 / -32600 | 无 | 协议错误，记进日志。程序反复这样就重启它 |
| -32601 | `unsupported` | 当作不支持。如果这个方法程序声明过，就算程序的错误 |
| -32602 | `invalidRequest` | 参数不合法 |
| 1001 | `unsupported` | 平台不支持这件事（Unsupported） |
| 1002 | `permissionDenied` | 没有权限，附带说明（PermissionDenied） |
| 1003 | `authExpired` | 停止读取，要求重新登录，附带 `reauth`（AuthExpired） |
| 1004 | `rateLimited` | 所有读取一起等 `retryAfterSecs` 秒（RateLimited） |
| 1005 | `notFound` | 平台明确说没有（NotFound） |
| 1006 | `invalidRequest` | 平台拒绝了参数（InvalidRequest） |
| 1007 | `disconnected` | 暂时失败，比如网络问题或平台临时故障，按退避重试（Disconnected） |
| 1008 | `protocolError` | 平台给了看不懂的数据（ProtocolError） |
| 1009 | `cancelled` | 请求被宿主取消了（Cancelled） |
| 1010 | `limitExceeded` | 超出平台限制，比如文件太大（LimitExceeded） |

`userMessage` 用宿主在 `initialize` 里给的 `locale` 来写，不能包含令牌或平台的原始响应。`platformCode` 原样记成文字。

## 13. 取消、超时与并发

- 宿主不再需要某个请求的结果时（比如已超时，或会话已移出读取范围），发通知 `$/cancel {id}`。程序应尽快停下，回 `cancelled` 或者干脆不回。之后才到的回复，宿主一律忽略。
- 宿主这边的超时：

  | 请求 | 超时 |
  |---|---|
  | 查询 | 10 秒（同 hhh-im 的 `QUERY_TIMEOUT`，超时后换个连接立刻重读一次） |
  | 读资源的一段 | 30 秒 |
  | 发送、上传一段、撤回、编辑、表情 | 60 秒 |
  | `initialize`、`ping` | 10 秒 |
  | `session/open` | 30 秒 |

- 宿主同时最多有 8 个请求在途。程序可以并发处理，也可以排队处理，但**查询不能被耗时长的下载或同步堵住**，否则会超时。

## 14. 宿主强制执行的预算

收到的数据超出下面的预算时，宿主按协议错误处理，不写进本地库：

| 项目 | 上限 |
|---|---|
| 单条 JSON-RPC 消息 | 4 MiB |
| 每批事件 | 500 个 |
| 每页结果 | 请求里给的 `limit` |
| 每条消息的内容块 | 200 个 |
| 格式正文、卡片 | 沿用 hhh-im 现有的节点数、深度预算 |
| 单块文字 | 64 KiB |
| 编号 | 256 字节，不含控制字符 |
| 资源的一段 | 1 MiB，而且不超过请求的 `length` |
| 资源总长 | 不超过 `total`；图片、头像还要不超过 hhh-im 现有的上限 |
| 登录二维码 | 512 KiB |
| 登录步骤的提示 `note.text` | 200 个字（按字符数；超出的提示不显示，这一步照常） |
| logo | 256 KiB |
| 凭据 | 64 KiB |
| 单条消息的 `raw` 原始数据 | 256 KiB |
| 一次 `messages/retranslate` | 100 条 |
| 日志行、`log/write` 的 `message` | 截到 4 KiB |

宿主还会核对这几件事：
- 结果里的会话编号和请求的一致。
- 消息属于请求的那个会话。
- 时间在合理范围内：不早于 2000 年，也不晚于当前时间加 1 天。

## 15. 版本与演进

- `protocolVersion` 是整数主版本号。只有不兼容的改动才升主版本，宿主至少同时支持当前和上一个主版本。
- 同一个主版本里只做加法。新增的方法、事件、字段、内容块和能力项都是可选的，并通过能力声明。
- 双方都忽略不认识的字段。宿主遇到不认识的事件，记日志、跳过，这一批仍然照常确认。程序遇到不认识的方法，回 -32601。
- 每加一项，都在本文末尾的“变更记录”里记一行，写明加了什么。

## 16. 最小可接入集

| 组合 | 程序最少要实现 | 在 HHH 里能做什么 |
|---|---|---|
| 只读，能重读历史 | `initialize`、`session/open`、登录、`conversations/list`、`conversations/get`、`messages/list` | 看会话和消息、加关注、伙伴能读；不能发消息 |
| 只读，平台只推一次 | `initialize`、`session/open`、登录、`conversations/get`、`events/push`（`messages` 和 `conversation` 两类事件） | 同上 |
| 能发文字 | 上面任意一种，再加上 `messages/send`（声明 `send.text`） | 再加上发消息、伙伴回复 |

要显示图片和文件，还要实现 `resources/read`；要显示头像，还要实现 `avatars/read`。

## 17. 自检工具 hhh-im-check

HHH 提供一个命令行工具。第三方开发者用它检查自己的程序，HHH 的验收也用它：

```
hhh-im-check <清单路径> [--mock] [--live --read-only] [--allow-send <会话编号>]
```

每一项检查都给出通过、失败或跳过，失败时写明原因：

1. 清单合法；程序在 10 秒内回复 `initialize`；标准输出里没有协议以外的内容。
2. 平台说明、能力和设置项都合法，能力组合自洽（§5.2）。
3. 每个声明了的方法都能调用；没声明的方法不会被调用。
4. 数据规则：
   - 编号稳定（同一条消息读两次，编号相同）；
   - 每页里从旧到新排；
   - 游标最终会翻到头；
   - 时间是毫秒；
   - 普通消息有发送者；
   - 撤回的消息内容为空。
5. 资源：分段读出的总长一致，`eof` 正确，`sha256` 和实际字节对得上。
6. 推送：
   - 同一批重发不会产生重复；
   - 不确认就会重发；
   - 交还 `syncState` 之后，从断点接着推。
7. 取消有效，`ping` 及时回复，`shutdown` 后 5 秒内退出。
8. 发送：只读程序会拒绝发送；拿不准的发送回 `unknown`，并且不重发。这一项需要 `--allow-send`。
9. 凭据：程序先存后用；宿主拒绝保存时，程序保留旧凭据。

`--mock` 要求程序支持不联网的模拟模式（见实现范式 §9）；`--live --read-only` 用真实账号，只做读取。

## 附录 A：协议与 hhh-im 接口的对应关系

| 协议 | hhh-im |
|---|---|
| `initialize` 里的 `platform` | `Platform` / `PlatformInfo`（声明式规则由外部程序适配器执行） |
| `session/open` 里的 `account` | `ImClient::account` |
| `conversations/list` / `search` / `get` | `ImClient::list_conversations` / `search_conversations` / `get_conversation` |
| `conversations/findDirect` | `ImClient::find_direct_conversation` |
| `conversations/members` | `ImClient::list_members` |
| `people/search` / `people/get` | `ImClient::search_users` / `look_up_people` |
| `messages/list` / `messages/get` | `ImClient::list_messages`（毫秒和秒在适配器里换算）/ `get_message` |
| `reactions/list` / `add` / `remove` | `ImClient::list_reactions` / `add_reaction` / `remove_reaction` |
| `emoji/catalog` / `emoji/image` | `ImClient::emoji_catalog` / `emoji_image` |
| `messages/send` / `lookupSend` | `ImClient::send` / `lookup_send` |
| `messages/recall` / `edit` | `ImClient::recall` / `edit` |
| `resources/read` / `avatars/read` | `ImClient::open_resource` / `open_avatar` |
| `uploads/*` | `ImClient::upload_resource` |
| `events/push` | `ImClient::live` → `LiveFeed`（新增：写进本地库后才确认） |
| `messages/retranslate` | `ImClient::content_rereads` → `ContentReread`（新增：数据来自程序的 `raw`，不联网） |
| `log/write` | 宿主日志系统（新增：程序把结构化日志透给宿主） |
| `login/*`、`credential/save`、`session/expired` | Friend 的授权和凭据保存（`StoreGrantBackend`）；外部程序走一套通用流程 |

协议的线上类型和 hhh-im 的内部类型分开定义、分开冻结，由适配器在两边之间翻译。这样内部模型怎么改，都不会影响已经发布的程序。协议里没有连接编号，`connection_id` 和 `ActorRef.id_type` 由适配器补上。

## 附录 B：卡片

结构同 hhh-im `ImCard` 的 JSON：`{title?, subtitle?, tags[], tone, wide, blocks[]}`。

- 块的种类：`paragraph{parts: [{type: "markdown", text}]}`、`columns{columns}`、`divider`、`rich{children}`、`images{columns, images: [{resource, alt?, width?, height?}]}`、`people{people: [{userId, name}], size, avatar, name}`、`panel{title, expanded, border, blocks}`。
- 色调取值：`neutral`、`grey`、`blue`、`green`、`yellow`、`orange`、`red`、`purple`。
- 这套结构在 v1 固定下来。

## 附录 C：一段完整交互示例

```text
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersions":[1],"host":{"name":"HHH","version":"0.9.0"},"locale":"zh-CN","tempDir":"…","network":{"proxy":null}}}
← {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"provider":{"id":"wechat-pad","name":"微信","version":"0.3.0"},"platform":{"avatarShape":"rounded","history":"none","actions":{"reply":false,"thread":false,"react":false,"recall":null,"edit":null}},"capabilities":{"conversations":{"list":false},"send":{"text":true,"image":true},"resources":{"read":true},"events":{"messages":true,"conversations":true,"people":true}},"settings":[]}}
→ {"jsonrpc":"2.0","id":2,"method":"session/open","params":{"credential":null,"settings":{},"syncState":null}}
← {"jsonrpc":"2.0","id":2,"result":{"state":"needsLogin","reason":"noCredential","message":"请扫码登录微信。"}}
→ {"jsonrpc":"2.0","id":3,"method":"login/start","params":{}}
← {"jsonrpc":"2.0","id":3,"result":{"flowId":"f1"}}
← {"jsonrpc":"2.0","method":"login/step","params":{"flowId":"f1","step":{"kind":"qrCode","image":"iVBOR…","expiresAtMs":1791090000000,"hint":"用手机微信扫码"}}}
← {"jsonrpc":"2.0","method":"login/step","params":{"flowId":"f1","step":{"kind":"confirmOnDevice","message":"请在手机上确认登录"}}}
← {"jsonrpc":"2.0","id":"p1","method":"credential/save","params":{"credential":"…"}}
→ {"jsonrpc":"2.0","id":"p1","result":{}}
← {"jsonrpc":"2.0","method":"login/finished","params":{"flowId":"f1","account":{"id":"wxid_me","name":"小满"}}}
← {"jsonrpc":"2.0","id":"p2","method":"events/push","params":{"events":[{"type":"conversation","conversation":{"id":"123@chatroom","kind":"group","name":"家人群"}},{"type":"messages","conversationId":"123@chatroom","messages":[{"id":"7012…","conversationId":"123@chatroom","sender":{"id":"wxid_mom","kind":"user"},"createdAtMs":1791080000000,"content":[{"type":"text","text":"吃饭了吗"}]}]}],"syncState":"…"}}
→ {"jsonrpc":"2.0","id":"p2","result":{}}
```

（`→` 是宿主发给程序的，`←` 是程序发给宿主的。）

## 变更记录

- 2026-10-04：v1 草案。补入 `log/write`（结构化日志）、消息 `raw` 原始数据与 `messages/retranslate`（可重新翻译）、`history=none` 平台存全部会话并走发现路径、上传即发送的平台说明、信任边界措辞纠正。
