//! `hhh-im-pack --publish`：把这次打的包与版本索引发到清单 `source` 写的仓库（契约 provider-protocol v10 §4.3、§4.9）。
//!
//! - GitHub：仓库的标签 `v<版本>` 上建（或找到）正式 Release，包与 `hhhim-release.json` 作为附件上传（同名的先删再传）。
//! - GitLab：包与索引放进项目的通用包仓库 `packages/generic/<插件编号>/<版本>/<文件名>`，Release 上挂指向它们的链接
//!   （附件上传区的文件令牌下载不了，契约 §4.3 第 4 条）。标签要先推到仓库里。
//! - 几台电脑各打各的目标时，发布前先读回仓库里同一版本的索引、合进这次的包，索引里就有全部目标。
//! - 新版本必须比仓库里最新的正式版本大（同一版本补传别的目标除外）。
//!
//! 走系统自带的 `curl`（Windows 10 起自带），协议包不带 HTTP 与 TLS 依赖。令牌从环境变量读（`HHH_IM_PUBLISH_TOKEN`，
//! 或 GitHub 的 `GITHUB_TOKEN` / GitLab 的 `GITLAB_TOKEN`），经标准输入当请求头交给 curl（`-H @-`），不出现在命令行、
//! 任何文件与输出里。

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use hhh_im_protocol::release::{RELEASE_INDEX_FILE, ReleaseIndex, Source, Version};

/// 令牌的环境变量（先看通用的，再看各站点习惯的）。
const TOKEN_ENV: &str = "HHH_IM_PUBLISH_TOKEN";
/// 测试用：换掉 GitHub API、上传地址与 GitLab 站点（只认本机回环地址）。
const GITHUB_API_ENV: &str = "HHH_IM_PACK_GITHUB_API";
const GITHUB_UPLOADS_ENV: &str = "HHH_IM_PACK_GITHUB_UPLOADS";
const GITLAB_ORIGIN_ENV: &str = "HHH_IM_PACK_GITLAB_ORIGIN";

/// 一次发布：来源、标签、令牌（只在内存）、各站点的地址。
pub(crate) struct Publish {
    source: Source,
    tag: String,
    version: Version,
    token: String,
    ends: Endpoints,
}

impl std::fmt::Debug for Publish {
    // 令牌不进任何输出：调试输出只写来源与标签。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Publish")
            .field("source", &self.source)
            .field("tag", &self.tag)
            .finish_non_exhaustive()
    }
}

/// 各站点的地址：平常是正式站点；测试换成本机假站点（环境变量只认本机回环地址）。
#[derive(Debug, Clone)]
struct Endpoints {
    github_api: String,
    github_uploads: String,
    gitlab_origin: Option<String>,
}

impl Endpoints {
    fn from_env() -> Self {
        Self {
            github_api: loopback(GITHUB_API_ENV).unwrap_or_else(|| "https://api.github.com".into()),
            github_uploads: loopback(GITHUB_UPLOADS_ENV)
                .unwrap_or_else(|| "https://uploads.github.com".into()),
            gitlab_origin: loopback(GITLAB_ORIGIN_ENV),
        }
    }

    fn gitlab(&self, host: &str) -> String {
        self.gitlab_origin
            .clone()
            .unwrap_or_else(|| host.trim_end_matches('/').to_owned())
    }
}

/// curl 的一次回应：状态码、正文、跳转目标。
struct Reply {
    status: u16,
    body: Vec<u8>,
    location: String,
}

impl Reply {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    fn json(&self) -> Result<serde_json::Value, String> {
        serde_json::from_slice(&self.body)
            .map_err(|_| format!("站点回应的格式不对（{}）", self.status))
    }

    /// 站点回了错误：说出状态与站点给的说明（不带请求头，令牌不会出现在这里）。
    fn failure(&self, what: &str) -> String {
        let text = String::from_utf8_lossy(&self.body);
        let detail: String = text.chars().take(300).collect();
        format!("{what}失败（{}）：{detail}", self.status)
    }
}

/// 请求体：没有、JSON 文本、上传一个文件（POST 用 `--data-binary`，PUT 用 `--upload-file`）。
enum Body<'a> {
    None,
    Json(String),
    File(&'a Path),
}

/// 测试入口的地址只收本机回环。
fn loopback(env: &str) -> Option<String> {
    let value = std::env::var(env).ok()?;
    let value = value.trim_end_matches('/').to_owned();
    ["http://127.0.0.1:", "http://localhost:"]
        .iter()
        .any(|prefix| value.starts_with(prefix))
        .then_some(value)
}

/// GitLab 项目路径进 API 地址要整段编码。
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// 组 curl 的参数（不含令牌：令牌在标准输入的请求头里）。单独拿出来，测试据此核对令牌不在命令行上。
fn curl_args(method: &str, url: &str, body: &Body<'_>, extra_headers: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = [
        "-sS",
        "-H",
        "@-",
        "-X",
        method,
        "-w",
        "\n%{http_code} %{redirect_url}",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for header in extra_headers {
        args.push("-H".into());
        args.push((*header).to_owned());
    }
    match body {
        Body::None => {}
        Body::Json(json) => {
            args.extend([
                "-H".into(),
                "Content-Type: application/json".into(),
                "--data-raw".into(),
                json.clone(),
            ]);
        }
        Body::File(path) if method == "PUT" => {
            args.extend(["--upload-file".into(), path.display().to_string()]);
        }
        Body::File(path) => {
            args.extend([
                "-H".into(),
                "Content-Type: application/octet-stream".into(),
                "--data-binary".into(),
                format!("@{}", path.display()),
            ]);
        }
    }
    args.push(url.to_owned());
    args
}

impl Publish {
    /// 读清单里的来源与环境变量里的令牌，准备一次发布。来源不是 GitHub / GitLab、没有令牌时说明原因。
    pub(crate) fn prepare(
        source: Option<&Source>,
        tag: &str,
        version: Version,
    ) -> Result<Self, String> {
        let source = source
            .ok_or("清单没写更新来源（source），不知道发到哪里。")?
            .clone();
        let site_env = match &source {
            Source::Github { .. } => "GITHUB_TOKEN",
            Source::Gitlab { .. } => "GITLAB_TOKEN",
            Source::Url { .. } => {
                return Err("网址来源请自己把 dist 里的包与索引放到索引地址同一目录。".into());
            }
        };
        let token = std::env::var(TOKEN_ENV)
            .or_else(|_| std::env::var(site_env))
            .ok()
            .map(|token| token.trim().to_owned())
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                format!("没有找到发布用的令牌：请设环境变量 {TOKEN_ENV} 或 {site_env}。")
            })?;
        Ok(Self {
            source,
            tag: tag.to_owned(),
            version,
            token,
            ends: Endpoints::from_env(),
        })
    }

    /// 请求头（经标准输入交给 curl）：令牌、身份、接受的格式。
    fn headers(&self) -> String {
        match &self.source {
            Source::Github { .. } => format!(
                "Authorization: Bearer {}\nAccept: application/vnd.github+json\nX-GitHub-Api-Version: 2022-11-28\nUser-Agent: hhh-im-pack\n",
                self.token
            ),
            _ => format!("PRIVATE-TOKEN: {}\nUser-Agent: hhh-im-pack\n", self.token),
        }
    }

    /// 交给 curl 的请求头：调用方另给 `Accept` 时用它的，不再带默认那条。两条 `Accept` 一起发，GitHub 按第一条
    /// 回附件的 JSON 说明而不是文件本身——读仓库里已有的版本索引就会读成说明（第二台电脑补传别的目标时合不上）。
    fn headers_for(&self, extra_headers: &[&str]) -> String {
        let accept = |line: &str| line.to_ascii_lowercase().starts_with("accept:");
        let own_accept = extra_headers.iter().any(|header| accept(header));
        self.headers()
            .lines()
            .filter(|line| !(own_accept && accept(line)))
            .map(|line| format!("{line}\n"))
            .collect()
    }

    fn curl(
        &self,
        method: &str,
        url: &str,
        body: Body<'_>,
        extra_headers: &[&str],
    ) -> Result<Reply, String> {
        run_curl(
            &curl_args(method, url, &body, extra_headers),
            &self.headers_for(extra_headers),
        )
    }

    /// 读回仓库里同一版本已有的索引（几台电脑各打各的目标时合并用）；没有为空。
    pub(crate) fn remote_index(&self, id: &str) -> Result<Option<ReleaseIndex>, String> {
        let raw = match &self.source {
            Source::Github { repo } => {
                let reply = self.curl(
                    "GET",
                    &format!(
                        "{}/repos/{repo}/releases/tags/{}",
                        self.ends.github_api, self.tag
                    ),
                    Body::None,
                    &[],
                )?;
                if reply.status == 404 {
                    return Ok(None);
                }
                if !reply.ok() {
                    return Err(reply.failure("读 Release"));
                }
                let release = reply.json()?;
                let Some(url) = asset_url(&release, RELEASE_INDEX_FILE) else {
                    return Ok(None);
                };
                // 附件地址带令牌问一次，站点 302 到存储服务器；那一跳不带令牌（不经这里的请求头）。
                let reply = self.curl(
                    "GET",
                    &url,
                    Body::None,
                    &["Accept: application/octet-stream"],
                )?;
                if reply.status / 100 == 3 && !reply.location.is_empty() {
                    run_curl(
                        &curl_args("GET", &reply.location, &Body::None, &[]),
                        "User-Agent: hhh-im-pack\n",
                    )?
                    .body
                } else if reply.ok() {
                    reply.body
                } else {
                    return Err(reply.failure("读已有的版本索引"));
                }
            }
            Source::Gitlab { host, project } => {
                let url = format!(
                    "{}/api/v4/projects/{}/packages/generic/{id}/{}/{RELEASE_INDEX_FILE}",
                    self.ends.gitlab(host),
                    encode(project),
                    self.version
                );
                let reply = self.curl("GET", &url, Body::None, &[])?;
                if reply.status == 404 {
                    return Ok(None);
                }
                if !reply.ok() {
                    return Err(reply.failure("读已有的版本索引"));
                }
                reply.body
            }
            Source::Url { .. } => return Ok(None),
        };
        let index: ReleaseIndex = serde_json::from_slice(&raw)
            .map_err(|_| "仓库里已有的版本索引格式不对。".to_owned())?;
        Ok(Some(index).filter(|index| index.id == id && index.version == self.version.to_string()))
    }

    /// 仓库里最新的正式版本（没有为空）：新版本要比它大（同一版本补传别的目标除外）。
    pub(crate) fn latest_version(&self) -> Result<Option<Version>, String> {
        let (url, field) = match &self.source {
            Source::Github { repo } => (
                format!("{}/repos/{repo}/releases/latest", self.ends.github_api),
                "tag_name",
            ),
            Source::Gitlab { host, project } => (
                format!(
                    "{}/api/v4/projects/{}/releases/permalink/latest",
                    self.ends.gitlab(host),
                    encode(project)
                ),
                "tag_name",
            ),
            Source::Url { .. } => return Ok(None),
        };
        let mut reply = self.curl("GET", &url, Body::None, &[])?;
        // GitLab 的“最新”302 到具体版本（同一站点，照带令牌）。
        if reply.status / 100 == 3 && !reply.location.is_empty() {
            reply = self.curl("GET", &reply.location, Body::None, &[])?;
        }
        if reply.status == 404 {
            return Ok(None);
        }
        if !reply.ok() {
            return Err(reply.failure("读最新版本"));
        }
        Ok(reply
            .json()?
            .get(field)
            .and_then(|tag| tag.as_str())
            .and_then(Version::parse))
    }

    /// 发布：包与索引传上去，Release 建好或补上。`files` 是这次要传的文件（包与索引），`notes` 是更新说明。
    pub(crate) fn upload(&self, id: &str, files: &[&Path], notes: &str) -> Result<String, String> {
        match &self.source {
            Source::Github { repo } => self.upload_github(repo, files, notes),
            Source::Gitlab { host, project } => self.upload_gitlab(host, project, id, files, notes),
            Source::Url { .. } => Err("网址来源不支持 --publish。".into()),
        }
    }

    fn upload_github(&self, repo: &str, files: &[&Path], notes: &str) -> Result<String, String> {
        let api = &self.ends.github_api;
        let reply = self.curl(
            "GET",
            &format!("{api}/repos/{repo}/releases/tags/{}", self.tag),
            Body::None,
            &[],
        )?;
        let release = if reply.status == 404 {
            let body = serde_json::json!({ "tag_name": self.tag, "name": self.tag, "body": notes, "draft": false, "prerelease": false });
            let created = self.curl(
                "POST",
                &format!("{api}/repos/{repo}/releases"),
                Body::Json(body.to_string()),
                &[],
            )?;
            if !created.ok() {
                return Err(created.failure("建 Release"));
            }
            created.json()?
        } else if reply.ok() {
            reply.json()?
        } else {
            return Err(reply.failure("读 Release"));
        };
        let id = release
            .get("id")
            .and_then(|id| id.as_u64())
            .ok_or("Release 回应里没有编号。")?;
        for file in files {
            let name = file_name(file)?;
            // 同名的附件先删（补传别的目标时索引要换成合并后的）。
            if let Some(asset) = release
                .get("assets")
                .and_then(|assets| assets.as_array())
                .and_then(|assets| {
                    assets
                        .iter()
                        .find(|asset| asset.get("name").and_then(|n| n.as_str()) == Some(name))
                })
                .and_then(|asset| asset.get("id").and_then(|id| id.as_u64()))
            {
                let removed = self.curl(
                    "DELETE",
                    &format!("{api}/repos/{repo}/releases/assets/{asset}"),
                    Body::None,
                    &[],
                )?;
                if !removed.ok() {
                    return Err(removed.failure(&format!("删旧附件 {name} ")));
                }
            }
            let url = format!(
                "{}/repos/{repo}/releases/{id}/assets?name={}",
                self.ends.github_uploads,
                encode(name)
            );
            let uploaded = self.curl("POST", &url, Body::File(file), &[])?;
            if !uploaded.ok() {
                return Err(uploaded.failure(&format!("上传 {name} ")));
            }
            println!("已上传 {name}");
        }
        Ok(release
            .get("html_url")
            .and_then(|url| url.as_str())
            .unwrap_or_default()
            .to_owned())
    }

    fn upload_gitlab(
        &self,
        host: &str,
        project: &str,
        id: &str,
        files: &[&Path],
        notes: &str,
    ) -> Result<String, String> {
        let api = format!(
            "{}/api/v4/projects/{}",
            self.ends.gitlab(host),
            encode(project)
        );
        let mut links = Vec::new();
        for file in files {
            let name = file_name(file)?;
            let url = format!(
                "{api}/packages/generic/{id}/{}/{}",
                self.version,
                encode(name)
            );
            let uploaded = self.curl("PUT", &url, Body::File(file), &[])?;
            if !uploaded.ok() {
                return Err(uploaded.failure(&format!("上传 {name} ")));
            }
            println!("已上传 {name}");
            // Release 上的链接写站点的正式地址（宿主按它下载），不写测试替换的地址。
            let public = format!(
                "{}/api/v4/projects/{}/packages/generic/{id}/{}/{}",
                host.trim_end_matches('/'),
                encode(project),
                self.version,
                encode(name)
            );
            links.push((name.to_owned(), public));
        }
        let reply = self.curl(
            "GET",
            &format!("{api}/releases/{}", self.tag),
            Body::None,
            &[],
        )?;
        if reply.status == 404 {
            let assets: Vec<serde_json::Value> = links
                .iter()
                .map(|(name, url)| serde_json::json!({ "name": name, "url": url, "link_type": "package" }))
                .collect();
            let body = serde_json::json!({ "tag_name": self.tag, "name": self.tag, "description": notes, "assets": { "links": assets } });
            let created = self.curl(
                "POST",
                &format!("{api}/releases"),
                Body::Json(body.to_string()),
                &[],
            )?;
            if created.status == 422 || created.status == 400 {
                return Err(format!(
                    "建 Release 失败：仓库里可能还没有标签 {}，先 git tag {} 并推上去。（{}）",
                    self.tag, self.tag, created.status
                ));
            }
            if !created.ok() {
                return Err(created.failure("建 Release"));
            }
            return Ok(created
                .json()?
                .pointer("/_links/self")
                .and_then(|url| url.as_str())
                .unwrap_or_default()
                .to_owned());
        }
        if !reply.ok() {
            return Err(reply.failure("读 Release"));
        }
        let release = reply.json()?;
        let existing: Vec<String> = release
            .pointer("/assets/links")
            .and_then(|links| links.as_array())
            .map(|links| {
                links
                    .iter()
                    .filter_map(|link| link.get("name").and_then(|n| n.as_str()).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        for (name, url) in links.iter().filter(|(name, _)| !existing.contains(name)) {
            let body = serde_json::json!({ "name": name, "url": url, "link_type": "package" });
            let added = self.curl(
                "POST",
                &format!("{api}/releases/{}/assets/links", self.tag),
                Body::Json(body.to_string()),
                &[],
            )?;
            if !added.ok() {
                return Err(added.failure(&format!("挂链接 {name} ")));
            }
        }
        Ok(release
            .pointer("/_links/self")
            .and_then(|url| url.as_str())
            .unwrap_or_default()
            .to_owned())
    }

    pub(crate) fn version(&self) -> Version {
        self.version
    }
}

fn file_name(path: &Path) -> Result<&str, String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("文件名读不出：{}", path.display()))
}

/// GitHub Release 里某个附件的 API 地址。
fn asset_url(release: &serde_json::Value, name: &str) -> Option<String> {
    release
        .get("assets")?
        .as_array()?
        .iter()
        .find(|asset| asset.get("name").and_then(|n| n.as_str()) == Some(name))?
        .get("url")?
        .as_str()
        .map(str::to_owned)
}

/// 跑一次 curl：请求头从标准输入给（令牌在里面），读回正文与最后一行的状态码、跳转目标。
fn run_curl(args: &[String], headers: &str) -> Result<Reply, String> {
    let mut child = Command::new("curl")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("找不到 curl（发布要用系统自带的 curl）：{error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(headers.as_bytes())
            .map_err(|error| format!("curl 没收到请求头：{error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("curl 没跑完：{error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("连不上站点：{}", stderr.trim()));
    }
    let mut body = output.stdout;
    let split = body
        .iter()
        .rposition(|b| *b == b'\n')
        .ok_or("curl 的输出读不懂。")?;
    let tail = String::from_utf8_lossy(&body[split + 1..]).to_string();
    body.truncate(split);
    let (status, location) = tail.split_once(' ').unwrap_or((tail.as_str(), ""));
    Ok(Reply {
        status: status
            .trim()
            .parse()
            .map_err(|_| "curl 没给出状态码。".to_owned())?,
        body,
        location: location.trim().to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // 令牌只在标准输入的请求头里：命令行参数里没有它（契约 v10 §4.9“令牌不写进任何文件”）。
    #[test]
    fn the_token_never_reaches_the_command_line() {
        let publish = Publish {
            source: Source::Github { repo: "a/b".into() },
            tag: "v1.0.0".into(),
            version: Version(1, 0, 0),
            token: "ghp_secret_token".into(),
            ends: Endpoints::from_env(),
        };
        let file = Path::new("dist/x.hhhim");
        for body in [Body::None, Body::Json("{}".into()), Body::File(file)] {
            let args = curl_args(
                "POST",
                "https://api.github.com/x",
                &body,
                &["Accept: application/octet-stream"],
            );
            assert!(
                args.iter().all(|arg| !arg.contains("ghp_secret")),
                "{args:?}"
            );
            assert!(
                args.windows(2)
                    .any(|pair| pair[0] == "-H" && pair[1] == "@-"),
                "请求头从标准输入读"
            );
        }
        assert!(publish.headers().contains("Bearer ghp_secret_token"));
        let put = curl_args("PUT", "https://git.example.com/x", &Body::File(file), &[]);
        assert!(put.contains(&"--upload-file".to_owned()));
    }

    fn publisher(source: Source, server: &str) -> Publish {
        Publish {
            source,
            tag: "v0.4.0".into(),
            version: Version(0, 4, 0),
            token: "tok_secret".into(),
            ends: Endpoints {
                github_api: server.into(),
                github_uploads: server.into(),
                gitlab_origin: Some(server.into()),
            },
        }
    }

    fn files(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let package = dir.join("demo-im-0.4.0-windows-x64.hhhim");
        let index = dir.join(RELEASE_INDEX_FILE);
        std::fs::write(&package, b"package").unwrap();
        std::fs::write(&index, b"{}").unwrap();
        (package, index)
    }

    // GitHub：标签上还没有 Release 就建一个，包与索引作为附件上传；令牌经请求头带上（契约 v10 §4.3、§4.9）。
    #[tokio::test(flavor = "multi_thread")]
    async fn publishing_to_github_creates_the_release_and_uploads_both_files() {
        use wiremock::matchers::{header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/someone/hhh-demo/releases/tags/v0.4.0"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path("/repos/someone/hhh-demo/releases")).and(header("authorization", "Bearer tok_secret"))
            .respond_with(ResponseTemplate::new(201).set_body_string(r#"{"id":7,"html_url":"https://github.com/someone/hhh-demo/releases/tag/v0.4.0","assets":[]}"#))
            .expect(1).mount(&server).await;
        for name in ["demo-im-0.4.0-windows-x64.hhhim", "hhhim-release.json"] {
            Mock::given(method("POST"))
                .and(path("/repos/someone/hhh-demo/releases/7/assets"))
                .and(query_param("name", name))
                .respond_with(ResponseTemplate::new(201).set_body_string("{}"))
                .expect(1)
                .mount(&server)
                .await;
        }
        let dir = tempfile::tempdir().unwrap();
        let (package, index) = files(dir.path());
        let publish = publisher(
            Source::Github {
                repo: "someone/hhh-demo".into(),
            },
            &server.uri(),
        );
        let page = tokio::task::spawn_blocking(move || {
            publish.upload("demo-im", &[&package, &index], "- 更快")
        })
        .await
        .unwrap()
        .unwrap();
        assert!(page.ends_with("/releases/tag/v0.4.0"));
    }

    // GitLab：包与索引放进通用包仓库，Release 上挂指向正式站点地址的链接；标签还没推时说清楚。
    #[tokio::test(flavor = "multi_thread")]
    async fn publishing_to_gitlab_uploads_generic_packages_and_links_them() {
        use wiremock::matchers::{body_string_contains, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let api = "/api/v4/projects/team%2Fwechat";
        for name in ["demo-im-0.4.0-windows-x64.hhhim", "hhhim-release.json"] {
            Mock::given(method("PUT"))
                .and(path(format!("{api}/packages/generic/demo-im/0.4.0/{name}")))
                .and(header("private-token", "tok_secret"))
                .respond_with(ResponseTemplate::new(201).set_body_string("{}"))
                .expect(1)
                .mount(&server)
                .await;
        }
        Mock::given(method("GET"))
            .and(path(format!("{api}/releases/v0.4.0")))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path(format!("{api}/releases")))
            .and(body_string_contains("https://git.example.com/api/v4/projects/team%2Fwechat/packages/generic/demo-im/0.4.0/hhhim-release.json"))
            .respond_with(ResponseTemplate::new(201).set_body_string(r#"{"_links":{"self":"https://git.example.com/team/wechat/-/releases/v0.4.0"}}"#))
            .expect(1).mount(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let (package, index) = files(dir.path());
        let publish = publisher(
            Source::Gitlab {
                host: "https://git.example.com".into(),
                project: "team/wechat".into(),
            },
            &server.uri(),
        );
        let page = tokio::task::spawn_blocking(move || {
            publish.upload("demo-im", &[&package, &index], "- 更快")
        })
        .await
        .unwrap()
        .unwrap();
        assert!(page.ends_with("/-/releases/v0.4.0"));
    }

    // 第二台电脑补传别的目标：读回 GitHub 上同一版本已有的索引文件本身。附件地址只带一条
    // `Accept: application/octet-stream`；GitHub 遇到 `application/vnd.github+json` 在前时回的是附件的 JSON 说明。
    #[tokio::test(flavor = "multi_thread")]
    async fn a_second_target_reads_back_the_index_file_not_its_description() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let index = r#"{"indexVersion":1,"id":"demo-im","version":"0.4.0","packages":[{"target":"macos-arm64","file":"demo-im-0.4.0-macos-arm64.hhhim","size":7,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#;
        Mock::given(method("GET"))
            .and(path("/repos/someone/hhh-demo/releases/tags/v0.4.0"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                r#"{{"id":7,"assets":[{{"id":9,"name":"hhhim-release.json","url":"{}/repos/someone/hhh-demo/releases/assets/9"}}]}}"#,
                server.uri()
            )))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/someone/hhh-demo/releases/assets/9"))
            .and(header("accept", "application/vnd.github+json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"id":9,"name":"hhhim-release.json","content_type":"application/json","size":208}"#,
            ))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/someone/hhh-demo/releases/assets/9"))
            .and(header("accept", "application/octet-stream"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/storage/index", server.uri())),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/storage/index"))
            .respond_with(ResponseTemplate::new(200).set_body_string(index))
            .mount(&server)
            .await;
        let publish = publisher(
            Source::Github {
                repo: "someone/hhh-demo".into(),
            },
            &server.uri(),
        );
        let read = tokio::task::spawn_blocking(move || publish.remote_index("demo-im"))
            .await
            .unwrap()
            .unwrap()
            .expect("读回已有的索引");
        assert_eq!(read.packages[0].file, "demo-im-0.4.0-macos-arm64.hhhim");
    }

    // 仓库里最新的正式版本：GitLab 的“最新”302 到具体版本，照带令牌跟过去。
    #[tokio::test(flavor = "multi_thread")]
    async fn the_latest_version_follows_gitlabs_redirect() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/api/v4/projects/team%2Fwechat/releases/permalink/latest",
            ))
            .respond_with(ResponseTemplate::new(302).insert_header(
                "location",
                format!("{}/api/v4/projects/7/releases/v0.3.2", server.uri()),
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7/releases/v0.3.2"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"tag_name":"v0.3.2"}"#))
            .mount(&server)
            .await;
        let publish = publisher(
            Source::Gitlab {
                host: "https://git.example.com".into(),
                project: "team/wechat".into(),
            },
            &server.uri(),
        );
        let latest = tokio::task::spawn_blocking(move || publish.latest_version())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest, Some(Version(0, 3, 2)));
    }

    #[test]
    fn preparing_needs_a_repository_source() {
        assert!(
            Publish::prepare(None, "v1.0.0", Version(1, 0, 0))
                .unwrap_err()
                .contains("source")
        );
        let url = Source::Url {
            index: "https://files.example.com/x/hhhim-release.json".into(),
        };
        assert!(Publish::prepare(Some(&url), "v1.0.0", Version(1, 0, 0)).is_err());
        assert_eq!(encode("team/sub/im"), "team%2Fsub%2Fim");
    }
}
