//! The one canonical launch identity of a Process row.
//!
//! Every ticket consumer - the driver, the provider runtime, and the durable
//! row readers - resolves a row's launch identity here and nowhere else, so no
//! two of them can disagree about the owner, the execution target, the
//! cross-target Guest selector, the VM scope, or the legacy runner role.

use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
use d2b_process_conformance::{LaunchIdentity, LaunchIdentityError};
use d2b_provider_volume_virtiofs::WORKER_TEMPLATE;

/// The Template + Process-name pairs of the Guest-owned guest-runtime rows
/// (the nested VMM and the qemu media runner): the rule that binds such a
/// row's launch target to its owning Guest.
const GUEST_RUNTIME_PROCESS_TEMPLATES: &[(&str, &str)] = &[
    ("cloud-hypervisor-runner", "-vmm"),
    ("qemu-media-runner", "-qemu"),
];

/// The row inputs one launch identity resolves from.
///
/// Every field either is persisted on the durable row or is authored in its
/// metadata/spec; nothing here is guessed, and no caller derives a subset of
/// the identity on its own.
pub struct LaunchRow<'a> {
    /// The row's semantic owner (manager owner key, or the authored
    /// `metadata.ownerRef` for an owner the manager does not hold).
    pub owner_ref: Option<&'a ResourceRef>,
    /// The durable owner linkage the row persists.
    pub owner_uid: Option<ResourceUid>,
    /// The exact execution target declared by the row's Process spec.
    pub execution_ref: &'a ResourceRef,
    /// The row's Process name (its legacy runner role id).
    pub process_name: &'a str,
    /// The template the row's Process spec declares.
    pub template: &'a str,
    /// A target ref declared for this row together with the owner it belongs
    /// to (the owning `VolumeBinding`'s attachment, or the runtime's target
    /// selector). It is authoritative only for its own owner.
    pub declared_target: Option<(&'a ResourceRef, &'a ResourceRef)>,
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
/// The binding-owned serving-worker split
/// ([`LaunchIdentity::is_binding_worker`]) is derived here too, from the
/// row's declared template under its `VolumeBinding` owner
/// ([`WORKER_TEMPLATE`]) - the same declared fact the launch path resolves
/// the trusted serving intent through - never from the owner kind alone.
///
/// A row whose launch cannot be named completely fails here, once, naming the
/// missing input ([`LaunchIdentityError`]).
pub fn resolve_launch_identity(
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
    // The serving-worker split keys on the trusted declaration, never on the
    // owner kind alone: a binding-owned row counts only when it declares the
    // serving template (`d2b_provider_volume_virtiofs::WORKER_TEMPLATE`) -
    // the same declared fact the rest of the launch path reads
    // (`process_provider_runtime::resource_ticket` re-checks it as
    // `provider-ticket:template-not-found` and the driver's serving-worker derivation
    // checks it before deriving the worker's arguments, both against
    // `find_volume_binding_worker_intent`, which matches the trusted intent by
    // this template). A `VolumeBinding`-owned row with any other template
    // keeps an ordinary launch instead of the worker's host-exec/guest-target
    // split. `LaunchTicket`'s owner setter normalizes the carried flag back to
    // the owner kind; the two rules agree on every row the binding driver
    // mints, which declares exactly the serving template.
    let binding_worker =
        owner.is_some_and(|owner| owner.resource_type().as_str() == "VolumeBinding")
            && row.template == WORKER_TEMPLATE;
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
pub fn guest_runtime_process_matches(
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

#[cfg(test)]
mod tests {
    use super::*;

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
    /// serving worker, Provider-owned controller) plus a binding-owned row
    /// that declares no serving template, and one incomplete row.
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
            // The owner kind alone never makes a serving worker: a
            // binding-owned row that declares another template keeps the
            // ordinary VM scope (the attachment Guest it targets) and the
            // ordinary launch path, exactly as `serving_worker_launch` and
            // `find_volume_binding_worker_intent` key on the template.
            Case {
                label: "binding-owned row declaring another template",
                owner_ref: Some(binding_ref()),
                owner_uid: None,
                execution_ref: "Host/host-system",
                process_name: "vol-probe-deadbeef",
                template: "virtiofsd-attachment-probe",
                declared_target: Some((guest_ref(), binding_ref())),
                expected: Expected::Complete {
                    target_ref: Some(guest_ref()),
                    vm: Some("acceptance-guest"),
                    launch_vm: "acceptance-guest",
                    binding_worker: false,
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
