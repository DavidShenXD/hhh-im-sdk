//! 账号拥有的贴纸与表情包。编号是插件提供的不透明值，不是 URL 或上传文件键。
use serde::{Deserialize, Serialize};

/// 一页最多 100 项，限制插件回包和界面内存。
pub const STICKER_PAGE_MAX: u16 = 100;
/// 编号和游标最多 512 字节；不允许空值或控制字符。
pub const STICKER_ID_MAX: usize = 512;
/// 核对插件编号；宿主不解释平台的编号格式。
pub fn valid_sticker_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= STICKER_ID_MAX && !value.chars().any(char::is_control)
}

/// 各项独立声明；旧插件全部关闭。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StickerCaps {
    /// 列账号已有的表情包及包内贴纸。
    #[serde(default)]
    pub packs: bool,
    /// 读取账号在平台上的收藏。
    #[serde(default)]
    pub favorites: bool,
    /// 收藏目录中已有的贴纸。
    #[serde(default)]
    pub favorite_add: bool,
    /// 取消收藏。
    #[serde(default)]
    pub favorite_remove: bool,
    /// 以原生贴纸发送；走 messages/send 与同一操作编号。
    #[serde(default)]
    pub send: bool,
}
impl StickerCaps {
    /// 全部关闭，供静态平台说明使用。
    pub const NONE: Self = Self {
        packs: false,
        favorites: false,
        favorite_add: false,
        favorite_remove: false,
        send: false,
    };
    /// 是否完全未声明。
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// 可发送的贴纸。图片通过 stickers/image 获取，不把凭据或远端 URL 交给界面。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sticker {
    /// 当前账号内稳定的发送与收藏编号。
    pub id: String,
    /// 可访问名称。
    pub name: String,
    /// 图片版本键；资源变化时插件必须换键。
    pub image_key: String,
    /// 是否已收藏；平台未知时不填，界面不猜。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favorite: Option<bool>,
}

/// 账号已有的一个表情包（不包括商店）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StickerPack {
    /// 包编号。
    pub id: String,
    /// 包名称。
    pub name: String,
    /// 封面图片键，同样交给 stickers/image；没有时用通用包图标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
}

/// 分页结果；next 为空才表示已经读完。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StickerPage<T> {
    /// 按平台顺序给出的一页。
    pub items: Vec<T>,
    /// 下一页游标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}
