//! 打包工具 `hhh-im-pack`（契约 provider-protocol v10 §4.3、§4.9）：把插件文件夹打成给某个目标的 `.hhhim`，并写好
//! 这个版本的版本索引 `hhhim-release.json`（同一版本打几个目标，索引里就有几个包）。
//!
//! 用法：
//! ```text
//! hhh-im-pack <插件文件夹> [--target <目标>] [--out <输出文件夹>] [--notes <说明.md>] [--tag <标签>] [--after <上一个版本>] [--sdk <版本>] [--sign-key <私钥文件>] [--publish [--latest-only]]
//! hhh-im-pack keygen <放钥匙的文件夹>
//! ```
//! 包里清单与版本索引的 `minHost` 自动写成插件需要的最低 IM SDK（插件锁定的 `hhh-im-protocol` 的“主.次.0”）：从插件
//! 文件夹往上找第一个 `Cargo.lock` 读出来；清单原来写得更高就照旧。宿主的 IM SDK 低于它就写“需要先更新 HHH”。
//! - `--target`：`windows-x64`、`macos-arm64`、`macos-x64`、`linux-x64`、`linux-arm64`，iPhone / iPad 用 `wasm32-wasip2`；
//!   不写用这台电脑的目标。
//! - `--out`：包与索引放哪，默认插件文件夹下的 `dist/`。
//! - `--notes`：更新说明（Markdown 文件）；不写时沿用输出文件夹里同一版本索引已有的说明。
//! - `--tag`：发布用的标签，必须是 `v<清单版本>`，不一致就不打。
//! - `--after`：上一个发布的版本；新版本必须比它大。
//! - `--sdk`：插件用的 IM SDK 版本（`主.次.修订`）；不是 Rust 写的插件、找不到 `Cargo.lock` 时用它写明。
//! - `--publish`：打好后发到清单 `source` 写的 GitHub / GitLab 仓库（见 [`publish`]）：标签默认 `v<清单版本>`，新版本要比
//!   仓库里最新的正式版本大，仓库里同一版本已有的索引先合进来；令牌只从环境变量读（`HHH_IM_PUBLISH_TOKEN`，或
//!   `GITHUB_TOKEN` / `GITLAB_TOKEN`）。
//!   GitLab 发布时另把索引原样放到通用包仓库的 `<插件编号>/latest/`（契约 v23 §6.1）。
//! - `--latest-only`（配合 `--publish`）：不打包、不传包，只给仓库里已发布的清单版本补上 `latest/` 那份索引；用旧版打包工具
//!   发布过的 GitLab 插件，第一次分享前补一次（版本不用变）。
//! - `--sign-key`：用这个私钥给写好的版本索引签名，写出 `hhhim-release.json.sig`，发布时一起上传（官方插件要，契约 v14 §2.5）。
//! - `keygen`：生成一对签名钥匙（见 [`sign`]），已有同名文件时不覆盖。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[path = "hhh_im_pack/publish.rs"]
mod publish;
#[path = "hhh_im_pack/sign.rs"]
mod sign;

use hhh_im_protocol::package;
use hhh_im_protocol::release::{
    INDEX_VERSION, LOGO_DATA_MAX_BYTES, NOTES_MAX_CHARS, RELEASE_INDEX_FILE,
    RELEASE_SIGNATURE_FILE, ReleaseIndex, ReleasePackage, Target, Version,
};

struct Args {
    dir: PathBuf,
    target: Target,
    out: PathBuf,
    notes: Option<PathBuf>,
    tag: Option<String>,
    after: Option<String>,
    sdk: Option<Version>,
    sign_key: Option<PathBuf>,
    publish: bool,
    latest_only: bool,
}

fn usage() -> String {
    "用法：hhh-im-pack <插件文件夹> [--target <目标>] [--out <输出文件夹>] [--notes <说明.md>] [--tag <标签>] [--after <上一个版本>] [--sdk <版本>] [--sign-key <私钥文件>] [--publish [--latest-only]]\n      hhh-im-pack keygen <放钥匙的文件夹>".into()
}

/// 插件锁定的 IM SDK 在 `Cargo.lock` 里的包名。
const SDK_PACKAGE: &str = "hhh-im-protocol";

/// 插件编进去的 IM SDK 版本：从插件文件夹往上找第一个 `Cargo.lock`（cargo 编插件用的就是它），读里面
/// `hhh-im-protocol` 的版本。返回版本与锁文件路径；找不到锁文件、锁文件里没有这个包时给原因。
fn locked_sdk(dir: &Path) -> Result<(Version, PathBuf), String> {
    let start = std::fs::canonicalize(dir)
        .map_err(|error| format!("插件文件夹 {} 读不了：{error}", dir.display()))?;
    // 往上找到仓库根（有 .git 的那一层）为止：插件仓库的锁文件在根上；不越过它读到外面别的项目的锁文件。
    let lock = start
        .ancestors()
        .scan(false, |past_root, folder| {
            if *past_root {
                return None;
            }
            *past_root = folder.join(".git").exists();
            Some(folder.join("Cargo.lock"))
        })
        .find(|path| path.is_file())
        .ok_or_else(|| {
            format!(
                "在 {} 和它上面的文件夹里（到仓库根为止）找不到 Cargo.lock，读不出插件用的 IM SDK 版本：在插件仓库里打包，或用 --sdk <版本> 写明。",
                start.display()
            )
        })?;
    let text = std::fs::read_to_string(&lock)
        .map_err(|error| format!("{} 读不了：{error}", lock.display()))?;
    match sdk_in_lock(&text) {
        Some(version) => Ok((version, lock)),
        None => Err(format!(
            "{} 里没有 {SDK_PACKAGE}，读不出插件用的 IM SDK 版本：用 --sdk <版本> 写明。",
            lock.display()
        )),
    }
}

/// 从 `Cargo.lock` 正文里读 `hhh-im-protocol` 的版本；锁了几个版本时取最高的。
fn sdk_in_lock(text: &str) -> Option<Version> {
    // Cargo.lock 一个包一段：`[[package]]` 开头，下面是 `name = "…"`、`version = "…"`；文件开头的 `version = 4`
    // 是锁文件格式的版本，那时还没进任何一段，不算。
    let mut best = None;
    let mut name: Option<&str> = None;
    for line in text.lines().map(str::trim) {
        if line == "[[package]]" {
            name = None;
        } else if let Some(value) = line.strip_prefix("name = ") {
            name = Some(value.trim_matches('"'));
        } else if let Some(value) = line.strip_prefix("version = ")
            && name == Some(SDK_PACKAGE)
            && let Some(version) = Version::parse(value.trim_matches('"'))
        {
            best = best.max(Some(version));
        }
    }
    best
}

fn parse_args(raw: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut dir = None;
    let mut target = None;
    let mut out = None;
    let mut notes = None;
    let mut tag = None;
    let mut after = None;
    let mut sdk = None;
    let mut sign_key = None;
    let mut publish = false;
    let mut latest_only = false;
    let mut raw = raw.peekable();
    while let Some(arg) = raw.next() {
        let mut value = |name: &str| {
            raw.next()
                .ok_or_else(|| format!("{name} 后面要跟一个值。\n{}", usage()))
        };
        match arg.as_str() {
            "--target" => {
                let name = value("--target")?;
                target = Some(Target::parse(&name).ok_or_else(|| format!("认不出目标 {name}。"))?);
            }
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            "--notes" => notes = Some(PathBuf::from(value("--notes")?)),
            "--tag" => tag = Some(value("--tag")?),
            "--after" => after = Some(value("--after")?),
            "--sdk" => {
                let text = value("--sdk")?;
                sdk = Some(
                    Version::parse(&text)
                        .ok_or_else(|| format!("--sdk 要写 主.次.修订（现在是 {text}）。"))?,
                );
            }
            "--sign-key" => sign_key = Some(PathBuf::from(value("--sign-key")?)),
            "--publish" => publish = true,
            "--latest-only" => latest_only = true,
            "-h" | "--help" => return Err(usage()),
            other if other.starts_with("--") => {
                return Err(format!("不认识的参数 {other}。\n{}", usage()));
            }
            other => {
                if dir.replace(PathBuf::from(other)).is_some() {
                    return Err(format!("只能给一个插件文件夹。\n{}", usage()));
                }
            }
        }
    }
    let dir = dir.ok_or_else(usage)?;
    if latest_only && !publish {
        return Err(format!(
            "--latest-only 要和 --publish 一起用（补的是仓库里的索引）。\n{}",
            usage()
        ));
    }
    let target = match target {
        Some(target) => target,
        None => Target::host().ok_or("认不出这台电脑的目标，请用 --target 指定。")?,
    };
    let out = out.unwrap_or_else(|| dir.join("dist"));
    Ok(Args {
        dir,
        target,
        out,
        notes,
        tag,
        after,
        sdk,
        sign_key,
        publish,
        latest_only,
    })
}

/// `--publish --latest-only`：读清单的编号、版本与来源，给仓库里已发布的这个版本补上“最新版本索引”，不打包。
fn latest_only(args: &Args) -> Result<String, String> {
    let path = args.dir.join(hhh_im_protocol::MANIFEST_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("读不到清单 {}：{error}", path.display()))?;
    let manifest: hhh_im_protocol::Manifest =
        serde_json::from_str(&text).map_err(|error| format!("清单格式不对：{error}"))?;
    let version = Version::parse(&manifest.version).ok_or("清单版本号不是 主.次.修订。")?;
    let tag = args.tag.clone().unwrap_or_else(|| format!("v{version}"));
    let publisher = publish::Publish::prepare(manifest.source.as_ref(), &tag, version)?;
    publisher.publish_latest_only(&manifest.id, &args.out)
}

/// 标志文件转成 data URL（版本索引内嵌，从仓库安装时预览用）；太大或类型不认就不放。
fn logo_data_url(dir: &Path, logo: &str) -> Option<String> {
    let mime = match Path::new(logo)
        .extension()?
        .to_str()?
        .to_ascii_lowercase()
        .as_str()
    {
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => return None,
    };
    let bytes = std::fs::read(dir.join(logo)).ok()?;
    let url = format!("data:{mime};base64,{}", base64(&bytes));
    (url.len() <= LOGO_DATA_MAX_BYTES).then_some(url)
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= chunk.len() {
                TABLE[(n >> shift) as usize & 63] as char
            } else {
                '='
            });
        }
    }
    out
}

fn run(args: Args) -> Result<String, String> {
    if args.latest_only {
        return latest_only(&args);
    }
    // 私钥先读：读不了就不打包、不发布（别发出一个没签名的官方版本）。
    let signing = args.sign_key.as_deref().map(sign::load).transpose()?;
    // 插件用的 IM SDK 先读：读不出就不打包（发出去的版本一定写着宿主要多新的 IM SDK）。
    let (sdk, sdk_from) = match args.sdk {
        Some(sdk) => (sdk, "--sdk".to_owned()),
        None => {
            let (sdk, lock) = locked_sdk(&args.dir)?;
            (sdk, lock.display().to_string())
        }
    };
    let options = package::PackOptions {
        min_host: Some(sdk.sdk_floor()),
    };
    let packed =
        package::pack_with(&args.dir, args.target, &args.out, &options).map_err(|error| error.0)?;
    let manifest = &packed.manifest;
    let version = Version::parse(&manifest.version).ok_or("清单版本号不是 主.次.修订。")?;
    if let Some(tag) = &args.tag
        && tag != &format!("v{}", manifest.version)
    {
        let _ = std::fs::remove_file(&packed.path);
        return Err(format!(
            "标签 {tag} 和清单版本 {} 对不上（应为 v{}）。",
            manifest.version, manifest.version
        ));
    }
    if let Some(after) = &args.after {
        let previous =
            Version::parse(after).ok_or_else(|| format!("上一个版本 {after} 不是 主.次.修订。"))?;
        if version <= previous {
            let _ = std::fs::remove_file(&packed.path);
            return Err(format!("新版本 {version} 要比上一个版本 {previous} 大。"));
        }
    }
    // 发布：先问仓库最新的正式版本（新版本要比它大，同一版本补传别的目标除外），再读回仓库里同一版本的索引合并。
    let publisher = if args.publish {
        let tag = args.tag.clone().unwrap_or_else(|| format!("v{version}"));
        let publisher = publish::Publish::prepare(manifest.source.as_ref(), &tag, version)
            .inspect_err(|_| {
                let _ = std::fs::remove_file(&packed.path);
            })?;
        if let Some(latest) = publisher.latest_version()?
            && latest > publisher.version()
        {
            let _ = std::fs::remove_file(&packed.path);
            return Err(format!("新版本 {version} 要比仓库里最新的 {latest} 大。"));
        }
        Some(publisher)
    } else {
        None
    };
    let index_path = args.out.join(RELEASE_INDEX_FILE);
    // 同一版本的索引已经在：把这个目标的包加进去（换掉同目标的旧条目）；不是同一版本就重新写。发布时仓库里同一版本的索引
    // 也合进来（几台电脑各打各的目标）。
    let local: Option<ReleaseIndex> = std::fs::read_to_string(&index_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .filter(|index: &ReleaseIndex| {
            index.id == manifest.id && index.version == manifest.version
        });
    let remote = match &publisher {
        Some(publisher) => publisher.remote_index(&manifest.id)?,
        None => None,
    };
    let existing = match (local, remote) {
        (Some(mut local), Some(remote)) => {
            for package in remote.packages {
                if !local
                    .packages
                    .iter()
                    .any(|known| known.target == package.target)
                {
                    local.packages.push(package);
                }
            }
            Some(local)
        }
        (local, remote) => local.or(remote),
    };
    let notes = match &args.notes {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|error| format!("更新说明 {} 读不了：{error}", path.display()))?,
        None => existing
            .as_ref()
            .map(|index| index.notes.clone())
            .unwrap_or_default(),
    };
    if notes.chars().count() > NOTES_MAX_CHARS {
        return Err(format!("更新说明超过 {NOTES_MAX_CHARS} 字。"));
    }
    // 同一版本几台电脑各打各的目标：索引里的 minHost 取已有的与这次的较高的，哪个目标的包都不会少要求。
    let min_host = [
        existing
            .as_ref()
            .and_then(|index| index.min_host.as_deref()),
        manifest.min_host.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter_map(Version::parse)
    .max();
    let mut packages: Vec<ReleasePackage> =
        existing.map(|index| index.packages).unwrap_or_default();
    packages.retain(|package| package.target != args.target);
    packages.push(ReleasePackage {
        target: args.target,
        file: packed.file.clone(),
        size: packed.size,
        sha256: packed.sha256.clone(),
    });
    packages.sort_by_key(|package| package.target.as_str());
    let index = ReleaseIndex {
        index_version: INDEX_VERSION,
        id: manifest.id.clone(),
        name: Some(manifest.name.clone()),
        publisher: manifest.publisher.clone(),
        im_type: manifest.im_type.clone(),
        logo: manifest
            .logo
            .as_deref()
            .and_then(|logo| logo_data_url(&args.dir, logo)),
        version: manifest.version.clone(),
        min_host: min_host.map(|version| version.to_string()),
        notes,
        packages,
    };
    index.check()?;
    let json =
        serde_json::to_string_pretty(&index).map_err(|error| format!("版本索引写不出：{error}"))?;
    let json = json + "\n";
    std::fs::write(&index_path, &json)
        .map_err(|error| format!("版本索引写不进 {}：{error}", index_path.display()))?;
    // 签名：签刚写下的这份字节（契约 v14 §2.5）；没给私钥时把旧的签名文件删掉，免得它对不上新索引。
    let signature_path = args.out.join(RELEASE_SIGNATURE_FILE);
    match &signing {
        Some(key) => std::fs::write(&signature_path, sign::sign(key, json.as_bytes()) + "\n")
            .map_err(|error| format!("签名写不进 {}：{error}", signature_path.display()))?,
        None => {
            let _ = std::fs::remove_file(&signature_path);
        }
    }
    let mut summary = format!(
        "已打包 {}（{} 字节，sha256 {}）\n版本索引 {}：{} {}，{} 个包",
        packed.path.display(),
        packed.size,
        packed.sha256,
        index_path.display(),
        index.id,
        index.version,
        index.packages.len()
    );
    summary.push_str(&format!(
        "\n需要的 IM SDK：{} 或更新（插件用 {sdk}，读自 {sdk_from}）",
        index.min_host.as_deref().unwrap_or("-")
    ));
    if signing.is_some() {
        summary.push_str(&format!("\n已签名 {}", signature_path.display()));
    }
    if let Some(publisher) = publisher {
        let mut files = vec![packed.path.as_path(), index_path.as_path()];
        if signing.is_some() {
            files.push(signature_path.as_path());
        }
        let page = publisher.upload(&index.id, &files, &index.notes)?;
        summary.push_str(&format!("\n已发布到 {page}"));
    }
    Ok(summary)
}

/// `keygen <文件夹>`：生成钥匙，告诉用户公钥与私钥放哪。
fn keygen(dir: Option<String>) -> Result<String, String> {
    let dir = dir.ok_or_else(usage)?;
    let (private, public, public_hex) = sign::keygen(Path::new(&dir))?;
    Ok(format!(
        "已生成签名钥匙：\n私钥 {}（只放私有密钥仓库，不进任何公开仓库、聊天与日志）\n公钥 {}：{public_hex}",
        private.display(),
        public.display()
    ))
}

fn main() -> ExitCode {
    let mut raw = std::env::args().skip(1).peekable();
    let result = if raw.peek().map(String::as_str) == Some("keygen") {
        raw.next();
        keygen(raw.next())
    } else {
        parse_args(raw).and_then(run)
    };
    match result {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(reason) => {
            eprintln!("hhh-im-pack：{reason}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn plugin(name: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("hhh-im-pack-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(hhh_im_protocol::MANIFEST_FILE), r#"{"manifestVersion":1,"id":"demo-im","name":"演示","version":"0.3.0","command":"demo","os":["windows","linux"],"logo":"logo.svg","publisher":"me","minHost":"0.1.0"}"#).unwrap();
        std::fs::write(dir.join("demo"), b"program").unwrap();
        std::fs::write(
            dir.join("logo.svg"),
            b"<svg xmlns='http://www.w3.org/2000/svg'/>",
        )
        .unwrap();
        std::fs::write(dir.join("Cargo.lock"), LOCK).unwrap();
        Dir(dir)
    }

    /// 插件仓库的锁文件：锁了 IM SDK 0.9.3（从公开仓库按标签取），另有别的包。
    const LOCK: &str = "# This file is automatically @generated by Cargo.\nversion = 4\n\n[[package]]\nname = \"demo-im\"\nversion = \"0.3.0\"\n\n[[package]]\nname = \"hhh-im-protocol\"\nversion = \"0.9.3\"\nsource = \"git+https://github.com/MagicLizi/hhh-im-sdk?tag=v0.9.3#abc\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.228\"\n";

    fn index_of(dir: &Path) -> ReleaseIndex {
        serde_json::from_str(
            &std::fs::read_to_string(dir.join("dist").join(RELEASE_INDEX_FILE)).unwrap(),
        )
        .unwrap()
    }

    fn packed_manifest(dir: &Path, file: &str) -> hhh_im_protocol::Manifest {
        package::read_manifest(std::fs::File::open(dir.join("dist").join(file)).unwrap()).unwrap()
    }

    // 契约 friend/im-update-hint v1 §兼容闸门：minHost 自动写成插件锁定的 IM SDK 的“主.次.0”，包里清单与版本索引一样；
    // 作者另写得更高的照旧。
    #[test]
    fn the_locked_sdk_becomes_the_min_host_of_package_and_index() {
        let dir = plugin("sdk");
        let summary = run(args(&dir.0, &["--target", "windows-x64"])).unwrap();
        assert!(
            summary.contains("需要的 IM SDK：0.9.0 或更新（插件用 0.9.3"),
            "{summary}"
        );
        assert_eq!(index_of(&dir.0).min_host.as_deref(), Some("0.9.0"));
        assert_eq!(
            packed_manifest(&dir.0, "demo-im-0.3.0-windows-x64.hhhim")
                .min_host
                .as_deref(),
            Some("0.9.0")
        );

        let higher = std::fs::read_to_string(dir.0.join(hhh_im_protocol::MANIFEST_FILE))
            .unwrap()
            .replace("\"minHost\":\"0.1.0\"", "\"minHost\":\"1.2.0\"");
        std::fs::write(dir.0.join(hhh_im_protocol::MANIFEST_FILE), higher).unwrap();
        run(args(&dir.0, &["--target", "linux-x64"])).unwrap();
        assert_eq!(index_of(&dir.0).min_host.as_deref(), Some("1.2.0"));
        assert_eq!(
            packed_manifest(&dir.0, "demo-im-0.3.0-linux-x64.hhhim")
                .min_host
                .as_deref(),
            Some("1.2.0")
        );
    }

    // 读不出插件用的 IM SDK 就不打包（发出去的版本一定写着要求）；不是 Rust 写的插件用 --sdk 写明。
    #[test]
    fn without_a_lock_the_pack_is_refused_unless_the_sdk_is_given() {
        // 外面那层有锁文件（别的项目），里面的插件仓库（有 .git）没有：不越过仓库根去读外面的。
        let outer = plugin("nolock");
        let repo = outer.0.join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        for file in [hhh_im_protocol::MANIFEST_FILE, "demo", "logo.svg"] {
            std::fs::copy(outer.0.join(file), repo.join(file)).unwrap();
        }
        let error = run(args(&repo, &["--target", "windows-x64"])).unwrap_err();
        assert!(
            error.contains("Cargo.lock") && error.contains("--sdk"),
            "{error}"
        );
        assert!(
            !repo.join("dist/demo-im-0.3.0-windows-x64.hhhim").exists(),
            "读不出要求时不留包"
        );

        run(args(&repo, &["--target", "windows-x64", "--sdk", "0.8.1"])).unwrap();
        assert_eq!(index_of(&repo).min_host.as_deref(), Some("0.8.0"));
        assert!(parse_args(["x".into(), "--sdk".into(), "0.8".into()].into_iter()).is_err());
    }

    // 锁文件里有几个版本取最高的；开头那行锁文件格式的 version 不算；锁里没有 IM SDK 的给原因。
    #[test]
    fn the_highest_locked_sdk_is_read_and_a_lock_without_it_is_explained() {
        let two = "version = 4\n\n[[package]]\nname = \"hhh-im-protocol\"\nversion = \"0.7.0\"\n\n[[package]]\nname = \"hhh-im-protocol\"\nversion = \"0.10.2\"\n";
        assert_eq!(sdk_in_lock(two), Some(Version(0, 10, 2)));
        assert_eq!(
            sdk_in_lock("version = 4\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n"),
            None
        );

        let dir = plugin("othercrate");
        std::fs::write(
            dir.0.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        let error = locked_sdk(&dir.0).unwrap_err();
        assert!(
            error.contains(SDK_PACKAGE) && error.contains("--sdk"),
            "{error}"
        );

        // 包放在插件仓库里的子文件夹（模板打包时是 dist/stage）：往上找到仓库根的锁文件。
        let stage = dir.0.join("dist").join("stage");
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::write(dir.0.join("Cargo.lock"), LOCK).unwrap();
        assert_eq!(locked_sdk(&stage).unwrap().0, Version(0, 9, 3));
    }

    fn args(dir: &Path, extra: &[&str]) -> Args {
        let mut raw = vec![dir.display().to_string()];
        raw.extend(extra.iter().map(|s| s.to_string()));
        parse_args(raw.into_iter()).unwrap()
    }

    // 契约 provider-protocol v14 §2.5：给了私钥就给写下的那份索引签名（签名能用公钥验过），再打一次不给私钥时删掉旧签名；
    // 私钥读不了时不打包。
    #[test]
    fn a_signing_key_signs_exactly_the_index_written() {
        let dir = plugin("signed");
        let keys = dir.0.join("keys");
        let (private, _, public_hex) = sign::keygen(&keys).unwrap();
        let summary = run(args(
            &dir.0,
            &[
                "--target",
                "windows-x64",
                "--sign-key",
                private.to_str().unwrap(),
            ],
        ))
        .unwrap();
        assert!(summary.contains("已签名"), "{summary}");
        let index = std::fs::read(dir.0.join("dist").join(RELEASE_INDEX_FILE)).unwrap();
        let signature =
            std::fs::read_to_string(dir.0.join("dist").join(RELEASE_SIGNATURE_FILE)).unwrap();
        let bytes: Vec<u8> = (0..64)
            .map(|i| u8::from_str_radix(&signature.trim()[i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        let public: Vec<u8> = (0..32)
            .map(|i| u8::from_str_radix(&public_hex[i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        let verifying =
            ed25519_dalek::VerifyingKey::from_bytes(&public.try_into().unwrap()).unwrap();
        assert!(
            verifying
                .verify_strict(
                    &index,
                    &ed25519_dalek::Signature::from_slice(&bytes).unwrap()
                )
                .is_ok()
        );

        run(args(&dir.0, &["--target", "linux-x64"])).unwrap();
        assert!(
            !dir.0.join("dist").join(RELEASE_SIGNATURE_FILE).exists(),
            "不签名时删掉对不上新索引的旧签名"
        );
        let missing = dir.0.join("keys").join("nope.key");
        assert!(
            run(args(&dir.0, &["--sign-key", missing.to_str().unwrap()]))
                .unwrap_err()
                .contains("读不了")
        );
    }

    #[test]
    fn two_targets_of_one_version_share_one_index() {
        let dir = plugin("two");
        let notes = dir.0.join("notes.md");
        std::fs::write(&notes, "### 改进\n- 更快").unwrap();
        run(args(
            &dir.0,
            &[
                "--target",
                "windows-x64",
                "--notes",
                notes.to_str().unwrap(),
                "--tag",
                "v0.3.0",
                "--after",
                "0.2.9",
            ],
        ))
        .unwrap();
        run(args(&dir.0, &["--target", "linux-x64"])).unwrap();
        let index: ReleaseIndex = serde_json::from_str(
            &std::fs::read_to_string(dir.0.join("dist").join(RELEASE_INDEX_FILE)).unwrap(),
        )
        .unwrap();
        index.check().unwrap();
        assert_eq!(
            index
                .packages
                .iter()
                .map(|p| p.target.as_str())
                .collect::<Vec<_>>(),
            ["linux-x64", "windows-x64"]
        );
        assert_eq!(index.notes, "### 改进\n- 更快");
        assert_eq!(
            (
                index.name.as_deref(),
                index.publisher.as_deref(),
                index.min_host.as_deref()
            ),
            (Some("演示"), Some("me"), Some("0.9.0"))
        );
        assert!(
            index
                .logo
                .as_deref()
                .unwrap()
                .starts_with("data:image/svg+xml;base64,")
        );
        for package in &index.packages {
            let (sha, size) =
                package::sha256_file(&dir.0.join("dist").join(&package.file)).unwrap();
            assert_eq!((sha, size), (package.sha256.clone(), package.size));
        }
    }

    #[test]
    fn a_wrong_tag_or_a_version_that_does_not_go_up_is_refused() {
        let dir = plugin("refuse");
        assert!(
            run(args(
                &dir.0,
                &["--target", "windows-x64", "--tag", "v0.3.1"]
            ))
            .unwrap_err()
            .contains("对不上")
        );
        assert!(
            run(args(
                &dir.0,
                &["--target", "windows-x64", "--after", "0.3.0"]
            ))
            .unwrap_err()
            .contains("要比上一个版本")
        );
        assert!(
            !dir.0.join("dist/demo-im-0.3.0-windows-x64.hhhim").exists(),
            "不合格的包不留下"
        );
        assert!(
            parse_args(["x".into(), "--target".into(), "windows-arm64".into()].into_iter())
                .is_err()
        );
    }

    // 只补最新版本索引补的是仓库里的那份：不和 --publish 一起用时说清，不打包。
    #[test]
    fn latest_only_needs_publish() {
        let error = parse_args(["x".into(), "--latest-only".into()].into_iter())
            .err()
            .expect("不带 --publish 要拒绝");
        assert!(error.contains("--publish"), "{error}");
        assert!(
            parse_args(["x".into(), "--publish".into(), "--latest-only".into()].into_iter())
                .unwrap()
                .latest_only
        );
    }

    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
