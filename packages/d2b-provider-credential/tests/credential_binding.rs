//! The `CredentialBinding` realization: delivery authority that cannot be
//! reused, and a rendered surface that carries no credential material.
//!
//! The four properties under test are the ones a generic binding lifecycle
//! has to earn for a typed secret:
//!
//! 1. **AE10.** An audience, operation, component, or credential revision
//!    change invalidates the prior authority; new use needs a fresh
//!    admission.
//! 2. No secret byte can reach a graph spec, a generic binding status, an
//!    audit record, or a publication snapshot. This is asserted against the
//!    *rendered* bytes, not against the types.
//! 3. A failed remote revoke leaves conservative lease state and can never
//!    report `Released`.
//! 4. An expired delivery cannot be renewed through a stale helper leg.

use std::sync::Arc;

use d2b_contracts_provider::v3::credential::{
    AudienceToken, CredentialAuthorization, CredentialDeliveryEvidence as ObservedDeliveryEvidence,
    CredentialLeaseState, CredentialMethod, CredentialScope, CredentialSpec, DeliveryRouteDigest,
    ExpirySpec, OperationClass, RevocationSpec, RotationPolicyClass, RotationSpec,
    admit_credential_delivery,
};
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingArbitration, BindingAuthorization, BindingContractError, BindingKey,
    BindingKind, BindingLifecycleState, BindingRealizationFacet, BindingRealizationSupport,
    BindingRowError, BindingSlot, BindingSpecFingerprint, ControllerGeneration,
    CredentialBindingRequest, CredentialBindingSpec, CredentialLifetime, CredentialOperation,
    DesiredDigest, DesiredRevision, FreshnessTuple, MAX_CREDENTIAL_LIFETIME_MS,
    MIN_CREDENTIAL_LIFETIME_MS, RefusalReason, RequestedRights, ResourceGeneration, ResourceRef,
    ResourceUid, SourceAdmission, StoreIncarnation, ZoneId, admit_binding_row_refs,
    canonical_json_bytes,
};
use d2b_provider_credential::test_support::{
    RecordingRuntime, RecordingSession, log, recording_facets,
};
use d2b_provider_credential::{
    CREDENTIAL_DELIVERY_SLOT, CredentialBindingAdmission, CredentialBindingDriverArgs,
    CredentialBindingDriverStatus, CredentialDeliveryAuthority, CredentialDeliveryEvidence,
    CredentialLeaseFacts, CredentialSourcePolicy, UndeliveredReason, canonical_binding_rows,
    credential_binding_descriptor, credential_binding_row_name, credential_binding_spec_decoder,
    credential_binding_support, credential_source_decision, delivery_operation,
};
use d2b_provider_credential::{CredentialRevocationOutcome, CredentialRevocationReport};
use d2b_provider_toolkit::testing::fakes::{RecordingManagerEndpoint, RecordingRequeue};
use d2b_resource_runtime::context::ResourceContext;
use d2b_resource_runtime::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome,
};
use d2b_resource_runtime::error::{FailureClass, FailureKinds};
use d2b_resource_runtime::manager::deterministic_uid;
use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};

const ZONE: &str = "work";
const CREDENTIAL: &str = "Credential/api-key";
const CONSUMER: &str = "Process/web";
const GUEST: &str = "Guest/work-vm";
const CONSUMER_PROVIDER: &str = d2b_provider_credential::SECRET_SERVICE_PROVIDER_REF;
const CREDENTIAL_UID: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
const CONSUMER_UID: &str = "9c5b94d1-6470-4b1a-9a41-0016d3cca427";
const GUEST_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
const AUDIENCE: &str = "azure-resource-manager";
const ROUTE_DIGEST: &str =
    "sha256:6f1c1b6f2a6f2cbb1f2e5b2f0c3a7a2b6c9d0e1f2a3b4c5d6e7f809a1b2c3d4e";
const DIGEST_INPUT: &[u8] = br#"{"spec":{"kind":"credential"}}"#;
const NOW: u64 = 1_760_000_000_000;

fn zone() -> ZoneId {
    ZoneId::parse(ZONE).expect("zone")
}

fn resource(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("resource ref")
}

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).expect("uid")
}

fn generation(value: u64) -> ResourceGeneration {
    ResourceGeneration::new(value).expect("generation")
}

fn audience() -> AudienceToken {
    AudienceToken::parse(AUDIENCE).expect("audience")
}

fn route_digest() -> DeliveryRouteDigest {
    DeliveryRouteDigest::parse(ROUTE_DIGEST).expect("route digest")
}

fn lifetime() -> CredentialLifetime {
    CredentialLifetime::new("600s", "900s").expect("lifetime")
}

fn revision() -> DesiredRevision {
    DesiredRevision::INITIAL.try_next().expect("revision")
}

fn dependency() -> FreshnessTuple {
    FreshnessTuple::new(
        zone(),
        StoreIncarnation::parse("store-1").expect("incarnation"),
        resource(CREDENTIAL),
        uid(CREDENTIAL_UID),
        revision(),
        DesiredDigest::of(DIGEST_INPUT),
    )
}

/// One `Credential` row's committed spec: the source's own policy.
fn credential_spec(operations: &[OperationClass], max_lease_lifetime_ms: u64) -> CredentialSpec {
    CredentialSpec::new(
        CredentialScope::new(Some(resource("Host/studio")), None, None).expect("scope"),
        audience(),
        None,
        operations.to_vec(),
        RotationSpec::new(
            RotationPolicyClass::OnExpiry,
            None,
            max_lease_lifetime_ms,
        )
        .expect("rotation"),
        ExpirySpec::new(0).expect("expiry"),
        RevocationSpec::default(),
        None,
        None,
    )
    .expect("credential spec")
}

/// One `Credential` row whose placement is declared by its own scope.
///
/// A scope names a Host or a Guest, which is the whole of the row's consumer
/// declaration; `None` is a row that declares none.
fn credential_spec_scoped(
    execution_ref: Option<&str>,
    operations: &[OperationClass],
    max_lease_lifetime_ms: u64,
) -> CredentialSpec {
    CredentialSpec::new(
        CredentialScope::new(execution_ref.map(resource), None, None).expect("scope"),
        audience(),
        Some(resource(CONSUMER_PROVIDER)),
        operations.to_vec(),
        RotationSpec::new(
            RotationPolicyClass::OnExpiry,
            None,
            max_lease_lifetime_ms,
        )
        .expect("rotation"),
        ExpirySpec::new(0).expect("expiry"),
        RevocationSpec::default(),
        None,
        None,
    )
    .expect("credential spec")
}

fn request(audience_token: &str, operations: Vec<CredentialOperation>) -> CredentialBindingRequest {
    request_with(audience_token, operations, "600s", "900s")
}

fn request_with(
    audience_token: &str,
    operations: Vec<CredentialOperation>,
    valid_for: &str,
    expires_in: &str,
) -> CredentialBindingRequest {
    CredentialBindingRequest::new(
        resource(CREDENTIAL),
        resource(CONSUMER),
        BindingSlot::parse("api").expect("slot"),
        d2b_contracts_resource::v3::execution_policy::BoundedToken::parse(audience_token)
            .expect("audience token"),
        operations,
        CredentialLifetime::new(valid_for, expires_in).expect("lifetime"),
    )
    .expect("request")
}

fn source_admission(request: &CredentialBindingRequest) -> SourceAdmission {
    SourceAdmission::new(
        request
            .key(zone(), uid(CREDENTIAL_UID), uid(CONSUMER_UID))
            .expect("key"),
        vec![RequestedRights::Consume],
        BindingArbitration::Shared,
    )
    .expect("source admission")
}

struct AdmissionFixture {
    spec: CredentialSpec,
    request: CredentialBindingRequest,
}

impl AdmissionFixture {
    fn new(operations: &[OperationClass], max_lease_lifetime_ms: u64) -> Self {
        let spec = credential_spec(operations, max_lease_lifetime_ms);
        let request = request(
            AUDIENCE,
            spec.allowed_operations()
                .iter()
                .copied()
                .filter_map(delivery_operation)
                .collect(),
        );
        Self { spec, request }
    }

    fn admission(&self) -> CredentialBindingAdmission {
        CredentialBindingAdmission::new(
            zone(),
            uid(CREDENTIAL_UID),
            uid(CONSUMER_UID),
            CredentialSourcePolicy::from_spec(&self.spec),
            BindingSpecFingerprint::from_request(&self.spec),
            BindingAuthorization::granted(),
            source_admission(&self.request),
            credential_binding_support(),
            vec![dependency()],
            resource(CREDENTIAL),
            generation(3),
            resource(CONSUMER_PROVIDER),
            generation(7),
            generation(11),
            4,
            audience(),
            route_digest(),
            4096,
            NOW,
        )
    }

    fn admit(&self) -> CredentialDeliveryAuthority {
        CredentialDeliveryAuthority::admit(self.request.clone(), self.admission())
            .expect("admitted delivery")
    }
}

fn evidence() -> CredentialDeliveryEvidence {
    CredentialDeliveryEvidence::new(generation(3), generation(7), generation(11), 4, vec![
        dependency(),
    ], NOW + 1_000)
}


/// The sorted field names of one rendered object.
fn object_keys(value: &serde_json::Value) -> Vec<String> {
    let mut keys = match value.as_object() {
        Some(fields) => fields.keys().map(|key| key.to_owned()).collect(),
        None => Vec::new(),
    };
    keys.sort();
    keys
}

/// The closed graph-spec field set a `CredentialBinding` request renders.
fn spec_fields() -> Vec<String> {
    [
        "audience",
        "consumerRef",
        "lifetime",
        "operations",
        "slot",
        "sourceRef",
    ]
    .map(str::to_owned)
    .into()
}

/// The closed generic-status field set a delivery relationship renders.
fn status_fields() -> Vec<String> {
    ["deadlineUnixMs", "expiryUnixMs", "sequence", "state"]
        .map(str::to_owned)
        .into()
}

/// Every field name in one rendered JSON document, at any depth.
fn rendered_field_names(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .for_each(|item| rendered_field_names(item, out)),
        serde_json::Value::Object(fields) => fields.iter().for_each(|(key, item)| {
            out.push(key.clone());
            rendered_field_names(item, out);
        }),
        _ => {}
    }
}

/// Field names that could only exist if credential material had been carried
/// into a rendered surface.
const SECRET_SHAPED_FIELDS: &[&str] = &[
    "accesstoken",
    "assertion",
    "bearer",
    "clientsecret",
    "jwt",
    "keyhandle",
    "leasehandle",
    "material",
    "nonce",
    "passphrase",
    "password",
    "plaintext",
    "privatekey",
    "record",
    "refreshtoken",
    "secret",
    "sessionkey",
    "token",
    "transcript",
];

/// Whether a rendered document has no field a credential byte could occupy.
///
/// The scan is over field *names* at every depth, which is the part a new
/// member would change: a value that happens to read like a token (an
/// `acquire-token` operation class, a `Credential/api-key` reference) is a
/// bounded non-secret identifier, while a field that could hold material is
/// named. The closed field-set assertions beside this are the second half of
/// the proof: they fail when a field is added at all.
fn assert_no_secret_shaped_field(rendered: &serde_json::Value) {
    let mut names = Vec::new();
    rendered_field_names(rendered, &mut names);
    for name in &names {
        let lowered = name.to_ascii_lowercase();
        for field in SECRET_SHAPED_FIELDS {
            assert!(
                !lowered.contains(field),
                "rendered surface carries secret-shaped field {field:?} in {name:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// AE10: prior authority cannot be reused
// ---------------------------------------------------------------------------

/// A delivery session that was minted for an admitted operation class is the
/// only authority the relationship recognizes afterwards. Re-minting through
/// a fresh, correctly authorized delivery invalidates the earlier session at
/// the protocol boundary, so a component that kept an old handle cannot
/// present it again.
#[test]
fn a_reminted_session_invalidates_the_earlier_one() {
    let fixture = AdmissionFixture::new(&[OperationClass::AcquireToken], 0);
    let mut authority = fixture.admit();

    let first = authority
        .mint(CredentialOperation::AcquireToken, &evidence())
        .expect("first delivery");
    assert_eq!(first.sequence(), 1);
    authority
        .verifies(&first.delivery_identity())
        .expect("the current session is the admitted one");

    let second = authority
        .mint(CredentialOperation::AcquireToken, &evidence())
        .expect("second delivery");
    assert_eq!(second.sequence(), 2);
    authority
        .verifies(&second.delivery_identity())
        .expect("the newest session is the admitted one");
    let stale = authority
        .verifies(&first.delivery_identity())
        .expect_err("the earlier session is no longer the admitted authority");
    assert_eq!(stale.stage(), AdmissionStage::Activate);
    assert_eq!(stale.reason(), RefusalReason::StaleAuthority);
}

/// The same check at the service boundary: a Provider handed the earlier
/// session's identity for the current method is denied, because the audience,
/// generations, operation class, or sequence no longer match what the
/// admitted relationship authorized.
#[test]
fn the_service_denies_a_session_the_relationship_no_longer_authorized() {
    let fixture = AdmissionFixture::new(&[OperationClass::AcquireToken], 0);
    let mut authority = fixture.admit();
    let first = authority
        .mint(CredentialOperation::AcquireToken, &evidence())
        .expect("first delivery");
    authority
        .mint(CredentialOperation::AcquireToken, &evidence())
        .expect("second delivery");

    let admitted = authority.delivery_identity();
    let earlier = CredentialAuthorization::new(
        CredentialMethod::AcquireToken,
        Some(first.clone()),
    )
    .expect("authorization")
    .verifies_delivery(CredentialMethod::AcquireToken, &admitted)
    .expect_err("the earlier session is refused at the service boundary");
    assert_eq!(earlier.code(), d2b_contracts_provider::v3::credential::CredentialServiceErrorCode::OperationDenied);

    let current = CredentialAuthorization::new(CredentialMethod::AcquireToken, Some(first))
        .expect("authorization");
    // The method must be the one the identity was minted for; a metadata
    // method never establishes a delivery session.
    current
        .verifies_delivery(CredentialMethod::InspectMetadata, &admitted)
        .expect_err("a non-delivery method cannot verify a delivery session");
}

/// An operation the request never admitted is refused at the mint, even though
/// the relationship is otherwise current. A binding that grants only token
/// acquisition cannot sign or refresh.
#[test]
fn an_unadmitted_operation_cannot_be_minted() {
    let fixture = AdmissionFixture::new(&[OperationClass::AcquireToken], 0);
    let mut authority = fixture.admit();
    assert!(!authority.admits_operation(CredentialOperation::SignChallenge));
    let refusal = authority
        .mint(CredentialOperation::SignChallenge, &evidence())
        .expect_err("signing is outside the admitted set");
    assert_eq!(refusal.stage(), AdmissionStage::Admit);
    assert_eq!(refusal.reason(), RefusalReason::SourcePolicyRefused);
}

/// A consumer component restart advances the component generation. The
/// earlier authority is then fenced even though the Credential row, its
/// audience, and its desired revision are all unchanged.
#[test]
fn a_component_restart_fences_the_prior_authority() {
    let fixture = AdmissionFixture::new(&[OperationClass::AcquireToken], 0);
    let mut authority = fixture.admit();
    authority
        .mint(CredentialOperation::AcquireToken, &evidence())
        .expect("first delivery");

    let restarted = CredentialDeliveryEvidence::new(generation(3), generation(8), generation(11), 4, vec![
        dependency(),
    ], NOW + 1_000);
    let refusal = authority
        .mint(CredentialOperation::AcquireToken, &restarted)
        .expect_err("a restarted component cannot reuse the earlier authority");
    assert_eq!(refusal.stage(), AdmissionStage::Admit);
    assert_eq!(refusal.reason(), RefusalReason::StaleAuthority);
}

/// A credential rotation and a changed provider assignment each fence the
/// earlier authority on their own.
#[test]
fn rotation_and_provider_assignment_each_fence_the_prior_authority() {
    let fixture = AdmissionFixture::new(&[OperationClass::AcquireToken], 0);
    let mut authority = fixture.admit();

    let rotated = CredentialDeliveryEvidence::new(generation(3), generation(7), generation(11), 5, vec![
        dependency(),
    ], NOW + 1_000);
    assert_eq!(
        authority
            .mint(CredentialOperation::AcquireToken, &rotated)
            .expect_err("a rotated credential cannot reuse the earlier authority")
            .reason(),
        RefusalReason::StaleAuthority
    );

    let reassigned = CredentialDeliveryEvidence::new(generation(3), generation(7), generation(12), 4, vec![
        dependency(),
    ], NOW + 1_000);
    assert_eq!(
        authority
            .mint(CredentialOperation::AcquireToken, &reassigned)
            .expect_err("a reassigned provider cannot reuse the earlier authority")
            .reason(),
        RefusalReason::StaleAuthority
    );

    authority
        .mint(CredentialOperation::AcquireToken, &evidence())
        .expect("the unchanged relationship still mints");
}

/// A committed desired revision that moves without a generation change still
/// invalidates the prior authority: the dependency set the admission was
/// evaluated against is the fence, not the spec generation a status happens to
/// report.
#[test]
fn a_moved_dependency_revision_invalidates_prior_authority() {
    let fixture = AdmissionFixture::new(&[OperationClass::AcquireToken], 0);
    let mut authority = fixture.admit();
    authority
        .mint(CredentialOperation::AcquireToken, &evidence())
        .expect("first delivery");

    let advanced = vec![FreshnessTuple::new(
        zone(),
        StoreIncarnation::parse("store-1").expect("incarnation"),
        resource(CREDENTIAL),
        uid(CREDENTIAL_UID),
        revision().try_next().expect("next revision"),
        DesiredDigest::of(br#"{"spec":{"kind":"credential","revision":2}}"#),
    )];
    let moved = CredentialDeliveryEvidence::new(
        generation(3),
        generation(7),
        generation(11),
        4,
        advanced,
        NOW + 1_000,
    );
    let refusal = authority
        .mint(CredentialOperation::AcquireToken, &moved)
        .expect_err("a newer committed revision invalidates the prior authority");
    assert_eq!(refusal.stage(), AdmissionStage::Admit);
    assert_eq!(refusal.reason(), RefusalReason::StaleAuthority);
}

/// The source's own policy is the ceiling. A request for another audience, a
/// longer lifetime, or an operation the row does not grant is refused at the
/// source policy, before the generic evaluator's reservation and before any
/// provider could be asked for material.
#[test]
fn the_source_policy_bounds_audience_operations_and_lifetime() {
    let spec = credential_spec(&[OperationClass::AcquireToken], 600_000);
    let policy = CredentialSourcePolicy::from_spec(&spec);

    let admitted = request_with(
        AUDIENCE,
        vec![CredentialOperation::AcquireToken],
        "600s",
        "600s",
    );
    policy
        .admits_delivery(&admitted, &audience())
        .expect("the row's own audience, operation, and lifetime are admitted");

    let other_audience = AudienceToken::parse("storage").expect("audience");
    assert_eq!(
        policy
            .admits_delivery(&admitted, &other_audience)
            .expect_err("a delivery for another audience is refused")
            .reason(),
        RefusalReason::SourcePolicyRefused
    );

    let longer = CredentialBindingRequest::new(
        resource(CREDENTIAL),
        resource(CONSUMER),
        BindingSlot::parse("api").expect("slot"),
        d2b_contracts_resource::v3::execution_policy::BoundedToken::parse(AUDIENCE)
            .expect("audience token"),
        vec![CredentialOperation::AcquireToken],
        CredentialLifetime::new("600s", "3600s").expect("lifetime"),
    )
    .expect("request");
    assert_eq!(
        policy
            .admits_delivery(&longer, &audience())
            .expect_err("a lifetime beyond the row's ceiling is refused")
            .reason(),
        RefusalReason::SourcePolicyRefused
    );

    let signing = request(
        AUDIENCE,
        vec![
            CredentialOperation::AcquireToken,
            CredentialOperation::SignChallenge,
        ],
    );
    assert_eq!(
        policy
            .admits_delivery(&signing, &audience())
            .expect_err("an operation the row does not grant is refused")
            .reason(),
        RefusalReason::SourcePolicyRefused
    );

    assert!(!policy.admits_class(OperationClass::RevokeToken));
    assert!(policy.admits_class(OperationClass::AcquireToken));
    assert_eq!(policy.audience().as_str(), AUDIENCE);
    assert_eq!(policy.max_lease_lifetime_ms(), 600_000);
}

/// A request the source policy refuses never becomes an authority, and a
/// missing authorization grant is refused at the authorization stage.
#[test]
fn admission_refuses_before_any_delivery_authority_exists() {
    let mut fixture = AdmissionFixture::new(&[OperationClass::AcquireToken], 0);
    fixture.request = request(AUDIENCE, vec![CredentialOperation::SignChallenge]);
    let refusal = CredentialDeliveryAuthority::admit(
        fixture.request.clone(),
        fixture.admission(),
    )
    .expect_err("signing is outside the row's granted operations");
    assert_eq!(refusal.stage(), AdmissionStage::Admit);
    assert_eq!(refusal.reason(), RefusalReason::SourcePolicyRefused);

    let granted = AdmissionFixture::new(&[OperationClass::AcquireToken], 0);
    let unauthorized = CredentialBindingAdmission::new(
        zone(),
        uid(CREDENTIAL_UID),
        uid(CONSUMER_UID),
        CredentialSourcePolicy::from_spec(&granted.spec),
        BindingSpecFingerprint::from_request(&granted.spec),
        BindingAuthorization::absent(),
        source_admission(&granted.request),
        credential_binding_support(),
        vec![dependency()],
        resource(CREDENTIAL),
        generation(3),
        resource(CONSUMER_PROVIDER),
        generation(7),
        generation(11),
        4,
        audience(),
        route_digest(),
        4096,
        NOW,
    );
    let refusal = CredentialDeliveryAuthority::admit(granted.request, unauthorized)
        .expect_err("an unauthorized request is refused");
    assert_eq!(refusal.stage(), AdmissionStage::Authorize);
    assert_eq!(refusal.reason(), RefusalReason::IdentityNotAuthorized);
}

// ---------------------------------------------------------------------------
// No credential material in a rendered surface
// ---------------------------------------------------------------------------

/// The graph spec is the consumer's own request, and the generic status is
/// what a status API, an audit record, and a publication snapshot answer.
/// Neither rendered document may carry a field that could hold material -
/// asserted over the rendered bytes, so a future field addition fails here
/// rather than shipping a leak.
#[test]
fn the_rendered_spec_and_status_carry_no_credential_material() {
    let fixture = AdmissionFixture::new(&[OperationClass::AcquireToken], 0);
    let mut authority = fixture.admit();
    authority
        .mint(CredentialOperation::AcquireToken, &evidence())
        .expect("first delivery");

    let spec = serde_json::to_value(&fixture.request).expect("request json");
    let status = authority.status(None).render();

    assert_eq!(object_keys(&spec), spec_fields());
    assert_eq!(object_keys(&status), status_fields());
    assert_no_secret_shaped_field(&spec);
    assert_no_secret_shaped_field(&status);

    // Every value in the spec is a bounded identity, a duration, or an
    // operation class: the audience renders as the row's own audience token
    // and the lifetime as its two bounds, and nothing else is present.
    assert_eq!(spec["audience"], serde_json::json!(AUDIENCE));
    assert_eq!(spec["sourceRef"], serde_json::json!(CREDENTIAL));
    assert_eq!(spec["consumerRef"], serde_json::json!(CONSUMER));
    assert_eq!(spec["slot"], serde_json::json!("api"));
    assert_eq!(
        spec["operations"],
        serde_json::json!(["acquire-token"]),
        "the rendered operation set is the admitted class, not a payload"
    );
    assert_eq!(
        spec["lifetime"],
        serde_json::json!({"expiresIn": "900s", "validFor": "600s"})
    );

    // The status renders only the observed state, the two bounds, and the
    // replay counter.
    assert_eq!(status["state"], serde_json::json!("active"));
    assert_eq!(
        status["expiryUnixMs"],
        serde_json::json!(authority.expiry_unix_ms())
    );
    assert_eq!(
        status["deadlineUnixMs"],
        serde_json::json!(authority.deadline_unix_ms())
    );
    assert_eq!(status["sequence"], serde_json::json!(1));
}

/// The authority and its legs are the private path: they are not serializable
/// at all, and their `Debug` output carries no audience, credential identity,
/// route digest, or leg name.
#[test]
fn the_private_authority_is_not_serializable_and_prints_nothing_sensitive() {
    let fixture = AdmissionFixture::new(&[OperationClass::AcquireToken], 0);
    let mut authority = fixture.admit();
    authority
        .mint(CredentialOperation::AcquireToken, &evidence())
        .expect("first delivery");
    let leg = authority
        .open_leg("web-helper", &evidence())
        .expect("bounded helper leg");

    let rendered = format!("{authority:?}");
    for leaked in [CREDENTIAL, CONSUMER_PROVIDER, AUDIENCE, ROUTE_DIGEST, "web-helper"] {
        assert!(
            !rendered.contains(leaked),
            "the authority's Debug output leaked {leaked:?}: {rendered}"
        );
    }
    assert!(rendered.contains("<redacted>"));
    assert!(format!("{leg:?}").contains("<redacted>"));
    assert_eq!(leg.as_str(), "web-helper");
    assert_eq!(leg.sequence(), 1);
}

// ---------------------------------------------------------------------------
// Conservative revocation
// ---------------------------------------------------------------------------

/// A revoke the provider could not confirm leaves the lease outstanding, and
/// the relationship can never be reported as `Released` on that evidence -
/// with or without a post-revocation read.
#[test]
fn a_failed_remote_revoke_never_reports_released() {
    let fixture = AdmissionFixture::new(&[OperationClass::AcquireToken], 0);
    let mut authority = fixture.admit();
    authority
        .mint(CredentialOperation::AcquireToken, &evidence())
        .expect("first delivery");

    for observed in [
        None,
        Some(CredentialLeaseState::Active),
        Some(CredentialLeaseState::Unknown),
    ] {
        let report = CredentialRevocationReport::of(CredentialRevocationOutcome::Uncertain, observed);
        assert_eq!(report, CredentialRevocationReport::Unconfirmed);
        assert!(!report.releases());
        assert_ne!(
            authority.observe_revocation(report, false),
            BindingLifecycleState::Released
        );
        assert_ne!(
            authority.status(Some(report)).state(),
            BindingLifecycleState::Released
        );
    }

    // A confirmed revoke with a live lease is still not a release.
    let retained = CredentialRevocationReport::of(
        CredentialRevocationOutcome::Revoked,
        Some(CredentialLeaseState::Active),
    );
    assert_eq!(retained, CredentialRevocationReport::Retained);
    assert_ne!(
        authority.observe_revocation(retained, false),
        BindingLifecycleState::Released
    );

    // Only a confirmed revoke with no live lease releases, and even then
    // outstanding use keeps it revoking.
    let released =
        CredentialRevocationReport::of(CredentialRevocationOutcome::Revoked, Some(CredentialLeaseState::Revoked));
    assert!(released.releases());
    assert_eq!(
        authority.observe_revocation(released, false),
        BindingLifecycleState::Released
    );
    assert_eq!(
        authority.observe_revocation(released, true),
        BindingLifecycleState::Revoking
    );
}

/// Once new use is fenced, every report is `Draining` - never `Released` -
/// because outstanding use is still being driven closed.
#[test]
fn fenced_new_use_drains_and_cannot_release() {
    let fixture = AdmissionFixture::new(&[OperationClass::AcquireToken], 0);
    let mut authority = fixture.admit();
    authority
        .mint(CredentialOperation::AcquireToken, &evidence())
        .expect("first delivery");
    authority.fence_new_use();

    let released = CredentialRevocationReport::of(
        CredentialRevocationOutcome::Revoked,
        Some(CredentialLeaseState::Revoked),
    );
    assert_eq!(
        authority.observe_revocation(released, false),
        BindingLifecycleState::Draining
    );
    assert_eq!(authority.status(Some(released)).state(), BindingLifecycleState::Draining);
    assert_eq!(
        authority
            .mint(CredentialOperation::AcquireToken, &evidence())
            .expect_err("fenced new use cannot mint another delivery")
            .stage(),
        AdmissionStage::Revoke
    );
}

// ---------------------------------------------------------------------------
// Legs and renewal
// ---------------------------------------------------------------------------

/// A helper leg is an attenuation of the live session. Once a newer session
/// is minted, the leg that rode the earlier one is stale and cannot renew, and
/// an expired leg is refused even while the relationship is otherwise
/// current.
#[test]
fn a_stale_or_expired_leg_cannot_renew_an_expired_delivery() {
    let fixture = AdmissionFixture::new(&[OperationClass::AcquireToken], 0);
    let mut authority = fixture.admit();
    authority
        .mint(CredentialOperation::AcquireToken, &evidence())
        .expect("first delivery");
    let leg = authority
        .open_leg("web-helper", &evidence())
        .expect("bounded helper leg");

    // The current leg renews while the relationship holds, and the renewed
    // session becomes the admitted identity.
    let renewed = authority
        .renew(CredentialOperation::AcquireToken, &leg, &evidence())
        .expect("the live leg renews");
    assert_eq!(renewed.sequence(), 2);
    authority
        .verifies(&renewed.delivery_identity())
        .expect("the renewed session is the admitted one");

    // The renewed session advanced the sequence, so the original leg is stale.
    let refusal = authority
        .renew(CredentialOperation::AcquireToken, &leg, &evidence())
        .expect_err("a leg that rode the previous session is stale");
    assert_eq!(refusal.stage(), AdmissionStage::Activate);
    assert_eq!(refusal.reason(), RefusalReason::StaleAuthority);

    // A leg can only be opened over a live session, and it never outlives the
    // relationship's own expiry.
    let fresh = authority
        .open_leg("web-helper-2", &evidence())
        .expect("bounded helper leg over the current session");
    assert_eq!(fresh.expires_at_unix_ms(), authority.expiry_unix_ms());
    assert!(fresh.expires_at_unix_ms() >= authority.deadline_unix_ms());

    // Once the hard deadline has passed, renewal through the live leg is
    // refused: an expired delivery cannot be renewed.
    let after_deadline = CredentialDeliveryEvidence::new(
        generation(3),
        generation(7),
        generation(11),
        4,
        vec![dependency()],
        authority.deadline_unix_ms(),
    );
    assert_eq!(
        authority
            .renew(CredentialOperation::AcquireToken, &fresh, &after_deadline)
            .expect_err("an expired delivery cannot be renewed")
            .stage(),
        AdmissionStage::Drain
    );

    // A leg cannot be opened before any session exists.
    let cold = AdmissionFixture::new(&[OperationClass::AcquireToken], 0);
    let cold = cold.admit();
    assert_eq!(
        cold.open_leg("web-helper", &evidence())
            .expect_err("a leg needs a live session")
            .stage(),
        AdmissionStage::Activate
    );
}

// ---------------------------------------------------------------------------
// Identity: the relationship is named by its slot, not its payload
// ---------------------------------------------------------------------------

/// The KTD3 key is derived from Zone, source, consumer, kind, and slot, so
/// the admitted authority carries the same identity whatever the request's
/// mutable payload is, and a different slot is a different relationship.
#[test]
fn the_admitted_authority_is_keyed_by_the_stable_slot() {
    let fixture = AdmissionFixture::new(&[OperationClass::AcquireToken], 0);
    let authority = fixture.admit();
    let expected = BindingKey::new(
        zone(),
        BindingKind::Credential,
        resource(CREDENTIAL),
        uid(CREDENTIAL_UID),
        resource(CONSUMER),
        uid(CONSUMER_UID),
        BindingSlot::parse("api").expect("slot"),
    )
    .expect("key");
    assert_eq!(authority.key(), &expected);
    assert_eq!(authority.request().kind(), BindingKind::Credential);
    assert_eq!(authority.request().requested_rights(), RequestedRights::Consume);
    assert_eq!(
        authority.request().required_facets(),
        &[d2b_contracts_resource::v3::BindingRealizationFacet::CredentialDelivery]
    );

    let other_slot = CredentialBindingRequest::new(
        resource(CREDENTIAL),
        resource(CONSUMER),
        BindingSlot::parse("other").expect("slot"),
        d2b_contracts_resource::v3::execution_policy::BoundedToken::parse(AUDIENCE)
            .expect("audience token"),
        vec![CredentialOperation::AcquireToken],
        lifetime(),
    )
    .expect("request");
    assert!(other_slot.key(zone(), uid(CREDENTIAL_UID), uid(CONSUMER_UID)).is_ok());
    assert_ne!(
        other_slot
            .key(zone(), uid(CREDENTIAL_UID), uid(CONSUMER_UID))
            .expect("key"),
        expected
    );
}

/// The admitted delivery session is bound to the source's Provider, its own
/// credential generation, the consumer's component generation, and the row's
/// audience: the existing delivery protocol's identity, not a new one.
#[test]
fn the_minted_session_carries_the_existing_delivery_identity() {
    let fixture = AdmissionFixture::new(&[OperationClass::AcquireToken], 0);
    let mut authority = fixture.admit();
    let params = authority
        .mint(CredentialOperation::AcquireToken, &evidence())
        .expect("first delivery");

    assert_eq!(params.credential_ref(), &resource(CREDENTIAL));
    assert_eq!(params.credential_uid(), &uid(CREDENTIAL_UID));
    assert_eq!(params.credential_generation(), generation(3));
    assert_eq!(params.consumer_provider_ref(), &resource(CONSUMER_PROVIDER));
    assert_eq!(params.consumer_component_generation(), generation(7));
    assert_eq!(params.audience().as_str(), AUDIENCE);
    assert_eq!(params.operation_class(), OperationClass::AcquireToken);
    assert_eq!(params.route_digest(), &route_digest());
    assert_eq!(params.max_token_bytes(), 4096);
    assert_eq!(params.expiry_unix_ms(), authority.expiry_unix_ms());
    assert_eq!(params.deadline_unix_ms(), authority.deadline_unix_ms());
    assert_eq!(authority.audience().as_str(), AUDIENCE);
    assert_eq!(authority.current_sequence(), 1);
    assert_eq!(
        authority.method_for(CredentialOperation::AcquireToken),
        Some(CredentialMethod::AcquireToken)
    );
    assert_eq!(authority.method_for(CredentialOperation::RefreshToken), None);
}

/// The closed request contract refuses a `Host` consumer, an empty operation
/// set, and a lifetime whose validity exceeds its expiry, so a delivery
/// binding cannot be spelled into a shape the realization does not admit.
#[test]
fn the_closed_request_contract_refuses_out_of_shape_delivery_bindings() {
    let host_consumer = CredentialBindingRequest::new(
        resource(CREDENTIAL),
        resource("Host/studio"),
        BindingSlot::parse("api").expect("slot"),
        d2b_contracts_resource::v3::execution_policy::BoundedToken::parse(AUDIENCE)
            .expect("audience token"),
        vec![CredentialOperation::AcquireToken],
        lifetime(),
    );
    assert_eq!(
        host_consumer.expect_err("a Host is not a credential consumer"),
        BindingContractError::UnsupportedConsumerKind
    );

    assert!(matches!(
        CredentialBindingRequest::new(
            resource(CREDENTIAL),
            resource(CONSUMER),
            BindingSlot::parse("api").expect("slot"),
            d2b_contracts_resource::v3::execution_policy::BoundedToken::parse(AUDIENCE)
                .expect("audience token"),
            Vec::new(),
            lifetime(),
        )
        .expect_err("an empty operation set is refused"),
        BindingContractError::InvalidCollection
    ));

    assert!(CredentialLifetime::new("900s", "600s").is_err());
    assert!(BindingSlot::parse("Not A Slot").is_err());
}

/// The contract-level evidence a Provider-side session can observe.
fn observed_evidence() -> ObservedDeliveryEvidence {
    ObservedDeliveryEvidence::new(generation(3), generation(7), generation(11), NOW + 1_000)
}

/// The admitted relationship is the one authority every Credential Provider
/// realization reaches, so the shared gate admits the session this authority
/// minted and refuses everything the relationship has moved past (U23).
#[test]
fn the_family_authority_drives_the_shared_admission_gate() {
    let fixture =
        AdmissionFixture::new(&[OperationClass::AcquireToken, OperationClass::RefreshToken], 0);
    let mut authority = fixture.admit();
    let observed = observed_evidence();
    let first = authority
        .mint(CredentialOperation::AcquireToken, &evidence())
        .expect("first delivery");
    let authorization =
        CredentialAuthorization::new(CredentialMethod::AcquireToken, Some(first.clone()))
            .expect("authorization");

    assert_eq!(
        admit_credential_delivery(
            &authorization,
            CredentialMethod::AcquireToken,
            &authority,
            &observed,
        )
        .expect("the minted session is the current delivery"),
        first
    );

    // An operation the `Credential` row never granted is refused by the
    // source's own policy, before the session comparison runs. The refresh
    // session is well formed - it is the one a row that *does* grant refresh
    // mints - so only the narrower relationship can refuse it.
    let mut granting = AdmissionFixture::new(
        &[OperationClass::AcquireToken, OperationClass::RefreshToken],
        0,
    )
    .admit();
    let acquire_only = AdmissionFixture::new(&[OperationClass::AcquireToken], 0).admit();
    let ungranted = CredentialAuthorization::new(
        CredentialMethod::RefreshToken,
        Some(
            granting
                .mint(CredentialOperation::RefreshToken, &evidence())
                .expect("refresh delivery"),
        ),
    )
    .expect("authorization");
    assert_eq!(
        admit_credential_delivery(
            &ungranted,
            CredentialMethod::RefreshToken,
            &acquire_only,
            &observed,
        )
        .expect_err("an ungranted operation is refused")
        .reason(),
        d2b_contracts_resource::v3::RefusalReason::SourcePolicyRefused
    );

    // A replaced consumer component is refused by the relationship's own
    // fence, and the refusal names the stage that fence reached.
    let moved = ObservedDeliveryEvidence::new(generation(3), generation(8), generation(11), NOW + 1_000);
    assert_eq!(
        admit_credential_delivery(
            &authorization,
            CredentialMethod::AcquireToken,
            &authority,
            &moved,
        )
        .expect_err("a replaced consumer component is refused")
        .code(),
        "credential-delivery-stale-authority"
    );

    // A session the relationship has already superseded is refused at
    // activation, so new use needs a fresh admission.
    authority
        .mint(CredentialOperation::AcquireToken, &evidence())
        .expect("second delivery");
    assert_eq!(
        admit_credential_delivery(
            &authorization,
            CredentialMethod::AcquireToken,
            &authority,
            &observed,
        )
        .expect_err("a superseded session is refused")
        .code(),
        "credential-delivery-session-superseded"
    );
}

// ---------------------------------------------------------------------------
// The committed rows a `Credential` row derives
// ---------------------------------------------------------------------------

/// One committed delivery relationship derived from one committed row.
struct DerivedRow {
    name: String,
    row: CredentialBindingSpec,
}

/// Derive the one row a `Credential` row commits to, decoding it back through
/// the family's own row contract.
fn derived_row(spec: &CredentialSpec) -> DerivedRow {
    let derived = canonical_binding_rows(&resource(CREDENTIAL), spec)
        .expect("derivable Credential row")
        .into_iter()
        .map(|child| DerivedRow {
            name: child.name,
            row: serde_json::from_slice::<CredentialBindingSpec>(&child.spec)
                .expect("committed bytes decode through the row contract"),
        })
        .collect::<Vec<_>>();
    assert_eq!(derived.len(), 1, "a Guest-scoped row commits exactly one");
    derived.into_iter().next().expect("the committed row")
}

/// The delivery request a committed row declares, restated in the request
/// vocabulary its own admission path is reached through.
fn request_for(spec: &CredentialSpec, row: &CredentialBindingSpec) -> CredentialBindingRequest {
    let policy = CredentialSourcePolicy::from_spec(spec);
    let lifetime = format!("{}ms", row.lifetime_ms());
    CredentialBindingRequest::new(
        row.credential_ref().clone(),
        row.execution_ref().clone(),
        BindingSlot::parse(row.slot().as_str()).expect("slot"),
        d2b_contracts_resource::v3::execution_policy::BoundedToken::parse(
            policy.audience().as_str(),
        )
        .expect("audience token"),
        row.operations().to_vec(),
        CredentialLifetime::new(lifetime.clone(), lifetime).expect("lifetime"),
    )
    .expect("request")
}

#[test]
fn a_credential_row_commits_the_delivery_it_actually_declares() {
    let spec = credential_spec_scoped(
        Some(GUEST),
        &[
            OperationClass::AcquireToken,
            OperationClass::RefreshToken,
            OperationClass::SignChallenge,
            OperationClass::RevokeToken,
            OperationClass::InspectMetadata,
        ],
        600_000,
    );
    let derived = derived_row(&spec);

    assert_eq!(derived.row.credential_ref(), &resource(CREDENTIAL));
    assert_eq!(derived.row.execution_ref(), &resource(GUEST));
    assert_eq!(derived.row.slot().as_str(), CREDENTIAL_DELIVERY_SLOT);
    // Every delivery class the row grants, and neither protocol operation:
    // revocation and metadata inspection establish no delivery session.
    assert_eq!(
        derived.row.operations(),
        [
            CredentialOperation::AcquireToken,
            CredentialOperation::RefreshToken,
            CredentialOperation::SignChallenge,
        ],
    );
    assert_eq!(derived.row.lifetime_ms(), 600_000);
    assert!((MIN_CREDENTIAL_LIFETIME_MS..=MAX_CREDENTIAL_LIFETIME_MS)
        .contains(&derived.row.lifetime_ms()));

    let decision = derived.row.source();
    assert_eq!(decision.admitted_rights(), [RequestedRights::Consume]);
    assert_eq!(
        decision.realized_facets(),
        credential_binding_support().facets(),
    );

    // The committed bytes are the row contract, and the row contract admits
    // the consumer the source derived it for.
    admit_binding_row_refs(
        BindingKind::Credential,
        derived.row.credential_ref(),
        derived.row.execution_ref(),
    )
    .expect("the committed consumer is admitted by the row contract");

    // Nothing but what is delivered, to whom, for how long, and with which
    // operations: the audience, the Provider, the generations, and the route
    // digest stay out of the row.
    let rendered: serde_json::Value =
        serde_json::from_slice(&canonical_json_bytes(&derived.row).expect("bytes"))
            .expect("canonical row value");
    assert_eq!(
        object_keys(&rendered),
        [
            "credentialRef",
            "executionRef",
            "lifetimeMs",
            "operations",
            "slot",
            "source",
        ]
        .map(str::to_owned),
    );
    assert_eq!(
        object_keys(&rendered["source"]),
        ["admittedRights", "arbitration", "realizedFacets"].map(str::to_owned),
    );
    assert_no_secret_shaped_field(&rendered);
}

#[test]
fn one_relationship_keeps_one_row_name_as_the_row_narrows() {
    let wide = credential_spec_scoped(
        Some(GUEST),
        &[OperationClass::AcquireToken, OperationClass::RefreshToken],
        600_000,
    );
    let narrow = credential_spec_scoped(
        Some(GUEST),
        &[OperationClass::AcquireToken],
        600_000,
    );
    let shorter = credential_spec_scoped(
        Some(GUEST),
        &[OperationClass::AcquireToken, OperationClass::RefreshToken],
        60_000,
    );

    // Deriving twice from the same row changes nothing, so a restart re-ensures
    // the same row instead of churning its identity.
    let first = derived_row(&wide);
    let again = derived_row(&wide);
    assert_eq!(first.name, again.name);
    assert_eq!(first.row, again.row);

    // Narrowing the operations or the lease updates the one relationship
    // rather than minting a second row beside it: the name is minted from the
    // identities the row carries, not from its mutable payload.
    assert_eq!(derived_row(&narrow).name, first.name);
    assert_eq!(derived_row(&shorter).name, first.name);
    assert_ne!(derived_row(&narrow).row, first.row);
}

#[test]
fn the_derived_row_is_admitted_by_the_familys_own_admission_path() {
    let spec = credential_spec_scoped(
        Some(GUEST),
        &[OperationClass::AcquireToken, OperationClass::SignChallenge],
        600_000,
    );
    let derived = derived_row(&spec);
    let request = request_for(&spec, &derived.row);
    let decision = credential_source_decision();

    // The row's own key and the request's key are the same relationship: a
    // boundary evaluating the committed row reaches exactly the key the source
    // admitted.
    assert_eq!(
        derived
            .row
            .key(zone(), uid(CREDENTIAL_UID), uid(GUEST_UID))
            .expect("row key"),
        request
            .key(zone(), uid(CREDENTIAL_UID), uid(GUEST_UID))
            .expect("request key"),
    );

    let source = SourceAdmission::new(
        request
            .key(zone(), uid(CREDENTIAL_UID), uid(GUEST_UID))
            .expect("key"),
        decision.admitted_rights().to_vec(),
        decision.arbitration(),
    )
    .expect("source admission");
    let admitted = CredentialDeliveryAuthority::admit(
        request,
        CredentialBindingAdmission::new(
            zone(),
            uid(CREDENTIAL_UID),
            uid(GUEST_UID),
            CredentialSourcePolicy::from_spec(&spec),
            BindingSpecFingerprint::from_request(&spec),
            BindingAuthorization::granted(),
            source,
            credential_binding_support(),
            vec![dependency()],
            resource(CREDENTIAL),
            generation(3),
            resource(CONSUMER_PROVIDER),
            generation(7),
            generation(11),
            4,
            CredentialSourcePolicy::from_spec(&spec)
                .audience()
                .clone(),
            route_digest(),
            4096,
            NOW,
        ),
    )
    .expect("the derived row's relationship is admitted");

    assert_eq!(admitted.deadline_unix_ms(), NOW + 600_000);
    assert_eq!(admitted.expiry_unix_ms(), NOW + 600_000);
    for operation in derived.row.operations() {
        assert!(admitted.admits_operation(*operation));
    }
}

#[test]
fn a_credential_row_that_declares_no_relationship_commits_no_row() {
    // A host-level need is a child target-support ceiling, not a binding row.
    assert!(canonical_binding_rows(
        &resource(CREDENTIAL),
        &credential_spec_scoped(
            Some("Host/studio"),
            &[OperationClass::AcquireToken],
            600_000,
        ),
    )
    .expect("derivable row")
    .is_empty());
    // A row that places itself nowhere names no consumer.
    assert!(canonical_binding_rows(
        &resource(CREDENTIAL),
        &credential_spec_scoped(None, &[OperationClass::AcquireToken], 600_000),
    )
    .expect("derivable row")
    .is_empty());
    // A row granting only the two protocol operations establishes no delivery
    // session and therefore names no `CredentialBinding` relationship.
    assert!(canonical_binding_rows(
        &resource(CREDENTIAL),
        &credential_spec_scoped(
            Some(GUEST),
            &[OperationClass::RevokeToken, OperationClass::InspectMetadata],
            600_000,
        ),
    )
    .expect("derivable row")
    .is_empty());
}

#[test]
fn a_row_naming_an_unadmitted_consumer_is_refused() {
    // The kind admits every consumer kind except the Host, and the row
    // contract enforces it rather than leaving it to a call site.
    assert_eq!(
        admit_binding_row_refs(BindingKind::Credential, &resource(CREDENTIAL), &resource("Host/studio"))
            .expect_err("a Host is not a credential consumer"),
        BindingRowError::ConsumerNotAdmitted,
    );
    assert_eq!(
        CredentialBindingSpec::new(
            resource(CREDENTIAL),
            resource("Host/studio"),
            vec![CredentialOperation::AcquireToken],
            600_000,
            d2b_contracts_resource::v3::execution_policy::BoundedToken::parse(
                CREDENTIAL_DELIVERY_SLOT,
            )
            .expect("slot token"),
            credential_source_decision(),
        )
        .expect_err("a Host is not a credential consumer"),
        BindingRowError::ConsumerNotAdmitted,
    );
}

#[test]
fn a_delivery_outside_the_declared_facets_is_refused() {
    // The derived row commits exactly the facets the family declares.
    assert_eq!(
        credential_source_decision().realized_facets(),
        credential_binding_support().facets(),
    );

    // A realization that never claimed credential delivery refuses the
    // relationship at prepare rather than approximating the delivery.
    let spec = credential_spec_scoped(Some(GUEST), &[OperationClass::AcquireToken], 600_000);
    let derived = derived_row(&spec);
    let request = request_for(&spec, &derived.row);
    let refusal = CredentialDeliveryAuthority::admit(
        request,
        CredentialBindingAdmission::new(
            zone(),
            uid(CREDENTIAL_UID),
            uid(GUEST_UID),
            CredentialSourcePolicy::from_spec(&spec),
            BindingSpecFingerprint::from_request(&spec),
            BindingAuthorization::granted(),
            source_admission_for(&derived.row),
            BindingRealizationSupport::new(vec![BindingRealizationFacet::FilesystemPresentation])
                .expect("support set"),
            vec![dependency()],
            resource(CREDENTIAL),
            generation(3),
            resource(CONSUMER_PROVIDER),
            generation(7),
            generation(11),
            4,
            CredentialSourcePolicy::from_spec(&spec)
                .audience()
                .clone(),
            route_digest(),
            4096,
            NOW,
        ),
    )
    .expect_err("a delivery with no declared realization is refused");
    assert_eq!(refusal.stage(), AdmissionStage::Prepare);
    assert_eq!(refusal.reason(), RefusalReason::MandatoryFacetUnsupported);
}

#[test]
fn a_lease_ceiling_within_no_representable_lifetime_is_refused() {
    assert_eq!(
        canonical_binding_rows(
            &resource(CREDENTIAL),
            &credential_spec_scoped(Some(GUEST), &[OperationClass::AcquireToken], 500),
        )
        .expect_err("a ceiling below the contract's own floor commits no row"),
        BindingContractError::OutOfRange,
    );
}

/// The source admission a committed row's own decision implies.
fn source_admission_for(row: &CredentialBindingSpec) -> SourceAdmission {
    let decision = credential_source_decision();
    SourceAdmission::new(
        row.key(zone(), uid(CREDENTIAL_UID), uid(GUEST_UID))
            .expect("row key"),
        decision.admitted_rights().to_vec(),
        decision.arbitration(),
    )
    .expect("source admission")
}

// ---------------------------------------------------------------------------
// The `CredentialBinding` serving driver (U37)
// ---------------------------------------------------------------------------
//
// The driver is driven through the composition root's own construction: the
// factory builds its effects from the declared [`CredentialEffectFacets`], so a
// test supplying the recording runtime is exercising the production wiring
// rather than a substituted port. The runtime shares one ordered log with the
// recording manager endpoint, so the ordering teardown depends on - the
// revocation call, and the fence read back after a pre-drain - is observed as
// it happens.
//
// The properties under test are the ones the serving half has to earn:
//
// 1. A committed row reaches `validate` and `reconcile` through every fence:
//    the wire decode, the committed decision, the parent row behind its owner
//    fence, the parent's own policy, and the consumer row.
// 2. The committed `BindingSourceDecision` is enforced, not trusted.
// 3. An unmintable delivery is reported as a named refusal, never as one.
// 4. Teardown revokes through the preserved protocol call with the parent
//    row's own uid and the live session generation, and withholds cleanup when
//    the Provider cannot confirm.

/// The parent `Credential` row the binding declares.
///
/// The Provider its delivery is scoped to is read off the committed spec,
/// because that is the reference the revocation request binds.
fn parent_credential_bytes() -> Vec<u8> {
    serde_json::json!({
        "audience": AUDIENCE,
        "consumerRef": CONSUMER_PROVIDER,
        "allowedOperations": ["acquire-token"],
        "rotation": { "policy": "on-expiry", "proactiveWindowMs": null, "maxLeaseLifetimeMs": 0 },
        "expiry": { "hardDeadlineMs": 0 },
        "scope": { "executionRef": GUEST, "domainFilter": null, "userRef": null },
        "identityGuestRef": null,
        "loginEndpointRef": null
    })
    .to_string()
    .into_bytes()
}

/// The store-assigned 16-byte uid one `(zone, type, name)` key resolves to.
///
/// The store's durable identity is a digest of the key the row was declared
/// under, so the owner fence is exercised against the same derivation the
/// manager itself uses rather than a constant.
fn row_uid(type_name: &str, name: &str) -> [u8; 16] {
    deterministic_uid(&ResourceKey::new(ZONE, type_name, name))
}

/// The committed `CredentialBinding` row, carrying the decision the source's own
/// [`canonical_binding_rows`] commits.
fn binding_row(name: &str, source: serde_json::Value, lifetime_ms: u64) -> StoredDesiredResource {
    StoredDesiredResource {
        key: ResourceKey::new(ZONE, "CredentialBinding", name),
        uid: row_uid("CredentialBinding", name),
        generation: 1,
        owner_uid: Some(row_uid("Credential", "api-key")),
        provenance: ResourceProvenance::Resource,
        deleting: false,
        spec: serde_json::json!({
            "credentialRef": CREDENTIAL,
            "executionRef": GUEST,
            "operations": ["acquire-token"],
            "lifetimeMs": lifetime_ms,
            "slot": "delivery",
            "source": source,
        })
        .to_string()
        .into_bytes(),
        metadata: Vec::new(),
        created_at: 0,
    }
}

/// The decision the source's own derivation commits, read back through the
/// family rather than spelled a second time.
fn committed_decision() -> serde_json::Value {
    serde_json::to_value(credential_source_decision()).expect("canonical decision")
}

/// The row name the source's own derivation mints, so the driver's row-name
/// fence is exercised against the real derivation rather than a constant.
fn derived_row_name() -> String {
    let derived = canonical_binding_rows(&resource(CREDENTIAL), &scoped_credential_spec())
        .expect("source derivation")
        .remove(0);
    let row: CredentialBindingSpec =
        serde_json::from_slice(&derived.spec).expect("canonical binding bytes");
    credential_binding_row_name(&row)
        .expect("row name")
        .as_str()
        .to_owned()
}

/// A `Credential` row whose scope names a Guest, so the source derives exactly
/// one delivery row for it.
fn scoped_credential_spec() -> CredentialSpec {
    let base = credential_spec(&[OperationClass::AcquireToken], 0);
    CredentialSpec::new(
        CredentialScope::new(Some(resource(GUEST)), None, None).expect("scope"),
        base.audience().clone(),
        base.consumer_ref().cloned(),
        base.allowed_operations().to_vec(),
        *base.rotation(),
        ExpirySpec::new(0).expect("expiry"),
        RevocationSpec::default(),
        None,
        None,
    )
    .expect("credential spec")
}

/// The driver context plus the manager endpoint and requeue it records
/// through, so a test observes one ordering.
struct Fixture {
    ctx: ResourceContext,
    manager: RecordingManagerEndpoint,
    requeue: RecordingRequeue,
}

fn binding_fixture(row: StoredDesiredResource, manager: RecordingManagerEndpoint) -> Fixture {
    let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
    let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
    let requeue = RecordingRequeue::default();
    let ctx = ResourceContext::new(
        row,
        credential_binding_spec_decoder(),
        Arc::new(manager.clone()),
        Arc::new(requeue.clone()),
        effects_tx,
        notify_tx,
    );
    Fixture { ctx, manager, requeue }
}

/// One seeded manager row.
fn seeded_row(key: ResourceKey, spec: Vec<u8>) -> StoredDesiredResource {
    StoredDesiredResource {
        uid: deterministic_uid(&key),
        generation: 2,
        owner_uid: None,
        provenance: ResourceProvenance::Resource,
        deleting: false,
        spec,
        metadata: Vec::new(),
        created_at: 0,
        key,
    }
}

/// A manager holding the parent `Credential` row and the consumer row, so every
/// fence has something real to resolve.
fn serving_manager() -> RecordingManagerEndpoint {
    RecordingManagerEndpoint::new()
        .with_row(seeded_row(
            ResourceKey::new(ZONE, "Credential", "api-key"),
            parent_credential_bytes(),
        ))
        .with_row(seeded_row(
            ResourceKey::new(ZONE, "Guest", "work-vm"),
            b"{}".to_vec(),
        ))
}

/// A manager holding the parent `Credential` row with a committed spec that
/// grants only `SignChallenge`, so the parent policy has withdrawn the
/// operation the binding row still claims.
fn withdrawing_manager() -> RecordingManagerEndpoint {
    RecordingManagerEndpoint::new()
        .with_row(seeded_row(
            ResourceKey::new(ZONE, "Credential", "api-key"),
            serde_json::json!({
                "audience": AUDIENCE,
                "consumerRef": CONSUMER_PROVIDER,
                "allowedOperations": ["sign-challenge"],
                "rotation": { "policy": "on-expiry", "proactiveWindowMs": null, "maxLeaseLifetimeMs": 0 },
                "expiry": { "hardDeadlineMs": 0 },
                "scope": { "executionRef": GUEST, "domainFilter": null, "userRef": null },
                "identityGuestRef": null,
                "loginEndpointRef": null
            })
            .to_string()
            .into_bytes(),
        ))
        .with_row(seeded_row(
            ResourceKey::new(ZONE, "Guest", "work-vm"),
            b"{}".to_vec(),
        ))
}

/// The driver over the production facet construction.
async fn binding_driver(runtime: Arc<RecordingRuntime>) -> Box<dyn DynResourceDriver> {
    credential_binding_descriptor(CredentialBindingDriverArgs {
        zone: ZoneId::parse(ZONE).expect("zone"),
        controller_generation: ControllerGeneration::new(1).expect("controller generation"),
        facets: recording_facets(runtime),
    })
    .factory
    .create(&ResourceKey::new(ZONE, "CredentialBinding", "row"))
    .await
}

/// A committed row reaches validate and reconcile through every fence, and the
/// pass reports the delivery it cannot mint as the named refusal rather than a
/// delivery.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_committed_row_reaches_validate_and_reconcile() {
    let manager = serving_manager();
    let runtime = RecordingRuntime::new(log());
    let mut f = binding_fixture(
        binding_row(&derived_row_name(), committed_decision(), 60_000),
        manager.clone(),
    );
    let mut d = binding_driver(runtime).await;

    d.validate(&mut f.ctx).await.expect("validate");

    assert_eq!(
        d.reconcile(&mut f.ctx).await.expect("reconcile"),
        ReconcileOutcome::Satisfied,
        "the pass converged its own work; the delivery itself is the named refusal below",
    );
    assert_eq!(
        f.ctx.status::<CredentialBindingDriverStatus>(),
        Some(&CredentialBindingDriverStatus::Undelivered {
            reason: UndeliveredReason::MintPathUnroutable,
        }),
        "the mint path cannot be driven from a committed row, and the row says so by name",
    );
    // An undelivered relationship re-checks on the preserved cadence, because
    // the generations the source's fence compares reach this actor as no watch
    // delivery on the binding row.
    assert_eq!(f.requeue.scheduled().len(), 1);
    // Both dependency edges were registered: the Credential row and the
    // consumer row (R12/R17).
    let order = f.manager.call_order();
    assert!(
        order.iter().any(|entry| entry.contains("watch:Credential/api-key")),
        "{order:?}"
    );
    assert!(order.iter().any(|entry| entry.contains("watch:Guest/work-vm")), "{order:?}");
}

/// A row whose name is not the one the source's own derivation mints was not
/// admitted here, and is refused rather than served.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_row_name_the_source_did_not_derive_is_refused() {
    let manager = serving_manager();
    let mut f = binding_fixture(
        binding_row("cred-binding-not-derived", committed_decision(), 60_000),
        manager,
    );
    let mut d = binding_driver(RecordingRuntime::new(log())).await;

    let failure = d
        .validate(&mut f.ctx)
        .await
        .expect_err("a row the source did not derive is refused");
    assert_eq!(failure.kind(), FailureKinds::BINDING_SPEC_INVALID);
    assert_eq!(failure.class(), FailureClass::Terminal);
}

/// The committed `BindingSourceDecision` is enforced, not trusted: a row that
/// drops the consuming right, drops the delivery facet, commits an arbitration
/// this family never commits, or names a facet outside the family's declared
/// support is refused terminally.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_committed_decision_that_does_not_admit_the_row_is_refused() {
    let manager = serving_manager();
    let name = derived_row_name();
    let refused: Vec<(&str, serde_json::Value)> = vec![
        (
            "observe-only",
            serde_json::json!({
                "admittedRights": ["observe"],
                "arbitration": "shared",
                "realizedFacets": ["credential-delivery"],
            }),
        ),
        (
            "no-delivery-facet",
            serde_json::json!({
                "admittedRights": ["consume"],
                "arbitration": "shared",
                "realizedFacets": ["filesystem-presentation"],
            }),
        ),
        (
            "exclusive",
            serde_json::json!({
                "admittedRights": ["consume"],
                "arbitration": "exclusive",
                "realizedFacets": ["credential-delivery"],
            }),
        ),
        (
            "unsupported-facet",
            serde_json::json!({
                "admittedRights": ["consume"],
                "arbitration": "shared",
                "realizedFacets": ["credential-delivery", "namespace-interface"],
            }),
        ),
    ];

    for (label, decision) in refused {
        let mut f = binding_fixture(binding_row(&name, decision, 60_000), manager.clone());
        let mut d = binding_driver(RecordingRuntime::new(log())).await;
        let failure = d
            .validate(&mut f.ctx)
            .await
            .expect_err("a decision that does not admit the row is refused");
        assert_eq!(
            failure.kind(),
            FailureKinds::BINDING_SPEC_INVALID,
            "{label} must be refused terminally"
        );
        assert_eq!(failure.class(), FailureClass::Terminal, "{label} cannot converge by retrying");
    }
}

/// A parent row whose owner uid differs from this binding's owner is refused
/// terminally: the manager would silently re-parent.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_parent_whose_owner_differs_is_refused() {
    // The parent row's uid is the Guest's, so it is not the row the manager
    // reports as this binding's owner.
    let manager = RecordingManagerEndpoint::new().with_row(StoredDesiredResource {
        key: ResourceKey::new(ZONE, "Credential", "api-key"),
        uid: row_uid("Guest", "work-vm"),
        generation: 2,
        owner_uid: None,
        provenance: ResourceProvenance::Resource,
        deleting: false,
        spec: parent_credential_bytes(),
        metadata: Vec::new(),
        created_at: 0,
    });
    let mut f = binding_fixture(
        binding_row(&derived_row_name(), committed_decision(), 60_000),
        manager,
    );
    let mut d = binding_driver(RecordingRuntime::new(log())).await;

    let failure = d
        .validate(&mut f.ctx)
        .await
        .expect_err("a re-parenting row is refused");
    assert_eq!(failure.kind(), FailureKinds::BINDING_OWNER_MISMATCH);
    assert_eq!(failure.class(), FailureClass::Terminal);
}

/// A parent row that is not observable yet defers retryably rather than failing
/// the binding terminal (issue #511): the row may simply not be committed.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn an_unobservable_parent_defers_retryably() {
    let mut f = binding_fixture(
        binding_row(&derived_row_name(), committed_decision(), 60_000),
        RecordingManagerEndpoint::new(),
    );
    let mut d = binding_driver(RecordingRuntime::new(log())).await;

    let failure = d
        .validate(&mut f.ctx)
        .await
        .expect_err("an absent parent is not observable");
    assert_eq!(failure.kind(), FailureKinds::BINDING_PARENT_UNAVAILABLE);
    assert_eq!(
        failure.class(),
        FailureClass::Retryable,
        "an absent parent may yet be committed",
    );
}

/// A consumer row that is absent is the same deferral on the other fence: the
/// consumer may not be committed yet.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn an_absent_consumer_defers_retryably() {
    // The parent resolves; the consumer row does not exist yet.
    let manager = RecordingManagerEndpoint::new().with_row(seeded_row(
        ResourceKey::new(ZONE, "Credential", "api-key"),
        parent_credential_bytes(),
    ));
    let mut f = binding_fixture(
        binding_row(&derived_row_name(), committed_decision(), 60_000),
        manager,
    );
    let mut d = binding_driver(RecordingRuntime::new(log())).await;

    let failure = d
        .validate(&mut f.ctx)
        .await
        .expect_err("an absent consumer is not observable");
    assert_eq!(failure.kind(), FailureKinds::BINDING_PARENT_UNAVAILABLE);
    assert_eq!(failure.class(), FailureClass::Retryable);
}

/// The parent's own source policy is the second, independent half of the
/// admission: a row whose policy no longer grants `AcquireToken` is refused
/// even though the binding's committed decision still claims it.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_parent_policy_that_withdrew_the_operation_is_refused() {
    let mut f = binding_fixture(
        binding_row(&derived_row_name(), committed_decision(), 60_000),
        withdrawing_manager(),
    );
    let mut d = binding_driver(RecordingRuntime::new(log())).await;

    let failure = d
        .validate(&mut f.ctx)
        .await
        .expect_err("a withdrawn operation is refused");
    assert_eq!(failure.kind(), FailureKinds::BINDING_PLAN_DERIVATION_INVALID);
    assert_eq!(failure.class(), FailureClass::Terminal);
}

/// Teardown revokes through the preserved protocol call, binding the parent
/// row's own uid and the live session generation rather than inventing either.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn teardown_revokes_with_the_real_parent_identity() {
    let manager = serving_manager();
    let runtime = RecordingRuntime::new(log());
    let mut f = binding_fixture(
        binding_row(&derived_row_name(), committed_decision(), 60_000),
        manager,
    );
    // An `Active` lease is the only state that is revoked; `None` would skip.
    runtime.set_lease(Some(CredentialLeaseFacts {
        state: CredentialLeaseState::Active,
        rotation_generation: 3,
    }));
    let mut d = binding_driver(runtime.clone()).await;

    d.delete(&mut f.ctx).await.expect("delete");

    // The revocation ran at all: the session was reached twice (once for the
    // live generation, once for the call), and only an `Active` lease reaches
    // either.
    assert_eq!(
        runtime
            .call_order()
            .iter()
            .filter(|entry| entry.as_str() == "session")
            .count(),
        2,
        "the session generation is read and then the revocation is issued through it: {:?}",
        runtime.call_order()
    );
    assert!(
        runtime.call_order().iter().any(|entry| entry.as_str() == "dependency-facts"),
        "the revocation binds the Provider row's own generation: {:?}",
        runtime.call_order()
    );
    // The revoke reached the session, which binds the generation exactly as the
    // real `ComponentCredentialSession` does: a request carrying a different
    // generation answers `Uncertain`, so a confirmed delete proves the driver
    // read the live generation rather than defaulting one (R28).
}

/// An unconfirmed revoke withholds cleanup: the durable deleting mark stays and
/// the pass retries rather than reporting a release it cannot prove (R36).
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn an_unconfirmed_revoke_withholds_cleanup() {
    let manager = serving_manager();
    let runtime = RecordingRuntime::new(log());
    let mut f = binding_fixture(
        binding_row(&derived_row_name(), committed_decision(), 60_000),
        manager,
    );
    runtime.set_lease(Some(CredentialLeaseFacts {
        state: CredentialLeaseState::Active,
        rotation_generation: 3,
    }));
    // A Provider that cannot confirm answers `Uncertain` whatever generation
    // the request carries, exactly as the real `ComponentCredentialSession`
    // does when the route stops being live.
    runtime.set_session(Some(Arc::new(RecordingSession::uncertain(Some(7)))));
    let mut d = binding_driver(runtime).await;

    let failure = d
        .delete(&mut f.ctx)
        .await
        .expect_err("an unconfirmed revoke withholds cleanup");
    assert_eq!(failure.kind(), FailureKinds::BINDING_SERVING_EFFECT_FAILED);
    assert_eq!(failure.class(), FailureClass::Retryable, "a retry may confirm it");
}

/// No session surface at all fails the revocation closed rather than binding a
/// zero generation (R28).
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_missing_session_surface_fails_the_revoke_closed() {
    let manager = serving_manager();
    let runtime = RecordingRuntime::new(log());
    let mut f = binding_fixture(
        binding_row(&derived_row_name(), committed_decision(), 60_000),
        manager,
    );
    runtime.set_lease(Some(CredentialLeaseFacts {
        state: CredentialLeaseState::Active,
        rotation_generation: 3,
    }));
    runtime.set_session(None);
    let mut d = binding_driver(runtime).await;

    let failure = d
        .delete(&mut f.ctx)
        .await
        .expect_err("no session surface means no authenticated revoker");
    assert_eq!(
        failure.class(),
        FailureClass::Terminal,
        "the identity could never be bound",
    );
}

/// A relationship whose lease never stood has nothing to retire: the pass
/// converges without a protocol call, exactly as the `Credential` row's own
/// teardown does.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_relationship_with_no_lease_skips_the_revocation() {
    let manager = serving_manager();
    let runtime = RecordingRuntime::new(log());
    let mut f = binding_fixture(
        binding_row(&derived_row_name(), committed_decision(), 60_000),
        manager,
    );
    runtime.set_lease(None);
    let mut d = binding_driver(runtime.clone()).await;

    d.delete(&mut f.ctx).await.expect("delete converges without effects");
    assert!(
        !runtime.call_order().iter().any(|entry| entry.starts_with("revoke:")),
        "nothing to retire: {:?}",
        runtime.call_order()
    );
}

/// A pre-drain fences the relationship: the next reconcile pass reads the
/// fence back out of the in-memory status instead of reporting it open again.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn pre_drain_fences_and_the_next_pass_reads_the_fence_back() {
    let manager = serving_manager();
    let mut f = binding_fixture(
        binding_row(&derived_row_name(), committed_decision(), 60_000),
        manager,
    );
    let mut d = binding_driver(RecordingRuntime::new(log())).await;

    d.pre_drain(&mut f.ctx).await.expect("pre_drain");
    assert!(
        matches!(
            f.ctx.status::<CredentialBindingDriverStatus>(),
            Some(&CredentialBindingDriverStatus::Draining { .. })
        ),
        "pre-drain fences the relationship",
    );

    d.reconcile(&mut f.ctx).await.expect("reconcile after the fence");
    assert!(
        matches!(
            f.ctx.status::<CredentialBindingDriverStatus>(),
            Some(&CredentialBindingDriverStatus::Draining { .. })
        ),
        "the fence survives the next pass (R11: the status slot is the fence)",
    );
}

/// A restart adopts nothing: the mint path never persisted a session this
/// actor could find, so adopting a delivery it cannot prove is exactly the
/// failure R41 exists to prevent.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn recover_adopts_nothing_it_cannot_prove() {
    let manager = serving_manager();
    let mut f = binding_fixture(
        binding_row(&derived_row_name(), committed_decision(), 60_000),
        manager,
    );
    let mut d = binding_driver(RecordingRuntime::new(log())).await;

    assert_eq!(d.recover(&mut f.ctx).await.expect("recover"), RecoveryOutcome::Missing);
}
