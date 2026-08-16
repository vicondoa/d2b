use d2b_provider_runtime_qemu_media::{ProviderConfig, WorkerConfigProjection};

#[test]
fn provider_config_requires_host_and_projects_controller_only() {
    let config = ProviderConfig::default();
    assert!(config.validate().is_err());

    let config = ProviderConfig::new(
        "Host/host-system",
        "qemu-system-x86-64",
        "Provider/network-local",
        "Provider/volume-local",
        None,
    )
    .unwrap();
    assert!(config.validate().is_ok());
    assert_eq!(config.project_worker(), WorkerConfigProjection);
    let projection = config.project_controller();
    assert_eq!(projection.controller_execution_ref(), "Host/host-system");
    assert_eq!(projection.qemu_binary_artifact_id(), "qemu-system-x86-64");
    assert_eq!(projection.network_provider_ref(), "Provider/network-local");
    assert_eq!(projection.volume_provider_ref(), "Provider/volume-local");
    assert_eq!(projection.display_provider_ref(), None);
}

#[test]
fn provider_config_rejects_invalid_bounds() {
    let mut config = ProviderConfig::new(
        "Host/host-system",
        "qemu-system-x86-64",
        "Provider/network-local",
        "Provider/volume-local",
        None,
    )
    .unwrap();
    config.qmp_ready_timeout_seconds = 4;
    assert!(config.validate().is_err());
    config.qmp_ready_timeout_seconds = 30;
    config.runtime_tmpfs_quota_bytes = 1024;
    assert!(config.validate().is_err());
}
