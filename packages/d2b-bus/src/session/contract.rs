//! Zone-typed endpoint policy over the extended v3 taxonomy.
//!
//! # The decision this module records
//!
//! `d2b_contracts_zone_session::v3::zone_session` extends the endpoint taxonomy but
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
//! # Authority boundary
//!
//! A policy value is desired-state configuration. It carries no session, no
//! admission evidence, no resolved subject, no key material, and no path.
//! The route-admission types below are the deliberate exception: their issuer
//! and paired verifier are runtime-owned, while the evidence itself is
//! non-cloneable and sealed to that verifier.

use sha2::{Digest, Sha256};
use std::fmt;
use std::sync::{Arc, Mutex};

use d2b_contracts_resource::v3::identity::{
    BindingDigest, EvidenceClass, Locality as IdentityLocality, ReconnectGeneration,
    TransportBinding as IdentityTransportBinding,
};
use d2b_contracts_resource::v3::resource_schema::canonical_json_bytes;
use d2b_contracts_resource::v3::{ResourceUid, SchemaFingerprint, ZoneRevision};
use d2b_contracts_zone_session::v3::component_session as base;
pub use d2b_contracts_zone_session::v3::zone_routing::ZoneLinkRouteAdmissionRequest;
use d2b_contracts_zone_session::v3::zone_routing::{
    ZoneLinkControllerGeneration, ZoneRouteCapability, ZoneTreeEdge,
};
use d2b_contracts_zone_session::v3::zone_session::{
    AttachmentPolicy, EndpointPurpose, EndpointRole, LimitProfile, NoiseProfile, PurposeClass,
    ServicePackage,
};

// `TransportBinding` is not part of the extended taxonomy, so the Zone
// contract module does not re-export it. It is taken from its single
// definition rather than restated here.
use d2b_contracts_zone_session::v3::component_session::TransportBinding;

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
    /// The complete endpoint policy is not the enrolled ZoneLink profile.
    ZoneLinkNotAdmissible,
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
            Self::ZoneLinkNotAdmissible => "zonelink-not-admissible",
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

/// Immutable authenticated ComponentSession profile bound to one ZoneLink
/// route admission.
///
/// The fields are private and the only production constructor requires both a
/// validated endpoint policy and an already authenticated session route
/// binding. It is therefore metadata obtained at the trusted session boundary,
/// not caller-supplied claims.
///
/// ```compile_fail
/// use d2b_bus::session::contract::RouteAdmissionSessionBinding;
///
/// fn forge(binding: RouteAdmissionSessionBinding) {
///     let _ = RouteAdmissionSessionBinding { ..binding };
/// }
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct RouteAdmissionSessionBinding {
    purpose: base::EndpointPurpose,
    purpose_class: base::PurposeClass,
    initiator_role: base::EndpointRole,
    responder_role: base::EndpointRole,
    endpoint_locality: base::Locality,
    service: base::ServicePackage,
    reconnect_generation: ReconnectGeneration,
    schema_fingerprint: SchemaFingerprint,
    transport_class: base::TransportClass,
    transport_binding: IdentityTransportBinding,
    liveness: Option<d2b_session::SessionLiveness>,
}

fn hex(bytes: &[u8]) -> String {
    let mut rendered = String::with_capacity(bytes.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        rendered.push(char::from(HEX[usize::from(byte >> 4)]));
        rendered.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    rendered
}

fn policy_schema_fingerprint(
    policy: &base::EndpointPolicy,
) -> Result<SchemaFingerprint, ZonePolicyError> {
    SchemaFingerprint::parse(format!("sha256:{}", hex(&policy.schema_fingerprint)))
        .map_err(|_| ZonePolicyError::ZoneLinkNotAdmissible)
}

fn policy_transport_binding(
    policy: &base::EndpointPolicy,
) -> Result<IdentityTransportBinding, ZonePolicyError> {
    let locality = match policy.transport_binding.locality {
        base::Locality::GuestLocal => IdentityLocality::Local,
        base::Locality::Remote => IdentityLocality::AdjacentZone,
        _ => return Err(ZonePolicyError::ZoneLinkNotAdmissible),
    };
    let digest = BindingDigest::parse(format!(
        "sha256:{}",
        hex(&policy.transport_binding.channel_binding)
    ))
    .map_err(|_| ZonePolicyError::ZoneLinkNotAdmissible)?;
    Ok(IdentityTransportBinding::new(locality, digest))
}

impl RouteAdmissionSessionBinding {
    #[cfg(test)]
    fn from_policy(policy: &base::EndpointPolicy) -> Result<Self, ZonePolicyError> {
        policy
            .validate_zone_link()
            .map_err(|_| ZonePolicyError::ZoneLinkNotAdmissible)?;
        let schema_fingerprint = policy_schema_fingerprint(policy)?;
        let transport_binding = policy_transport_binding(policy)?;
        Ok(Self {
            purpose: policy.purpose,
            purpose_class: policy.purpose_class,
            initiator_role: policy.initiator_role,
            responder_role: policy.responder_role,
            endpoint_locality: policy.transport_binding.locality,
            service: policy.service,
            reconnect_generation: ReconnectGeneration::new(policy.reconnect_generation)
                .map_err(|_| ZonePolicyError::ZoneLinkNotAdmissible)?,
            schema_fingerprint,
            transport_class: policy.transport_binding.transport,
            transport_binding,
            liveness: None,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn from_authenticated_session(
        policy: &base::EndpointPolicy,
        session: &d2b_session::AuthenticatedSessionRouteBinding,
    ) -> Result<Self, ZonePolicyError> {
        policy
            .validate_zone_link()
            .map_err(|_| ZonePolicyError::ZoneLinkNotAdmissible)?;
        let context = session.context();
        let expected_locality = match policy.transport_binding.locality {
            base::Locality::GuestLocal => IdentityLocality::Local,
            base::Locality::Remote => IdentityLocality::AdjacentZone,
            _ => return Err(ZonePolicyError::ZoneLinkNotAdmissible),
        };
        let expected_schema = policy_schema_fingerprint(policy)?;
        let expected_transport = policy_transport_binding(policy)?;
        if context.session_purpose().as_str() != policy.purpose.as_str()
            || session.purpose_class() != policy.purpose_class
            || session.initiator_role() != policy.initiator_role
            || session.responder_role() != policy.responder_role
            || session.endpoint_locality() != policy.transport_binding.locality
            || session.schema() != &expected_schema
            || session.transport_class() != policy.transport_binding.transport
            || session.transport_binding().locality() != expected_locality
            || session.transport_binding().binding_digest() != expected_transport.binding_digest()
            || session.service().as_str() != policy.service.as_str()
            || session.reconnect_generation().get() != policy.reconnect_generation
            || session.locality() != expected_locality
        {
            return Err(ZonePolicyError::ZoneLinkNotAdmissible);
        }
        Ok(Self {
            purpose: policy.purpose,
            purpose_class: policy.purpose_class,
            initiator_role: policy.initiator_role,
            responder_role: policy.responder_role,
            endpoint_locality: session.endpoint_locality(),
            service: policy.service,
            reconnect_generation: session.reconnect_generation(),
            schema_fingerprint: session.schema().clone(),
            transport_class: session.transport_class(),
            transport_binding: session.transport_binding().clone(),
            liveness: Some(session.liveness()),
        })
    }

    pub const fn purpose(&self) -> base::EndpointPurpose {
        self.purpose
    }

    pub const fn purpose_class(&self) -> base::PurposeClass {
        self.purpose_class
    }

    pub const fn initiator_role(&self) -> base::EndpointRole {
        self.initiator_role
    }

    pub const fn responder_role(&self) -> base::EndpointRole {
        self.responder_role
    }

    pub const fn endpoint_locality(&self) -> base::Locality {
        self.endpoint_locality
    }

    pub const fn service(&self) -> base::ServicePackage {
        self.service
    }

    pub const fn reconnect_generation(&self) -> ReconnectGeneration {
        self.reconnect_generation
    }

    pub const fn schema_fingerprint(&self) -> &SchemaFingerprint {
        &self.schema_fingerprint
    }

    pub const fn transport_class(&self) -> base::TransportClass {
        self.transport_class
    }

    pub const fn transport_binding(&self) -> &IdentityTransportBinding {
        &self.transport_binding
    }

    /// Verify that this sealed route profile belongs to one exact
    /// authenticated ComponentSession.
    ///
    /// The liveness marker is compared by identity, not merely by value, so a
    /// second session with the same public profile cannot substitute for the
    /// owner that established the route admission.
    pub(crate) fn matches_authenticated_session(
        &self,
        session: &d2b_session::AuthenticatedSessionRouteBinding,
    ) -> Result<(), ZonePolicyError> {
        let session_liveness = session.liveness();
        let context = session.context();
        if self.liveness.as_ref() != Some(&session_liveness)
            || session.evidence_class()
                != d2b_contracts_resource::v3::identity::EvidenceClass::EnrolledKk
            || context.subject_ref() != session.subject_ref()
            || context.subject_uid() != session.subject_uid()
            || context.zone_ref().resource_type().as_str() != "Zone"
            || context.zone_ref().name().as_str() != session.zone().as_str()
            || context.evidence_class() != EvidenceClass::EnrolledKk
            || context.session_purpose().as_str() != self.purpose.as_str()
            || session.purpose_class() != self.purpose_class
            || session.initiator_role() != self.initiator_role
            || session.responder_role() != self.responder_role
            || session.endpoint_locality() != self.endpoint_locality
            || session.service().as_str() != self.service.as_str()
            || session.schema() != &self.schema_fingerprint
            || context.schema_fingerprint() != &self.schema_fingerprint
            || session.reconnect_generation() != self.reconnect_generation
            || context.reconnect_generation() != self.reconnect_generation
            || session.transport_class() != self.transport_class
            || session.transport_binding() != &self.transport_binding
            || context.transport_binding() != &self.transport_binding
            || session.locality() != self.transport_binding.locality()
        {
            return Err(ZonePolicyError::ZoneLinkNotAdmissible);
        }
        Ok(())
    }
}

/// Maximum lifetime of one runtime-issued route admission.
pub const MAX_ROUTE_ADMISSION_LIFETIME_MS: u64 = base::MAX_REQUEST_LIFETIME_MS;

/// Closed failures for route-admission construction and verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RouteAdmissionError {
    InvalidState,
    AuthorityMismatch,
    SealMismatch,
    Expired,
    Revoked,
    ZoneLinkMismatch,
    EdgeMismatch,
    ControllerGenerationMismatch,
    ReconnectGenerationMismatch,
    SourceZoneMismatch,
    TargetZoneMismatch,
    CapabilityMismatch,
    VerbMismatch,
    PolicyRevisionMismatch,
    PolicyRevisionRollback,
    SessionBindingMismatch,
    SessionNotLive,
}

impl RouteAdmissionError {
    /// Return the stable path-free failure label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidState => "route-admission-invalid-state",
            Self::AuthorityMismatch => "route-admission-authority-mismatch",
            Self::SealMismatch => "route-admission-seal-mismatch",
            Self::Expired => "route-admission-expired",
            Self::Revoked => "route-admission-revoked",
            Self::ZoneLinkMismatch => "route-admission-zonelink-mismatch",
            Self::EdgeMismatch => "route-admission-edge-mismatch",
            Self::ControllerGenerationMismatch => "route-admission-controller-generation-mismatch",
            Self::ReconnectGenerationMismatch => "route-admission-reconnect-generation-mismatch",
            Self::SourceZoneMismatch => "route-admission-source-zone-mismatch",
            Self::TargetZoneMismatch => "route-admission-target-zone-mismatch",
            Self::CapabilityMismatch => "route-admission-capability-mismatch",
            Self::VerbMismatch => "route-admission-verb-mismatch",
            Self::PolicyRevisionMismatch => "route-admission-policy-revision-mismatch",
            Self::PolicyRevisionRollback => "route-admission-policy-revision-rollback",
            Self::SessionBindingMismatch => "route-admission-session-binding-mismatch",
            Self::SessionNotLive => "route-admission-session-not-live",
        }
    }
}

impl core::fmt::Display for RouteAdmissionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl core::error::Error for RouteAdmissionError {}

struct RouteAdmissionState {
    zone_link_uid: ResourceUid,
    edge: ZoneTreeEdge,
    controller_generation: ZoneLinkControllerGeneration,
    source_zone_uid: ResourceUid,
    target_zone_uid: ResourceUid,
    required_capability: ZoneRouteCapability,
    verb: base::OperationClass,
    policy_revision: ZoneRevision,
    session_binding: RouteAdmissionSessionBinding,
    liveness: Option<d2b_session::SessionLiveness>,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    revoked: bool,
}

/// Runtime-owned route-admission configuration.
///
/// This constructor is crate-private on purpose. The public request can name
/// only the operation being attempted; all authenticated identity, policy,
/// generation, and time claims enter through this trusted boundary.
#[allow(dead_code)]
pub(crate) struct RouteAdmissionConfig {
    zone_link_uid: ResourceUid,
    edge: ZoneTreeEdge,
    controller_generation: ZoneLinkControllerGeneration,
    source_zone_uid: ResourceUid,
    target_zone_uid: ResourceUid,
    required_capability: ZoneRouteCapability,
    verb: base::OperationClass,
    policy_revision: ZoneRevision,
    session_binding: RouteAdmissionSessionBinding,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl RouteAdmissionConfig {
    #[allow(dead_code)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        zone_link_uid: ResourceUid,
        edge: ZoneTreeEdge,
        controller_generation: ZoneLinkControllerGeneration,
        source_zone_uid: ResourceUid,
        target_zone_uid: ResourceUid,
        required_capability: ZoneRouteCapability,
        verb: base::OperationClass,
        policy_revision: ZoneRevision,
        session_binding: RouteAdmissionSessionBinding,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Result<Self, RouteAdmissionError> {
        if source_zone_uid == target_zone_uid
            || policy_revision.get() == 0
            || (clock)()
                .checked_add(MAX_ROUTE_ADMISSION_LIFETIME_MS)
                .is_none()
        {
            return Err(RouteAdmissionError::InvalidState);
        }
        Ok(Self {
            zone_link_uid,
            edge,
            controller_generation,
            source_zone_uid,
            target_zone_uid,
            required_capability,
            verb,
            policy_revision,
            session_binding,
            clock,
        })
    }
}

struct RouteAdmissionAuthority {
    state: Mutex<RouteAdmissionState>,
}

/// Runtime-only issuer for one ZoneLink route-admission authority.
#[allow(dead_code)]
pub(crate) struct RouteAdmissionIssuer {
    authority: Arc<RouteAdmissionAuthority>,
}

/// Paired downstream verifier for runtime-issued route admission evidence.
///
/// The verifier cannot be constructed, cloned, defaulted, or converted from
/// caller input. It is handed out only by the runtime-owned admission pair.
pub struct RouteAdmissionVerifier {
    authority: Arc<RouteAdmissionAuthority>,
}

/// Runtime-owned ZoneLink route-admission issuer.
///
/// The authority is constructed only from an already authenticated
/// ComponentSession route and the exact committed ZoneLink identity. Callers
/// receive sealed evidence paired with a verifier for one downstream
/// consumer; they cannot construct or clone either half.
pub struct RuntimeRouteAdmissionAuthority {
    issuer: RouteAdmissionIssuer,
    verifier: RouteAdmissionVerifier,
}

impl fmt::Debug for RuntimeRouteAdmissionAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeRouteAdmissionAuthority(<redacted>)")
    }
}

impl RuntimeRouteAdmissionAuthority {
    /// Bind route admissions to one authenticated ComponentSession and
    /// committed ZoneLink policy.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        zone_link_uid: ResourceUid,
        edge: ZoneTreeEdge,
        controller_generation: ZoneLinkControllerGeneration,
        source_zone_uid: ResourceUid,
        target_zone_uid: ResourceUid,
        required_capability: ZoneRouteCapability,
        verb: base::OperationClass,
        policy_revision: ZoneRevision,
        policy: &base::EndpointPolicy,
        session: &d2b_session::AuthenticatedSessionRouteBinding,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Result<Self, RouteAdmissionError> {
        let session_binding =
            RouteAdmissionSessionBinding::from_authenticated_session(policy, session)
                .map_err(|_| RouteAdmissionError::SessionBindingMismatch)?;
        let config = RouteAdmissionConfig::new(
            zone_link_uid,
            edge,
            controller_generation,
            source_zone_uid,
            target_zone_uid,
            required_capability,
            verb,
            policy_revision,
            session_binding,
            clock,
        )?;
        let (issuer, verifier) = route_admission_pair(config);
        Ok(Self { issuer, verifier })
    }

    /// Issue evidence and a fresh paired verifier for one operation.
    pub fn issue(
        &self,
        request: ZoneLinkRouteAdmissionRequest,
    ) -> Result<(RouteAdmissionVerifier, RouteAdmissionEvidence), RouteAdmissionError> {
        let evidence = self.issuer.issue(request)?;
        let verifier = RouteAdmissionVerifier {
            authority: Arc::clone(&self.verifier.authority),
        };
        Ok((verifier, evidence))
    }

    /// Advance the committed policy without widening an existing authority.
    pub fn update_policy(
        &self,
        required_capability: ZoneRouteCapability,
        verb: base::OperationClass,
        revision: ZoneRevision,
    ) -> Result<(), RouteAdmissionError> {
        self.verifier
            .update_policy(required_capability, verb, revision)
    }

    /// Revoke all future admissions from this exact authority.
    pub fn revoke(&self) {
        self.verifier.revoke();
    }
}

/// Sealed route-admission evidence.
///
/// The evidence has no public constructor, serializer, debugger, or cloning
/// path. Only the paired [`RouteAdmissionVerifier`] can consume it.
///
/// ```compile_fail
/// use d2b_bus::session::contract::RouteAdmissionEvidence;
///
/// fn forge(mut value: RouteAdmissionEvidence) {
///     value.body = todo!();
/// }
/// ```
pub struct RouteAdmissionEvidence {
    authority: Arc<RouteAdmissionAuthority>,
    body: RouteAdmissionBody,
    seal: [u8; 32],
}

/// Verified, immutable route-admission claims for one downstream consumer.
pub struct VerifiedRouteAdmission {
    authority: Arc<RouteAdmissionAuthority>,
    body: RouteAdmissionBody,
}

#[derive(Clone)]
struct RouteAdmissionBody {
    zone_link_uid: ResourceUid,
    edge: ZoneTreeEdge,
    controller_generation: ZoneLinkControllerGeneration,
    reconnect_generation: ReconnectGeneration,
    source_zone_uid: ResourceUid,
    target_zone_uid: ResourceUid,
    operation_id: base::OperationId,
    verb: base::OperationClass,
    required_capability: ZoneRouteCapability,
    policy_revision: ZoneRevision,
    issued_at_unix_ms: u64,
    expires_at_unix_ms: u64,
    session_binding: RouteAdmissionSessionBinding,
}

fn route_admission_verb_label(verb: base::OperationClass) -> &'static str {
    match verb {
        base::OperationClass::Connect => "connect",
        base::OperationClass::Invoke => "invoke",
        base::OperationClass::OpenStream => "open-stream",
        base::OperationClass::Relay => "relay",
        base::OperationClass::Attach => "attach",
        base::OperationClass::Cancel => "cancel",
        base::OperationClass::Observe => "observe",
        base::OperationClass::AuditExport => "audit-export",
        base::OperationClass::SupportBundle => "support-bundle",
    }
}

fn route_admission_digest(body: &RouteAdmissionBody) -> [u8; 32] {
    let encoded = canonical_json_bytes(&(
        (
            &body.zone_link_uid,
            &body.edge,
            &body.controller_generation,
            body.reconnect_generation,
            &body.source_zone_uid,
            &body.target_zone_uid,
            &body.operation_id,
            route_admission_verb_label(body.verb),
            &body.required_capability,
            body.policy_revision,
            body.issued_at_unix_ms,
            body.expires_at_unix_ms,
        ),
        (
            body.session_binding.purpose().as_str(),
            body.session_binding.purpose_class().as_str(),
            body.session_binding.initiator_role().as_str(),
            body.session_binding.responder_role().as_str(),
            body.session_binding.endpoint_locality().as_str(),
            body.session_binding.service().as_str(),
            body.session_binding.reconnect_generation(),
            body.session_binding.schema_fingerprint(),
            body.session_binding.transport_class(),
            body.session_binding.transport_binding().locality(),
            body.session_binding.transport_binding().binding_digest(),
        ),
    ))
    .expect("route admission body is serializable");
    Sha256::digest(encoded).into()
}

/// Create the runtime-owned issuer and its paired verifier.
#[allow(dead_code)]
pub(crate) fn route_admission_pair(
    config: RouteAdmissionConfig,
) -> (RouteAdmissionIssuer, RouteAdmissionVerifier) {
    let liveness = config.session_binding.liveness.clone();
    let authority = Arc::new(RouteAdmissionAuthority {
        state: Mutex::new(RouteAdmissionState {
            zone_link_uid: config.zone_link_uid,
            edge: config.edge,
            controller_generation: config.controller_generation,
            source_zone_uid: config.source_zone_uid,
            target_zone_uid: config.target_zone_uid,
            required_capability: config.required_capability,
            verb: config.verb,
            policy_revision: config.policy_revision,
            session_binding: config.session_binding,
            liveness,
            clock: config.clock,
            revoked: false,
        }),
    });
    (
        RouteAdmissionIssuer {
            authority: Arc::clone(&authority),
        },
        RouteAdmissionVerifier { authority },
    )
}

impl RouteAdmissionIssuer {
    /// Issue evidence using only runtime-owned identity, policy, and clock
    /// state. The caller cannot supply those claims.
    #[allow(dead_code)]
    pub(crate) fn issue(
        &self,
        request: ZoneLinkRouteAdmissionRequest,
    ) -> Result<RouteAdmissionEvidence, RouteAdmissionError> {
        let state = self
            .authority
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.revoked {
            return Err(RouteAdmissionError::Revoked);
        }
        if state.policy_revision.get() == 0 {
            return Err(RouteAdmissionError::InvalidState);
        }
        if state
            .liveness
            .as_ref()
            .is_some_and(|liveness| !liveness.is_live())
        {
            return Err(RouteAdmissionError::SessionNotLive);
        }
        if request.verb() != state.verb {
            return Err(RouteAdmissionError::VerbMismatch);
        }
        let issued_at_unix_ms = (state.clock)();
        let expires_at_unix_ms = issued_at_unix_ms
            .checked_add(MAX_ROUTE_ADMISSION_LIFETIME_MS)
            .ok_or(RouteAdmissionError::InvalidState)?;
        let body = RouteAdmissionBody {
            zone_link_uid: state.zone_link_uid.clone(),
            edge: state.edge.clone(),
            controller_generation: state.controller_generation.clone(),
            reconnect_generation: state.session_binding.reconnect_generation(),
            source_zone_uid: state.source_zone_uid.clone(),
            target_zone_uid: state.target_zone_uid.clone(),
            operation_id: request.operation_id().clone(),
            verb: state.verb,
            required_capability: state.required_capability.clone(),
            policy_revision: state.policy_revision,
            issued_at_unix_ms,
            expires_at_unix_ms,
            session_binding: state.session_binding.clone(),
        };
        Ok(RouteAdmissionEvidence {
            authority: Arc::clone(&self.authority),
            seal: route_admission_digest(&body),
            body,
        })
    }
}

impl RouteAdmissionVerifier {
    /// Consume and verify one sealed route admission.
    pub fn verify(
        &self,
        evidence: RouteAdmissionEvidence,
    ) -> Result<VerifiedRouteAdmission, RouteAdmissionError> {
        if !Arc::ptr_eq(&self.authority, &evidence.authority) {
            return Err(RouteAdmissionError::AuthorityMismatch);
        }
        if route_admission_digest(&evidence.body) != evidence.seal {
            return Err(RouteAdmissionError::SealMismatch);
        }
        let state = self
            .authority
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.revoked {
            return Err(RouteAdmissionError::Revoked);
        }
        if state.policy_revision.get() == 0 {
            return Err(RouteAdmissionError::InvalidState);
        }
        if state
            .liveness
            .as_ref()
            .is_some_and(|liveness| !liveness.is_live())
        {
            return Err(RouteAdmissionError::SessionNotLive);
        }
        let now = (state.clock)();
        if evidence.body.expires_at_unix_ms <= evidence.body.issued_at_unix_ms
            || evidence.body.expires_at_unix_ms - evidence.body.issued_at_unix_ms
                > MAX_ROUTE_ADMISSION_LIFETIME_MS
            || now < evidence.body.issued_at_unix_ms
            || now >= evidence.body.expires_at_unix_ms
        {
            return Err(RouteAdmissionError::Expired);
        }
        if evidence.body.zone_link_uid != state.zone_link_uid {
            return Err(RouteAdmissionError::ZoneLinkMismatch);
        }
        if evidence.body.edge != state.edge {
            return Err(RouteAdmissionError::EdgeMismatch);
        }
        if evidence.body.controller_generation != state.controller_generation {
            return Err(RouteAdmissionError::ControllerGenerationMismatch);
        }
        if evidence.body.reconnect_generation != state.session_binding.reconnect_generation() {
            return Err(RouteAdmissionError::ReconnectGenerationMismatch);
        }
        if evidence.body.source_zone_uid != state.source_zone_uid {
            return Err(RouteAdmissionError::SourceZoneMismatch);
        }
        if evidence.body.target_zone_uid != state.target_zone_uid {
            return Err(RouteAdmissionError::TargetZoneMismatch);
        }
        if evidence.body.verb != state.verb {
            return Err(RouteAdmissionError::VerbMismatch);
        }
        if evidence.body.required_capability != state.required_capability {
            return Err(RouteAdmissionError::CapabilityMismatch);
        }
        if evidence.body.policy_revision != state.policy_revision {
            return Err(RouteAdmissionError::PolicyRevisionMismatch);
        }
        if evidence.body.session_binding != state.session_binding {
            return Err(RouteAdmissionError::SessionBindingMismatch);
        }
        Ok(VerifiedRouteAdmission {
            authority: Arc::clone(&self.authority),
            body: evidence.body,
        })
    }

    /// Atomically replace the runtime route policy snapshot.
    #[allow(dead_code)]
    pub(crate) fn update_policy(
        &self,
        required_capability: ZoneRouteCapability,
        verb: base::OperationClass,
        revision: ZoneRevision,
    ) -> Result<(), RouteAdmissionError> {
        if revision.get() == 0 {
            return Err(RouteAdmissionError::InvalidState);
        }
        let mut state = self
            .authority
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if revision <= state.policy_revision {
            return Err(RouteAdmissionError::PolicyRevisionRollback);
        }
        state.required_capability = required_capability;
        state.verb = verb;
        state.policy_revision = revision;
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn revoke(&self) {
        self.authority
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .revoked = true;
    }
}

impl VerifiedRouteAdmission {
    /// Re-check the runtime-owned authority before using this admission.
    pub(crate) fn revalidate(&self) -> Result<(), RouteAdmissionError> {
        let verifier = RouteAdmissionVerifier {
            authority: Arc::clone(&self.authority),
        };
        verifier
            .verify(RouteAdmissionEvidence {
                authority: Arc::clone(&self.authority),
                seal: route_admission_digest(&self.body),
                body: self.body.clone(),
            })
            .map(|_| ())
    }

    pub const fn zone_link_uid(&self) -> &ResourceUid {
        &self.body.zone_link_uid
    }

    pub const fn edge(&self) -> &ZoneTreeEdge {
        &self.body.edge
    }

    pub const fn controller_generation(&self) -> &ZoneLinkControllerGeneration {
        &self.body.controller_generation
    }

    pub const fn reconnect_generation(&self) -> ReconnectGeneration {
        self.body.reconnect_generation
    }

    pub const fn source_zone_uid(&self) -> &ResourceUid {
        &self.body.source_zone_uid
    }

    pub const fn target_zone_uid(&self) -> &ResourceUid {
        &self.body.target_zone_uid
    }

    pub const fn operation_id(&self) -> &base::OperationId {
        &self.body.operation_id
    }

    pub const fn verb(&self) -> base::OperationClass {
        self.body.verb
    }

    pub const fn required_capability(&self) -> &ZoneRouteCapability {
        &self.body.required_capability
    }

    pub const fn policy_revision(&self) -> ZoneRevision {
        self.body.policy_revision
    }

    pub const fn issued_at_unix_ms(&self) -> u64 {
        self.body.issued_at_unix_ms
    }

    pub const fn expires_at_unix_ms(&self) -> u64 {
        self.body.expires_at_unix_ms
    }

    pub const fn session_binding(&self) -> &RouteAdmissionSessionBinding {
        &self.body.session_binding
    }
}

fn assert_route_admission_types_have_no_minting_traits() {
    trait CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<A> {
        fn some_item() {}
    }
    impl<T: ?Sized> CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<()> for T {}
    impl<T: Clone> CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<u8> for T {}
    impl<T: Copy> CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<u16> for T {}
    impl<T: Default> CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<u32> for T {}
    impl<T: core::fmt::Debug> CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<u64> for T {}
    impl<T: From<()>> CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<u128> for T {}
    let _ =
        <RouteAdmissionEvidence as CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<_>>::some_item;
    let _ =
        <RouteAdmissionIssuer as CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<_>>::some_item;
    let _ =
        <RouteAdmissionVerifier as CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<_>>::some_item;
    let _ =
        <VerifiedRouteAdmission as CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom<_>>::some_item;
}

const _: fn() = assert_route_admission_types_have_no_minting_traits;

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use d2b_contracts_zone_session::v3::zone_session::{
        IdentityEvidenceRequirement, Locality, TransportClass,
    };

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

#[cfg(test)]
mod route_admission_tests {
    use super::*;
    use d2b_contracts_resource::v3::identity::ReconnectGeneration;
    use d2b_contracts_resource::v3::{ResourceUid, ZoneRevision};
    use d2b_contracts_zone_session::v3::component_session::{OperationClass, OperationId};
    use d2b_contracts_zone_session::v3::zone_routing::{
        ZoneLabelId, ZoneLinkControllerGeneration, ZonePath, ZoneRouteCapability, ZoneTreeEdge,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    fn uid(value: char) -> ResourceUid {
        let value = match value {
            '1' => "11111111-1111-4111-8111-111111111111",
            '2' => "22222222-2222-4222-8222-222222222222",
            '3' => "33333333-3333-4333-8333-333333333333",
            '4' => "44444444-4444-4444-8444-444444444444",
            _ => panic!("test UID marker must be one of 1..=4"),
        };
        ResourceUid::parse(value).expect("valid resource UID")
    }

    fn path(labels: &[&str]) -> ZonePath {
        ZonePath::new(
            labels
                .iter()
                .map(|label| ZoneLabelId::parse(*label).expect("valid Zone label"))
                .collect(),
        )
        .expect("valid Zone path")
    }

    fn route_admission_pair_with_clock(
        initial_time: u64,
    ) -> (RouteAdmissionIssuer, RouteAdmissionVerifier, Arc<AtomicU64>) {
        let policy = fixtures::enrolled_zone_link(7)
            .lower()
            .expect("valid ZoneLink session policy");
        let session = RouteAdmissionSessionBinding::from_policy(&policy)
            .expect("valid ZoneLink session policy");
        let clock = Arc::new(AtomicU64::new(initial_time));
        let clock_for_config = Arc::clone(&clock);
        let config = RouteAdmissionConfig::new(
            uid('1'),
            ZoneTreeEdge::new(path(&["parent"]), path(&["child", "parent"]))
                .expect("direct Zone edge"),
            ZoneLinkControllerGeneration::parse("generation-1")
                .expect("valid controller generation"),
            uid('2'),
            uid('3'),
            ZoneRouteCapability::parse("resource-read").expect("valid capability"),
            OperationClass::Invoke,
            ZoneRevision::new(9),
            session,
            Arc::new(move || clock_for_config.load(Ordering::Acquire)),
        )
        .expect("valid route admission state");
        let (issuer, verifier) = super::route_admission_pair(config);
        (issuer, verifier, clock)
    }

    fn route_admission_pair() -> (RouteAdmissionIssuer, RouteAdmissionVerifier) {
        let (issuer, verifier, _clock) = route_admission_pair_with_clock(1_000);
        (issuer, verifier)
    }

    fn request() -> ZoneLinkRouteAdmissionRequest {
        ZoneLinkRouteAdmissionRequest::new(
            OperationId::new(vec![0x11; 16]).expect("valid operation id"),
            OperationClass::Invoke,
        )
        .expect("valid route admission request")
    }

    #[test]
    fn runtime_issued_route_admission_is_bound_and_verifiable() {
        let (issuer, verifier) = route_admission_pair();
        let opened = verifier
            .verify(issuer.issue(request()).expect("issue"))
            .expect("verify");

        assert_eq!(opened.zone_link_uid(), &uid('1'));
        assert_eq!(opened.source_zone_uid(), &uid('2'));
        assert_eq!(opened.target_zone_uid(), &uid('3'));
        assert_eq!(opened.controller_generation().as_str(), "generation-1");
        assert_eq!(opened.reconnect_generation().get(), 7);
        assert_eq!(opened.policy_revision().get(), 9);
        assert_eq!(opened.verb(), OperationClass::Invoke);
        assert_eq!(opened.required_capability().as_str(), "resource-read");
        assert_eq!(opened.issued_at_unix_ms(), 1_000);
        assert_eq!(
            opened.expires_at_unix_ms(),
            1_000 + MAX_ROUTE_ADMISSION_LIFETIME_MS
        );
        assert_eq!(
            opened.session_binding().purpose(),
            base::EndpointPurpose::ZoneLink
        );
    }

    #[test]
    fn a_seal_rejects_forged_time_policy_capability_target_and_verb() {
        for forge in [
            |body: &mut RouteAdmissionBody| body.issued_at_unix_ms += 1,
            |body: &mut RouteAdmissionBody| body.expires_at_unix_ms += 1,
            |body: &mut RouteAdmissionBody| body.policy_revision = ZoneRevision::new(10),
            |body: &mut RouteAdmissionBody| {
                body.required_capability =
                    ZoneRouteCapability::parse("resource-write").expect("valid capability")
            },
            |body: &mut RouteAdmissionBody| body.target_zone_uid = uid('4'),
            |body: &mut RouteAdmissionBody| body.verb = OperationClass::Cancel,
        ] {
            let (issuer, verifier) = route_admission_pair();
            let mut evidence = issuer.issue(request()).expect("issue");
            forge(&mut evidence.body);
            match verifier.verify(evidence) {
                Err(error) => assert_eq!(error, RouteAdmissionError::SealMismatch),
                Ok(_) => panic!("forged route admission was accepted"),
            }
        }
    }

    #[test]
    fn stale_controller_and_reconnect_generations_fail_closed() {
        let (issuer, verifier) = route_admission_pair();
        let evidence = issuer.issue(request()).expect("issue");
        // Test-only state mutation models a fresh authenticated controller binding.
        verifier
            .authority
            .state
            .lock()
            .unwrap()
            .controller_generation =
            ZoneLinkControllerGeneration::parse("generation-2").expect("valid generation");
        match verifier.verify(evidence) {
            Err(error) => assert_eq!(error, RouteAdmissionError::ControllerGenerationMismatch),
            Ok(_) => panic!("stale controller generation was accepted"),
        }

        let (issuer, verifier) = route_admission_pair();
        let evidence = issuer.issue(request()).expect("issue");
        // Test-only state mutation models a fresh authenticated session binding.
        verifier
            .authority
            .state
            .lock()
            .unwrap()
            .session_binding
            .reconnect_generation = ReconnectGeneration::new(8).expect("valid generation");
        match verifier.verify(evidence) {
            Err(error) => assert_eq!(error, RouteAdmissionError::ReconnectGenerationMismatch),
            Ok(_) => panic!("stale reconnect generation was accepted"),
        }
    }

    #[test]
    fn route_admission_expiry_uses_the_runtime_clock() {
        let (issuer, verifier, clock) = route_admission_pair_with_clock(1_000);
        let evidence = issuer.issue(request()).expect("issue");
        clock.store(1_000 + MAX_ROUTE_ADMISSION_LIFETIME_MS, Ordering::Release);
        match verifier.verify(evidence) {
            Err(error) => assert_eq!(error, RouteAdmissionError::Expired),
            Ok(_) => panic!("expired route admission was accepted"),
        }
    }

    #[test]
    fn capability_and_policy_changes_fence_existing_evidence() {
        let (issuer, verifier) = route_admission_pair();
        let evidence = issuer.issue(request()).expect("issue");
        verifier
            .update_policy(
                ZoneRouteCapability::parse("resource-write").expect("valid capability"),
                OperationClass::Invoke,
                ZoneRevision::new(10),
            )
            .expect("advance policy with a narrowed capability");
        match verifier.verify(evidence) {
            Err(error) => assert_eq!(error, RouteAdmissionError::CapabilityMismatch),
            Ok(_) => panic!("evidence survived capability narrowing"),
        }

        let (issuer, verifier) = route_admission_pair();
        let evidence = issuer.issue(request()).expect("issue");
        verifier
            .update_policy(
                ZoneRouteCapability::parse("resource-read").expect("valid capability"),
                OperationClass::Invoke,
                ZoneRevision::new(10),
            )
            .expect("advance policy revision");
        match verifier.verify(evidence) {
            Err(error) => assert_eq!(error, RouteAdmissionError::PolicyRevisionMismatch),
            Ok(_) => panic!("evidence survived policy revision change"),
        }

        assert_eq!(
            verifier.update_policy(
                ZoneRouteCapability::parse("resource-read").expect("valid capability"),
                OperationClass::Invoke,
                ZoneRevision::new(0),
            ),
            Err(RouteAdmissionError::InvalidState)
        );
        assert_eq!(
            verifier.update_policy(
                ZoneRouteCapability::parse("resource-read").expect("valid capability"),
                OperationClass::Invoke,
                ZoneRevision::new(9),
            ),
            Err(RouteAdmissionError::PolicyRevisionRollback)
        );

        let (issuer, verifier) = route_admission_pair();
        verifier
            .update_policy(
                ZoneRouteCapability::parse("resource-read").expect("valid capability"),
                OperationClass::Cancel,
                ZoneRevision::new(10),
            )
            .expect("advance operation policy");
        match issuer.issue(request()) {
            Err(error) => assert_eq!(error, RouteAdmissionError::VerbMismatch),
            Ok(_) => panic!("issuer accepted a verb outside its authority"),
        }
    }

    #[test]
    fn evidence_from_another_runtime_authority_is_not_verifiable() {
        let (issuer, _first_verifier) = route_admission_pair();
        let (_second_issuer, second_verifier) = route_admission_pair();
        let evidence = issuer.issue(request()).expect("issue");
        match second_verifier.verify(evidence) {
            Err(error) => assert_eq!(error, RouteAdmissionError::AuthorityMismatch),
            Ok(_) => panic!("evidence crossed runtime authorities"),
        }
    }

    #[test]
    fn revoked_route_admission_fails_closed() {
        let (issuer, verifier) = route_admission_pair();
        let evidence = issuer.issue(request()).expect("issue");
        verifier.revoke();
        match verifier.verify(evidence) {
            Err(error) => assert_eq!(error, RouteAdmissionError::Revoked),
            Ok(_) => panic!("revoked route admission was accepted"),
        }
        match issuer.issue(request()) {
            Err(error) => assert_eq!(error, RouteAdmissionError::Revoked),
            Ok(_) => panic!("revoked issuer minted route admission"),
        }
    }

    #[test]
    fn zone_link_attach_requests_are_rejected_before_issuance() {
        let request = ZoneLinkRouteAdmissionRequest::new(
            OperationId::new(vec![0x12; 16]).expect("valid operation id"),
            OperationClass::Attach,
        );
        assert!(request.is_err());
    }
}
