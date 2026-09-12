//! Process-family launch identity and the durable readers the pre-v3 plane
//! still needs.
//!
//! The U12 conversion moved Process/EphemeralProcess reconciliation onto the
//! new plane ([`crate::process_driver`]), so the old typed Runner machinery -
//! `ProcessResourceRuntime`, `ProcessResourceReconciler`, its descriptor, the
//! Guest-local source, and the ephemeral TTL/status helpers - is gone. What
//! stays here until U14 is exactly what other pre-v3 surfaces consume:
//!
//! - [`resolve_launch_identity`], the one canonical launch-identity resolver
//!   every ticket consumer shares (the Process driver, the provider runtime,
//!   and the durable-row helpers), including the guest-runtime target rule
//!   [`guest_runtime_process_matches`];
//! - [`list_process_resources`], the generic Process list the controller
//!   sessions fence against; and
//! - [`PROCESS_RESTART_ANNOTATION`], the persisted restart annotation the
//!   interaction composition's durable Process specs still carry.

use d2b_contracts_resource::v3::{ResourceRef, ResourceTypeName, ResourceUid, ZoneId};
use d2b_process_conformance::{LaunchIdentity, LaunchIdentityError};
use d2b_resource_api::ResourceStoreBackend;
use d2b_resource_store::{
    StoreListRequest, StoreOperationContext, StoreProjection, StoredResource,
};
use d2b_resource_store_redb::RedbResourceStore;
use d2bd_runtime::resource_runtime_support::retry_transient_store_list;

const PROCESS_TYPE: &str = "Process";
const EPHEMERAL_PROCESS_TYPE: &str = "EphemeralProcess";
pub(crate) const PROCESS_RESTART_ANNOTATION: &str = "d2b.d2bus.org/restart-generation";
const GUEST_RUNTIME_PROCESS_TEMPLATES: &[(&str, &str)] = &[
    ("cloud-hypervisor-runner", "-vmm"),
    ("qemu-media-runner", "-qemu"),
];

/// Stable failures for the daemon-owned generic process path.
///
/// The classification surface stays intact for the controller-session
/// mapping (`resource_runtime::map_process_runtime_error` matches every
/// variant) after the U12 conversion retired the runtime that constructed
/// most of them; only [`Self::Store`] is constructed by the list readers that
/// remain.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessResourceRuntimeError {
    /// A durable resource did not decode as the closed Process contract.
    InvalidResource,
    /// The resource selected a Provider not owned by this runtime.
    UnsupportedProvider,
    /// The trusted bundle did not contain the requested template binding.
    TemplateUnavailable,
    /// A process identity was ambiguous during adoption or stop.
    IdentityAmbiguous,
    /// A Provider effect failed.
    ProviderEffect,
    /// A Provider controller bootstrap endpoint did not become readable.
    ControllerBootstrapUnavailable,
    /// A required Process Provider has no committed identity projection.
    ProviderIdentityUnavailable,
    /// A semantic Process owner has no committed identity projection.
    OwnerIdentityUnavailable,
    /// The durable store could not be listed or watched.
    Store,
}

impl core::fmt::Display for ProcessResourceRuntimeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidResource => "process-resource-invalid",
            Self::UnsupportedProvider => "process-resource-provider-unsupported",
            Self::TemplateUnavailable => "process-resource-template-unavailable",
            Self::IdentityAmbiguous => "process-resource-identity-ambiguous",
            Self::ProviderEffect => "process-resource-provider-effect-failed",
            Self::ControllerBootstrapUnavailable => {
                "process-resource-controller-bootstrap-unavailable"
            }
            Self::ProviderIdentityUnavailable => "process-resource-provider-identity-unavailable",
            Self::OwnerIdentityUnavailable => "process-resource-owner-identity-unavailable",
            Self::Store => "process-resource-store-failed",
        })
    }
}

impl std::error::Error for ProcessResourceRuntimeError {}

/// The row inputs one launch identity resolves from.
///
/// Every field either is persisted on the durable row or is authored in its
/// metadata/spec; nothing here is guessed, and no caller derives a subset of
/// the identity on its own.
pub(crate) struct LaunchRow<'a> {
    /// The row's semantic owner (manager owner key, or the authored
    /// `metadata.ownerRef` for an owner the manager does not hold).
    pub(crate) owner_ref: Option<&'a ResourceRef>,
    /// The durable owner linkage the row persists.
    pub(crate) owner_uid: Option<ResourceUid>,
    /// The exact execution target declared by the row's Process spec.
    pub(crate) execution_ref: &'a ResourceRef,
    /// The row's Process name (its legacy runner role id).
    pub(crate) process_name: &'a str,
    /// The template the row's Process spec declares.
    pub(crate) template: &'a str,
    /// A target ref declared for this row together with the owner it belongs
    /// to (the owning `VolumeBinding`'s attachment, or the runtime's target
    /// selector). It is authoritative only for its own owner.
    pub(crate) declared_target: Option<(&'a ResourceRef, &'a ResourceRef)>,
}

/// Resolve the one canonical [`LaunchIdentity`] of a Process row (KTD7).
///
/// The owner ref/UID, execution target, and the legacy role come straight
/// from the row. The launch target is derived here and only here:
///
/// - a declared target (the attachment Guest a `VolumeBinding`-owned worker
///   serves, or the runtime's owner-scoped selector) applies when it belongs
///   to the row's own owner;
/// - otherwise a Guest-owned guest-runtime row - the nested VMM, the qemu
///   media runner - targets its owning Guest (the old `scoped_target_ref`
///   rule), because its signed template executes on the Host while its launch
///   intent is minted for the Guest.
///
/// A row whose launch cannot be named completely fails here, once, naming the
/// missing input ([`LaunchIdentityError`]).
pub(crate) fn resolve_launch_identity(
    row: &LaunchRow<'_>,
) -> Result<LaunchIdentity, LaunchIdentityError> {
    let owner = row.owner_ref;
    let declared_target = match row.declared_target {
        Some((target, target_owner)) if owner == Some(target_owner) => Some(target),
        _ => None,
    };
    // The host-exec/guest-target split only exists for Host execution: a
    // Guest execution target already names its own VM.
    let target_ref = if row.execution_ref.resource_type().as_str() == "Host" {
        declared_target
            .cloned()
            .or_else(|| {
                owner
                    .filter(|owner| {
                        owner.resource_type().as_str() == "Guest"
                            && guest_runtime_process_matches(
                                row.template,
                                row.process_name,
                                owner.name().as_str(),
                            )
                    })
                    .cloned()
            })
    } else {
        None
    };
    let binding_worker = owner.is_some_and(|owner| owner.resource_type().as_str() == "VolumeBinding");
    LaunchIdentity::new(
        owner.cloned(),
        row.owner_uid.clone(),
        row.execution_ref.clone(),
        target_ref,
        row.process_name,
        binding_worker,
    )
}

/// Whether one Template + Process name pair is a Guest-owned guest-runtime
/// process (the nested VMM, the qemu media runner): the rule the old
/// `scoped_target_ref` used to bind such a row's launch target to its owning
/// Guest, now one rule inside [`resolve_launch_identity`].
pub(crate) fn guest_runtime_process_matches(
    template: &str,
    process_name: &str,
    guest_name: &str,
) -> bool {
    GUEST_RUNTIME_PROCESS_TEMPLATES
        .iter()
        .any(|(expected_template, suffix)| {
            template == *expected_template
                && process_name == format!("{guest_name}{suffix}")
        })
}

/// Build the generic Process resource list request.
pub(crate) fn process_resource_list_request(zone: &ZoneId) -> StoreListRequest {
    StoreListRequest {
        operation: StoreOperationContext {
            operation_id: "process-resource-reconcile".to_owned(),
            idempotency_key: None,
            correlation_id: "process-resource-reconcile".to_owned(),
            trace_id: None,
            deadline_ms: 10_000,
        },
        zone: zone.clone(),
        resource_types: vec![
            ResourceTypeName::parse(PROCESS_TYPE).expect("static Process type"),
            ResourceTypeName::parse(EPHEMERAL_PROCESS_TYPE).expect("static EphemeralProcess type"),
        ],
        resource_names: Vec::new(),
        filters: Vec::new(),
        page_size: 256,
        cursor: None,
        projection: StoreProjection::Full,
    }
}

/// Relist generic Process resources from the authoritative Zone store.
pub(crate) async fn list_process_resources(
    store: &RedbResourceStore,
    zone: &ZoneId,
) -> Result<Vec<StoredResource>, ProcessResourceRuntimeError> {
    let mut request = process_resource_list_request(zone);
    let mut resources = Vec::new();
    loop {
        let result = retry_transient_store_list(zone, &request.operation.operation_id, || {
            store.list(request.clone())
        })
        .await
            .map_err(|_| ProcessResourceRuntimeError::Store)?;
        resources.extend(result.resources);
        let Some(cursor) = result.next_cursor else {
            break;
        };
        request.cursor = Some(cursor);
    }
    Ok(resources)
}

/// Relist generic Process resources through a session-bound Resource API
/// backend while preserving the backend's reconnect fence.
#[allow(dead_code)]
pub(crate) async fn list_process_resources_backend<S: ResourceStoreBackend>(
    store: &S,
    zone: &ZoneId,
) -> Result<Vec<StoredResource>, ProcessResourceRuntimeError> {
    let mut request = process_resource_list_request(zone);
    let mut resources = Vec::new();
    loop {
        let result = retry_transient_store_list(zone, &request.operation.operation_id, || {
            store.list(request.clone())
        })
        .await
            .map_err(|_| ProcessResourceRuntimeError::Store)?;
        resources.extend(result.resources);
        let Some(cursor) = result.next_cursor else {
            break;
        };
        request.cursor = Some(cursor);
    }
    Ok(resources)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_requests_use_both_generic_resource_types() {
        let zone = ZoneId::parse("test").expect("valid zone");
        let request = process_resource_list_request(&zone);
        assert_eq!(request.resource_types.len(), 2);
        assert_eq!(request.resource_types[0].as_str(), PROCESS_TYPE);
        assert_eq!(request.resource_types[1].as_str(), EPHEMERAL_PROCESS_TYPE);
    }

    /// The declared target is authoritative only for its own owner, and the
    /// Guest-owned guest-runtime rows (nested VMM, qemu media runner) target
    /// their owning Guest.
    #[test]
    fn declared_target_and_guest_runtime_rows_resolve_through_the_one_resolver() {
        let guest_ref = || ResourceRef::parse("Guest/work").expect("guest ref");
        let other_owner = ResourceRef::parse("Guest/other").expect("other owner");
        let binding_ref = ResourceRef::parse("VolumeBinding/data").expect("binding ref");
        let execution_ref =
            ResourceRef::parse("Host/host-system").expect("execution ref");
        let target_of = |owner: Option<&ResourceRef>, declared: Option<(&ResourceRef, &ResourceRef)>| {
            resolve_launch_identity(&LaunchRow {
                owner_ref: owner,
                owner_uid: None,
                execution_ref: &execution_ref,
                process_name: "worker",
                template: "reaction",
                declared_target: declared,
            })
            .expect("complete launch identity")
            .target_ref()
            .cloned()
        };

        let worker_ref = ResourceRef::parse("Process/worker").expect("process ref");
        assert_eq!(
            target_of(Some(&worker_ref), Some((&guest_ref(), &worker_ref))),
            Some(guest_ref()),
            "a target declared by the row's own owner applies"
        );
        assert_eq!(
            target_of(Some(&other_owner), Some((&guest_ref(), &worker_ref))),
            None,
            "a target declared by another owner never applies"
        );
        assert_eq!(target_of(None, None), None, "a host-owned row has no target");

        let vmm_owner = ResourceRef::parse("Guest/acceptance-guest").expect("guest owner");
        let vmm = resolve_launch_identity(&LaunchRow {
            owner_ref: Some(&vmm_owner),
            owner_uid: None,
            execution_ref: &execution_ref,
            process_name: "acceptance-guest-vmm",
            template: "cloud-hypervisor-runner",
            declared_target: None,
        })
        .expect("VMM identity");
        assert_eq!(vmm.target_ref(), Some(&vmm_owner));

        let qemu_owner = ResourceRef::parse("Guest/media-vm").expect("guest owner");
        let qemu = resolve_launch_identity(&LaunchRow {
            owner_ref: Some(&qemu_owner),
            owner_uid: None,
            execution_ref: &execution_ref,
            process_name: "media-vm-qemu",
            template: "qemu-media-runner",
            declared_target: None,
        })
        .expect("QEMU media identity");
        assert_eq!(qemu.target_ref(), Some(&qemu_owner));

        let binding_worker = resolve_launch_identity(&LaunchRow {
            owner_ref: Some(&binding_ref),
            owner_uid: None,
            execution_ref: &execution_ref,
            process_name: "vol-vfd-deadbeef",
            template: "virtiofsd-worker",
            declared_target: None,
        })
        .expect_err("a binding worker without its attachment target is incomplete");
        assert_eq!(binding_worker.code(), "launch-identity-missing-target-ref");
    }

    /// One row shape -> complete canonical identity or a named construction
    /// error. The shapes are the four owner kinds a Process row takes
    /// (host-owned worker, Guest-owned guest-runtime child, binding-owned
    /// serving worker, Provider-owned controller) plus one incomplete row.
    #[test]
    fn launch_identity_table_covers_owner_shapes() {
        let guest_ref = || ResourceRef::parse("Guest/acceptance-guest").expect("guest ref");
        let binding_ref = || ResourceRef::parse("VolumeBinding/data").expect("binding ref");

        struct Case {
            label: &'static str,
            owner_ref: Option<ResourceRef>,
            owner_uid: Option<&'static str>,
            execution_ref: &'static str,
            process_name: &'static str,
            template: &'static str,
            declared_target: Option<(ResourceRef, ResourceRef)>,
            expected: Expected,
        }

        enum Expected {
            Complete {
                target_ref: Option<ResourceRef>,
                vm: Option<&'static str>,
                launch_vm: &'static str,
                binding_worker: bool,
            },
            Error(&'static str),
        }

        let cases = [
            Case {
                label: "host-owned",
                owner_ref: None,
                owner_uid: None,
                execution_ref: "Host/host-system",
                process_name: "worker",
                template: "reaction",
                declared_target: None,
                expected: Expected::Complete {
                    target_ref: None,
                    vm: None,
                    launch_vm: "host-system",
                    binding_worker: false,
                },
            },
            Case {
                label: "guest-owned",
                owner_ref: Some(guest_ref()),
                owner_uid: Some("323e4567-e89b-42d3-a456-426614174001"),
                execution_ref: "Host/host-system",
                process_name: "acceptance-guest-vmm",
                template: "cloud-hypervisor-runner",
                declared_target: None,
                expected: Expected::Complete {
                    target_ref: Some(guest_ref()),
                    vm: Some("acceptance-guest"),
                    launch_vm: "acceptance-guest",
                    binding_worker: false,
                },
            },
            Case {
                label: "binding-owned",
                owner_ref: Some(binding_ref()),
                owner_uid: None,
                execution_ref: "Host/host-system",
                process_name: "vol-vfd-deadbeef",
                template: "virtiofsd-worker",
                declared_target: Some((guest_ref(), binding_ref())),
                expected: Expected::Complete {
                    target_ref: Some(guest_ref()),
                    vm: Some("acceptance-guest"),
                    launch_vm: "host-system",
                    binding_worker: true,
                },
            },
            Case {
                label: "controller-owned",
                owner_ref: Some(
                    ResourceRef::parse("Provider/network-local").expect("provider ref"),
                ),
                owner_uid: Some("123e4567-e89b-42d3-a456-426614174010"),
                execution_ref: "Host/host-system",
                process_name: "controller",
                template: "reaction",
                declared_target: None,
                expected: Expected::Complete {
                    target_ref: None,
                    vm: None,
                    launch_vm: "host-system",
                    binding_worker: false,
                },
            },
            Case {
                label: "incomplete: binding worker without its attachment target",
                owner_ref: Some(binding_ref()),
                owner_uid: None,
                execution_ref: "Host/host-system",
                process_name: "vol-vfd-deadbeef",
                template: "virtiofsd-worker",
                declared_target: None,
                expected: Expected::Error("launch-identity-missing-target-ref"),
            },
        ];

        for case in cases {
            let owner_ref = case.owner_ref.clone();
            let owner_uid = case
                .owner_uid
                .map(|uid| ResourceUid::parse(uid).expect("owner uid"));
            let execution_ref =
                ResourceRef::parse(case.execution_ref).expect("execution ref");
            let resolved = resolve_launch_identity(&LaunchRow {
                owner_ref: owner_ref.as_ref(),
                owner_uid: owner_uid.clone(),
                execution_ref: &execution_ref,
                process_name: case.process_name,
                template: case.template,
                declared_target: case
                    .declared_target
                    .as_ref()
                    .map(|(target, owner)| (target, owner)),
            });
            match (&case.expected, resolved) {
                (
                    Expected::Complete {
                        target_ref,
                        vm,
                        launch_vm,
                        binding_worker,
                    },
                    Ok(identity),
                ) => {
                    assert_eq!(
                        identity.target_ref(),
                        target_ref.as_ref(),
                        "{}: target ref",
                        case.label
                    );
                    assert_eq!(identity.vm(), *vm, "{}: vm", case.label);
                    assert_eq!(identity.launch_vm(), *launch_vm, "{}: launch vm", case.label);
                    assert_eq!(
                        identity.is_binding_worker(),
                        *binding_worker,
                        "{}: binding worker",
                        case.label
                    );
                    assert_eq!(
                        identity.owner_uid(),
                        owner_uid.as_ref(),
                        "{}: owner uid",
                        case.label
                    );
                }
                (Expected::Complete { .. }, Err(error)) => {
                    panic!("{}: expected a complete identity, got {error}", case.label)
                }
                (Expected::Error(code), Err(error)) => {
                    assert_eq!(error.code(), *code, "{}: named error", case.label)
                }
                (Expected::Error(code), Ok(identity)) => panic!(
                    "{}: expected construction error {code}, got identity with vm {:?}",
                    case.label,
                    identity.vm()
                ),
            }
        }
    }
}
