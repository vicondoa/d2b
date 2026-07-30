//! The ownership boundary: `system-core` reconciles Host and User, and
//! refuses everything else.
//!
//! `ADR-046-provider-model-and-packaging`, section "system-core bootstrap",
//! states the negative list explicitly. These cases hold the boundary from
//! the outside, so a later caller cannot widen the most privileged Provider
//! in the Zone by handing it a ResourceType the specification denied it.

use d2b_contracts::v3::ResourceRef;
use d2b_provider_system_core::testing::{ScriptedDiscoveryPort, block_on, fixtures};
use d2b_provider_system_core::{
    DISOWNED_RESOURCE_TYPES, HostReconciler, OWNED_RESOURCE_TYPES, SystemCoreError, UserReconciler,
};

fn reference(resource_type: &str) -> ResourceRef {
    ResourceRef::parse(&format!("{resource_type}/some-resource")).expect("valid fixture reference")
}

#[test]
fn the_owned_set_is_exactly_host_and_user() {
    assert_eq!(OWNED_RESOURCE_TYPES, ["Host", "User"]);
}

#[test]
fn host_reconciliation_refuses_every_disowned_resource_type() {
    let reconciler = HostReconciler::new();
    for disowned in DISOWNED_RESOURCE_TYPES {
        assert_eq!(
            reconciler
                .reconcile(
                    &reference(disowned),
                    &fixtures::system_core_provider_ref(),
                    &fixtures::system_host_spec(),
                )
                .unwrap_err(),
            SystemCoreError::ResourceTypeNotOwned,
            "{disowned} must be refused"
        );
    }
}

#[test]
fn a_user_reference_is_not_a_host_and_a_host_reference_is_not_a_user() {
    // Both types are owned, so this proves the per-call type check runs
    // rather than the allowlist alone letting an owned type through the
    // wrong reconciler.
    let host = HostReconciler::new();
    assert_eq!(
        host.reconcile(
            &fixtures::user_ref(),
            &fixtures::system_core_provider_ref(),
            &fixtures::system_host_spec(),
        )
        .unwrap_err(),
        SystemCoreError::ResourceTypeNotOwned
    );

    let user = UserReconciler::new(ScriptedDiscoveryPort::absent());
    assert_eq!(
        block_on(user.reconcile(&fixtures::host_ref(), &fixtures::user_spec())).unwrap_err(),
        SystemCoreError::ResourceTypeNotOwned
    );
    // The refusal happens before the effect port is reached, so a denied
    // ResourceType causes no host effect at all.
    assert_eq!(user.port().call_count(), 0);
}

#[test]
fn a_guest_is_not_a_host() {
    // A Guest is an execution parent like a Host, and it is the nearest
    // neighbour to mistakenly accept. It belongs to a runtime Provider.
    let reconciler = HostReconciler::new();
    assert_eq!(
        reconciler
            .reconcile(
                &reference("Guest"),
                &fixtures::system_core_provider_ref(),
                &fixtures::system_host_spec(),
            )
            .unwrap_err(),
        SystemCoreError::ResourceTypeNotOwned
    );
}

#[test]
fn a_host_declaring_a_foreign_provider_is_refused() {
    let reconciler = HostReconciler::new();
    let foreign = ResourceRef::parse("Provider/runtime-cloud-hypervisor").expect("valid reference");
    assert_eq!(
        reconciler
            .reconcile(
                &fixtures::host_ref(),
                &foreign,
                &fixtures::system_host_spec()
            )
            .unwrap_err(),
        SystemCoreError::ProviderRefMismatch
    );
}
