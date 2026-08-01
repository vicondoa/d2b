use d2b_contracts::v3::credential::OperationClass;
use d2b_credential_service::{
    CredentialAdminAction, CredentialMethod, CredentialResourceVerb, RolePermission,
    authorize_admin, authorize_operation,
};

#[test]
fn exact_use_credential_role_matrix_is_closed() {
    let cases = [
        (CredentialMethod::AcquireToken, OperationClass::AcquireToken),
        (CredentialMethod::RefreshToken, OperationClass::RefreshToken),
        (CredentialMethod::RevokeToken, OperationClass::RevokeToken),
        (
            CredentialMethod::SignChallenge,
            OperationClass::SignChallenge,
        ),
        (
            CredentialMethod::InspectMetadata,
            OperationClass::InspectMetadata,
        ),
    ];
    for (method, operation) in cases {
        let permission =
            RolePermission::new(CredentialResourceVerb::UseCredential, method.subresource());
        assert!(authorize_operation(method, &[operation], &permission).is_ok());
        for rejected in [
            "",
            "*",
            "unknown",
            "AcquireToken",
            "acquire_token",
            CredentialMethod::InspectMetadata.subresource(),
        ] {
            if rejected != method.subresource() {
                assert!(
                    authorize_operation(
                        method,
                        &[operation],
                        &RolePermission::new(CredentialResourceVerb::UseCredential, rejected),
                    )
                    .is_err()
                );
            }
        }
        assert!(
            authorize_operation(
                method,
                &[operation],
                &RolePermission::new(CredentialResourceVerb::Get, method.subresource()),
            )
            .is_err()
        );
        assert!(authorize_operation(method, &[], &permission).is_err());
    }
}

#[test]
fn administrative_permission_is_supplemental_to_ordinary_crud() {
    for action in [
        CredentialAdminAction::Create,
        CredentialAdminAction::UpdateSpec,
        CredentialAdminAction::Delete,
    ] {
        let ordinary = RolePermission::new(action.ordinary_verb(), "");
        let admin = RolePermission::new(
            CredentialResourceVerb::AdminCredential,
            action.subresource(),
        );
        assert!(authorize_admin(action, &ordinary, &admin).is_ok());
        assert!(
            authorize_admin(
                action,
                &RolePermission::new(CredentialResourceVerb::Get, ""),
                &admin,
            )
            .is_err()
        );
        assert!(
            authorize_admin(
                action,
                &ordinary,
                &RolePermission::new(CredentialResourceVerb::AdminCredential, "*"),
            )
            .is_err()
        );
        assert!(
            authorize_admin(
                action,
                &ordinary,
                &RolePermission::new(action.ordinary_verb(), action.subresource()),
            )
            .is_err()
        );
    }
}
