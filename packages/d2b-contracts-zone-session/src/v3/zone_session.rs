//! Zone session wire contracts for v3 ZoneLink and Zone-local sessions.
//!
//! This module is the contract surface every d2b-bus session and transport
//! module imports. It owns exactly two things:
//!
//! 1. The v3 Zone endpoint taxonomy: the closed [`EndpointPurpose`],
//!    [`EndpointRole`], and [`ServicePackage`] enumerations, extended with the
//!    Zone members the v3 routing plane needs.
//! 2. A single re-export point for the protocol constants and the transport
//!    neutral session primitives that are carried into v3 unchanged.
//!
//! Everything in group 2 is re-exported from [`super::component_session`]
//! rather than copied. The work item's detailed design says "copy verbatim";
//! a re-export is the non-duplicating spelling of the same contract, keeps a
//! single definition of every byte layout, and means a golden vector proved
//! once stays proved. Nothing is forked.
//!
//! # No authority
//!
//! Every type reachable from here is plain desired-state or wire metadata.
//! Nothing in this module carries authority: there is no session object,
//! admission evidence, verified peer, resolved subject, or proof, and no uid,
//! gid, host path, socket path, store path, key, or credential. The session
//! implementation layer supplies all runtime state itself.
//!
//! # Guest-session credentials are stripped
//!
//! The ADR45 `GUEST_SESSION_CREDENTIAL_*` and `GUEST_BOOTSTRAP_CREDENTIAL_*`
//! constants and types are excluded from the v3 ZoneLink session contract.
//! They are absent from [`super::component_session`] as well, so there is
//! nothing to strip here; this module deliberately declares no replacement and
//! re-exports no credential embedding. v3 Guest enrollment goes through the
//! Zone resource model, and ZoneLink bootstrap is the one-time IKpsk2
//! handshake that terminates into a distinct enrolled `Noise_KK` handshake.
//!
//! # Wire tag values: stated versus inferred
//!
//! The u8 tags below are a wire contract. They are frozen once and never
//! reused or renumbered, exactly as the store discriminants are. The rules
//! this module applies, and their provenance:
//!
//! - Tags already assigned in [`super::component_session`] are preserved at
//!   their existing values. `ADR-046-resources-zone-control` states that v3
//!   "will append new tags for Zone API endpoints without renumbering
//!   existing ones", and `ADR-046-nix-configuration` states repeatedly that a
//!   variant may be renamed but "wire tag values are stable and must not
//!   change".
//! - New Zone members are appended at the next unused tag. This is the
//!   "at new tag values" instruction in the work item's detailed design.
//! - Tags 7 and 8 of [`EndpointRole`] are permanently reserved and
//!   unassigned here. They held the generic `Relay` and `Bootstrapper` roles;
//!   the v3 Zone taxonomy names `ZoneRelay` and `ZoneBootstrap` instead, and
//!   the two spellings must not coexist. Reserving rather than reusing keeps
//!   a v3 peer from silently reading an old tag as a new role.
//!
//! Two service-package wire strings are stated by the specs and used verbatim:
//! `d2b.resource.v3` and `d2b.zone.v3`. The remaining new wire strings and
//! every new numeric tag are the minimal defensible extension of the frozen
//! scheme, not a spec quotation. They are listed in the module's report as
//! inferences pending explicit contract confirmation:
//! `EndpointPurpose::ZoneLocal` = 14, `EndpointPurpose::ZoneControl` = 15,
//! `EndpointRole::ZoneRelay` = 9, `EndpointRole::ZoneBootstrap` = 10,
//! `ServicePackage::ZoneV3` = 7, `ServicePackage::ZoneLinkV3` = 8, and the
//! wire string `d2b.zonelink.v3`.
//!
//! # Relationship to the v2-shaped session structs
//!
//! [`super::component_session`] owns `HandshakeOffer`, `EndpointPolicy`,
//! `MetricLabels`, and `AttachmentDescriptor`, whose enum-typed fields name
//! *that* module's enumerations. Those structs are not re-exported here except
//! for `AttachmentDescriptor`, which the work item names explicitly. Its
//! `service` field remains the component-session `ServicePackage`. Use
//! [`ServicePackage::to_component_session`] to lower a Zone service package
//! into that field, and [`ServicePackage::from_component_session`] to lift one
//! back. The same total-lift / partial-lower pair exists for
//! [`EndpointPurpose`] and [`EndpointRole`]. Widening those struct fields to
//! the Zone enumerations is a change to `component_session.rs`, which this
//! work item does not own.

use std::fmt;

use d2b_contracts_resource::v3::identity::ReconnectGeneration;
use d2b_contracts_resource::v3::{ResourceUid, ZoneId};
use serde::{Deserialize, Serialize};

use super::component_session as base;
use super::zone_routing::{ZoneLinkControllerGeneration, ZoneTreeEdge};

pub use super::component_session::{
    COMPONENT_SESSION_MAJOR, COMPONENT_SESSION_MINOR, ENDPOINT_POLICY_IDENTITY_CANONICAL_LEN,
    FRAGMENT_HEADER_LEN, HANDSHAKE_OFFER_CANONICAL_LEN, LOCAL_HANDSHAKE_DEADLINE_MS,
    LOCAL_RECONNECT_DEADLINE_MS, MAX_ACTIVE_NAMED_STREAMS, MAX_AGGREGATE_NAMED_STREAM_QUEUE_BYTES,
    MAX_CLOCK_SKEW_MS, MAX_HANDSHAKE_OFFER_BYTES, MAX_HOST_ATTACHMENT_CREDITS, MAX_ID_BYTES,
    MAX_KEEPALIVE_INTERVAL_MS, MAX_KEEPALIVE_TIMEOUT_MS, MAX_LOGICAL_MESSAGE_BYTES,
    MAX_NAMED_STREAM_QUEUE_BYTES, MAX_OPERATION_ATTACHMENTS, MAX_PACKET_ATTACHMENTS,
    MAX_PROCESS_ATTACHMENT_CREDITS, MAX_PROTECTED_CIPHERTEXT_BYTES, MAX_PROTECTED_PLAINTEXT_BYTES,
    MAX_RECONNECT_ATTEMPTS, MAX_RECONNECT_WINDOW_MS, MAX_REQUEST_ATTACHMENTS,
    MAX_REQUEST_LIFETIME_MS, MAX_SESSION_ATTACHMENTS, MAX_SESSION_CONTROL_QUEUE_BYTES,
    MAX_TTRPC_CONTROL_QUEUE_BYTES, NOISE_TAG_BYTES, PREFACE_LEN, PREFACE_MAGIC, RECORD_HEADER_LEN,
    RECORD_LENGTH_BYTES, REMOTE_HANDSHAKE_DEADLINE_MS, REMOTE_RECONNECT_DEADLINE_MS,
    RESERVED_CONTROL_FDS,
};

pub use super::component_session::{
    AttachmentAccess, AttachmentCreditClass, AttachmentCredits, AttachmentDescriptor,
    AttachmentKind, AttachmentPacket, AttachmentPolicy, AttachmentPolicyKind, AttachmentPurpose,
    AttachmentReceiveError, BinaryError, BoundedVec, ChannelClass, ChannelId, CloseReason,
    CloseRecord, ComponentSessionPreface, ContractError, CorrelationId, FragmentHeader,
    FragmentSequence, FragmentSequenceError, IdempotencyKey, IdentityEvidenceRequirement,
    KeepaliveRecord, KernelObjectType, LimitProfile, Locality, NoiseProfile, OperationId,
    PrefaceError, PurposeClass, ReceiveSequence, RecordHeader, RecordKind, Remediation, RequestId,
    SendSequence, SequenceError, SessionErrorCode, TraceId, TransportClass,
};

/// Declares a closed wire enumeration with a frozen u8 tag and wire string.
///
/// This mirrors the `closed_enum!` shape used by [`super::component_session`],
/// which is a private macro there. Redeclaring the three-line shape locally is
/// cheaper and less coupled than exporting a macro across module boundaries,
/// and the generated surface is identical: `ALL`, `tag`, `as_str`, `from_tag`.
macro_rules! zone_closed_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident = $tag:literal => $wire:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
            Serialize, Deserialize, schemars::JsonSchema,
        )]
        pub enum $name {
            $(
                $(#[$vmeta])*
                #[serde(rename = $wire)]
                #[schemars(rename = $wire)]
                $variant
            ),+
        }

        impl $name {
            /// Every variant, in frozen tag order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The frozen u8 wire tag.
            pub const fn tag(self) -> u8 {
                match self {
                    $(Self::$variant => $tag),+
                }
            }

            /// The frozen wire string, also the audit and metric label.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),+
                }
            }

            /// Decodes a wire tag, fail-closed on an unassigned or reserved
            /// value.
            pub fn from_tag(tag: u8) -> Result<Self, BinaryError> {
                match tag {
                    $($tag => Ok(Self::$variant),)+
                    _ => Err(BinaryError::UnknownEnumTag),
                }
            }
        }
    };
}

zone_closed_enum!(
    /// The closed purpose of one v3 Zone session endpoint.
    ///
    /// Tags 1 through 13 are preserved from the component-session assignment.
    /// Tags 14 and 15 are the appended v3 Zone purposes.
    EndpointPurpose {
        /// Lifecycle control on a Zone-local endpoint.
        LocalLifecycle = 1 => "local-lifecycle",
        /// The `d2b.resource.v3` resource service.
        ResourceService = 2 => "resource-service",
        /// An enrolled ZoneLink between a parent and a child Zone.
        ZoneLink = 3 => "zone-link",
        /// The one-time IKpsk2 enrollment bootstrap, which terminates.
        Bootstrap = 4 => "bootstrap",
        /// ComponentSession traffic.
        ComponentSession = 5 => "component-session",
        /// Bulk resource transfer.
        ResourceTransfer = 6 => "resource-transfer",
        /// Provider agent control traffic.
        ProviderControl = 7 => "provider-control",
        /// A dedicated end-to-end sensitive-credential delivery session.
        SensitiveCredential = 8 => "sensitive-credential",
        /// User agent control traffic.
        UserControl = 9 => "user-control",
        /// A controller watch stream.
        ControllerWatch = 10 => "controller-watch",
        /// A named stream channel.
        NamedStream = 11 => "named-stream",
        /// Audit export.
        AuditExport = 12 => "audit-export",
        /// Support-bundle collection.
        SupportBundle = 13 => "support-bundle",
        /// A Zone-local endpoint reached over an allocator-issued socket,
        /// never over a ZoneLink.
        ZoneLocal = 14 => "zone-local",
        /// The `d2b.zone.v3` Zone control service.
        ZoneControl = 15 => "zone-control",
    }
);

zone_closed_enum!(
    /// The closed role one v3 Zone session endpoint plays.
    ///
    /// Tags 1 through 6 are preserved from the component-session assignment.
    /// Tags 7 and 8 are permanently reserved and deliberately unassigned; see
    /// the module documentation. Tags 9 and 10 are the appended Zone roles.
    EndpointRole {
        /// A component within a Zone.
        Component = 1 => "component",
        /// The Zone runtime controller.
        ZoneController = 2 => "zone-controller",
        /// A Host-side agent.
        HostAgent = 3 => "host-agent",
        /// A Guest-side agent.
        GuestAgent = 4 => "guest-agent",
        /// A Provider agent.
        Provider = 5 => "provider",
        /// A per-user agent.
        UserAgent = 6 => "user-agent",
        /// A Zone that forwards a call on behalf of another Zone under an
        /// exact ZoneLink-scoped relay grant.
        ZoneRelay = 9 => "zone-relay",
        /// The endpoint of a one-time ZoneLink enrollment bootstrap.
        ZoneBootstrap = 10 => "zone-bootstrap",
    }
);

zone_closed_enum!(
    /// The closed service package a v3 Zone session carries.
    ///
    /// Tags 1 through 6 are preserved from the component-session assignment.
    /// Tags 7 and 8 are the appended Zone service packages. Tags 9 through
    /// 13 carry the interaction Provider packages shared with
    /// ComponentSession. Tag 14 carries the service-only config-nixos
    /// Provider. Protobuf field numbers for the v3 services are
    /// frozen independently of the v2 assignments and are not restated by
    /// this module.
    ServicePackage {
        /// `d2b.resource.v3.ResourceService`.
        ResourceV3 = 1 => "d2b.resource.v3",
        /// The controller service package.
        ControllerV3 = 2 => "d2b.controller.v3",
        /// The Provider service package.
        ProviderV3 = 3 => "d2b.provider.v3",
        /// The audit service package.
        AuditV3 = 4 => "d2b.audit.v3",
        /// The support service package.
        SupportV3 = 5 => "d2b.support.v3",
        /// The credential service package.
        CredentialV3 = 6 => "d2b.credential.v3",
        /// `d2b.zone.v3.ZoneService`.
        ZoneV3 = 7 => "d2b.zone.v3",
        /// The ZoneLink carriage service package.
        ZoneLinkV3 = 8 => "d2b.zonelink.v3",
        /// The Wayland display Provider package.
        DisplayV3 = 9 => "d2b.display.v3",
        /// The Wayland clipboard Provider package.
        ClipboardV3 = 10 => "d2b.clipboard.v3",
        /// The Wayland clipboard bridge package.
        ClipboardBridgeV3 = 11 => "d2b.clipboard.bridge.v3",
        /// The Wayland clipboard picker-coordination package.
        ClipboardPickerCoordV3 = 12 => "d2b.clipboard.picker-coord.v3",
        /// The desktop notification Provider package.
        NotificationV3 = 13 => "d2b.notification.v3",
        /// The service-only NixOS configuration Provider package.
        ConfigNixosV3 = 14 => "d2b.config-nixos.v3",
    }
);

impl EndpointPurpose {
    /// Whether this purpose may be offered under `class`.
    ///
    /// Two rules hold. The bootstrap rule is preserved verbatim from the
    /// component-session endpoint-shape check: a bootstrap purpose requires
    /// the bootstrap class and no other purpose may claim it. The Zone-local
    /// rule is the v3 addition: a Zone-local endpoint is reached over an
    /// allocator-issued local socket and is therefore only ever local class.
    pub const fn permits_class(self, class: PurposeClass) -> bool {
        match self {
            Self::Bootstrap => matches!(class, PurposeClass::Bootstrap),
            Self::ZoneLocal => matches!(class, PurposeClass::Local),
            _ => !matches!(class, PurposeClass::Bootstrap),
        }
    }

    /// Lifts a component-session purpose into the Zone taxonomy.
    ///
    /// Total: every component-session purpose has a Zone counterpart at the
    /// same tag. The exhaustive match makes a new component-session variant a
    /// compile error here rather than a runtime panic.
    pub fn from_component_session(value: base::EndpointPurpose) -> Self {
        match value {
            base::EndpointPurpose::LocalLifecycle => Self::LocalLifecycle,
            base::EndpointPurpose::ResourceService => Self::ResourceService,
            base::EndpointPurpose::ZoneLink => Self::ZoneLink,
            base::EndpointPurpose::Bootstrap => Self::Bootstrap,
            base::EndpointPurpose::ComponentSession => Self::ComponentSession,
            base::EndpointPurpose::ResourceTransfer => Self::ResourceTransfer,
            base::EndpointPurpose::ProviderControl => Self::ProviderControl,
            base::EndpointPurpose::SensitiveCredential => Self::SensitiveCredential,
            base::EndpointPurpose::UserControl => Self::UserControl,
            base::EndpointPurpose::ControllerWatch => Self::ControllerWatch,
            base::EndpointPurpose::NamedStream => Self::NamedStream,
            base::EndpointPurpose::AuditExport => Self::AuditExport,
            base::EndpointPurpose::SupportBundle => Self::SupportBundle,
        }
    }

    /// Lowers this purpose into the component-session taxonomy.
    ///
    /// Partial: the appended Zone purposes have no component-session
    /// counterpart and return `None` rather than a nearest match.
    pub fn to_component_session(self) -> Option<base::EndpointPurpose> {
        base::EndpointPurpose::from_tag(self.tag()).ok()
    }
}

impl EndpointRole {
    /// Lifts a component-session role into the Zone taxonomy.
    ///
    /// Partial: the reserved component-session tags 7 and 8 are unassigned in
    /// v3 and return `None`.
    pub fn from_component_session(value: base::EndpointRole) -> Option<Self> {
        Self::from_tag(value.tag()).ok()
    }

    /// Lowers this role into the component-session taxonomy.
    ///
    /// Partial: the appended Zone roles have no component-session counterpart.
    pub fn to_component_session(self) -> Option<base::EndpointRole> {
        base::EndpointRole::from_tag(self.tag()).ok()
    }
}

impl ServicePackage {
    /// Lifts a component-session service package into the Zone taxonomy.
    ///
    /// Total: every component-session package has a Zone counterpart at the
    /// same tag. The exhaustive match makes a new component-session variant a
    /// compile error here rather than a runtime panic.
    pub fn from_component_session(value: base::ServicePackage) -> Self {
        match value {
            base::ServicePackage::ResourceV3 => Self::ResourceV3,
            base::ServicePackage::ControllerV3 => Self::ControllerV3,
            base::ServicePackage::ProviderV3 => Self::ProviderV3,
            base::ServicePackage::AuditV3 => Self::AuditV3,
            base::ServicePackage::SupportV3 => Self::SupportV3,
            base::ServicePackage::CredentialV3 => Self::CredentialV3,
            base::ServicePackage::DisplayV3 => Self::DisplayV3,
            base::ServicePackage::ClipboardV3 => Self::ClipboardV3,
            base::ServicePackage::ClipboardBridgeV3 => Self::ClipboardBridgeV3,
            base::ServicePackage::ClipboardPickerCoordV3 => Self::ClipboardPickerCoordV3,
            base::ServicePackage::NotificationV3 => Self::NotificationV3,
            base::ServicePackage::ConfigNixosV3 => Self::ConfigNixosV3,
        }
    }

    /// Lowers this service package into the component-session taxonomy.
    ///
    /// Partial: the appended Zone packages have no component-session
    /// counterpart. Use this to populate the `service` field of a re-exported
    /// [`AttachmentDescriptor`].
    pub fn to_component_session(self) -> Option<base::ServicePackage> {
        base::ServicePackage::from_tag(self.tag()).ok()
    }
}

// ---------------------------------------------------------------------------
// Zone enrollment control messages
// ---------------------------------------------------------------------------

/// Frozen wire method name of the one-time ZoneLink enrollment bootstrap.
///
/// The Zone service's own method inventory reports this constant rather than a
/// second copy of the spelling, so the service, the allocator, and a Guest
/// agent cannot drift apart on the method name.
pub const ZONE_BOOTSTRAP_METHOD: &str = "zone-bootstrap";

/// Frozen wire method name of the enrolled `Noise_KK` enrollment.
pub const ZONE_ENROLL_METHOD: &str = "zone-enroll";

/// Protocol marker carried by every Zone enrollment control payload.
pub const ZONE_ENROLLMENT_PROTOCOL: &str = "d2b-zone-enrollment-v1";

/// Largest encoded Zone enrollment control payload one endpoint accepts.
pub const MAX_ZONE_ENROLLMENT_PAYLOAD_BYTES: usize = 16 * 1024;

/// The exact link identity one Zone enrollment control payload names.
///
/// These are comparison inputs, never authority: the authority is the
/// runtime-issued admission the serving Zone holds. Naming the identity on the
/// wire is what lets a Guest agent's request be refused before any PSK or
/// enrollment record is touched when the allocator issued its admission for a
/// different link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ZoneEnrollmentIdentity {
    /// The committed ZoneLink resource identity this enrollment is for.
    pub zone_link_uid: ResourceUid,
    /// The immutable parent/child edge the link joins.
    pub edge: ZoneTreeEdge,
    /// The ZoneLink controller generation that authorized the link.
    pub controller_generation: ZoneLinkControllerGeneration,
    /// The link identity generation this enrollment is for.
    pub reconnect_generation: ReconnectGeneration,
    /// The compiled schema fingerprint of the session being enrolled.
    pub schema_fingerprint: [u8; 32],
}

impl ZoneEnrollmentIdentity {
    /// Whether the declared identity is structurally admissible.
    ///
    /// An all-zero schema fingerprint is refused: an uninitialised buffer can
    /// never become a fingerprint that later compares equal to another one.
    pub fn validate(&self) -> Result<(), ZoneEnrollmentRefusal> {
        if self.schema_fingerprint == [0; 32] {
            return Err(ZoneEnrollmentRefusal::MalformedRequest);
        }
        Ok(())
    }
}

/// One `zone-bootstrap` request: the link identity plus the allocator-issued
/// single-use PSK issuance descriptor it presents.
///
/// The PSK itself never appears here. An issuance is its ordinal and the
/// lifetime it was issued with, which is all the consuming state machine
/// needs and all the wire may carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ZoneBootstrapCall {
    protocol: String,
    /// The link identity being bootstrapped.
    pub identity: ZoneEnrollmentIdentity,
    /// The allocator's issuance ordinal for the presented PSK.
    pub issuance: u64,
    /// The declared lifetime of the presented PSK in milliseconds.
    pub ttl_ms: u64,
    /// When the allocator issued the presented PSK, in Unix milliseconds.
    pub issued_at_unix_ms: u64,
}

impl ZoneBootstrapCall {
    /// Build one bootstrap request.
    pub fn new(
        identity: ZoneEnrollmentIdentity,
        issuance: u64,
        ttl_ms: u64,
        issued_at_unix_ms: u64,
    ) -> Self {
        Self {
            protocol: ZONE_ENROLLMENT_PROTOCOL.to_owned(),
            identity,
            issuance,
            ttl_ms,
            issued_at_unix_ms,
        }
    }

    /// Encode the bounded request payload.
    pub fn encode(&self) -> Result<Vec<u8>, ZoneEnrollmentRefusal> {
        encode_enrollment_payload(self)
    }

    /// Decode one bounded request payload, refusing anything else.
    pub fn decode(bytes: &[u8]) -> Result<Self, ZoneEnrollmentRefusal> {
        let call: Self = decode_enrollment_payload(bytes)?;
        if call.protocol != ZONE_ENROLLMENT_PROTOCOL {
            return Err(ZoneEnrollmentRefusal::MalformedRequest);
        }
        call.identity.validate()?;
        if call.issuance == 0 || call.ttl_ms == 0 {
            return Err(ZoneEnrollmentRefusal::MalformedRequest);
        }
        Ok(call)
    }
}

/// One `zone-enroll` request: the link identity and the observed peer
/// static-key fingerprint.
///
/// Only what the peer itself observed crosses the wire. The fingerprint the
/// allocator sealed, and the opaque digest of the allocator enrollment that
/// authorized it, stay with the allocator: neither is something a Guest agent
/// is asked to echo back, and nothing here can be used to enroll a peer the
/// allocator did not pin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ZoneEnrollCall {
    protocol: String,
    /// The link identity being enrolled.
    pub identity: ZoneEnrollmentIdentity,
    /// The observed peer static-key fingerprint.
    pub observed_peer_fingerprint: [u8; 32],
    /// When the enrollment was completed, in Unix milliseconds.
    pub enrolled_at_unix_ms: u64,
}

impl ZoneEnrollCall {
    /// Build one enrollment request.
    pub fn new(
        identity: ZoneEnrollmentIdentity,
        observed_peer_fingerprint: [u8; 32],
        enrolled_at_unix_ms: u64,
    ) -> Self {
        Self {
            protocol: ZONE_ENROLLMENT_PROTOCOL.to_owned(),
            identity,
            observed_peer_fingerprint,
            enrolled_at_unix_ms,
        }
    }

    /// Encode the bounded request payload.
    pub fn encode(&self) -> Result<Vec<u8>, ZoneEnrollmentRefusal> {
        encode_enrollment_payload(self)
    }

    /// Decode one bounded request payload, refusing anything else.
    pub fn decode(bytes: &[u8]) -> Result<Self, ZoneEnrollmentRefusal> {
        let call: Self = decode_enrollment_payload(bytes)?;
        if call.protocol != ZONE_ENROLLMENT_PROTOCOL {
            return Err(ZoneEnrollmentRefusal::MalformedRequest);
        }
        call.identity.validate()?;
        if call.observed_peer_fingerprint == [0; 32] {
            return Err(ZoneEnrollmentRefusal::MalformedRequest);
        }
        Ok(call)
    }
}

/// The closed reason one Zone enrollment control request was refused.
///
/// Every variant is a stable lower-kebab label. The label set is the union of
/// the service's own admission refusals and the ZoneLink enrollment state
/// machine's refusals, so a Guest agent learns exactly which rule refused it
/// without learning any identity, key, path, or store fact.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneEnrollmentRefusal {
    /// No runtime-issued admission was presented.
    AdmissionAbsent,
    /// The presented admission was already consumed.
    AdmissionConsumed,
    /// The presented admission is past its validity.
    AdmissionExpired,
    /// Policy refused the request.
    PolicyDenial,
    /// The named edge is not one this Zone's sealed topology contains.
    UnsealedZoneLink,
    /// The request's identity is not the identity the admission was issued
    /// for.
    IdentityMismatch,
    /// The session profile is not the enrolled Guest-local carriage profile.
    SessionProfileRefused,
    /// The presented PSK issuance was already burned or superseded.
    BootstrapPskConsumed,
    /// The presented PSK is past its absolute expiry.
    BootstrapPskExpired,
    /// The IKpsk2 bootstrap handshake failed. The PSK stays burned.
    BootstrapHandshakeFailed,
    /// The peer's static key does not match the sealed enrollment.
    ZoneLinkEnrollmentKeyMismatch,
    /// The sealed enrollment is durably invalidated.
    ZoneLinkRevoked,
    /// The transition is not defined from the current state.
    InvalidTransition,
    /// The declared PSK lifetime is outside the frozen range.
    BootstrapPskTtlOutOfRange,
    /// The declared enrolled-session lifetime is outside the frozen range.
    KkSessionLifetimeOutOfRange,
    /// A link epoch must be nonzero and must not wrap.
    LinkEpochExhausted,
    /// Resource traffic was offered before the link reached `Ready`.
    ResourceTrafficBeforeReady,
    /// The request payload is malformed, truncated, or names a zero digest.
    MalformedRequest,
    /// The request payload exceeds the bounded enrollment payload size.
    PayloadTooLarge,
}

impl ZoneEnrollmentRefusal {
    /// Every variant, in declaration order.
    pub const ALL: &'static [Self] = &[
        Self::AdmissionAbsent,
        Self::AdmissionConsumed,
        Self::AdmissionExpired,
        Self::PolicyDenial,
        Self::UnsealedZoneLink,
        Self::IdentityMismatch,
        Self::SessionProfileRefused,
        Self::BootstrapPskConsumed,
        Self::BootstrapPskExpired,
        Self::BootstrapHandshakeFailed,
        Self::ZoneLinkEnrollmentKeyMismatch,
        Self::ZoneLinkRevoked,
        Self::InvalidTransition,
        Self::BootstrapPskTtlOutOfRange,
        Self::KkSessionLifetimeOutOfRange,
        Self::LinkEpochExhausted,
        Self::ResourceTrafficBeforeReady,
        Self::MalformedRequest,
        Self::PayloadTooLarge,
    ];

    /// The stable lower-kebab label of this refusal.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AdmissionAbsent => "admission-absent",
            Self::AdmissionConsumed => "admission-consumed",
            Self::AdmissionExpired => "admission-expired",
            Self::PolicyDenial => "policy-denial",
            Self::UnsealedZoneLink => "unsealed-zone-link",
            Self::IdentityMismatch => "identity-mismatch",
            Self::SessionProfileRefused => "session-profile-refused",
            Self::BootstrapPskConsumed => "bootstrap-psk-consumed",
            Self::BootstrapPskExpired => "bootstrap-psk-expired",
            Self::BootstrapHandshakeFailed => "bootstrap-handshake-failed",
            Self::ZoneLinkEnrollmentKeyMismatch => "zone-link-enrollment-key-mismatch",
            Self::ZoneLinkRevoked => "zone-link-revoked",
            Self::InvalidTransition => "invalid-transition",
            Self::BootstrapPskTtlOutOfRange => "bootstrap-psk-ttl-out-of-range",
            Self::KkSessionLifetimeOutOfRange => "kk-session-lifetime-out-of-range",
            Self::LinkEpochExhausted => "link-epoch-exhausted",
            Self::ResourceTrafficBeforeReady => "resource-traffic-before-ready",
            Self::MalformedRequest => "malformed-request",
            Self::PayloadTooLarge => "payload-too-large",
        }
    }
}

impl fmt::Display for ZoneEnrollmentRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::error::Error for ZoneEnrollmentRefusal {}

/// The answer to one `zone-bootstrap` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(
    deny_unknown_fields,
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ZoneBootstrapReply {
    /// The bootstrap was admitted and the presented PSK is burned.
    Admitted {
        /// The absolute expiry of the consumed issuance, in Unix milliseconds.
        expires_at_unix_ms: u64,
    },
    /// The bootstrap was refused for one closed reason.
    Refused {
        /// The closed refusal reason.
        reason: ZoneEnrollmentRefusal,
    },
}

impl ZoneBootstrapReply {
    /// Encode the bounded reply payload.
    pub fn encode(&self) -> Result<Vec<u8>, ZoneEnrollmentRefusal> {
        encode_enrollment_payload(self)
    }

    /// Decode one bounded reply payload, refusing anything else.
    pub fn decode(bytes: &[u8]) -> Result<Self, ZoneEnrollmentRefusal> {
        decode_enrollment_payload(bytes)
    }
}

/// The answer to one `zone-enroll` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(
    deny_unknown_fields,
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ZoneEnrollReply {
    /// The enrollment committed and the enrolled session reached `Ready`.
    Enrolled {
        /// The Zone the allocator placed this agent in.
        zone: ZoneId,
        /// The link epoch the established session was assigned.
        generation: u64,
    },
    /// The enrollment was refused for one closed reason.
    Refused {
        /// The closed refusal reason.
        reason: ZoneEnrollmentRefusal,
    },
}

impl ZoneEnrollReply {
    /// Encode the bounded reply payload.
    pub fn encode(&self) -> Result<Vec<u8>, ZoneEnrollmentRefusal> {
        encode_enrollment_payload(self)
    }

    /// Decode one bounded reply payload, refusing anything else.
    pub fn decode(bytes: &[u8]) -> Result<Self, ZoneEnrollmentRefusal> {
        let reply: Self = decode_enrollment_payload(bytes)?;
        if let Self::Enrolled { generation, .. } = &reply
            && *generation == 0
        {
            return Err(ZoneEnrollmentRefusal::MalformedRequest);
        }
        Ok(reply)
    }
}

fn encode_enrollment_payload<T: Serialize>(value: &T) -> Result<Vec<u8>, ZoneEnrollmentRefusal> {
    let bytes = serde_json::to_vec(value).map_err(|_| ZoneEnrollmentRefusal::MalformedRequest)?;
    if bytes.len() > MAX_ZONE_ENROLLMENT_PAYLOAD_BYTES {
        return Err(ZoneEnrollmentRefusal::PayloadTooLarge);
    }
    Ok(bytes)
}

fn decode_enrollment_payload<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, ZoneEnrollmentRefusal> {
    if bytes.is_empty() {
        return Err(ZoneEnrollmentRefusal::MalformedRequest);
    }
    if bytes.len() > MAX_ZONE_ENROLLMENT_PAYLOAD_BYTES {
        return Err(ZoneEnrollmentRefusal::PayloadTooLarge);
    }
    serde_json::from_slice(bytes).map_err(|_| ZoneEnrollmentRefusal::MalformedRequest)
}

#[cfg(test)]
mod tests {
    use super::super::zone_routing::{ZoneLabelId, ZonePath};
    use super::*;

    /// Golden canonical wire vector: every variant, its frozen tag, and its
    /// frozen wire string. A change to any row is a wire break.
    const PURPOSE_VECTORS: &[(EndpointPurpose, u8, &str)] = &[
        (EndpointPurpose::LocalLifecycle, 1, "local-lifecycle"),
        (EndpointPurpose::ResourceService, 2, "resource-service"),
        (EndpointPurpose::ZoneLink, 3, "zone-link"),
        (EndpointPurpose::Bootstrap, 4, "bootstrap"),
        (EndpointPurpose::ComponentSession, 5, "component-session"),
        (EndpointPurpose::ResourceTransfer, 6, "resource-transfer"),
        (EndpointPurpose::ProviderControl, 7, "provider-control"),
        (
            EndpointPurpose::SensitiveCredential,
            8,
            "sensitive-credential",
        ),
        (EndpointPurpose::UserControl, 9, "user-control"),
        (EndpointPurpose::ControllerWatch, 10, "controller-watch"),
        (EndpointPurpose::NamedStream, 11, "named-stream"),
        (EndpointPurpose::AuditExport, 12, "audit-export"),
        (EndpointPurpose::SupportBundle, 13, "support-bundle"),
        (EndpointPurpose::ZoneLocal, 14, "zone-local"),
        (EndpointPurpose::ZoneControl, 15, "zone-control"),
    ];

    const ROLE_VECTORS: &[(EndpointRole, u8, &str)] = &[
        (EndpointRole::Component, 1, "component"),
        (EndpointRole::ZoneController, 2, "zone-controller"),
        (EndpointRole::HostAgent, 3, "host-agent"),
        (EndpointRole::GuestAgent, 4, "guest-agent"),
        (EndpointRole::Provider, 5, "provider"),
        (EndpointRole::UserAgent, 6, "user-agent"),
        (EndpointRole::ZoneRelay, 9, "zone-relay"),
        (EndpointRole::ZoneBootstrap, 10, "zone-bootstrap"),
    ];

    const SERVICE_VECTORS: &[(ServicePackage, u8, &str)] = &[
        (ServicePackage::ResourceV3, 1, "d2b.resource.v3"),
        (ServicePackage::ControllerV3, 2, "d2b.controller.v3"),
        (ServicePackage::ProviderV3, 3, "d2b.provider.v3"),
        (ServicePackage::AuditV3, 4, "d2b.audit.v3"),
        (ServicePackage::SupportV3, 5, "d2b.support.v3"),
        (ServicePackage::CredentialV3, 6, "d2b.credential.v3"),
        (ServicePackage::ZoneV3, 7, "d2b.zone.v3"),
        (ServicePackage::ZoneLinkV3, 8, "d2b.zonelink.v3"),
        (ServicePackage::DisplayV3, 9, "d2b.display.v3"),
        (ServicePackage::ClipboardV3, 10, "d2b.clipboard.v3"),
        (
            ServicePackage::ClipboardBridgeV3,
            11,
            "d2b.clipboard.bridge.v3",
        ),
        (
            ServicePackage::ClipboardPickerCoordV3,
            12,
            "d2b.clipboard.picker-coord.v3",
        ),
        (ServicePackage::NotificationV3, 13, "d2b.notification.v3"),
        (ServicePackage::ConfigNixosV3, 14, "d2b.config-nixos.v3"),
    ];

    #[test]
    fn frozen_tag_and_wire_string_vectors_are_exact() {
        assert_eq!(PURPOSE_VECTORS.len(), EndpointPurpose::ALL.len());
        for (index, (value, tag, wire)) in PURPOSE_VECTORS.iter().enumerate() {
            assert_eq!(EndpointPurpose::ALL[index], *value);
            assert_eq!(value.tag(), *tag);
            assert_eq!(value.as_str(), *wire);
        }

        assert_eq!(ROLE_VECTORS.len(), EndpointRole::ALL.len());
        for (index, (value, tag, wire)) in ROLE_VECTORS.iter().enumerate() {
            assert_eq!(EndpointRole::ALL[index], *value);
            assert_eq!(value.tag(), *tag);
            assert_eq!(value.as_str(), *wire);
        }

        assert_eq!(SERVICE_VECTORS.len(), ServicePackage::ALL.len());
        for (index, (value, tag, wire)) in SERVICE_VECTORS.iter().enumerate() {
            assert_eq!(ServicePackage::ALL[index], *value);
            assert_eq!(value.tag(), *tag);
            assert_eq!(value.as_str(), *wire);
        }
    }

    #[test]
    fn every_tag_round_trips_and_is_unique() {
        let mut purpose_tags: Vec<u8> = EndpointPurpose::ALL.iter().map(|v| v.tag()).collect();
        for value in EndpointPurpose::ALL {
            assert_eq!(EndpointPurpose::from_tag(value.tag()), Ok(*value));
        }
        purpose_tags.sort_unstable();
        purpose_tags.dedup();
        assert_eq!(purpose_tags.len(), EndpointPurpose::ALL.len());

        let mut role_tags: Vec<u8> = EndpointRole::ALL.iter().map(|v| v.tag()).collect();
        for value in EndpointRole::ALL {
            assert_eq!(EndpointRole::from_tag(value.tag()), Ok(*value));
        }
        role_tags.sort_unstable();
        role_tags.dedup();
        assert_eq!(role_tags.len(), EndpointRole::ALL.len());

        let mut service_tags: Vec<u8> = ServicePackage::ALL.iter().map(|v| v.tag()).collect();
        for value in ServicePackage::ALL {
            assert_eq!(ServicePackage::from_tag(value.tag()), Ok(*value));
        }
        service_tags.sort_unstable();
        service_tags.dedup();
        assert_eq!(service_tags.len(), ServicePackage::ALL.len());
    }

    #[test]
    fn wire_strings_are_unique_within_each_enumeration() {
        let mut purposes: Vec<&str> = EndpointPurpose::ALL.iter().map(|v| v.as_str()).collect();
        purposes.sort_unstable();
        purposes.dedup();
        assert_eq!(purposes.len(), EndpointPurpose::ALL.len());

        let mut roles: Vec<&str> = EndpointRole::ALL.iter().map(|v| v.as_str()).collect();
        roles.sort_unstable();
        roles.dedup();
        assert_eq!(roles.len(), EndpointRole::ALL.len());

        let mut services: Vec<&str> = ServicePackage::ALL.iter().map(|v| v.as_str()).collect();
        services.sort_unstable();
        services.dedup();
        assert_eq!(services.len(), ServicePackage::ALL.len());
    }

    #[test]
    fn unassigned_and_reserved_tags_fail_closed() {
        assert_eq!(
            EndpointPurpose::from_tag(0),
            Err(BinaryError::UnknownEnumTag)
        );
        assert_eq!(
            EndpointPurpose::from_tag(16),
            Err(BinaryError::UnknownEnumTag)
        );
        assert_eq!(
            EndpointPurpose::from_tag(255),
            Err(BinaryError::UnknownEnumTag)
        );

        // Tags 7 and 8 held the retired generic relay and bootstrapper roles.
        // They are permanently reserved and must never decode.
        assert_eq!(EndpointRole::from_tag(7), Err(BinaryError::UnknownEnumTag));
        assert_eq!(EndpointRole::from_tag(8), Err(BinaryError::UnknownEnumTag));
        assert_eq!(EndpointRole::from_tag(0), Err(BinaryError::UnknownEnumTag));
        assert_eq!(EndpointRole::from_tag(11), Err(BinaryError::UnknownEnumTag));

        assert_eq!(
            ServicePackage::from_tag(0),
            Err(BinaryError::UnknownEnumTag)
        );
        assert_eq!(
            ServicePackage::from_tag(15),
            Err(BinaryError::UnknownEnumTag)
        );
    }

    #[test]
    fn serde_round_trips_through_the_frozen_wire_string() {
        for (value, _, wire) in PURPOSE_VECTORS {
            let json = serde_json::to_string(value).expect("serialize");
            assert_eq!(json, format!("\"{wire}\""));
            let parsed: EndpointPurpose = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(parsed, *value);
        }
        for (value, _, wire) in ROLE_VECTORS {
            let json = serde_json::to_string(value).expect("serialize");
            assert_eq!(json, format!("\"{wire}\""));
            let parsed: EndpointRole = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(parsed, *value);
        }
        for (value, _, wire) in SERVICE_VECTORS {
            let json = serde_json::to_string(value).expect("serialize");
            assert_eq!(json, format!("\"{wire}\""));
            let parsed: ServicePackage = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(parsed, *value);
        }
    }

    #[test]
    fn unknown_wire_strings_are_rejected() {
        assert!(serde_json::from_str::<EndpointPurpose>("\"zone-admin\"").is_err());
        assert!(serde_json::from_str::<EndpointRole>("\"relay\"").is_err());
        assert!(serde_json::from_str::<EndpointRole>("\"bootstrapper\"").is_err());
        assert!(serde_json::from_str::<ServicePackage>("\"d2b.realm.v2\"").is_err());
    }

    #[test]
    fn purpose_class_admission_is_fail_closed_on_bootstrap_and_zone_local() {
        assert!(EndpointPurpose::Bootstrap.permits_class(PurposeClass::Bootstrap));
        assert!(!EndpointPurpose::Bootstrap.permits_class(PurposeClass::Local));
        assert!(!EndpointPurpose::Bootstrap.permits_class(PurposeClass::Enrolled));

        assert!(EndpointPurpose::ZoneLocal.permits_class(PurposeClass::Local));
        assert!(!EndpointPurpose::ZoneLocal.permits_class(PurposeClass::Enrolled));
        assert!(!EndpointPurpose::ZoneLocal.permits_class(PurposeClass::Bootstrap));

        assert!(EndpointPurpose::ZoneLink.permits_class(PurposeClass::Enrolled));
        assert!(!EndpointPurpose::ZoneLink.permits_class(PurposeClass::Bootstrap));

        for value in EndpointPurpose::ALL {
            assert_eq!(
                value.permits_class(PurposeClass::Bootstrap),
                *value == EndpointPurpose::Bootstrap,
                "only the bootstrap purpose may claim the bootstrap class"
            );
        }
    }

    #[test]
    fn component_session_lifts_are_total_and_lowers_are_partial() {
        for value in base::EndpointPurpose::ALL {
            let lifted = EndpointPurpose::from_component_session(*value);
            assert_eq!(lifted.tag(), value.tag());
            assert_eq!(lifted.as_str(), value.as_str());
            assert_eq!(lifted.to_component_session(), Some(*value));
        }
        assert_eq!(EndpointPurpose::ZoneLocal.to_component_session(), None);
        assert_eq!(EndpointPurpose::ZoneControl.to_component_session(), None);

        for value in base::ServicePackage::ALL {
            let lifted = ServicePackage::from_component_session(*value);
            assert_eq!(lifted.tag(), value.tag());
            assert_eq!(lifted.as_str(), value.as_str());
            assert_eq!(lifted.to_component_session(), Some(*value));
        }
        assert_eq!(ServicePackage::ZoneV3.to_component_session(), None);
        assert_eq!(ServicePackage::ZoneLinkV3.to_component_session(), None);

        for value in base::EndpointRole::ALL {
            match EndpointRole::from_component_session(*value) {
                Some(lifted) => {
                    assert_eq!(lifted.tag(), value.tag());
                    assert_eq!(lifted.as_str(), value.as_str());
                    assert_eq!(lifted.to_component_session(), Some(*value));
                }
                None => assert!(
                    matches!(value.tag(), 7 | 8),
                    "only the reserved tags may fail to lift"
                ),
            }
        }
        assert_eq!(EndpointRole::ZoneRelay.to_component_session(), None);
        assert_eq!(EndpointRole::ZoneBootstrap.to_component_session(), None);
    }

    #[test]
    fn debug_output_is_the_variant_name_only() {
        // These enumerations are field-free, so their derived Debug can never
        // echo a path, a credential, a uid, or caller-supplied text.
        assert_eq!(format!("{:?}", EndpointPurpose::ZoneLocal), "ZoneLocal");
        assert_eq!(format!("{:?}", EndpointRole::ZoneRelay), "ZoneRelay");
        assert_eq!(format!("{:?}", ServicePackage::ZoneLinkV3), "ZoneLinkV3");
    }

    #[test]
    fn guest_session_credential_surface_is_absent() {
        // The v3 ZoneLink session contract carries no guest bootstrap
        // credential. This test pins the absence by asserting the closed
        // service-package set never names one.
        for value in ServicePackage::ALL {
            assert!(!value.as_str().contains("guest"));
        }
        for value in EndpointPurpose::ALL {
            assert!(!value.as_str().contains("guest-bootstrap"));
        }
    }

    // -- Zone enrollment control messages --------------------------------

    fn enrollment_identity() -> ZoneEnrollmentIdentity {
        ZoneEnrollmentIdentity {
            zone_link_uid: ResourceUid::parse("11111111-1111-4111-8111-111111111111")
                .expect("valid UID"),
            edge: ZoneTreeEdge::new(
                ZonePath::new(vec![ZoneLabelId::parse("k0").expect("label")]).expect("zone"),
                ZonePath::new(vec![
                    ZoneLabelId::parse("k1").expect("label"),
                    ZoneLabelId::parse("k0").expect("label"),
                ])
                .expect("zone"),
            )
            .expect("direct child edge"),
            controller_generation: ZoneLinkControllerGeneration::parse("controller-1")
                .expect("valid generation"),
            reconnect_generation: ReconnectGeneration::new(7).expect("valid generation"),
            schema_fingerprint: [0x2a; 32],
        }
    }

    #[test]
    fn enrollment_payloads_round_trip_within_the_bound() {
        let bootstrap =
            ZoneBootstrapCall::new(enrollment_identity(), 3, 300_000, 1_700_000_000_000);
        let encoded = bootstrap.encode().expect("encodes");
        assert!(encoded.len() <= MAX_ZONE_ENROLLMENT_PAYLOAD_BYTES);
        assert_eq!(ZoneBootstrapCall::decode(&encoded), Ok(bootstrap));

        let enroll = ZoneEnrollCall::new(enrollment_identity(), [0x11; 32], 1_700_000_000_100);
        let encoded = enroll.encode().expect("encodes");
        assert_eq!(ZoneEnrollCall::decode(&encoded), Ok(enroll));

        let admitted = ZoneBootstrapReply::Admitted {
            expires_at_unix_ms: 1_700_000_300_000,
        };
        let encoded = admitted.encode().expect("encodes");
        assert_eq!(ZoneBootstrapReply::decode(&encoded), Ok(admitted));

        let enrolled = ZoneEnrollReply::Enrolled {
            zone: ZoneId::parse("zone-1").expect("valid zone"),
            generation: 1,
        };
        let encoded = enrolled.encode().expect("encodes");
        assert_eq!(ZoneEnrollReply::decode(&encoded), Ok(enrolled));

        let refused = ZoneEnrollReply::Refused {
            reason: ZoneEnrollmentRefusal::BootstrapPskConsumed,
        };
        let encoded = refused.encode().expect("encodes");
        assert_eq!(ZoneEnrollReply::decode(&encoded), Ok(refused));
    }

    #[test]
    fn enrollment_payload_refusals_are_closed_and_bounded() {
        let bootstrap = ZoneBootstrapCall::new(enrollment_identity(), 3, 300_000, 0);
        let encoded = bootstrap.encode().expect("encodes");

        // Truncated, empty, oversize, and unknown-member payloads are all
        // refused, and a zero-digest identity never decodes.
        assert_eq!(
            ZoneBootstrapCall::decode(&[]),
            Err(ZoneEnrollmentRefusal::MalformedRequest)
        );
        assert_eq!(
            ZoneBootstrapCall::decode(&encoded[..encoded.len() - 1]),
            Err(ZoneEnrollmentRefusal::MalformedRequest)
        );
        let oversize = vec![b' '; MAX_ZONE_ENROLLMENT_PAYLOAD_BYTES + 1];
        assert_eq!(
            ZoneBootstrapCall::decode(&oversize),
            Err(ZoneEnrollmentRefusal::PayloadTooLarge)
        );
        assert_eq!(
            ZoneBootstrapCall::decode(br#"{"protocol":"d2b-zone-enrollment-v1"}"#),
            Err(ZoneEnrollmentRefusal::MalformedRequest)
        );

        let mut zero_digest = enrollment_identity();
        zero_digest.schema_fingerprint = [0; 32];
        let call = ZoneBootstrapCall::new(zero_digest, 3, 300_000, 0);
        let encoded = call.encode().expect("encodes");
        assert_eq!(
            ZoneBootstrapCall::decode(&encoded),
            Err(ZoneEnrollmentRefusal::MalformedRequest)
        );

        let zero_issuance = ZoneBootstrapCall::new(enrollment_identity(), 0, 300_000, 0);
        assert_eq!(
            ZoneBootstrapCall::decode(&zero_issuance.encode().expect("encodes")),
            Err(ZoneEnrollmentRefusal::MalformedRequest)
        );

        let mut reply = ZoneEnrollReply::Enrolled {
            zone: ZoneId::parse("zone-1").expect("valid zone"),
            generation: 0,
        }
        .encode()
        .expect("encodes");
        assert_eq!(
            ZoneEnrollReply::decode(&reply),
            Err(ZoneEnrollmentRefusal::MalformedRequest)
        );
        reply.clear();
        assert_eq!(
            ZoneEnrollReply::decode(&reply),
            Err(ZoneEnrollmentRefusal::MalformedRequest)
        );
    }

    #[test]
    fn enrollment_refusal_labels_are_unique_and_stable() {
        let mut labels: Vec<&str> = ZoneEnrollmentRefusal::ALL
            .iter()
            .map(|reason| reason.as_str())
            .collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), ZoneEnrollmentRefusal::ALL.len());
        for reason in ZoneEnrollmentRefusal::ALL {
            assert_eq!(reason.as_str(), reason.as_str().to_ascii_lowercase());
            assert!(!reason.as_str().contains('_'));
            let json = serde_json::to_string(reason).expect("serialize");
            assert_eq!(json, format!("\"{}\"", reason.as_str()));
            assert_eq!(
                serde_json::from_str::<ZoneEnrollmentRefusal>(&json).expect("deserialize"),
                *reason
            );
        }
        assert_eq!(
            ZoneEnrollmentRefusal::ZoneLinkEnrollmentKeyMismatch.as_str(),
            "zone-link-enrollment-key-mismatch"
        );
        assert_eq!(
            serde_json::to_string(&ZoneEnrollmentRefusal::PolicyDenial).expect("serialize"),
            "\"policy-denial\""
        );
    }
}
