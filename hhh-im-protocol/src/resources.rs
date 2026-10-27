//! 消息资源的可选版本说明；版本编号由插件解释，宿主不接触平台地址。

use serde::{Deserialize, Serialize};

/// 一份资源最多描述的版本数；覆盖常见的多档预览与一份原件，限制声明体积。
pub const RESOURCE_VARIANTS_MAX: usize = 16;
/// MIME 类型与文件名的字节预算。
const MIME_MAX_BYTES: usize = 128;
const NAME_MAX_BYTES: usize = 1024;

/// 一个实际字节版本；同一份字节可以同时用于显示和原件保存。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceVariant {
    /// 在这份资源内稳定的版本编号；不可包含地址、凭据或控制字符。
    pub id: String,
    /// 可用于显示。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preview: bool,
    /// 平台提供的原始文件；不能用压缩预览代替。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub original: bool,
    /// 实际版本的 MIME。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    /// 实际版本的文件名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 实际版本的字节数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// 按显示方向的宽。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// 按显示方向的高。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

/// `resources/describe` 的结果；空列表表示当前没有任何可读取版本。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceVariants {
    pub variants: Vec<ResourceVariant>,
}

impl ResourceVariants {
    /// 核对声明预算、稳定编号、用途及尺寸，不接受相互矛盾的原件声明。
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.variants.len() > RESOURCE_VARIANTS_MAX {
            return Err("资源版本太多。");
        }
        let mut ids = std::collections::HashSet::new();
        let mut originals = 0;
        for variant in &self.variants {
            if variant.id.is_empty()
                || variant.id.len() > crate::budget::ID_MAX_BYTES
                || variant.id.chars().any(char::is_control)
                || variant.id.contains("://")
                || !ids.insert(&variant.id)
                || (!variant.preview && !variant.original)
            {
                return Err("资源版本的编号或用途不正确。");
            }
            originals += usize::from(variant.original);
            if originals > 1 {
                return Err("同一资源只能声明一份原件。");
            }
            if !matches!(
                (variant.width, variant.height),
                (None, None) | (Some(1..), Some(1..))
            ) {
                return Err("资源版本的尺寸不正确。");
            }
            if variant
                .mime
                .as_ref()
                .is_some_and(|v| v.len() > MIME_MAX_BYTES || v.chars().any(char::is_control))
                || variant
                    .name
                    .as_ref()
                    .is_some_and(|v| v.len() > NAME_MAX_BYTES || v.chars().any(char::is_control))
            {
                return Err("资源版本的格式或名字不正确。");
            }
        }
        Ok(())
    }

    /// 选能覆盖目标尺寸的最小预览；都不够时选最大的，不把原件偷偷当预览。
    pub fn preview(&self, side: u32) -> Option<&ResourceVariant> {
        self.variants.iter().filter(|v| v.preview).min_by_key(|v| {
            let actual = v.width.zip(v.height).map(|(w, h)| w.max(h));
            match actual {
                Some(size) if size >= side => (0, size),
                Some(size) => (1, u32::MAX - size),
                None => (2, 0),
            }
        })
    }

    /// 取如实声明的原件；没有时由宿主明确说明，不回退到预览。
    pub fn original(&self) -> Option<&ResourceVariant> {
        self.variants.iter().find(|v| v.original)
    }
}
