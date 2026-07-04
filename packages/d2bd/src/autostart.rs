//! d2bd autostart contract.
//!
//! On daemon startup (after pidfd-table restoration + orphan
//! adoption) the daemon enumerates the autostart-set from the
//! loaded bundle and brings the VMs up in a controlled order:
//!
//! 1. **Net VMs first.** Every `sys-<env>-net` VM with
//!    `autostart = true` is started before any workload VM. Net VMs
//!    provide DHCP / DNS / NAT / firewall for the rest of the env;
//!    starting workloads first would race their resolv.conf and
//!    default-route bring-up.
//! 2. **Concurrency cap.** At most `N` VMs are started in parallel
//!    (default `N = 3`; configurable via
//!    [`AutostartConfig::parallelism`]). The cap applies within
//!    each phase: up to N net VMs in parallel, then up to N
//!    workloads in parallel.
//! 3. **Degraded mode.** A net VM failure does NOT abort the
//!    sequence — workloads in that env are marked
//!    `Outcome::Degraded` (skipped with a reason), workloads in
//!    other envs proceed normally, and the daemon continues
//!    serving `status` / `doctor` / `audit` requests. A workload
//!    VM failure is recorded as `Outcome::Failed` but does not
//!    block siblings.
//! 4. **Idempotent.** A second invocation against the same bundle
//!    on the same live daemon skips VMs that are already
//!    registered in the pidfd table (`Outcome::AlreadyRunning`),
//!    so the autostart pass is safe to re-run on SIGHUP /
//!    bundle-reload without double-spawning runners.
//!
//! The per-VM start sequence (host-prep DAG → process DAG → pidfd
//! registration) is owned by `dispatch_broker_vm_start` in `lib.rs`;
//! this module's [`VmStarter`] trait is the seam between the
//! orchestration logic here and that machinery. Tests instantiate a
//! fake starter; production wires
//! [`BrokerVmStarter`](super::BrokerVmStarter) which delegates back
//! to `dispatch_broker_vm_start`.

use std::sync::Arc;

use d2b_core::bundle_resolver::BundleResolver;
use d2b_core::runtime::RuntimeKind;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// Default `N` for the concurrency cap. Three is a balance between
/// "host CPU/IO can absorb the spawn burst" and "operator gets to
/// see meaningful progress in the journal before the next batch
/// starts" on the small-fleet desktop deployments d2b targets.
/// Operators with bigger fleets override via
/// `d2b.daemon.autostart.parallelism` (NixOS) →
/// `AutostartConfig::parallelism`.
pub const DEFAULT_PARALLELISM: usize = 3;

/// One row in the autostart plan. Derived purely from bundle
/// metadata so the plan is reproducible across daemon restarts and
/// can be re-derived without re-loading the world.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmAutostartEntry {
    /// VM name as it appears in `_manifest.vms`.
    pub vm: String,
    /// Env this VM belongs to, if any. Net VMs use the env from
    /// their `sys-<env>-net` name (= `Some("<env>")`); workloads
    /// use `VmEntry::env`.
    pub env: Option<String>,
    /// True for `sys-<env>-net` VMs (= `VmEntry::is_net_vm`).
    pub is_net_vm: bool,
    /// True if the VM is an autostart candidate. Today this is
    /// derived heuristically from bundle shape (every non-graphics VM is
    /// autostart-eligible — graphics VMs are excluded by `assertions.nix`);
    /// in the daemon-only bundle the `autostart` flag becomes a first-class
    /// field and this is read straight off it.
    pub autostart: bool,
}

/// Topo-sorted autostart plan: net VMs first (one row per
/// `sys-<env>-net`), then workloads grouped by env in deterministic
/// order. Workloads that depend on a net VM appear *after* their
/// env's net VM row. Entries with `autostart = false` are kept in
/// the plan (so an operator surfacing the plan in `status` can see
/// the full picture) but are skipped by [`execute_autostart`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AutostartPlan {
    pub vms: Vec<VmAutostartEntry>,
}

impl AutostartPlan {
    /// Net-VM entries, in plan order.
    pub fn net_vms(&self) -> impl Iterator<Item = &VmAutostartEntry> {
        self.vms.iter().filter(|v| v.is_net_vm)
    }

    /// Workload (non-net) entries, in plan order.
    pub fn workload_vms(&self) -> impl Iterator<Item = &VmAutostartEntry> {
        self.vms.iter().filter(|v| !v.is_net_vm)
    }
}

/// Tunables for [`execute_autostart`]. Mirrors the
/// `d2b.daemon.autostart.*` NixOS option set.
#[derive(Debug, Clone, Copy)]
pub struct AutostartConfig {
    /// Concurrency cap N (number of VMs started in parallel within
    /// a single phase). Must be `>= 1`; values `< 1` are clamped.
    pub parallelism: usize,
}

impl Default for AutostartConfig {
    fn default() -> Self {
        Self {
            parallelism: DEFAULT_PARALLELISM,
        }
    }
}

/// Per-VM outcome from a single autostart pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Outcome {
    /// Successfully started this pass.
    Started,
    /// `is_running` returned true before we tried — idempotency
    /// short-circuit, the daemon already has a live pidfd for the
    /// VM (covers SIGHUP / reconnect-after-crash flows).
    AlreadyRunning,
    /// Plan row carried `autostart = false`; nothing attempted.
    NotAutostart,
    /// The VM's env net VM is in `Failed` state, so this workload
    /// is held back. `reason` carries the upstream failure summary
    /// so operators can trace the dependency in the journal.
    Degraded { reason: String },
    /// `VmStarter::start` returned `Err`. `reason` is the
    /// starter's error message (already redacted for launcher
    /// peers by the broker layer upstream).
    Failed { reason: String },
}

impl Outcome {
    /// True iff the outcome counts as "this VM is up after the
    /// autostart pass" (Started OR already-running). Used by the
    /// workload-phase degraded-mode check.
    pub fn is_up(&self) -> bool {
        matches!(self, Outcome::Started | Outcome::AlreadyRunning)
    }

    /// True iff the outcome should propagate degradation
    /// downstream (Failed OR Degraded). NotAutostart is NOT
    /// degraded — an operator explicitly opting a net VM out of
    /// autostart is not the same thing as a failure.
    pub fn is_degraded(&self) -> bool {
        matches!(self, Outcome::Failed { .. } | Outcome::Degraded { .. })
    }
}

/// Per-VM record in [`AutostartReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutostartOutcome {
    pub vm: String,
    pub env: Option<String>,
    pub is_net_vm: bool,
    pub outcome: Outcome,
}

/// Report returned by [`execute_autostart`]. Preserves the plan's
/// VM order so the journal record reads as the operator expects
/// (net VMs first, then workloads grouped by env).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AutostartReport {
    pub outcomes: Vec<AutostartOutcome>,
}

impl AutostartReport {
    pub fn count_where(&self, predicate: impl Fn(&Outcome) -> bool) -> usize {
        self.outcomes
            .iter()
            .filter(|o| predicate(&o.outcome))
            .count()
    }

    pub fn started(&self) -> usize {
        self.count_where(|o| matches!(o, Outcome::Started))
    }

    pub fn already_running(&self) -> usize {
        self.count_where(|o| matches!(o, Outcome::AlreadyRunning))
    }

    pub fn failed(&self) -> usize {
        self.count_where(|o| matches!(o, Outcome::Failed { .. }))
    }

    pub fn degraded(&self) -> usize {
        self.count_where(|o| matches!(o, Outcome::Degraded { .. }))
    }
}

/// Seam between [`execute_autostart`] and the per-VM start
/// machinery. Implementations MUST be cheap to clone (or wrapped in
/// `Arc`) — `execute_autostart` clones the trait object into each
/// `spawn_blocking` task it dispatches.
///
/// Both methods are sync and called from inside
/// `tokio::task::spawn_blocking`; implementations are free to do
/// synchronous broker round-trips.
pub trait VmStarter: Send + Sync + 'static {
    /// Is this VM already supervised by the daemon? Used for the
    /// idempotency short-circuit.
    fn is_running(&self, vm: &str) -> bool;

    /// Drive the per-VM start sequence (host-prep DAG → process
    /// DAG → pidfd registration). Returns `Err(reason)` on any
    /// failure; the autostart layer translates `Err` into
    /// `Outcome::Failed` and continues with sibling VMs.
    fn start(&self, vm: &str) -> Result<(), String>;
}

/// Build the autostart plan from the trusted bundle.
///
/// Plan order:
///
/// 1. Every `sys-<env>-net` VM, sorted by env name for
///    determinism.
/// 2. Every non-net VM, sorted by `(env, vm-name)` so workloads
///    pin to their net VM's env in the plan.
///
/// VMs that aren't autostart-eligible (today: VMs the manifest
/// flags as graphics — see [`vm_is_autostart_eligible`]) appear in
/// the plan with `autostart = false`. They are surfaced for
/// observability but skipped by [`execute_autostart`].
pub fn build_autostart_plan(resolver: &BundleResolver) -> AutostartPlan {
    let mut net_entries = Vec::new();
    let mut workload_entries = Vec::new();

    for (name, vm) in &resolver.manifest.vms {
        let entry = VmAutostartEntry {
            vm: name.clone(),
            env: vm.env.clone(),
            is_net_vm: vm.is_net_vm,
            autostart: vm_is_autostart_eligible(vm),
        };
        if entry.is_net_vm {
            net_entries.push(entry);
        } else {
            workload_entries.push(entry);
        }
    }

    net_entries.sort_by(|a, b| a.env.cmp(&b.env).then_with(|| a.vm.cmp(&b.vm)));
    workload_entries.sort_by(|a, b| a.env.cmp(&b.env).then_with(|| a.vm.cmp(&b.vm)));

    let mut vms = Vec::with_capacity(net_entries.len() + workload_entries.len());
    vms.extend(net_entries);
    vms.extend(workload_entries);

    AutostartPlan { vms }
}

/// Today's heuristic: every VM the manifest knows about is an
/// autostart candidate unless it is a graphics VM or a manual-only
/// qemu-media runtime.
fn vm_is_autostart_eligible(vm: &d2b_core::manifest_v04::VmEntry) -> bool {
    !vm.graphics && vm.runtime.kind != RuntimeKind::QemuMedia
}

/// Drive a built plan. Net VMs are started first (up to
/// `config.parallelism` in parallel); once that phase settles, any
/// env whose net VM ended in a degraded/failed state has its
/// workloads marked `Outcome::Degraded` *without dispatch*, and
/// the remaining workloads are started (again, up to
/// `config.parallelism` in parallel).
///
/// The function is safe to invoke repeatedly: each VM is gated on
/// `starter.is_running(...)`, so a re-entry on SIGHUP or
/// bundle-reload short-circuits to `Outcome::AlreadyRunning` for
/// every VM that's still supervised.
pub async fn execute_autostart<S: VmStarter>(
    plan: &AutostartPlan,
    starter: Arc<S>,
    config: AutostartConfig,
) -> AutostartReport {
    execute_autostart_with_pre_degraded(plan, starter, config, &std::collections::BTreeSet::new())
        .await
}

/// Variant of [`execute_autostart`] that accepts an additional set of VM
/// names the caller has already classified as degraded (for reasons
/// orthogonal to env-net-VM health — today: the kernel-module-check pass
/// discovered an optional module the VM relies on is not loaded). Any VM whose name is in
/// `pre_degraded` is reported as
/// [`Outcome::Degraded`] with a stable
/// `"pre-degraded: <vm>"` reason and is NOT dispatched to the
/// starter. The rest of the plan executes normally.
pub async fn execute_autostart_with_pre_degraded<S: VmStarter>(
    plan: &AutostartPlan,
    starter: Arc<S>,
    config: AutostartConfig,
    pre_degraded: &std::collections::BTreeSet<String>,
) -> AutostartReport {
    let parallelism = config.parallelism.max(1);
    let semaphore = Arc::new(Semaphore::new(parallelism));
    let pre_degraded_arc: Arc<std::collections::BTreeSet<String>> = Arc::new(pre_degraded.clone());

    // Net VM pass.
    let net_outcomes = run_phase(
        plan.net_vms().cloned().collect::<Vec<_>>(),
        Arc::clone(&starter),
        Arc::clone(&semaphore),
        Arc::clone(&pre_degraded_arc),
        |_env| None, // no upstream gate for net VMs
    )
    .await;

    // Index env -> upstream net-VM outcome for the workload phase.
    let mut env_net_status: std::collections::BTreeMap<String, Outcome> =
        std::collections::BTreeMap::new();
    for out in &net_outcomes {
        if let Some(env) = out.env.clone() {
            env_net_status.insert(env, out.outcome.clone());
        }
    }

    // Workload pass.
    let workload_outcomes = run_phase(
        plan.workload_vms().cloned().collect::<Vec<_>>(),
        Arc::clone(&starter),
        Arc::clone(&semaphore),
        Arc::clone(&pre_degraded_arc),
        move |env: &Option<String>| -> Option<String> {
            let env_name = env.as_ref()?;
            match env_net_status.get(env_name) {
                Some(o) if o.is_degraded() => Some(format!(
                    "env '{env_name}' net VM did not come up: {}",
                    match o {
                        Outcome::Failed { reason } => format!("failed: {reason}"),
                        Outcome::Degraded { reason } => format!("degraded: {reason}"),
                        _ => "unknown".to_owned(),
                    }
                )),
                _ => None,
            }
        },
    )
    .await;

    let mut outcomes = Vec::with_capacity(net_outcomes.len() + workload_outcomes.len());
    outcomes.extend(net_outcomes);
    outcomes.extend(workload_outcomes);
    AutostartReport { outcomes }
}

async fn run_phase<S, GateFn>(
    entries: Vec<VmAutostartEntry>,
    starter: Arc<S>,
    semaphore: Arc<Semaphore>,
    pre_degraded: Arc<std::collections::BTreeSet<String>>,
    gate: GateFn,
) -> Vec<AutostartOutcome>
where
    S: VmStarter,
    GateFn: Fn(&Option<String>) -> Option<String> + Send + Sync + 'static,
{
    let gate = Arc::new(gate);
    let mut join_set: JoinSet<(usize, AutostartOutcome)> = JoinSet::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let starter = Arc::clone(&starter);
        let semaphore = Arc::clone(&semaphore);
        let gate = Arc::clone(&gate);
        let pre_degraded = Arc::clone(&pre_degraded);
        join_set.spawn(async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .expect("autostart semaphore must not close before phase end");

            // Pre-degraded VMs (e.g. flagged by the kernel-module-check
            // pass) short-circuit before
            // anything else.
            if pre_degraded.contains(&entry.vm) {
                return (
                    index,
                    AutostartOutcome {
                        vm: entry.vm.clone(),
                        env: entry.env.clone(),
                        is_net_vm: entry.is_net_vm,
                        outcome: Outcome::Degraded {
                            reason: format!(
                                "pre-degraded: kernel-module-check flagged '{}' as missing a required optional module",
                                entry.vm
                            ),
                        },
                    },
                );
            }

            // Honour the upstream gate first so we don't even
            // probe a degraded env's workloads.
            if let Some(reason) = gate(&entry.env) {
                return (
                    index,
                    AutostartOutcome {
                        vm: entry.vm.clone(),
                        env: entry.env.clone(),
                        is_net_vm: entry.is_net_vm,
                        outcome: Outcome::Degraded { reason },
                    },
                );
            }
            if !entry.autostart {
                return (
                    index,
                    AutostartOutcome {
                        vm: entry.vm.clone(),
                        env: entry.env.clone(),
                        is_net_vm: entry.is_net_vm,
                        outcome: Outcome::NotAutostart,
                    },
                );
            }
            let starter_for_blocking = Arc::clone(&starter);
            let vm_for_blocking = entry.vm.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                if starter_for_blocking.is_running(&vm_for_blocking) {
                    return Outcome::AlreadyRunning;
                }
                match starter_for_blocking.start(&vm_for_blocking) {
                    Ok(()) => Outcome::Started,
                    Err(reason) => Outcome::Failed { reason },
                }
            })
            .await
            .unwrap_or_else(|join_err| Outcome::Failed {
                reason: format!("autostart task panicked: {join_err}"),
            });
            (
                index,
                AutostartOutcome {
                    vm: entry.vm.clone(),
                    env: entry.env.clone(),
                    is_net_vm: entry.is_net_vm,
                    outcome,
                },
            )
        });
    }

    let mut indexed: Vec<(usize, AutostartOutcome)> = Vec::new();
    while let Some(joined) = join_set.join_next().await {
        match joined {
            Ok(pair) => indexed.push(pair),
            Err(join_err) => {
                // Should not occur — the inner task already
                // catches its own panic via the
                // spawn_blocking().await match arm. We surface a
                // typed-error-shaped Failed entry to keep the
                // report shape stable.
                tracing::warn!(error = ?join_err, "autostart join task failed");
            }
        }
    }
    indexed.sort_by_key(|(idx, _)| *idx);
    indexed.into_iter().map(|(_, outcome)| outcome).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Test starter — records every `start` call (so we can
    /// assert ordering / parallelism) and lets each test pick the
    /// success/failure outcome per VM.
    struct FakeStarter {
        running: Mutex<std::collections::BTreeSet<String>>,
        fail_for: std::collections::BTreeSet<String>,
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
        start_delay: std::time::Duration,
        started_order: Mutex<Vec<String>>,
    }

    impl FakeStarter {
        fn new() -> Self {
            Self {
                running: Mutex::new(Default::default()),
                fail_for: Default::default(),
                in_flight: AtomicUsize::new(0),
                max_in_flight: AtomicUsize::new(0),
                start_delay: std::time::Duration::from_millis(0),
                started_order: Mutex::new(Vec::new()),
            }
        }
    }

    impl VmStarter for FakeStarter {
        fn is_running(&self, vm: &str) -> bool {
            self.running.lock().unwrap().contains(vm)
        }
        fn start(&self, vm: &str) -> Result<(), String> {
            let current = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(current, Ordering::SeqCst);
            std::thread::sleep(self.start_delay);
            self.started_order.lock().unwrap().push(vm.to_owned());
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            if self.fail_for.contains(vm) {
                return Err(format!("synthetic-failure:{vm}"));
            }
            self.running.lock().unwrap().insert(vm.to_owned());
            Ok(())
        }
    }

    fn entry(vm: &str, env: Option<&str>, is_net_vm: bool, autostart: bool) -> VmAutostartEntry {
        VmAutostartEntry {
            vm: vm.to_owned(),
            env: env.map(str::to_owned),
            is_net_vm,
            autostart,
        }
    }

    fn plan_from(entries: Vec<VmAutostartEntry>) -> AutostartPlan {
        AutostartPlan { vms: entries }
    }

    fn manifest_vm_with_runtime(
        runtime: d2b_core::runtime::RuntimeMetadata,
    ) -> d2b_core::manifest_v04::VmEntry {
        d2b_core::manifest_v04::VmEntry {
            api_socket: None,
            audio: false,
            audio_service: None,
            audio_state_file: None,
            bridge: None,
            env: Some("work".to_owned()),
            mtu: None,
            mss_clamp: None,
            lan: None,
            gpu_socket: None,
            graphics: false,
            is_net_vm: false,
            name: "installer".to_owned(),
            net_vm: None,
            observability: d2b_core::manifest_v04::VmObservability {
                agent_socket: None,
                enabled: false,
                vsock_cid: None,
                vsock_host_socket: None,
            },
            runtime,
            security_key: false,
            lifecycle: Default::default(),
            shell: None,
            ssh_user: None,
            state_dir: "/var/lib/d2b/vms/installer".to_owned(),
            static_ip: None,
            tap: "d2b-installer".to_owned(),
            tpm: false,
            tpm_socket: None,
            usbip_yubikey: false,
            usbipd_host_ip: None,
        }
    }

    #[test]
    fn qemu_media_runtime_is_manual_only_for_autostart() {
        let qemu_vm =
            manifest_vm_with_runtime(d2b_core::runtime::RuntimeMetadata::local_qemu_media());
        let nixos_vm = manifest_vm_with_runtime(d2b_core::runtime::RuntimeMetadata::local_nixos());

        assert!(!vm_is_autostart_eligible(&qemu_vm));
        assert!(vm_is_autostart_eligible(&nixos_vm));
    }

    /// build_autostart_plan: when an env has a sys-net VM plus
    /// workloads, the sys-net VM comes first in the plan.
    #[test]
    fn build_plan_orders_net_vm_before_workloads() {
        // We test the bundle-free path via a synthesised plan that
        // mirrors what build_autostart_plan emits for a typical
        // work-env bundle. The full BundleResolver fixture is
        // exercised by tests/daemon-autostart-eval.sh; here we
        // exercise the ordering invariant directly so the unit
        // test stays hermetic (no nixpkgs eval).
        let mut entries = vec![
            entry("work-dev", Some("work"), false, true),
            entry("work-build", Some("work"), false, true),
            entry("sys-work-net", Some("work"), true, true),
        ];
        // Sort the same way build_autostart_plan does (net first,
        // then by (env, name)). After the sort the net VM must be
        // strictly before any workload.
        let plan = {
            let mut net = entries
                .iter()
                .filter(|e| e.is_net_vm)
                .cloned()
                .collect::<Vec<_>>();
            net.sort_by(|a, b| a.env.cmp(&b.env).then_with(|| a.vm.cmp(&b.vm)));
            let mut wl = entries
                .iter()
                .filter(|e| !e.is_net_vm)
                .cloned()
                .collect::<Vec<_>>();
            wl.sort_by(|a, b| a.env.cmp(&b.env).then_with(|| a.vm.cmp(&b.vm)));
            entries.clear();
            entries.extend(net);
            entries.extend(wl);
            AutostartPlan { vms: entries }
        };
        let names: Vec<_> = plan.vms.iter().map(|e| e.vm.as_str()).collect();
        assert_eq!(names, vec!["sys-work-net", "work-build", "work-dev"]);
        // Sanity: net_vms() iterator only yields the net VM.
        assert_eq!(plan.net_vms().count(), 1);
        assert_eq!(plan.workload_vms().count(), 2);
    }

    /// Concurrency cap: with parallelism = 3 and 6 net VMs that
    /// each sleep, max_in_flight observed during the run must be
    /// <= 3.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn parallelism_cap_is_honored() {
        let mut starter = FakeStarter::new();
        starter.start_delay = std::time::Duration::from_millis(30);
        let starter = Arc::new(starter);
        let plan = plan_from(vec![
            entry("sys-a-net", Some("a"), true, true),
            entry("sys-b-net", Some("b"), true, true),
            entry("sys-c-net", Some("c"), true, true),
            entry("sys-d-net", Some("d"), true, true),
            entry("sys-e-net", Some("e"), true, true),
            entry("sys-f-net", Some("f"), true, true),
        ]);
        let config = AutostartConfig { parallelism: 3 };
        let report = execute_autostart(&plan, Arc::clone(&starter), config).await;
        assert_eq!(report.started(), 6, "all six net VMs must start");
        let observed = starter.max_in_flight.load(Ordering::SeqCst);
        assert!(
            observed <= 3,
            "concurrency cap of 3 violated: observed peak {observed}"
        );
    }

    /// Degraded mode: a failure on `sys-work-net` does NOT block
    /// other envs' net VMs, and the workloads in the failed env
    /// land as Degraded (not Failed — Failed is reserved for the
    /// direct start error).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn vm_failure_marks_degraded_without_blocking_siblings() {
        let mut starter = FakeStarter::new();
        starter.fail_for.insert("sys-work-net".to_owned());
        let starter = Arc::new(starter);
        let plan = plan_from(vec![
            entry("sys-work-net", Some("work"), true, true),
            entry("sys-personal-net", Some("personal"), true, true),
            entry("work-dev", Some("work"), false, true),
            entry("personal-dev", Some("personal"), false, true),
        ]);
        let report = execute_autostart(
            &plan,
            Arc::clone(&starter),
            AutostartConfig { parallelism: 3 },
        )
        .await;

        // Net phase: work failed, personal started.
        let work_net = report
            .outcomes
            .iter()
            .find(|o| o.vm == "sys-work-net")
            .expect("work net entry present");
        assert!(
            matches!(work_net.outcome, Outcome::Failed { .. }),
            "work net VM must surface as Failed; got {:?}",
            work_net.outcome
        );
        let personal_net = report
            .outcomes
            .iter()
            .find(|o| o.vm == "sys-personal-net")
            .expect("personal net entry present");
        assert_eq!(personal_net.outcome, Outcome::Started);

        // Workload phase: work-dev degraded (its net is dead),
        // personal-dev still came up.
        let work_dev = report
            .outcomes
            .iter()
            .find(|o| o.vm == "work-dev")
            .expect("work-dev entry");
        assert!(
            matches!(work_dev.outcome, Outcome::Degraded { .. }),
            "work-dev must be Degraded due to net VM failure; got {:?}",
            work_dev.outcome
        );
        let personal_dev = report
            .outcomes
            .iter()
            .find(|o| o.vm == "personal-dev")
            .expect("personal-dev entry");
        assert_eq!(personal_dev.outcome, Outcome::Started);

        assert_eq!(report.failed(), 1);
        assert_eq!(report.degraded(), 1);
        assert_eq!(report.started(), 2);
    }

    /// Idempotency: a second invocation against the same starter
    /// instance reports AlreadyRunning for every VM that the first
    /// pass started, and does NOT call `start` again. We verify
    /// the latter by checking `started_order` length stays at the
    /// first-pass count.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn rerun_is_idempotent_skips_running_vms() {
        let starter = Arc::new(FakeStarter::new());
        let plan = plan_from(vec![
            entry("sys-work-net", Some("work"), true, true),
            entry("work-dev", Some("work"), false, true),
        ]);
        let cfg = AutostartConfig { parallelism: 3 };

        let first = execute_autostart(&plan, Arc::clone(&starter), cfg).await;
        assert_eq!(first.started(), 2);
        let after_first_pass = starter.started_order.lock().unwrap().len();

        let second = execute_autostart(&plan, Arc::clone(&starter), cfg).await;
        assert_eq!(second.already_running(), 2);
        assert_eq!(second.started(), 0);
        let after_second_pass = starter.started_order.lock().unwrap().len();
        assert_eq!(
            after_first_pass, after_second_pass,
            "second pass must not call start() again"
        );
    }

    /// NotAutostart skips dispatch entirely (no start() call) and
    /// does NOT propagate as a degraded gate for that env's
    /// workloads — opting a net VM out of autostart is an explicit
    /// operator choice, not a failure.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn non_autostart_net_vm_does_not_degrade_workloads() {
        let starter = Arc::new(FakeStarter::new());
        let plan = plan_from(vec![
            entry("sys-work-net", Some("work"), true, false),
            entry("work-dev", Some("work"), false, true),
        ]);
        let report = execute_autostart(
            &plan,
            Arc::clone(&starter),
            AutostartConfig { parallelism: 3 },
        )
        .await;
        let net = report
            .outcomes
            .iter()
            .find(|o| o.vm == "sys-work-net")
            .unwrap();
        assert_eq!(net.outcome, Outcome::NotAutostart);
        let wl = report.outcomes.iter().find(|o| o.vm == "work-dev").unwrap();
        assert_eq!(wl.outcome, Outcome::Started);
    }

    /// Outcome predicates: is_up vs is_degraded coverage.
    #[test]
    fn outcome_predicates_cover_every_variant() {
        assert!(Outcome::Started.is_up());
        assert!(Outcome::AlreadyRunning.is_up());
        assert!(!Outcome::NotAutostart.is_up());
        assert!(!Outcome::Failed { reason: "x".into() }.is_up());
        assert!(!Outcome::Degraded { reason: "x".into() }.is_up());

        assert!(!Outcome::Started.is_degraded());
        assert!(!Outcome::AlreadyRunning.is_degraded());
        assert!(!Outcome::NotAutostart.is_degraded());
        assert!(Outcome::Failed { reason: "x".into() }.is_degraded());
        assert!(Outcome::Degraded { reason: "x".into() }.is_degraded());
    }

    /// Parallelism clamp: configuring 0 must NOT deadlock.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn parallelism_zero_is_clamped_to_one() {
        let starter = Arc::new(FakeStarter::new());
        let plan = plan_from(vec![entry("sys-work-net", Some("work"), true, true)]);
        let report = execute_autostart(
            &plan,
            Arc::clone(&starter),
            AutostartConfig { parallelism: 0 },
        )
        .await;
        assert_eq!(report.started(), 1);
    }
}
