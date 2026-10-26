//! 请求体：整块放在内存里的字节（对应 `reqwest::Body`；iPhone 版没有流式请求体）。

use bytes::Bytes;

/// 请求体。字节、字符串都能直接转成它；跟随 307 / 308 跳转时整块重发。
#[derive(Debug, Clone, Default)]
pub struct Body(Bytes);

impl Body {
    /// 整块的内容（iPhone 版总是整块，所以总是 `Some`）。
    pub fn as_bytes(&self) -> Option<&[u8]> {
        Some(&self.0)
    }

    pub(crate) fn bytes(&self) -> Bytes {
        self.0.clone()
    }
}

impl From<Bytes> for Body {
    fn from(bytes: Bytes) -> Self {
        Self(bytes)
    }
}

impl From<Vec<u8>> for Body {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes.into())
    }
}

impl From<String> for Body {
    fn from(text: String) -> Self {
        Self(text.into())
    }
}

impl From<&'static str> for Body {
    fn from(text: &'static str) -> Self {
        Self(Bytes::from_static(text.as_bytes()))
    }
}

impl From<&'static [u8]> for Body {
    fn from(bytes: &'static [u8]) -> Self {
        Self(Bytes::from_static(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::Body;

    #[test]
    fn every_source_keeps_its_bytes() {
        assert_eq!(Body::from("hi").as_bytes(), Some(&b"hi"[..]));
        assert_eq!(Body::from(String::from("hi")).as_bytes(), Some(&b"hi"[..]));
        assert_eq!(Body::from(vec![1u8, 2]).as_bytes(), Some(&[1u8, 2][..]));
        assert_eq!(Body::from(&b"ab"[..]).as_bytes(), Some(&b"ab"[..]));
        assert_eq!(Body::default().as_bytes(), Some(&b""[..]));
    }
}
