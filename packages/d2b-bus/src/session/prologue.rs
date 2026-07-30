//! Binding an authenticated inbound subject into the next-hop session
//! prologue.
//!
//! # What the relay slice needs, and what this provides
//!
//! When an intermediate Zone forwards a call one hop, the next-hop
//! ComponentSession must be bound to the *inbound* authenticated subject the
//! hop was authorized for. Without that binding a next-hop session, once
//! established, would be usable for any inbound subject, and the per-hop
//! authorization the relay handler performed would not be cryptographically
//! attached to the carriage it authorized.
//!
//! [`ZoneLinkPrologue`] provides that binding by folding a
//! [`SubjectContextDigest`] into the next-hop endpoint policy's transport
//! channel binding. The channel binding is already a field of the canonical
//! handshake offer, and the audited Noise prologue is `preface` followed by
//! the canonical offer bytes, so the digest lands inside the handshake
//! transcript with no change to the frozen wire encoding and no new field.
//! A peer that computed a different inbound subject derives a different
//! prologue and the handshake fails to authenticate.
//!
//! The choice of the channel-binding field is this module's, not a spec
//! quotation. The requirement stated to this layer was that the subject
//! context digest reach the session prologue; the specs read for this work
//! item name no field for it. Folding it into an existing 32-byte
//! transcript-bound field was preferred over adding an offer field, because
//! adding one is a change to a frozen canonical encoding owned elsewhere.
//!
//! # No authority is created here
//!
//! A subject context is *borrowed* and never stored, never constructed, and
//! never reconstructed from a digest. There is no constructor taking a subject
//! reference, uid, or claim: authoritative subject resolution stays exclusively
//! the registrar's. A digest is one-way, so a Zone that receives a bound offer
//! cannot expand it back into an identity.

use d2b_contracts::v3::{AuthenticatedSubjectContext, EvidenceClass, Locality};
use sha2::{Digest, Sha256};

use crate::session::contract::{ZoneEndpointPolicy, ZonePolicyError};

/// Domain separation for the subject-context digest.
///
/// A distinct domain string keeps this digest from ever colliding with the
/// resource-name digests, the principal digest, or the schema fingerprint,
/// even where those digest structurally similar input.
const SUBJECT_CONTEXT_DOMAIN: &[u8] = b"d2b-zone-link-subject-context-v3";

/// Domain separation for the channel-binding fold.
const CHANNEL_FOLD_DOMAIN: &[u8] = b"d2b-zone-link-channel-binding-v3";

/// A one-way digest of an already authenticated subject context.
///
/// The digest covers the authenticated identity and the authenticated session
/// shape: the subject reference, its Zone, the session purpose, the selected
/// service, the evidence class, and the transport locality. It deliberately
/// omits the subject uid and the transcript hash. The uid is the registrar's
/// resolution output rather than a carriage property, and the transcript hash
/// is the *inbound* hop's binding material, which must not be folded into an
/// outbound transcript.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SubjectContextDigest([u8; 32]);

impl SubjectContextDigest {
    /// Digest an already authenticated subject context.
    ///
    /// The context is borrowed for the duration of the call and is neither
    /// retained nor cloned.
    pub fn of_subject(context: &AuthenticatedSubjectContext) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(SUBJECT_CONTEXT_DOMAIN);
        for field in [
            context.subject_ref().to_canonical_string(),
            context.zone_ref().to_canonical_string(),
            context.session_purpose().as_str().to_owned(),
            context.service().as_str().to_owned(),
            evidence_class_label(context.evidence_class()).to_owned(),
            locality_label(context.transport_binding().locality()).to_owned(),
        ] {
            // Each field is length-prefixed so no two distinct field tuples
            // can produce one concatenation.
            hasher.update((field.len() as u64).to_be_bytes());
            hasher.update(field.as_bytes());
        }
        Self(hasher.finalize().into())
    }
}

impl core::fmt::Debug for SubjectContextDigest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("SubjectContextDigest(<redacted>)")
    }
}

const fn evidence_class_label(value: EvidenceClass) -> &'static str {
    match value {
        EvidenceClass::UnixPeer => "unix-peer",
        EvidenceClass::EnrolledKk => "enrolled-kk",
        EvidenceClass::BootstrapIkpsk2 => "bootstrap-ikpsk2",
        EvidenceClass::NativeVsock => "native-vsock",
    }
}

const fn locality_label(value: Locality) -> &'static str {
    match value {
        Locality::Local => "local",
        Locality::AdjacentZone => "adjacent-zone",
        Locality::Remote => "remote",
    }
}

/// A next-hop endpoint policy bound to one inbound authenticated subject.
///
/// The bound policy is produced by value and the binding is applied once. There
/// is no accessor that yields the unbound policy back, so a caller cannot open
/// the next hop with the binding stripped.
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneLinkPrologue {
    policy: ZoneEndpointPolicy,
}

impl ZoneLinkPrologue {
    /// Bind a next-hop policy to the inbound authenticated subject.
    ///
    /// Refuses a next-hop policy that does not lower for the wire, so a
    /// binding is never computed over a policy that could not have been
    /// offered in the first place.
    pub fn bind(
        mut policy: ZoneEndpointPolicy,
        subject: SubjectContextDigest,
    ) -> Result<Self, ZonePolicyError> {
        policy.lower()?;
        let mut hasher = Sha256::new();
        hasher.update(CHANNEL_FOLD_DOMAIN);
        hasher.update(policy.transport_binding.channel_binding);
        hasher.update(subject.0);
        policy.transport_binding.channel_binding = hasher.finalize().into();
        Ok(Self { policy })
    }

    /// Borrow the bound next-hop policy.
    pub const fn policy(&self) -> &ZoneEndpointPolicy {
        &self.policy
    }

    /// Whether this prologue was bound to the given inbound subject.
    ///
    /// The comparison recomputes the fold from the *unbound* channel binding
    /// the caller supplies, so it proves the binding rather than trusting it.
    pub fn is_bound_to(
        &self,
        unbound_channel_binding: [u8; 32],
        subject: SubjectContextDigest,
    ) -> bool {
        let mut hasher = Sha256::new();
        hasher.update(CHANNEL_FOLD_DOMAIN);
        hasher.update(unbound_channel_binding);
        hasher.update(subject.0);
        let expected: [u8; 32] = hasher.finalize().into();
        self.policy.transport_binding.channel_binding == expected
    }
}

impl core::fmt::Debug for ZoneLinkPrologue {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ZoneLinkPrologue(<redacted>)")
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use d2b_contracts::v3::{
        AuthenticatedSubjectContext, BindingDigest, EvidenceClass, Locality, ReconnectGeneration,
        ResourceRef, ResourceUid, SchemaFingerprint, ServiceName, SessionBinding, SessionPurpose,
        TranscriptHash, TransportBinding,
    };

    /// One authenticated subject context, as the registrar would have
    /// produced it. The test builds it directly because no registrar is wired
    /// in a hermetic unit test; nothing under test constructs one.
    pub(crate) fn subject(name: &str, purpose: &str) -> AuthenticatedSubjectContext {
        let binding = SessionBinding::new(
            SchemaFingerprint::parse(
                "sha256:0000000000000000000000000000000000000000000000000000000000000001",
            )
            .expect("a schema fingerprint"),
            TransportBinding::new(
                Locality::AdjacentZone,
                BindingDigest::parse(
                    "sha256:0000000000000000000000000000000000000000000000000000000000000002",
                )
                .expect("a binding digest"),
            ),
            ReconnectGeneration::new(1).expect("a nonzero generation"),
            TranscriptHash::parse_hex(
                "0000000000000000000000000000000000000000000000000000000000000003",
            )
            .expect("a transcript hash"),
        );
        AuthenticatedSubjectContext::new(
            ResourceRef::parse(&format!("Provider/{name}")).expect("a subject reference"),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("a subject uid"),
            ResourceRef::parse("Zone/child").expect("a Zone reference"),
            EvidenceClass::EnrolledKk,
            SessionPurpose::parse(purpose).expect("a bounded purpose token"),
            ServiceName::parse("d2b.resource.v3").expect("a service name"),
            binding,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::contract::fixtures::enrolled_zone_link;

    #[test]
    fn the_digest_is_stable_for_one_subject() {
        let context = fixtures::subject("relay-a", "zone-link");
        assert_eq!(
            SubjectContextDigest::of_subject(&context),
            SubjectContextDigest::of_subject(&context)
        );
    }

    #[test]
    fn distinct_subjects_and_distinct_purposes_digest_differently() {
        let first = SubjectContextDigest::of_subject(&fixtures::subject("relay-a", "zone-link"));
        let second = SubjectContextDigest::of_subject(&fixtures::subject("relay-b", "zone-link"));
        let third = SubjectContextDigest::of_subject(&fixtures::subject("relay-a", "zone-control"));
        assert_ne!(first, second);
        assert_ne!(first, third);
        assert_ne!(second, third);
    }

    #[test]
    fn binding_changes_the_channel_binding_that_the_prologue_covers() {
        let policy = enrolled_zone_link(7);
        let unbound = policy.transport_binding.channel_binding;
        let subject = SubjectContextDigest::of_subject(&fixtures::subject("relay-a", "zone-link"));
        let prologue = ZoneLinkPrologue::bind(policy, subject).expect("bind");
        assert_ne!(prologue.policy().transport_binding.channel_binding, unbound);
        assert!(prologue.is_bound_to(unbound, subject));
    }

    #[test]
    fn a_prologue_bound_to_one_subject_does_not_verify_against_another() {
        let policy = enrolled_zone_link(7);
        let unbound = policy.transport_binding.channel_binding;
        let authorized =
            SubjectContextDigest::of_subject(&fixtures::subject("relay-a", "zone-link"));
        let other = SubjectContextDigest::of_subject(&fixtures::subject("relay-b", "zone-link"));
        let prologue = ZoneLinkPrologue::bind(policy, authorized).expect("bind");
        assert!(!prologue.is_bound_to(unbound, other));
    }

    #[test]
    fn the_bound_policy_still_lowers_for_the_wire() {
        let subject = SubjectContextDigest::of_subject(&fixtures::subject("relay-a", "zone-link"));
        let prologue = ZoneLinkPrologue::bind(enrolled_zone_link(7), subject).expect("bind");
        prologue
            .policy()
            .lower()
            .expect("a bound ZoneLink policy still lowers");
    }

    #[test]
    fn a_policy_that_cannot_reach_the_wire_is_never_bound() {
        let mut policy = enrolled_zone_link(7);
        policy.purpose = d2b_contracts::v3::zone_session::EndpointPurpose::ZoneControl;
        let subject = SubjectContextDigest::of_subject(&fixtures::subject("relay-a", "zone-link"));
        assert_eq!(
            ZoneLinkPrologue::bind(policy, subject).err(),
            Some(ZonePolicyError::PurposeNotEncodable)
        );
    }

    #[test]
    fn debug_output_never_echoes_the_digest_or_the_policy() {
        let subject = SubjectContextDigest::of_subject(&fixtures::subject("relay-a", "zone-link"));
        assert_eq!(format!("{subject:?}"), "SubjectContextDigest(<redacted>)");
        let prologue = ZoneLinkPrologue::bind(enrolled_zone_link(7), subject).expect("bind");
        assert_eq!(format!("{prologue:?}"), "ZoneLinkPrologue(<redacted>)");
    }
}
