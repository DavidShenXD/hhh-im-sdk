//! 能当请求地址的类型（对应 `reqwest::IntoUrl`）：`&str`、`String`、`&String`、`Url`。

use url::Url;

use super::error::{self, Result};

/// 能当请求地址的类型。不能在外面再实现它（与 reqwest 一样是封住的）。
pub trait IntoUrl: sealed::IntoUrlSealed {}

impl IntoUrl for Url {}
impl IntoUrl for String {}
impl IntoUrl for &str {}
impl IntoUrl for &String {}

pub(crate) mod sealed {
    use super::*;

    pub trait IntoUrlSealed {
        fn into_url(self) -> Result<Url>;
    }

    impl IntoUrlSealed for Url {
        fn into_url(self) -> Result<Url> {
            if self.has_host() {
                Ok(self)
            } else {
                Err(error::url_bad_scheme(self))
            }
        }
    }

    impl IntoUrlSealed for &str {
        fn into_url(self) -> Result<Url> {
            Url::parse(self).map_err(error::builder)?.into_url()
        }
    }

    impl IntoUrlSealed for &String {
        fn into_url(self) -> Result<Url> {
            self.as_str().into_url()
        }
    }

    impl IntoUrlSealed for String {
        fn into_url(self) -> Result<Url> {
            self.as_str().into_url()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sealed::IntoUrlSealed as _;

    #[test]
    fn parses_strings_and_rejects_urls_without_host() {
        assert_eq!(
            "https://example.com/a".into_url().unwrap().as_str(),
            "https://example.com/a"
        );
        assert!("not a url".into_url().unwrap_err().is_builder());
        let error = "mailto:someone@example.com".into_url().unwrap_err();
        assert!(error.is_builder());
        assert_eq!(error.url().map(|u| u.scheme()), Some("mailto"));
    }
}
