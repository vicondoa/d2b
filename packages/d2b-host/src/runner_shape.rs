//! Runner-shape preflight + CH net-handoff probe.
//!
//! The host check consumes `host.json`, `processes.json`, and
//! `closures/<vm>.json` runner-parity data and validates them **without**
//! launching CH. The same module derives the recorded net-handoff mode
//! (`tap-fd` preferred, `persistent-tap` fallback) for later consumers.
//!
//! Everything in this module is pure: real-host wiring lives in the
//! daemon/CLI, and the L1c canary
//! (`tests/runner-shape-preflight.sh`) drives the deterministic
//! analyzers against the golden fixtures under
//! `tests/golden/runner-shape/`.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// CH net-handoff mode recorded in `host.json` under
/// `host.ch.netHandoffMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetHandoffMode {
    /// Broker opens TAP + `/dev/vhost-net` and passes fds via
    /// `SCM_RIGHTS`; runner has no `CAP_NET_ADMIN`. Preferred.
    TapFd,
    /// Broker creates a persistent TAP with `TUNSETOWNER`/`TUNSETGROUP`
    /// for the runner uid/gid; runner mounts the device node
    /// read-only.
    PersistentTap,
}

impl NetHandoffMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TapFd => "tap-fd",
            Self::PersistentTap => "persistent-tap",
        }
    }
}

/// Why the CH net-handoff probe selected (or failed to select) a
/// mode. The error variants are surfaced to `host check` as
/// `ch-net-handoff-not-supported`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetHandoffProbeError {
    /// Neither `tap-fd` nor `persistent-tap` is supported by the
    /// packaged CH binary.
    NeitherSupported,
    /// Only modes that require `CAP_NET_ADMIN` in the runner are
    /// available. Long-lived runners must not hold `CAP_NET_ADMIN`.
    RequiresCapNetAdmin,
}

/// Outcome of `probe_ch_net_handoff_mode`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetHandoffProbeOutcome {
    Selected {
        mode: NetHandoffMode,
        evidence: Vec<String>,
    },
    Failed(NetHandoffProbeError),
}

/// Probes the CH net-handoff support surface from `--help` argv output.
///
/// Prefer `tap-fd`. If `--net` accepts the `fd=` form (the CH argv
/// parser exposes this as `fd=<n>` in `--net help`), `tap-fd` is
/// selected. Otherwise the older `tap=<name>` form is selected as
/// `persistent-tap`. If neither token appears, the probe fails closed.
pub fn probe_ch_net_handoff_mode(ch_help_text: &str) -> NetHandoffProbeOutcome {
    let mut evidence = Vec::new();
    let supports_fd = ch_help_text.contains("fd=<") || ch_help_text.contains("fd=FD");
    let supports_tap = ch_help_text.contains("tap=<") || ch_help_text.contains("tap=TAP");
    if supports_fd {
        evidence.push("ch --net accepts fd=<n>".to_owned());
        NetHandoffProbeOutcome::Selected {
            mode: NetHandoffMode::TapFd,
            evidence,
        }
    } else if supports_tap {
        evidence.push("ch --net accepts tap=<name>".to_owned());
        if ch_help_text.contains("requires CAP_NET_ADMIN") {
            return NetHandoffProbeOutcome::Failed(NetHandoffProbeError::RequiresCapNetAdmin);
        }
        NetHandoffProbeOutcome::Selected {
            mode: NetHandoffMode::PersistentTap,
            evidence,
        }
    } else {
        NetHandoffProbeOutcome::Failed(NetHandoffProbeError::NeitherSupported)
    }
}

/// Inputs to [`runner_shape_preflight`]. Each field is a serialized
/// blob from the trusted bundle; using strings keeps the boundary thin
/// and lets the CLI use either DTOs or the on-disk file paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerShapePreflightInput {
    /// CH capabilities advertised by `host.json`'s
    /// `cloudHypervisorCapabilities` rows (capability name list).
    pub ch_capabilities_declared: Vec<String>,
    /// CH capabilities the packaged binary actually supports (probed
    /// from `ch --help` / `ch --version --json`).
    pub ch_capabilities_packaged: Vec<String>,
    /// Per-VM `declaredRunner` argv snapshot text (from
    /// `closures/<vm>.json`).
    pub declared_runner_argv: Vec<DeclaredRunnerEntry>,
    /// CH API socket path declarations from `host.json`.
    pub ch_api_socket_paths: Vec<ChApiSocket>,
    /// vsock transport entries; Unix-socket-backed vsock only.
    pub vsock_transports: Vec<VsockTransport>,
    /// virtiofsd / swtpm sidecar declarations cross-checked against
    /// the `processes.json` DAG.
    pub sidecar_nodes: Vec<SidecarNode>,
    /// `processes.json` DAG node ids the preflight expects (subset
    /// check; mismatch surfaces `runner-shape-drift`).
    pub processes_dag_node_ids: Vec<String>,
}

/// Single `declaredRunner` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclaredRunnerEntry {
    pub vm: String,
    pub argv_hash: String,
}

/// CH API socket path expectation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChApiSocket {
    pub vm: String,
    pub path: String,
    pub mode: u32,
    pub owner: String,
}

/// vsock transport expectation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VsockTransport {
    pub vm: String,
    pub transport: String,
}

/// virtiofsd / swtpm sidecar expectation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarNode {
    pub vm: String,
    pub kind: String,
    pub dag_node_id: String,
}

/// Aggregate findings for the runner-shape preflight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerShapeReport {
    pub findings: Vec<RunnerShapeFinding>,
    pub net_handoff_outcome: Option<NetHandoffProbeOutcome>,
}

impl RunnerShapeReport {
    pub fn fail_closed(&self) -> bool {
        self.findings.iter().any(|f| {
            matches!(
                f.kind,
                RunnerShapeKind::RunnerShapeDrift | RunnerShapeKind::Missing
            )
        })
    }
}

/// Finding classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerShapeKind {
    Ok,
    Missing,
    RunnerShapeDrift,
    CapabilityMismatch,
    ApiSocketPathOwnershipMismatch,
    VsockTransportInvalid,
    SidecarDagMismatch,
}

/// Per-row finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerShapeFinding {
    pub kind: RunnerShapeKind,
    pub subject: String,
    pub detail: String,
}

/// Runs the deterministic preflight. Pure: inputs in, findings out.
pub fn runner_shape_preflight(
    input: &RunnerShapePreflightInput,
    net_probe: Option<NetHandoffProbeOutcome>,
) -> RunnerShapeReport {
    let mut findings = Vec::new();

    let declared: BTreeSet<&str> = input
        .ch_capabilities_declared
        .iter()
        .map(String::as_str)
        .collect();
    let packaged: BTreeSet<&str> = input
        .ch_capabilities_packaged
        .iter()
        .map(String::as_str)
        .collect();
    for missing in declared.difference(&packaged) {
        findings.push(RunnerShapeFinding {
            kind: RunnerShapeKind::CapabilityMismatch,
            subject: (*missing).to_owned(),
            detail: "host.json declared CH capability is absent from packaged binary".to_owned(),
        });
    }

    if input.declared_runner_argv.is_empty() {
        findings.push(RunnerShapeFinding {
            kind: RunnerShapeKind::Missing,
            subject: "declaredRunner".to_owned(),
            detail: "no declaredRunner parity snapshots present".to_owned(),
        });
    }

    for entry in &input.declared_runner_argv {
        if entry.argv_hash.is_empty() {
            findings.push(RunnerShapeFinding {
                kind: RunnerShapeKind::RunnerShapeDrift,
                subject: entry.vm.clone(),
                detail: "declaredRunner argv hash is empty".to_owned(),
            });
        }
    }

    for sock in &input.ch_api_socket_paths {
        if sock.mode & 0o777 != 0o660 {
            findings.push(RunnerShapeFinding {
                kind: RunnerShapeKind::ApiSocketPathOwnershipMismatch,
                subject: sock.vm.clone(),
                detail: format!(
                    "CH API socket {} mode {:o} != 0660",
                    sock.path,
                    sock.mode & 0o777
                ),
            });
        }
        if sock.owner.is_empty() {
            findings.push(RunnerShapeFinding {
                kind: RunnerShapeKind::ApiSocketPathOwnershipMismatch,
                subject: sock.vm.clone(),
                detail: format!("CH API socket {} has empty owner", sock.path),
            });
        }
    }

    for vt in &input.vsock_transports {
        if vt.transport != "unix" {
            findings.push(RunnerShapeFinding {
                kind: RunnerShapeKind::VsockTransportInvalid,
                subject: vt.vm.clone(),
                detail: format!(
                    "vsock transport {:?} must be unix-socket-backed",
                    vt.transport
                ),
            });
        }
    }

    let dag_ids: BTreeSet<&str> = input
        .processes_dag_node_ids
        .iter()
        .map(String::as_str)
        .collect();
    for side in &input.sidecar_nodes {
        if !dag_ids.contains(side.dag_node_id.as_str()) {
            findings.push(RunnerShapeFinding {
                kind: RunnerShapeKind::SidecarDagMismatch,
                subject: format!("{}:{}", side.vm, side.kind),
                detail: format!(
                    "sidecar dag node {} not present in processes.json",
                    side.dag_node_id
                ),
            });
        }
    }

    if findings.is_empty() {
        findings.push(RunnerShapeFinding {
            kind: RunnerShapeKind::Ok,
            subject: "runner-shape".to_owned(),
            detail: "all checks passed".to_owned(),
        });
    }

    RunnerShapeReport {
        findings,
        net_handoff_outcome: net_probe,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn happy_input() -> RunnerShapePreflightInput {
        RunnerShapePreflightInput {
            ch_capabilities_declared: vec!["headless".into(), "virtio-fs".into()],
            ch_capabilities_packaged: vec!["headless".into(), "virtio-fs".into()],
            declared_runner_argv: vec![DeclaredRunnerEntry {
                vm: "corp-vm".into(),
                argv_hash: "abc123".into(),
            }],
            ch_api_socket_paths: vec![ChApiSocket {
                vm: "corp-vm".into(),
                path: "/run/d2b/vms/corp-vm/ch.sock".into(),
                mode: 0o660,
                owner: "d2bd".into(),
            }],
            vsock_transports: vec![VsockTransport {
                vm: "corp-vm".into(),
                transport: "unix".into(),
            }],
            sidecar_nodes: vec![SidecarNode {
                vm: "corp-vm".into(),
                kind: "virtiofsd".into(),
                dag_node_id: "corp-vm/virtiofsd".into(),
            }],
            processes_dag_node_ids: vec!["corp-vm/virtiofsd".into()],
        }
    }

    #[test]
    fn happy_path_yields_only_ok_finding() {
        let report = runner_shape_preflight(&happy_input(), None);
        assert!(!report.fail_closed());
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].kind, RunnerShapeKind::Ok);
    }

    #[test]
    fn missing_declared_runner_fails_closed() {
        let mut input = happy_input();
        input.declared_runner_argv.clear();
        let report = runner_shape_preflight(&input, None);
        assert!(report.fail_closed());
    }

    #[test]
    fn empty_argv_hash_is_runner_shape_drift() {
        let mut input = happy_input();
        input.declared_runner_argv[0].argv_hash.clear();
        let report = runner_shape_preflight(&input, None);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == RunnerShapeKind::RunnerShapeDrift)
        );
    }

    #[test]
    fn capability_drift_surfaces() {
        let mut input = happy_input();
        input.ch_capabilities_packaged.retain(|c| c != "virtio-fs");
        let report = runner_shape_preflight(&input, None);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == RunnerShapeKind::CapabilityMismatch)
        );
    }

    #[test]
    fn non_unix_vsock_transport_rejected() {
        let mut input = happy_input();
        input.vsock_transports[0].transport = "ip".into();
        let report = runner_shape_preflight(&input, None);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == RunnerShapeKind::VsockTransportInvalid)
        );
    }

    #[test]
    fn sidecar_dag_mismatch_surfaces() {
        let mut input = happy_input();
        input.sidecar_nodes[0].dag_node_id = "nope".into();
        let report = runner_shape_preflight(&input, None);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == RunnerShapeKind::SidecarDagMismatch)
        );
    }

    #[test]
    fn ch_help_with_fd_selects_tap_fd() {
        let outcome = probe_ch_net_handoff_mode("--net <mac=MAC,fd=<n>,...>");
        match outcome {
            NetHandoffProbeOutcome::Selected { mode, .. } => {
                assert_eq!(mode, NetHandoffMode::TapFd);
            }
            other => panic!("expected TapFd, got {other:?}"),
        }
    }

    #[test]
    fn ch_help_without_fd_or_tap_fails_closed() {
        let outcome = probe_ch_net_handoff_mode("nothing useful here");
        assert_eq!(
            outcome,
            NetHandoffProbeOutcome::Failed(NetHandoffProbeError::NeitherSupported)
        );
    }

    #[test]
    fn parity_drift_fixture_fails_closed() {
        // Parse the on-disk parity-drift golden fixture
        // (tests/golden/runner-shape/parity-drift.json) and drive
        // runner_shape_preflight against it. Every fail-closed class
        // in the fixture must surface.
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent() // packages/
            .and_then(|p| p.parent()) // repo root
            .expect("repo root")
            .join("tests/golden/runner-shape/parity-drift.json");
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let input: RunnerShapePreflightInput =
            serde_json::from_str(&body).unwrap_or_else(|err| panic!("parse fixture: {err}"));
        let report = runner_shape_preflight(&input, None);
        assert!(
            report.fail_closed(),
            "parity-drift fixture must trip fail-closed but produced: {:?}",
            report.findings
        );
        // Capability mismatch (vhost-user-net declared but not packaged).
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == RunnerShapeKind::CapabilityMismatch)
        );
        // RunnerShapeDrift on empty argv hash.
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == RunnerShapeKind::RunnerShapeDrift)
        );
        // ApiSocketPathOwnershipMismatch on empty owner.
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == RunnerShapeKind::ApiSocketPathOwnershipMismatch)
        );
        // VsockTransportInvalid for "ip".
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == RunnerShapeKind::VsockTransportInvalid)
        );
        // SidecarDagMismatch for missing-node.
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == RunnerShapeKind::SidecarDagMismatch)
        );
    }

    #[test]
    fn ch_help_with_tap_only_selects_persistent_tap() {
        let outcome = probe_ch_net_handoff_mode("--net <mac=MAC,tap=<name>>");
        match outcome {
            NetHandoffProbeOutcome::Selected { mode, .. } => {
                assert_eq!(mode, NetHandoffMode::PersistentTap);
            }
            other => panic!("expected PersistentTap, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Production bundle entry points
// ---------------------------------------------------------------------------

use d2b_core::bundle::Bundle;
use d2b_core::host::HostJson;
use std::path::{Path, PathBuf};

/// Production-entry error variants for the bundle-driven preflight.
#[derive(Debug)]
pub enum RunnerShapeError {
    Io {
        path: PathBuf,
        detail: String,
    },
    ParseHostJson {
        path: PathBuf,
        detail: String,
    },
    ParseProcessesJson {
        path: PathBuf,
        detail: String,
    },
    ParseClosureJson {
        vm: String,
        path: PathBuf,
        detail: String,
    },
    ChProbeFailed {
        binary: PathBuf,
        detail: String,
    },
}

impl std::fmt::Display for RunnerShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, detail } => write!(f, "io {}: {detail}", path.display()),
            Self::ParseHostJson { path, detail } => {
                write!(f, "parse host.json {}: {detail}", path.display())
            }
            Self::ParseProcessesJson { path, detail } => {
                write!(f, "parse processes.json {}: {detail}", path.display())
            }
            Self::ParseClosureJson { vm, path, detail } => {
                write!(f, "parse closures/{vm}.json {}: {detail}", path.display())
            }
            Self::ChProbeFailed { binary, detail } => {
                write!(f, "ch probe via {}: {detail}", binary.display())
            }
        }
    }
}

impl std::error::Error for RunnerShapeError {}

/// Resolve a bundle-relative or absolute path against the bundle's
/// root directory.
fn resolve_bundle_path(bundle_root: &Path, path_str: &str) -> PathBuf {
    let p = Path::new(path_str);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        bundle_root.join(p)
    }
}

/// Production entry point for the runner-shape preflight. Consumes
/// every enabled VM's `host.json`, `processes.json`, and
/// `closures/<vm>.json` artifacts plus a packaged Cloud Hypervisor
/// binary; produces a [`RunnerShapeReport`] surfaced by `host check`.
///
/// Validation surface:
///
/// 1. Supported headless CH capability rows match the packaged binary
///    (probe `ch --help` and compare against `host.json`'s
///    `cloudHypervisorCapabilities`).
/// 2. `declaredRunner` parity snapshots are present per VM closure.
/// 3. CH API socket path ownership/mode expectations match `host.json`
///    declarations (mode `0o660`, non-empty owner).
/// 4. vsock is Unix-socket-backed per `processes.json` DAG.
/// 5. virtiofsd/swtpm sidecar input declarations match the
///    `processes.json` DAG node ids.
///
/// The low-level [`runner_shape_preflight`] helper remains testable;
/// this function is the production entry the CLI's `host check`
/// surface calls.
pub fn runner_shape_preflight_from_bundle(
    bundle: &Bundle,
    bundle_root: &Path,
    ch_binary_path: &Path,
) -> Result<RunnerShapeReport, RunnerShapeError> {
    let host_path = resolve_bundle_path(bundle_root, &bundle.host_path);
    let host_json_bytes = std::fs::read(&host_path).map_err(|err| RunnerShapeError::Io {
        path: host_path.clone(),
        detail: err.to_string(),
    })?;
    let host_json: HostJson = serde_json::from_slice(&host_json_bytes).map_err(|err| {
        RunnerShapeError::ParseHostJson {
            path: host_path.clone(),
            detail: err.to_string(),
        }
    })?;

    let processes_path = resolve_bundle_path(bundle_root, &bundle.processes_path);
    let processes_value: serde_json::Value = match std::fs::read(&processes_path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|err| RunnerShapeError::ParseProcessesJson {
                path: processes_path.clone(),
                detail: err.to_string(),
            })?
        }
        Err(err) => {
            return Err(RunnerShapeError::Io {
                path: processes_path,
                detail: err.to_string(),
            });
        }
    };

    let ch_capabilities_declared: Vec<String> = host_json
        .cloud_hypervisor_capabilities
        .iter()
        .map(|c| c.capability.clone())
        .collect();

    let ch_help = probe_ch_help(ch_binary_path)?;
    let ch_capabilities_packaged = derive_packaged_capabilities(&ch_help);
    let net_probe = probe_ch_net_handoff_mode(&ch_help);

    // Per-VM closure walk.
    let mut declared_runner_argv = Vec::new();
    let mut ch_api_socket_paths = Vec::new();
    let mut vsock_transports = Vec::new();
    let mut sidecar_nodes = Vec::new();

    for cref in &bundle.closures {
        let closure_path = resolve_bundle_path(bundle_root, &cref.path);
        let closure_bytes = std::fs::read(&closure_path).map_err(|err| RunnerShapeError::Io {
            path: closure_path.clone(),
            detail: err.to_string(),
        })?;
        let closure_value: serde_json::Value =
            serde_json::from_slice(&closure_bytes).map_err(|err| {
                RunnerShapeError::ParseClosureJson {
                    vm: cref.vm.clone(),
                    path: closure_path.clone(),
                    detail: err.to_string(),
                }
            })?;
        // `declaredRunner.argvHash` parity snapshot.
        let argv_hash = closure_value
            .get("declaredRunner")
            .and_then(|r| r.get("argvHash"))
            .and_then(|h| h.as_str())
            .unwrap_or("")
            .to_owned();
        declared_runner_argv.push(DeclaredRunnerEntry {
            vm: cref.vm.clone(),
            argv_hash,
        });
        // CH API socket path expectation.
        if let Some(sock) = closure_value.get("chApiSocket") {
            ch_api_socket_paths.push(ChApiSocket {
                vm: cref.vm.clone(),
                path: sock
                    .get("path")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_owned(),
                mode: sock.get("mode").and_then(|m| m.as_u64()).unwrap_or(0) as u32,
                owner: sock
                    .get("owner")
                    .and_then(|o| o.as_str())
                    .unwrap_or("")
                    .to_owned(),
            });
        }
        // vsock transport.
        if let Some(t) = closure_value
            .get("vsock")
            .and_then(|v| v.get("transport"))
            .and_then(|s| s.as_str())
        {
            vsock_transports.push(VsockTransport {
                vm: cref.vm.clone(),
                transport: t.to_owned(),
            });
        }
        // sidecar declarations.
        if let Some(sidecars) = closure_value.get("sidecars").and_then(|s| s.as_array()) {
            for s in sidecars {
                let kind = s
                    .get("kind")
                    .and_then(|k| k.as_str())
                    .unwrap_or("")
                    .to_owned();
                let dag_node_id = s
                    .get("dagNodeId")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_owned();
                sidecar_nodes.push(SidecarNode {
                    vm: cref.vm.clone(),
                    kind,
                    dag_node_id,
                });
            }
        }
    }

    let processes_dag_node_ids = extract_processes_dag_node_ids(&processes_value);

    let input = RunnerShapePreflightInput {
        ch_capabilities_declared,
        ch_capabilities_packaged,
        declared_runner_argv,
        ch_api_socket_paths,
        vsock_transports,
        sidecar_nodes,
        processes_dag_node_ids,
    };
    Ok(runner_shape_preflight(&input, Some(net_probe)))
}

/// Production entry for the CH net-handoff probe driven by the
/// bundle's packaged Cloud Hypervisor binary. Resolves the binary
/// path against the bundle root and feeds the `--help` output to
/// [`probe_ch_net_handoff_mode`]. The resulting mode is recorded by
/// H3 in `host.json` under `host.ch.netHandoffMode`.
pub fn probe_ch_net_handoff_mode_for_bundle(
    _bundle: &Bundle,
    ch_binary_path: &Path,
) -> Result<NetHandoffMode, RunnerShapeError> {
    let help = probe_ch_help(ch_binary_path)?;
    match probe_ch_net_handoff_mode(&help) {
        NetHandoffProbeOutcome::Selected { mode, .. } => Ok(mode),
        NetHandoffProbeOutcome::Failed(err) => Err(RunnerShapeError::ChProbeFailed {
            binary: ch_binary_path.to_path_buf(),
            detail: format!("net handoff probe failed: {err:?}"),
        }),
    }
}

fn probe_ch_help(ch_binary_path: &Path) -> Result<String, RunnerShapeError> {
    use std::process::Command;
    let output = Command::new(ch_binary_path)
        .arg("--help")
        .output()
        .map_err(|err| RunnerShapeError::ChProbeFailed {
            binary: ch_binary_path.to_path_buf(),
            detail: err.to_string(),
        })?;
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(combined)
}

/// Derive packaged CH capabilities from the `--help` text. Best-
/// effort: every long-form `--<flag>` is treated as a capability
/// surface name plus a handful of curated tokens (`headless`,
/// `virtio-fs`, `vhost-user-net`, `vsock`) that map 1:1 to the
/// `host.json` capability matrix rows.
fn derive_packaged_capabilities(help_text: &str) -> Vec<String> {
    let mut caps = std::collections::BTreeSet::new();
    for token in [
        "headless",
        "virtio-fs",
        "vhost-user-net",
        "vsock",
        "tpm",
        "console",
    ] {
        if help_text.contains(token) {
            caps.insert(token.to_owned());
        }
    }
    caps.into_iter().collect()
}

fn extract_processes_dag_node_ids(processes: &serde_json::Value) -> Vec<String> {
    let mut ids = std::collections::BTreeSet::new();
    fn walk(v: &serde_json::Value, ids: &mut std::collections::BTreeSet<String>) {
        match v {
            serde_json::Value::Object(map) => {
                if let Some(id) = map.get("dagNodeId").and_then(|s| s.as_str()) {
                    ids.insert(id.to_owned());
                }
                if let Some(id) = map.get("nodeId").and_then(|s| s.as_str()) {
                    ids.insert(id.to_owned());
                }
                for (_, val) in map {
                    walk(val, ids);
                }
            }
            serde_json::Value::Array(arr) => {
                for el in arr {
                    walk(el, ids);
                }
            }
            _ => {}
        }
    }
    walk(processes, &mut ids);
    ids.into_iter().collect()
}

#[cfg(test)]
mod bundle_tests {
    use super::*;

    #[test]
    fn extract_dag_node_ids_walks_arbitrary_shapes() {
        let v: serde_json::Value = serde_json::json!({
            "vms": [
                {"vm": "a", "nodes": [{"dagNodeId": "a/virtiofsd"}, {"nodeId": "a/swtpm"}]}
            ]
        });
        let ids = extract_processes_dag_node_ids(&v);
        assert!(ids.contains(&"a/virtiofsd".to_owned()));
        assert!(ids.contains(&"a/swtpm".to_owned()));
    }

    #[test]
    fn derive_packaged_capabilities_picks_up_known_tokens() {
        let caps = derive_packaged_capabilities("Usage: ch --headless --virtio-fs --vsock\n");
        assert!(caps.contains(&"headless".to_owned()));
        assert!(caps.contains(&"virtio-fs".to_owned()));
        assert!(caps.contains(&"vsock".to_owned()));
    }
}
