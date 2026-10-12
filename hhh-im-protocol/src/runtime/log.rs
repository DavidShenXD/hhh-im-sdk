//! 日志（protocol-v1 §11.5）：插件里的 tracing 日志转成 `log/write` 通知交给宿主，宿主并进 HHH 的日志。
//!
//! 插件自己的日志（目标以 `own` 开头，一般是插件的包名，如 `hhh_im_slack`）从 debug 起都交；别的库（网络库这类）
//! 只交 warn 与 error，免得刷屏。插件本来就不该往日志里写凭据与消息正文；这里另把每条截到 4 KiB（§14）。

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::Value;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::Host;
use crate::budget::LOG_LINE_MAX_BYTES;
use crate::method::{LogLevel, LogWriteParams};

/// 把 tracing 事件写成 `log/write` 的一层。
struct ToHost {
    host: Host,
    own: &'static str,
}

/// 收一条事件的消息与字段。
#[derive(Default)]
struct Fields {
    message: String,
    fields: BTreeMap<String, Value>,
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let mut text = String::new();
        let _ = write!(text, "{value:?}");
        if field.name() == "message" {
            self.message = text;
        } else {
            self.fields
                .insert(field.name().to_owned(), Value::String(text));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_owned();
        } else {
            self.fields
                .insert(field.name().to_owned(), Value::String(value.to_owned()));
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.insert(field.name().to_owned(), value.into());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.insert(field.name().to_owned(), value.into());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.insert(field.name().to_owned(), value.into());
    }
}

/// 截到 `max` 字节以内（不切开一个字）。
fn truncate(text: &mut String, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

impl ToHost {
    /// 一条事件写成 `log/write` 的参数；不交的（别的库的 info、debug）为空。
    fn params(&self, event: &Event<'_>) -> Option<LogWriteParams> {
        let metadata = event.metadata();
        let ours = metadata.target().starts_with(self.own);
        let level = match *metadata.level() {
            Level::ERROR => LogLevel::Error,
            Level::WARN => LogLevel::Warn,
            Level::INFO if ours => LogLevel::Info,
            Level::DEBUG if ours => LogLevel::Debug,
            _ => return None,
        };
        let mut fields = Fields::default();
        event.record(&mut fields);
        truncate(&mut fields.message, LOG_LINE_MAX_BYTES);
        fields
            .fields
            .insert("target".into(), metadata.target().into());
        Some(LogWriteParams {
            level,
            message: fields.message,
            fields: Some(fields.fields),
        })
    }
}

impl<S: Subscriber> Layer<S> for ToHost {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if let Some(params) = self.params(event) {
            self.host.notify("log/write", params);
        }
    }
}

/// 装上：之后的 tracing 日志都交给宿主。`own` 是插件自己日志的目标前缀（一般是包名，如 `"hhh_im_slack"`）。
/// 已经装过（测试里起了好几个服务）时什么也不做。
pub fn install_log(host: &Host, own: &'static str) {
    let subscriber = tracing_subscriber::registry().with(ToHost {
        host: host.clone(),
        own,
    });
    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_are_cut_on_a_character_boundary() {
        let mut text = "字".repeat(2000);
        truncate(&mut text, LOG_LINE_MAX_BYTES);
        assert!(text.len() <= LOG_LINE_MAX_BYTES);
        assert!(text.chars().all(|c| c == '字'));
    }

    // 插件自己的 warn 带字段交给宿主；别的库的 debug 不交。
    #[tokio::test]
    async fn events_become_log_write_notifications() {
        let (host, mut writes) = Host::new();
        let subscriber = tracing_subscriber::registry().with(ToHost {
            host,
            own: "hhh_im_protocol",
        });
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(status = 401, "平台拒绝了请求");
            tracing::debug!(target: "hyper", "别的库的过程日志不交");
            tracing::debug!("自己的过程日志交");
        });
        let first = writes.next().await.unwrap();
        assert_eq!(first.method.as_deref(), Some("log/write"));
        let params = first.params.unwrap();
        assert_eq!(params["level"], "warn");
        assert_eq!(params["message"], "平台拒绝了请求");
        assert_eq!(params["fields"]["status"], 401);
        let second = writes.next().await.unwrap().params.unwrap();
        assert_eq!(second["level"], "debug");
        assert_eq!(second["message"], "自己的过程日志交");
    }
}
