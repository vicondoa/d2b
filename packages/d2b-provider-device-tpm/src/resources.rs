//! Canonical child-resource builders for the TPM Provider.

use d2b_contracts::v3::execution_policy::{BoundedToken, DurationMs, ExecutionDomain};
use d2b_contracts::v3::{
    AdoptionPolicy, DesiredLifecycle, EphemeralProcessSpec, ExecutionSpec, HealthCheckClass,
    HealthCheckSpec, MountAccess, MountSpec, NamespaceClass, ProcessClass, ProcessSpec,
    ReadinessClass, ReadinessSpec, ResourceRef, ResourceUid, RestartClass, RestartPolicySpec,
    SandboxSpec, TelemetrySpec,
};
use serde_json::{Value, json};

use crate::resource_effect::TpmResourceEffectError;

fn device_short(device_uid: &ResourceUid) -> String {
    device_uid
        .as_str()
        .bytes()
        .filter(|byte| byte.is_ascii_hexdigit())
        .take(12)
        .map(char::from)
        .collect()
}

/// Build the controller-created TPM state Volume spec.
///
/// The returned document contains only opaque policy references. In
/// particular, it has no host path, socket, UID, GID, or binary field.
pub fn build_tpm_state_volume_spec(
    device_uid: &ResourceUid,
    execution_ref: &ResourceRef,
) -> Result<Value, TpmResourceEffectError> {
    if execution_ref.resource_type().as_str() != "Host" {
        return Err(TpmResourceEffectError::InvalidExecutionRef);
    }
    let short = device_short(device_uid);
    if short.len() != 12 {
        return Err(TpmResourceEffectError::InvalidDevice);
    }
    let owner = format!("User/device-{short}-swtpm-system");
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
            "ownerRef": owner,
            "groupRef": owner,
            "mode": "0700",
            "sensitivity": "secret-adjacent",
            "createPolicy": "create-if-never-provisioned",
            "repairPolicy": "fail-closed",
            "cleanupPolicy": "never",
            "adoptionPolicy": "quarantine-on-ambiguity",
            "restartPolicy": "preserve-across-controller-restart",
            "leaseClass": "none",
            "noFollow": true,
            "recursive": false,
            "foreignChildPolicy": "preserve",
            "accessAcl": [],
            "defaultAcl": [],
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
pub fn build_tpm_state_volume_resource(
    device_uid: &ResourceUid,
    device_ref: &ResourceRef,
    zone: &str,
    execution_ref: &ResourceRef,
) -> Result<Value, TpmResourceEffectError> {
    if device_ref.resource_type().as_str() != "Device" {
        return Err(TpmResourceEffectError::InvalidDevice);
    }
    let spec = build_tpm_state_volume_spec(device_uid, execution_ref)?;
    let short = device_short(device_uid);
    Ok(serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": "Volume",
        "metadata": {
            "name": format!("device-{short}-tpm-state"),
            "zone": zone,
            "ownerRef": device_ref.to_canonical_string(),
            "managedBy": "controller"
        },
        "spec": spec
    }))
}

/// Build the long-lived swtpm Process base spec.
pub fn build_swtpm_process_spec(
    device_uid: &ResourceUid,
    execution_ref: &ResourceRef,
) -> Result<Value, TpmResourceEffectError> {
    if execution_ref.resource_type().as_str() != "Host" {
        return Err(TpmResourceEffectError::InvalidExecutionRef);
    }
    let execution = swtpm_execution(
        execution_ref,
        ProcessClass::Worker,
        "swtpm-socket",
        swtpm_mount(device_uid)?,
    )?;
    serde_json::to_value(
        ProcessSpec::new(
            execution,
            DesiredLifecycle::Running,
            RestartPolicySpec::new(
                RestartClass::Always,
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
                true,
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
pub fn build_swtpm_flush_spec(
    device_uid: &ResourceUid,
    execution_ref: &ResourceRef,
) -> Result<Value, TpmResourceEffectError> {
    if execution_ref.resource_type().as_str() != "Host" {
        return Err(TpmResourceEffectError::InvalidExecutionRef);
    }
    let execution = swtpm_execution(
        execution_ref,
        ProcessClass::Worker,
        "swtpm-init-flush",
        swtpm_mount(device_uid)?,
    )?;
    serde_json::to_value(
        EphemeralProcessSpec::minimal(execution)
            .map_err(|_| TpmResourceEffectError::InvalidDevice)?,
    )
    .map_err(|_| TpmResourceEffectError::InvalidDevice)
}

fn swtpm_mount(device_uid: &ResourceUid) -> Result<MountSpec, TpmResourceEffectError> {
    let short = device_short(device_uid);
    MountSpec::new(
        ResourceRef::parse(&format!("Volume/device-{short}-tpm-state"))
            .map_err(|_| TpmResourceEffectError::InvalidDevice)?,
        BoundedToken::parse("swtpm-process").map_err(|_| TpmResourceEffectError::InvalidDevice)?,
        "/var/lib/swtpm",
        MountAccess::ReadWrite,
        true,
    )
    .map_err(|_| TpmResourceEffectError::InvalidDevice)
}

fn swtpm_execution(
    execution_ref: &ResourceRef,
    process_class: ProcessClass,
    template: &str,
    mount: MountSpec,
) -> Result<ExecutionSpec, TpmResourceEffectError> {
    ExecutionSpec::new(
        execution_ref.clone(),
        Some(ExecutionDomain::System),
        None,
        process_class,
        BoundedToken::parse(template).map_err(|_| TpmResourceEffectError::InvalidDevice)?,
        None,
        Vec::new(),
        vec![mount],
        SandboxSpec::new(
            vec![NamespaceClass::Pid, NamespaceClass::Mount],
            Vec::new(),
            BoundedToken::parse("strict").map_err(|_| TpmResourceEffectError::InvalidDevice)?,
            true,
            false,
            d2b_contracts::v3::EnvironmentClass::Minimal,
            true,
            Some("0022".to_owned()),
            0,
            None,
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
    use d2b_contracts::v3::{EphemeralProcessSpec, ProcessSpec};

    fn device_uid() -> ResourceUid {
        ResourceUid::parse("6f9619ff-8b86-4d01-b42d-00cf4fc964ff").unwrap()
    }

    #[test]
    fn generated_process_specs_round_trip_through_v3_contracts() {
        let device = device_uid();
        let host = ResourceRef::parse("Host/host-system").unwrap();

        let process = build_swtpm_process_spec(&device, &host).unwrap();
        let process: ProcessSpec = serde_json::from_value(process).unwrap();
        assert_eq!(process.execution().process_class(), ProcessClass::Worker);
        assert_eq!(process.execution().mounts().len(), 1);
        assert_eq!(
            process.execution().mounts()[0].mount_path(),
            "/var/lib/swtpm"
        );

        let flush = build_swtpm_flush_spec(&device, &host).unwrap();
        let flush: EphemeralProcessSpec = serde_json::from_value(flush).unwrap();
        assert_eq!(flush.execution().process_class(), ProcessClass::Worker);
        assert_eq!(flush.execution().mounts().len(), 1);
        assert_eq!(flush.execution().mounts()[0].mount_path(), "/var/lib/swtpm");
    }

    #[test]
    fn state_volume_owner_is_the_authenticated_device_reference() {
        let device = device_uid();
        let device_ref = ResourceRef::parse("Device/vm-tpm").unwrap();
        let host = ResourceRef::parse("Host/host-system").unwrap();
        let resource = build_tpm_state_volume_resource(&device, &device_ref, "dev", &host).unwrap();

        assert_eq!(
            resource["metadata"]["ownerRef"],
            serde_json::json!("Device/vm-tpm")
        );
        assert_ne!(
            resource["metadata"]["ownerRef"],
            serde_json::json!(format!("Device/{device}"))
        );
    }

    #[test]
    fn flush_builder_rejects_non_host_execution_refs() {
        let device = device_uid();
        let zone = ResourceRef::parse("Zone/dev").unwrap();

        assert!(matches!(
            build_swtpm_flush_spec(&device, &zone),
            Err(TpmResourceEffectError::InvalidExecutionRef)
        ));
    }
}
