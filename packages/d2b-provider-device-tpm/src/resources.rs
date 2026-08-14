//! Canonical child-resource builders for the TPM Provider.

use d2b_contracts::v3::{ResourceRef, ResourceUid};
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
    zone: &str,
    execution_ref: &ResourceRef,
) -> Result<Value, TpmResourceEffectError> {
    let spec = build_tpm_state_volume_spec(device_uid, execution_ref)?;
    let short = device_short(device_uid);
    Ok(json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": "Volume",
        "metadata": {
            "name": format!("device-{short}-tpm-state"),
            "zone": zone,
            "ownerRef": format!("Device/{device_uid}"),
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
    let short = device_short(device_uid);
    Ok(json!({
        "executionRef": execution_ref.to_canonical_string(),
        "domain": "system",
        "processClass": "worker",
        "template": "swtpm-socket",
        "desiredLifecycle": "running",
        "adoptionPolicy": "adopt-on-restart",
        "restartPolicy": { "class": "always" },
        "readiness": { "class": "provider-defined", "timeout": "30s" },
        "healthCheck": { "enabled": true, "class": "provider-defined" },
        "mounts": [{
            "volumeRef": format!("Volume/device-{short}-tpm-state"),
            "view": "swtpm-process",
            "access": "read-write",
            "required": true
        }],
        "sandbox": {
            "namespaceClasses": ["pid", "mount"],
            "capabilityClasses": [],
            "noNewPrivileges": true,
            "startRoot": false,
            "readOnlyRoot": true
        }
    }))
}

/// Build the mandatory pre-start flush EphemeralProcess spec.
pub fn build_swtpm_flush_spec(
    device_uid: &ResourceUid,
    execution_ref: &ResourceRef,
) -> Result<Value, TpmResourceEffectError> {
    let mut spec = build_swtpm_process_spec(device_uid, execution_ref)?;
    if let Some(object) = spec.as_object_mut() {
        object.insert(
            "processClass".to_owned(),
            Value::String("ephemeral-worker".to_owned()),
        );
        object.insert(
            "template".to_owned(),
            Value::String("swtpm-init-flush".to_owned()),
        );
        object.insert(
            "desiredLifecycle".to_owned(),
            Value::String("run-to-completion".to_owned()),
        );
        object.insert(
            "restartPolicy".to_owned(),
            Value::Object(serde_json::Map::new()),
        );
    }
    Ok(spec)
}
