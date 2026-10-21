//! 插件安装包与版本发布（契约 provider-protocol v10 §4.1–§4.3、v18）：安装包给哪种电脑（[`Target`]）、清单里的更新来源
//! （[`Source`]）、版本号比大小（[`Version`]）、每个版本附带的版本索引（[`ReleaseIndex`]）。
//!
//! 宿主（检查更新、下载、核对）、打包工具与插件仓库模板共用这一份：
//! - 安装包 `<插件编号>-<版本>-<目标>.hhhim` 就是 zip，最上层是清单 [`crate::MANIFEST_FILE`]，一个包只给一个目标；
//! - 发版本 = 在仓库打标签 `v<版本>`、建正式 Release，附上各目标的包与版本索引 [`RELEASE_INDEX_FILE`]；
//! - 宿主只读版本索引：编号、版本、最低宿主版本、更新说明（Markdown）、每个目标的包名、大小与 sha256。
//! - GitLab 另把最新一版的索引原样再放一份到通用包仓库的固定位置（[`LATEST_INDEX_VERSION`]）：只能读包仓库的令牌
//!   （分享码里的部署令牌）查不了 Release，靠它找到最新版本（契约 provider-protocol v23 §6.1）。

use serde::{Deserialize, Serialize};

/// 安装包的扩展名。
pub const PACKAGE_EXTENSION: &str = "hhhim";
/// 每个版本附带的版本索引文件名。
pub const RELEASE_INDEX_FILE: &str = "hhhim-release.json";
/// GitLab 通用包仓库里放“最新版本索引”的版本名：最新一版的 [`RELEASE_INDEX_FILE`] 原样再放一份到
/// `packages/generic/<插件编号>/latest/hhhim-release.json`，每次发布覆盖同一个文件名（站点读同名文件给最新的一份）。
/// 字节与版本目录里的那份相同，官方签名照旧核对版本目录里的 [`RELEASE_SIGNATURE_FILE`]。
///
/// # Examples
///
/// ```
/// use hhh_im_protocol::release::{LATEST_INDEX_VERSION, RELEASE_INDEX_FILE};
/// let path = format!("packages/generic/demo-im/{LATEST_INDEX_VERSION}/{RELEASE_INDEX_FILE}");
/// assert_eq!(path, "packages/generic/demo-im/latest/hhhim-release.json");
/// ```
pub const LATEST_INDEX_VERSION: &str = "latest";
/// 官方插件的版本索引签名（ed25519，索引原文的签名）文件名。
pub const RELEASE_SIGNATURE_FILE: &str = "hhhim-release.json.sig";
/// 版本索引格式的版本。
pub const INDEX_VERSION: u32 = 1;
/// 版本索引最大字节数（宿主读之前先看大小）。
pub const INDEX_MAX_BYTES: usize = 256 * 1024;
/// 更新说明最多几个字（按字符数）。
pub const NOTES_MAX_CHARS: usize = 2000;
/// 一个版本最多几个包（每个目标一个）。
pub const PACKAGES_MAX: usize = 8;
/// 索引里内嵌的标志（data URL）最大字节数：仓库安装的预览用它，不用先下载整个包。
pub const LOGO_DATA_MAX_BYTES: usize = 96 * 1024;

/// 安装包给哪种电脑（系统 + 芯片）。ARM 芯片的 Windows 也运行 x64 的包。
///
/// iPhone / iPad 不能运行下载来的原生程序，它的包是 WASM 组件（[`Target::Wasm32Wasip2`]），由 App 自己解释执行；
/// 桌面各系统照旧只用原生程序的包（契约 provider-protocol v18）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Target {
    /// Windows（x64；ARM 芯片的 Windows 也用它）。
    #[serde(rename = "windows-x64")]
    WindowsX64,
    /// M 芯片的 Mac。
    #[serde(rename = "macos-arm64")]
    MacosArm64,
    /// Intel 芯片的 Mac。
    #[serde(rename = "macos-x64")]
    MacosX64,
    /// x86 的 Linux。
    #[serde(rename = "linux-x64")]
    LinuxX64,
    /// ARM 的 Linux。
    #[serde(rename = "linux-arm64")]
    LinuxArm64,
    /// iPhone / iPad：WASM 组件（`wasm32-wasip2`），App 进程里解释执行。
    #[serde(rename = "wasm32-wasip2")]
    Wasm32Wasip2,
}

impl Target {
    /// 全部目标。
    pub const ALL: [Target; 6] = [
        Target::WindowsX64,
        Target::MacosArm64,
        Target::MacosX64,
        Target::LinuxX64,
        Target::LinuxArm64,
        Target::Wasm32Wasip2,
    ];

    /// 写进包名、清单与索引的名字。
    ///
    /// # Examples
    ///
    /// ```
    /// use hhh_im_protocol::release::Target;
    /// assert_eq!(Target::WindowsX64.as_str(), "windows-x64");
    /// assert_eq!(Target::parse("macos-arm64"), Some(Target::MacosArm64));
    /// assert_eq!(Target::parse("windows-arm64"), None);
    /// ```
    pub fn as_str(self) -> &'static str {
        match self {
            Target::WindowsX64 => "windows-x64",
            Target::MacosArm64 => "macos-arm64",
            Target::MacosX64 => "macos-x64",
            Target::LinuxX64 => "linux-x64",
            Target::LinuxArm64 => "linux-arm64",
            Target::Wasm32Wasip2 => "wasm32-wasip2",
        }
    }

    /// 从名字认出目标；认不出为空。
    pub fn parse(name: &str) -> Option<Target> {
        Target::ALL
            .into_iter()
            .find(|target| target.as_str() == name)
    }

    /// 这个目标的系统（清单 `os` 里的写法）。
    pub fn os(self) -> &'static str {
        match self {
            Target::WindowsX64 => "windows",
            Target::MacosArm64 | Target::MacosX64 => "macos",
            Target::LinuxX64 | Target::LinuxArm64 => "linux",
            Target::Wasm32Wasip2 => "ios",
        }
    }

    /// 给人看的叫法（预览“适用”一行、装错包时的说明）。
    pub fn label(self) -> &'static str {
        match self {
            Target::WindowsX64 => "Windows",
            Target::MacosArm64 => "M 芯片的 Mac",
            Target::MacosX64 => "Intel 芯片的 Mac",
            Target::LinuxX64 => "Linux（x86）",
            Target::LinuxArm64 => "Linux（ARM）",
            Target::Wasm32Wasip2 => "iPhone / iPad",
        }
    }

    /// 这个目标的包装的是不是 WASM 组件（不是原生程序）。
    pub fn is_wasm(self) -> bool {
        matches!(self, Target::Wasm32Wasip2)
    }

    /// 这台电脑该用哪个目标的包；编译到不认识的平台时为空。
    pub fn host() -> Option<Target> {
        Target::for_platform(std::env::consts::OS, std::env::consts::ARCH)
    }

    /// 系统与芯片（`std::env::consts` 的写法）对应的目标。
    ///
    /// # Examples
    ///
    /// ```
    /// use hhh_im_protocol::release::Target;
    /// assert_eq!(Target::for_platform("windows", "aarch64"), Some(Target::WindowsX64));
    /// assert_eq!(Target::for_platform("macos", "aarch64"), Some(Target::MacosArm64));
    /// assert_eq!(Target::for_platform("ios", "aarch64"), Some(Target::Wasm32Wasip2));
    /// assert_eq!(Target::for_platform("freebsd", "x86_64"), None);
    /// ```
    pub fn for_platform(os: &str, arch: &str) -> Option<Target> {
        match (os, arch) {
            // 真机与模拟器都用 WASM 包：与芯片无关。
            ("ios", _) => Some(Target::Wasm32Wasip2),
            ("windows", "x86_64" | "aarch64") => Some(Target::WindowsX64),
            ("macos", "aarch64") => Some(Target::MacosArm64),
            ("macos", "x86_64") => Some(Target::MacosX64),
            ("linux", "x86_64") => Some(Target::LinuxX64),
            ("linux", "aarch64") => Some(Target::LinuxArm64),
            _ => None,
        }
    }
}

/// 安装包的文件名：`<插件编号>-<版本>-<目标>.hhhim`。
///
/// # Examples
///
/// ```
/// use hhh_im_protocol::release::{Target, package_file_name};
/// assert_eq!(package_file_name("wechat-pad", "0.4.0", Target::WindowsX64), "wechat-pad-0.4.0-windows-x64.hhhim");
/// ```
pub fn package_file_name(id: &str, version: &str, target: Target) -> String {
    format!("{id}-{version}-{}.{PACKAGE_EXTENSION}", target.as_str())
}

/// 版本号 `主.次.修订`（只认正式版的三段数字；预发布、构建号不认）。按数字比大小。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    /// 读版本号；不是三段数字（可以带前导 `v`）时为空。
    ///
    /// # Examples
    ///
    /// ```
    /// use hhh_im_protocol::release::Version;
    /// assert!(Version::parse("0.10.0") > Version::parse("0.9.3"));
    /// assert_eq!(Version::parse("v1.2.3"), Some(Version(1, 2, 3)));
    /// assert_eq!(Version::parse("1.2.3-beta"), None);
    /// assert_eq!(Version::parse("01.2.3"), None);
    /// ```
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.strip_prefix('v').unwrap_or(text);
        let mut parts = text.split('.');
        let mut next = || {
            let part = parts.next()?;
            let plain = !part.is_empty()
                && part.len() <= 9
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part == "0" || !part.starts_with('0'));
            plain.then(|| part.parse().ok()).flatten()
        };
        let version = Version(next()?, next()?, next()?);
        parts.next().is_none().then_some(version)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// 插件的更新来源（清单 `source`）：宿主只从这里检查、下载新版本，更新也只接受同一来源。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Source {
    /// GitHub 仓库：`owner/name`。
    Github {
        /// 仓库路径 `owner/name`。
        repo: String,
    },
    /// GitLab（gitlab.com 或自建）：站点地址与项目路径（可以带子组：`group/sub/name`）。
    Gitlab {
        /// 站点地址，只有 `https://<主机>`。
        host: String,
        /// 项目路径。
        project: String,
    },
    /// 一个放版本索引的网址（任何能放文件的地方），包与索引放在同一目录。
    Url {
        /// 版本索引的 https 地址。
        index: String,
    },
}

/// 路径的一段：字母、数字与 `-` `_` `.`，不以点开头，最长 100。
fn path_segment_ok(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 100
        && !segment.starts_with('.')
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// 主机名：小写字母、数字、`-` 与 `.`，至少一个点，最长 253。
fn host_ok(host: &str) -> bool {
    host.len() <= 253
        && host.contains('.')
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// `https://<主机>[/路径]` 拆成主机与路径段；别的协议、带账号密码、带端口以外的写法都不认。
fn split_https(url: &str) -> Option<(String, Vec<&str>)> {
    let rest = url.strip_prefix("https://")?;
    let rest = rest.split(['?', '#']).next()?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.to_ascii_lowercase();
    let bare = host.split_once(':').map(|(name, port)| {
        (
            name.to_owned(),
            !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()),
        )
    });
    let name_ok = match &bare {
        Some((name, port_ok)) => *port_ok && host_ok(name),
        None => host_ok(&host),
    };
    if host.contains('@') || !name_ok {
        return None;
    }
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    Some((host, segments))
}

impl Source {
    /// 来源写法是否合规（仓库路径、站点地址、网址）。
    ///
    /// # Examples
    ///
    /// ```
    /// use hhh_im_protocol::release::Source;
    /// assert!(Source::Github { repo: "someone/hhh-telegram".into() }.check().is_ok());
    /// assert!(Source::Github { repo: "../x".into() }.check().is_err());
    /// assert!(Source::Gitlab { host: "https://git.example.com".into(), project: "team/wechat-plugin".into() }.check().is_ok());
    /// assert!(Source::Url { index: "http://example.com/hhhim-release.json".into() }.check().is_err());
    /// ```
    pub fn check(&self) -> Result<(), String> {
        match self {
            Source::Github { repo } => {
                let parts: Vec<&str> = repo.split('/').collect();
                if parts.len() == 2 && parts.iter().all(|part| path_segment_ok(part)) {
                    Ok(())
                } else {
                    Err(format!("GitHub 仓库写法不对：{repo}（应为 owner/name）"))
                }
            }
            Source::Gitlab { host, project } => {
                let host_fine = split_https(host).is_some_and(|(_, path)| path.is_empty());
                let parts: Vec<&str> = project.split('/').collect();
                let project_fine = (2..=8).contains(&parts.len())
                    && parts.iter().all(|part| path_segment_ok(part));
                if host_fine && project_fine {
                    Ok(())
                } else {
                    Err(format!("GitLab 来源写法不对：{host} {project}"))
                }
            }
            Source::Url { index } => {
                if index.len() <= 2048
                    && split_https(index).is_some_and(|(_, path)| !path.is_empty())
                {
                    Ok(())
                } else {
                    Err(format!("版本索引地址只收 https：{index}"))
                }
            }
        }
    }

    /// 来源所在的站点（主机名，可能带端口）：访问令牌按它存、只发给它。
    pub fn site(&self) -> String {
        match self {
            Source::Github { .. } => "github.com".into(),
            Source::Gitlab { host, .. } => {
                split_https(host).map(|(host, _)| host).unwrap_or_default()
            }
            Source::Url { index } => split_https(index).map(|(host, _)| host).unwrap_or_default(),
        }
    }

    /// 给人看的写法：`github.com/owner/name`、`git.example.com/group/name`、索引网址去掉 `https://`。
    ///
    /// # Examples
    ///
    /// ```
    /// use hhh_im_protocol::release::Source;
    /// let source = Source::Gitlab { host: "https://git.example.com".into(), project: "team/wechat-plugin".into() };
    /// assert_eq!(source.display(), "git.example.com/team/wechat-plugin");
    /// assert_eq!(source.site(), "git.example.com");
    /// ```
    pub fn display(&self) -> String {
        match self {
            Source::Github { repo } => format!("github.com/{repo}"),
            Source::Gitlab { project, .. } => format!("{}/{project}", self.site()),
            Source::Url { index } => index.trim_start_matches("https://").to_owned(),
        }
    }

    /// 仓库路径（`owner/name`、`group/name`）；网址来源为空。
    pub fn repo_path(&self) -> Option<&str> {
        match self {
            Source::Github { repo } => Some(repo),
            Source::Gitlab { project, .. } => Some(project),
            Source::Url { .. } => None,
        }
    }

    /// 从用户填的地址认出来源（「添加 IM → 仓库」）：`https://github.com/<owner>/<name>` 是 GitHub；以 `.json` 结尾的
    /// https 地址是版本索引网址；别的 `https://<主机>/<组>/<项目>` 当 GitLab（gitlab.com 或自建）。仓库页面后面的
    /// `/-/…`、`/tree/…`、`.git` 都去掉。
    ///
    /// # Examples
    ///
    /// ```
    /// use hhh_im_protocol::release::Source;
    /// assert_eq!(
    ///     Source::from_url("https://github.com/someone/hhh-telegram/releases").unwrap(),
    ///     Source::Github { repo: "someone/hhh-telegram".into() },
    /// );
    /// assert_eq!(
    ///     Source::from_url("https://git.example.com/team/wechat-plugin.git").unwrap(),
    ///     Source::Gitlab { host: "https://git.example.com".into(), project: "team/wechat-plugin".into() },
    /// );
    /// assert!(Source::from_url("https://github.com/someone").is_err());
    /// ```
    pub fn from_url(url: &str) -> Result<Source, String> {
        let url = url.trim();
        let Some((host, segments)) = split_https(url) else {
            return Err("请填 https 开头的仓库地址。".into());
        };
        let source = if url
            .split(['?', '#'])
            .next()
            .is_some_and(|path| path.ends_with(".json"))
        {
            Source::Url {
                index: url.to_owned(),
            }
        } else if host == "github.com" {
            let [owner, name, ..] = segments.as_slice() else {
                return Err(
                    "GitHub 仓库地址要写到仓库名：https://github.com/<作者>/<仓库>。".into(),
                );
            };
            Source::Github {
                repo: format!("{owner}/{}", name.trim_end_matches(".git")),
            }
        } else {
            let project: Vec<&str> = segments
                .iter()
                .take_while(|segment| **segment != "-")
                .map(|segment| segment.trim_end_matches(".git"))
                .collect();
            if project.len() < 2 {
                return Err("GitLab 仓库地址要写到项目名：https://<站点>/<组>/<项目>。".into());
            }
            Source::Gitlab {
                host: format!("https://{host}"),
                project: project.join("/"),
            }
        };
        source.check()?;
        Ok(source)
    }
}

/// 版本索引里的一个包。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleasePackage {
    /// 给哪种电脑。
    pub target: Target,
    /// 包的文件名（只是名字，不带路径）。
    pub file: String,
    /// 字节数。
    pub size: u64,
    /// 小写十六进制 sha256。
    pub sha256: String,
}

/// 每个版本附带的版本索引 `hhhim-release.json`：宿主检查更新只读它。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseIndex {
    /// 索引格式版本，当前 [`INDEX_VERSION`]。
    pub index_version: u32,
    /// 插件编号（同清单 `id`）。
    pub id: String,
    /// 给用户看的名字（同清单 `name`）：从仓库安装时预览用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 发布者（同清单 `publisher`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    /// IM 类型（同清单 `imType`）：从仓库安装时预览“同一种只能用一个”用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub im_type: Option<String>,
    /// 标志的 data URL（同清单 `logo` 那个文件），≤ [`LOGO_DATA_MAX_BYTES`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    /// 这个版本（同清单 `version`）。
    pub version: String,
    /// 需要的最低宿主版本。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_host: Option<String>,
    /// 更新说明（Markdown，给用户看）。
    #[serde(default)]
    pub notes: String,
    /// 各目标的包。读的时候跳过这个宿主不认识的目标（以后加的系统），其余照常（契约 provider-protocol v18 D2）：
    /// 旧版宿主读得了带新目标的索引。签名核对的是索引原文，不受影响。
    #[serde(deserialize_with = "known_packages")]
    pub packages: Vec<ReleasePackage>,
}

/// 读 `packages`：目标认得出的按 [`ReleasePackage`] 严格读，认不出的整项跳过（它的字段可能是以后的写法）。
fn known_packages<'de, D>(deserializer: D) -> Result<Vec<ReleasePackage>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Vec::<serde_json::Value>::deserialize(deserializer)?;
    let mut packages = Vec::with_capacity(raw.len());
    for value in raw {
        let target = value.get("target").and_then(serde_json::Value::as_str);
        if target.is_some_and(|name| Target::parse(name).is_none()) {
            tracing::info!(
                package_target = target.unwrap_or_default(),
                "版本索引里有这个 HHH 不认识的目标，跳过这个包"
            );
            continue;
        }
        packages.push(serde_json::from_value(value).map_err(serde::de::Error::custom)?);
    }
    Ok(packages)
}

/// 只是一个文件名：没有路径符号、不以点开头。
fn plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':'])
        && !name.contains("..")
}

impl ReleaseIndex {
    /// 索引是否合规：格式版本、编号、版本号、最低宿主版本、说明长度、标志、每个包（目标不重复、文件名、sha256）。
    ///
    /// # Examples
    ///
    /// ```
    /// use hhh_im_protocol::release::ReleaseIndex;
    /// let index: ReleaseIndex = serde_json::from_str(r#"{"indexVersion":1,"id":"wechat-pad","version":"0.4.0",
    ///   "packages":[{"target":"windows-x64","file":"wechat-pad-0.4.0-windows-x64.hhhim","size":10,
    ///   "sha256":"0000000000000000000000000000000000000000000000000000000000000000"}]}"#).unwrap();
    /// assert!(index.check().is_ok());
    /// ```
    pub fn check(&self) -> Result<(), String> {
        if self.index_version != INDEX_VERSION {
            return Err(format!(
                "版本索引的格式版本 {} 认不出（这个 HHH 认 {INDEX_VERSION}）。",
                self.index_version
            ));
        }
        if !crate::Manifest::id_ok(&self.id) {
            return Err("版本索引里的插件编号不合规。".into());
        }
        if self
            .im_type
            .as_deref()
            .is_some_and(|kind| !crate::Manifest::id_ok(kind))
        {
            return Err("版本索引里的 IM 类型写法不合规。".into());
        }
        if Version::parse(&self.version).is_none() {
            return Err(format!(
                "版本索引里的版本号 {} 不是 主.次.修订。",
                self.version
            ));
        }
        if self
            .min_host
            .as_deref()
            .is_some_and(|min| Version::parse(min).is_none())
        {
            return Err("版本索引里的最低 HHH 版本写法不对。".into());
        }
        if self.notes.chars().count() > NOTES_MAX_CHARS {
            return Err(format!("更新说明超过 {NOTES_MAX_CHARS} 字。"));
        }
        if let Some(logo) = &self.logo
            && (logo.len() > LOGO_DATA_MAX_BYTES || !logo.starts_with("data:image/"))
        {
            return Err("版本索引里的标志不是合规的图片 data URL。".into());
        }
        if self.packages.is_empty() || self.packages.len() > PACKAGES_MAX {
            return Err("版本索引里没有安装包，或包太多。".into());
        }
        for (index, package) in self.packages.iter().enumerate() {
            if self.packages[..index]
                .iter()
                .any(|other| other.target == package.target)
            {
                return Err(format!(
                    "版本索引里 {} 的包重复了。",
                    package.target.as_str()
                ));
            }
            if !plain_file_name(&package.file)
                || !package.file.ends_with(&format!(".{PACKAGE_EXTENSION}"))
            {
                return Err(format!("版本索引里的包名不对：{}", package.file));
            }
            if package.sha256.len() != 64
                || !package
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(format!("版本索引里 {} 的 sha256 写法不对。", package.file));
            }
        }
        Ok(())
    }

    /// 给这个目标的包；没有为空。
    pub fn package_for(&self, target: Target) -> Option<&ReleasePackage> {
        self.packages
            .iter()
            .find(|package| package.target == target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> ReleaseIndex {
        ReleaseIndex {
            index_version: 1,
            id: "wechat-pad".into(),
            name: Some("微信".into()),
            publisher: Some("someone".into()),
            im_type: None,
            logo: None,
            version: "0.4.0".into(),
            min_host: Some("0.1.0".into()),
            notes: "### 修复\n- 扫码".into(),
            packages: vec![ReleasePackage {
                target: Target::WindowsX64,
                file: "wechat-pad-0.4.0-windows-x64.hhhim".into(),
                size: 12,
                sha256: "a".repeat(64),
            }],
        }
    }

    #[test]
    fn versions_compare_by_number_and_refuse_anything_but_three_plain_parts() {
        assert!(Version::parse("1.10.0") > Version::parse("1.9.9"));
        assert!(Version::parse("2.0.0") > Version::parse("1.99.99"));
        assert_eq!(Version::parse("0.0.0"), Some(Version(0, 0, 0)));
        for bad in [
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "",
            "1..3",
            "1.2.3+build",
            "-1.2.3",
            "1.2.03",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert_eq!(Version(1, 2, 3).to_string(), "1.2.3");
    }

    #[test]
    fn every_target_round_trips_and_windows_arm_uses_the_x64_package() {
        for target in Target::ALL {
            assert_eq!(Target::parse(target.as_str()), Some(target));
            let json = serde_json::to_string(&target).unwrap();
            assert_eq!(serde_json::from_str::<Target>(&json).unwrap(), target);
        }
        assert_eq!(
            Target::for_platform("windows", "aarch64"),
            Some(Target::WindowsX64)
        );
        assert_eq!(
            Target::for_platform("linux", "aarch64"),
            Some(Target::LinuxArm64)
        );
        assert_eq!(Target::for_platform("windows", "x86"), None);
        assert!(Target::host().is_some(), "开发与测试的电脑都该认得出");
    }

    /// iPhone / iPad（真机与模拟器）用 WASM 包：系统写 ios、给人看叫 iPhone / iPad；桌面各系统都不是它。
    #[test]
    fn iphones_use_the_wasm_package_and_desktops_never_do() {
        assert_eq!(
            Target::for_platform("ios", "aarch64"),
            Some(Target::Wasm32Wasip2)
        );
        assert_eq!(
            Target::for_platform("ios", "x86_64"),
            Some(Target::Wasm32Wasip2)
        );
        let wasm = Target::Wasm32Wasip2;
        assert_eq!(
            (wasm.as_str(), wasm.os(), wasm.label()),
            ("wasm32-wasip2", "ios", "iPhone / iPad")
        );
        assert!(wasm.is_wasm());
        for target in Target::ALL {
            assert_eq!(target.is_wasm(), target.os() == "ios");
        }
        assert_ne!(
            Target::host(),
            Some(Target::Wasm32Wasip2),
            "桌面不选 WASM 包"
        );
    }

    /// 版本索引里有不认识的目标（以后加的系统，字段也可能是新写法）：那一项跳过，认得出的照常读；
    /// 认得出的目标写错了照旧整份报错。
    #[test]
    fn unknown_targets_in_a_release_index_are_skipped() {
        let sha = "a".repeat(64);
        let text = format!(
            r#"{{"indexVersion":1,"id":"demo-im","version":"0.2.0","packages":[
              {{"target":"plan9-risc","file":"demo-im-0.2.0-plan9-risc.hhhim","size":1,"sha256":"{sha}","future":true}},
              {{"target":"macos-arm64","file":"demo-im-0.2.0-macos-arm64.hhhim","size":2,"sha256":"{sha}"}},
              {{"target":"wasm32-wasip2","file":"demo-im-0.2.0-wasm32-wasip2.hhhim","size":3,"sha256":"{sha}"}}]}}"#
        );
        let index: ReleaseIndex = serde_json::from_str(&text).unwrap();
        assert!(index.check().is_ok());
        assert_eq!(
            index
                .packages
                .iter()
                .map(|package| package.target)
                .collect::<Vec<_>>(),
            [Target::MacosArm64, Target::Wasm32Wasip2]
        );
        assert_eq!(index.package_for(Target::Wasm32Wasip2).unwrap().size, 3);
        let broken = text.replace(r#""size":2,"#, r#""size":2,"extra":1,"#);
        assert!(serde_json::from_str::<ReleaseIndex>(&broken).is_err());
        let only_unknown = format!(
            r#"{{"indexVersion":1,"id":"demo-im","version":"0.2.0","packages":[
              {{"target":"plan9-risc","file":"x.hhhim","size":1,"sha256":"{sha}"}}]}}"#
        );
        let index: ReleaseIndex = serde_json::from_str(&only_unknown).unwrap();
        assert!(index.check().unwrap_err().contains("没有安装包"));
    }

    #[test]
    fn sources_from_repository_addresses() {
        let cases = [
            (
                "https://github.com/someone/hhh-telegram",
                Source::Github {
                    repo: "someone/hhh-telegram".into(),
                },
            ),
            (
                "https://github.com/someone/hhh-telegram.git",
                Source::Github {
                    repo: "someone/hhh-telegram".into(),
                },
            ),
            (
                "https://GitHub.com/someone/hhh-telegram/tree/main",
                Source::Github {
                    repo: "someone/hhh-telegram".into(),
                },
            ),
            (
                "https://gitlab.com/g/sub/proj/-/releases",
                Source::Gitlab {
                    host: "https://gitlab.com".into(),
                    project: "g/sub/proj".into(),
                },
            ),
            (
                "https://git.example.com:8443/team/im",
                Source::Gitlab {
                    host: "https://git.example.com:8443".into(),
                    project: "team/im".into(),
                },
            ),
            (
                "https://files.example.com/im/hhhim-release.json",
                Source::Url {
                    index: "https://files.example.com/im/hhhim-release.json".into(),
                },
            ),
        ];
        for (url, want) in cases {
            assert_eq!(Source::from_url(url).unwrap(), want, "{url}");
        }
        for bad in [
            "http://github.com/a/b",
            "https://github.com/a",
            "https://user:pw@gitlab.com/a/b",
            "git@gitlab.com:a/b.git",
            "https://gitlab.com/a",
            "https://localhost/a/b",
        ] {
            assert!(Source::from_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn source_sites_and_display_names() {
        let lab = Source::Gitlab {
            host: "https://git.example.com:8443".into(),
            project: "team/im".into(),
        };
        assert_eq!(lab.site(), "git.example.com:8443");
        assert_eq!(lab.display(), "git.example.com:8443/team/im");
        assert_eq!(Source::Github { repo: "a/b".into() }.site(), "github.com");
        let url = Source::Url {
            index: "https://files.example.com/x/hhhim-release.json".into(),
        };
        assert_eq!(
            (url.site(), url.display(), url.repo_path()),
            (
                "files.example.com".into(),
                "files.example.com/x/hhhim-release.json".into(),
                None
            )
        );
        let json = serde_json::to_string(&lab).unwrap();
        assert_eq!(
            json,
            r#"{"type":"gitlab","host":"https://git.example.com:8443","project":"team/im"}"#
        );
        assert!(
            serde_json::from_str::<Source>(r#"{"type":"github","repo":"a/b","extra":1}"#).is_err()
        );
    }

    #[test]
    fn release_indexes_are_checked_field_by_field() {
        assert!(index().check().is_ok());
        let mut bad = index();
        bad.index_version = 2;
        assert!(bad.check().unwrap_err().contains("格式版本"));
        let mut bad = index();
        bad.version = "0.4".into();
        assert!(bad.check().is_err());
        let mut bad = index();
        bad.notes = "字".repeat(NOTES_MAX_CHARS + 1);
        assert!(bad.check().unwrap_err().contains("更新说明"));
        let mut bad = index();
        bad.packages[0].file = "../evil.hhhim".into();
        assert!(bad.check().is_err());
        let mut bad = index();
        bad.packages[0].sha256 = "A".repeat(64);
        assert!(bad.check().is_err());
        let mut bad = index();
        bad.packages.push(bad.packages[0].clone());
        assert!(bad.check().unwrap_err().contains("重复"));
        let mut bad = index();
        bad.logo = Some("javascript:alert(1)".into());
        assert!(bad.check().is_err());
        assert!(
            serde_json::from_str::<ReleaseIndex>(
                r#"{"indexVersion":1,"id":"a-b","version":"1.0.0","packages":[],"surprise":1}"#
            )
            .is_err()
        );
        assert_eq!(index().package_for(Target::WindowsX64).unwrap().size, 12);
        assert!(index().package_for(Target::MacosArm64).is_none());
    }
}
