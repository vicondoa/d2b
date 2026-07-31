use d2b_credential_service::{
    CredentialMethod, CredentialResourceVerb, RolePermission, authorize_operation,
};

#[test]
fn exact_role_subresource_matrix_is_closed() {
    for method in [
        CredentialMethod::AcquireToken,
        CredentialMethod::RefreshToken,
        CredentialMethod::RevokeToken,
        CredentialMethod::InspectMetadata,
    ] {
        let permission =
            RolePermission::new(CredentialResourceVerb::UseCredential, method.subresource());
        assert!(authorize_operation(method, &[method.operation_class()], &permission).is_ok());
        assert!(
            authorize_operation(
                method,
                &[method.operation_class()],
                &RolePermission::new(CredentialResourceVerb::UseCredential, "*"),
            )
            .is_err()
        );
    }
}
