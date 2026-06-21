use nixling_core::{
    processes::ProcessRole,
    runtime::{
        RuntimeAutostartPolicy, RuntimeMetadata, RuntimeOperationCapabilities,
        RuntimeProviderDriver, RuntimeProviderType, RuntimeServiceRole, RuntimeServiceSummary,
    },
};
use serde_json::json;

#[test]
fn local_nixos_advertises_positive_operation_axes() {
    let runtime = RuntimeMetadata::local_nixos();

    assert!(runtime.capabilities.lifecycle);
    assert!(runtime.capabilities.guest_control);
    assert!(runtime.operation_capabilities.lifecycle.start);
    assert!(runtime.operation_capabilities.lifecycle.switch);
    assert!(runtime.operation_capabilities.display.graphics);
    assert!(runtime.operation_capabilities.guest.exec);
    assert!(runtime.operation_capabilities.storage.store_sync);
    assert_eq!(
        runtime.autostart_policy,
        RuntimeAutostartPolicy::HostBootEligible
    );
    assert!(matches!(
        runtime.provider.driver,
        RuntimeProviderDriver::CloudHypervisor
    ));
    assert!(matches!(
        runtime.provider.provider_type,
        RuntimeProviderType::Local
    ));
    assert!(
        runtime
            .services
            .iter()
            .any(|service| service.id == "cloud-hypervisor"
                && service.role == RuntimeServiceRole::Hypervisor)
    );
    assert!(runtime.services.iter().any(|service| service.optional));

    let value = serde_json::to_value(runtime).expect("serializes");
    assert_eq!(
        value.pointer("/operationCapabilities/guest/guestControl"),
        Some(&json!(true))
    );
    assert!(value.pointer("/services/3/processRole").is_none());
    assert_eq!(
        value.pointer("/autostartPolicy"),
        Some(&json!("host-boot-eligible"))
    );
}

#[test]
fn local_qemu_media_uses_same_axes_without_guest_control() {
    let runtime = RuntimeMetadata::local_qemu_media();

    assert!(runtime.capabilities.lifecycle);
    assert!(!runtime.capabilities.guest_control);
    assert!(runtime.operation_capabilities.lifecycle.start);
    assert!(!runtime.operation_capabilities.lifecycle.switch);
    assert!(runtime.operation_capabilities.media.qemu_media);
    assert!(runtime.operation_capabilities.media.removable_media);
    assert!(!runtime.operation_capabilities.guest.exec);
    assert_eq!(runtime.autostart_policy, RuntimeAutostartPolicy::ManualOnly);
    assert!(matches!(
        runtime.provider.driver,
        RuntimeProviderDriver::Qemu
    ));
    assert!(runtime.services.iter().any(
        |service| service.id == "qemu-media" && service.role == RuntimeServiceRole::Hypervisor
    ));
}

#[test]
fn legacy_runtime_metadata_defaults_new_fields() {
    let runtime: RuntimeMetadata = serde_json::from_value(json!({
        "capabilities": {
            "configSync": true,
            "display": true,
            "exec": true,
            "guestControl": true,
            "inGuestObservability": true,
            "keys": true,
            "lifecycle": true,
            "ssh": true,
            "storeSync": true,
            "usbHotplug": true
        },
        "kind": "nixos",
        "provider": {
            "driver": "cloud-hypervisor",
            "id": "local-cloud-hypervisor",
            "type": "local"
        }
    }))
    .expect("legacy shape deserializes");

    assert!(runtime.operation_capabilities.is_empty());
    assert_eq!(runtime.autostart_policy, RuntimeAutostartPolicy::Unknown);
    assert!(runtime.services.is_empty());

    let value = serde_json::to_value(runtime).expect("serializes");
    assert!(value.get("operationCapabilities").is_none());
    assert!(value.get("autostartPolicy").is_none());
    assert!(value.get("services").is_none());
}

#[test]
fn service_summary_derives_public_role_from_process_role() {
    let qemu =
        RuntimeServiceSummary::from_process_role("qemu-media", ProcessRole::QemuMediaRunner, false);
    let gpu = RuntimeServiceSummary::from_process_role("gpu", ProcessRole::GpuRenderNode, true);

    assert_eq!(qemu.role, RuntimeServiceRole::Hypervisor);
    assert!(!qemu.optional);
    assert_eq!(gpu.role, RuntimeServiceRole::Display);
    assert!(gpu.optional);

    let value = serde_json::to_value(qemu).expect("serializes");
    assert!(value.get("processRole").is_none());
}

#[test]
fn operation_capabilities_empty_default_is_serially_omittable() {
    assert!(RuntimeOperationCapabilities::default().is_empty());
    assert!(!RuntimeOperationCapabilities::local_nixos().is_empty());
    assert!(!RuntimeOperationCapabilities::local_qemu_media().is_empty());
}
