//! Hermetic layout-engine conformance for volume-local.
//!
//! Every case drives the controller over the scripted effect port, so the
//! policy decisions are proven without a filesystem, a broker, or a
//! privileged host.

use std::collections::BTreeSet;

use d2b_provider_volume_local::testing::{PortCall, ScriptedPort, block_on, fixtures};
use d2b_provider_volume_local::{
    ConditionSeverity, DriftClass, LayoutPhase, MarkerState, ObservedEntry, OwnerProof,
    VolumeLocalController, VolumeLocalError, VolumeLocalProfile,
};

fn controller(port: &ScriptedPort) -> VolumeLocalController<&ScriptedPort, &ScriptedPort> {
    VolumeLocalController::new(VolumeLocalProfile::shipped(), port, port)
}

#[test]
fn an_absent_entry_is_provisioned_and_reported_ready() {
    let port = ScriptedPort::empty();
    let report =
        block_on(controller(&port).reconcile(&fixtures::volume_uid(), &fixtures::state_volume()))
            .expect("reconcile succeeds");
    assert_eq!(report.layout_phase, LayoutPhase::Ready);
    assert!(report.layout_conditions.is_empty());
    assert!(
        port.calls()
            .iter()
            .any(|call| matches!(call, PortCall::Provision(_)))
    );
}

#[test]
fn a_converged_entry_requests_no_effect() {
    let port = ScriptedPort::converged();
    let report =
        block_on(controller(&port).reconcile(&fixtures::volume_uid(), &fixtures::state_volume()))
            .expect("reconcile succeeds");
    assert_eq!(report.layout_phase, LayoutPhase::Ready);
    assert!(
        !port
            .calls()
            .iter()
            .any(|call| matches!(call, PortCall::Provision(_) | PortCall::Repair(_)))
    );
}

#[test]
fn owner_drift_is_repaired_under_the_exact_owner_policy() {
    let mut drifted = ObservedEntry::conformant(OwnerProof::NotApplicable);
    drifted.drift = BTreeSet::from([DriftClass::Owner]);
    let port = ScriptedPort::converged().with_observation("", drifted);
    let report =
        block_on(controller(&port).reconcile(&fixtures::volume_uid(), &fixtures::state_volume()))
            .expect("reconcile succeeds");
    assert_eq!(report.layout_phase, LayoutPhase::Ready);
    assert!(
        port.calls()
            .iter()
            .any(|call| matches!(call, PortCall::Repair(_)))
    );
}

#[test]
fn unrepairable_drift_degrades_instead_of_silently_converging() {
    let mut drifted = ObservedEntry::conformant(OwnerProof::NotApplicable);
    drifted.drift = BTreeSet::from([DriftClass::Mode]);
    let port = ScriptedPort::converged().with_observation("", drifted);
    let report =
        block_on(controller(&port).reconcile(&fixtures::volume_uid(), &fixtures::state_volume()))
            .expect("reconcile succeeds");
    assert_eq!(report.layout_phase, LayoutPhase::Degraded);
    assert_eq!(
        report.layout_conditions[0].reason,
        VolumeLocalError::EntryDrift
    );
}

#[test]
fn a_symlink_on_a_no_follow_walk_fails_closed_and_mutates_nothing() {
    let mut observed = ObservedEntry::conformant(OwnerProof::NotApplicable);
    observed.symlink_encountered = true;
    let port = ScriptedPort::converged().with_observation("", observed);
    let report =
        block_on(controller(&port).reconcile(&fixtures::volume_uid(), &fixtures::state_volume()))
            .expect("reconcile reports");
    assert_eq!(report.layout_phase, LayoutPhase::Failed);
    assert_eq!(
        report.layout_conditions[0].reason,
        VolumeLocalError::SymlinkTraversalRejected
    );
    assert_eq!(
        report.layout_conditions[0].severity,
        ConditionSeverity::Failed
    );
    assert!(!port.calls().iter().any(|call| matches!(
        call,
        PortCall::Provision(_) | PortCall::Repair(_) | PortCall::Cleanup(_)
    )));
}

#[test]
fn a_wrong_entry_class_or_split_filesystem_is_a_fail_closed_invariant() {
    for drift in [DriftClass::EntryType, DriftClass::SameFilesystem] {
        let mut observed = ObservedEntry::conformant(OwnerProof::NotApplicable);
        observed.drift = BTreeSet::from([drift]);
        let port = ScriptedPort::converged().with_observation("", observed);
        let report = block_on(
            controller(&port).reconcile(&fixtures::volume_uid(), &fixtures::state_volume()),
        )
        .expect("reconcile reports");
        assert_eq!(report.layout_phase, LayoutPhase::Failed);
        assert_eq!(
            report.layout_conditions[0].reason,
            VolumeLocalError::InvariantViolated
        );
    }
}

#[test]
fn ambiguous_ownership_quarantines_rather_than_deleting_or_reusing() {
    let port = ScriptedPort::converged()
        .with_observation("", ObservedEntry::conformant(OwnerProof::Unknown));
    let report =
        block_on(controller(&port).reconcile(&fixtures::volume_uid(), &fixtures::state_volume()))
            .expect("reconcile reports");
    assert_eq!(report.layout_phase, LayoutPhase::Degraded);
    assert_eq!(
        report.layout_conditions[0].reason,
        VolumeLocalError::EntryQuarantined
    );
    assert!(
        !port
            .calls()
            .iter()
            .any(|call| matches!(call, PortCall::Cleanup(_)))
    );
}

#[test]
fn declared_acls_are_re_applied_on_every_repair_cycle() {
    let port = ScriptedPort::converged();
    let report = block_on(
        controller(&port).reconcile(&fixtures::volume_uid(), &fixtures::acl_volume("preserve")),
    )
    .expect("reconcile succeeds");
    assert_eq!(report.layout_phase, LayoutPhase::Ready);
    assert!(
        port.calls()
            .iter()
            .any(|call| matches!(call, PortCall::ApplyAcl(_)))
    );
}

#[test]
fn a_foreign_child_acl_is_preserved_or_reported_per_policy() {
    let foreign = |policy: &str| {
        let mut observed = ObservedEntry::conformant(OwnerProof::NotApplicable);
        observed.foreign_children = true;
        let port = ScriptedPort::converged().with_observation("", observed);
        block_on(
            controller(&port).reconcile(&fixtures::volume_uid(), &fixtures::acl_volume(policy)),
        )
        .expect("reconcile reports")
    };
    assert_eq!(foreign("preserve").layout_phase, LayoutPhase::Ready);
    let failing = foreign("fail");
    assert_eq!(failing.layout_phase, LayoutPhase::Degraded);
    assert_eq!(
        failing.layout_conditions[0].reason,
        VolumeLocalError::ForeignAclViolation
    );
}

#[test]
fn a_provisioned_marker_with_missing_state_never_re_provisions() {
    let port = ScriptedPort::empty().with_marker(MarkerState::Provisioned);
    let report =
        block_on(controller(&port).reconcile(&fixtures::volume_uid(), &fixtures::swtpm_volume()))
            .expect("reconcile reports");
    assert_eq!(report.layout_phase, LayoutPhase::Failed);
    assert_eq!(
        report.layout_conditions[0].reason,
        VolumeLocalError::PreviouslyProvisionedStateMissing
    );
    assert!(
        !port
            .calls()
            .iter()
            .any(|call| matches!(call, PortCall::Provision(_)))
    );
}

#[test]
fn cleanup_preserves_every_never_policy_entry() {
    let port = ScriptedPort::converged();
    let removed = block_on(
        controller(&port).cleanup(&fixtures::volume_uid(), &fixtures::store_view_volume()),
    )
    .expect("cleanup succeeds");
    assert!(removed.is_empty());
    assert!(
        !port
            .calls()
            .iter()
            .any(|call| matches!(call, PortCall::Cleanup(_)))
    );
}

#[test]
fn hard_quota_on_a_filesystem_that_cannot_enforce_it_fails_the_volume() {
    use d2b_provider_volume_local::QuotaCapability;
    let spec: d2b_contracts::v3::volume::VolumeSpec = serde_json::from_value(serde_json::json!({
        "source": {
            "executionRef": "Host/host-system",
            "settings": { "kind": "tmpfs" },
        },
        "kind": "ephemeral",
        "layout": [],
        "views": { "controller": { "path": "", "rights": ["read", "traverse"] } },
        "quota": { "maxBytes": 1048576, "maxInodes": 1024, "enforcement": "hard" },
    }))
    .expect("conformant fixture");
    let port = ScriptedPort::empty().with_quota(QuotaCapability::Unenforceable);
    assert_eq!(
        block_on(controller(&port).reconcile(&fixtures::volume_uid(), &spec)).unwrap_err(),
        VolumeLocalError::QuotaUnenforceable
    );
}
