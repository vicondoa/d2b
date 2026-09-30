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

use d2b_contracts_provider::v3::credential::{
    AudienceToken, CredentialAuthorization, CredentialDeliveryEvidence as ObservedDeliveryEvidence,
    CredentialLeaseState, CredentialMethod, CredentialScope, CredentialSpec, DeliveryRouteDigest,
    ExpirySpec, OperationClass, RevocationSpec, RotationPolicyClass, RotationSpec,
    admit_credential_delivery,
};
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingArbitration, BindingAuthorization, BindingContractError, BindingKey,
    BindingKind, BindingLifecycleState, BindingSlot, BindingSpecFingerprint, CredentialBindingRequest,
    CredentialLifetime, CredentialOperation, DesiredDigest, DesiredRevision, FreshnessTuple,
    RefusalReason, RequestedRights, ResourceGeneration, ResourceRef, ResourceUid, SourceAdmission,
    StoreIncarnation, ZoneId,
};
use d2b_provider_credential::{
    CredentialBindingAdmission, CredentialDeliveryAuthority, CredentialDeliveryEvidence,
    CredentialSourcePolicy, credential_binding_support,
};
use d2b_provider_credential::{CredentialRevocationOutcome, CredentialRevocationReport};

const ZONE: &str = "work";
const CREDENTIAL: &str = "Credential/api-key";
const CONSUMER: &str = "Process/web";
const CONSUMER_PROVIDER: &str = "Provider/secret-service";
const CREDENTIAL_UID: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
const CONSUMER_UID: &str = "9c5b94d1-6470-4b1a-9a41-0016d3cca427";
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

/// The three delivery operation classes, filtered out of a service operation
/// set: revocation and metadata inspection have no delivery session.
fn delivery_operation(class: OperationClass) -> Option<CredentialOperation> {
    match class {
        OperationClass::AcquireToken => Some(CredentialOperation::AcquireToken),
        OperationClass::RefreshToken => Some(CredentialOperation::RefreshToken),
        OperationClass::SignChallenge => Some(CredentialOperation::SignChallenge),
        _ => None,
    }
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
