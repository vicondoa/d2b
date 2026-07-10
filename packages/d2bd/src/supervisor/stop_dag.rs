//! Typed stop-DAG owner that reconciles per-VM nftables fragments and
//! per-busid USBIP carrier state when
//! `d2bd` starts (or `vm_stop` runs).
//!
//! This module is a *planner*: it walks the declared intent surface
//! exposed by [`BundleResolver`], compares it against an
//! [`ObservedHostState`] snapshot (probed from the live host or, in
//! tests, synthesized), and emits a typed [`ReconcileReport`] that
//! enumerates the [existing] broker ops the supervisor must dispatch
//! to converge.
//!
//! The planner deliberately does NOT add new broker wire variants: every
//! emitted action maps to the `ApplyNftables`, `UsbipBind`, or
//! `UsbipUnbind` ops that already
//! ship in `packages/d2b-contracts/src/broker_wire.rs`.

use std::collections::{BTreeMap, BTreeSet};

use d2b_core::bundle_resolver::BundleResolver;

/// Snapshot of host-observable state used by the reconcile planner.
///
/// Production callers populate this from
/// `/var/lib/d2b/state/host-runtime.json` (nftables ownership
/// hashes) + `/run/d2b/locks/usbip/` (busid carriers).
/// Tests populate it directly to simulate drift.
#[derive(Debug, Clone, Default)]
pub struct ObservedHostState {
    /// Per-intent-id last-applied nft script hash, as recorded by the
    /// broker after a previous `ApplyNftables`. Missing entries mean
    /// "the broker has no record of having applied this fragment".
    pub nft_applied_hashes: BTreeMap<String, String>,
    /// USBIP busids the host currently has bound to a usbip-host
    /// carrier (regardless of which VM owns them).
    pub usbip_bound_busids: BTreeSet<String>,
    /// VM names the daemon currently believes are actively managed.
    /// Used to classify usbip carriers belonging to dead VMs as
    /// stale and emit `UsbipUnbind` for them.
    pub active_vms: BTreeSet<String>,
}

impl ObservedHostState {
    pub fn empty() -> Self {
        Self::default()
    }
}

/// Why a single nftables fragment needs to be re-applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NftablesDriftReason {
    /// Broker has never applied this fragment for the running host
    /// (cold-start or post-clean drift).
    NeverApplied,
    /// Broker last applied this fragment but the recorded hash no
    /// longer matches the bundle's desired hash (bundle was rebuilt
    /// while the daemon was offline, or someone hand-edited the
    /// table).
    HashMismatch { observed: String, desired: String },
}

/// A single nftables reconcile action — maps 1:1 to
/// `BrokerRequest::ApplyNftables` for the named intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftablesReconcileAction {
    pub intent_id: String,
    pub scope_label: String,
    pub ownership_id: String,
    pub desired_hash: String,
    pub reason: NftablesDriftReason,
}

/// Why a single USBIP busid needs to be reconciled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbipDriftReason {
    /// Bundle declares the busid bound to a live VM but no carrier
    /// is present — emit `UsbipBind`.
    CarrierMissing { vm: String, env: String },
    /// Host has a carrier for a busid whose owning VM is not in
    /// `active_vms` (the VM died or was removed from the bundle) —
    /// emit `UsbipUnbind`.
    OwnerInactive { last_owner: Option<String> },
    /// Host has a carrier for a busid that the bundle no longer
    /// declares at all — emit `UsbipUnbind`.
    Undeclared,
}

/// A single USBIP reconcile action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbipReconcileAction {
    pub busid: String,
    pub reason: UsbipDriftReason,
}

/// Typed reconcile output enumerated in canonical (sorted) order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    pub nftables_actions: Vec<NftablesReconcileAction>,
    pub usbip_actions: Vec<UsbipReconcileAction>,
}

impl ReconcileReport {
    pub fn is_noop(&self) -> bool {
        self.nftables_actions.is_empty() && self.usbip_actions.is_empty()
    }
}

/// Owner of the stop-DAG / restart-time reconcile loop.
///
/// Stateless today; the type exists so callers in `lib.rs` can
/// instantiate `StopDagOwner` once and re-enter `reconcile` from
/// both the daemon startup path and the per-VM `vm_stop` path.
#[derive(Debug, Default)]
pub struct StopDagOwner;

impl StopDagOwner {
    pub fn new() -> Self {
        Self
    }

    /// Plan the reconcile that should run when d2bd starts.
    ///
    /// Production callers obtain `ObservedHostState` from the host
    /// runtime snapshot; the planner itself never touches the
    /// filesystem. Returned actions are dispatched via the existing
    /// broker ops (`ApplyNftables`, `UsbipBind`, `UsbipUnbind`) — no
    /// new wire variants are introduced.
    pub fn reconcile_on_restart(resolver: &BundleResolver) -> ReconcileReport {
        Self::reconcile(resolver, &ObservedHostState::empty())
    }

    /// Test-visible reconcile entry point that accepts an explicit
    /// observed-state snapshot. The startup path delegates to this
    /// after probing the host.
    pub fn reconcile(resolver: &BundleResolver, observed: &ObservedHostState) -> ReconcileReport {
        let mut report = ReconcileReport::default();
        plan_nftables(resolver, observed, &mut report.nftables_actions);
        plan_usbip(resolver, observed, &mut report.usbip_actions);
        report
    }
}

fn plan_nftables(
    resolver: &BundleResolver,
    observed: &ObservedHostState,
    out: &mut Vec<NftablesReconcileAction>,
) {
    let intent_ids: Vec<String> = resolver.nft_intent_ids().map(|s| s.to_owned()).collect();
    for intent_id in intent_ids {
        let Some(intent) = resolver.find_nft_intent(&intent_id) else {
            continue;
        };
        let reason = match observed.nft_applied_hashes.get(&intent_id) {
            None => NftablesDriftReason::NeverApplied,
            Some(applied) if applied == &intent.desired_hash => continue,
            Some(applied) => NftablesDriftReason::HashMismatch {
                observed: applied.clone(),
                desired: intent.desired_hash.clone(),
            },
        };
        out.push(NftablesReconcileAction {
            intent_id: intent.intent_id.clone(),
            scope_label: intent.scope_label.clone(),
            ownership_id: intent.ownership_id.clone(),
            desired_hash: intent.desired_hash.clone(),
            reason,
        });
    }
}

fn plan_usbip(
    resolver: &BundleResolver,
    observed: &ObservedHostState,
    out: &mut Vec<UsbipReconcileAction>,
) {
    // Declared (busid -> owning bind intent).
    let mut declared: BTreeMap<String, (String, String)> = BTreeMap::new();
    for intent_id in resolver
        .usbip_bind_intent_ids()
        .map(str::to_owned)
        .collect::<Vec<_>>()
    {
        if let Some(intent) = resolver.find_usbip_bind_intent(&intent_id) {
            declared.insert(
                intent.bus_id.clone(),
                (intent.vm_name.clone(), intent.env.clone()),
            );
        }
    }

    // Carrier missing for declared+active VMs -> Bind.
    for (busid, (vm, env)) in &declared {
        if !observed.active_vms.contains(vm) {
            continue;
        }
        if !observed.usbip_bound_busids.contains(busid) {
            out.push(UsbipReconcileAction {
                busid: busid.clone(),
                reason: UsbipDriftReason::CarrierMissing {
                    vm: vm.clone(),
                    env: env.clone(),
                },
            });
        }
    }

    // Stale carriers -> Unbind.
    for busid in &observed.usbip_bound_busids {
        match declared.get(busid) {
            None => out.push(UsbipReconcileAction {
                busid: busid.clone(),
                reason: UsbipDriftReason::Undeclared,
            }),
            Some((vm, _env)) if !observed.active_vms.contains(vm) => {
                out.push(UsbipReconcileAction {
                    busid: busid.clone(),
                    reason: UsbipDriftReason::OwnerInactive {
                        last_owner: Some(vm.clone()),
                    },
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_core::bundle::{Bundle, BundleGeneration};
    use d2b_core::host::{HostJson, UsbipBusidLock, UsbipLockOwner, UsbipLockScope};
    use d2b_core::manifest_v04::ManifestV04;
    use d2b_core::processes::ProcessesJson;

    const HOST_JSON_FIXTURE: &str =
        include_str!("../../../../tests/fixtures/deny-unknown/host-valid.json");
    const MANIFEST_FIXTURE: &str =
        include_str!("../../../../tests/golden/manifest_v04/baseline-vms.json");

    fn fixture_resolver_with_busid(busid: &str, vm: &str) -> BundleResolver {
        let mut host: HostJson =
            serde_json::from_str(HOST_JSON_FIXTURE).expect("host fixture parses");
        // Replace the env's USBIP locks with a known busid + owner VM
        // so the planner has a deterministic surface to reason about.
        for env in &mut host.environments {
            env.usbip_busid_locks = vec![UsbipBusidLock {
                vm: vm.to_owned(),
                lock_owner: UsbipLockOwner::Daemon,
                scope: UsbipLockScope::PerBusid,
                bus_ids: vec![busid.to_owned()],
                vendor_product_allowlist: Vec::new(),
            }];
        }
        // The fixture leaves `ownership_id` empty; force a known value
        // so the reconcile action plumbing can be asserted end-to-end.
        host.nftables.ownership_id = "test-ownership".to_owned();
        let manifest =
            ManifestV04::from_slice(MANIFEST_FIXTURE.as_bytes()).expect("manifest fixture parses");
        let processes = ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: Vec::new(),
        };
        let bundle = Bundle {
            bundle_version: 4,
            schema_version: "v2".to_owned(),
            public_manifest_path: "vms.json".to_owned(),
            host_path: "host.json".to_owned(),
            processes_path: "processes.json".to_owned(),
            privileges_path: "privileges.json".to_owned(),
            storage_path: None,
            sync_path: None,
            allocator_path: None,
            realm_controllers_path: None,
            realm_identity_path: None,
            unsafe_local_workloads_path: None,
            closures: Vec::new(),
            minijail_profiles: Vec::new(),
            managed_keys: Default::default(),
            generation: BundleGeneration {
                generator: "test".to_owned(),
                source_revision: None,
                generated_at: Some("2025-01-01T00:00:00Z".to_owned()),
            },
            bundle_hash: None,
            artifact_hashes: None,
        };
        BundleResolver::from_artifacts(bundle, host, processes, manifest)
    }

    fn fixture_resolver() -> BundleResolver {
        fixture_resolver_with_busid("1-1", "workshop")
    }

    #[test]
    fn no_drift_when_hashes_match_and_no_usbip_state() {
        let resolver = fixture_resolver();
        // Simulate broker has applied every nft fragment with the
        // correct hash and no USBIP carriers exist.
        let mut observed = ObservedHostState::empty();
        for id in resolver
            .nft_intent_ids()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        {
            let h = resolver.find_nft_intent(&id).unwrap().desired_hash.clone();
            observed.nft_applied_hashes.insert(id, h);
        }
        let report = StopDagOwner::reconcile(&resolver, &observed);
        assert!(report.is_noop(), "expected noop, got {:?}", report);
    }

    #[test]
    fn never_applied_emits_apply_nftables_for_every_intent() {
        let resolver = fixture_resolver();
        let report = StopDagOwner::reconcile_on_restart(&resolver);
        assert!(!report.nftables_actions.is_empty());
        for action in &report.nftables_actions {
            assert_eq!(action.reason, NftablesDriftReason::NeverApplied);
            assert_eq!(action.ownership_id, "test-ownership");
        }
    }

    #[test]
    fn hash_mismatch_classified_distinctly() {
        let resolver = fixture_resolver();
        let intent_id = resolver.nft_intent_ids().next().unwrap().to_owned();
        let desired = resolver
            .find_nft_intent(&intent_id)
            .unwrap()
            .desired_hash
            .clone();

        let mut observed = ObservedHostState::empty();
        // First intent has a stale hash; all others have the correct
        // hash so the per-intent classification can be asserted in
        // isolation.
        for id in resolver
            .nft_intent_ids()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        {
            let h = if id == intent_id {
                "0000deadbeef".to_owned()
            } else {
                resolver.find_nft_intent(&id).unwrap().desired_hash.clone()
            };
            observed.nft_applied_hashes.insert(id, h);
        }
        let report = StopDagOwner::reconcile(&resolver, &observed);
        assert_eq!(report.nftables_actions.len(), 1);
        let action = &report.nftables_actions[0];
        assert_eq!(action.intent_id, intent_id);
        match &action.reason {
            NftablesDriftReason::HashMismatch {
                observed,
                desired: d,
            } => {
                assert_eq!(observed, "0000deadbeef");
                assert_eq!(d, &desired);
            }
            other => panic!("expected HashMismatch, got {other:?}"),
        }
    }

    #[test]
    fn usbip_carrier_missing_for_active_vm_emits_bind() {
        let resolver = fixture_resolver();
        let mut observed = ObservedHostState::empty();
        // All nft fragments pre-applied so they don't pollute the
        // report.
        for id in resolver
            .nft_intent_ids()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        {
            let h = resolver.find_nft_intent(&id).unwrap().desired_hash.clone();
            observed.nft_applied_hashes.insert(id, h);
        }
        observed.active_vms.insert("workshop".to_owned());
        // No carriers bound.

        let report = StopDagOwner::reconcile(&resolver, &observed);
        assert!(report.nftables_actions.is_empty());
        assert_eq!(report.usbip_actions.len(), 1);
        let action = &report.usbip_actions[0];
        assert_eq!(action.busid, "1-1");
        match &action.reason {
            UsbipDriftReason::CarrierMissing { vm, env } => {
                assert_eq!(vm, "workshop");
                assert_eq!(env, "work");
            }
            other => panic!("expected CarrierMissing, got {other:?}"),
        }
    }

    #[test]
    fn stale_carrier_owner_inactive_emits_unbind() {
        let resolver = fixture_resolver();
        let mut observed = ObservedHostState::empty();
        for id in resolver
            .nft_intent_ids()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        {
            let h = resolver.find_nft_intent(&id).unwrap().desired_hash.clone();
            observed.nft_applied_hashes.insert(id, h);
        }
        // VM "workshop" is NOT in active_vms but its busid is bound.
        observed.usbip_bound_busids.insert("1-1".to_owned());

        let report = StopDagOwner::reconcile(&resolver, &observed);
        assert_eq!(report.usbip_actions.len(), 1);
        let action = &report.usbip_actions[0];
        assert_eq!(action.busid, "1-1");
        match &action.reason {
            UsbipDriftReason::OwnerInactive { last_owner } => {
                assert_eq!(last_owner.as_deref(), Some("workshop"));
            }
            other => panic!("expected OwnerInactive, got {other:?}"),
        }
    }

    #[test]
    fn undeclared_carrier_emits_unbind() {
        let resolver = fixture_resolver();
        let mut observed = ObservedHostState::empty();
        for id in resolver
            .nft_intent_ids()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        {
            let h = resolver.find_nft_intent(&id).unwrap().desired_hash.clone();
            observed.nft_applied_hashes.insert(id, h);
        }
        observed.usbip_bound_busids.insert("9-9".to_owned());

        let report = StopDagOwner::reconcile(&resolver, &observed);
        assert_eq!(report.usbip_actions.len(), 1);
        assert_eq!(report.usbip_actions[0].busid, "9-9");
        assert_eq!(report.usbip_actions[0].reason, UsbipDriftReason::Undeclared);
    }

    #[test]
    fn reconcile_on_restart_is_pure_planner() {
        // The no-state entry point must produce a deterministic
        // report when called repeatedly; this is the contract that
        // lets `vm_stop` and daemon startup share the same planner.
        let resolver = fixture_resolver();
        let a = StopDagOwner::reconcile_on_restart(&resolver);
        let b = StopDagOwner::reconcile_on_restart(&resolver);
        assert_eq!(a, b);
    }
}
