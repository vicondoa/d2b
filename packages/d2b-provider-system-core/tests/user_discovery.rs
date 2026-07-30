//! Local User discovery and status.
//!
//! `Provider/system-core` owns local User discovery and status
//! (`ADR-046-provider-model-and-packaging`, section "system-core
//! bootstrap"). These cases prove the decision logic over a scripted
//! discovery port: what counts as discovered, what counts as drift, and
//! that nothing the port resolved reaches public status.

use d2b_contracts::v3::resource_status::ResourcePhase;
use d2b_provider_system_core::testing::{
    SCRIPTED_IDENTITY, ScriptedDiscoveryPort, block_on, fixtures,
};
use d2b_provider_system_core::{
    SystemCoreError, UserBinding, UserDiscoveryCondition, UserReconciler,
};

#[test]
fn a_fully_verified_user_is_discovered() {
    let reconciler = UserReconciler::new(ScriptedDiscoveryPort::resolving([
        UserBinding::NssRecord,
        UserBinding::PrimaryGroup,
    ]));
    let status = block_on(reconciler.reconcile(&fixtures::user_ref(), &fixtures::user_spec()))
        .expect("discovery reports");
    assert_eq!(status.phase, ResourcePhase::Ready);
    assert_eq!(status.discovery, UserDiscoveryCondition::Discovered);
    assert_eq!(status.identity, Some(SCRIPTED_IDENTITY));
    assert_eq!(reconciler.port().call_count(), 1);
}

#[test]
fn an_unresolved_user_is_absent_rather_than_a_failure() {
    let reconciler = UserReconciler::new(ScriptedDiscoveryPort::absent());
    let status = block_on(reconciler.reconcile(&fixtures::user_ref(), &fixtures::user_spec()))
        .expect("discovery reports");
    assert_eq!(status.phase, ResourcePhase::Pending);
    assert_eq!(status.discovery, UserDiscoveryCondition::Absent);
    assert_eq!(status.identity, None);
}

#[test]
fn declared_groups_are_required_and_their_absence_is_drift_not_readiness() {
    let spec = fixtures::user_spec_with_groups();
    assert!(
        UserReconciler::<ScriptedDiscoveryPort>::required_bindings(&spec)
            .contains(&UserBinding::GroupMemberships)
    );
    let reconciler = UserReconciler::new(ScriptedDiscoveryPort::resolving([
        UserBinding::NssRecord,
        UserBinding::PrimaryGroup,
    ]));
    let status =
        block_on(reconciler.reconcile(&fixtures::user_ref(), &spec)).expect("discovery reports");
    assert_eq!(status.phase, ResourcePhase::Degraded);
    assert_eq!(status.discovery, UserDiscoveryCondition::Drifted);

    // The same port is Ready once the memberships verify.
    let complete = UserReconciler::new(ScriptedDiscoveryPort::resolving([
        UserBinding::NssRecord,
        UserBinding::PrimaryGroup,
        UserBinding::GroupMemberships,
    ]));
    let status =
        block_on(complete.reconcile(&fixtures::user_ref(), &spec)).expect("discovery reports");
    assert_eq!(status.phase, ResourcePhase::Ready);
    assert_eq!(status.discovery, UserDiscoveryCondition::Discovered);
}

#[test]
fn a_user_that_declares_no_group_is_not_held_to_a_membership_check() {
    let spec = fixtures::user_spec();
    assert!(
        !UserReconciler::<ScriptedDiscoveryPort>::required_bindings(&spec)
            .contains(&UserBinding::GroupMemberships)
    );
}

#[test]
fn a_single_matching_property_is_never_enough_to_be_this_user() {
    // The pid-reuse guard's rule, applied to identity discovery: one
    // matching property is not identity. A record without its primary
    // group is Unknown and unverified, not Ready and not drift.
    for partial in [
        vec![UserBinding::NssRecord],
        vec![UserBinding::PrimaryGroup],
        vec![UserBinding::UserManager],
        vec![],
    ] {
        let reconciler = UserReconciler::new(ScriptedDiscoveryPort::resolving(partial.clone()));
        let status = block_on(reconciler.reconcile(&fixtures::user_ref(), &fixtures::user_spec()))
            .expect("discovery reports");
        assert_eq!(
            status.phase,
            ResourcePhase::Unknown,
            "{partial:?} must not establish the identity"
        );
        assert_eq!(status.discovery, UserDiscoveryCondition::Unverified);
    }
}

#[test]
fn a_discovery_failure_propagates_rather_than_being_reported_as_absent() {
    let reconciler = UserReconciler::new(ScriptedDiscoveryPort::failing(
        SystemCoreError::DiscoveryUnavailable,
    ));
    assert_eq!(
        block_on(reconciler.reconcile(&fixtures::user_ref(), &fixtures::user_spec())).unwrap_err(),
        SystemCoreError::DiscoveryUnavailable
    );
}

#[test]
fn user_status_is_redacted() {
    const FORBIDDEN: [&str; 12] = [
        "pid",
        "pidfd",
        "unit",
        "invocation",
        "cgroup",
        "path",
        "argv",
        "command",
        "binary",
        "env",
        "uid",
        "gid",
    ];
    let reconciler = UserReconciler::new(ScriptedDiscoveryPort::resolving([
        UserBinding::NssRecord,
        UserBinding::PrimaryGroup,
    ]));
    let status = block_on(reconciler.reconcile(&fixtures::user_ref(), &fixtures::user_spec()))
        .expect("discovery reports");
    let rendered = serde_json::to_value(&status).expect("status serializes");
    let object = rendered.as_object().expect("status is an object");
    for key in object.keys() {
        let lowered = key.to_ascii_lowercase();
        for fragment in FORBIDDEN {
            assert!(
                !lowered.contains(fragment),
                "public status key {key} carries the forbidden fragment {fragment}"
            );
        }
    }
    // The declared OS username is never restated in status; the User
    // reference is, because a resource reference is public status.
    let serialized = serde_json::to_string(&status).expect("status serializes");
    assert!(!serialized.contains(fixtures::OS_USERNAME));
    assert!(!serialized.contains("osUsername"));
    assert_eq!(format!("{status:?}"), "UserStatusReport(<redacted>)");
    assert_eq!(
        format!("{:?}", SCRIPTED_IDENTITY),
        "UserIdentityDigest(<redacted>)"
    );
}
