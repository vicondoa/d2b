//! P3 prometheus-otlp shape: daemon metrics registry + Prometheus
//! text-format exposition for `GET /metrics` on the daemon's public
//! socket.
//!
//! Why a hand-rolled registry rather than the `prometheus` crate:
//! the daemon-only worktree has no other consumer of the crate, the
//! metric set is small and closed (see
//! `docs/reference/daemon-metrics.md`), and avoiding a new transitive
//! dependency keeps the supply-chain audit footprint minimal. The
//! exposition format we emit is the documented
//! [text-format v0.0.4](https://prometheus.io/docs/instrumenting/exposition_formats/#text-based-format)
//! that every Prometheus-compatible scraper accepts.
//!
//! The module is the canonical source of truth for the metric
//! inventory: [`METRIC_INVENTORY`] enumerates every series the
//! daemon exposes, with the same names + labels + bucket boundaries
//! as the reference doc. `tests/daemon-metrics-eval.sh` asserts
//! the two stay in lock-step.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Instant;

/// Static descriptor for one metric. Mirrors the rows in
/// `docs/reference/daemon-metrics.md` one-for-one; the eval gate
/// asserts byte-equal parity between this table and the doc.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetricDescriptor {
    pub name: &'static str,
    pub kind: MetricKind,
    pub labels: &'static [&'static str],
    /// Histogram bucket upper bounds in seconds. Empty for non-
    /// histogram metrics.
    pub buckets_seconds: &'static [f64],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricKind {
    Counter,
    Gauge,
    Histogram,
}

impl MetricKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MetricKind::Counter => "counter",
            MetricKind::Gauge => "gauge",
            MetricKind::Histogram => "histogram",
        }
    }
}

/// Histogram bucket boundaries for VM start durations (seconds).
pub const VM_START_BUCKETS_SECONDS: &[f64] =
    &[0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0, 300.0];

/// Histogram bucket boundaries for per-step host-prepare durations
/// (seconds).
pub const HOST_PREP_STEP_BUCKETS_SECONDS: &[f64] =
    &[0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0];

/// Histogram bucket boundaries for broker-request durations
/// (seconds).
pub const BROKER_REQUEST_BUCKETS_SECONDS: &[f64] =
    &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0];

/// Canonical metric inventory. The order is the order the
/// exposition format will render in.
pub const METRIC_INVENTORY: &[MetricDescriptor] = &[
    MetricDescriptor {
        name: "nixling_daemon_vm_state",
        kind: MetricKind::Gauge,
        labels: &["vm", "state"],
        buckets_seconds: &[],
    },
    MetricDescriptor {
        name: "nixling_daemon_vm_start_duration_seconds",
        kind: MetricKind::Histogram,
        labels: &["vm", "outcome"],
        buckets_seconds: VM_START_BUCKETS_SECONDS,
    },
    MetricDescriptor {
        name: "nixling_daemon_host_prep_step_duration_seconds",
        kind: MetricKind::Histogram,
        labels: &["step"],
        buckets_seconds: HOST_PREP_STEP_BUCKETS_SECONDS,
    },
    MetricDescriptor {
        name: "nixling_daemon_broker_request_total",
        kind: MetricKind::Counter,
        labels: &["op", "outcome"],
        buckets_seconds: &[],
    },
    MetricDescriptor {
        name: "nixling_daemon_broker_request_duration_seconds",
        kind: MetricKind::Histogram,
        labels: &["op"],
        buckets_seconds: BROKER_REQUEST_BUCKETS_SECONDS,
    },
    MetricDescriptor {
        name: "nixling_daemon_ownership_drift_total",
        kind: MetricKind::Counter,
        labels: &["vm"],
        buckets_seconds: &[],
    },
    MetricDescriptor {
        name: "nixling_daemon_ssh_host_key_drift_total",
        kind: MetricKind::Counter,
        labels: &["vm"],
        buckets_seconds: &[],
    },
    MetricDescriptor {
        name: "nixling_daemon_pidfd_table_size",
        kind: MetricKind::Gauge,
        labels: &[],
        buckets_seconds: &[],
    },
    MetricDescriptor {
        name: "nixling_daemon_uptime_seconds",
        kind: MetricKind::Gauge,
        labels: &[],
        buckets_seconds: &[],
    },
];

/// Lookup a descriptor by name. `None` for any unknown name —
/// callers MUST only emit metrics declared in [`METRIC_INVENTORY`].
pub fn descriptor(name: &str) -> Option<&'static MetricDescriptor> {
    METRIC_INVENTORY.iter().find(|m| m.name == name)
}

/// Owned label tuple. Stored as `(key, value)` pairs in declared
/// order so the exposition output is deterministic.
type LabelSet = Vec<(String, String)>;

#[derive(Debug, Default)]
struct HistogramSample {
    /// Cumulative count of observations per bucket. The trailing
    /// `+Inf` bucket is the total count.
    bucket_counts: Vec<u64>,
    sum: f64,
    count: u64,
}

#[derive(Debug, Default)]
struct ScalarSample {
    value: f64,
}

#[derive(Debug, Default)]
struct RegistryInner {
    counters: BTreeMap<(&'static str, LabelSet), ScalarSample>,
    gauges: BTreeMap<(&'static str, LabelSet), ScalarSample>,
    histograms: BTreeMap<(&'static str, LabelSet), HistogramSample>,
}

/// In-process metrics registry. One per daemon process.
///
/// The registry is intentionally synchronous + mutex-guarded: the
/// metric volume is low (one increment per broker request, one
/// observation per VM start) and the lock is never held across
/// `await` points.
pub struct Registry {
    inner: Mutex<RegistryInner>,
    started_at: Instant,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(RegistryInner::default()),
            started_at: Instant::now(),
        }
    }

    fn validate(name: &'static str, kind: MetricKind, labels: &[(String, String)]) {
        let d = descriptor(name).unwrap_or_else(|| panic!("unknown metric: {name}"));
        assert_eq!(d.kind, kind, "wrong metric kind for {name}");
        assert_eq!(
            d.labels.len(),
            labels.len(),
            "metric {name} expects {} labels, got {}",
            d.labels.len(),
            labels.len()
        );
        for (decl, (k, _)) in d.labels.iter().zip(labels.iter()) {
            assert_eq!(*decl, k.as_str(), "metric {name} label order mismatch");
        }
    }

    pub fn counter_inc(&self, name: &'static str, labels: &[(&str, &str)]) {
        let owned: LabelSet = labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Self::validate(name, MetricKind::Counter, &owned);
        let mut g = self.inner.lock().expect("metrics registry poisoned");
        let entry = g.counters.entry((name, owned)).or_default();
        entry.value += 1.0;
    }

    pub fn gauge_set(&self, name: &'static str, labels: &[(&str, &str)], value: f64) {
        let owned: LabelSet = labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Self::validate(name, MetricKind::Gauge, &owned);
        let mut g = self.inner.lock().expect("metrics registry poisoned");
        let entry = g.gauges.entry((name, owned)).or_default();
        entry.value = value;
    }

    pub fn histogram_observe(
        &self,
        name: &'static str,
        labels: &[(&str, &str)],
        value_seconds: f64,
    ) {
        let owned: LabelSet = labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Self::validate(name, MetricKind::Histogram, &owned);
        let d = descriptor(name).expect("validated above");
        let mut g = self.inner.lock().expect("metrics registry poisoned");
        let entry = g.histograms.entry((name, owned)).or_default();
        if entry.bucket_counts.is_empty() {
            entry.bucket_counts = vec![0u64; d.buckets_seconds.len() + 1];
        }
        for (idx, ub) in d.buckets_seconds.iter().enumerate() {
            if value_seconds <= *ub {
                entry.bucket_counts[idx] += 1;
            }
        }
        let last = entry.bucket_counts.len() - 1;
        entry.bucket_counts[last] += 1;
        entry.sum += value_seconds;
        entry.count += 1;
    }

    /// Render the entire registry as a Prometheus
    /// `text/plain; version=0.0.4` body. Uptime is computed on
    /// every render so callers do not need to refresh it.
    pub fn render(&self) -> String {
        let uptime = self.started_at.elapsed().as_secs_f64();
        let mut out = String::new();
        let g = self.inner.lock().expect("metrics registry poisoned");

        for d in METRIC_INVENTORY {
            out.push_str(&format!("# HELP {} {}\n", d.name, help_text(d.name)));
            out.push_str(&format!("# TYPE {} {}\n", d.name, d.kind.as_str()));
            match d.kind {
                MetricKind::Counter => {
                    for ((_, labels), s) in g.counters.iter().filter(|((n, _), _)| *n == d.name) {
                        out.push_str(&format!(
                            "{}{} {}\n",
                            d.name,
                            render_labels(labels),
                            render_float(s.value)
                        ));
                    }
                }
                MetricKind::Gauge => {
                    if d.name == "nixling_daemon_uptime_seconds" {
                        out.push_str(&format!(
                            "{} {}\n",
                            d.name,
                            render_float(uptime)
                        ));
                        continue;
                    }
                    for ((_, labels), s) in g.gauges.iter().filter(|((n, _), _)| *n == d.name) {
                        out.push_str(&format!(
                            "{}{} {}\n",
                            d.name,
                            render_labels(labels),
                            render_float(s.value)
                        ));
                    }
                }
                MetricKind::Histogram => {
                    for ((_, labels), h) in g.histograms.iter().filter(|((n, _), _)| *n == d.name) {
                        for (idx, ub) in d.buckets_seconds.iter().enumerate() {
                            let mut le_labels = labels.clone();
                            le_labels.push(("le".to_owned(), render_float(*ub)));
                            out.push_str(&format!(
                                "{}_bucket{} {}\n",
                                d.name,
                                render_labels(&le_labels),
                                h.bucket_counts[idx]
                            ));
                        }
                        let mut inf_labels = labels.clone();
                        inf_labels.push(("le".to_owned(), "+Inf".to_owned()));
                        out.push_str(&format!(
                            "{}_bucket{} {}\n",
                            d.name,
                            render_labels(&inf_labels),
                            h.count
                        ));
                        out.push_str(&format!(
                            "{}_sum{} {}\n",
                            d.name,
                            render_labels(labels),
                            render_float(h.sum)
                        ));
                        out.push_str(&format!(
                            "{}_count{} {}\n",
                            d.name,
                            render_labels(labels),
                            h.count
                        ));
                    }
                }
            }
        }
        out
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

fn help_text(name: &str) -> &'static str {
    match name {
        "nixling_daemon_vm_state" => "Per-VM lifecycle state (running/stopped/degraded).",
        "nixling_daemon_vm_start_duration_seconds" => {
            "Wall-clock duration of a VM start, by outcome."
        }
        "nixling_daemon_host_prep_step_duration_seconds" => {
            "Per-step duration of one host-prepare reconcile pass."
        }
        "nixling_daemon_broker_request_total" => {
            "Cumulative count of broker requests by op and outcome."
        }
        "nixling_daemon_broker_request_duration_seconds" => {
            "Round-trip latency of a single broker request."
        }
        "nixling_daemon_ownership_drift_total" => {
            "Per-VM ownership-preflight drift detections."
        }
        "nixling_daemon_ssh_host_key_drift_total" => {
            "Per-VM SSH host-key drift detections."
        }
        "nixling_daemon_pidfd_table_size" => "Live pidfd entries held by the supervisor.",
        "nixling_daemon_uptime_seconds" => "Seconds since the daemon process started.",
        _ => "",
    }
}

fn render_labels(labels: &[(String, String)]) -> String {
    if labels.is_empty() {
        return String::new();
    }
    let mut out = String::from("{");
    for (i, (k, v)) in labels.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(k);
        out.push_str("=\"");
        out.push_str(&escape_label_value(v));
        out.push('"');
    }
    out.push('}');
    out
}

fn escape_label_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for ch in v.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out
}

fn render_float(v: f64) -> String {
    if v.is_infinite() {
        return if v.is_sign_negative() {
            "-Inf".to_owned()
        } else {
            "+Inf".to_owned()
        };
    }
    if v == v.trunc() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// HTTP response for a `GET /metrics` request. Other paths return
/// `404 Not Found`; non-GET methods return `405 Method Not Allowed`.
/// The parser is intentionally minimal — the daemon's accept loop
/// gates everything else (peer creds, frame size). Returns the full
/// HTTP/1.1 response as bytes ready to write back to the client.
pub fn metrics_handler(request: &[u8], registry: &Registry) -> Vec<u8> {
    let head = request.split(|b| *b == b'\n').next().unwrap_or(&[]);
    let head = std::str::from_utf8(head).unwrap_or("");
    let mut parts = head.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    if method != "GET" {
        return http_response(405, "text/plain; charset=utf-8", "method not allowed\n");
    }
    if path != "/metrics" {
        return http_response(404, "text/plain; charset=utf-8", "not found\n");
    }

    let body = registry.render();
    http_response(200, "text/plain; version=0.0.4; charset=utf-8", &body)
}

/// Variant of [`metrics_handler`] that, after rendering the
/// daemon-internal `nixling_daemon_*` registry, appends the
/// per-VM Cloud Hypervisor stats produced by [`crate::ch_stats`].
/// P3 ph3-p3-ch-exporter-retire: this is the seam that lets the
/// daemon replace the legacy `nixling-ch-exporter.service`
/// singleton — same metric names (`nixling_vm_ch_api_up`,
/// `nixling_vm_state`, `nixling_vm_running`), bounded
/// `{vm, env, role}` cardinality, no separate listener.
pub fn metrics_handler_with_ch_stats(
    request: &[u8],
    registry: &Registry,
    ch_vms: &[crate::ch_stats::ChVmInput],
    ch_source: &dyn crate::ch_stats::ChStatsSource,
    running_probe: &dyn crate::ch_stats::VmRunningProbe,
) -> Vec<u8> {
    let head = request.split(|b| *b == b'\n').next().unwrap_or(&[]);
    let head = std::str::from_utf8(head).unwrap_or("");
    let mut parts = head.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    if method != "GET" {
        return http_response(405, "text/plain; charset=utf-8", "method not allowed\n");
    }
    if path != "/metrics" {
        return http_response(404, "text/plain; charset=utf-8", "not found\n");
    }

    let mut body = registry.render();
    body.push_str(&crate::ch_stats::render_ch_stats(
        ch_vms,
        ch_source,
        running_probe,
    ));
    http_response(200, "text/plain; version=0.0.4; charset=utf-8", &body)
}

fn http_response(status: u16, content_type: &str, body: &str) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "OK",
    };
    let mut out = Vec::with_capacity(body.len() + 128);
    out.extend_from_slice(
        format!(
            "HTTP/1.1 {status} {reason}\r\n\
             Content-Type: {content_type}\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            body.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(body.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_has_expected_names() {
        let names: Vec<&str> = METRIC_INVENTORY.iter().map(|m| m.name).collect();
        assert_eq!(
            names,
            vec![
                "nixling_daemon_vm_state",
                "nixling_daemon_vm_start_duration_seconds",
                "nixling_daemon_host_prep_step_duration_seconds",
                "nixling_daemon_broker_request_total",
                "nixling_daemon_broker_request_duration_seconds",
                "nixling_daemon_ownership_drift_total",
                "nixling_daemon_ssh_host_key_drift_total",
                "nixling_daemon_pidfd_table_size",
                "nixling_daemon_uptime_seconds",
            ]
        );
    }

    #[test]
    fn vm_state_labels() {
        let d = descriptor("nixling_daemon_vm_state").expect("vm_state");
        assert_eq!(d.kind, MetricKind::Gauge);
        assert_eq!(d.labels, &["vm", "state"]);
    }

    #[test]
    fn vm_start_buckets_are_canonical() {
        let d = descriptor("nixling_daemon_vm_start_duration_seconds").expect("vm_start");
        assert_eq!(d.buckets_seconds, VM_START_BUCKETS_SECONDS);
    }

    #[test]
    fn render_emits_help_and_type_lines() {
        let r = Registry::new();
        r.counter_inc(
            "nixling_daemon_broker_request_total",
            &[("op", "ApplyNftables"), ("outcome", "ok")],
        );
        let body = r.render();
        assert!(body.contains("# HELP nixling_daemon_broker_request_total"));
        assert!(body.contains("# TYPE nixling_daemon_broker_request_total counter"));
        assert!(body.contains(
            "nixling_daemon_broker_request_total{op=\"ApplyNftables\",outcome=\"ok\"} 1"
        ));
        assert!(body.contains("nixling_daemon_uptime_seconds "));
    }

    #[test]
    fn histogram_emits_buckets_sum_and_count() {
        let r = Registry::new();
        r.histogram_observe(
            "nixling_daemon_broker_request_duration_seconds",
            &[("op", "OpenPidfd")],
            0.03,
        );
        let body = r.render();
        assert!(body.contains(
            "nixling_daemon_broker_request_duration_seconds_bucket{op=\"OpenPidfd\",le=\"0.05\"} 1"
        ));
        assert!(body.contains(
            "nixling_daemon_broker_request_duration_seconds_bucket{op=\"OpenPidfd\",le=\"+Inf\"} 1"
        ));
        assert!(body.contains("nixling_daemon_broker_request_duration_seconds_count{op=\"OpenPidfd\"} 1"));
    }

    #[test]
    fn gauge_set_overrides() {
        let r = Registry::new();
        r.gauge_set("nixling_daemon_pidfd_table_size", &[], 3.0);
        r.gauge_set("nixling_daemon_pidfd_table_size", &[], 5.0);
        let body = r.render();
        assert!(body.contains("nixling_daemon_pidfd_table_size 5"));
        assert!(!body.contains("nixling_daemon_pidfd_table_size 3"));
    }

    #[test]
    fn metrics_handler_returns_text_format_on_get() {
        let r = Registry::new();
        let resp = metrics_handler(b"GET /metrics HTTP/1.1\r\n\r\n", &r);
        let s = std::str::from_utf8(&resp).expect("utf8 response");
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(s.contains("Content-Type: text/plain; version=0.0.4"));
        assert!(s.contains("# TYPE nixling_daemon_uptime_seconds gauge"));
    }

    #[test]
    fn metrics_handler_rejects_non_metrics_path() {
        let r = Registry::new();
        let resp = metrics_handler(b"GET /other HTTP/1.1\r\n\r\n", &r);
        let s = std::str::from_utf8(&resp).expect("utf8 response");
        assert!(s.starts_with("HTTP/1.1 404 "));
    }

    #[test]
    fn metrics_handler_rejects_non_get_method() {
        let r = Registry::new();
        let resp = metrics_handler(b"POST /metrics HTTP/1.1\r\n\r\n", &r);
        let s = std::str::from_utf8(&resp).expect("utf8 response");
        assert!(s.starts_with("HTTP/1.1 405 "));
    }

    #[test]
    #[should_panic(expected = "wrong metric kind")]
    fn validate_rejects_wrong_kind() {
        let r = Registry::new();
        r.counter_inc("nixling_daemon_pidfd_table_size", &[]);
    }

    #[test]
    #[should_panic(expected = "label order mismatch")]
    fn validate_rejects_label_misorder() {
        let r = Registry::new();
        r.counter_inc(
            "nixling_daemon_broker_request_total",
            &[("outcome", "ok"), ("op", "OpenPidfd")],
        );
    }

    #[test]
    fn metrics_handler_with_ch_stats_appends_vm_metrics() {
        use crate::ch_stats::{ChVmInput, NullChStatsSource};
        let r = Registry::new();
        let vms = vec![ChVmInput {
            vm: "corp-vm".to_owned(),
            env: "work".to_owned(),
            role: "workload".to_owned(),
            api_socket: "/run/nixling/corp-vm.sock".to_owned(),
        }];
        let resp = metrics_handler_with_ch_stats(
            b"GET /metrics HTTP/1.1\r\n\r\n",
            &r,
            &vms,
            &NullChStatsSource,
            &|_: &str| false,
        );
        let s = std::str::from_utf8(&resp).expect("utf8");
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(s.contains("# TYPE nixling_daemon_uptime_seconds gauge"));
        assert!(s.contains("# TYPE nixling_vm_ch_api_up gauge"));
        assert!(s.contains(
            "nixling_vm_ch_api_up{vm=\"corp-vm\",env=\"work\",role=\"workload\"} 0"
        ));
    }

    #[test]
    fn metrics_handler_with_ch_stats_rejects_non_metrics_path() {
        use crate::ch_stats::NullChStatsSource;
        let r = Registry::new();
        let resp = metrics_handler_with_ch_stats(
            b"GET /other HTTP/1.1\r\n\r\n",
            &r,
            &[],
            &NullChStatsSource,
            &|_: &str| false,
        );
        let s = std::str::from_utf8(&resp).expect("utf8");
        assert!(s.starts_with("HTTP/1.1 404 "));
    }
}
