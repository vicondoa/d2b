//! Net-VM start preflight that refuses to bring up a `sys-<env>-net` VM
//! when the on-disk dnsmasq.conf
//! hash diverges from the hash the trusted bundle implies for that
//! env.
//!
//! # Why this preflight exists
//!
//! The per-env dnsmasq.conf is rendered from three bundle-owned intent
//! sources:
//!
//! * `hosts_intent[host]` — `/etc/hosts` managed block (one line per
//!   env / bridge / MTU) ← `BundleResolver::find_hosts_intent`.
//! * `nft_intent[env:<env>]` — per-env nftables subset whose
//!   `desired_hash` already digests every bridge port-flag /
//!   forward-blocklist line that informs DHCP visibility.
//! * `route_intent[env:<env>:*]` — per-env route specs the net VM
//!   relies on for its uplink view.
//!
//! When the bundle is updated (e.g. workloads added, an env's bridge
//! flags flipped, the route table changed) the dnsmasq.conf the net
//! VM consumes must be re-rendered in lock-step. The render itself
//! is owned by a host singleton (or, in the legacy world, a systemd
//! oneshot). If the render step fails — or, worse, was never
//! triggered — the running net VM would silently serve a stale lease
//! table to its workloads, leaving the bundle's intent and the
//! observable network behaviour out of sync.
//!
//! This preflight is the fail-closed guard for that gap. The daemon
//! computes the expected dnsmasq.conf hash from the bundle's three
//! intent sources and compares it against the SHA-256 of the
//! on-disk dnsmasq.conf at
//! `${dnsmasq_dir}/<env>.conf` (default
//! `/var/lib/nixling/dnsmasq/<env>.conf`). On mismatch the daemon
//! refuses VM start with [`TypedError::BundleDnsmasqDrift`] (exit
//! code 63) and the operator-facing remediation is "re-render
//! dnsmasq.conf, then retry".
//!
//! # Scope
//!
//! Only VMs with `is_net_vm = true` in the manifest are gated.
//! Workload VMs short-circuit to `Ok(())` with no I/O.
//!
//! The expected-hash computation is hermetic: pure functions of the
//! bundle resolver. The on-disk read is the only side effect.

use std::fs;
use std::path::{Path, PathBuf};

use nixling_core::bundle_resolver::{intent_id_hosts_host, intent_id_nft_env, BundleResolver};
use sha2::Digest as _;

/// Default parent dir holding `<env>.conf` for each net VM.
pub const DEFAULT_DNSMASQ_DIR: &str = "/var/lib/nixling/dnsmasq";

/// One drift finding produced by the net-VM bundle gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleGateDrift {
    /// The manifest carries no env field for this net VM.
    EnvMissing { vm: String },
    /// The on-disk dnsmasq.conf does not exist.
    ConfigMissing { env: String, path: PathBuf },
    /// stat / read of the on-disk dnsmasq.conf failed.
    ConfigReadFailed {
        env: String,
        path: PathBuf,
        detail: String,
    },
    /// Hash digest mismatch between bundle-derived expectation and
    /// the on-disk dnsmasq.conf.
    HashMismatch {
        env: String,
        path: PathBuf,
        expected: String,
        actual: String,
    },
}

impl BundleGateDrift {
    /// Path most relevant to the operator-facing diagnostic. For
    /// `EnvMissing` (no on-disk artifact involved) returns the
    /// canonical `/var/lib/nixling/dnsmasq/` parent.
    pub fn path(&self) -> PathBuf {
        match self {
            Self::EnvMissing { .. } => PathBuf::from(DEFAULT_DNSMASQ_DIR),
            Self::ConfigMissing { path, .. }
            | Self::ConfigReadFailed { path, .. }
            | Self::HashMismatch { path, .. } => path.clone(),
        }
    }

    /// Env scope this drift was raised for, when known.
    pub fn env(&self) -> &str {
        match self {
            Self::EnvMissing { .. } => "",
            Self::ConfigMissing { env, .. }
            | Self::ConfigReadFailed { env, .. }
            | Self::HashMismatch { env, .. } => env.as_str(),
        }
    }

    /// Short, single-line, path-redacted reason for the typed-error
    /// envelope's `message` field. The full path is logged separately
    /// via `tracing::warn!`.
    pub fn reason(&self) -> String {
        match self {
            Self::EnvMissing { vm } => {
                format!("net VM '{vm}' has no env in manifest; cannot resolve dnsmasq scope")
            }
            Self::ConfigMissing { env, .. } => format!(
                "dnsmasq.conf for env '{env}' is missing; bundle/dnsmasq render did not run"
            ),
            Self::ConfigReadFailed { env, detail, .. } => {
                format!("dnsmasq.conf for env '{env}' could not be read: {detail}")
            }
            Self::HashMismatch {
                env,
                expected,
                actual,
                ..
            } => format!(
                "dnsmasq.conf hash for env '{env}' diverges from bundle expectation \
                 (expected {expected}, actual {actual}); rebuild required"
            ),
        }
    }
}

/// Compute the bundle-derived expected dnsmasq.conf hash for `env`.
///
/// The inputs are deterministically combined as:
///
/// ```text
/// b"nixling-dnsmasq:v1\n"
///   || b"nft:" || <nft env script body or "<absent>">       || b"\n"
///   || b"hosts:" || <hosts host managed block or "<absent>"> || b"\n"
///   || b"routes:\n"
///   || for each route_spec (sorted by intent_id):
///        b"  " || <route_spec> || b"\n"
/// ```
///
/// The function is pure and panic-free. Callers compare the return
/// against the hex SHA-256 of the on-disk dnsmasq.conf.
pub fn compute_expected_dnsmasq_hash(resolver: &BundleResolver, env: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"nixling-dnsmasq:v1\n");

    let nft_id = intent_id_nft_env(env);
    let nft_body = resolver
        .find_nft_intent(&nft_id)
        .map(|intent| intent.script_body.as_str())
        .unwrap_or("<absent>");
    hasher.update(b"nft:");
    hasher.update(nft_body.as_bytes());
    hasher.update(b"\n");

    let hosts_id = intent_id_hosts_host();
    let hosts_body = resolver
        .find_hosts_intent(&hosts_id)
        .map(|intent| intent.managed_block.as_str())
        .unwrap_or("<absent>");
    hasher.update(b"hosts:");
    hasher.update(hosts_body.as_bytes());
    hasher.update(b"\n");

    hasher.update(b"routes:\n");
    let scope_prefix = format!("route:env:{env}:");
    let mut route_ids: Vec<&str> = resolver
        .route_intent_ids()
        .filter(|id| id.starts_with(&scope_prefix))
        .collect();
    route_ids.sort();
    for id in route_ids {
        if let Some(intent) = resolver.find_route_intent(id) {
            hasher.update(b"  ");
            hasher.update(intent.route_spec.as_bytes());
            hasher.update(b"\n");
        }
    }

    let digest: [u8; 32] = hasher.finalize().into();
    hex_lower(&digest)
}

/// Hex-encode 32 bytes in lowercase. Local helper to avoid pulling
/// in `hex` as a dependency.
fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(nibble(byte >> 4));
        out.push(nibble(byte & 0x0f));
    }
    out
}

fn nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _ => unreachable!(),
    }
}

/// Compute the SHA-256 hex digest of the bytes at `path`.
fn read_actual_dnsmasq_hash(path: &Path) -> Result<String, std::io::Error> {
    let bytes = fs::read(path)?;
    let digest: [u8; 32] = sha2::Sha256::digest(&bytes).into();
    Ok(hex_lower(&digest))
}

/// Outcome of the net-VM bundle gate check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleGateOutcome {
    /// VM is not a net VM. No further work; the caller should
    /// proceed to its usual VM-start path.
    NotANetVm,
    /// All inputs matched; the net VM may start.
    Ok,
    /// Refuse to start the net VM: one of the drift classes fired.
    Drift(BundleGateDrift),
}

/// Run the net-VM bundle gate for `vm`. The `dnsmasq_dir` argument
/// is the parent dir holding `<env>.conf`; production uses
/// [`DEFAULT_DNSMASQ_DIR`].
///
/// Workload VMs short-circuit to [`BundleGateOutcome::NotANetVm`]
/// without any filesystem reads.
pub fn check_net_vm_bundle_gate(
    resolver: &BundleResolver,
    vm: &str,
    dnsmasq_dir: &Path,
) -> BundleGateOutcome {
    let Some(entry) = resolver.find_manifest_vm(vm) else {
        // Caller already surfaces this via a typed `InternalIo`; we
        // refuse to second-guess it here.
        return BundleGateOutcome::NotANetVm;
    };
    if !entry.is_net_vm {
        return BundleGateOutcome::NotANetVm;
    }
    let Some(env) = entry.env.as_deref() else {
        return BundleGateOutcome::Drift(BundleGateDrift::EnvMissing { vm: vm.to_owned() });
    };

    let path = dnsmasq_dir.join(format!("{env}.conf"));
    let actual = match read_actual_dnsmasq_hash(&path) {
        Ok(h) => h,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return BundleGateOutcome::Drift(BundleGateDrift::ConfigMissing {
                env: env.to_owned(),
                path,
            });
        }
        Err(err) => {
            return BundleGateOutcome::Drift(BundleGateDrift::ConfigReadFailed {
                env: env.to_owned(),
                path,
                detail: err.to_string(),
            });
        }
    };

    let expected = compute_expected_dnsmasq_hash(resolver, env);
    if expected != actual {
        return BundleGateOutcome::Drift(BundleGateDrift::HashMismatch {
            env: env.to_owned(),
            path,
            expected,
            actual,
        });
    }
    BundleGateOutcome::Ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_core::bundle::{Bundle, BundleGeneration};
    use nixling_core::bundle_resolver::BundleResolver;
    use nixling_core::host::HostJson;
    use nixling_core::manifest_v04::{
        ChExporterMeta, ManifestMeta, ManifestV04, ObservabilityMeta, VmEntry, VmObservability,
    };
    use nixling_core::processes::ProcessesJson;
    use std::collections::BTreeMap;
    use std::fs;

    /// Minimal bundle fixture exposing one env (`work`) and one net
    /// VM (`sys-work-net`). We construct it via
    /// `BundleResolver::from_artifacts` so the test does not need to
    /// touch disk for the bundle itself.
    fn build_resolver() -> BundleResolver {
        let host_json: HostJson = serde_json::from_str(
            r##"{
                "schemaVersion": "v2",
                "site": { "allowUnsafeEastWest": false },
                "environments": [{
                    "env": "work",
                    "bridge": "nlworkbr0",
                    "mtu": 1500,
                    "mssClamp": 1460,
                    "lan": { "allowEastWest": false, "effectiveEastWest": false },
                    "netVmForwardBlocklist": ["192.0.2.0/24"],
                    "bridgePortFlags": [],
                    "ipv6Sysctls": [],
                    "usbipBusidLocks": []
                }],
                "nftables": {
                    "family": "inet",
                    "table": "nixling",
                    "chains": [],
                    "ownershipId": "ownership-test"
                },
                "networkManager": {
                    "filePath": "/etc/NetworkManager/conf.d/00-nixling-unmanaged.conf",
                    "matchCriteria": ["interface-name:nl-*"],
                    "reloadBehavior": "atomic-reload",
                    "ownership": {
                        "owner": "root",
                        "group": "root",
                        "mode": "0644",
                        "driftPolicy": "replace"
                    }
                },
                "hostsFile": {
                    "startMarker": "# nixling-managed begin",
                    "endMarker": "# nixling-managed end",
                    "rule": "replace-managed-block"
                },
                "kernelModules": [],
                "fdOwnership": [],
                "cloudHypervisorCapabilities": [],
                "ifNameMappings": [],
                "ch": { "netHandoffMode": "tap-fd" }
            }"##,
        )
        .expect("host fixture parses");

        let processes = ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: Vec::new(),
        };

        let net_vm = VmEntry {
            api_socket: "/run/nixling/vms/sys-work-net/api.sock".to_owned(),
            audio: false,
            audio_service: String::new(),
            audio_state_file: String::new(),
            bridge: Some("nlworkbr0".to_owned()),
            env: Some("work".to_owned()),
            mtu: Some(1500),
            mss_clamp: Some(1460),
            lan: None,
            gpu_socket: String::new(),
            graphics: false,
            is_net_vm: true,
            name: "sys-work-net".to_owned(),
            net_vm: None,
            observability: VmObservability {
                agent_socket: String::new(),
                enabled: false,
                vsock_cid: 100,
                vsock_host_socket: String::new(),
            },
            ssh_user: None,
            state_dir: "/var/lib/nixling/vms/sys-work-net".to_owned(),
            static_ip: Some("10.20.0.1".to_owned()),
            tap: "work-net".to_owned(),
            tpm: false,
            tpm_socket: String::new(),
            usbip_yubikey: false,
            usbipd_host_ip: None,
        };

        let workload_vm = VmEntry {
            is_net_vm: false,
            name: "corp-vm".to_owned(),
            env: Some("work".to_owned()),
            net_vm: Some("sys-work-net".to_owned()),
            ..net_vm.clone()
        };

        let manifest = ManifestV04 {
            manifest: ManifestMeta {
                manifest_version: 4,
            },
            observability: ObservabilityMeta {
                ch_exporter: ChExporterMeta { listen_port: 9100 },
                enabled: false,
                grafana_url: "http://127.0.0.1:3000".to_owned(),
                obs_vsock_cid: 3,
                obs_vsock_host_socket: "/run/nixling/obs.sock".to_owned(),
                vm_name: "obs".to_owned(),
            },
            vms: BTreeMap::from([
                ("sys-work-net".to_owned(), net_vm),
                ("corp-vm".to_owned(), workload_vm),
            ]),
        };

        let bundle = Bundle {
            bundle_version: 3,
            schema_version: "v2".to_owned(),
            public_manifest_path: "vms.json".to_owned(),
            host_path: "host.json".to_owned(),
            processes_path: "processes.json".to_owned(),
            privileges_path: "privileges.json".to_owned(),
            closures: Vec::new(),
            minijail_profiles: Vec::new(),
            managed_keys: Default::default(),
            generation: BundleGeneration {
                generator: "test".to_owned(),
                source_revision: None,
                generated_at: None,
            },
            bundle_hash: None,
            artifact_hashes: None,
        };
        BundleResolver::from_artifacts(bundle, host_json, processes, manifest)
    }

    fn write_matching_conf(dir: &Path, env: &str, resolver: &BundleResolver) -> PathBuf {
        let path = dir.join(format!("{env}.conf"));
        // Stash the *expected hash's preimage* by writing the
        // canonical concatenation; then sha256(file bytes) ==
        // sha256(canonical) == expected.
        let mut buf = Vec::new();
        buf.extend_from_slice(b"nixling-dnsmasq:v1\n");
        let nft_id = intent_id_nft_env(env);
        let nft_body = resolver
            .find_nft_intent(&nft_id)
            .map(|i| i.script_body.as_str())
            .unwrap_or("<absent>");
        buf.extend_from_slice(b"nft:");
        buf.extend_from_slice(nft_body.as_bytes());
        buf.extend_from_slice(b"\n");
        let hosts_body = resolver
            .find_hosts_intent(&intent_id_hosts_host())
            .map(|i| i.managed_block.as_str())
            .unwrap_or("<absent>");
        buf.extend_from_slice(b"hosts:");
        buf.extend_from_slice(hosts_body.as_bytes());
        buf.extend_from_slice(b"\n");
        buf.extend_from_slice(b"routes:\n");
        let scope_prefix = format!("route:env:{env}:");
        let mut route_ids: Vec<&str> = resolver
            .route_intent_ids()
            .filter(|id| id.starts_with(&scope_prefix))
            .collect();
        route_ids.sort();
        for id in route_ids {
            if let Some(intent) = resolver.find_route_intent(id) {
                buf.extend_from_slice(b"  ");
                buf.extend_from_slice(intent.route_spec.as_bytes());
                buf.extend_from_slice(b"\n");
            }
        }
        fs::write(&path, &buf).expect("write conf");
        path
    }

    #[test]
    fn workload_vm_short_circuits_to_not_a_net_vm() {
        let resolver = build_resolver();
        let tmp = tempfile::tempdir().unwrap();
        let outcome = check_net_vm_bundle_gate(&resolver, "corp-vm", tmp.path());
        assert_eq!(outcome, BundleGateOutcome::NotANetVm);
    }

    #[test]
    fn unknown_vm_short_circuits_to_not_a_net_vm() {
        let resolver = build_resolver();
        let tmp = tempfile::tempdir().unwrap();
        let outcome = check_net_vm_bundle_gate(&resolver, "ghost-vm", tmp.path());
        assert_eq!(outcome, BundleGateOutcome::NotANetVm);
    }

    #[test]
    fn missing_dnsmasq_conf_is_drift() {
        let resolver = build_resolver();
        let tmp = tempfile::tempdir().unwrap();
        let outcome = check_net_vm_bundle_gate(&resolver, "sys-work-net", tmp.path());
        match outcome {
            BundleGateOutcome::Drift(BundleGateDrift::ConfigMissing { env, path }) => {
                assert_eq!(env, "work");
                assert_eq!(path, tmp.path().join("work.conf"));
            }
            other => panic!("expected ConfigMissing, got {other:?}"),
        }
    }

    #[test]
    fn matching_hash_is_ok() {
        let resolver = build_resolver();
        let tmp = tempfile::tempdir().unwrap();
        write_matching_conf(tmp.path(), "work", &resolver);
        let outcome = check_net_vm_bundle_gate(&resolver, "sys-work-net", tmp.path());
        assert_eq!(outcome, BundleGateOutcome::Ok);
    }

    #[test]
    fn divergent_bytes_surface_hash_mismatch() {
        let resolver = build_resolver();
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("work.conf");
        fs::write(&path, b"stale dnsmasq.conf bytes\n").unwrap();
        let outcome = check_net_vm_bundle_gate(&resolver, "sys-work-net", tmp.path());
        match outcome {
            BundleGateOutcome::Drift(BundleGateDrift::HashMismatch {
                env,
                expected,
                actual,
                ..
            }) => {
                assert_eq!(env, "work");
                assert_ne!(expected, actual);
                assert_eq!(expected.len(), 64, "expected sha256 hex");
                assert_eq!(actual.len(), 64, "actual sha256 hex");
            }
            other => panic!("expected HashMismatch, got {other:?}"),
        }
    }

    #[test]
    fn expected_hash_is_deterministic() {
        let r1 = build_resolver();
        let r2 = build_resolver();
        let h1 = compute_expected_dnsmasq_hash(&r1, "work");
        let h2 = compute_expected_dnsmasq_hash(&r2, "work");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn drift_reason_redacts_paths() {
        let drift = BundleGateDrift::HashMismatch {
            env: "work".to_owned(),
            path: PathBuf::from("/var/lib/nixling/dnsmasq/work.conf"),
            expected: "a".repeat(64),
            actual: "b".repeat(64),
        };
        let reason = drift.reason();
        assert!(reason.contains("'work'"));
        assert!(reason.contains("rebuild required"));
        // No file path in reason; the path is retrieved via path().
        assert!(!reason.contains("/var/lib"));
    }

    #[test]
    fn drift_path_accessor_returns_offending_path() {
        let drift = BundleGateDrift::ConfigMissing {
            env: "work".to_owned(),
            path: PathBuf::from("/var/lib/nixling/dnsmasq/work.conf"),
        };
        assert_eq!(
            drift.path(),
            PathBuf::from("/var/lib/nixling/dnsmasq/work.conf")
        );
        assert_eq!(drift.env(), "work");
    }
}
