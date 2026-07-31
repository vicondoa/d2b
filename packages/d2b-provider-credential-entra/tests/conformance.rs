mod common;
#[allow(dead_code)]
#[path = "../../d2b-provider-toolkit/src/conformance.rs"]
mod provider_conformance;

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::credential::{AudienceToken, CredentialSpec, OperationClass};
use d2b_contracts::v3::execution_policy::to_base_object;
use d2b_contracts::v3::{
    BaseSchemaBinding, BaseSchemaIdentity, ObjectFieldSchema, ResourceSchemaContract, ResourceSpec,
    ResourceTypeName, SchemaFingerprint, SchemaVersion,
};
use d2b_credential_service::{
    CredentialMethod, CredentialResourceVerb, RolePermission, authorize_operation,
};
use d2b_provider_credential_entra::{EntraEndpointPolicy, LOGIN_ENDPOINT_PURPOSE};
use provider_conformance::{
    CapabilityMatrix, ConformanceError, ProviderResourceTypeBinding, check_descriptor_conformance,
    check_provider_conformance,
};

fn conformance_fixture() -> (
    ProviderResourceTypeBinding,
    Vec<ResourceSchemaContract>,
    ResourceSpec,
) {
    let resource_type = ResourceTypeName::parse("Credential").unwrap();
    let base_binding = BaseSchemaBinding {
        spec: schema_identity('1'),
        status: schema_identity('2'),
    };
    let fields = [
        "scope",
        "audience",
        "consumerRef",
        "allowedOperations",
        "rotation",
        "expiry",
        "revocation",
        "identityGuestRef",
        "loginEndpointRef",
    ]
    .map(str::to_owned);
    let contract = ResourceSchemaContract::new(
        resource_type.clone(),
        base_binding.clone(),
        ObjectFieldSchema::new(fields.clone(), fields).unwrap(),
        ObjectFieldSchema::empty(),
        [],
    )
    .unwrap();
    let binding = ProviderResourceTypeBinding::new(
        resource_type,
        base_binding,
        CapabilityMatrix::new([], []).unwrap(),
    );
    let minimal = CredentialSpec::minimal(AudienceToken::parse("azure-resource-manager").unwrap());
    let spec = ResourceSpec::new(None, None, to_base_object(&minimal).unwrap(), None).unwrap();
    (binding, vec![contract], spec)
}

fn schema_identity(fill: char) -> BaseSchemaIdentity {
    BaseSchemaIdentity {
        version: SchemaVersion::new(1, 0).unwrap(),
        fingerprint: SchemaFingerprint::parse(format!("sha256:{}", fill.to_string().repeat(64)))
            .unwrap(),
    }
}

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

#[test]
fn provider_toolkit_conformance_arms_pass_and_refuse_perturbations() {
    let (binding, installed, minimal) = conformance_fixture();
    assert_eq!(
        check_descriptor_conformance(std::slice::from_ref(&binding), &installed),
        Ok(())
    );
    assert_eq!(
        check_provider_conformance(&binding, &installed, &minimal),
        Ok(())
    );
    assert_eq!(
        check_descriptor_conformance(&[], &installed),
        Err(ConformanceError::NoResourceTypeBinding)
    );
    assert_eq!(
        check_descriptor_conformance(&[binding.clone(), binding], &installed),
        Err(ConformanceError::DuplicateResourceTypeBinding)
    );
}
