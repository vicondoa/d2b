mod common;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use d2b_contracts_provider::v3::credential::{
    CredentialAuthorization, CredentialMethod, CredentialProvider, CredentialRequest,
    CredentialResponse, CredentialServiceError, CredentialServiceErrorCode, DeliverySessionParams,
    OperationClass,
};
use d2b_contracts_resource::v3::{
    BindingSlot, BoundedToken, CredentialBindingRequest, CredentialLifetime, ResourceRef,
};
use d2b_provider_credential_secret_service::{
    LockPolicy, SecretServiceConfig, SecretServiceController, SecretServiceControllerHealth,
    SecretServiceCredentialProvider, SecretServiceSessionCapability, SecretServiceState,
};

use common::{
    Admission, AdmittedRelationship, ProviderHarness, SessionAdmission, SessionCapabilitySource,
    TestAdmission, request, setup,
};

#[test]
fn response_uses_the_read_only_adapter_binding() {
    let (provider, _) = setup(64);
    let server = ProviderHarness::new(provider, Admission);
    let response = server
        .call(CredentialMethod::AcquireToken, request("idem-delivery"))
        .unwrap();
    let CredentialResponse::AcquireToken(response) = response else {
        panic!("acquire response");
    };
    assert_eq!(response.delivery_session_params.sequence(), 1);
}

#[test]
fn refresh_response_preserves_the_authorization_owned_binding() {
    let (provider, _) = setup(64);
    let server = ProviderHarness::new(provider, Admission);
    server
        .call(CredentialMethod::AcquireToken, request("idem-acquire"))
        .unwrap();
    let response = server
        .call(CredentialMethod::RefreshToken, request("idem-refresh"))
        .unwrap();
    let CredentialResponse::RefreshToken(response) = response else {
        panic!("refresh response");
    };
    assert_eq!(
        response.delivery_session_params,
        common::delivery(CredentialMethod::RefreshToken, 1)
    );
}

#[derive(Clone)]
struct MismatchedAdmission {
    authorized: DeliverySessionParams,
}

impl TestAdmission for MismatchedAdmission {
    fn authorize(
        &self,
        method: CredentialMethod,
        _request: &CredentialRequest,
    ) -> Result<CredentialAuthorization, CredentialServiceError> {
        CredentialAuthorization::new(method, Some(self.authorized.clone()))
    }
}

struct BindingReplacingProvider {
    inner: SecretServiceCredentialProvider,
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

impl SessionCapabilitySource for BindingReplacingProvider {
    fn test_session_capability(&self) -> SecretServiceSessionCapability {
        self.inner
            .issue_session_capability(
                d2b_contracts_resource::v3::ResourceGeneration::new(1).unwrap(),
            )
            .expect("test provider must issue its placement-bound capability")
    }
}

#[test]
fn adapter_refuses_a_provider_response_with_a_different_binding() {
    let authorized = common::delivery(CredentialMethod::AcquireToken, 1);
    let replacement = common::delivery(CredentialMethod::AcquireToken, 2);
    let (provider, _) = setup(64);
    let server = ProviderHarness::new(
        BindingReplacingProvider {
            inner: provider,
            replacement,
        },
        MismatchedAdmission { authorized },
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

#[test]
fn provider_refuses_an_incoming_delivery_binding_for_another_credential() {
    let (provider, _) = common::setup(64);
    let wrong_binding = common::delivery_for(
        CredentialMethod::AcquireToken,
        1,
        d2b_contracts_resource::v3::ResourceRef::parse("Credential/other").unwrap(),
    );
    let server = ProviderHarness::new(
        provider,
        MismatchedAdmission {
            authorized: wrong_binding,
        },
    );
    assert_eq!(
        server
            .call(CredentialMethod::AcquireToken, request("wrong-credential"))
            .unwrap_err()
            .code(),
        CredentialServiceErrorCode::OperationDenied
    );
}

/// The audience the `Credential` row is admitted for.
const AUDIENCE: &str = "user-session";

/// Drive one admitted dispatch to completion.
///
/// The Secret Service Provider's capability authority and session teardown are
/// synchronous custody boundaries, so the harness drives the asynchronous
/// admitted-dispatch path from outside the runtime.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

fn relationship(
    request: &CredentialRequest,
    granted: &[OperationClass],
    sequence: u64,
) -> AdmittedRelationship {
    AdmittedRelationship::new(request, granted, AUDIENCE, sequence)
}

/// One authorization bound to the provider's own placement capability.
fn bound(
    capability: &Arc<SecretServiceSessionCapability>,
    delivery: DeliverySessionParams,
    provider_generation: u64,
    method: CredentialMethod,
    request: &CredentialRequest,
) -> Result<CredentialAuthorization, CredentialServiceError> {
    SessionAdmission {
        delivery,
        provider_generation,
        capability: Arc::clone(capability),
    }
    .authorize(method, request)
}

/// A refresh may not reach for an operation the admitted relationship never
/// granted it, and the keyring port is never asked.
#[test]
fn a_refresh_cannot_widen_the_allowed_operations() {
    let (provider, port) = setup(64);
    let capability = Arc::new(provider.test_session_capability());
    let acquire = request("idem-admitted-acquire");
    let relationship = relationship(&acquire, &[OperationClass::AcquireToken], 1);
    let acquire_authorization = bound(
        &capability,
        common::delivery_session(&acquire, OperationClass::AcquireToken, 1, AUDIENCE, 1),
        1,
        CredentialMethod::AcquireToken,
        &acquire,
    )
    .expect("authorization");

    let admitted = block_on(provider.dispatch_admitted(
        CredentialMethod::AcquireToken,
        &acquire,
        &acquire_authorization,
        &relationship,
    ))
    .expect("the admitted relationship serves the acquisition");
    let CredentialResponse::AcquireToken(delivered) = admitted else {
        panic!("acquire response");
    };
    assert_eq!(delivered.metadata.rotation_generation, 1);
    assert_eq!(port.issue_calls.load(Ordering::SeqCst), 1);

    let refresh = request("idem-admitted-refresh");
    let refresh_authorization = bound(
        &capability,
        common::delivery_session(&refresh, OperationClass::RefreshToken, 1, AUDIENCE, 1),
        1,
        CredentialMethod::RefreshToken,
        &refresh,
    )
    .expect("authorization");
    assert_eq!(
        block_on(provider.dispatch_admitted(
            CredentialMethod::RefreshToken,
            &refresh,
            &refresh_authorization,
            &relationship,
        ))
        .expect_err("an ungranted operation is refused")
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );
    assert_eq!(
        port.issue_calls.load(Ordering::SeqCst),
        1,
        "the refused refresh never reached the keyring"
    );
    assert_eq!(port.refresh_calls.load(Ordering::SeqCst), 0);
}

/// A refresh may not re-scope the audience the `Credential` row was admitted
/// for, even when the relationship does grant the operation.
#[test]
fn a_refresh_cannot_widen_the_audience() {
    let (provider, port) = setup(64);
    let capability = Arc::new(provider.test_session_capability());
    let acquire = request("idem-audience-acquire");
    let relationship = relationship(&acquire, &[OperationClass::AcquireToken], 1)
        .delivering(OperationClass::RefreshToken);
    let refresh = request("idem-audience-refresh");
    let refresh_authorization = bound(
        &capability,
        common::delivery_session(
            &refresh,
            OperationClass::RefreshToken,
            1,
            "machine-session",
            1,
        ),
        1,
        CredentialMethod::RefreshToken,
        &refresh,
    )
    .expect("authorization");

    assert_eq!(
        block_on(provider.dispatch_admitted(
            CredentialMethod::RefreshToken,
            &refresh,
            &refresh_authorization,
            &relationship,
        ))
        .expect_err("a re-scoped audience is refused")
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );
    assert_eq!(
        port.refresh_calls.load(Ordering::SeqCst),
        0,
        "a re-scoped audience never reached the keyring"
    );
}

/// A replaced consumer component, a reconnected Provider session, and a
/// superseded delivery session each refuse the old delivery.
#[test]
fn a_changed_consumer_or_provider_session_rejects_the_old_delivery() {
    let (provider, port) = setup(64);
    let capability = Arc::new(provider.test_session_capability());
    let request = request("idem-fenced");

    // The replacement session is internally consistent - it carries the new
    // component generation in both the session and the delivery - so only the
    // relationship's own fence can refuse it.
    let replaced_consumer = relationship(&request, &[OperationClass::AcquireToken], 1);
    let replaced = bound(
        &capability,
        common::delivery_session(&request, OperationClass::AcquireToken, 1, AUDIENCE, 2),
        2,
        CredentialMethod::AcquireToken,
        &request,
    )
    .expect("authorization");
    assert_eq!(
        block_on(provider.dispatch_admitted(
            CredentialMethod::AcquireToken,
            &request,
            &replaced,
            &replaced_consumer,
        ))
        .expect_err("a replaced consumer component is refused")
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );

    let reconnected_session = relationship(&request, &[OperationClass::AcquireToken], 1);
    let reconnected = bound(
        &capability,
        common::delivery_session(&request, OperationClass::AcquireToken, 1, AUDIENCE, 1),
        2,
        CredentialMethod::AcquireToken,
        &request,
    )
    .expect("authorization");
    assert_eq!(
        block_on(provider.dispatch_admitted(
            CredentialMethod::AcquireToken,
            &request,
            &reconnected,
            &reconnected_session,
        ))
        .expect_err("a reconnected Provider session is refused")
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );

    let superseded_session = relationship(&request, &[OperationClass::AcquireToken], 2);
    let superseded = bound(
        &capability,
        common::delivery_session(&request, OperationClass::AcquireToken, 1, AUDIENCE, 1),
        1,
        CredentialMethod::AcquireToken,
        &request,
    )
    .expect("authorization");
    assert_eq!(
        block_on(provider.dispatch_admitted(
            CredentialMethod::AcquireToken,
            &request,
            &superseded,
            &superseded_session,
        ))
        .expect_err("a superseded delivery session is refused")
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );

    assert_eq!(
        port.issue_calls.load(Ordering::SeqCst),
        0,
        "a fenced delivery never reached the keyring"
    );
}

/// A transport disconnect under the admitted relationship revokes the lease
/// and leaves the keyring honestly unavailable rather than serving the old
/// session again.
#[test]
fn a_disconnect_keeps_an_honest_degraded_state() {
    let (provider, port) = setup(64);
    let capability = Arc::new(provider.test_session_capability());
    let acquired = request("idem-disconnect");
    let relationship = relationship(&acquired, &[OperationClass::AcquireToken], 1);
    let authorization = bound(
        &capability,
        common::delivery_session(&acquired, OperationClass::AcquireToken, 1, AUDIENCE, 1),
        1,
        CredentialMethod::AcquireToken,
        &acquired,
    )
    .expect("authorization");
    let CredentialResponse::AcquireToken(delivered) =
        block_on(provider.dispatch_admitted(
            CredentialMethod::AcquireToken,
            &acquired,
            &authorization,
            &relationship,
        ))
        .expect("admitted acquisition")
    else {
        panic!("acquire response");
    };

    provider.disconnect(&authorization).expect("disconnect");
    assert_eq!(port.revoke_calls.load(Ordering::SeqCst), 1);

    // The keyring's own state is what the controller publishes, so a
    // disconnected session is never reported as a usable lease.
    let projection = SecretServiceController::new(
        SecretServiceConfig::new("login collection", 64, LockPolicy::FailClosed).unwrap(),
    )
    .reconcile(SecretServiceState::Locked, None)
    .expect("status projection");
    assert_eq!(
        projection.health,
        SecretServiceControllerHealth::Unavailable
    );
    assert!(projection.status.credential().is_none());

    // The revoked lease cannot be refreshed through the same relationship.
    let refresh = request("idem-disconnect-refresh");
    let refresh_authorization = bound(
        &capability,
        common::delivery_session(&refresh, OperationClass::RefreshToken, 1, AUDIENCE, 1),
        1,
        CredentialMethod::RefreshToken,
        &refresh,
    )
    .expect("authorization");
    assert_eq!(
        block_on(provider.dispatch_admitted(
            CredentialMethod::RefreshToken,
            &refresh,
            &refresh_authorization,
            &relationship,
        ))
        .expect_err("a disconnected session is refused")
        .code(),
        CredentialServiceErrorCode::OperationDenied
    );
    assert_eq!(
        port.refresh_calls.load(Ordering::SeqCst),
        0,
        "the disconnected session never reached the keyring"
    );
    drop(delivered);
}

/// The graph spec and the published status carry no credential material. The
/// assertion is over the rendered bytes, so a future field fails here.
#[test]
fn the_graph_spec_and_the_publication_carry_no_credential_material() {
    let (provider, port) = setup(64);
    let capability = Arc::new(provider.test_session_capability());
    let request = request("idem-rendered");
    let relationship = relationship(&request, &[OperationClass::AcquireToken], 1);
    let authorization = bound(
        &capability,
        common::delivery_session(&request, OperationClass::AcquireToken, 1, AUDIENCE, 1),
        1,
        CredentialMethod::AcquireToken,
        &request,
    )
    .expect("authorization");
    let CredentialResponse::AcquireToken(delivered) =
        block_on(provider.dispatch_admitted(
            CredentialMethod::AcquireToken,
            &request,
            &authorization,
            &relationship,
        ))
        .expect("admitted acquisition")
    else {
        panic!("acquire response");
    };

    let desired = CredentialBindingRequest::new(
        request.credential_ref().clone(),
        ResourceRef::parse("Process/keyring-consumer").unwrap(),
        BindingSlot::parse("keyring").unwrap(),
        BoundedToken::parse(AUDIENCE).unwrap(),
        vec![d2b_contracts_resource::v3::CredentialOperation::AcquireToken],
        CredentialLifetime::new("600s", "900s").unwrap(),
    )
    .unwrap();
    let status = SecretServiceController::new(
        SecretServiceConfig::new("login collection", 64, LockPolicy::FailClosed).unwrap(),
    )
    .reconcile(SecretServiceState::Unlocked, Some(&delivered.metadata))
    .expect("status projection")
    .status;

    for (surface, rendered) in [
        ("credential binding spec", serde_json::to_value(&desired).unwrap()),
        ("credential status", serde_json::to_value(&status).unwrap()),
    ] {
        let bytes = serde_json::to_string(&rendered).unwrap();
        for material in [
            port.credential_canary.as_str(),
            port.object_path_canary.as_str(),
            "login collection",
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
