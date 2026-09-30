mod common;

use std::sync::atomic::Ordering;

use d2b_contracts_provider::v3::credential::{
    CREDENTIAL_DELIVERY_NOISE_PROFILE, CredentialAuthorization, CredentialMethod, CredentialProvider,
    CredentialRequest, CredentialResponse, CredentialServiceError, CredentialServiceErrorCode,
    DeliverySessionParams, OperationClass, SensitiveDeliveryRecord,
};
use d2b_contracts_resource::v3::{
    BindingSlot, BoundedToken, CredentialBindingRequest, CredentialLifetime, ResourceRef,
};
use d2b_provider_credential_entra::{
    EntraClientError, EntraClientState, EntraController, EntraCredentialProvider,
};

use common::{
    AdmittedRelationship, ProviderHarness, SessionAdmission, TestAdmission, admitted, delivery,
    request, session_binding, setup, subject_context,
};

/// The audience the `Credential` row is admitted for.
const AUDIENCE: &str = "azure-resource-manager";

fn relationship(
    request: &CredentialRequest,
    granted: &[OperationClass],
    sequence: u64,
) -> AdmittedRelationship {
    AdmittedRelationship::new(request, granted, AUDIENCE, sequence)
}

async fn call_admitted(
    provider: &EntraCredentialProvider,
    relationship: &AdmittedRelationship,
    delivery: DeliverySessionParams,
    provider_generation: u64,
    method: CredentialMethod,
    request: &CredentialRequest,
) -> Result<CredentialResponse, CredentialServiceError> {
    let admission = SessionAdmission {
        delivery,
        provider_generation,
    };
    let authorization = admission.authorize(method, request)?;
    provider
        .dispatch_admitted(method, request, &authorization, relationship)
        .await
}

#[test]
fn provider_returns_exactly_the_read_only_adapter_binding() {
    let expected = delivery(CredentialMethod::AcquireToken, 1);
    let (provider, _) = setup();
    let server = ProviderHarness::new(provider, admitted());
    let response = server
        .call(CredentialMethod::AcquireToken, request("idem-binding"))
        .unwrap();
    let CredentialResponse::AcquireToken(response) = response else {
        panic!("acquire response");
    };
    assert_eq!(response.delivery_session_params, expected);
}

#[test]
fn refresh_response_preserves_the_authorization_owned_binding() {
    let (provider, _) = setup();
    let server = ProviderHarness::new(provider, admitted());
    server
        .call(CredentialMethod::AcquireToken, request("idem-acquire"))
        .unwrap();
    let refresh_request = request("idem-refresh");
    let response = server
        .call(CredentialMethod::RefreshToken, refresh_request.clone())
        .unwrap();
    let CredentialResponse::RefreshToken(response) = response else {
        panic!("refresh response");
    };
    assert_eq!(
        response.delivery_session_params,
        common::delivery_for_request(CredentialMethod::RefreshToken, &refresh_request)
    );
}

#[test]
fn delivery_records_zeroize() {
    assert_eq!(
        CREDENTIAL_DELIVERY_NOISE_PROFILE,
        "Noise_KK_25519_ChaChaPoly_SHA256"
    );
    let mut record = SensitiveDeliveryRecord::new(b"access-token".to_vec(), 64).unwrap();
    let mut destination = [0; 12];
    record.copy_to(&mut destination).unwrap();
    destination.fill(0);
    record.clear();
    assert!(record.is_zeroized());
}

#[derive(Clone)]
struct FixedAdmission {
    authorized: DeliverySessionParams,
}

impl TestAdmission for FixedAdmission {
    fn authorize(
        &self,
        method: CredentialMethod,
        _request: &CredentialRequest,
    ) -> Result<CredentialAuthorization, CredentialServiceError> {
        CredentialAuthorization::new_for_subject(
            method,
            Some(self.authorized.clone()),
            subject_context(),
        )
        .and_then(|authorization| authorization.with_authenticated_session(session_binding()))
    }
}

struct BindingReplacingProvider {
    inner: EntraCredentialProvider,
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
fn adapter_refuses_an_entra_provider_binding_replacement() {
    let authorized = delivery(CredentialMethod::AcquireToken, 1);
    let replacement = delivery(CredentialMethod::AcquireToken, 2);
    let (provider, _) = setup();
    let server = ProviderHarness::new(
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

/// A refresh may not reach for an operation the admitted relationship never
/// granted it, and the identity Guest is never asked.
#[tokio::test]
async fn a_refresh_cannot_widen_the_allowed_operations() {
    let (provider, client) = setup();
    let acquire = request("idem-admitted-acquire");
    let relationship = relationship(&acquire, &[OperationClass::AcquireToken], 1);

    let admitted = call_admitted(
        &provider,
        &relationship,
        common::delivery_session(
            &acquire,
            OperationClass::AcquireToken,
            1,
            AUDIENCE,
            1,
        ),
        1,
        CredentialMethod::AcquireToken,
        &acquire,
    )
    .await
    .expect("the admitted relationship serves the acquisition");
    let CredentialResponse::AcquireToken(delivered) = admitted else {
        panic!("acquire response");
    };
    assert_eq!(delivered.metadata.rotation_generation, 1);
    assert_eq!(client.issue_calls.load(Ordering::SeqCst), 1);

    let refresh = request("idem-admitted-refresh");
    assert_eq!(
        call_admitted(
            &provider,
            &relationship,
            common::delivery_session(
                &refresh,
                OperationClass::RefreshToken,
                1,
                AUDIENCE,
                1,
            ),
            1,
            CredentialMethod::RefreshToken,
            &refresh,
        )
        .await
        .expect_err("an ungranted operation is refused")
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );
    assert_eq!(
        client.issue_calls.load(Ordering::SeqCst),
        1,
        "the refused refresh never reached the identity Guest"
    );
    assert_eq!(client.refresh_calls.load(Ordering::SeqCst), 0);
}

/// A refresh may not re-scope the audience the `Credential` row was admitted
/// for, even when the relationship does grant the operation.
#[tokio::test]
async fn a_refresh_cannot_widen_the_audience() {
    let (provider, client) = setup();
    let acquire = request("idem-audience-acquire");
    let relationship = relationship(&acquire, &[OperationClass::AcquireToken], 1)
        .delivering(OperationClass::RefreshToken);

    let refresh = request("idem-audience-refresh");
    assert_eq!(
        call_admitted(
            &provider,
            &relationship,
            common::delivery_session(
                &refresh,
                OperationClass::RefreshToken,
                1,
                "keyvault-datasphere",
                1,
            ),
            1,
            CredentialMethod::RefreshToken,
            &refresh,
        )
        .await
        .expect_err("a re-scoped audience is refused")
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );
    assert_eq!(
        client.refresh_calls.load(Ordering::SeqCst),
        0,
        "a re-scoped audience never reached the identity Guest"
    );
}

/// A replaced consumer component, a reconnected Provider session, and a
/// superseded delivery session each refuse the old delivery.
#[tokio::test]
async fn a_changed_consumer_or_provider_session_rejects_the_old_delivery() {
    let (provider, client) = setup();
    let request = request("idem-fenced");

    // The replacement session is internally consistent - it carries the new
    // component generation in both the session and the delivery - so only the
    // relationship's own fence can refuse it.
    let replaced_consumer = relationship(&request, &[OperationClass::AcquireToken], 1);
    assert_eq!(
        call_admitted(
            &provider,
            &replaced_consumer,
            common::delivery_session(&request, OperationClass::AcquireToken, 1, AUDIENCE, 2),
            2,
            CredentialMethod::AcquireToken,
            &request,
        )
        .await
        .expect_err("a replaced consumer component is refused")
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );

    let reconnected_session = relationship(&request, &[OperationClass::AcquireToken], 1);
    assert_eq!(
        call_admitted(
            &provider,
            &reconnected_session,
            common::delivery_session(&request, OperationClass::AcquireToken, 1, AUDIENCE, 1),
            2,
            CredentialMethod::AcquireToken,
            &request,
        )
        .await
        .expect_err("a reconnected Provider session is refused")
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );

    let superseded_session = relationship(&request, &[OperationClass::AcquireToken], 2);
    assert_eq!(
        call_admitted(
            &provider,
            &superseded_session,
            common::delivery_session(&request, OperationClass::AcquireToken, 1, AUDIENCE, 1),
            1,
            CredentialMethod::AcquireToken,
            &request,
        )
        .await
        .expect_err("a superseded delivery session is refused")
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );

    assert_eq!(
        client.issue_calls.load(Ordering::SeqCst),
        0,
        "a fenced delivery never reached the identity Guest"
    );
}

/// A cloud acquisition failure under the admitted relationship leaves the
/// owning resource honestly degraded instead of reporting a usable lease.
#[tokio::test]
async fn a_cloud_acquisition_failure_keeps_an_honest_degraded_state() {
    let (provider, client) = setup();
    let request = request("idem-cloud-failure");
    let relationship = relationship(&request, &[OperationClass::AcquireToken], 1);
    *client.issue_error.try_lock().unwrap() = Some(EntraClientError::Unavailable);

    assert_eq!(
        call_admitted(
            &provider,
            &relationship,
            common::delivery_session(&request, OperationClass::AcquireToken, 1, AUDIENCE, 1),
            1,
            CredentialMethod::AcquireToken,
            &request,
        )
        .await
        .expect_err("an unavailable identity Guest is not a lease")
        .code(),
        CredentialServiceErrorCode::ProviderUnavailable
    );
    assert_eq!(provider.active_lease_count().await, 0);
    assert_eq!(
        provider
            .resource_health(&ResourceRef::parse("Credential/work-entra").unwrap())
            .await,
        None,
        "a failed acquisition owns no lease to report health for"
    );
}

/// The graph spec and the published status carry no credential material. The
/// assertion is over the rendered bytes, so a future field fails here.
#[tokio::test]
async fn the_graph_spec_and_the_publication_carry_no_credential_material() {
    let (provider, client) = setup();
    let request = request("idem-rendered");
    let relationship = relationship(&request, &[OperationClass::AcquireToken], 1);
    let CredentialResponse::AcquireToken(delivered) = call_admitted(
        &provider,
        &relationship,
        common::delivery_session(&request, OperationClass::AcquireToken, 1, AUDIENCE, 1),
        1,
        CredentialMethod::AcquireToken,
        &request,
    )
    .await
    .expect("admitted acquisition")
    else {
        panic!("acquire response");
    };

    let desired = CredentialBindingRequest::new(
        ResourceRef::parse("Credential/work-entra").unwrap(),
        ResourceRef::parse("Guest/consumer").unwrap(),
        BindingSlot::parse("api").unwrap(),
        BoundedToken::parse(AUDIENCE).unwrap(),
        vec![d2b_contracts_resource::v3::CredentialOperation::AcquireToken],
        CredentialLifetime::new("600s", "900s").unwrap(),
    )
    .unwrap();
    let status = EntraController::new(provider.placement().clone())
        .reconcile(EntraClientState::Ready, Some(&delivered.metadata))
        .expect("status projection")
        .status;

    for (surface, rendered) in [
        ("credential binding spec", serde_json::to_value(&desired).unwrap()),
        ("credential status", serde_json::to_value(&status).unwrap()),
    ] {
        let bytes = serde_json::to_string(&rendered).unwrap();
        for material in [
            client.token_canary.as_str(),
            client.endpoint_canary.as_str(),
            client.cookie_canary.as_str(),
            "tenant-1234",
        ] {
            assert!(
                !bytes.contains(material),
                "{surface} published credential material {material:?}"
            );
        }
        assert_no_secret_shaped_field(&rendered, surface);
    }
}

/// Every field name in one rendered document, at any depth.
fn rendered_field_names(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Array(items) => {
            items
                .iter()
                .for_each(|item| rendered_field_names(item, out));
        }
        serde_json::Value::Object(fields) => fields.iter().for_each(|(key, item)| {
            out.push(key.clone());
            rendered_field_names(item, out);
        }),
        serde_json::Value::Null => {}
        _ => {}
    }
}

fn assert_no_secret_shaped_field(rendered: &serde_json::Value, surface: &str) {
    let mut names = Vec::new();
    rendered_field_names(rendered, &mut names);
    for name in names {
        let lowered = name.to_ascii_lowercase();
        assert!(
            !(lowered.contains("secret")
                || lowered.contains("token")
                || lowered.contains("password")
                || lowered.contains("cookie")
                || lowered.contains("key")),
            "{surface} published a secret-shaped field {name:?}"
        );
    }
}
