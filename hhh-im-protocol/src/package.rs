//! 安装包 `.hhhim` 的打包、读清单与解包（契约 provider-protocol v10 §4.1；feature `package`）。
//!
//! 安装包就是 zip：最上层是清单 [`crate::MANIFEST_FILE`]，其余文件按清单里的相对路径放。宿主解包时拒绝绝对路径、`..`、
//! 链接与超过上限的包（文件数、单个与合计解压后字节数都按真实写出的字节算，防“压缩炸弹”）。打包工具与宿主共用这一份。

use std::io::{Read, Seek, Write};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::Manifest;
use crate::release::{PACKAGE_EXTENSION, Target, package_file_name};

/// 一个包最多几个文件（与宿主复制插件文件夹的上限相同）。
pub const FILES_MAX: usize = 10_000;
/// 一个包解压后合计最多多少字节。
pub const BYTES_MAX: u64 = 1024 * 1024 * 1024;
/// 包内路径最多几层。
pub const DEPTH_MAX: usize = 32;
/// 清单最大字节数。
pub const MANIFEST_MAX_BYTES: u64 = 256 * 1024;

/// 读不了、不合规的包。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageError(pub String);

impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PackageError {}

fn fail<T>(message: impl Into<String>) -> Result<T, PackageError> {
    Err(PackageError(message.into()))
}

/// 包内一个路径是否安全：相对、每一段都是普通名字（没有 `..`、盘符、根），层数不超过 [`DEPTH_MAX`]。
fn safe_relative(path: &Path) -> bool {
    let mut depth = 0;
    for component in path.components() {
        match component {
            Component::Normal(_) => depth += 1,
            _ => return false,
        }
    }
    depth > 0 && depth <= DEPTH_MAX
}

/// 只从包里读清单（不解包）：清单在最上层、大小有限、是合法 JSON。
///
/// # Examples
///
/// ```
/// let dir = std::env::temp_dir().join(format!("hhh-im-pack-doc-{}", std::process::id()));
/// std::fs::create_dir_all(&dir).unwrap();
/// std::fs::write(dir.join("hhh-im-provider.json"), r#"{"manifestVersion":1,"id":"demo-im","name":"演示","version":"0.1.0","command":"demo","os":["windows","macos","linux"]}"#).unwrap();
/// std::fs::write(dir.join("demo"), b"#!/bin/sh").unwrap();
/// let out = dir.join("dist");
/// let packed = hhh_im_protocol::package::pack(&dir, hhh_im_protocol::release::Target::LinuxX64, &out).unwrap();
/// let manifest = hhh_im_protocol::package::read_manifest(std::fs::File::open(&packed.path).unwrap()).unwrap();
/// assert_eq!(manifest.target.as_deref(), Some("linux-x64"));
/// std::fs::remove_dir_all(&dir).unwrap();
/// ```
pub fn read_manifest<R: Read + Seek>(reader: R) -> Result<Manifest, PackageError> {
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|error| PackageError(format!("不是能读的安装包：{error}")))?;
    let mut entry = archive
        .by_name(crate::MANIFEST_FILE)
        .map_err(|_| PackageError(format!("安装包最上层没有 {}。", crate::MANIFEST_FILE)))?;
    if entry.size() > MANIFEST_MAX_BYTES {
        return fail("安装包里的清单太大。");
    }
    let mut text = String::new();
    entry
        .by_ref()
        .take(MANIFEST_MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|error| PackageError(format!("安装包里的清单读不了：{error}")))?;
    serde_json::from_str(&text)
        .map_err(|error| PackageError(format!("安装包里的清单格式不对：{error}")))
}

/// 解包到 `dest`（必须是已经建好的空文件夹）：先核对每一项（路径、链接、文件数），再逐个写出、边写边数字节。
/// 任何一项不过就停下返回原因；已经写出的文件由调用方连文件夹一起删。返回写出的文件数。
pub fn unpack<R: Read + Seek>(reader: R, dest: &Path) -> Result<usize, PackageError> {
    unpack_within(reader, dest, FILES_MAX, BYTES_MAX)
}

/// [`unpack`] 的本体，上限由调用方给（测试用小上限核对“超过就停”，不必真造 1 GB 的包）。
fn unpack_within<R: Read + Seek>(
    reader: R,
    dest: &Path,
    files_max: usize,
    bytes_max: u64,
) -> Result<usize, PackageError> {
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|error| PackageError(format!("不是能读的安装包：{error}")))?;
    if archive.len() > files_max * 2 {
        return fail("安装包里的文件太多，不能安装。");
    }
    let mut plan: Vec<(usize, PathBuf, bool)> = Vec::with_capacity(archive.len());
    let mut files = 0usize;
    let mut declared = 0u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| PackageError(format!("安装包读不了：{error}")))?;
        // 原名也要看：zip 库会把开头的 `/` 去掉再给路径，绝对路径、反斜杠、盘符这种写法一律不收。
        let Ok(raw) = entry.name() else {
            return fail("安装包里有读不出的文件名，不能安装。");
        };
        let raw_ok = !raw.starts_with('/') && !raw.contains('\\') && !raw.contains(':');
        let Some(path) = entry
            .enclosed_name()
            .filter(|path| raw_ok && safe_relative(path))
        else {
            return fail(format!("安装包里有不安全的路径（{raw}），不能安装。"));
        };
        if entry.is_symlink() {
            return fail(format!("安装包里有链接（{raw}），不能安装。"));
        }
        if !entry.is_dir() {
            files += 1;
            declared = declared.saturating_add(entry.size());
        }
        if files > files_max || declared > bytes_max {
            return fail("安装包太大（超过 1 万个文件或解压后 1 GB），不能安装。");
        }
        plan.push((index, path, entry.is_dir()));
    }
    let mut written = 0u64;
    for (index, path, is_dir) in plan {
        let to = dest.join(&path);
        if is_dir {
            std::fs::create_dir_all(&to).map_err(|error| {
                PackageError(format!("解包建不了文件夹 {}：{error}", path.display()))
            })?;
            continue;
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| PackageError(format!("解包建不了文件夹：{error}")))?;
        }
        let mut entry = archive
            .by_index(index)
            .map_err(|error| PackageError(format!("安装包读不了：{error}")))?;
        let mode = entry.unix_mode();
        let mut out = std::fs::File::create(&to)
            .map_err(|error| PackageError(format!("解包写不了 {}：{error}", path.display())))?;
        // 按真实写出的字节数：声明的大小可能是假的。
        let budget = bytes_max - written;
        let copied = std::io::copy(&mut entry.by_ref().take(budget + 1), &mut out)
            .map_err(|error| PackageError(format!("解包写不了 {}：{error}", path.display())))?;
        if copied > budget {
            return fail("安装包解压后超过 1 GB，不能安装。");
        }
        written += copied;
        set_mode(&to, mode);
    }
    Ok(files)
}

/// Unix 上照包里记的权限补上执行位（插件程序要能运行）；Windows 不需要。
fn set_mode(path: &Path, mode: Option<u32>) {
    #[cfg(unix)]
    if let Some(mode) = mode.filter(|mode| mode & 0o111 != 0) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o755));
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
}

/// 一个文件的 sha256（小写十六进制）与字节数。
pub fn sha256_file(path: &Path) -> std::io::Result<(String, u64)> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((hex(&hasher.finalize()), size))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 打出来的一个包。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packed {
    /// 包的完整路径。
    pub path: PathBuf,
    /// 包名（不带路径）。
    pub file: String,
    pub sha256: String,
    pub size: u64,
    /// 包里的清单（已写上目标）。
    pub manifest: Manifest,
}

/// 把插件文件夹打成给 `target` 的安装包，放进 `out`（没有就建）：清单写上目标、其余文件原样；跳过以点开头的文件与
/// 文件夹、`out` 本身和别的 `.hhhim`。清单要合规（编号、版本号、发布字段，且支持这个目标的系统）。
/// 打 Windows 包时清单的 `command` 没写扩展名、文件夹里有 `<command>.exe`，包里的清单写成带 `.exe` 的：同一份清单能打
/// 各系统的包（宿主按清单路径直接启动程序，Windows 上要带扩展名）。打 iPhone / iPad 的 WASM 包时同理找 `<command>.wasm`，
/// 而且程序必须是 WASM 文件（契约 provider-protocol v18）。
pub fn pack(dir: &Path, target: Target, out: &Path) -> Result<Packed, PackageError> {
    let manifest_path = dir.join(crate::MANIFEST_FILE);
    let text = std::fs::read_to_string(&manifest_path).map_err(|error| {
        PackageError(format!("读不到清单 {}：{error}", manifest_path.display()))
    })?;
    let mut manifest: Manifest = serde_json::from_str(&text)
        .map_err(|error| PackageError(format!("清单格式不对：{error}")))?;
    if !Manifest::id_ok(&manifest.id) {
        return fail(format!("清单编号 {} 不合规。", manifest.id));
    }
    if crate::release::Version::parse(&manifest.version).is_none() {
        return fail(format!("清单版本号 {} 不是 主.次.修订。", manifest.version));
    }
    if !manifest.os.iter().any(|os| os == target.os()) {
        return fail(format!(
            "清单的 os 里没有 {}，不能打 {} 的包。",
            target.os(),
            target.as_str()
        ));
    }
    manifest.target = Some(target.as_str().to_owned());
    let windows_program = format!("{}.exe", manifest.command);
    if target.os() == "windows"
        && !dir.join(&manifest.command).is_file()
        && dir.join(&windows_program).is_file()
    {
        manifest.command = windows_program;
    }
    if target.is_wasm() {
        let wasm_program = format!("{}.wasm", manifest.command);
        if !manifest.command.ends_with(".wasm") && dir.join(&wasm_program).is_file() {
            manifest.command = wasm_program;
        }
        if !wasm_file(&dir.join(&manifest.command)) {
            return fail(format!(
                "打 {} 的包要放 WASM 程序：{} 不是 WASM 文件。",
                target.label(),
                manifest.command
            ));
        }
    }
    manifest.release_fields_ok().map_err(PackageError)?;
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|error| PackageError(format!("清单写不出：{error}")))?;
    std::fs::create_dir_all(out)
        .map_err(|error| PackageError(format!("输出文件夹建不了：{error}")))?;
    let out_canonical = std::fs::canonicalize(out).ok();
    let file = package_file_name(&manifest.id, &manifest.version, target);
    let path = out.join(&file);
    let partial = out.join(format!(".{file}.partial"));
    let result = (|| {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&partial)?);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(crate::MANIFEST_FILE, options.unix_permissions(0o644))?;
        zip.write_all(manifest_json.as_bytes())?;
        let mut stack = vec![PathBuf::new()];
        let mut files = 1usize;
        while let Some(relative) = stack.pop() {
            let mut children: Vec<_> =
                std::fs::read_dir(dir.join(&relative))?.collect::<Result<_, _>>()?;
            children.sort_by_key(|entry| entry.file_name());
            for entry in children {
                let name = entry.file_name();
                let name_text = name.to_string_lossy();
                let child = relative.join(&name);
                let kind = entry.file_type()?;
                let skip = name_text.starts_with('.')
                    || (relative.as_os_str().is_empty() && name_text == crate::MANIFEST_FILE)
                    || name_text.ends_with(&format!(".{PACKAGE_EXTENSION}"))
                    || out_canonical.as_ref().is_some_and(|out| {
                        std::fs::canonicalize(entry.path()).ok().as_ref() == Some(out)
                    });
                if skip {
                    continue;
                }
                if kind.is_symlink() {
                    return Err(std::io::Error::other(format!(
                        "插件文件夹里有链接（{}），换成真正的文件再打包",
                        child.display()
                    )));
                }
                let zip_name = child.to_string_lossy().replace('\\', "/");
                if kind.is_dir() {
                    zip.add_directory(format!("{zip_name}/"), options)?;
                    stack.push(child);
                    continue;
                }
                files += 1;
                if files > FILES_MAX {
                    return Err(std::io::Error::other("插件文件夹超过 1 万个文件"));
                }
                // 启动程序要能运行：清单里的命令一律给执行位，别的照本机权限（Windows 上没有，给 0644）。
                let executable = zip_name == manifest.command || unix_executable(&entry.path());
                zip.start_file(
                    zip_name,
                    options.unix_permissions(if executable { 0o755 } else { 0o644 }),
                )?;
                std::io::copy(&mut std::fs::File::open(entry.path())?, &mut zip)?;
            }
        }
        zip.finish()?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&partial);
        return fail(format!("打包失败：{error}"));
    }
    std::fs::rename(&partial, &path)
        .map_err(|error| PackageError(format!("包换不上名字：{error}")))?;
    let (sha256, size) =
        sha256_file(&path).map_err(|error| PackageError(format!("包读不了：{error}")))?;
    Ok(Packed {
        path,
        file,
        sha256,
        size,
        manifest,
    })
}

/// 文件开头是不是 WASM 的魔数（`\0asm`）。
fn wasm_file(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_ok()
        && &magic == b"\0asm"
}

fn unix_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
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
    fn temp(name: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!(
            "hhh-im-package-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }

    const MANIFEST: &str = r#"{"manifestVersion":1,"id":"demo-im","name":"演示","version":"0.2.0","command":"bin/demo","os":["windows","linux"],
        "source":{"type":"github","repo":"someone/hhh-demo"}}"#;

    fn plugin(root: &Path) -> PathBuf {
        let dir = root.join("plugin");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join(crate::MANIFEST_FILE), MANIFEST).unwrap();
        std::fs::write(dir.join("bin/demo"), b"program").unwrap();
        std::fs::write(dir.join("logo.svg"), b"<svg/>").unwrap();
        std::fs::write(dir.join(".secret"), b"skip me").unwrap();
        std::fs::write(dir.join("old-0.1.0-linux-x64.hhhim"), b"old").unwrap();
        dir
    }

    fn zip_with(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, body) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(body).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn a_packed_plugin_unpacks_to_the_same_files_with_the_target_in_its_manifest() {
        let root = temp("roundtrip");
        let dir = plugin(&root.0);
        let out = dir.join("dist");
        let packed = pack(&dir, Target::LinuxX64, &out).unwrap();
        assert_eq!(packed.file, "demo-im-0.2.0-linux-x64.hhhim");
        assert_eq!(
            (packed.size, packed.sha256.len()),
            (std::fs::metadata(&packed.path).unwrap().len(), 64)
        );
        assert_eq!(sha256_file(&packed.path).unwrap().0, packed.sha256);
        let manifest = read_manifest(std::fs::File::open(&packed.path).unwrap()).unwrap();
        assert_eq!(manifest.target.as_deref(), Some("linux-x64"));
        let dest = root.0.join("unpacked");
        std::fs::create_dir_all(&dest).unwrap();
        assert_eq!(
            unpack(std::fs::File::open(&packed.path).unwrap(), &dest).unwrap(),
            3
        );
        assert_eq!(std::fs::read(dest.join("bin/demo")).unwrap(), b"program");
        assert!(dest.join("logo.svg").is_file());
        assert!(
            !dest.join(".secret").exists()
                && !dest.join("dist").exists()
                && !dest.join("old-0.1.0-linux-x64.hhhim").exists()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                std::fs::metadata(dest.join("bin/demo"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o111,
                0
            );
        }
    }

    // 同一份清单打各系统的包：Windows 包里命令写成带 .exe 的（文件夹里有它时），别的系统原样。
    #[test]
    fn a_windows_package_points_the_command_at_the_exe() {
        let root = temp("exe");
        let dir = root.0.join("plugin");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join(crate::MANIFEST_FILE), MANIFEST).unwrap();
        std::fs::write(dir.join("bin/demo.exe"), b"windows program").unwrap();
        let packed = pack(&dir, Target::WindowsX64, &root.0.join("dist")).unwrap();
        assert_eq!(packed.manifest.command, "bin/demo.exe");
        let linux = pack(&dir, Target::LinuxX64, &root.0.join("dist")).unwrap();
        assert_eq!(linux.manifest.command, "bin/demo", "别的系统不改");
    }

    // iPhone / iPad 的包：命令指向 `<command>.wasm`（文件夹里有它时），程序必须是 WASM 文件；清单的 os 要写 ios。
    #[test]
    fn a_wasm_package_points_the_command_at_the_wasm_program() {
        let root = temp("wasm");
        let dir = root.0.join("plugin");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let manifest = MANIFEST.replace(r#"["windows","linux"]"#, r#"["windows","linux","ios"]"#);
        std::fs::write(dir.join(crate::MANIFEST_FILE), &manifest).unwrap();
        std::fs::write(dir.join("bin/demo.wasm"), b"\0asm\x0d\0\x01\0").unwrap();
        let packed = pack(&dir, Target::Wasm32Wasip2, &root.0.join("dist")).unwrap();
        assert_eq!(packed.file, "demo-im-0.2.0-wasm32-wasip2.hhhim");
        assert_eq!(packed.manifest.command, "bin/demo.wasm");
        assert_eq!(packed.manifest.target.as_deref(), Some("wasm32-wasip2"));
        assert!(packed.manifest.release_fields_ok().is_ok());

        std::fs::write(dir.join("bin/demo.wasm"), b"not wasm").unwrap();
        let error = pack(&dir, Target::Wasm32Wasip2, &root.0.join("dist")).unwrap_err();
        assert!(error.0.contains("WASM"), "{}", error.0);

        std::fs::write(dir.join(crate::MANIFEST_FILE), MANIFEST).unwrap();
        let error = pack(&dir, Target::Wasm32Wasip2, &root.0.join("dist")).unwrap_err();
        assert!(error.0.contains("ios"), "清单没写 ios 不打：{}", error.0);
    }

    #[test]
    fn packing_refuses_a_target_the_manifest_does_not_support_or_a_bad_version() {
        let root = temp("refuse");
        let dir = plugin(&root.0);
        assert!(
            pack(&dir, Target::MacosArm64, &dir.join("dist"))
                .unwrap_err()
                .0
                .contains("macos")
        );
        std::fs::write(
            dir.join(crate::MANIFEST_FILE),
            MANIFEST.replace("0.2.0", "0.2"),
        )
        .unwrap();
        assert!(
            pack(&dir, Target::LinuxX64, &dir.join("dist"))
                .unwrap_err()
                .0
                .contains("版本号")
        );
    }

    #[test]
    fn unsafe_paths_links_and_missing_manifests_are_refused() {
        let root = temp("unsafe");
        let evil = root.0.join("evil.hhhim");
        zip_with(
            &evil,
            &[
                (crate::MANIFEST_FILE, MANIFEST.as_bytes()),
                ("../outside.txt", b"x"),
            ],
        );
        let dest = root.0.join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        assert!(
            unpack(std::fs::File::open(&evil).unwrap(), &dest)
                .unwrap_err()
                .0
                .contains("不安全的路径")
        );
        assert!(!root.0.join("outside.txt").exists());
        zip_with(
            &evil,
            &[
                (crate::MANIFEST_FILE, MANIFEST.as_bytes()),
                ("/abs.txt", b"x"),
            ],
        );
        assert!(unpack(std::fs::File::open(&evil).unwrap(), &dest).is_err());
        let bare = root.0.join("bare.hhhim");
        zip_with(&bare, &[("readme.md", b"x")]);
        assert!(
            read_manifest(std::fs::File::open(&bare).unwrap())
                .unwrap_err()
                .0
                .contains("最上层没有")
        );
        std::fs::write(&bare, b"not a zip").unwrap();
        assert!(
            read_manifest(std::fs::File::open(&bare).unwrap())
                .unwrap_err()
                .0
                .contains("不是能读的安装包")
        );
        let linked = root.0.join("linked.hhhim");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&linked).unwrap());
        zip.add_symlink(
            "link",
            "/etc/passwd",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.finish().unwrap();
        assert!(
            unpack(std::fs::File::open(&linked).unwrap(), &dest)
                .unwrap_err()
                .0
                .contains("链接")
        );
    }

    // 超过上限的包（防“压缩炸弹”）：文件数、解压后合计字节数超了都拒绝，什么也不写出；上限正好时照装。
    #[test]
    fn packages_over_the_file_or_byte_limits_are_refused() {
        let root = temp("limits");
        let many = root.0.join("many.hhhim");
        let names: Vec<String> = (0..4).map(|i| format!("f{i}.txt")).collect();
        let mut entries: Vec<(&str, &[u8])> = vec![(crate::MANIFEST_FILE, MANIFEST.as_bytes())];
        entries.extend(names.iter().map(|name| (name.as_str(), b"x".as_slice())));
        zip_with(&many, &entries);
        let dest = root.0.join("many");
        std::fs::create_dir_all(&dest).unwrap();
        let open = || std::fs::File::open(&many).unwrap();
        assert!(
            unpack_within(open(), &dest, 4, BYTES_MAX)
                .unwrap_err()
                .0
                .contains("太大")
        );
        assert_eq!(
            std::fs::read_dir(&dest).unwrap().count(),
            0,
            "超了什么也不写"
        );
        assert_eq!(unpack_within(open(), &dest, 5, BYTES_MAX).unwrap(), 5);

        let big = root.0.join("big.hhhim");
        let zeros = vec![0u8; 4096];
        zip_with(
            &big,
            &[
                (crate::MANIFEST_FILE, MANIFEST.as_bytes()),
                ("zeros.bin", &zeros),
            ],
        );
        let limit = MANIFEST.len() as u64 + 4096;
        let dest = root.0.join("big");
        std::fs::create_dir_all(&dest).unwrap();
        let open = || std::fs::File::open(&big).unwrap();
        assert!(
            unpack_within(open(), &dest, FILES_MAX, limit - 1)
                .unwrap_err()
                .0
                .contains("太大")
        );
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0);
        assert_eq!(unpack_within(open(), &dest, FILES_MAX, limit).unwrap(), 2);
    }

    #[test]
    fn the_safe_path_rule_takes_only_plain_relative_names() {
        assert!(safe_relative(Path::new("a/b/c.txt")));
        assert!(!safe_relative(Path::new("")));
        assert!(!safe_relative(Path::new("../a")));
        assert!(!safe_relative(Path::new("/a")));
        let deep: PathBuf = (0..=DEPTH_MAX).map(|i| i.to_string()).collect();
        assert!(!safe_relative(&deep));
    }
}
