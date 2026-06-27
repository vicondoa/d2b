//! `ApplyNftables` broker op implementation.
//!
//! Re-derives the `inet d2b` chain layout from the trusted bundle,
//! refuses unless the detected firewall manager matches the bundle's
//! declared [`CoexistencePolicy`], and re-hashes the live table via
//! `nft list table inet d2b -j` both periodically and right before
//! every VM start. Foreign tables are NEVER flushed.
//!
//! The audit envelope for `ApplyNftables` carries:
//! `table_hash_before`, `table_hash_after`, `coexistence_policy`,
//! `manager_detected`. This module exposes those fields through
//! [`ApplyNftablesAudit`]; the broker runtime writes the JSON envelope
//! with the common header.

use crate::live_handlers::LiveHandlerError;
use crate::ops::exec_reconcile::{ReconcileExecError, ReconcileExecutor};
use d2b_core::host_w3::{CoexistencePolicy, FirewallCoexistencePolicy, FirewallManager};
use d2b_host::nftables::{
    self, DetectorProbe, NftBatch, NftError, ParseNftScriptError, Sha256, build_inet_d2b_chains,
    evaluate_coexistence_policy, hash_inet_d2b_table,
};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Audit-event payload for `ApplyNftables`. Combined with the broker
/// common header at write time. Sensitive identifiers go through the
/// hash discipline upstream; this struct stores already-hashed values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyNftablesAudit {
    pub table_hash_before: Sha256,
    pub table_hash_after: Sha256,
    pub coexistence_policy: CoexistencePolicy,
    pub manager_detected: FirewallManager,
}

/// Decision returned by [`apply_nftables`] after the typed reconcile
/// loop, before the broker hands off to `nft -f -`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyNftablesDecision {
    pub batch: NftBatch,
    pub audit: ApplyNftablesAudit,
    pub nft_script: String,
}

/// Bundle-derived inputs to the `ApplyNftables` op. Mirrors the typed
/// `host.json` row the integrator-prep commit emits.
#[derive(Debug, Clone)]
pub struct ApplyNftablesInputs {
    pub detected: FirewallManager,
    pub declared_policy: CoexistencePolicy,
    /// `nft list table inet d2b -j` of the live table before
    /// applying. Empty on first install.
    pub live_table_json: Vec<u8>,
}

/// Re-derive the chain layout from the bundle and produce a decision
/// the broker runtime feeds to `nft -f -`.
pub fn apply_nftables(inputs: &ApplyNftablesInputs) -> Result<ApplyNftablesDecision, NftError> {
    evaluate_coexistence_policy(inputs.detected, inputs.declared_policy)?;
    let batch = build_inet_d2b_chains();
    nftables::assert_no_forbidden_hooks(&batch)?;
    batch.assert_carveout_ordering()?;

    let table_hash_before = hash_inet_d2b_table(&inputs.live_table_json);
    let table_hash_after = batch.canonical_hash();
    let nft_script = batch.render_nft_script();

    Ok(ApplyNftablesDecision {
        batch,
        audit: ApplyNftablesAudit {
            table_hash_before,
            table_hash_after,
            coexistence_policy: inputs.declared_policy,
            manager_detected: inputs.detected,
        },
        nft_script,
    })
}

/// Re-hash the live `inet d2b` table for drift detection. Run
/// periodically and immediately before every VM start; compare against
/// the digest stored in `host.json`.
pub fn rehash_for_drift(live_table_json: &[u8]) -> Sha256 {
    hash_inet_d2b_table(live_table_json)
}

pub const DEFAULT_HOST_RUNTIME_PATH: &str = "/var/lib/d2b/runtime/host-runtime.json";
/// Backward-compatible alias: the broker now persists the applied nft
/// hash in `host-runtime.json`, but runtime code still threads this
/// constant through the older `*_SIDECAR_PATH` call sites.
pub const DEFAULT_NFT_HASH_SIDECAR_PATH: &str = DEFAULT_HOST_RUNTIME_PATH;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NftHashSidecar {
    pub table_hash_after_apply: String,
}

pub fn read_persisted_nft_hash(path: &Path) -> Result<Option<String>, ReconcileExecError> {
    let host_runtime_path = canonical_host_runtime_path(path);
    if let Some(hash) = crate::live_handlers::read_host_runtime_nft_hash(&host_runtime_path)? {
        return Ok(Some(hash));
    }
    if host_runtime_path != path {
        return read_legacy_nft_hash_sidecar(path);
    }
    Ok(None)
}

pub fn persist_live_nft_hash(
    _executor: &dyn ReconcileExecutor,
    nft_binary: &Path,
    family: &str,
    table: &str,
    sidecar_path: &Path,
) -> Result<String, ReconcileExecError> {
    let live_json = read_live_table_json_required(nft_binary, family, table)?;
    let live_hash = hash_inet_d2b_table(&live_json).to_string();
    crate::live_handlers::update_host_runtime_nft_hash(
        &canonical_host_runtime_path(sidecar_path),
        Some(live_hash.as_str()),
    )?;
    Ok(live_hash)
}

fn persisted_nft_hash_path() -> PathBuf {
    env::var_os("D2B_BROKER_NFT_HASH_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_NFT_HASH_SIDECAR_PATH))
}

fn canonical_host_runtime_path(path: &Path) -> PathBuf {
    match path.file_name().and_then(|name| name.to_str()) {
        Some("nft-hash.json") => path.with_file_name("host-runtime.json"),
        _ => path.to_path_buf(),
    }
}

fn read_legacy_nft_hash_sidecar(path: &Path) -> Result<Option<String>, ReconcileExecError> {
    match fs::read(path) {
        Ok(bytes) => {
            let sidecar = serde_json::from_slice::<NftHashSidecar>(&bytes).map_err(|err| {
                ReconcileExecError::InvalidInput {
                    detail: format!("invalid nft hash sidecar {}: {err}", path.display()),
                }
            })?;
            Ok(Some(sidecar.table_hash_after_apply))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(ReconcileExecError::Io {
            path: path.display().to_string(),
            detail: err.to_string(),
        }),
    }
}

fn select_drift_expected_hash(
    canonical_hash: &str,
    caller_expected_hash: Option<&str>,
    persisted_hash: Option<&str>,
) -> Option<String> {
    persisted_hash.map(ToOwned::to_owned).or_else(|| {
        caller_expected_hash.map(|expected| {
            if expected == canonical_hash {
                canonical_hash.to_owned()
            } else {
                expected.to_owned()
            }
        })
    })
}

/// Helper for the broker runtime: build a [`DetectorProbe`] from
/// individual shell-out results. Kept here so the call site does not
/// have to import `d2b-host` directly.
pub fn detector_probe_from_results(
    firewalld_active: bool,
    ufw_active: bool,
    docker_active: bool,
    libvirt_active: bool,
    iptables_reports_nf_tables: bool,
) -> DetectorProbe {
    DetectorProbe {
        firewalld_active,
        ufw_active,
        docker_active,
        libvirt_active,
        iptables_reports_nf_tables,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyWithCoexistenceError {
    CoexistenceRefused {
        manager: FirewallManager,
        rationale: String,
    },
    ParseFailed(ParseNftScriptError),
    CarveoutOrderingViolation(NftError),
    DriftDetected {
        expected: String,
        observed: String,
    },
    ReconcileExec(ReconcileExecError),
}

impl std::fmt::Display for ApplyWithCoexistenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CoexistenceRefused { manager, rationale } => write!(
                f,
                "apply-nftables refused by firewall coexistence policy for {manager:?}: {rationale}"
            ),
            Self::ParseFailed(err) => {
                write!(f, "apply-nftables: failed to parse nft script: {err}")
            }
            Self::CarveoutOrderingViolation(NftError::ForeignNftRuleShadowsD2b { details }) => {
                write!(f, "apply-nftables: carve-out ordering violation: {details}")
            }
            Self::CarveoutOrderingViolation(err) => {
                write!(f, "apply-nftables: carve-out ordering violation: {err}")
            }
            Self::DriftDetected { expected, observed } => write!(
                f,
                "apply-nftables: canonical inet d2b hash drift detected (expected={expected}, observed={observed})"
            ),
            Self::ReconcileExec(err) => write!(f, "apply-nftables: {err}"),
        }
    }
}

impl std::error::Error for ApplyWithCoexistenceError {}

/// Runtime entry-point for `ApplyNftables`.
///
/// Refuses fail-closed when the loaded `host.json` says the detected
/// manager must not coexist, then parses the emitted script back into a
/// structured batch so carve-out ordering and canonical drift hashing are
/// re-asserted immediately before the live `nft -f -` apply.
pub fn apply_with_coexistence(
    executor: &dyn ReconcileExecutor,
    nft_binary: &Path,
    script_body: &str,
    ownership_id: &str,
    host_policy: Option<&FirewallCoexistencePolicy>,
    expected_table_hash: Option<&str>,
) -> Result<String, ApplyWithCoexistenceError> {
    let batch = NftBatch::parse(script_body).map_err(ApplyWithCoexistenceError::ParseFailed)?;
    let canonical = batch.canonical_hash().to_string();
    let persisted_hash = read_persisted_nft_hash(&persisted_nft_hash_path())
        .map_err(ApplyWithCoexistenceError::ReconcileExec)?;
    let live_table_json =
        read_live_table_json_optional(nft_binary, batch.table_family, batch.table_name)
            .map_err(ApplyWithCoexistenceError::ReconcileExec)?;
    let (drift_expected_hash, observed_table_hash) = if let Some(json) = live_table_json {
        (
            select_drift_expected_hash(
                canonical.as_str(),
                expected_table_hash,
                persisted_hash.as_deref(),
            ),
            Some(hash_inet_d2b_table(&json).to_string()),
        )
    } else {
        (None, None)
    };
    apply_with_coexistence_inner(
        executor,
        nft_binary,
        script_body,
        ownership_id,
        host_policy,
        drift_expected_hash.as_deref(),
        observed_table_hash.as_deref(),
    )
}

fn apply_with_coexistence_inner(
    executor: &dyn ReconcileExecutor,
    nft_binary: &Path,
    script_body: &str,
    ownership_id: &str,
    host_policy: Option<&FirewallCoexistencePolicy>,
    expected_table_hash: Option<&str>,
    observed_table_hash: Option<&str>,
) -> Result<String, ApplyWithCoexistenceError> {
    if let Some(policy) = host_policy.filter(|policy| policy.policy == CoexistencePolicy::Refuse) {
        return Err(ApplyWithCoexistenceError::CoexistenceRefused {
            manager: policy.manager,
            rationale: policy.rationale.clone(),
        });
    }
    let _ = ownership_id;
    let batch = NftBatch::parse(script_body).map_err(ApplyWithCoexistenceError::ParseFailed)?;
    batch
        .assert_carveout_ordering()
        .map_err(ApplyWithCoexistenceError::CarveoutOrderingViolation)?;
    let canonical = batch.canonical_hash();
    if let Some(expected) = expected_table_hash {
        let observed = observed_table_hash.unwrap_or("absent");
        if observed != expected {
            return Err(ApplyWithCoexistenceError::DriftDetected {
                expected: expected.to_owned(),
                observed: observed.to_owned(),
            });
        }
    }
    let replace_script = render_owned_table_replace_script(&batch, script_body);
    crate::live_handlers::live_apply_nftables(executor, nft_binary, &replace_script)
        .map_err(map_live_nft_error)?;
    Ok(canonical.as_str().to_owned())
}

fn render_owned_table_replace_script(batch: &NftBatch, script_body: &str) -> String {
    format!(
        "table {} {}\ndelete table {} {}\n{}",
        batch.table_family, batch.table_name, batch.table_family, batch.table_name, script_body
    )
}

fn map_live_nft_error(err: LiveHandlerError) -> ApplyWithCoexistenceError {
    match err {
        LiveHandlerError::ReconcileExec(inner) => ApplyWithCoexistenceError::ReconcileExec(inner),
        other => ApplyWithCoexistenceError::ReconcileExec(ReconcileExecError::InvalidInput {
            detail: other.to_string(),
        }),
    }
}

pub fn read_live_table_json_optional(
    nft_binary: &Path,
    family: &str,
    table: &str,
) -> Result<Option<Vec<u8>>, ReconcileExecError> {
    if !nft_binary
        .to_str()
        .map(|s| s.starts_with('/'))
        .unwrap_or(false)
    {
        return Err(ReconcileExecError::InvalidInput {
            detail: format!(
                "nft binary path must be absolute, got {:?}",
                nft_binary.display().to_string()
            ),
        });
    }
    let output = Command::new(nft_binary)
        .args(["-j", "list", "table", family, table])
        .env_remove("NOTIFY_SOCKET")
        .output()
        .map_err(|err| ReconcileExecError::BinaryMissing {
            which: "nft".to_owned(),
            detail: err.to_string(),
        })?;
    if output.status.success() {
        return Ok(Some(output.stdout));
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.contains("No such file or directory") || stderr.contains("No such file") {
        return Ok(None);
    }
    Err(ReconcileExecError::NonZeroExit {
        which: "nft".to_owned(),
        exit_code: output.status.code().unwrap_or(-1),
        stderr,
    })
}

fn read_live_table_json_required(
    nft_binary: &Path,
    family: &str,
    table: &str,
) -> Result<Vec<u8>, ReconcileExecError> {
    read_live_table_json_optional(nft_binary, family, table)?.ok_or_else(|| {
        ReconcileExecError::Io {
            path: format!("nft:{family}:{table}"),
            detail: "live nft table missing after successful apply".to_owned(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::exec_reconcile::{FakeReconcileExecutor, ReconcileOp};

    fn inputs(detected: FirewallManager, declared: CoexistencePolicy) -> ApplyNftablesInputs {
        ApplyNftablesInputs {
            detected,
            declared_policy: declared,
            live_table_json: br#"{"nftables":[]}"#.to_vec(),
        }
    }

    fn policy(manager: FirewallManager, declared: CoexistencePolicy) -> FirewallCoexistencePolicy {
        FirewallCoexistencePolicy {
            manager,
            policy: declared,
            rationale: format!("policy for {manager:?}"),
        }
    }

    fn parseable_script() -> (String, String) {
        let batch = build_inet_d2b_chains();
        let script = batch.render_nft_script();
        let hash = batch.canonical_hash().to_string();
        (script, hash)
    }

    fn usbip_script() -> (String, String) {
        let mut batch = build_inet_d2b_chains();
        batch
            .add_usbip_carveout(&d2b_host::nftables::BusId::new("1-1.2"))
            .expect("carveout");
        let script = batch.render_nft_script();
        let hash = batch.canonical_hash().to_string();
        (script, hash)
    }

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            use std::time::{SystemTime, UNIX_EPOCH};

            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock before epoch")
                .as_nanos();
            let path = std::env::current_dir()
                .expect("cwd")
                .join("target")
                .join(format!("{prefix}-{unique}"));
            std::fs::create_dir_all(&path).expect("create test dir");
            Self { path }
        }

        fn join(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn applies_on_clean_host_with_coexist() {
        let decision =
            apply_nftables(&inputs(FirewallManager::None, CoexistencePolicy::Coexist)).unwrap();
        assert_eq!(decision.audit.manager_detected, FirewallManager::None);
        assert_eq!(
            decision.audit.coexistence_policy,
            CoexistencePolicy::Coexist
        );
        // table_hash_after pinned via render_nft_script.
        assert_eq!(
            decision.audit.table_hash_after,
            decision.batch.canonical_hash()
        );
    }

    #[test]
    fn refuses_when_firewalld_declared_coexist() {
        let err = apply_nftables(&inputs(
            FirewallManager::Firewalld,
            CoexistencePolicy::Coexist,
        ))
        .unwrap_err();
        assert_eq!(err.as_kebab_case(), "firewall-coexistence-mismatch");
    }

    #[test]
    fn nft_script_includes_all_four_chains() {
        let decision =
            apply_nftables(&inputs(FirewallManager::None, CoexistencePolicy::Coexist)).unwrap();
        for chain in &["prerouting", "forward", "output", "input"] {
            assert!(
                decision.nft_script.contains(&format!("chain {chain} {{")),
                "missing chain {chain} in rendered nft script"
            );
        }
        for forbidden in &["raw {", "mangle {", "nat {"] {
            assert!(
                !decision.nft_script.contains(forbidden),
                "rendered nft script must not declare a {forbidden} chain"
            );
        }
    }

    #[test]
    fn apply_with_coexistence_drives_executor_when_policy_allows() {
        let exec = FakeReconcileExecutor::new();
        let (script, expected_hash) = parseable_script();
        let applied_hash = apply_with_coexistence_inner(
            &exec,
            Path::new("/usr/sbin/nft"),
            &script,
            "owner-1",
            Some(&policy(FirewallManager::None, CoexistencePolicy::Coexist)),
            Some(expected_hash.as_str()),
            Some(expected_hash.as_str()),
        )
        .unwrap();
        assert_eq!(applied_hash, expected_hash);
        let log = exec.take_log();
        assert_eq!(log.len(), 1);
        match &log[0] {
            ReconcileOp::ApplyNftScript { binary, script } => {
                assert!(binary.ends_with("nft"));
                assert!(script.contains("inet d2b"));
                assert!(script.starts_with("table inet d2b\ndelete table inet d2b\n"));
            }
            other => panic!("unexpected op: {other:?}"),
        }
    }

    #[test]
    fn apply_with_coexistence_refuses_on_drift() {
        let exec = FakeReconcileExecutor::new();
        let (script, expected_hash) = parseable_script();
        let err = apply_with_coexistence_inner(
            &exec,
            Path::new("/usr/sbin/nft"),
            &script,
            "owner-1",
            Some(&policy(FirewallManager::None, CoexistencePolicy::Coexist)),
            Some(expected_hash.as_str()),
            Some("wrong-hash"),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ApplyWithCoexistenceError::DriftDetected {
                expected,
                observed,
            } if expected == expected_hash && observed == "wrong-hash"
        ));
        assert!(exec.take_log().is_empty());
    }

    #[test]
    fn apply_with_coexistence_refuses_on_carveout_misorder() {
        let exec = FakeReconcileExecutor::new();
        let script = concat!(
            "table inet d2b {\n",
            "  chain forward {\n",
            "    type filter hook forward priority -5; policy drop;\n",
            "    drop comment \"d2b managed: broad-drop\"\n",
            "    meta iifname \"usbip-3-1\" accept comment \"d2b managed: owner-1\"\n",
            "  }\n",
            "}\n"
        );
        let err = apply_with_coexistence_inner(
            &exec,
            Path::new("/usr/sbin/nft"),
            script,
            "owner-1",
            Some(&policy(FirewallManager::None, CoexistencePolicy::Coexist)),
            None,
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ApplyWithCoexistenceError::CarveoutOrderingViolation(
                NftError::ForeignNftRuleShadowsD2b { .. }
            )
        ));
        assert!(exec.take_log().is_empty());
    }

    #[test]
    fn apply_with_coexistence_accepts_first_apply_no_drift_check() {
        let exec = FakeReconcileExecutor::new();
        let (script, expected_hash) = parseable_script();
        let applied_hash = apply_with_coexistence_inner(
            &exec,
            Path::new("/usr/sbin/nft"),
            &script,
            "owner-1",
            Some(&policy(FirewallManager::None, CoexistencePolicy::Coexist)),
            None,
            None,
        )
        .unwrap();
        assert_eq!(applied_hash, expected_hash);
        assert_eq!(exec.take_log().len(), 1);
    }

    #[test]
    fn apply_with_coexistence_refuses_when_policy_is_refuse() {
        let exec = FakeReconcileExecutor::new();
        let (script, _) = parseable_script();
        let err = apply_with_coexistence_inner(
            &exec,
            Path::new("/usr/sbin/nft"),
            &script,
            "owner-1",
            Some(&policy(
                FirewallManager::Firewalld,
                CoexistencePolicy::Refuse,
            )),
            None,
            None,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ApplyWithCoexistenceError::CoexistenceRefused {
                manager: FirewallManager::Firewalld,
                ..
            }
        ));
        assert!(exec.take_log().is_empty());
    }

    #[test]
    fn usbip_batch_can_apply_against_matching_prior_live_hash() {
        let exec = FakeReconcileExecutor::new();
        let (_base_script, base_hash) = parseable_script();
        let (usbip_script, usbip_hash) = usbip_script();
        let applied_hash = apply_with_coexistence_inner(
            &exec,
            Path::new("/usr/sbin/nft"),
            &usbip_script,
            "owner-1",
            None,
            Some(base_hash.as_str()),
            Some(base_hash.as_str()),
        )
        .unwrap();
        assert_eq!(applied_hash, usbip_hash);
    }

    #[test]
    fn read_persisted_nft_hash_reads_host_runtime_hash() {
        let root = TestDir::new("nft-host-runtime");
        let runtime_path = root.join("host-runtime.json");
        std::fs::write(
            &runtime_path,
            serde_json::json!({
                "schemaVersion": "v2",
                "bundleVersion": 4,
                "generatedAt": "2025-01-01T00:00:00Z",
                "nftAppliedHash": "0123456789abcdef",
                "ifnames": []
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(
            read_persisted_nft_hash(&runtime_path).unwrap(),
            Some("0123456789abcdef".to_owned())
        );
    }

    #[test]
    fn read_persisted_nft_hash_falls_back_to_legacy_sidecar() {
        let root = TestDir::new("nft-sidecar-fallback");
        let runtime_path = root.join("host-runtime.json");
        std::fs::write(
            &runtime_path,
            serde_json::json!({
                "schemaVersion": "v2",
                "bundleVersion": 4,
                "generatedAt": "2025-01-01T00:00:00Z",
                "nftAppliedHash": null,
                "ifnames": []
            })
            .to_string(),
        )
        .unwrap();
        let sidecar_path = root.join("nft-hash.json");
        std::fs::write(
            &sidecar_path,
            serde_json::to_vec(&NftHashSidecar {
                table_hash_after_apply: "feedfacefeedface".to_owned(),
            })
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            read_persisted_nft_hash(&sidecar_path).unwrap(),
            Some("feedfacefeedface".to_owned())
        );
    }

    #[test]
    fn persisted_hash_wins_over_caller_expected_hash() {
        assert_eq!(
            select_drift_expected_hash("current-hash", Some("current-hash"), Some("old-hash")),
            Some("old-hash".to_owned())
        );
    }
}
