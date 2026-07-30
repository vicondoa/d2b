//! Zone-typed endpoint policy over the extended v3 taxonomy.
//!
//! # The decision this module records
//!
//! `d2b_contracts::v3::zone_session` extends the endpoint taxonomy but
//! deliberately does not re-export `HandshakeOffer`, `EndpointPolicy`, or
//! `EndpointPolicyIdentity`, because those structs' enum-typed fields name the
//! *un-extended* component-session enumerations. Re-exporting them would have
//! handed this layer the old taxonomy under a new path.
//!
//! Two ways out existed. The first is to widen the field types in
//! `component_session.rs`. That was rejected: the offer's canonical encoding
//! is a frozen 148-byte wire contract with committed golden vectors, its
//! encoder is a change this work item does not own, and widening an enum field
//! in place is precisely the "renumber in passing" hazard the frozen-tag rule
//! exists to prevent.
//!
//! The second, taken here, is to define the policy over the Zone enumerations
//! locally and *lower* it into the component-session policy that the audited
//! handshake encoder consumes. Lowering is total for every preserved tag and
//! fail-closed for the appended Zone members, so a Zone-only purpose, role, or
//! service package cannot reach the wire mis-encoded - it cannot reach the
//! wire at all. That is a structural refusal rather than a policy one.
//!
//! # The resulting gap, stated plainly
//!
//! Because the canonical encoder still speaks the component-session tags, a
//! session whose offer names `EndpointPurpose::ZoneLocal`,
//! `EndpointPurpose::ZoneControl`, `EndpointRole::ZoneRelay`,
//! `EndpointRole::ZoneBootstrap`, `ServicePackage::ZoneV3`, or
//! `ServicePackage::ZoneLinkV3` cannot be offered yet.
//! [`ZonePolicyError::PurposeNotEncodable`] and its siblings name exactly
//! which member refused. Carrying those tags on the wire requires widening the
//! canonical offer encoding, which is an owned change to `component_session.rs`
//! and a wire decision of its own.
//!
//! This does not block the ZoneLink path this work item exists to serve: an
//! enrolled ZoneLink offers `ZoneLink`/`ZoneController`/`ResourceV3`, and the
//! one-time enrollment offers `Bootstrap`, all of which are preserved tags and
//! lower cleanly.
//!
//! # No authority
//!
//! A policy value is desired-state configuration. It carries no session, no
//! admission evidence, no resolved subject, no key material, and no path.

use d2b_contracts::v3::component_session as base;
use d2b_contracts::v3::zone_session::{
    AttachmentPolicy, EndpointPurpose, EndpointRole, LimitProfile, NoiseProfile, PurposeClass,
    ServicePackage,
};

// `TransportBinding` is not part of the extended taxonomy, so the Zone
// contract module does not re-export it. It is taken from its single
// definition rather than restated here.
use d2b_contracts::v3::component_session::TransportBinding;

/// A structural refusal raised while lowering a Zone policy for the wire.
///
/// Every variant names a member the frozen canonical offer encoding has no tag
/// for. None of them is a policy decision a caller could relax.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ZonePolicyError {
    /// The purpose is an appended Zone purpose with no component-session tag.
    PurposeNotEncodable,
    /// The initiator role is an appended Zone role with no counterpart.
    InitiatorRoleNotEncodable,
    /// The responder role is an appended Zone role with no counterpart.
    ResponderRoleNotEncodable,
    /// The service package is an appended Zone package with no counterpart.
    ServiceNotEncodable,
    /// The purpose may not be offered under the declared purpose class.
    PurposeClassRejected,
}

impl ZonePolicyError {
    /// The closed, path-free label for this refusal.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PurposeNotEncodable => "purpose-not-encodable",
            Self::InitiatorRoleNotEncodable => "initiator-role-not-encodable",
            Self::ResponderRoleNotEncodable => "responder-role-not-encodable",
            Self::ServiceNotEncodable => "service-not-encodable",
            Self::PurposeClassRejected => "purpose-class-rejected",
        }
    }
}

impl core::fmt::Display for ZonePolicyError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl core::error::Error for ZonePolicyError {}

/// One v3 Zone endpoint policy, typed with the extended taxonomy.
///
/// The field set is the component-session policy's field set unchanged; only
/// the three enumeration types differ. Keeping the shape identical is what
/// makes [`ZoneEndpointPolicy::lower`] a field copy rather than a translation
/// with rules of its own.
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneEndpointPolicy {
    /// The closed endpoint purpose.
    pub purpose: EndpointPurpose,
    /// The class the purpose is offered under.
    pub purpose_class: PurposeClass,
    /// The role the initiating endpoint plays.
    pub initiator_role: EndpointRole,
    /// The role the responding endpoint plays.
    pub responder_role: EndpointRole,
    /// The service package carried.
    pub service: ServicePackage,
    /// The service schema fingerprint bound into the offer.
    pub schema_fingerprint: [u8; 32],
    /// The Noise profile this endpoint uses.
    pub noise_profile: NoiseProfile,
    /// The negotiated limit profile.
    pub limits: LimitProfile,
    /// The transport binding.
    pub transport_binding: TransportBinding,
    /// The reconnect generation bound into protected record headers.
    pub reconnect_generation: u64,
    /// The attachment policy.
    pub attachment_policy: AttachmentPolicy,
}

impl core::fmt::Debug for ZoneEndpointPolicy {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Mirrors the component-session policy's whole-struct redaction. The
        // fingerprint, channel binding, and generation are transcript-binding
        // material and must never reach a log, span, or metric.
        formatter.write_str("ZoneEndpointPolicy(<redacted>)")
    }
}

impl ZoneEndpointPolicy {
    /// Lower this policy into the component-session policy the audited
    /// handshake encoder consumes.
    ///
    /// Fail-closed on every appended Zone member and on a purpose offered
    /// under a class it does not permit. The class rule is the contract's own
    /// [`EndpointPurpose::permits_class`]; this module restates none of it.
    pub fn lower(&self) -> Result<base::EndpointPolicy, ZonePolicyError> {
        if !self.purpose.permits_class(self.purpose_class) {
            return Err(ZonePolicyError::PurposeClassRejected);
        }
        Ok(base::EndpointPolicy {
            purpose: self
                .purpose
                .to_component_session()
                .ok_or(ZonePolicyError::PurposeNotEncodable)?,
            purpose_class: self.purpose_class,
            initiator_role: self
                .initiator_role
                .to_component_session()
                .ok_or(ZonePolicyError::InitiatorRoleNotEncodable)?,
            responder_role: self
                .responder_role
                .to_component_session()
                .ok_or(ZonePolicyError::ResponderRoleNotEncodable)?,
            service: self
                .service
                .to_component_session()
                .ok_or(ZonePolicyError::ServiceNotEncodable)?,
            schema_fingerprint: self.schema_fingerprint,
            noise_profile: self.noise_profile,
            limits: self.limits,
            transport_binding: self.transport_binding,
            reconnect_generation: self.reconnect_generation,
            attachment_policy: self.attachment_policy,
        })
    }

    /// Lift a component-session policy into the Zone taxonomy.
    ///
    /// Partial in exactly one place: the permanently reserved role tags 7 and
    /// 8 have no v3 spelling, so a policy naming one does not lift.
    pub fn lift(value: &base::EndpointPolicy) -> Option<Self> {
        Some(Self {
            purpose: EndpointPurpose::from_component_session(value.purpose),
            purpose_class: value.purpose_class,
            initiator_role: EndpointRole::from_component_session(value.initiator_role)?,
            responder_role: EndpointRole::from_component_session(value.responder_role)?,
            service: ServicePackage::from_component_session(value.service),
            schema_fingerprint: value.schema_fingerprint,
            noise_profile: value.noise_profile,
            limits: value.limits,
            transport_binding: value.transport_binding,
            reconnect_generation: value.reconnect_generation,
            attachment_policy: value.attachment_policy,
        })
    }

    /// The generation-independent identity of this policy.
    pub fn identity(&self) -> ZoneEndpointPolicyIdentity {
        ZoneEndpointPolicyIdentity {
            purpose: self.purpose,
            purpose_class: self.purpose_class,
            initiator_role: self.initiator_role,
            responder_role: self.responder_role,
            service: self.service,
            schema_fingerprint: self.schema_fingerprint,
            noise_profile: self.noise_profile,
            limits: self.limits,
            transport_binding: self.transport_binding,
            attachment_policy: self.attachment_policy,
        }
    }
}

/// The generation-independent part of a Zone endpoint policy.
///
/// This is what generation discovery sends before a reconnect generation is
/// known. It is not an authenticated session policy and authorizes nothing.
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneEndpointPolicyIdentity {
    /// The closed endpoint purpose.
    pub purpose: EndpointPurpose,
    /// The class the purpose is offered under.
    pub purpose_class: PurposeClass,
    /// The role the initiating endpoint plays.
    pub initiator_role: EndpointRole,
    /// The role the responding endpoint plays.
    pub responder_role: EndpointRole,
    /// The service package carried.
    pub service: ServicePackage,
    /// The service schema fingerprint.
    pub schema_fingerprint: [u8; 32],
    /// The Noise profile this endpoint uses.
    pub noise_profile: NoiseProfile,
    /// The negotiated limit profile.
    pub limits: LimitProfile,
    /// The transport binding.
    pub transport_binding: TransportBinding,
    /// The attachment policy.
    pub attachment_policy: AttachmentPolicy,
}

impl core::fmt::Debug for ZoneEndpointPolicyIdentity {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ZoneEndpointPolicyIdentity(<redacted>)")
    }
}

impl ZoneEndpointPolicyIdentity {
    /// Lower this identity into the component-session identity that the
    /// audited generation-discovery encoder consumes.
    pub fn lower(&self) -> Result<base::EndpointPolicyIdentity, ZonePolicyError> {
        if !self.purpose.permits_class(self.purpose_class) {
            return Err(ZonePolicyError::PurposeClassRejected);
        }
        Ok(base::EndpointPolicyIdentity {
            purpose: self
                .purpose
                .to_component_session()
                .ok_or(ZonePolicyError::PurposeNotEncodable)?,
            purpose_class: self.purpose_class,
            initiator_role: self
                .initiator_role
                .to_component_session()
                .ok_or(ZonePolicyError::InitiatorRoleNotEncodable)?,
            responder_role: self
                .responder_role
                .to_component_session()
                .ok_or(ZonePolicyError::ResponderRoleNotEncodable)?,
            service: self
                .service
                .to_component_session()
                .ok_or(ZonePolicyError::ServiceNotEncodable)?,
            schema_fingerprint: self.schema_fingerprint,
            noise_profile: self.noise_profile,
            limits: self.limits,
            transport_binding: self.transport_binding,
            attachment_policy: self.attachment_policy,
        })
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use d2b_contracts::v3::zone_session::{IdentityEvidenceRequirement, Locality, TransportClass};

    /// The enrolled ZoneLink policy: `Noise_KK` over adjacent-Zone carriage.
    ///
    /// A ZoneLink hop is attachment-free, which is why the attachment policy
    /// is the forbidding one rather than a bounded allowance.
    pub(crate) fn enrolled_zone_link(reconnect_generation: u64) -> ZoneEndpointPolicy {
        ZoneEndpointPolicy {
            purpose: EndpointPurpose::ZoneLink,
            purpose_class: PurposeClass::Enrolled,
            initiator_role: EndpointRole::ZoneController,
            responder_role: EndpointRole::ZoneController,
            service: ServicePackage::ResourceV3,
            schema_fingerprint: [0x11; 32],
            noise_profile: NoiseProfile::Kk25519ChaChaPolySha256,
            limits: LimitProfile::local_default(),
            transport_binding: TransportBinding {
                transport: TransportClass::ProviderStream,
                locality: Locality::Remote,
                channel_binding: [0x22; 32],
                identity_evidence: IdentityEvidenceRequirement::EnrolledStaticKeys,
            },
            reconnect_generation,
            attachment_policy: AttachmentPolicy::disabled(),
        }
    }

    /// The one-time ZoneLink enrollment bootstrap policy: IKpsk2.
    pub(crate) fn zone_link_bootstrap(reconnect_generation: u64) -> ZoneEndpointPolicy {
        ZoneEndpointPolicy {
            purpose: EndpointPurpose::Bootstrap,
            purpose_class: PurposeClass::Bootstrap,
            initiator_role: EndpointRole::ZoneController,
            responder_role: EndpointRole::ZoneController,
            service: ServicePackage::ResourceV3,
            schema_fingerprint: [0x11; 32],
            noise_profile: NoiseProfile::Ikpsk2_25519ChaChaPolySha256,
            limits: LimitProfile::local_default(),
            transport_binding: TransportBinding {
                transport: TransportClass::ProviderStream,
                locality: Locality::Remote,
                channel_binding: [0x22; 32],
                identity_evidence: IdentityEvidenceRequirement::ParentStaticAndSingleUsePsk,
            },
            reconnect_generation,
            attachment_policy: AttachmentPolicy::disabled(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_enrolled_zone_link_policy_lowers_for_the_wire() {
        let policy = fixtures::enrolled_zone_link(7);
        let lowered = policy.lower().expect("an enrolled ZoneLink policy lowers");
        assert_eq!(lowered.purpose, base::EndpointPurpose::ZoneLink);
        assert_eq!(lowered.initiator_role, base::EndpointRole::ZoneController);
        assert_eq!(lowered.service, base::ServicePackage::ResourceV3);
        assert_eq!(lowered.reconnect_generation, 7);
    }

    #[test]
    fn the_bootstrap_policy_lowers_for_the_wire() {
        let policy = fixtures::zone_link_bootstrap(1);
        let lowered = policy.lower().expect("a bootstrap policy lowers");
        assert_eq!(lowered.purpose, base::EndpointPurpose::Bootstrap);
        assert_eq!(lowered.purpose_class, PurposeClass::Bootstrap);
    }

    #[test]
    fn every_appended_zone_member_refuses_to_reach_the_wire() {
        let mut policy = fixtures::enrolled_zone_link(7);

        policy.purpose = EndpointPurpose::ZoneControl;
        assert_eq!(policy.lower(), Err(ZonePolicyError::PurposeNotEncodable));

        policy = fixtures::enrolled_zone_link(7);
        policy.initiator_role = EndpointRole::ZoneRelay;
        assert_eq!(
            policy.lower(),
            Err(ZonePolicyError::InitiatorRoleNotEncodable)
        );

        policy = fixtures::enrolled_zone_link(7);
        policy.responder_role = EndpointRole::ZoneBootstrap;
        assert_eq!(
            policy.lower(),
            Err(ZonePolicyError::ResponderRoleNotEncodable)
        );

        policy = fixtures::enrolled_zone_link(7);
        policy.service = ServicePackage::ZoneLinkV3;
        assert_eq!(policy.lower(), Err(ZonePolicyError::ServiceNotEncodable));
    }

    #[test]
    fn a_purpose_offered_under_a_class_it_does_not_permit_refuses() {
        let mut policy = fixtures::enrolled_zone_link(7);
        policy.purpose_class = PurposeClass::Bootstrap;
        assert_eq!(policy.lower(), Err(ZonePolicyError::PurposeClassRejected));

        let mut bootstrap = fixtures::zone_link_bootstrap(1);
        bootstrap.purpose_class = PurposeClass::Enrolled;
        assert_eq!(
            bootstrap.lower(),
            Err(ZonePolicyError::PurposeClassRejected)
        );
    }

    #[test]
    fn lowering_then_lifting_preserves_every_field() {
        let policy = fixtures::enrolled_zone_link(9);
        let lowered = policy.lower().expect("lower");
        let lifted = ZoneEndpointPolicy::lift(&lowered).expect("lift a preserved-tag policy");
        assert!(lifted == policy);
    }

    #[test]
    fn a_reserved_role_tag_does_not_lift() {
        let mut lowered = fixtures::enrolled_zone_link(9).lower().expect("lower");
        // Tags 7 and 8 are permanently reserved in v3. The component-session
        // taxonomy still names them, so a policy carrying one must not lift
        // into a v3 role by nearest match.
        lowered.responder_role = base::EndpointRole::from_tag(7).expect("a component-session tag");
        assert!(ZoneEndpointPolicy::lift(&lowered).is_none());
    }

    #[test]
    fn the_identity_projection_drops_only_the_generation() {
        let policy = fixtures::enrolled_zone_link(4);
        let identity = policy.identity();
        let lowered = identity.lower().expect("an identity lowers");
        assert_eq!(lowered.purpose, base::EndpointPurpose::ZoneLink);
        assert_eq!(lowered.schema_fingerprint, policy.schema_fingerprint);
    }

    #[test]
    fn debug_output_never_echoes_binding_material() {
        let policy = fixtures::enrolled_zone_link(7);
        assert_eq!(format!("{policy:?}"), "ZoneEndpointPolicy(<redacted>)");
        assert_eq!(
            format!("{:?}", policy.identity()),
            "ZoneEndpointPolicyIdentity(<redacted>)"
        );
    }

    #[test]
    fn every_refusal_renders_a_closed_path_free_label() {
        for error in [
            ZonePolicyError::PurposeNotEncodable,
            ZonePolicyError::InitiatorRoleNotEncodable,
            ZonePolicyError::ResponderRoleNotEncodable,
            ZonePolicyError::ServiceNotEncodable,
            ZonePolicyError::PurposeClassRejected,
        ] {
            let label = error.as_str();
            assert!(!label.is_empty());
            assert!(!label.contains('/'));
            assert_eq!(label, format!("{error}"));
        }
    }
}
