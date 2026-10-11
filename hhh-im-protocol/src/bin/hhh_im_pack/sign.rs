//! 官方签名（契约 provider-protocol v14 §2.5）：`hhh-im-pack keygen` 生成一对 ed25519 钥匙，发版本时 `--sign-key <私钥文件>`
//! 给版本索引签名，写出 `hhhim-release.json.sig`（与索引一起上传）。HHH 内置公钥，保留编号（官方插件）只认它签过的版本索引。
//!
//! 钥匙都是一行 64 位十六进制的文本文件（私钥是 32 字节种子），哪个系统都能读写；私钥放在用户的私有密钥仓库，不进任何公开仓库。
//! 签的是这次要上传的索引文件的原样字节：之后改了索引（补传别的目标）要重新签。

use std::path::{Path, PathBuf};

use ed25519_dalek::{Signer as _, SigningKey};

/// 私钥文件名（`keygen` 写出、`--sign-key` 常用的名字）。
pub(crate) const PRIVATE_KEY_FILE: &str = "hhh-official-signing.key";
/// 公钥文件名（写进 HHH 的 `OFFICIAL_KEYS`）。
pub(crate) const PUBLIC_KEY_FILE: &str = "hhh-official-signing.pub";

/// 生成一对钥匙写进 `dir`；已有同名文件时拒绝（不覆盖已在用的钥匙）。返回两个文件的路径与公钥（十六进制）。
pub(crate) fn keygen(dir: &Path) -> Result<(PathBuf, PathBuf, String), String> {
    let private = dir.join(PRIVATE_KEY_FILE);
    let public = dir.join(PUBLIC_KEY_FILE);
    if private.exists() || public.exists() {
        return Err(format!(
            "{} 里已经有钥匙了，不覆盖（换钥匙要先自己挪走旧的）。",
            dir.display()
        ));
    }
    std::fs::create_dir_all(dir).map_err(|error| format!("{} 建不了：{error}", dir.display()))?;
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|error| format!("系统随机数读不到：{error}"))?;
    let key = SigningKey::from_bytes(&seed);
    let public_hex = hex(key.verifying_key().as_bytes());
    write_new(&private, &format!("{}\n", hex(&seed)))?;
    write_new(&public, &format!("{public_hex}\n"))?;
    Ok((private, public, public_hex))
}

/// 读私钥文件（一行 64 位十六进制，前后空白不算）。
pub(crate) fn load(path: &Path) -> Result<SigningKey, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("私钥文件 {} 读不了：{error}", path.display()))?;
    let seed = parse_hex32(text.trim())
        .ok_or_else(|| format!("私钥文件 {} 不是一行 64 位十六进制。", path.display()))?;
    Ok(SigningKey::from_bytes(&seed))
}

/// 给一份字节签名，返回 128 位十六进制（HHH 认十六进制、base64 与原始 64 字节三种写法）。
pub(crate) fn sign(key: &SigningKey, message: &[u8]) -> String {
    hex(&key.sign(message).to_bytes())
}

fn write_new(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("{} 写不了：{error}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|error| format!("{} 写不了：{error}", path.display()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_hex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 8032 §7.1 的测试 1：同一个种子、同一份消息，签出来的与标准一致（Friend 用 aws-lc-rs 验签，两边是同一套算法；
    // Friend 那边用同一组向量测 `signature_ok`）。
    #[test]
    fn signing_matches_the_rfc_8032_vector() {
        let seed = parse_hex32("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
            .unwrap();
        let key = SigningKey::from_bytes(&seed);
        assert_eq!(
            hex(key.verifying_key().as_bytes()),
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
        assert_eq!(
            sign(&key, b""),
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
        );
    }

    // 生成的钥匙读得回、签得出能验过的签名；同一个目录不再生成第二对；私钥文件写坏了说清楚。
    #[test]
    fn a_generated_key_signs_and_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let (private, public, public_hex) = keygen(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&public).unwrap().trim(), public_hex);
        let key = load(&private).unwrap();
        assert_eq!(hex(key.verifying_key().as_bytes()), public_hex);
        let signature = sign(&key, b"{\"indexVersion\":1}\n");
        let bytes: Vec<u8> = (0..64)
            .map(|i| u8::from_str_radix(&signature[i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        let signature = ed25519_dalek::Signature::from_slice(&bytes).unwrap();
        assert!(
            key.verifying_key()
                .verify_strict(b"{\"indexVersion\":1}\n", &signature)
                .is_ok()
        );
        assert!(keygen(dir.path()).is_err(), "已有钥匙时不覆盖");
        std::fs::write(&private, "not a key").unwrap();
        assert!(load(&private).unwrap_err().contains("64 位十六进制"));
    }
}
