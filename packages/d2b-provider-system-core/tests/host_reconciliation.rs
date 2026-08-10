//! Host reconciliation, including the non-negotiable no-isolation posture.
//!
//! The posture obligations are stated in
//! `ADR-046-telemetry-audit-and-support`, section "Host resource status":
//! the reconciler sets `isolationPosture` and `isolationPostureMessage` on
//! every user-only Host unconditionally, operators can neither suppress nor
//! override them, and a Host with a different execution policy does not
//! receive the field at all.

use std::{collections::BTreeSet, future::Future};

use d2b_contracts::v3::execution_policy::ExecutionDomain;
use d2b_contracts::v3::host::IsolationPosture;
use d2b_contracts::v3::resource_status::ResourcePhase;
use d2b_provider_system_core::testing::{block_on, fixtures};
use d2b_provider_system_core::{
    HostCapabilityClass, HostProbeEffectPort, HostProbeMetadata, HostReconciler,
    ISOLATION_POSTURE_MESSAGE, MinijailPlatformGate, NO_ISOLATION_STATUS_FIELDS, SystemCoreError,
};

#[test]
fn a_user_only_host_always_carries_the_no_isolation_posture() {
    let status = HostReconciler::new()
        .reconcile(
            &fixtures::user_only_host_ref(),
            &fixtures::system_core_provider_ref(),
            &fixtures::user_only_host_spec(),
        )
        .expect("the user-only host reconciles");
    assert_eq!(status.phase, ResourcePhase::Ready);
    assert_eq!(
        status.isolation_posture,
        Some(IsolationPosture::NoIsolation)
    );
    assert_eq!(
        status.isolation_posture_message,
        Some(ISOLATION_POSTURE_MESSAGE)
    );
    assert!(status.is_no_isolation());
    assert_eq!(status.default_domain, ExecutionDomain::User);
    assert_eq!(status.allowed_domains, vec![ExecutionDomain::User]);
    assert_eq!(status.default_user_ref, Some(fixtures::user_ref()));
}

#[test]
fn a_host_with_another_execution_policy_carries_no_posture() {
    let status = HostReconciler::new()
        .reconcile(
            &fixtures::host_ref(),
            &fixtures::system_core_provider_ref(),
            &fixtures::system_host_spec(),
        )
        .expect("the system host reconciles");
    // Absent, not "has isolation": the posture is a property of the
    // user-only Host and says nothing about any other Host shape.
    assert_eq!(status.isolation_posture, None);
    assert_eq!(status.isolation_posture_message, None);
    assert!(!status.is_no_isolation());
    assert_eq!(status.default_domain, ExecutionDomain::System);
    assert_eq!(status.default_user_ref, None);
}

#[test]
fn the_posture_is_derived_from_the_spec_and_never_from_a_submitted_status() {
    // Both spellings of an override attempt are refused: naming the field
    // with a value, and naming it as an explicit null to suppress it.
    for submitted in [
        serde_json::json!({"phase": "Ready", "isolationPosture": "none"}),
        serde_json::json!({"phase": "Ready", "isolationPosture": null}),
        serde_json::json!({"isolationPostureMessage": "this host is safe"}),
    ] {
        assert_eq!(
            HostReconciler::reject_operator_status_fields(&submitted).unwrap_err(),
            SystemCoreError::OperatorSuppliedStatusField
        );
    }
    // A status naming neither field is admitted unchanged.
    HostReconciler::reject_operator_status_fields(&serde_json::json!({"phase": "Ready"}))
        .expect("an unreserved status is admitted");
    // A status that is not an object is refused rather than ignored.
    assert_eq!(
        HostReconciler::reject_operator_status_fields(&serde_json::json!("Ready")).unwrap_err(),
        SystemCoreError::StatusNotAnObject
    );
    assert_eq!(NO_ISOLATION_STATUS_FIELDS.len(), 2);
}

#[test]
fn a_user_domain_host_must_name_the_exact_user_it_resolves() {
    // The unsafe-local predecessor enforced this as an allowed-uid set. In
    // v3 the execution policy itself refuses to construct a Host that
    // admits the user domain without naming the exact User, so a spec that
    // could reach the reconciler in that state does not exist. The
    // reconciler keeps its own check as defence in depth; this case pins
    // where the rule is actually enforced, so a later relaxation of the
    // primitive is visible here rather than silent.
    use d2b_contracts::v3::execution_policy::{BudgetSpec, ExecutionPolicy, PrimitiveSpecError};

    assert_eq!(
        ExecutionPolicy::new(
            ExecutionDomain::System,
            vec![ExecutionDomain::System, ExecutionDomain::User],
            None,
            BudgetSpec::default(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap_err(),
        PrimitiveSpecError::MissingRequiredField
    );

    // A mixed-domain Host that does name the User reconciles, and carries
    // that exact reference into status.
    let policy = ExecutionPolicy::new(
        ExecutionDomain::System,
        vec![ExecutionDomain::System, ExecutionDomain::User],
        Some(fixtures::user_ref()),
        BudgetSpec::default(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("the mixed-domain policy is well formed");
    let spec = d2b_contracts::v3::host::HostSpec::new(policy, None)
        .expect("a mixed-domain host carries no posture");
    let status = HostReconciler::new()
        .reconcile(
            &fixtures::host_ref(),
            &fixtures::system_core_provider_ref(),
            &spec,
        )
        .expect("the mixed-domain host reconciles");
    assert_eq!(status.default_user_ref, Some(fixtures::user_ref()));
    assert_eq!(status.isolation_posture, None);
}

#[test]
fn host_status_is_redacted() {
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
    let status = HostReconciler::new()
        .reconcile(
            &fixtures::user_only_host_ref(),
            &fixtures::system_core_provider_ref(),
            &fixtures::user_only_host_spec(),
        )
        .expect("the user-only host reconciles");
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
    assert_eq!(format!("{status:?}"), "HostStatusReport(<redacted>)");
    // The Host and User references render redacted even though they are
    // serialized, so a diagnostic cannot echo a resource name.
    assert_eq!(format!("{:?}", status.host_ref), "ResourceRef(<redacted>)");
}

struct Probe {
    capabilities: BTreeSet<HostCapabilityClass>,
    metadata: HostProbeMetadata,
    gate: MinijailPlatformGate,
}

impl HostProbeEffectPort for Probe {
    async fn probe(&self, capability: HostCapabilityClass) -> Result<bool, SystemCoreError> {
        Ok(self.capabilities.contains(&capability))
    }

    async fn platform(&self) -> Result<MinijailPlatformGate, SystemCoreError> {
        Ok(self.gate)
    }

    fn metadata(&self) -> impl Future<Output = Result<HostProbeMetadata, SystemCoreError>> {
        std::future::ready(Ok(self.metadata.clone()))
    }
}

#[test]
fn bounded_probe_reconciles_all_capabilities_and_gates_minijail() {
    let probe = Probe {
        capabilities: HostCapabilityClass::ALL.into_iter().collect(),
        metadata: HostProbeMetadata {
            kernel_release: "6.1".to_owned(),
            os_name: "test-os".to_owned(),
            user_manager_available: true,
            active_process_count: 3,
        },
        gate: MinijailPlatformGate::new(6, 1, true),
    };
    let result = block_on(HostReconciler::new().reconcile_with_probe(
        &fixtures::host_ref(),
        &fixtures::system_core_provider_ref(),
        &fixtures::system_host_spec(),
        &probe,
        &BTreeSet::from([HostCapabilityClass::Pidfd]),
        true,
    ))
    .expect("the complete bounded probe passes");
    assert_eq!(result.capabilities.len(), HostCapabilityClass::ALL.len());
    assert_eq!(result.active_process_count, 3);
    assert!(result.minijail_ready);
}

#[test]
fn user_capable_host_without_user_manager_is_degraded() {
    let probe = Probe {
        capabilities: HostCapabilityClass::ALL.into_iter().collect(),
        metadata: HostProbeMetadata {
            kernel_release: "6.1".to_owned(),
            os_name: "test-os".to_owned(),
            user_manager_available: false,
            active_process_count: 0,
        },
        gate: MinijailPlatformGate::new(6, 1, true),
    };
    let result = block_on(HostReconciler::new().reconcile_with_probe(
        &fixtures::user_only_host_ref(),
        &fixtures::system_core_provider_ref(),
        &fixtures::user_only_host_spec(),
        &probe,
        &BTreeSet::new(),
        false,
    ))
    .expect("the host policy itself is valid");
    assert_eq!(result.status.phase, ResourcePhase::Degraded);
}
