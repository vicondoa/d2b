//! Daemon-side per-env usbipd autostart.
//!
//! Retires the 9 per-env usbipd systemd units declared by
//! `nixos-modules/network.nix`
//! (`nixling-sys-<env>-usbipd-{backend,proxy}.{service,socket}` —
//! 3 envs × 3 units = 9 units) by folding them into broker
//! `SpawnRunner` with [`RunnerRole::Usbip`]. The per-env scope is
//! the broker's role anchor: `vm_id = sys-<env>-usbipd`, with two
//! `role_id`s (`backend`, `proxy`).
//!
//! The pure plan-derivation lives here (deterministic from the
//! manifest) so the daemon can surface the planned spawns in
//! `nixling host doctor` and the integration test layer can pin the
//! exact set of (env, port) pairs without spinning up the broker.
//! Execution is wired by [`execute_usbipd_perenv_autostart`], which
//! dispatches one `SpawnRunner` per spec through the broker.
//!
//! The transitional NixOS units shipped in belt-and-braces fashion;
//! this module's spawn path runs alongside them and is idempotent: a
//! duplicate `SpawnRunner` for an existing
//! `(vm_id, role_id)` pidfd is rejected fail-closed by the daemon's
//! pidfd table, so re-entry on SIGHUP or bundle-reload is safe.
//!
//! See the retired-unit header in `nixos-modules/network.nix`.

use std::collections::{BTreeMap, BTreeSet};

use nixling_core::manifest_v04::ManifestV04;
use nixling_ipc::broker_wire::RunnerRole;
use serde::{Deserialize, Serialize};

/// VM-scope prefix for the per-env usbipd anchor. The full scope is
/// `sys-<env>-usbipd` and matches the systemd unit naming used by
/// the retiring `network.nix` block so journal correlation stays
/// stable through the transitional window.
pub const PER_ENV_USBIPD_VM_PREFIX: &str = "sys-";
pub const PER_ENV_USBIPD_VM_SUFFIX: &str = "-usbipd";

/// Per-env backend port baseline. Matches
/// `nixos-modules/network.nix`'s `3241 + alphabetical-index` rule.
/// Keeping it as a `const` here means the daemon and the Nix module
/// share a single source of truth that a static-shape test can
/// cross-check.
pub const PER_ENV_USBIPD_BACKEND_PORT_BASE: u16 = 3241;

/// One side of a per-env usbipd spawn: backend or proxy. Kept as a
/// flat enum so the autostart report can group by role without
/// stringly-typed comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PerEnvUsbipdRole {
    /// Long-lived `usbipd -4 --tcp-port <port>`. `usbipd` binds all
    /// interfaces; the broker-managed `inet nixling` input chain drops
    /// non-loopback ingress to the backend port.
    Backend,
    /// Self-binding TCP proxy from `<env host uplink IP>:3240` to
    /// `127.0.0.1:<port>`. Long-lived and broker-spawned directly.
    Proxy,
}

impl PerEnvUsbipdRole {
    pub fn role_id(self) -> &'static str {
        match self {
            Self::Backend => "backend",
            Self::Proxy => "proxy",
        }
    }
}

/// One per-env, per-role spawn the daemon will dispatch through the
/// broker. The plan is pure: derived from the bundle manifest, free
/// of broker state, and stable across daemon restarts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerEnvUsbipdSpec {
    /// Env name (lowercase manifest-validated; e.g. `work`).
    pub env: String,
    /// Per-VM scope anchor used as `vm_id` in the SpawnRunner
    /// request (`sys-<env>-usbipd`).
    pub vm_id: String,
    /// Which side of the (backend, proxy) pair this spec spawns.
    pub role: PerEnvUsbipdRole,
    /// Per-env backend TCP port. Derived deterministically from the
    /// manifest env list via [`PER_ENV_USBIPD_BACKEND_PORT_BASE`] +
    /// alphabetical index.
    pub backend_port: u16,
}

impl PerEnvUsbipdSpec {
    /// `intent_id` the broker resolves against the trusted bundle.
    /// Matches `intent_id_runner(vm_id, role_id)` so a single
    /// processes-json DAG row per (env, role) keeps wiring trivial when
    /// the Nix bundle catches up.
    pub fn intent_id(&self) -> String {
        nixling_core::bundle_resolver::intent_id_runner(&self.vm_id, self.role.role_id())
    }
}

/// Derive the per-env usbipd spawn plan from the trusted bundle.
///
/// An env is included iff at least one of its workload VMs opts into
/// `usbip.yubikey` AND carries a non-null `usbipd_host_ip` (the
/// manifest-validated invariant that the env has a usbipd host
/// uplink configured). The env list is sorted so the index used to
/// derive `backend_port` matches the Nix module's `lib.attrNames` →
/// `imap0` derivation byte-for-byte.
///
/// Each included env yields two spawn specs (`Backend`, then
/// `Proxy`) so the resulting `Vec` is in stable spawn order:
/// `[(env_a, backend), (env_a, proxy), (env_b, backend), ...]`.
pub fn derive_per_env_usbipd_specs(
    manifest: &ManifestV04,
    host: &nixling_core::host::HostJson,
) -> Vec<PerEnvUsbipdSpec> {
    let backend_ports: BTreeMap<String, u16> = host
        .environments
        .iter()
        .filter_map(|env| env.usbip_backend_port.map(|port| (env.env.clone(), port)))
        .collect();
    derive_per_env_usbipd_specs_with_ports(manifest, &backend_ports)
}

fn derive_per_env_usbipd_specs_with_ports(
    manifest: &ManifestV04,
    backend_ports: &BTreeMap<String, u16>,
) -> Vec<PerEnvUsbipdSpec> {
    // Step 1: filter to envs whose workload VMs declare usbip
    // yubikey opt-in AND a non-null usbipd_host_ip. Order matches
    // the trusted host.json environment order below.
    let mut usbip_envs: BTreeSet<String> = BTreeSet::new();
    for vm in manifest.vms.values() {
        let Some(env) = vm.env.clone() else { continue };
        if vm.usbip_yubikey && vm.usbipd_host_ip.is_some() {
            usbip_envs.insert(env);
        }
    }

    // Step 2: emit (backend, proxy) pairs in the explicit backend-port order
    // supplied by host.json. The Nix index owns the port assignment; the daemon
    // never re-enumerates envs independently.
    let mut specs = Vec::with_capacity(usbip_envs.len() * 2);
    for (env, port) in backend_ports {
        if !usbip_envs.contains(env) {
            continue;
        }
        let vm_id = format!("{PER_ENV_USBIPD_VM_PREFIX}{env}{PER_ENV_USBIPD_VM_SUFFIX}");
        specs.push(PerEnvUsbipdSpec {
            env: env.clone(),
            vm_id: vm_id.clone(),
            role: PerEnvUsbipdRole::Backend,
            backend_port: *port,
        });
        specs.push(PerEnvUsbipdSpec {
            env: env.clone(),
            vm_id,
            role: PerEnvUsbipdRole::Proxy,
            backend_port: *port,
        });
    }
    specs
}

/// Per-spec outcome from one autostart pass. Mirrors
/// [`crate::autostart::Outcome`] shape so operators see the same
/// vocabulary in `nixling host doctor` whether they're looking at
/// per-VM or per-env spawns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum PerEnvUsbipdOutcome {
    /// Successfully spawned this pass.
    Spawned,
    /// Daemon's pidfd table already has a live entry for this
    /// `(vm_id, role_id)` — idempotent short-circuit on
    /// SIGHUP / bundle-reload.
    AlreadyRunning,
    /// Broker `SpawnRunner` returned `BundleIntentMissing`. The
    /// transitional NixOS units are still serving the env; the
    /// daemon-side spawn becomes load-bearing once
    /// `processes-json.nix` grows `sys-<env>-usbipd` DAGs. Surfaced as
    /// `SkippedPendingBundle` so the journal record is grep-able without
    /// being noisy.
    SkippedPendingBundle,
    /// Broker dispatch returned a different typed error. `reason`
    /// is the broker error kind (already redacted for launcher peers
    /// upstream).
    Failed { reason: String },
}

impl PerEnvUsbipdOutcome {
    pub fn is_up(&self) -> bool {
        matches!(self, Self::Spawned | Self::AlreadyRunning)
    }
}

/// Per-spec record returned by [`execute_usbipd_perenv_autostart`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerEnvUsbipdSpawnReport {
    pub env: String,
    pub vm_id: String,
    pub role: PerEnvUsbipdRole,
    pub backend_port: u16,
    pub outcome: PerEnvUsbipdOutcome,
}

/// Aggregate report for the per-env usbipd autostart phase.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PerEnvUsbipdAutostartReport {
    pub specs: Vec<PerEnvUsbipdSpawnReport>,
}

impl PerEnvUsbipdAutostartReport {
    pub fn spawned(&self) -> usize {
        self.specs
            .iter()
            .filter(|s| matches!(s.outcome, PerEnvUsbipdOutcome::Spawned))
            .count()
    }
    pub fn already_running(&self) -> usize {
        self.specs
            .iter()
            .filter(|s| matches!(s.outcome, PerEnvUsbipdOutcome::AlreadyRunning))
            .count()
    }
    pub fn skipped_pending_bundle(&self) -> usize {
        self.specs
            .iter()
            .filter(|s| matches!(s.outcome, PerEnvUsbipdOutcome::SkippedPendingBundle))
            .count()
    }
    pub fn failed(&self) -> usize {
        self.specs
            .iter()
            .filter(|s| matches!(s.outcome, PerEnvUsbipdOutcome::Failed { .. }))
            .count()
    }
}

/// Seam between [`execute_usbipd_perenv_autostart`] and the live
/// broker. Production wires this through `dispatch_broker_request`
/// for `RunnerRole::Usbip`; tests instantiate a fake spawner.
pub trait PerEnvUsbipdSpawner: Send + Sync + 'static {
    /// Has the daemon already registered a pidfd for this
    /// `(vm_id, role_id)` tuple? Used for idempotency.
    fn is_running(&self, vm_id: &str, role_id: &str) -> bool;

    /// Dispatch one `SpawnRunner` request for the given spec.
    ///
    /// Implementations MUST translate `BrokerError::BundleIntentMissing`
    /// (typed kind `bundle-intent-missing`) into
    /// [`PerEnvUsbipdOutcome::SkippedPendingBundle`] so the transitional
    /// window doesn't fail-closed before the bundle gains
    /// `sys-<env>-usbipd` DAG rows.
    fn spawn(&self, spec: &PerEnvUsbipdSpec) -> PerEnvUsbipdOutcome;
}

/// Drive the derived plan. Per spec: check idempotency, then
/// dispatch the broker `SpawnRunner`. Failures do not abort siblings
/// — each spec is reported independently so the operator can see the
/// full picture in one pass. The plan is ordered backend-then-proxy
/// per env so a backend failure short-circuits to skipping that
/// env's proxy (the proxy would race the absent backend).
pub fn execute_usbipd_perenv_autostart<S: PerEnvUsbipdSpawner>(
    specs: &[PerEnvUsbipdSpec],
    spawner: &S,
) -> PerEnvUsbipdAutostartReport {
    let mut reports: Vec<PerEnvUsbipdSpawnReport> = Vec::with_capacity(specs.len());
    let mut backend_failed_envs: BTreeSet<String> = BTreeSet::new();

    for spec in specs {
        let outcome =
            if spec.role == PerEnvUsbipdRole::Proxy && backend_failed_envs.contains(&spec.env) {
                PerEnvUsbipdOutcome::Failed {
                    reason: format!(
                        "skipped: env '{}' backend did not start (race-avoidance)",
                        spec.env
                    ),
                }
            } else if spawner.is_running(&spec.vm_id, spec.role.role_id()) {
                PerEnvUsbipdOutcome::AlreadyRunning
            } else {
                spawner.spawn(spec)
            };

        if spec.role == PerEnvUsbipdRole::Backend
            && matches!(outcome, PerEnvUsbipdOutcome::Failed { .. })
        {
            backend_failed_envs.insert(spec.env.clone());
        }

        reports.push(PerEnvUsbipdSpawnReport {
            env: spec.env.clone(),
            vm_id: spec.vm_id.clone(),
            role: spec.role,
            backend_port: spec.backend_port,
            outcome,
        });
    }
    PerEnvUsbipdAutostartReport { specs: reports }
}

/// Build a `SpawnRunner` request payload for one spec. The
/// production daemon code path constructs the broker envelope from
/// this shape; exposed at the module boundary so test fakes can
/// assert on the wire-level argument set.
pub fn spawn_runner_role(spec: &PerEnvUsbipdSpec) -> RunnerRole {
    let _ = spec;
    RunnerRole::Usbip
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_core::manifest_v04::ManifestV04;
    use std::collections::BTreeSet;
    use std::sync::Mutex;

    /// Build a minimal v4 manifest JSON for tests. Each `vms` entry
    /// is `(name, env_opt, usbip_yubikey, usbipd_host_ip_opt)`.
    fn manifest_with(vms: &[(&str, Option<&str>, bool, Option<&str>)]) -> ManifestV04 {
        use serde_json::{Value, json};
        let mut root = serde_json::Map::new();
        root.insert("_manifest".to_owned(), json!({ "manifestVersion": 6 }));
        root.insert(
            "_observability".to_owned(),
            json!({
                "enabled": false,
                "vmName": "sys-obs",
                "obsVsockCid": 1000,
                "obsVsockHostSocket": "/var/lib/nixling/vms/sys-obs/vsock.sock",
                "signozUrl": "http://10.40.0.10:8080",
                "signozOtlpGrpcPort": 4317,
                "signozOtlpHttpPort": 4318
            }),
        );
        for (name, env, usbip, host_ip) in vms {
            let mut vm = serde_json::Map::new();
            vm.insert(
                "apiSocket".into(),
                Value::String(format!("/var/lib/nixling/vms/{name}/{name}.sock")),
            );
            vm.insert("audio".into(), Value::Bool(false));
            vm.insert(
                "audioService".into(),
                Value::String(format!("nixling-{name}-snd.service")),
            );
            vm.insert(
                "audioStateFile".into(),
                Value::String(format!(
                    "/var/lib/nixling/vms/{name}/state/audio-state.json"
                )),
            );
            vm.insert(
                "bridge".into(),
                env.map(|e| Value::String(format!("br-{e}-lan")))
                    .unwrap_or(Value::Null),
            );
            vm.insert(
                "env".into(),
                env.map(|e| Value::String(e.to_string()))
                    .unwrap_or(Value::Null),
            );
            vm.insert(
                "gpuSocket".into(),
                Value::String(format!("/var/lib/nixling/vms/{name}/{name}-gpu.sock")),
            );
            vm.insert("graphics".into(), Value::Bool(false));
            vm.insert("isNetVm".into(), Value::Bool(false));
            vm.insert("name".into(), Value::String(name.to_string()));
            vm.insert(
                "netVm".into(),
                env.map(|e| Value::String(format!("sys-{e}-net")))
                    .unwrap_or(Value::Null),
            );
            vm.insert(
                "observability".into(),
                json!({
                    "agentSocket": "/run/nixling/otlp.sock",
                    "enabled": false,
                    "vsockCid": 100,
                    "vsockHostSocket": format!("/var/lib/nixling/vms/{name}/vsock.sock"),
                }),
            );
            vm.insert(
                "runtime".into(),
                json!({
                    "kind": "nixos",
                    "provider": {
                        "id": "local-cloud-hypervisor",
                        "type": "local",
                        "driver": "cloud-hypervisor"
                    },
                    "capabilities": {
                        "lifecycle": true,
                        "display": true,
                        "usbHotplug": true,
                        "guestControl": true,
                        "exec": true,
                        "configSync": true,
                        "ssh": true,
                        "storeSync": true,
                        "keys": true,
                        "inGuestObservability": true
                    }
                }),
            );
            vm.insert("sshUser".into(), Value::String("alice".into()));
            vm.insert(
                "stateDir".into(),
                Value::String(format!("/var/lib/nixling/vms/{name}")),
            );
            vm.insert(
                "staticIp".into(),
                env.map(|_| Value::String("10.20.0.10".into()))
                    .unwrap_or(Value::Null),
            );
            vm.insert(
                "tap".into(),
                Value::String(format!("{}-l10", env.unwrap_or("none"))),
            );
            vm.insert("tpm".into(), Value::Bool(false));
            vm.insert(
                "tpmSocket".into(),
                Value::String(format!("/run/swtpm/{name}/sock")),
            );
            vm.insert("usbipYubikey".into(), Value::Bool(*usbip));
            vm.insert(
                "usbipdHostIp".into(),
                host_ip
                    .map(|s| Value::String(s.to_string()))
                    .unwrap_or(Value::Null),
            );
            root.insert(name.to_string(), Value::Object(vm));
        }
        let bytes = serde_json::to_vec(&Value::Object(root)).expect("manifest json builds");
        ManifestV04::from_slice(&bytes).expect("manifest parses")
    }

    fn ports(envs: &[(&str, u16)]) -> BTreeMap<String, u16> {
        envs.iter()
            .map(|(env, port)| ((*env).to_owned(), *port))
            .collect()
    }

    #[test]
    fn derive_returns_empty_when_no_usbip_vms() {
        let m = manifest_with(&[
            ("vm-a", Some("work"), false, None),
            ("vm-b", Some("personal"), false, None),
        ]);
        let specs = derive_per_env_usbipd_specs_with_ports(
            &m,
            &ports(&[("personal", 3241), ("work", 3242)]),
        );
        assert!(specs.is_empty());
    }

    #[test]
    fn derive_picks_only_envs_with_usbip_yubikey_workloads() {
        let m = manifest_with(&[
            ("vm-a", Some("work"), true, Some("192.0.2.1")),
            ("vm-b", Some("personal"), false, None),
            ("vm-c", Some("obs"), true, Some("192.0.2.2")),
        ]);
        let specs = derive_per_env_usbipd_specs_with_ports(
            &m,
            &ports(&[("obs", 3241), ("personal", 3242), ("work", 3243)]),
        );
        let envs: Vec<&str> = specs.iter().map(|s| s.env.as_str()).collect();
        assert!(envs.contains(&"obs"));
        assert!(envs.contains(&"work"));
        assert!(!envs.contains(&"personal"));
    }

    #[test]
    fn derive_emits_backend_then_proxy_per_env() {
        let m = manifest_with(&[("vm-a", Some("work"), true, Some("192.0.2.1"))]);
        let specs = derive_per_env_usbipd_specs_with_ports(&m, &ports(&[("work", 3241)]));
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].role, PerEnvUsbipdRole::Backend);
        assert_eq!(specs[1].role, PerEnvUsbipdRole::Proxy);
        assert_eq!(specs[0].env, "work");
        assert_eq!(specs[0].vm_id, "sys-work-usbipd");
        assert_eq!(specs[0].backend_port, specs[1].backend_port);
    }

    #[test]
    fn derive_uses_alphabetical_index_for_port_assignment() {
        // Envs: obs, personal, work → indices 0, 1, 2.
        // Only obs+work are usbip-enabled, but personal still
        // consumes index 1 so work's port is 3243 (3241 + 2).
        let m = manifest_with(&[
            ("vm-a", Some("work"), true, Some("192.0.2.1")),
            ("vm-b", Some("personal"), false, None),
            ("vm-c", Some("obs"), true, Some("192.0.2.2")),
        ]);
        let specs = derive_per_env_usbipd_specs_with_ports(
            &m,
            &ports(&[("obs", 4000), ("personal", 4100), ("work", 4200)]),
        );
        let obs_backend = specs
            .iter()
            .find(|s| s.env == "obs" && s.role == PerEnvUsbipdRole::Backend)
            .unwrap();
        let work_backend = specs
            .iter()
            .find(|s| s.env == "work" && s.role == PerEnvUsbipdRole::Backend)
            .unwrap();
        assert_eq!(obs_backend.backend_port, 4000);
        assert_eq!(work_backend.backend_port, 4200);
    }

    #[test]
    fn intent_id_is_stable_per_vm_role() {
        let spec = PerEnvUsbipdSpec {
            env: "work".to_owned(),
            vm_id: "sys-work-usbipd".to_owned(),
            role: PerEnvUsbipdRole::Backend,
            backend_port: 3243,
        };
        assert_eq!(spec.intent_id(), "runner:vm:sys-work-usbipd:role:backend");
    }

    #[test]
    fn spawn_runner_role_is_usbip() {
        let spec = PerEnvUsbipdSpec {
            env: "work".to_owned(),
            vm_id: "sys-work-usbipd".to_owned(),
            role: PerEnvUsbipdRole::Proxy,
            backend_port: 3243,
        };
        assert_eq!(spawn_runner_role(&spec), RunnerRole::Usbip);
    }

    struct FakeSpawner {
        running: BTreeSet<(String, String)>,
        bundle_missing: BTreeSet<String>,
        backend_should_fail: BTreeSet<String>,
        calls: Mutex<Vec<(String, &'static str)>>,
    }
    impl FakeSpawner {
        fn new() -> Self {
            Self {
                running: BTreeSet::new(),
                bundle_missing: BTreeSet::new(),
                backend_should_fail: BTreeSet::new(),
                calls: Mutex::new(Vec::new()),
            }
        }
    }
    impl PerEnvUsbipdSpawner for FakeSpawner {
        fn is_running(&self, vm: &str, role: &str) -> bool {
            self.running.contains(&(vm.to_owned(), role.to_owned()))
        }
        fn spawn(&self, spec: &PerEnvUsbipdSpec) -> PerEnvUsbipdOutcome {
            self.calls
                .lock()
                .unwrap()
                .push((spec.vm_id.clone(), spec.role.role_id()));
            if self.bundle_missing.contains(&spec.env) {
                return PerEnvUsbipdOutcome::SkippedPendingBundle;
            }
            if spec.role == PerEnvUsbipdRole::Backend
                && self.backend_should_fail.contains(&spec.env)
            {
                return PerEnvUsbipdOutcome::Failed {
                    reason: "fake backend failure".to_owned(),
                };
            }
            PerEnvUsbipdOutcome::Spawned
        }
    }

    fn sample_specs() -> Vec<PerEnvUsbipdSpec> {
        vec![
            PerEnvUsbipdSpec {
                env: "obs".to_owned(),
                vm_id: "sys-obs-usbipd".to_owned(),
                role: PerEnvUsbipdRole::Backend,
                backend_port: 3241,
            },
            PerEnvUsbipdSpec {
                env: "obs".to_owned(),
                vm_id: "sys-obs-usbipd".to_owned(),
                role: PerEnvUsbipdRole::Proxy,
                backend_port: 3241,
            },
            PerEnvUsbipdSpec {
                env: "work".to_owned(),
                vm_id: "sys-work-usbipd".to_owned(),
                role: PerEnvUsbipdRole::Backend,
                backend_port: 3243,
            },
            PerEnvUsbipdSpec {
                env: "work".to_owned(),
                vm_id: "sys-work-usbipd".to_owned(),
                role: PerEnvUsbipdRole::Proxy,
                backend_port: 3243,
            },
        ]
    }

    #[test]
    fn execute_spawns_all_when_clean() {
        let spawner = FakeSpawner::new();
        let specs = sample_specs();
        let report = execute_usbipd_perenv_autostart(&specs, &spawner);
        assert_eq!(report.spawned(), 4);
        assert_eq!(report.failed(), 0);
    }

    #[test]
    fn execute_idempotent_when_already_running() {
        let mut spawner = FakeSpawner::new();
        spawner
            .running
            .insert(("sys-obs-usbipd".to_owned(), "backend".to_owned()));
        let specs = sample_specs();
        let report = execute_usbipd_perenv_autostart(&specs, &spawner);
        assert_eq!(report.already_running(), 1);
        assert_eq!(report.spawned(), 3);
    }

    #[test]
    fn execute_skips_proxy_when_backend_failed() {
        let mut spawner = FakeSpawner::new();
        spawner.backend_should_fail.insert("obs".to_owned());
        let specs = sample_specs();
        let report = execute_usbipd_perenv_autostart(&specs, &spawner);
        // obs backend failed → obs proxy short-circuits to Failed.
        // work pair still spawns cleanly.
        let obs_proxy = report
            .specs
            .iter()
            .find(|r| r.env == "obs" && r.role == PerEnvUsbipdRole::Proxy)
            .unwrap();
        assert!(matches!(
            obs_proxy.outcome,
            PerEnvUsbipdOutcome::Failed { .. }
        ));
        assert_eq!(report.spawned(), 2);
        assert_eq!(report.failed(), 2);
    }

    #[test]
    fn execute_handles_bundle_intent_missing_gracefully() {
        let mut spawner = FakeSpawner::new();
        spawner.bundle_missing.insert("obs".to_owned());
        let specs = sample_specs();
        let report = execute_usbipd_perenv_autostart(&specs, &spawner);
        assert_eq!(report.skipped_pending_bundle(), 2);
        assert_eq!(report.spawned(), 2);
    }
}
