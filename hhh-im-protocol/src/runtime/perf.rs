//! 性能自报（契约 provider-protocol v32 §4）：用运行时的插件升级 SDK 即有，插件代码不改。
//!
//! 宿主剖析开着时发 `perf/start`，运行时开始记：每个宿主请求的排队与处理耗时（按方法名）、向平台发了几次请求、
//! 等宿主放行读取等了多久、资源分段与上传的字节数、插件进程里 Tokio 的存活任务与繁忙度；每个间隔攒成一条
//! `perf/report` 发给宿主，绝不逐条转发。`perf/stop` 后停下、清空。没收到 `perf/start` 时每个记录点只多一次原子读取。
//!
//! 名字只用方法名与固定标签，不含会话、消息、人员编号或正文。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::task::AbortHandle;

use super::Host;
use crate::budget::{PERF_INTERVAL_MIN_MS, PERF_NAME_CHARS_MAX, PERF_SERIES_MAX, PERF_SPANS_MAX};
use crate::method::{
    PERF_HISTOGRAM_BOUNDS_MS, PerfCounter, PerfHistogram, PerfReportParams, PerfSpan,
    PerfStartParams, PerfStopParams, perf_bucket_of,
};
use crate::rpc::RpcError;

/// 多慢的一段才单独报成区间（其余只进分布）：比一帧的几倍还长，宿主追踪里看得出来。
const SPAN_THRESHOLD_MS: f64 = 50.0;

/// 序列满了以后新名字统一记成这个。
const OTHER: &str = "其他";

/// 固定标签（与宿主剖析页面的显示对应）。
pub(super) const QUEUE: &str = "排队";
pub(super) const HANDLE: &str = "处理";
pub(super) const PLATFORM_REQUESTS: &str = "平台请求";
pub(super) const PERMIT_WAIT: &str = "等待放行";
pub(super) const SEGMENT_BYTES: &str = "资源分段字节";
pub(super) const UPLOAD_BYTES: &str = "上传字节";
const ALIVE_TASKS: &str = "存活任务";
const BUSY_PERCENT: &str = "繁忙度%";

tokio::task_local! {
    // 当前宿主请求所在任务的记录器（剖析开着时才设）：分段、上传、读取放行这些不经过宿主那一头的地方据此记。
    static CURRENT: Arc<Recorder>;
}

#[derive(Default)]
struct State {
    session: Option<u64>,
    seq: u64,
    /// 本窗口起点。
    window: Option<Instant>,
    /// 会话开始时刻：窗口起止报成相对它的毫秒。
    origin: Option<Instant>,
    spans: Vec<PerfSpan>,
    histograms: BTreeMap<String, PerfHistogram>,
    counters: BTreeMap<String, u64>,
    reporter: Option<AbortHandle>,
    /// 上一次读 Tokio 繁忙时长：(读的时刻, 各工作线程繁忙时长合计)。
    busy: Option<(Instant, Duration)>,
}

/// 一个插件进程一份（放在宿主那一头里）。
#[derive(Default)]
pub(super) struct Recorder {
    on: AtomicBool,
    state: Mutex<State>,
}

fn label(name: &str) -> String {
    match name.char_indices().nth(PERF_NAME_CHARS_MAX) {
        Some((at, _)) => name[..at].to_owned(),
        None => name.to_owned(),
    }
}

impl State {
    /// 序列满了：给“其他”与收尾时补的两个 Tokio 读数留三个位置。
    fn series_full(&self, name: &str) -> bool {
        !self.histograms.contains_key(name)
            && !self.counters.contains_key(name)
            && self.histograms.len() + self.counters.len() >= PERF_SERIES_MAX - 3
    }
}

impl Recorder {
    fn lock(&self) -> MutexGuard<'_, State> {
        // 只是统计：别处带着锁崩了也照常用，不把插件拖下水。
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 剖析开着（收到了 `perf/start`、还没 `perf/stop`）。
    #[inline]
    pub(super) fn active(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    /// 记一段耗时：进分布；够慢的另报成区间。
    pub(super) fn time(&self, name: &str, started: Instant, dur: Duration) {
        if !self.active() {
            return;
        }
        let mut state = self.lock();
        let Some(window) = state.window else {
            return;
        };
        let ms = dur.as_secs_f64() * 1000.0;
        let name = if state.series_full(name) {
            OTHER.to_owned()
        } else {
            label(name)
        };
        if ms >= SPAN_THRESHOLD_MS {
            let span = PerfSpan {
                name: name.clone(),
                start_ms: started.saturating_duration_since(window).as_secs_f64() * 1000.0,
                dur_ms: ms,
            };
            if state.spans.len() < PERF_SPANS_MAX {
                state.spans.push(span);
            } else if let Some(shortest) = state
                .spans
                .iter_mut()
                .min_by(|a, b| a.dur_ms.total_cmp(&b.dur_ms))
                .filter(|s| s.dur_ms < ms)
            {
                *shortest = span;
            }
        }
        let hist = state
            .histograms
            .entry(name.clone())
            .or_insert_with(|| PerfHistogram {
                name,
                count: 0,
                sum_ms: 0.0,
                max_ms: 0.0,
                buckets: vec![0; PERF_HISTOGRAM_BOUNDS_MS.len() + 1],
            });
        hist.count += 1;
        hist.sum_ms += ms;
        hist.max_ms = hist.max_ms.max(ms);
        hist.buckets[perf_bucket_of(ms)] += 1;
    }

    /// 计数：本窗口里累加。
    pub(super) fn count(&self, name: &str, delta: u64) {
        if !self.active() || delta == 0 {
            return;
        }
        let mut state = self.lock();
        if state.session.is_none() {
            return;
        }
        let name = if state.series_full(name) {
            OTHER.to_owned()
        } else {
            label(name)
        };
        *state.counters.entry(name).or_insert(0) += delta;
    }

    /// 收尾这个窗口：攒成一条报告，开下一个窗口。
    fn take(&self) -> Option<PerfReportParams> {
        let mut state = self.lock();
        let session = state.session?;
        let now = Instant::now();
        let origin = state.origin?;
        let window = state.window.replace(now)?;
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let metrics = runtime.metrics();
            state
                .counters
                .insert(ALIVE_TASKS.into(), metrics.num_alive_tasks() as u64);
            let workers = metrics.num_workers();
            let busy: Duration = (0..workers)
                .map(|w| metrics.worker_total_busy_duration(w))
                .sum();
            if let Some((at, before)) = state.busy {
                let wall = now.saturating_duration_since(at).as_secs_f64() * workers.max(1) as f64;
                if wall > 0.0 {
                    let percent =
                        (busy.saturating_sub(before).as_secs_f64() / wall * 100.0).round();
                    state.counters.insert(BUSY_PERCENT.into(), percent as u64);
                }
            }
            state.busy = Some((now, busy));
        }
        let ms = |at: Instant| at.saturating_duration_since(origin).as_secs_f64() * 1000.0;
        let report = PerfReportParams {
            session,
            seq: state.seq,
            from_ms: ms(window),
            to_ms: ms(now),
            spans: std::mem::take(&mut state.spans),
            counters: std::mem::take(&mut state.counters)
                .into_iter()
                .map(|(name, value)| PerfCounter { name, value })
                .collect(),
            histograms: std::mem::take(&mut state.histograms)
                .into_values()
                .collect(),
        };
        state.seq += 1;
        Some(report)
    }

    /// `perf/start`：开始记，每个间隔发一条 `perf/report`；已经开着的换成新会话。
    fn start(self: &Arc<Self>, host: &Host, params: PerfStartParams) {
        let interval = Duration::from_millis(params.interval_ms.max(PERF_INTERVAL_MIN_MS));
        let mut state = self.lock();
        if let Some(old) = state.reporter.take() {
            old.abort();
        }
        let now = Instant::now();
        *state = State {
            session: Some(params.session),
            window: Some(now),
            origin: Some(now),
            ..State::default()
        };
        let (recorder, host) = (self.clone(), host.clone());
        let reporter = tokio::spawn(async move {
            let mut ticker =
                tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let Some(report) = recorder.take() else {
                    return;
                };
                host.notify(crate::Method::PerfReport.as_str(), report);
            }
        });
        state.reporter = Some(reporter.abort_handle());
        drop(state);
        self.on.store(true, Ordering::Relaxed);
        tracing::debug!(session = params.session, "宿主开了剖析，开始自报");
    }

    /// `perf/stop`：停下、清空；不是当前会话的忽略。
    fn stop(&self, session: u64) {
        let mut state = self.lock();
        if state.session != Some(session) {
            return;
        }
        self.on.store(false, Ordering::Relaxed);
        if let Some(reporter) = state.reporter.take() {
            reporter.abort();
        }
        *state = State::default();
        tracing::debug!(session, "宿主关了剖析，停止自报");
    }
}

/// 宿主发来的 `perf/start` / `perf/stop`：运行时自己回，不交给插件。
pub(super) fn handle(host: &Host, method: &str, params: Value) -> Result<Value, RpcError> {
    let recorder = host.perf();
    if method == crate::Method::PerfStart.as_str() {
        recorder.start(host, super::params::<PerfStartParams>(params)?);
    } else {
        recorder.stop(super::params::<PerfStopParams>(params)?.session);
    }
    Ok(serde_json::json!({}))
}

/// 在当前宿主请求的任务里跑（剖析开着时）：分段、上传、读取放行据此记到同一个记录器。
pub(super) async fn scoped<F: std::future::Future>(
    recorder: Option<Arc<Recorder>>,
    future: F,
) -> F::Output {
    match recorder {
        Some(recorder) => CURRENT.scope(recorder, future).await,
        None => future.await,
    }
}

/// 记在当前任务的记录器上；不在剖析开着时的宿主请求里什么都不做。
pub(super) fn time_current(name: &str, started: Instant, dur: Duration) {
    let _ = CURRENT.try_with(|recorder| recorder.time(name, started, dur));
}

/// 同 [`time_current`]，计数。
pub(super) fn count_current(name: &str, delta: u64) {
    let _ = CURRENT.try_with(|recorder| recorder.count(name, delta));
}

/// `initialize` 的结果里替插件声明 `capabilities.perf`：用运行时的插件都能自报。
pub(super) fn declare(result: &mut Value) {
    if let Some(caps) = result
        .get_mut("capabilities")
        .and_then(Value::as_object_mut)
    {
        caps.insert("perf".into(), Value::Bool(true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reports_come_once_per_interval_with_bounded_series_and_stop_ends_them() {
        let (host, mut writes) = Host::new();
        let recorder = host.perf();
        // 关着时什么都不记。
        recorder.time("处理 · ping", Instant::now(), Duration::from_millis(80));
        assert!(recorder.take().is_none());

        handle(
            &host,
            "perf/start",
            serde_json::json!({ "session": 7, "intervalMs": 100 }),
        )
        .unwrap();
        assert!(recorder.active());
        let started = Instant::now();
        recorder.time("处理 · messages/list", started, Duration::from_millis(120));
        recorder.time("处理 · messages/list", started, Duration::from_millis(3));
        recorder.count(PLATFORM_REQUESTS, 2);
        for i in 0..PERF_SERIES_MAX + 10 {
            recorder.count(&format!("c{i}"), 1);
        }
        // 间隔不足 500 毫秒按 500 算：400 毫秒时还没有报告。
        assert!(
            tokio::time::timeout(Duration::from_millis(400), writes.next())
                .await
                .is_err()
        );
        let message = tokio::time::timeout(Duration::from_secs(3), writes.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(message.method.as_deref(), Some("perf/report"));
        let report: PerfReportParams = serde_json::from_value(message.params.unwrap()).unwrap();
        assert_eq!((report.session, report.seq), (7, 0));
        assert_eq!(report.check(), Ok(()));
        assert_eq!(report.spans.len(), 1, "只有够慢的报成区间");
        let hist = &report.histograms[0];
        assert_eq!((hist.count, hist.buckets.iter().sum::<u64>()), (2, 2));
        assert!(report.counters.len() + report.histograms.len() <= PERF_SERIES_MAX);
        assert!(
            report.counters.iter().any(|c| c.name == OTHER),
            "序列满了并进“其他”"
        );

        handle(&host, "perf/stop", serde_json::json!({ "session": 7 })).unwrap();
        assert!(!recorder.active());
        recorder.count(PLATFORM_REQUESTS, 1);
        assert!(recorder.take().is_none(), "停了以后不再攒");
    }

    #[test]
    fn initialize_results_declare_perf() {
        let mut result = serde_json::json!({ "capabilities": { "messages": { "list": true } } });
        declare(&mut result);
        assert_eq!(result["capabilities"]["perf"], true);
        let mut odd = serde_json::json!({ "capabilities": null });
        declare(&mut odd);
        assert!(odd["capabilities"].is_null(), "形状不对的不碰");
    }
}
