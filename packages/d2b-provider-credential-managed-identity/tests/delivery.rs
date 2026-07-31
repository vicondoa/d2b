mod common;

use d2b_credential_service::{
    CredentialAdmission, CredentialAuthorization, CredentialMethod, CredentialProvider,
    CredentialRequest, CredentialResponse, CredentialServer, CredentialServiceError,
    CredentialServiceErrorCode, CredentialTransport, DeliverySessionParams,
    SensitiveDeliveryRecord,
};
use d2b_provider_credential_managed_identity::ManagedIdentityCredentialProvider;

use common::{admitted, delivery, request, setup};

#[test]
fn provider_returns_the_adapter_supplied_binding_unchanged() {
    let expected = delivery(CredentialMethod::AcquireToken, 1);
    let (provider, _) = setup();
    let server = CredentialServer::new(provider, admitted());
    let response = server
        .call(CredentialMethod::AcquireToken, request("idem-binding"))
        .unwrap();
    let CredentialResponse::AcquireToken(response) = response else {
        panic!("acquire response");
    };
    assert_eq!(response.delivery_session_params, expected);
}

#[test]
fn delivery_zeroizes_and_replay_sequence_advances() {
    let mut record = SensitiveDeliveryRecord::new(b"managed-token".to_vec(), 64).unwrap();
    let mut destination = [0; 13];
    record.copy_to(&mut destination).unwrap();
    destination.fill(0);
    record.clear();
    assert!(record.is_zeroized());
    assert!(
        delivery(CredentialMethod::RefreshToken, 2).sequence()
            > delivery(CredentialMethod::AcquireToken, 1).sequence()
    );
}

#[derive(Clone)]
struct FixedAdmission {
    authorized: DeliverySessionParams,
}

impl CredentialAdmission for FixedAdmission {
    fn authorize(
        &self,
        method: CredentialMethod,
        _request: &CredentialRequest,
    ) -> Result<CredentialAuthorization, CredentialServiceError> {
        CredentialAuthorization::new(method, Some(self.authorized.clone()))
    }
}

struct BindingReplacingProvider {
    inner: ManagedIdentityCredentialProvider,
    replacement: DeliverySessionParams,
}

impl CredentialProvider for BindingReplacingProvider {
    fn dispatch(
        &self,
        method: CredentialMethod,
        request: &CredentialRequest,
        authorization: &CredentialAuthorization,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        let mut response = self.inner.dispatch(method, request, authorization)?;
        if let CredentialResponse::AcquireToken(delivery) = &mut response {
            delivery.delivery_session_params = self.replacement.clone();
        }
        Ok(response)
    }
}

#[test]
fn adapter_refuses_a_managed_identity_provider_binding_replacement() {
    let authorized = delivery(CredentialMethod::AcquireToken, 1);
    let replacement = delivery(CredentialMethod::AcquireToken, 2);
    let (provider, _) = setup();
    let server = CredentialServer::new(
        BindingReplacingProvider {
            inner: provider,
            replacement,
        },
        FixedAdmission { authorized },
    );
    assert_eq!(
        server
            .call(
                CredentialMethod::AcquireToken,
                request("idem-refuse-binding")
            )
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::InvariantFailure
    );
}
