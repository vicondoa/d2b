//! Per-VM Cloud Hypervisor stats
//! scraper folded into the daemon's `/metrics` endpoint.
//!
//! Background. The legacy `nixling-ch-exporter.service` was a host
//! singleton bash daemon that polled every VM's Cloud Hypervisor API
//! socket (`/api/v1/vmm.ping`, `/api/v1/vm.info`) on a 10s loop,
//! cached the result to a temp file, and served it as Prometheus
//! text on `127.0.0.1:9101`. This module replaces that data path:
//! the nixlingd daemon already owns a public socket and an HTTP
//! request loop, so the scrape runs synchronously per `/metrics`
//! request and the singleton service can be retired.
//!
//! Cardinality contract. By default only `{vm, env, role}` labels
//! are emitted, matching `docs/reference/loki-label-contract.md`
//! (the bridge/tap/graphics/tpm/audio topology axes are dropped
//! and only re-enabled via a future operator opt-in). The same
//! three names the legacy exporter exposed are preserved verbatim:
//! `nixling_vm_ch_api_up`, `nixling_vm_state`, `nixling_vm_running`.
//! Two new gauges describe the CH-reported VM shape:
//! `nixling_vm_ch_vcpu_count` and `nixling_vm_ch_memory_bytes`.
//!
//! Why a separate module from `metrics.rs`. The CH stats are
//! discovered per scrape from an external surface (the per-VM
//! Cloud Hypervisor JSON API), not produced by daemon internals.
//! Keeping them outside `METRIC_INVENTORY` keeps the static
//! `tests/daemon-metrics-eval.sh` gate scoped to daemon-internal
//! metrics; the CH series are documented inline here.

use std::path::Path;
use std::time::Duration;

use crate::ch_api;

/// One VM the scraper should attempt to query on a scrape cycle.
/// Built from the host manifest. Only the three label values
/// (`vm`, `env`, `role`) and the API socket path are needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChVmInput {
    pub vm: String,
    pub env: String,
    pub role: String,
    pub api_socket: String,
}

/// Outcome of one VM's CH stats scrape. Every field is optional
/// because each step of the scrape may fail independently (socket
/// missing, ping fails, info parse fails). Renderers translate
/// these into the documented metric series.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChVmStats {
    pub api_up: bool,
    pub state: Option<String>,
    pub vcpu_count: Option<u64>,
    pub memory_bytes: Option<u64>,
}

/// Source that produces a [`ChVmStats`] for a given input. The
/// daemon's `/metrics` handler invokes one of these per VM on each
/// scrape. The trait is the seam tests use to drive the renderer
/// without opening real sockets.
pub trait ChStatsSource: Send + Sync {
    fn scrape(&self, input: &ChVmInput) -> ChVmStats;
}

/// Source that does no work and always reports the VM as down.
/// Useful when the operator has disabled CH stats collection.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullChStatsSource;

impl ChStatsSource for NullChStatsSource {
    fn scrape(&self, _input: &ChVmInput) -> ChVmStats {
        ChVmStats::default()
    }
}

/// Predicate that decides whether the host currently considers a
/// VM "running". The legacy exporter checked
/// `systemctl is-active microvm@<vm>.service` (or the gpu sidecar
/// for graphics VMs); the daemon supplies its own equivalent via
/// this trait so the renderer remains pure.
pub trait VmRunningProbe: Send + Sync {
    fn is_running(&self, vm: &str) -> bool;
}

impl<F> VmRunningProbe for F
where
    F: Fn(&str) -> bool + Send + Sync,
{
    fn is_running(&self, vm: &str) -> bool {
        self(vm)
    }
}

/// Known Cloud Hypervisor lifecycle states that are always
/// emitted even when no VM is in that state. Mirrors the legacy
/// exporter's `KNOWN_STATES` array so dashboards keep working.
pub const KNOWN_STATES: &[&str] = &["Created", "Running", "Shutdown", "Paused"];

/// HTTP read timeout for a single Cloud Hypervisor API request.
/// Matches the legacy exporter's `curl --max-time 5`.
const CH_HTTP_TIMEOUT: Duration = Duration::from_secs(5);

/// Real source: speaks minimal HTTP/1.0 over the VM's
/// Cloud Hypervisor unix socket. Used in production; tests use
/// custom `ChStatsSource` impls instead.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnixSocketChStatsSource;

impl ChStatsSource for UnixSocketChStatsSource {
    fn scrape(&self, input: &ChVmInput) -> ChVmStats {
        let mut out = ChVmStats::default();
        let socket = Path::new(&input.api_socket);
        if !socket.exists() {
            return out;
        }
        if ch_api::blocking_get_json(socket, "/api/v1/vmm.ping", CH_HTTP_TIMEOUT).is_err() {
            return out;
        }
        out.api_up = true;
        match ch_api::blocking_get_json(socket, "/api/v1/vm.info", CH_HTTP_TIMEOUT) {
            Ok(body) => match ch_api::parse_vm_info(&body) {
                Ok(info) => {
                    out.state = info.state;
                    out.vcpu_count = info.vcpu_count;
                    out.memory_bytes = info.memory_mib;
                }
                Err(_) => return out,
            },
            Err(_) => return out,
        }
        out
    }
}

/// Render the full Prometheus text-format block for a set of VMs.
/// The output is appended to the daemon's `/metrics` response by
/// [`metrics_handler_with_ch_stats`]. Output is deterministic:
/// VMs are emitted in the slice order supplied by the caller, and
/// the metric families are emitted in a fixed sequence.
pub fn render_ch_stats(
    vms: &[ChVmInput],
    source: &dyn ChStatsSource,
    running: &dyn VmRunningProbe,
) -> String {
    let mut out = String::new();

    out.push_str(
        "# HELP nixling_vm_ch_api_up Whether the VM Cloud Hypervisor API responded to /vmm.ping.\n",
    );
    out.push_str("# TYPE nixling_vm_ch_api_up gauge\n");
    out.push_str(
        "# HELP nixling_vm_state Cloud Hypervisor VM state exported as a one-hot gauge per state label.\n",
    );
    out.push_str("# TYPE nixling_vm_state gauge\n");
    out.push_str(
        "# HELP nixling_vm_running Whether the host currently considers the VM running.\n",
    );
    out.push_str("# TYPE nixling_vm_running gauge\n");
    out.push_str(
        "# HELP nixling_vm_ch_vcpu_count Boot vCPU count reported by Cloud Hypervisor vm.info.\n",
    );
    out.push_str("# TYPE nixling_vm_ch_vcpu_count gauge\n");
    out.push_str("# HELP nixling_vm_ch_memory_bytes Configured guest memory in bytes reported by Cloud Hypervisor vm.info.\n");
    out.push_str("# TYPE nixling_vm_ch_memory_bytes gauge\n");

    // Cache per-(vm,env,role) stats first so the multi-line state
    // block can iterate without redundant scrape calls.
    let scrapes: Vec<(&ChVmInput, ChVmStats)> = vms
        .iter()
        .map(|input| (input, source.scrape(input)))
        .collect();

    for (input, stats) in &scrapes {
        let labels = base_labels(input);
        let api_up = if stats.api_up { 1 } else { 0 };
        out.push_str(&format!("nixling_vm_ch_api_up{{{labels}}} {api_up}\n"));

        let mut states: Vec<&str> = KNOWN_STATES.to_vec();
        if let Some(s) = stats.state.as_deref()
            && !states.contains(&s)
        {
            states.push(s);
        }
        for st in &states {
            let v = if Some(*st) == stats.state.as_deref() {
                1
            } else {
                0
            };
            let escaped = escape_label_value(st);
            out.push_str(&format!(
                "nixling_vm_state{{{labels},state=\"{escaped}\"}} {v}\n"
            ));
        }

        let running_v = if running.is_running(&input.vm) { 1 } else { 0 };
        out.push_str(&format!("nixling_vm_running{{{labels}}} {running_v}\n"));

        if let Some(vcpus) = stats.vcpu_count {
            out.push_str(&format!("nixling_vm_ch_vcpu_count{{{labels}}} {vcpus}\n"));
        }
        if let Some(memory) = stats.memory_bytes {
            out.push_str(&format!(
                "nixling_vm_ch_memory_bytes{{{labels}}} {memory}\n"
            ));
        }
    }

    out
}

/// Build the canonical `vm=...,env=...,role=...` label triple in
/// the documented order. Values are quoted + escaped per the
/// Prometheus text format.
fn base_labels(input: &ChVmInput) -> String {
    format!(
        "vm=\"{}\",env=\"{}\",role=\"{}\"",
        escape_label_value(&input.vm),
        escape_label_value(&input.env),
        escape_label_value(&input.role)
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct StubSource(Mutex<HashMap<String, ChVmStats>>);

    impl StubSource {
        fn new(entries: &[(&str, ChVmStats)]) -> Self {
            let m = entries
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect();
            StubSource(Mutex::new(m))
        }
    }

    impl ChStatsSource for StubSource {
        fn scrape(&self, input: &ChVmInput) -> ChVmStats {
            self.0
                .lock()
                .unwrap()
                .get(&input.vm)
                .cloned()
                .unwrap_or_default()
        }
    }

    fn vm(name: &str, env: &str, role: &str) -> ChVmInput {
        ChVmInput {
            vm: name.to_owned(),
            env: env.to_owned(),
            role: role.to_owned(),
            api_socket: format!("/run/nixling/{name}.sock"),
        }
    }

    #[test]
    fn render_emits_help_and_type_headers() {
        let body = render_ch_stats(&[], &NullChStatsSource, &|_: &str| false);
        assert!(body.contains("# HELP nixling_vm_ch_api_up"));
        assert!(body.contains("# TYPE nixling_vm_ch_api_up gauge"));
        assert!(body.contains("# HELP nixling_vm_state"));
        assert!(body.contains("# TYPE nixling_vm_state gauge"));
        assert!(body.contains("# HELP nixling_vm_running"));
        assert!(body.contains("# TYPE nixling_vm_running gauge"));
        assert!(body.contains("# HELP nixling_vm_ch_vcpu_count"));
        assert!(body.contains("# HELP nixling_vm_ch_memory_bytes"));
    }

    #[test]
    fn render_uses_only_vm_env_role_labels() {
        let source = StubSource::new(&[]);
        let body = render_ch_stats(&[vm("corp-vm", "work", "workload")], &source, &|_: &str| {
            false
        });
        assert!(
            body.contains("nixling_vm_ch_api_up{vm=\"corp-vm\",env=\"work\",role=\"workload\"} 0")
        );
        // No topology labels by default.
        assert!(!body.contains("bridge="));
        assert!(!body.contains("tap="));
        assert!(!body.contains("graphics="));
        assert!(!body.contains("tpm="));
        assert!(!body.contains("audio="));
        assert!(!body.contains("usbip_yubikey="));
    }

    #[test]
    fn api_up_zero_when_socket_missing() {
        let body = render_ch_stats(
            &[vm("ghost", "work", "workload")],
            &NullChStatsSource,
            &|_: &str| false,
        );
        assert!(
            body.contains("nixling_vm_ch_api_up{vm=\"ghost\",env=\"work\",role=\"workload\"} 0")
        );
    }

    #[test]
    fn known_states_are_always_emitted_as_one_hot() {
        let source = StubSource::new(&[(
            "corp-vm",
            ChVmStats {
                api_up: true,
                state: Some("Running".to_owned()),
                vcpu_count: Some(4),
                memory_bytes: Some(2 * 1024 * 1024 * 1024),
            },
        )]);
        let body = render_ch_stats(
            &[vm("corp-vm", "work", "workload")],
            &source,
            &|name: &str| name == "corp-vm",
        );
        for st in KNOWN_STATES {
            let v = if *st == "Running" { 1 } else { 0 };
            let line = format!(
                "nixling_vm_state{{vm=\"corp-vm\",env=\"work\",role=\"workload\",state=\"{st}\"}} {v}"
            );
            assert!(body.contains(&line), "missing line: {line}\n--\n{body}");
        }
        assert!(
            body.contains("nixling_vm_running{vm=\"corp-vm\",env=\"work\",role=\"workload\"} 1")
        );
        assert!(
            body.contains(
                "nixling_vm_ch_vcpu_count{vm=\"corp-vm\",env=\"work\",role=\"workload\"} 4"
            )
        );
        assert!(body.contains(
            "nixling_vm_ch_memory_bytes{vm=\"corp-vm\",env=\"work\",role=\"workload\"} 2147483648"
        ));
    }

    #[test]
    fn unknown_state_is_appended_to_known_list() {
        let source = StubSource::new(&[(
            "weird",
            ChVmStats {
                api_up: true,
                state: Some("Crashing".to_owned()),
                vcpu_count: None,
                memory_bytes: None,
            },
        )]);
        let body = render_ch_stats(&[vm("weird", "work", "workload")], &source, &|_: &str| {
            false
        });
        assert!(body.contains(
            "nixling_vm_state{vm=\"weird\",env=\"work\",role=\"workload\",state=\"Crashing\"} 1"
        ));
        assert!(body.contains(
            "nixling_vm_state{vm=\"weird\",env=\"work\",role=\"workload\",state=\"Running\"} 0"
        ));
    }

    #[test]
    fn no_vcpu_or_memory_lines_when_missing() {
        let source = StubSource::new(&[(
            "noinfo",
            ChVmStats {
                api_up: true,
                state: None,
                vcpu_count: None,
                memory_bytes: None,
            },
        )]);
        let body = render_ch_stats(&[vm("noinfo", "work", "workload")], &source, &|_: &str| {
            false
        });
        assert!(!body.contains("nixling_vm_ch_vcpu_count{vm=\"noinfo\""));
        assert!(!body.contains("nixling_vm_ch_memory_bytes{vm=\"noinfo\""));
    }

    #[test]
    fn parse_vm_info_extracts_state_cpus_memory() {
        let body = br#"{
            "state": "Running",
            "config": {
                "cpus": {"boot_vcpus": 2, "max_vcpus": 2},
                "memory": {"size": 1073741824}
            }
        }"#;
        let info = ch_api::parse_vm_info(body).expect("parse");
        assert_eq!(info.state.as_deref(), Some("Running"));
        assert_eq!(info.vcpu_count, Some(2));
        assert_eq!(info.memory_mib, Some(1073741824));
    }

    #[test]
    fn parse_vm_info_tolerates_missing_fields() {
        let body = br#"{"state":"Shutdown"}"#;
        let info = ch_api::parse_vm_info(body).expect("parse");
        assert_eq!(info.state.as_deref(), Some("Shutdown"));
        assert_eq!(info.vcpu_count, None);
        assert_eq!(info.memory_mib, None);
    }

    #[test]
    fn split_http_body_rejects_5xx() {
        let raw = b"HTTP/1.0 500 Internal Server Error\r\n\r\noops";
        assert!(ch_api::split_http_body(raw).is_err());
    }

    #[test]
    fn split_http_body_returns_body_after_double_crlf() {
        let raw = b"HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"ok\":1}";
        let body = ch_api::split_http_body(raw).expect("body");
        assert_eq!(body, b"{\"ok\":1}");
    }

    #[test]
    fn vms_are_rendered_in_input_order() {
        let source = StubSource::new(&[
            (
                "a",
                ChVmStats {
                    api_up: true,
                    ..Default::default()
                },
            ),
            (
                "b",
                ChVmStats {
                    api_up: false,
                    ..Default::default()
                },
            ),
        ]);
        let body = render_ch_stats(
            &[vm("a", "work", "workload"), vm("b", "work", "workload")],
            &source,
            &|_: &str| false,
        );
        let pos_a = body
            .find("nixling_vm_ch_api_up{vm=\"a\"")
            .expect("a present");
        let pos_b = body
            .find("nixling_vm_ch_api_up{vm=\"b\"")
            .expect("b present");
        assert!(pos_a < pos_b);
    }
}
