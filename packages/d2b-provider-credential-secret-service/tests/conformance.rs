mod common;

use d2b_contracts::v3::credential::OperationClass;
use d2b_credential_service::{
    CredentialMethod, CredentialResourceVerb, RolePermission, authorize_operation,
};

use common::operation_class;

#[test]
fn exact_method_operation_and_role_matrix_is_closed() {
    let supported = [
        CredentialMethod::AcquireToken,
        CredentialMethod::RefreshToken,
        CredentialMethod::RevokeToken,
        CredentialMethod::InspectMetadata,
    ];
    for method in supported {
        let permission =
            RolePermission::new(CredentialResourceVerb::UseCredential, method.subresource());
        assert!(authorize_operation(method, &[operation_class(method)], &permission).is_ok());
        assert!(
            authorize_operation(
                method,
                &[operation_class(method)],
                &RolePermission::new(CredentialResourceVerb::UseCredential, "*"),
            )
            .is_err()
        );
        assert!(authorize_operation(method, &[], &permission).is_err());
    }
    assert!(!supported.contains(&CredentialMethod::SignChallenge));
    assert_eq!(
        CredentialMethod::SignChallenge.operation_class(),
        OperationClass::SignChallenge
    );
}
