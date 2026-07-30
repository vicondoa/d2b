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
//! inferences pending panel confirmation:
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

use serde::{Deserialize, Serialize};

use super::component_session as base;

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
        /// Guest control traffic.
        GuestControl = 5 => "guest-control",
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
    /// Tags 7 and 8 are the appended Zone service packages. Protobuf field
    /// numbers for the v3 services are frozen independently of the v2
    /// assignments and are not restated by this module.
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
    /// same tag.
    pub fn from_component_session(value: base::EndpointPurpose) -> Self {
        Self::from_tag(value.tag()).expect("preserved component-session tag")
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
    /// same tag.
    pub fn from_component_session(value: base::ServicePackage) -> Self {
        Self::from_tag(value.tag()).expect("preserved component-session tag")
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden canonical wire vector: every variant, its frozen tag, and its
    /// frozen wire string. A change to any row is a wire break.
    const PURPOSE_VECTORS: &[(EndpointPurpose, u8, &str)] = &[
        (EndpointPurpose::LocalLifecycle, 1, "local-lifecycle"),
        (EndpointPurpose::ResourceService, 2, "resource-service"),
        (EndpointPurpose::ZoneLink, 3, "zone-link"),
        (EndpointPurpose::Bootstrap, 4, "bootstrap"),
        (EndpointPurpose::GuestControl, 5, "guest-control"),
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
            ServicePackage::from_tag(9),
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
}
