//! 打包工具 `hhh-im-pack`（契约 provider-protocol v10 §4.3、§4.9）：把插件文件夹打成给某个目标的 `.hhhim`，并写好
//! 这个版本的版本索引 `hhhim-release.json`（同一版本打几个目标，索引里就有几个包）。
//!
//! 用法：
//! ```text
//! hhh-im-pack <插件文件夹> [--target <目标>] [--out <输出文件夹>] [--notes <说明.md>] [--tag <标签>] [--after <上一个版本>] [--publish]
//! ```
//! - `--target`：`windows-x64`、`macos-arm64`、`macos-x64`、`linux-x64`、`linux-arm64`；不写用这台电脑的目标。
//! - `--out`：包与索引放哪，默认插件文件夹下的 `dist/`。
//! - `--notes`：更新说明（Markdown 文件）；不写时沿用输出文件夹里同一版本索引已有的说明。
//! - `--tag`：发布用的标签，必须是 `v<清单版本>`，不一致就不打。
//! - `--after`：上一个发布的版本；新版本必须比它大。
//! - `--publish`：打好后发到清单 `source` 写的 GitHub / GitLab 仓库（见 [`publish`]）：标签默认 `v<清单版本>`，新版本要比
//!   仓库里最新的正式版本大，仓库里同一版本已有的索引先合进来；令牌只从环境变量读（`HHH_IM_PUBLISH_TOKEN`，或
//!   `GITHUB_TOKEN` / `GITLAB_TOKEN`）。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[path = "hhh_im_pack/publish.rs"]
mod publish;

use hhh_im_protocol::package;
use hhh_im_protocol::release::{
    INDEX_VERSION, LOGO_DATA_MAX_BYTES, NOTES_MAX_CHARS, RELEASE_INDEX_FILE, ReleaseIndex,
    ReleasePackage, Target, Version,
};

struct Args {
    dir: PathBuf,
    target: Target,
    out: PathBuf,
    notes: Option<PathBuf>,
    tag: Option<String>,
    after: Option<String>,
    publish: bool,
}

fn usage() -> String {
    "用法：hhh-im-pack <插件文件夹> [--target <目标>] [--out <输出文件夹>] [--notes <说明.md>] [--tag <标签>] [--after <上一个版本>] [--publish]".into()
}

fn parse_args(raw: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut dir = None;
    let mut target = None;
    let mut out = None;
    let mut notes = None;
    let mut tag = None;
    let mut after = None;
    let mut publish = false;
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
            "--publish" => publish = true,
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
        publish,
    })
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
    let packed = package::pack(&args.dir, args.target, &args.out).map_err(|error| error.0)?;
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
        min_host: manifest.min_host.clone(),
        notes,
        packages,
    };
    index.check()?;
    let json =
        serde_json::to_string_pretty(&index).map_err(|error| format!("版本索引写不出：{error}"))?;
    std::fs::write(&index_path, json + "\n")
        .map_err(|error| format!("版本索引写不进 {}：{error}", index_path.display()))?;
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
    if let Some(publisher) = publisher {
        let page = publisher.upload(
            &index.id,
            &[packed.path.as_path(), index_path.as_path()],
            &index.notes,
        )?;
        summary.push_str(&format!("\n已发布到 {page}"));
    }
    Ok(summary)
}

fn main() -> ExitCode {
    match parse_args(std::env::args().skip(1)).and_then(run) {
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
        Dir(dir)
    }

    fn args(dir: &Path, extra: &[&str]) -> Args {
        let mut raw = vec![dir.display().to_string()];
        raw.extend(extra.iter().map(|s| s.to_string()));
        parse_args(raw.into_iter()).unwrap()
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
            (Some("演示"), Some("me"), Some("0.1.0"))
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

    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
