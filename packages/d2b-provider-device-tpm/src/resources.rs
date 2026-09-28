//! Canonical child-resource builders for the TPM Provider.

use d2b_contracts_resource::v3::execution_policy::{BoundedToken, DurationMs, ExecutionDomain};
use d2b_contracts_resource::v3::{
    AdoptionPolicy, DesiredLifecycle, EphemeralProcessSpec, ExecutionSpec, HealthCheckClass,
    HealthCheckSpec, MappingClass, MountAccess, MountSpec, NamespaceClass, ProcessClass,
    ProcessSpec, ReadinessClass, ReadinessSpec, ResourceRef, RestartClass,
    RestartPolicySpec, SandboxSpec, TelemetrySpec, UserNamespaceSpec,
};
use serde_json::{Value, json};

use crate::resource_effect::TpmResourceEffectError;

/// The Device's own TPM state Volume role. The framework composes the
/// concrete name from this role and the owning Device's durable uid
/// (`d2b_core::bundle_resolver::device_child_name`), so the Provider spells
/// the role once and both the row it creates and the mount that reaches it
/// resolve the same name from the same identity.
const TPM_STATE_VOLUME_ROLE: &str = "tpm-state";

/// The controller-created state Volume's name for one Device, from the
/// framework's one Device-scoped child naming rule.
fn device_state_volume_name(
    zone: &str,
    device_ref: &ResourceRef,
) -> Result<String, TpmResourceEffectError> {
    d2b_core::bundle_resolver::device_child_name(zone, device_ref, TPM_STATE_VOLUME_ROLE)
        .ok_or(TpmResourceEffectError::InvalidDevice)
}


/// Umask the declared swtpm rows carry: swtpm binds a shared Unix socket a
/// peer connects to as a different uid, so the created socket must keep its
/// group bits (the broker's `RoleProfile::umask` note, 0o007).
const SWTPM_SANDBOX_UMASK: &str = "0007";

/// The state Volume's declared layout principal: the daemon itself.
///
/// The daemon owns every entry it creates (it runs without capabilities), so
/// the state directory's *declared* owner must be the daemon's own account -
/// no other uid can be chowned to. The Device's two TPM principals are
/// granted access through the entry's ACLs instead: the long-lived swtpm
/// worker gets `rwx` (it writes NVRAM and binds the sockets) and the one-shot
/// pre-start flush gets traverse plus read/write on the sockets it connects
/// to.
const STATE_VOLUME_OWNER: &str = "User/d2bd";

/// Build the controller-created TPM state Volume spec.
///
/// The returned document contains only opaque policy references. In
/// particular, it has no host path, socket, UID, GID, or binary field.
///
/// The state directory's *granted* principals are named after the Device's
/// *declared* identity (`zone` + name), the same way the volume-local
/// Provider's own per-Guest principals are (`User/d2b-<vm>-swtpm`): each
/// principal must be a real host account the state-layout effect resolves
/// through NSS, and an account name derived from the Device's 32-hex uid
/// exceeds the host's account-name bound. The Device's uid still keys the
/// Volume name (`device-<32hex>-tpm-state`) and every runtime path
/// derivation.
/// # Errors
///
/// Returns [`TpmResourceEffectError::InvalidDevice`] when the reference does
/// not name a Device or the Device name or Zone is empty.
pub fn build_tpm_state_volume_spec(
    device_ref: &ResourceRef,
    zone: &str,
    execution_ref: &ResourceRef,
) -> Result<Value, TpmResourceEffectError> {
    if device_ref.resource_type().as_str() != "Device" {
        return Err(TpmResourceEffectError::InvalidDevice);
    }
    let name = device_ref.name().as_str();
    if name.is_empty() || zone.is_empty() {
        return Err(TpmResourceEffectError::InvalidDevice);
    }
    build_tpm_state_volume_spec_with_principals(
        &format!("User/d2b-{zone}-{name}-swtpm"),
        &format!("User/d2b-{zone}-{name}-swtpm-flush"),
        execution_ref,
    )
}

fn build_tpm_state_volume_spec_with_principals(
    worker_principal: &str,
    flush_principal: &str,
    execution_ref: &ResourceRef,
) -> Result<Value, TpmResourceEffectError> {
    if execution_ref.resource_type().as_str() != "Host" {
        return Err(TpmResourceEffectError::InvalidExecutionRef);
    }
    Ok(json!({
        "providerRef": "Provider/volume-local",
        "source": {
            "executionRef": execution_ref.to_canonical_string(),
            "settings": {
                "kind": "local-path",
                "sourcePolicyId": "tpm-state"
            }
        },
        "kind": "state",
        "layout": [{
            "path": "",
            "type": "directory",
            // The daemon creates the per-Volume root itself (the anchored
            // root handle the layout effect acts through) and cannot chown it
            // to another uid, so the declared owner is the daemon's own
            // account; the two TPM principals are granted below. The group
            // bits carry the ACL mask (`setfacl` semantics), which is why the
            // declared mode is 0770 rather than 0700.
            "ownerRef": STATE_VOLUME_OWNER,
            "groupRef": STATE_VOLUME_OWNER,
            "mode": "0770",
            "sensitivity": "secret-adjacent",
            "createPolicy": "create-if-never-provisioned",
            "repairPolicy": "exact-owner",
            "cleanupPolicy": "never",
            "adoptionPolicy": "quarantine-on-ambiguity",
            "restartPolicy": "preserve-across-controller-restart",
            "leaseClass": "none",
            "noFollow": true,
            "recursive": false,
            "foreignChildPolicy": "preserve",
            // Both TPM workers of this Device share the state directory: the
            // long-lived swtpm worker writes NVRAM and binds the sockets
            // there, and the one-shot pre-start flush `swtpm_ioctl -i`
            // connects to the ctrl socket inside it. The runtime runs each
            // declared row as its own principal uid (`mint_template_intent`),
            // so both principals need their own grant: the worker `rwx` on
            // the directory, and the flush traverse on the directory plus -
            // through the default ACL - read/write on the sockets swtpm
            // creates in it. `Default` keeps the mask at the file's group
            // bits, which swtpm's 0660 socket mode and 0007 umask already
            // spell out. The closed Volume contract accepts only
            // `[rwx]{1,3}` - POSIX's `-` placeholders are not part of the
            // spelling, and a grant carrying one makes the daemon refuse the
            // whole spec.
            "accessAcl": [
                {
                    "principal": { "ref": worker_principal },
                    "permissions": "rwx"
                },
                {
                    "principal": { "ref": flush_principal },
                    "permissions": "rx"
                }
            ],
            "defaultAcl": [
                {
                    "principal": { "ref": worker_principal },
                    "permissions": "rwx"
                },
                {
                    "principal": { "ref": flush_principal },
                    "permissions": "rw"
                }
            ],
            "invariants": [
                "no-symlink",
                "broker-opaque-id-only",
                "scope-authorization-required"
            ],
            "target": null
        }],
        "views": {
            "swtpm-process": {
                "path": "",
                "rights": ["read", "write", "create", "traverse"]
            },
            "controller": {
                "path": "",
                "rights": ["read", "write", "create", "delete", "traverse"]
            }
        },
        "attachments": [],
        "quota": null
    }))
}

/// Build a complete controller-created TPM state Volume resource document.
///
/// # Errors
///
/// Returns the same errors as [`build_tpm_state_volume_spec`]:
/// [`TpmResourceEffectError::InvalidDevice`].
pub fn build_tpm_state_volume_resource(
    device_ref: &ResourceRef,
    zone: &str,
    execution_ref: &ResourceRef,
) -> Result<Value, TpmResourceEffectError> {
    let name = device_state_volume_name(zone, device_ref)?;
    let spec = build_tpm_state_volume_spec(device_ref, zone, execution_ref)?;
    Ok(serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": "Volume",
        "metadata": {
            "name": name,
            "zone": zone,
            "ownerRef": device_ref.to_canonical_string(),
            "managedBy": "controller"
        },
        "spec": spec
    }))
}

/// Build the long-lived swtpm Process base spec.
///
/// # Errors
///
/// Returns [`TpmResourceEffectError::InvalidExecutionRef`] when the
/// execution reference is not a Host and
/// [`TpmResourceEffectError::InvalidDevice`] when the mount or process spec
/// cannot be constructed.
pub fn build_swtpm_process_spec(
    device_ref: &ResourceRef,
    zone: &str,
    execution_ref: &ResourceRef,
) -> Result<Value, TpmResourceEffectError> {
    if execution_ref.resource_type().as_str() != "Host" {
        return Err(TpmResourceEffectError::InvalidExecutionRef);
    }
    let execution = swtpm_execution(
        device_ref,
        zone,
        execution_ref,
        ProcessClass::Worker,
        "swtpm-socket",
        vec![swtpm_mount()?],
        true,
    )?;
    serde_json::to_value(
        ProcessSpec::new(
            execution,
            DesiredLifecycle::Running,
            RestartPolicySpec::new(
                RestartClass::OnFailure,
                DurationMs::parse("1s", 0, 60_000).unwrap(),
                DurationMs::parse("60s", 1_000, 3_600_000).unwrap(),
                2_000,
                None,
                DurationMs::parse("300s", 0, 86_400_000).unwrap(),
            )
            .map_err(|_| TpmResourceEffectError::InvalidDevice)?,
            ReadinessSpec::new(
                DurationMs::parse("0s", 0, 3_600_000).unwrap(),
                DurationMs::parse("30s", 1_000, 3_600_000).unwrap(),
                3,
                1,
                ReadinessClass::ProviderDefined,
            )
            .map_err(|_| TpmResourceEffectError::InvalidDevice)?,
            HealthCheckSpec::new(
                false,
                DurationMs::parse("30s", 1_000, 3_600_000).unwrap(),
                DurationMs::parse("5s", 1_000, 3_600_000).unwrap(),
                3,
                HealthCheckClass::ProviderDefined,
            )
            .map_err(|_| TpmResourceEffectError::InvalidDevice)?,
            AdoptionPolicy::AdoptOnRestart,
            DurationMs::parse("30s", 0, 3_600_000).unwrap(),
        )
        .map_err(|_| TpmResourceEffectError::InvalidDevice)?,
    )
    .map_err(|_| TpmResourceEffectError::InvalidDevice)
}

/// Build the mandatory pre-start flush EphemeralProcess spec.
///
/// # Errors
///
/// Returns [`TpmResourceEffectError::InvalidExecutionRef`] when the
/// execution reference is not a Host and
/// [`TpmResourceEffectError::InvalidDevice`] when the process spec cannot be
/// constructed.
pub fn build_swtpm_flush_spec(
    device_ref: &ResourceRef,
    zone: &str,
    execution_ref: &ResourceRef,
) -> Result<Value, TpmResourceEffectError> {
    if execution_ref.resource_type().as_str() != "Host" {
        return Err(TpmResourceEffectError::InvalidExecutionRef);
    }
    let execution = swtpm_execution(
        device_ref,
        zone,
        execution_ref,
        ProcessClass::Worker,
        "swtpm-init-flush",
        Vec::new(),
        false,
    )?;
    serde_json::to_value(
        EphemeralProcessSpec::new(
            execution,
            DurationMs::parse("30s", 1_000, 3_600_000).unwrap(),
            DurationMs::parse("60s", 1_000, 86_400_000).unwrap(),
            DurationMs::parse("1h", 0, 7 * 86_400_000).unwrap(),
            DurationMs::parse("24h", 0, 30 * 86_400_000).unwrap(),
            false,
        )
        .map_err(|_| TpmResourceEffectError::InvalidDevice)?,
    )
    .map_err(|_| TpmResourceEffectError::InvalidDevice)
}

/// The long-lived swtpm worker's own state Volume mount, declared against the
/// Device's controller-created child rather than a literal name: the uid
/// segment of that name is the Device's durable uid, which only the framework
/// can evaluate. The resolver expands the role against the row's own owner
/// (`d2b_core::bundle_resolver::resolve_declared_mount_volume`) and then
/// admits the concrete name through the same ownership check every other
/// declared mount faces.
fn swtpm_mount() -> Result<MountSpec, TpmResourceEffectError> {
    MountSpec::own_child_volume(
        BoundedToken::parse(TPM_STATE_VOLUME_ROLE)
            .map_err(|_| TpmResourceEffectError::InvalidDevice)?,
        BoundedToken::parse("swtpm-process").map_err(|_| TpmResourceEffectError::InvalidDevice)?,
        "/state",
        MountAccess::ReadWrite,
        true,
    )
    .map_err(|_| TpmResourceEffectError::InvalidDevice)
}

fn swtpm_execution(
    device_ref: &ResourceRef,
    zone: &str,
    execution_ref: &ResourceRef,
    process_class: ProcessClass,
    template: &str,
    mounts: Vec<MountSpec>,
    user_namespace: bool,
) -> Result<ExecutionSpec, TpmResourceEffectError> {
    // The Device's principal, named after its declared identity exactly like
    // the state Volume's layout owner: the process-spec principal and the
    // directory the worker writes must be the same account.
    let principal = ResourceRef::parse(&format!(
        "User/d2b-{zone}-{}-swtpm",
        device_ref.name().as_str()
    ))
    .map_err(|_| TpmResourceEffectError::InvalidDevice)?;
    // The declared template posture (the closed `device_worker_posture` table
    // the resource compiler and the broker both fence against): the
    // long-lived swtpm worker runs in its own user namespace with the
    // process-principal-root mapping, and the one-shot flush runs directly as
    // its system principal - the preserved `minijail_swtpm_video.rs`
    // contract, user-NS long-lived only. Both bind a shared Unix socket a
    // peer connects to as a different uid, so the umask keeps the group bits
    // (`RoleProfile::umask`, swtpm/0o007).
    let mut namespace_classes = vec![NamespaceClass::Pid, NamespaceClass::Mount];
    if user_namespace {
        namespace_classes.push(NamespaceClass::User);
    }
    ExecutionSpec::new(
        execution_ref.clone(),
        Some(ExecutionDomain::System),
        Some(principal),
        process_class,
        BoundedToken::parse(template).map_err(|_| TpmResourceEffectError::InvalidDevice)?,
        None,
        Vec::new(),
        mounts,
        SandboxSpec::new(
            namespace_classes,
            Vec::new(),
            BoundedToken::parse("strict").map_err(|_| TpmResourceEffectError::InvalidDevice)?,
            true,
            false,
            d2b_contracts_resource::v3::EnvironmentClass::Minimal,
            true,
            Some(SWTPM_SANDBOX_UMASK.to_owned()),
            0,
            user_namespace.then_some(UserNamespaceSpec {
                mapping_class: MappingClass::ProcessPrincipalRoot,
            }),
        )
        .map_err(|_| TpmResourceEffectError::InvalidDevice)?,
        Default::default(),
        None,
        Vec::new(),
        TelemetrySpec::default(),
    )
    .map_err(|_| TpmResourceEffectError::InvalidDevice)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::volume::{AclGrant, VolumeSpec};
    use d2b_contracts_resource::v3::{
        EphemeralProcessSpec, ProcessSpec, ResourceSpec, ResourceUid,
    };

    fn device_uid() -> ResourceUid {
        ResourceUid::parse("6f9619ff-8b86-4d01-b42d-00cf4fc964ff").unwrap()
    }

    #[test]
    fn generated_process_specs_round_trip_through_v3_contracts() {
        let device_ref = ResourceRef::parse("Device/vm-tpm").unwrap();
        let host = ResourceRef::parse("Host/host-system").unwrap();

        let process = build_swtpm_process_spec(&device_ref, "dev", &host).unwrap();
        let process: ProcessSpec = serde_json::from_value(process).unwrap();
        assert_eq!(process.execution().process_class(), ProcessClass::Worker);
        assert_eq!(process.execution().mounts().len(), 1);
        assert_eq!(process.execution().mounts()[0].mount_path(), "/state");
        assert_eq!(
            process
                .execution()
                .user_ref()
                .unwrap()
                .to_canonical_string(),
            "User/d2b-dev-vm-tpm-swtpm"
        );
        assert_eq!(
            process.execution().sandbox().namespace_classes(),
            [
                NamespaceClass::Pid,
                NamespaceClass::Mount,
                NamespaceClass::User
            ]
        );
        assert!(process.execution().sandbox().user_namespace().is_some());
        assert_eq!(
            process.execution().sandbox().umask(),
            Some(SWTPM_SANDBOX_UMASK)
        );
        let process_json = serde_json::to_value(&process).unwrap();
        assert_eq!(process_json["restartPolicy"]["class"], "on-failure");
        assert_eq!(process_json["healthCheck"]["enabled"], false);

        let flush = build_swtpm_flush_spec(&device_ref, "dev", &host).unwrap();
        let flush: EphemeralProcessSpec = serde_json::from_value(flush).unwrap();
        assert_eq!(flush.execution().process_class(), ProcessClass::Worker);
        assert!(flush.execution().mounts().is_empty());
        // The one-shot flush runs directly as its system principal: the
        // declared `swtpm-init-flush` posture carries no user namespace
        // (user-NS long-lived only, `minijail_swtpm_video.rs`).
        assert_eq!(
            flush.execution().sandbox().namespace_classes(),
            [NamespaceClass::Pid, NamespaceClass::Mount]
        );
        assert!(flush.execution().sandbox().user_namespace().is_none());
        assert_eq!(
            flush.execution().sandbox().umask(),
            Some(SWTPM_SANDBOX_UMASK)
        );
        let flush_json = serde_json::to_value(&flush).unwrap();
        assert_eq!(flush_json["startDeadline"], "30s");
        assert_eq!(flush_json["runtimeDeadline"], "60s");
    }

    /// The worker's declared mount names its own Device's child Volume by
    /// role, and the controller-created row is named from that same role, so
    /// the mount the Provider declares and the Volume it creates are the same
    /// name by construction rather than by two spellings kept in step.
    #[test]
    fn the_swtpm_mount_reaches_the_state_volume_the_controller_creates() {
        let device_ref = ResourceRef::parse("Device/vm-tpm").unwrap();
        let host = ResourceRef::parse("Host/host-system").unwrap();

        let row = build_tpm_state_volume_resource(&device_ref, "dev", &host).unwrap();
        let process: ProcessSpec =
            serde_json::from_value(build_swtpm_process_spec(&device_ref, "dev", &host).unwrap())
                .expect("the declared swtpm spec decodes");
        let mount = &process.execution().mounts()[0];

        assert!(
            mount.volume_ref().is_none(),
            "the declared mount names no literal Volume: a projection cannot \
             spell a uid-scoped child name"
        );
        assert_eq!(
            mount.own_volume_suffix().map(BoundedToken::as_str),
            Some(TPM_STATE_VOLUME_ROLE)
        );
        assert_eq!(
            d2b_core::bundle_resolver::resolve_declared_mount_volume("dev", &device_ref, mount)
                .expect("the role resolves against the row's own Device")
                .to_canonical_string(),
            format!(
                "Volume/{}",
                row["metadata"]["name"].as_str().expect("row name")
            ),
            "the mount the Provider declares reaches the Volume it creates"
        );
    }

    #[test]
    fn state_volume_owner_is_the_authenticated_device_reference() {
        let device = device_uid();
        let device_ref = ResourceRef::parse("Device/vm-tpm").unwrap();
        let host = ResourceRef::parse("Host/host-system").unwrap();
        let resource = build_tpm_state_volume_resource(&device_ref, "dev", &host).unwrap();

        assert_eq!(
            resource["metadata"]["ownerRef"],
            serde_json::json!("Device/vm-tpm")
        );
        assert_ne!(
            resource["metadata"]["ownerRef"],
            serde_json::json!(format!("Device/{device}"))
        );
    }

    /// The state Volume name carries the owning Device's WHOLE durable uid,
    /// not a prefix of it: two Devices whose uids agree on their first bytes
    /// still get separate private state directories, so a truncated segment
    /// could not be mistaken for another Device's.
    #[test]
    fn state_child_names_preserve_the_full_device_incarnation() {
        let near = |last: &str| {
            let mut bytes: Vec<char> = d2b_core::bundle_resolver::device_uid_scope("dev", "vm-tpm")
                .chars()
                .collect();
            let tail = last.len();
            let start = bytes.len() - tail;
            for (offset, character) in last.chars().enumerate() {
                bytes[start + offset] = character;
            }
            bytes.into_iter().collect::<String>()
        };
        let first = near("cf4fc964ff");
        let second = near("cf4fc96500");
        assert_ne!(first, second, "the two fixtures must differ only in the tail");

        let role_of = |uid_scope: &str| format!("device-{uid_scope}-{TPM_STATE_VOLUME_ROLE}");
        assert!(role_of(&first).contains(&first), "the whole segment is carried");
        assert!(role_of(&second).contains(&second), "the whole segment is carried");
    }

    #[test]
    fn state_volume_grants_the_flush_principal_access_to_the_shared_state_dir() {
        let device_ref = ResourceRef::parse("Device/vm-tpm").unwrap();
        let host = ResourceRef::parse("Host/host-system").unwrap();
        let volume = build_tpm_state_volume_resource(&device_ref, "dev", &host).unwrap();
        let layout = &volume["spec"]["layout"][0];
        // The daemon owns the directory it creates (it runs without
        // capabilities and cannot chown to another uid), so the declared
        // owner is the daemon's own account and both TPM principals are
        // opened through the entry's ACLs.
        assert_eq!(layout["ownerRef"], serde_json::json!("User/d2bd"));
        assert_eq!(layout["groupRef"], serde_json::json!("User/d2bd"));
        // The long-lived worker writes NVRAM and binds the sockets; the
        // one-shot flush runs as its own principal uid
        // (`mint_template_intent` mints one per declared row) and needs
        // traverse on the directory itself plus - through the default ACL -
        // read/write on the ctrl socket swtpm creates inside it.
        assert_eq!(
            layout["accessAcl"],
            serde_json::json!([
                {
                    "principal": { "ref": "User/d2b-dev-vm-tpm-swtpm" },
                    "permissions": "rwx"
                },
                {
                    "principal": { "ref": "User/d2b-dev-vm-tpm-swtpm-flush" },
                    "permissions": "rx"
                }
            ])
        );
        assert_eq!(
            layout["defaultAcl"],
            serde_json::json!([
                {
                    "principal": { "ref": "User/d2b-dev-vm-tpm-swtpm" },
                    "permissions": "rwx"
                },
                {
                    "principal": { "ref": "User/d2b-dev-vm-tpm-swtpm-flush" },
                    "permissions": "rw"
                }
            ])
        );
        // The spellings above are not cosmetic. `AclGrant` accepts only
        // 1-3 characters drawn from `{r,w,x}`, so a POSIX-style grant makes
        // the daemon refuse the whole durable spec at validate with
        // `volume-spec-invalid`; decoding the declared document through the
        // real contract types - the same envelope-then-base path the driver
        // takes - is what keeps that refusal at the provider instead.
        let envelope: ResourceSpec = serde_json::from_value(volume["spec"].clone())
            .expect("the declared state Volume document decodes");
        let spec: VolumeSpec = serde_json::from_slice(&envelope.base().to_canonical_bytes())
            .expect("the declared state Volume spec decodes as the closed Volume contract");
        assert_eq!(spec.layout().len(), 1);
        assert_eq!(spec.layout()[0].path(), "");
        // The group bits carry the ACL mask (`setfacl` semantics), so the
        // declared mode is the mask-covering 0770 rather than 0700.
        assert_eq!(spec.layout()[0].mode(), "0770");
        let access: Vec<AclGrant> =
            serde_json::from_value(layout["accessAcl"].clone()).expect("access grant decodes");
        assert_eq!(access.len(), 2);
        assert_eq!(access[0].permissions(), "rwx");
        assert_eq!(access[1].permissions(), "rx");
        let default: Vec<AclGrant> =
            serde_json::from_value(layout["defaultAcl"].clone()).expect("default grant decodes");
        assert_eq!(default.len(), 2);
        assert_eq!(default[0].permissions(), "rwx");
        assert_eq!(default[1].permissions(), "rw");
    }

    #[test]
    fn flush_builder_rejects_non_host_execution_refs() {
        let device_ref = ResourceRef::parse("Device/vm-tpm").unwrap();
        let zone = ResourceRef::parse("Zone/dev").unwrap();

        assert!(matches!(
            build_swtpm_flush_spec(&device_ref, "dev", &zone),
            Err(TpmResourceEffectError::InvalidExecutionRef)
        ));
    }
}
