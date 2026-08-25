use d2b_contracts_broker::{
    BrokerCapabilities, PROTOCOL_VERSION, broker_wire::BrokerErrorResponse,
};

#[test]
fn capabilities_keep_protocol_version_and_wire_tags() {
    let capabilities = BrokerCapabilities::w3();
    assert_eq!(capabilities.protocol_version, PROTOCOL_VERSION);
    assert_eq!(
        capabilities.broker_operations,
        [
            "ApplyNftables",
            "ApplyNftablesProjection",
            "ApplyNmUnmanaged",
            "ApplyRoute",
            "ApplySysctl",
            "BindUnixSocket",
            "CreateBridge",
            "CreateOrReconcileUsersGroups",
            "CreatePersistentTap",
            "CreateTapFd",
            "DelegateCgroupV2",
            "DeleteBridge",
            "DeletePersistentTap",
            "ExportBrokerAudit",
            "Hello",
            "InjectSecretById",
            "LaunchMinijailChild",
            "MigrateLegacySwtpmState",
            "ModprobeIfAllowed",
            "OpenCgroupDir",
            "OpenDevice",
            "OpenFuse",
            "OpenKvm",
            "OpenVhostNet",
            "PauseBroker",
            "PrepareRuntimeDir",
            "PrepareStateDir",
            "PrepareStoreView",
            "ReadSecretById",
            "ResumeBroker",
            "RotateSecretById",
            "SecurityKeyApplyUdevRules",
            "SecurityKeyOpenDevice",
            "SetBridgePortFlags",
            "SetSocketAcl",
            "SetupMountNamespace",
            "UpdateHostsFile",
            "UsbipBind",
            "UsbipBindFirewallRule",
            "UsbipProxyReconcile",
            "UsbipUnbind",
            "ValidateBundle",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>()
    );

    let encoded = serde_json::to_value(&capabilities).expect("capabilities serialize");
    assert_eq!(encoded["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(
        encoded["brokerOperations"].as_array().map(Vec::len),
        Some(capabilities.broker_operations.len())
    );
    let decoded: BrokerCapabilities =
        serde_json::from_value(encoded).expect("capabilities deserialize");
    assert_eq!(decoded, capabilities);
}

#[test]
fn broker_error_debug_redacts_values() {
    let error = BrokerErrorResponse {
        kind: "wire-invalid-field".to_owned(),
        operation: "OpenDevice".to_owned(),
        target_wave: Some("secret-target".to_owned()),
        message: "secret-message".to_owned(),
        action: "secret-action".to_owned(),
    };
    let debug = format!("{error:?}");
    assert!(debug.contains("has_target_wave: true"));
    assert!(!debug.contains("secret-target"));
    assert!(!debug.contains("secret-message"));
    assert!(!debug.contains("secret-action"));
}
