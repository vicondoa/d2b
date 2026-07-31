mod common;

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::credential::OperationClass;
use d2b_credential_service::{
    CredentialMethod, CredentialResourceVerb, RolePermission, authorize_operation,
};
use d2b_provider_credential_entra::{EntraEndpointPolicy, LOGIN_ENDPOINT_PURPOSE};

#[test]
fn exact_role_subresources_and_endpoint_policy_are_closed() {
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
    assert_eq!(
        CredentialMethod::SignChallenge.operation_class(),
        OperationClass::SignChallenge
    );

    let provider = ResourceRef::parse("Provider/credential-entra").unwrap();
    let consumer = ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap();
    let policy = EntraEndpointPolicy::new("provider", provider.clone(), consumer.clone()).unwrap();
    assert_eq!(policy.purpose(), LOGIN_ENDPOINT_PURPOSE);
    assert!(policy.allows_subject(&provider));
    assert!(policy.allows_subject(&consumer));
    assert!(EntraEndpointPolicy::new("zone", provider, consumer).is_err());
}
