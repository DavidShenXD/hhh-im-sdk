//! 请求与请求构造（对应 `reqwest::Request` / `reqwest::RequestBuilder`）。
//!
//! 构造中途出的错（请求头不合法、序列化失败等）先记着，到 `send()` / `build()` 时再返回，与 reqwest 一样。

use std::fmt;
use std::time::Duration;

use http::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, Entry, HeaderName, OccupiedEntry};
use http::{HeaderMap, HeaderValue, Method};
use serde::Serialize;
use url::Url;

use super::body::Body;
use super::client::Client;
use super::error::{self, Result};
use super::multipart;
use super::response::Response;

/// 一个请求。
pub struct Request {
    method: Method,
    url: Url,
    headers: HeaderMap,
    body: Option<Body>,
    timeout: Option<Duration>,
}

impl Request {
    /// 新请求，不带请求头与请求体。
    pub fn new(method: Method, url: Url) -> Self {
        Self {
            method,
            url,
            headers: HeaderMap::new(),
            body: None,
            timeout: None,
        }
    }

    pub fn method(&self) -> &Method {
        &self.method
    }

    pub fn method_mut(&mut self) -> &mut Method {
        &mut self.method
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn url_mut(&mut self) -> &mut Url {
        &mut self.url
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    pub fn body(&self) -> Option<&Body> {
        self.body.as_ref()
    }

    pub fn body_mut(&mut self) -> &mut Option<Body> {
        &mut self.body
    }

    /// 本请求自己的整体超时（盖过客户端的）。
    pub fn timeout(&self) -> Option<&Duration> {
        self.timeout.as_ref()
    }

    pub fn timeout_mut(&mut self) -> &mut Option<Duration> {
        &mut self.timeout
    }

    /// 复制一份（iPhone 版请求体总能复制，所以总是 `Some`）。
    pub fn try_clone(&self) -> Option<Self> {
        Some(Self {
            method: self.method.clone(),
            url: self.url.clone(),
            headers: self.headers.clone(),
            body: self.body.clone(),
            timeout: self.timeout,
        })
    }

    pub(crate) fn into_parts(self) -> (Method, Url, HeaderMap, Option<Body>, Option<Duration>) {
        (self.method, self.url, self.headers, self.body, self.timeout)
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("url", &self.url.as_str())
            .field("headers", &self.headers)
            .finish()
    }
}

/// 一步步拼请求，最后 [`send`](Self::send)。
#[must_use = "RequestBuilder 不发出去就什么都不做"]
pub struct RequestBuilder {
    client: Client,
    request: Result<Request>,
}

impl RequestBuilder {
    pub(crate) fn new(client: Client, request: Result<Request>) -> Self {
        let mut builder = Self { client, request };
        let auth = builder
            .request
            .as_mut()
            .ok()
            .and_then(|request| extract_authority(&mut request.url));
        match auth {
            Some((username, password)) => builder.basic_auth(username, password),
            None => builder,
        }
    }

    /// 用已有的客户端与请求接着拼。
    pub fn from_parts(client: Client, request: Request) -> Self {
        Self {
            client,
            request: Ok(request),
        }
    }

    /// 加一个请求头（同名的不覆盖，追加）。
    pub fn header<K, V>(self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<http::Error>,
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        self.header_sensitive(key, value, false)
    }

    fn header_sensitive<K, V>(mut self, key: K, value: V, sensitive: bool) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<http::Error>,
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        let mut failure = None;
        if let Ok(request) = &mut self.request {
            match <HeaderName as TryFrom<K>>::try_from(key) {
                Ok(key) => match <HeaderValue as TryFrom<V>>::try_from(value) {
                    Ok(mut value) => {
                        // 只会把普通的标成敏感，不会反过来。
                        if sensitive {
                            value.set_sensitive(true);
                        }
                        request.headers.append(key, value);
                    }
                    Err(e) => failure = Some(error::builder(e.into())),
                },
                Err(e) => failure = Some(error::builder(e.into())),
            }
        }
        if let Some(failure) = failure {
            self.request = Err(failure);
        }
        self
    }

    /// 合进一组请求头：同名的整组替换。
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        if let Ok(request) = &mut self.request {
            replace_headers(&mut request.headers, headers);
        }
        self
    }

    /// `Authorization: Basic …`（标为敏感）。
    pub fn basic_auth<U, P>(self, username: U, password: Option<P>) -> Self
    where
        U: fmt::Display,
        P: fmt::Display,
    {
        let header_value = basic_auth(username, password);
        self.header_sensitive(AUTHORIZATION, header_value, true)
    }

    /// `Authorization: Bearer …`（标为敏感）。
    pub fn bearer_auth<T>(self, token: T) -> Self
    where
        T: fmt::Display,
    {
        let header_value = format!("Bearer {token}");
        self.header_sensitive(AUTHORIZATION, header_value, true)
    }

    /// 请求体。
    pub fn body<T: Into<Body>>(mut self, body: T) -> Self {
        if let Ok(request) = &mut self.request {
            request.body = Some(body.into());
        }
        self
    }

    /// 本请求的整体超时（从发出到读完响应体），盖过客户端的。
    pub fn timeout(mut self, timeout: Duration) -> Self {
        if let Ok(request) = &mut self.request {
            request.timeout = Some(timeout);
        }
        self
    }

    /// `multipart/form-data` 表单：设好 `Content-Type`（带分界串）与 `Content-Length`。
    pub fn multipart(self, form: multipart::Form) -> Self {
        let mut builder = self.header(
            CONTENT_TYPE,
            format!("multipart/form-data; boundary={}", form.boundary()).as_str(),
        );
        builder = builder.header(CONTENT_LENGTH, form.compute_length());
        if let Ok(request) = &mut builder.request {
            request.body = Some(Body::from(form.into_bytes()));
        }
        builder
    }

    /// 把查询参数追加到地址后面（`serde_urlencoded` 编码）。
    pub fn query<T: Serialize + ?Sized>(mut self, query: &T) -> Self {
        let mut failure = None;
        if let Ok(request) = &mut self.request {
            let url = &mut request.url;
            let mut pairs = url.query_pairs_mut();
            let serializer = serde_urlencoded::Serializer::new(&mut pairs);
            if let Err(e) = query.serialize(serializer) {
                failure = Some(error::builder(e));
            }
        }
        if let Ok(request) = &mut self.request
            && let Some("") = request.url.query()
        {
            request.url.set_query(None);
        }
        if let Some(failure) = failure {
            self.request = Err(failure);
        }
        self
    }

    /// 表单请求体（`application/x-www-form-urlencoded`，已设类型时不改）。
    pub fn form<T: Serialize + ?Sized>(mut self, form: &T) -> Self {
        let mut failure = None;
        if let Ok(request) = &mut self.request {
            match serde_urlencoded::to_string(form) {
                Ok(body) => {
                    request
                        .headers
                        .entry(CONTENT_TYPE)
                        .or_insert(HeaderValue::from_static(
                            "application/x-www-form-urlencoded",
                        ));
                    request.body = Some(body.into());
                }
                Err(e) => failure = Some(error::builder(e)),
            }
        }
        if let Some(failure) = failure {
            self.request = Err(failure);
        }
        self
    }

    /// JSON 请求体（`application/json`，已设类型时不改）。
    pub fn json<T: Serialize + ?Sized>(mut self, json: &T) -> Self {
        let mut failure = None;
        if let Ok(request) = &mut self.request {
            match serde_json::to_vec(json) {
                Ok(body) => {
                    if !request.headers.contains_key(CONTENT_TYPE) {
                        request
                            .headers
                            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
                    }
                    request.body = Some(body.into());
                }
                Err(e) => failure = Some(error::builder(e)),
            }
        }
        if let Some(failure) = failure {
            self.request = Err(failure);
        }
        self
    }

    /// 拼好但不发。
    ///
    /// # Errors
    ///
    /// 拼的过程中出过错时返回那个错误。
    pub fn build(self) -> Result<Request> {
        self.request
    }

    /// 拼好但不发，连同客户端一起交回。
    pub fn build_split(self) -> (Client, Result<Request>) {
        (self.client, self.request)
    }

    /// 发出去，等响应头到达。
    ///
    /// # Errors
    ///
    /// 拼的过程中出过错、连不上、超时、跳转出错时返回错误。
    pub async fn send(self) -> Result<Response> {
        let request = self.request?;
        self.client.execute(request).await
    }

    /// 复制一份（iPhone 版总能复制，构造中途出过错时返回 `None`）。
    pub fn try_clone(&self) -> Option<Self> {
        let request = self.request.as_ref().ok()?.try_clone()?;
        Some(Self {
            client: self.client.clone(),
            request: Ok(request),
        })
    }
}

impl fmt::Debug for RequestBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.request {
            Ok(request) => request.fmt(f),
            Err(error) => f.debug_tuple("RequestBuilder").field(error).finish(),
        }
    }
}

/// `Basic base64(用户名:密码)`，标为敏感。
fn basic_auth<U, P>(username: U, password: Option<P>) -> HeaderValue
where
    U: fmt::Display,
    P: fmt::Display,
{
    use base64::Engine as _;
    let mut plain = format!("{username}:");
    if let Some(password) = password {
        plain.push_str(&password.to_string());
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(plain);
    let mut header =
        HeaderValue::try_from(format!("Basic {encoded}")).expect("base64 总是合法的请求头内容");
    header.set_sensitive(true);
    header
}

/// 把地址里的账号密码取出来（改用 `Authorization` 头发），地址里去掉它们。
fn extract_authority(url: &mut Url) -> Option<(String, Option<String>)> {
    use percent_encoding::percent_decode;
    if !url.has_authority() {
        return None;
    }
    let username: String = percent_decode(url.username().as_bytes())
        .decode_utf8()
        .ok()?
        .into();
    let password = url.password().and_then(|password| {
        percent_decode(password.as_bytes())
            .decode_utf8()
            .ok()
            .map(String::from)
    });
    if username.is_empty() && password.is_none() {
        return None;
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    Some((username, password))
}

/// 同名的请求头整组替换，其余保留。
fn replace_headers(dst: &mut HeaderMap, src: HeaderMap) {
    // HeaderMap 的 into_iter 对同名的多个值只在第一个给出名字，后面给 None。
    let mut previous: Option<OccupiedEntry<'_, HeaderValue>> = None;
    for (key, value) in src {
        match key {
            Some(key) => match dst.entry(key) {
                Entry::Occupied(mut entry) => {
                    entry.insert(value);
                    previous = Some(entry);
                }
                Entry::Vacant(entry) => {
                    previous = Some(entry.insert_entry(value));
                }
            },
            None => {
                if let Some(entry) = &mut previous {
                    entry.append(value);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> Client {
        Client::new()
    }

    #[test]
    fn query_appends_and_drops_an_empty_query() {
        let request = client()
            .get("https://example.com/a?x=1")
            .query(&[("b", "c d"), ("e", "中")])
            .build()
            .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://example.com/a?x=1&b=c+d&e=%E4%B8%AD"
        );
        let empty: [(&str, &str); 0] = [];
        let request = client()
            .get("https://example.com/a")
            .query(&empty)
            .build()
            .unwrap();
        assert_eq!(request.url().as_str(), "https://example.com/a");
    }

    #[test]
    fn auth_headers_are_sensitive() {
        let request = client()
            .get("https://example.com/")
            .bearer_auth("token")
            .build()
            .unwrap();
        let value = request.headers().get(AUTHORIZATION).unwrap();
        assert_eq!(value, "Bearer token");
        assert!(value.is_sensitive());
        let request = client()
            .get("https://example.com/")
            .basic_auth("user", Some("pass"))
            .build()
            .unwrap();
        assert_eq!(request.headers()[AUTHORIZATION], "Basic dXNlcjpwYXNz");
        let request = client()
            .get("https://example.com/")
            .basic_auth("user", None::<&str>)
            .build()
            .unwrap();
        assert_eq!(request.headers()[AUTHORIZATION], "Basic dXNlcjo=");
    }

    #[test]
    fn credentials_in_the_url_become_basic_auth() {
        let request = client()
            .get("https://us%20er:p%40ss@example.com/x")
            .build()
            .unwrap();
        assert_eq!(request.url().as_str(), "https://example.com/x");
        assert_eq!(request.headers()[AUTHORIZATION], "Basic dXMgZXI6cEBzcw==");
    }

    #[test]
    fn json_and_form_keep_an_explicit_content_type() {
        let request = client()
            .post("https://example.com/")
            .header(CONTENT_TYPE, "application/vnd+json")
            .json(&serde_json::json!({"a": 1}))
            .build()
            .unwrap();
        assert_eq!(request.headers()[CONTENT_TYPE], "application/vnd+json");
        assert_eq!(request.body().unwrap().as_bytes(), Some(&br#"{"a":1}"#[..]));
        let request = client()
            .post("https://example.com/")
            .form(&[("a", "b c")])
            .build()
            .unwrap();
        assert_eq!(
            request.headers()[CONTENT_TYPE],
            "application/x-www-form-urlencoded"
        );
        assert_eq!(request.body().unwrap().as_bytes(), Some(&b"a=b+c"[..]));
    }

    #[test]
    fn errors_wait_until_send_or_build() {
        let error = client()
            .get("https://example.com/")
            .header("bad header", "v")
            .bearer_auth("t")
            .build()
            .unwrap_err();
        assert!(error.is_builder());
        assert!(client().get("not a url").build().unwrap_err().is_builder());
    }

    #[test]
    fn headers_replace_by_name() {
        let mut first = HeaderMap::new();
        first.append("x-a", "1".parse().unwrap());
        first.append("x-a", "2".parse().unwrap());
        first.insert("x-b", "keep".parse().unwrap());
        let mut second = HeaderMap::new();
        second.append("x-a", "3".parse().unwrap());
        second.append("x-a", "4".parse().unwrap());
        let request = client()
            .get("https://example.com/")
            .headers(first)
            .headers(second)
            .build()
            .unwrap();
        let values: Vec<_> = request.headers().get_all("x-a").iter().collect();
        assert_eq!(values, ["3", "4"]);
        assert_eq!(request.headers()["x-b"], "keep");
    }

    #[test]
    fn multipart_sets_type_and_length() {
        let form = multipart::Form::new().text("a", "b");
        let boundary = form.boundary().to_owned();
        let request = client()
            .post("https://example.com/")
            .multipart(form)
            .build()
            .unwrap();
        assert_eq!(
            request.headers()[CONTENT_TYPE],
            format!("multipart/form-data; boundary={boundary}").as_str()
        );
        let length: usize = request.headers()[CONTENT_LENGTH]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(length, request.body().unwrap().as_bytes().unwrap().len());
    }
}
