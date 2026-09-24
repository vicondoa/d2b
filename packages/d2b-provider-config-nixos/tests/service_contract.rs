use std::sync::Arc;

use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_config_nixos::{
    ConfigError, ConfigOperation, ConfigService, ConfigServiceBackend, ConfigSyncRequest,
    GuestConfigDocument, create_ttrpc_services,
};

struct TestBackend;

impl ConfigServiceBackend for TestBackend {
    fn dispatch(
        &self,
        _operation: ConfigOperation,
        _payload: serde_json::Value,
    ) -> Result<serde_json::Value, ConfigError> {
        Ok(serde_json::json!({}))
    }
}

#[test]
fn request_and_document_bounds_are_enforced() {
    let guest = ResourceRef::parse("Guest/work").expect("guest ref");
    let request = ConfigSyncRequest::new(guest).expect("request");
    let service = ConfigService;
    assert!(
        service
            .validate_operation(
                ConfigOperation::ReadGuestConfig,
                &serde_json::to_value(&request).expect("request JSON")
            )
            .is_ok()
    );
    assert!(
        GuestConfigDocument::new(vec![b'x'; d2b_provider_config_nixos::MAX_CONFIG_BYTES + 1])
            .is_err()
    );
}

#[test]
fn operation_validation_enforces_closed_identifiers_and_semantic_bounds() {
    let service = ConfigService;
    let wrong_guest = serde_json::json!({
        "guestRef": "Host/work",
        "identifier": "guest-config"
    });
    assert_eq!(
        service
            .validate_operation(ConfigOperation::ReadGuestConfig, &wrong_guest)
            .expect_err("non-Guest references must fail")
            .code(),
        "config-request-invalid"
    );

    let wrong_identifier = serde_json::json!({
        "guestRef": "Guest/work",
        "identifier": "other-config"
    });
    assert_eq!(
        service
            .validate_operation(ConfigOperation::ReadGuestConfig, &wrong_identifier)
            .expect_err("open identifiers must fail")
            .code(),
        "config-request-invalid"
    );

    let invalid_view = serde_json::json!({
        "guestRef": "Guest/work",
        "identifier": "guest-config",
        "against": "not-a-digest"
    });
    assert_eq!(
        service
            .validate_operation(ConfigOperation::Diff, &invalid_view)
            .expect_err("diff views must be commitments")
            .code(),
        "config-view-invalid"
    );
}

#[test]
fn config_service_publishes_only_the_closed_typed_methods() {
    let services = create_ttrpc_services(Arc::new(TestBackend));
    let service = services
        .get("d2b.config-nixos.v3.ConfigNixosService")
        .expect("config service");
    assert_eq!(service.methods.len(), ConfigOperation::ALL.len());
    for operation in ConfigOperation::ALL {
        let method = operation
            .as_str()
            .strip_prefix("ConfigNixosService/")
            .expect("method prefix");
        assert!(service.methods.contains_key(method));
    }
    assert!(
        services
            .keys()
            .all(|name| name == "d2b.config-nixos.v3.ConfigNixosService")
    );
}
