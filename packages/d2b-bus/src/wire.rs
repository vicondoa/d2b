//! v3 Zone bus wire constants and Zone-bound policy identity.
//!
//! The byte-level ComponentSession constants remain defined once in
//! `d2b-contracts`; this module is the bus-facing destination-compatible
//! import surface.  A Zone identity is prepended to policy fingerprints so a
//! policy cannot be replayed under a different local Zone name.

use d2b_contracts::v3::{
    ZoneId,
    component_session::{EndpointPolicyIdentity, LimitProfile},
    resource_schema::canonical_digest,
    zone_session,
};

pub use zone_session::{
    COMPONENT_SESSION_MAJOR, COMPONENT_SESSION_MINOR, FRAGMENT_HEADER_LEN,
    HANDSHAKE_OFFER_CANONICAL_LEN, LOCAL_HANDSHAKE_DEADLINE_MS, LOCAL_RECONNECT_DEADLINE_MS,
    MAX_ACTIVE_NAMED_STREAMS, MAX_AGGREGATE_NAMED_STREAM_QUEUE_BYTES, MAX_CLOCK_SKEW_MS,
    MAX_HANDSHAKE_OFFER_BYTES, MAX_HOST_ATTACHMENT_CREDITS, MAX_ID_BYTES,
    MAX_KEEPALIVE_INTERVAL_MS, MAX_KEEPALIVE_TIMEOUT_MS, MAX_LOGICAL_MESSAGE_BYTES,
    MAX_NAMED_STREAM_QUEUE_BYTES, MAX_OPERATION_ATTACHMENTS, MAX_PACKET_ATTACHMENTS,
    MAX_PROCESS_ATTACHMENT_CREDITS, MAX_PROTECTED_CIPHERTEXT_BYTES, MAX_PROTECTED_PLAINTEXT_BYTES,
    MAX_RECONNECT_ATTEMPTS, MAX_RECONNECT_WINDOW_MS, MAX_REQUEST_ATTACHMENTS,
    MAX_REQUEST_LIFETIME_MS, MAX_SESSION_ATTACHMENTS, MAX_SESSION_CONTROL_QUEUE_BYTES,
    MAX_TTRPC_CONTROL_QUEUE_BYTES, NOISE_TAG_BYTES, PREFACE_LEN, PREFACE_MAGIC, RECORD_HEADER_LEN,
    RECORD_LENGTH_BYTES, REMOTE_HANDSHAKE_DEADLINE_MS, REMOTE_RECONNECT_DEADLINE_MS,
    RESERVED_CONTROL_FDS,
};

/// The canonical v3 wire identity bound to one Zone.
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneBoundPolicyIdentity {
    zone: ZoneId,
    policy: EndpointPolicyIdentity,
}

impl ZoneBoundPolicyIdentity {
    /// Construct a Zone-bound identity.
    pub fn new(zone: ZoneId, policy: EndpointPolicyIdentity) -> Self {
        Self { zone, policy }
    }

    /// Borrow the Zone identity.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the component-session policy identity.
    pub const fn policy(&self) -> &EndpointPolicyIdentity {
        &self.policy
    }

    /// Render the stable digest used in local policy comparison.
    pub fn digest(&self) -> Result<String, d2b_contracts::v3::component_session::BinaryError> {
        let encoded = self.policy.encode_canonical()?;
        let mut bytes = Vec::with_capacity(self.zone.as_str().len() + encoded.len() + 1);
        bytes.extend_from_slice(self.zone.as_str().as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&encoded);
        Ok(canonical_digest("d2b:v3:zone-policy-identity", &bytes))
    }
}

impl std::fmt::Debug for ZoneBoundPolicyIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ZoneBoundPolicyIdentity(<redacted>)")
    }
}

/// Return the source-of-truth local capacity profile.
pub const fn local_default_limits() -> LimitProfile {
    LimitProfile::local_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::component_session::{
        AttachmentPolicy, AttachmentPolicyKind, EndpointPurpose, EndpointRole,
        IdentityEvidenceRequirement, Locality, NoiseProfile, PurposeClass, ServicePackage,
        TransportBinding, TransportClass,
    };

    fn identity() -> EndpointPolicyIdentity {
        EndpointPolicyIdentity {
            purpose: EndpointPurpose::ResourceService,
            purpose_class: PurposeClass::Local,
            initiator_role: EndpointRole::Component,
            responder_role: EndpointRole::ZoneController,
            service: ServicePackage::ResourceV3,
            schema_fingerprint: [1; 32],
            noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
            limits: LimitProfile::local_default(),
            transport_binding: TransportBinding {
                transport: TransportClass::UnixSeqpacket,
                locality: Locality::HostLocal,
                channel_binding: [2; 32],
                identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
            },
            attachment_policy: AttachmentPolicy {
                kind: AttachmentPolicyKind::Disabled,
                max_per_packet: 0,
                max_per_request: 0,
                max_per_operation: 0,
                max_per_session: 0,
                credentials_allowed: false,
            },
        }
    }

    #[test]
    fn copied_limits_and_magic_are_stable() {
        let limits = local_default_limits();
        assert_eq!(limits, LimitProfile::local_default());
        assert_eq!(PREFACE_MAGIC, *b"D2BCS3\r\n");
        assert_eq!(MAX_PROCESS_ATTACHMENT_CREDITS, 2_048);
        assert_eq!(MAX_HOST_ATTACHMENT_CREDITS, 8_192);
        assert_eq!(RESERVED_CONTROL_FDS, 64);
    }

    #[test]
    fn zone_encoding_changes_the_policy_identity() {
        let first = ZoneBoundPolicyIdentity::new(ZoneId::parse("dev").unwrap(), identity());
        let second = ZoneBoundPolicyIdentity::new(ZoneId::parse("prod").unwrap(), identity());
        assert_ne!(first.digest().unwrap(), second.digest().unwrap());
        assert_eq!(format!("{first:?}"), "ZoneBoundPolicyIdentity(<redacted>)");
    }
}
