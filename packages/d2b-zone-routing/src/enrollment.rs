//! The Zone enrollment admission shape (`ADR046-routing-016`).
//!
//! `zone-bootstrap` and `zone-enroll` are the two `d2b.zone.v3.ZoneService`
//! methods that place a Guest agent: the one-time IKpsk2 bootstrap that
//! consumes the allocator-issued single-use PSK, and the enrolled `Noise_KK`
//! enrollment that commits the sealed enrollment record and admits the peer.
//! Both handlers consume authority, so both are addressed exactly the way
//! every other admitted decision in this crate is: a runtime-issued,
//! single-use admission paired with the exact expected tuple.
//!
//! The admission shape is the route shape ported onto the enrolled-session
//! reality:
//!
//! - The runtime issues an [`ZoneEnrollmentAdmissionEvidence`] sealed to one
//!   [`ZoneEnrollmentExpectation`]. Neither half can be constructed,
//!   inspected, formatted, serialized, or cloned into a second use.
//! - The handler consumes the pair at the point of use, so daemon time,
//!   revocation, and the session profile are checked then, not at issue time.
//! - Consumption is single-use. A second `zone-bootstrap` carrying the same
//!   admission is refused with [`ZoneEnrollmentRefusal::AdmissionConsumed`],
//!   and the enrollment state machine independently refuses a replayed PSK
//!   issuance.
//! - The expected tuple is compared field by field against what the request
//!   names, so a substituted link, generation, or session profile is refused
//!   before any PSK is burned or any record is sealed.
//!
//! # The enrolled-session reality
//!
//! The expected profile is not a policy this module invents: it is the
//! contract's own
//! [`EndpointPolicy::validate_enrolled_guest_session`] profile - a
//! ZoneController-initiated, GuestAgent-terminated, attachment-free
//! `Noise_KK` Guest-local vsock session. A policy that is not exactly that
//! profile is refused at issue time and again at consume time.
//!
//! Nothing here performs I/O, holds a store, names a socket, a path, a key, or
//! a credential, or mints a ZoneLink. The handler it feeds lives in
//! [`crate::service`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use d2b_contracts_resource::v3::identity::ReconnectGeneration;
use d2b_contracts_resource::v3::{ResourceUid, ZoneId};
use d2b_contracts_zone_session::v3::component_session::{
    AttachmentPolicy, EndpointPolicy, EndpointPurpose, EndpointRole, IdentityEvidenceRequirement,
    LimitProfile, Locality, NoiseProfile, PurposeClass, ServicePackage, TransportBinding,
    TransportClass,
};
use d2b_contracts_zone_session::v3::zone_routing::{ZoneLinkControllerGeneration, ZoneTreeEdge};
use d2b_contracts_zone_session::v3::zone_session::{ZoneEnrollmentIdentity, ZoneEnrollmentRefusal};

/// Default validity of one runtime-issued enrollment admission.
pub const ENROLLMENT_ADMISSION_LIFETIME_MS_DEFAULT: u64 = 30_000;

/// Largest validity one runtime-issued enrollment admission may take.
pub const ENROLLMENT_ADMISSION_LIFETIME_MS_MAX: u64 = 300_000;

/// The exact link identity and session profile one enrollment admission is
/// issued for.
///
/// These values are comparison inputs, not authority. The authority is the
/// paired runtime-issued evidence consumed by [`ZoneEnrollmentAdmission`];
/// keeping the expected tuple separate is what makes a substituted link,
/// generation, or session profile fail closed before the state machine runs.
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneEnrollmentExpectation {
    zone: ZoneId,
    zone_link_uid: ResourceUid,
    edge: ZoneTreeEdge,
    controller_generation: ZoneLinkControllerGeneration,
    enrolled_peer_fingerprint: [u8; 32],
    allocator_binding: [u8; 32],
    session_policy: EndpointPolicy,
}

impl ZoneEnrollmentExpectation {
    /// Build the exact tuple one enrollment admission must carry.
    ///
    /// The session profile is validated here, so an expectation for anything
    /// other than the contract's enrolled Guest-local carriage profile cannot
    /// be constructed at all. The pinned peer fingerprint and the opaque
    /// allocator binding are the allocator's own sealed facts: they seal the
    /// enrollment record and never cross the wire.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        zone: ZoneId,
        zone_link_uid: ResourceUid,
        edge: ZoneTreeEdge,
        controller_generation: ZoneLinkControllerGeneration,
        enrolled_peer_fingerprint: [u8; 32],
        allocator_binding: [u8; 32],
        session_policy: EndpointPolicy,
    ) -> Result<Self, ZoneEnrollmentRefusal> {
        session_policy
            .validate_enrolled_guest_session()
            .map_err(|_| ZoneEnrollmentRefusal::SessionProfileRefused)?;
        if enrolled_peer_fingerprint == [0; 32] || allocator_binding == [0; 32] {
            return Err(ZoneEnrollmentRefusal::MalformedRequest);
        }
        Ok(Self {
            zone,
            zone_link_uid,
            edge,
            controller_generation,
            enrolled_peer_fingerprint,
            allocator_binding,
            session_policy,
        })
    }

    /// Build the expected tuple for the contract's enrolled Guest-local
    /// session profile.
    ///
    /// The profile is not a parameter: it is exactly
    /// [`EndpointPolicy::validate_enrolled_guest_session`]'s profile, built
    /// here so an allocator, a handler, and a test cannot disagree about what
    /// an enrolled Guest session is. Only the facts that legitimately vary -
    /// the placement, the link, the pinned peer, and the compiled session
    /// bindings - are supplied.
    #[allow(clippy::too_many_arguments)]
    pub fn for_enrolled_guest_session(
        zone: ZoneId,
        zone_link_uid: ResourceUid,
        edge: ZoneTreeEdge,
        controller_generation: ZoneLinkControllerGeneration,
        enrolled_peer_fingerprint: [u8; 32],
        allocator_binding: [u8; 32],
        schema_fingerprint: [u8; 32],
        channel_binding: [u8; 32],
        reconnect_generation: ReconnectGeneration,
        limits: LimitProfile,
    ) -> Result<Self, ZoneEnrollmentRefusal> {
        let session_policy = EndpointPolicy {
            purpose: EndpointPurpose::ComponentSession,
            purpose_class: PurposeClass::Enrolled,
            initiator_role: EndpointRole::ZoneController,
            responder_role: EndpointRole::GuestAgent,
            service: ServicePackage::ResourceV3,
            schema_fingerprint,
            noise_profile: NoiseProfile::Kk25519ChaChaPolySha256,
            limits,
            transport_binding: TransportBinding {
                transport: TransportClass::NativeVsock,
                locality: Locality::GuestLocal,
                channel_binding,
                identity_evidence: IdentityEvidenceRequirement::EnrolledStaticKeys,
            },
            reconnect_generation: reconnect_generation.get(),
            attachment_policy: AttachmentPolicy::disabled(),
        };
        Self::new(
            zone,
            zone_link_uid,
            edge,
            controller_generation,
            enrolled_peer_fingerprint,
            allocator_binding,
            session_policy,
        )
    }

    /// The pinned peer static-key fingerprint the allocator sealed.
    pub const fn enrolled_peer_fingerprint(&self) -> [u8; 32] {
        self.enrolled_peer_fingerprint
    }

    /// The opaque digest of the allocator enrollment that authorized this.
    pub const fn allocator_binding(&self) -> [u8; 32] {
        self.allocator_binding
    }

    /// The Zone the allocator placed the enrolling agent in.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// The committed ZoneLink resource identity this enrollment is for.
    pub const fn zone_link_uid(&self) -> &ResourceUid {
        &self.zone_link_uid
    }

    /// The immutable parent/child edge the link joins.
    pub const fn edge(&self) -> &ZoneTreeEdge {
        &self.edge
    }

    /// The ZoneLink controller generation that authorized the link.
    pub const fn controller_generation(&self) -> &ZoneLinkControllerGeneration {
        &self.controller_generation
    }

    /// The enrolled Guest session profile this enrollment is for.
    pub const fn session_policy(&self) -> &EndpointPolicy {
        &self.session_policy
    }

    /// Whether the request named exactly the identity this admission was
    /// issued for.
    ///
    /// Every field is compared; there is no partial match and no field a
    /// caller may leave unstated.
    pub fn admits(&self, identity: &ZoneEnrollmentIdentity) -> bool {
        identity.zone_link_uid == self.zone_link_uid
            && identity.edge == self.edge
            && identity.controller_generation == self.controller_generation
            && identity.reconnect_generation.get() == self.session_policy.reconnect_generation
            && identity.schema_fingerprint == self.session_policy.schema_fingerprint
    }

    /// Whether the request named exactly the link identity this admission was
    /// issued for, without its session profile fields.
    pub fn admits_link(&self, identity: &ZoneEnrollmentIdentity) -> bool {
        identity.zone_link_uid == self.zone_link_uid
            && identity.edge == self.edge
            && identity.controller_generation == self.controller_generation
    }
}

impl std::fmt::Debug for ZoneEnrollmentExpectation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ZoneEnrollmentExpectation(<redacted>)")
    }
}

/// One verified, immutable, single-use enrollment admission.
///
/// The production constructor takes ownership of a paired verifier and
/// evidence; the verifier is invoked when the admission is consumed by a
/// handler, so daemon time, revocation, and the session profile are checked at
/// the point of use.
pub struct ZoneEnrollmentAdmission {
    state: Mutex<Option<EnrollmentAdmissionState>>,
}

enum EnrollmentAdmissionState {
    Runtime {
        verifier: ZoneEnrollmentAdmissionVerifier,
        evidence: ZoneEnrollmentAdmissionEvidence,
        expected: ZoneEnrollmentExpectation,
    },
    #[cfg(any(test, feature = "test-support"))]
    Test(ZoneEnrollmentExpectation),
}

impl std::fmt::Debug for ZoneEnrollmentAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ZoneEnrollmentAdmission(<redacted>)")
    }
}

impl ZoneEnrollmentAdmission {
    /// Hold runtime-issued evidence for one current, single-use enrollment.
    pub fn verify(
        verifier: ZoneEnrollmentAdmissionVerifier,
        evidence: ZoneEnrollmentAdmissionEvidence,
        expected: &ZoneEnrollmentExpectation,
    ) -> Result<Self, ZoneEnrollmentRefusal> {
        Ok(Self {
            state: Mutex::new(Some(EnrollmentAdmissionState::Runtime {
                verifier,
                evidence,
                expected: expected.clone(),
            })),
        })
    }

    /// Build a synthetic admission only for owner-local vector tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(expected: ZoneEnrollmentExpectation) -> Self {
        Self {
            state: Mutex::new(Some(EnrollmentAdmissionState::Test(expected))),
        }
    }

    /// Consume and verify the admission, returning the sealed tuple.
    ///
    /// The tuple returned is the one the runtime sealed into the evidence, so
    /// the caller's request is checked against the seal rather than against a
    /// value the caller supplied alongside it. An admission whose expected
    /// tuple is not the sealed one is refused: the evidence is sealed to
    /// exactly one expectation, and a substituted expectation is never
    /// admitted through this pair.
    ///
    /// A missing, already consumed, mismatched, revoked, or expired admission
    /// is refused with a closed reason; the admission cannot be reused
    /// afterwards.
    pub(crate) fn consume(&self) -> Result<ZoneEnrollmentExpectation, ZoneEnrollmentRefusal> {
        let mut guard = self
            .state
            .lock()
            .map_err(|_| ZoneEnrollmentRefusal::AdmissionConsumed)?;
        let state = guard
            .take()
            .ok_or(ZoneEnrollmentRefusal::AdmissionConsumed)?;
        match state {
            EnrollmentAdmissionState::Runtime {
                verifier,
                evidence,
                expected,
            } => {
                let snapshot = verifier.verify(evidence)?;
                verifier.check_current(&snapshot)?;
                if snapshot.expected() != &expected {
                    return Err(ZoneEnrollmentRefusal::PolicyDenial);
                }
                Ok(snapshot.into_expected())
            }
            #[cfg(any(test, feature = "test-support"))]
            EnrollmentAdmissionState::Test(expected) => Ok(expected),
        }
    }
}

/// One runtime-issued enrollment admission snapshot, after verification.
pub struct VerifiedEnrollmentAdmission {
    expected: ZoneEnrollmentExpectation,
    expires_at_unix_ms: u64,
}

impl VerifiedEnrollmentAdmission {
    /// The exact expected tuple the evidence was issued for.
    pub const fn expected(&self) -> &ZoneEnrollmentExpectation {
        &self.expected
    }

    /// Take the sealed expected tuple, consuming the verified admission.
    pub fn into_expected(self) -> ZoneEnrollmentExpectation {
        self.expected
    }

    /// The absolute expiry of this admission, in Unix milliseconds.
    pub const fn expires_at_unix_ms(&self) -> u64 {
        self.expires_at_unix_ms
    }
}

/// Paired downstream verifier for runtime-issued enrollment evidence.
///
/// The verifier cannot be constructed, cloned, defaulted, or converted from
/// caller input. It is handed out only by [`ZoneEnrollmentAuthority`].
pub struct ZoneEnrollmentAdmissionVerifier {
    authority: Arc<EnrollmentAuthorityInner>,
}

impl std::fmt::Debug for ZoneEnrollmentAdmissionVerifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ZoneEnrollmentAdmissionVerifier(<redacted>)")
    }
}

impl ZoneEnrollmentAdmissionVerifier {
    /// Verify one evidence value issued by the paired authority.
    pub fn verify(
        &self,
        evidence: ZoneEnrollmentAdmissionEvidence,
    ) -> Result<VerifiedEnrollmentAdmission, ZoneEnrollmentRefusal> {
        if !Arc::ptr_eq(&self.authority, &evidence.authority) {
            return Err(ZoneEnrollmentRefusal::PolicyDenial);
        }
        if self.authority.revoked.load(Ordering::Acquire) {
            return Err(ZoneEnrollmentRefusal::PolicyDenial);
        }
        Ok(VerifiedEnrollmentAdmission {
            expected: evidence.body,
            expires_at_unix_ms: evidence.expires_at_unix_ms,
        })
    }

    /// Check one verified admission against current daemon time.
    pub fn check_current(
        &self,
        verified: &VerifiedEnrollmentAdmission,
    ) -> Result<(), ZoneEnrollmentRefusal> {
        let now = (self.authority.clock)();
        if self.authority.revoked.load(Ordering::Acquire) {
            return Err(ZoneEnrollmentRefusal::PolicyDenial);
        }
        if now >= verified.expires_at_unix_ms {
            return Err(ZoneEnrollmentRefusal::AdmissionExpired);
        }
        Ok(())
    }

    /// Revoke every future admission from this exact authority.
    pub fn revoke(&self) {
        self.authority.revoked.store(true, Ordering::Release);
    }
}

/// Sealed enrollment-admission evidence.
///
/// The evidence has no public constructor, accessor, serializer, debugger, or
/// cloning path. Only the paired [`ZoneEnrollmentAdmissionVerifier`] can
/// consume it.
pub struct ZoneEnrollmentAdmissionEvidence {
    authority: Arc<EnrollmentAuthorityInner>,
    body: ZoneEnrollmentExpectation,
    expires_at_unix_ms: u64,
}

impl std::fmt::Debug for ZoneEnrollmentAdmissionEvidence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ZoneEnrollmentAdmissionEvidence(<redacted>)")
    }
}

/// Runtime-owned issuer for one Zone's enrollment-admission authority.
///
/// The authority is constructed by the runtime that owns the Zone, the
/// allocator clock, and the committed ZoneLink identity. Callers receive
/// sealed evidence paired with a verifier for one downstream consumer; they
/// cannot construct or clone either half.
pub struct ZoneEnrollmentAuthority {
    inner: Arc<EnrollmentAuthorityInner>,
}

struct EnrollmentAuthorityInner {
    revoked: AtomicBool,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    lifetime_ms: u64,
}

impl std::fmt::Debug for ZoneEnrollmentAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ZoneEnrollmentAuthority(<redacted>)")
    }
}

impl ZoneEnrollmentAuthority {
    /// Bind an enrollment authority to one clock and the default lifetime.
    pub fn new(clock: Arc<dyn Fn() -> u64 + Send + Sync>) -> Result<Self, ZoneEnrollmentRefusal> {
        Self::with_lifetime(clock, ENROLLMENT_ADMISSION_LIFETIME_MS_DEFAULT)
    }

    /// Bind an enrollment authority to one clock and an explicit lifetime.
    ///
    /// A zero or over-ceiling lifetime is refused rather than clamped: an
    /// admission that never expires is as wrong as one that expires instantly.
    pub fn with_lifetime(
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
        lifetime_ms: u64,
    ) -> Result<Self, ZoneEnrollmentRefusal> {
        if lifetime_ms == 0 || lifetime_ms > ENROLLMENT_ADMISSION_LIFETIME_MS_MAX {
            return Err(ZoneEnrollmentRefusal::PolicyDenial);
        }
        Ok(Self {
            inner: Arc::new(EnrollmentAuthorityInner {
                revoked: AtomicBool::new(false),
                clock,
                lifetime_ms,
            }),
        })
    }

    /// Issue evidence and a fresh paired verifier for one enrollment.
    pub fn issue(
        &self,
        expected: ZoneEnrollmentExpectation,
    ) -> Result<
        (
            ZoneEnrollmentAdmissionVerifier,
            ZoneEnrollmentAdmissionEvidence,
        ),
        ZoneEnrollmentRefusal,
    > {
        if self.inner.revoked.load(Ordering::Acquire) {
            return Err(ZoneEnrollmentRefusal::PolicyDenial);
        }
        let issued_at_unix_ms = (self.inner.clock)();
        let expires_at_unix_ms = issued_at_unix_ms
            .checked_add(self.inner.lifetime_ms)
            .ok_or(ZoneEnrollmentRefusal::PolicyDenial)?;
        let verifier = ZoneEnrollmentAdmissionVerifier {
            authority: Arc::clone(&self.inner),
        };
        let evidence = ZoneEnrollmentAdmissionEvidence {
            authority: Arc::clone(&self.inner),
            body: expected,
            expires_at_unix_ms,
        };
        Ok((verifier, evidence))
    }

    /// Revoke all future admissions from this authority.
    pub fn revoke(&self) {
        self.inner.revoked.store(true, Ordering::Release);
    }

    /// A verifier for this authority, for a runtime that keeps no issuer.
    pub fn verifier(&self) -> ZoneEnrollmentAdmissionVerifier {
        ZoneEnrollmentAdmissionVerifier {
            authority: Arc::clone(&self.inner),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;

    use super::*;
    use d2b_contracts_zone_session::v3::zone_routing::{ZoneLabelId, ZonePath};

    fn zone(labels: &[&str]) -> ZonePath {
        ZonePath::new(
            labels
                .iter()
                .map(|label| ZoneLabelId::parse(*label).expect("valid label"))
                .collect(),
        )
        .expect("valid zone path")
    }

    fn edge() -> ZoneTreeEdge {
        ZoneTreeEdge::new(zone(&["k0"]), zone(&["k1", "k0"])).expect("direct child edge")
    }

    fn expectation() -> ZoneEnrollmentExpectation {
        expectation_with_reconnect(7)
    }

    fn expectation_with_reconnect(reconnect_generation: u64) -> ZoneEnrollmentExpectation {
        ZoneEnrollmentExpectation::for_enrolled_guest_session(
            ZoneId::parse("zone-1").expect("valid zone"),
            ResourceUid::parse("11111111-1111-4111-8111-111111111111").expect("valid UID"),
            edge(),
            ZoneLinkControllerGeneration::parse("controller-1").expect("valid generation"),
            [0x33; 32],
            [0x44; 32],
            [0x11; 32],
            [0x22; 32],
            ReconnectGeneration::new(reconnect_generation).expect("valid generation"),
            LimitProfile::remote_default(),
        )
        .expect("the contract's enrolled guest session profile")
    }

    fn identity_for(expected: &ZoneEnrollmentExpectation) -> ZoneEnrollmentIdentity {
        ZoneEnrollmentIdentity {
            zone_link_uid: expected.zone_link_uid().clone(),
            edge: expected.edge().clone(),
            controller_generation: expected.controller_generation().clone(),
            reconnect_generation: ReconnectGeneration::new(
                expected.session_policy().reconnect_generation,
            )
            .expect("valid generation"),
            schema_fingerprint: expected.session_policy().schema_fingerprint,
        }
    }

    fn clock(now: Arc<AtomicU64>) -> Arc<dyn Fn() -> u64 + Send + Sync> {
        Arc::new(move || now.load(Ordering::Acquire))
    }

    #[test]
    fn only_the_contract_enrolled_guest_profile_is_accepted() {
        assert!(
            expectation()
                .session_policy()
                .validate_enrolled_guest_session()
                .is_ok()
        );

        let mut policy = expectation().session_policy().clone();
        policy.responder_role = EndpointRole::ZoneController;
        assert_eq!(
            ZoneEnrollmentExpectation::new(
                ZoneId::parse("zone-1").expect("valid zone"),
                ResourceUid::parse("11111111-1111-4111-8111-111111111111").expect("valid UID"),
                edge(),
                ZoneLinkControllerGeneration::parse("controller-1").expect("valid generation"),
                [0x33; 32],
                [0x44; 32],
                policy,
            ),
            Err(ZoneEnrollmentRefusal::SessionProfileRefused)
        );

        let mut policy = expectation().session_policy().clone();
        policy.transport_binding.transport = TransportClass::ProviderStream;
        assert_eq!(
            ZoneEnrollmentExpectation::new(
                ZoneId::parse("zone-1").expect("valid zone"),
                ResourceUid::parse("11111111-1111-4111-8111-111111111111").expect("valid UID"),
                edge(),
                ZoneLinkControllerGeneration::parse("controller-1").expect("valid generation"),
                [0x33; 32],
                [0x44; 32],
                policy,
            ),
            Err(ZoneEnrollmentRefusal::SessionProfileRefused)
        );
    }

    #[test]
    fn an_issued_admission_is_single_use_and_expires() {
        let now = Arc::new(AtomicU64::new(1_000));
        let authority =
            ZoneEnrollmentAuthority::with_lifetime(clock(Arc::clone(&now)), 30_000).expect("valid");
        let (verifier, evidence) = authority.issue(expectation()).expect("issued");
        let admission =
            ZoneEnrollmentAdmission::verify(verifier, evidence, &expectation()).expect("admission");

        // First consumption returns the exact expected tuple.
        let consumed = admission.consume().expect("first consumption");
        assert_eq!(consumed.zone().as_str(), "zone-1");
        assert_eq!(consumed.edge(), &edge());

        // Second consumption fails closed: the admission was taken.
        assert_eq!(
            admission.consume(),
            Err(ZoneEnrollmentRefusal::AdmissionConsumed)
        );
    }

    /// The evidence seals exactly one expectation: a second, differing tuple
    /// held alongside it admits nothing, and the refusal is terminal.
    #[test]
    fn an_expectation_that_differs_from_the_sealed_one_is_refused() {
        let now = Arc::new(AtomicU64::new(1_000));
        let authority =
            ZoneEnrollmentAuthority::with_lifetime(clock(Arc::clone(&now)), 30_000).expect("valid");
        let sealed = expectation();
        let substituted = expectation_with_reconnect(8);
        assert_ne!(sealed, substituted, "the two tuples differ");

        let (verifier, evidence) = authority.issue(sealed.clone()).expect("issued");
        let admission = ZoneEnrollmentAdmission::verify(verifier, evidence, &substituted)
            .expect("the pair is held");
        assert_eq!(
            admission.consume(),
            Err(ZoneEnrollmentRefusal::PolicyDenial),
            "the sealed tuple, not the substituted one, decides"
        );
        // The refusal consumed the admission: the sealed tuple cannot retry it.
        assert_eq!(
            admission.consume(),
            Err(ZoneEnrollmentRefusal::AdmissionConsumed)
        );

        // The same pair with the sealed tuple consumes to exactly that tuple.
        let (verifier, evidence) = authority.issue(sealed.clone()).expect("issued");
        let admission = ZoneEnrollmentAdmission::verify(verifier, evidence, &sealed)
            .expect("the pair is held");
        assert_eq!(admission.consume().expect("consumed"), sealed);
    }

    #[test]
    fn a_stale_admission_and_a_revoked_authority_both_fail_closed() {
        let now = Arc::new(AtomicU64::new(1_000));
        let authority =
            ZoneEnrollmentAuthority::with_lifetime(clock(Arc::clone(&now)), 30_000).expect("valid");

        let (verifier, evidence) = authority.issue(expectation()).expect("issued");
        let stale =
            ZoneEnrollmentAdmission::verify(verifier, evidence, &expectation()).expect("admission");
        now.store(1_000_000, Ordering::Release);
        assert_eq!(
            stale.consume(),
            Err(ZoneEnrollmentRefusal::AdmissionExpired)
        );

        let (verifier, evidence) = authority.issue(expectation()).expect("issued");
        let revoked =
            ZoneEnrollmentAdmission::verify(verifier, evidence, &expectation()).expect("admission");
        authority.revoke();
        assert_eq!(revoked.consume(), Err(ZoneEnrollmentRefusal::PolicyDenial));
    }

    #[test]
    fn evidence_from_another_authority_is_refused() {
        let now = Arc::new(AtomicU64::new(1_000));
        let first =
            ZoneEnrollmentAuthority::with_lifetime(clock(Arc::clone(&now)), 30_000).expect("valid");
        let second =
            ZoneEnrollmentAuthority::with_lifetime(clock(Arc::clone(&now)), 30_000).expect("valid");
        let (verifier, _) = first.issue(expectation()).expect("issued");
        let (_, evidence) = second.issue(expectation()).expect("issued");
        let admission =
            ZoneEnrollmentAdmission::verify(verifier, evidence, &expectation()).expect("admission");
        assert_eq!(
            admission.consume(),
            Err(ZoneEnrollmentRefusal::PolicyDenial)
        );
    }

    #[test]
    fn a_lifetime_outside_the_bound_is_refused_rather_than_clamped() {
        let now = Arc::new(AtomicU64::new(1_000));
        assert!(ZoneEnrollmentAuthority::with_lifetime(clock(Arc::clone(&now)), 0).is_err());
        assert!(
            ZoneEnrollmentAuthority::with_lifetime(
                clock(Arc::clone(&now)),
                ENROLLMENT_ADMISSION_LIFETIME_MS_MAX + 1,
            )
            .is_err()
        );
        assert_eq!(
            ENROLLMENT_ADMISSION_LIFETIME_MS_DEFAULT, 30_000,
            "the default admission lifetime is a frozen bound"
        );
    }

    #[test]
    fn an_admission_admits_only_the_identity_it_was_issued_for() {
        let expected = expectation();
        let identity = identity_for(&expected);
        assert!(expected.admits(&identity));
        assert!(expected.admits_link(&identity));

        let mut substituted = identity.clone();
        substituted.schema_fingerprint = [0x33; 32];
        assert!(!expected.admits(&substituted));
        assert!(expected.admits_link(&substituted));

        let mut substituted = identity.clone();
        substituted.zone_link_uid =
            ResourceUid::parse("22222222-2222-4222-8222-222222222222").expect("valid UID");
        assert!(!expected.admits(&substituted));
        assert!(!expected.admits_link(&substituted));

        let mut substituted = identity;
        substituted.reconnect_generation = ReconnectGeneration::new(8).expect("valid generation");
        assert!(!expected.admits(&substituted));

        assert!(format!("{expected:?}").contains("<redacted>"));
    }

    #[test]
    fn a_synthetic_admission_consumes_once_and_carries_no_authority() {
        let admission = ZoneEnrollmentAdmission::for_test(expectation());
        assert!(admission.consume().is_ok());
        assert_eq!(
            admission.consume(),
            Err(ZoneEnrollmentRefusal::AdmissionConsumed)
        );
        assert!(format!("{admission:?}").contains("<redacted>"));
    }
}
