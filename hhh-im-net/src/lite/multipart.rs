//! `multipart/form-data` 表单（对应 `reqwest::multipart`），拼出来的字节与 reqwest 逐字节相同：
//! 每段 `--分界\r\n` + 段头 + `\r\n\r\n` + 内容 + `\r\n`，最后 `--分界--\r\n`；字段名按路径段规则百分号编码
//! （编码过就写成 `name*=utf-8''…`），文件名只转义 `\`、`"`、回车与换行。iPhone 版每段内容整块放在内存里。

use std::borrow::Cow;
use std::fmt;

use bytes::{Bytes, BytesMut};
use http::HeaderMap;
use mime::Mime;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC};

use super::error::{self, Result};

/// 一份表单。
pub struct Form {
    boundary: String,
    fields: Vec<(Cow<'static, str>, Part)>,
    percent_encoding: PercentEncoding,
}

/// 表单里的一段：文字或字节，可带文件名、类型与自定义段头。
pub struct Part {
    value: Bytes,
    mime: Option<Mime>,
    file_name: Option<Cow<'static, str>>,
    headers: HeaderMap,
}

#[derive(Debug, Clone, Copy)]
enum PercentEncoding {
    PathSegment,
    AttrChar,
    NoOp,
}

impl Default for Form {
    fn default() -> Self {
        Self::new()
    }
}

impl Form {
    /// 空表单，分界串随机生成。
    pub fn new() -> Self {
        Self {
            boundary: gen_boundary(),
            fields: Vec::new(),
            percent_encoding: PercentEncoding::PathSegment,
        }
    }

    /// 分界串。
    pub fn boundary(&self) -> &str {
        &self.boundary
    }

    /// 加一段文字。
    pub fn text<T, U>(self, name: T, value: U) -> Self
    where
        T: Into<Cow<'static, str>>,
        U: Into<Cow<'static, str>>,
    {
        self.part(name, Part::text(value))
    }

    /// 加一段。
    pub fn part<T>(mut self, name: T, part: Part) -> Self
    where
        T: Into<Cow<'static, str>>,
    {
        self.fields.push((name.into(), part));
        self
    }

    /// 字段名按 URL 路径段规则编码（缺省）。
    pub fn percent_encode_path_segment(mut self) -> Self {
        self.percent_encoding = PercentEncoding::PathSegment;
        self
    }

    /// 字段名按 RFC 8187 的属性字符规则编码。
    pub fn percent_encode_attr_chars(mut self) -> Self {
        self.percent_encoding = PercentEncoding::AttrChar;
        self
    }

    /// 字段名不编码。
    pub fn percent_encode_noop(mut self) -> Self {
        self.percent_encoding = PercentEncoding::NoOp;
        self
    }

    /// 整份表单的字节数（请求的 `Content-Length`）。
    pub(crate) fn compute_length(&self) -> u64 {
        let mut length = 0u64;
        for (name, part) in &self.fields {
            let header = self.percent_encoding.encode_headers(name, part);
            length += 2 + self.boundary.len() as u64 + 2 + header.len() as u64 + 4;
            length += part.value.len() as u64 + 2;
        }
        if !self.fields.is_empty() {
            length += 2 + self.boundary.len() as u64 + 4;
        }
        length
    }

    /// 拼成请求体。空表单是空请求体。
    pub(crate) fn into_bytes(self) -> Bytes {
        if self.fields.is_empty() {
            return Bytes::new();
        }
        let mut body = BytesMut::new();
        for (name, part) in &self.fields {
            body.extend_from_slice(format!("--{}\r\n", self.boundary).as_bytes());
            body.extend_from_slice(&self.percent_encoding.encode_headers(name, part));
            body.extend_from_slice(b"\r\n\r\n");
            body.extend_from_slice(&part.value);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        body.freeze()
    }
}

impl fmt::Debug for Form {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Form")
            .field("boundary", &self.boundary)
            .field("fields", &self.fields)
            .finish()
    }
}

impl Part {
    /// 一段文字（不带类型）。
    pub fn text<T>(value: T) -> Self
    where
        T: Into<Cow<'static, str>>,
    {
        let value = match value.into() {
            Cow::Borrowed(text) => Bytes::from_static(text.as_bytes()),
            Cow::Owned(text) => Bytes::from(text),
        };
        Self::new(value)
    }

    /// 一段字节（不带类型）。
    pub fn bytes<T>(value: T) -> Self
    where
        T: Into<Cow<'static, [u8]>>,
    {
        let value = match value.into() {
            Cow::Borrowed(bytes) => Bytes::from_static(bytes),
            Cow::Owned(bytes) => Bytes::from(bytes),
        };
        Self::new(value)
    }

    fn new(value: Bytes) -> Self {
        Self {
            value,
            mime: None,
            file_name: None,
            headers: HeaderMap::new(),
        }
    }

    /// 设这段的类型（`Content-Type`）。
    ///
    /// # Errors
    ///
    /// 类型写法不合法时返回 builder 错误。
    pub fn mime_str(mut self, mime: &str) -> Result<Self> {
        self.mime = Some(mime.parse().map_err(error::builder)?);
        Ok(self)
    }

    /// 设文件名。
    pub fn file_name<T>(mut self, file_name: T) -> Self
    where
        T: Into<Cow<'static, str>>,
    {
        self.file_name = Some(file_name.into());
        self
    }

    /// 设自定义段头。
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }
}

impl fmt::Debug for Part {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Part")
            .field("len", &self.value.len())
            .field("mime", &self.mime)
            .field("file_name", &self.file_name)
            .field("headers", &self.headers)
            .finish()
    }
}

// https://url.spec.whatwg.org/#fragment-percent-encode-set
const FRAGMENT_ENCODE_SET: &AsciiSet = &percent_encoding::CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'<')
    .add(b'>')
    .add(b'`');

// https://url.spec.whatwg.org/#path-percent-encode-set
const PATH_ENCODE_SET: &AsciiSet = &FRAGMENT_ENCODE_SET.add(b'#').add(b'?').add(b'{').add(b'}');

const PATH_SEGMENT_ENCODE_SET: &AsciiSet = &PATH_ENCODE_SET.add(b'/').add(b'%');

// https://tools.ietf.org/html/rfc8187#section-3.2.1
const ATTR_CHAR_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'!')
    .remove(b'#')
    .remove(b'$')
    .remove(b'&')
    .remove(b'+')
    .remove(b'-')
    .remove(b'.')
    .remove(b'^')
    .remove(b'_')
    .remove(b'`')
    .remove(b'|')
    .remove(b'~');

impl PercentEncoding {
    fn encode_headers(self, name: &str, part: &Part) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"Content-Disposition: form-data; ");
        match self.percent_encode(name) {
            Cow::Borrowed(value) => {
                buf.extend_from_slice(b"name=\"");
                buf.extend_from_slice(value.as_bytes());
                buf.extend_from_slice(b"\"");
            }
            Cow::Owned(value) => {
                buf.extend_from_slice(b"name*=utf-8''");
                buf.extend_from_slice(value.as_bytes());
            }
        }
        // RFC 7578 §4.2：不能用 `filename*=` 写法。
        if let Some(file_name) = &part.file_name {
            buf.extend_from_slice(b"; filename=\"");
            let legal = file_name
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\r', "\\\r")
                .replace('\n', "\\\n");
            buf.extend_from_slice(legal.as_bytes());
            buf.extend_from_slice(b"\"");
        }
        if let Some(mime) = &part.mime {
            buf.extend_from_slice(b"\r\nContent-Type: ");
            buf.extend_from_slice(mime.as_ref().as_bytes());
        }
        for (key, value) in &part.headers {
            buf.extend_from_slice(b"\r\n");
            buf.extend_from_slice(key.as_str().as_bytes());
            buf.extend_from_slice(b": ");
            buf.extend_from_slice(value.as_bytes());
        }
        buf
    }

    fn percent_encode(self, value: &str) -> Cow<'_, str> {
        use percent_encoding::utf8_percent_encode as percent_encode;
        match self {
            Self::PathSegment => percent_encode(value, PATH_SEGMENT_ENCODE_SET).into(),
            Self::AttrChar => percent_encode(value, ATTR_CHAR_ENCODE_SET).into(),
            Self::NoOp => value.into(),
        }
    }
}

/// 分界串：四个随机 64 位数的十六进制，用连字符连起来（与 reqwest 同一写法）。
fn gen_boundary() -> String {
    let (a, b, c, d) = (random(), random(), random(), random());
    format!("{a:016x}-{b:016x}-{c:016x}-{d:016x}")
}

/// 系统随机数；取不到时退回“当前时间 + 递增计数”：分界串只要不和内容撞上，不要求密码学强度。
fn random() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    getrandom::u64().unwrap_or_else(|_| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or_default();
        nanos ^ COUNTER.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(form: Form) -> Form {
        Form {
            boundary: "BOUNDARY".into(),
            ..form
        }
    }

    #[test]
    fn layout_matches_reqwest_byte_for_byte() {
        let form = fixed(
            Form::new()
                .text("plain", "hello")
                .part(
                    "file",
                    Part::bytes(vec![1u8, 2, 3])
                        .file_name("a \"b\".png")
                        .mime_str("image/png")
                        .unwrap(),
                )
                .text("名字", "值"),
        );
        let length = form.compute_length();
        let body = form.into_bytes();
        let expected = concat!(
            "--BOUNDARY\r\nContent-Disposition: form-data; name=\"plain\"\r\n\r\nhello\r\n",
            "--BOUNDARY\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a \\\"b\\\".png\"\r\n",
            "Content-Type: image/png\r\n\r\n\u{1}\u{2}\u{3}\r\n",
            "--BOUNDARY\r\nContent-Disposition: form-data; name*=utf-8''%E5%90%8D%E5%AD%97\r\n\r\n值\r\n",
            "--BOUNDARY--\r\n"
        );
        assert_eq!(std::str::from_utf8(&body).unwrap(), expected);
        assert_eq!(length, body.len() as u64);
    }

    #[test]
    fn an_empty_form_is_an_empty_body() {
        let form = Form::new();
        assert_eq!(form.compute_length(), 0);
        assert!(form.into_bytes().is_empty());
    }

    #[test]
    fn boundaries_look_like_reqwest_and_differ() {
        let a = Form::new();
        let b = Form::new();
        assert_eq!(a.boundary().len(), 16 * 4 + 3);
        assert!(
            a.boundary()
                .chars()
                .all(|c| c == '-' || c.is_ascii_hexdigit())
        );
        assert_ne!(a.boundary(), b.boundary());
    }

    #[test]
    fn a_bad_mime_is_a_builder_error() {
        assert!(
            Part::text("x")
                .mime_str("not a mime")
                .unwrap_err()
                .is_builder()
        );
    }

    #[test]
    fn custom_part_headers_follow_the_type() {
        let mut headers = HeaderMap::new();
        headers.insert("x-tag", "1".parse().unwrap());
        let form = fixed(Form::new().part("f", Part::text("v").headers(headers)));
        let body = form.into_bytes();
        assert!(
            std::str::from_utf8(&body)
                .unwrap()
                .contains("name=\"f\"\r\nx-tag: 1\r\n\r\nv")
        );
    }
}
