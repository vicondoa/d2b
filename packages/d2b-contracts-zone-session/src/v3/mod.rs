//! Canonical Zone and ComponentSession contract family.

/// Captures the crate's Wire-struct `Deserialize` shape: a private
/// `#[derive(Deserialize)]` wire struct with `deny_unknown_fields`, then a
/// validated constructor admission check.
///
/// The binding name is passed explicitly (`wire` at every site) so the
/// construct expression can reference it across macro hygiene.
macro_rules! wire_deserialize {
    (
        $type:ty,
        $(#[$container:meta])*
        $wire:ident {
            $( $(#[$field_attr:meta])* $field:ident : $field_ty:ty ),* $(,)?
        },
        $binding:ident,
        $construct:expr
    ) => {
        impl<'de> serde::Deserialize<'de> for $type {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                #[derive(serde::Deserialize)]
                $(#[$container])*
                struct $wire {
                    $( $(#[$field_attr])* $field: $field_ty, )*
                }
                let $binding = $wire::deserialize(deserializer)?;
                $construct
            }
        }
    };
}

pub mod component_session;
pub mod emergency_policy;
pub mod resource_bundle;
pub mod resource_export;
pub mod resource_import;
pub mod role;
pub mod role_binding;
pub mod services;
pub mod zone;
pub mod zone_link;
pub mod zone_routing;
pub mod zone_session;

pub use component_session::{
    AdmittedDeadline, AttachmentAccess, AttachmentCreditClass, AttachmentCredits,
    AttachmentDescriptor, AttachmentKind, AttachmentPacket, AttachmentPolicy, AttachmentPolicyKind,
    AttachmentPurpose, AttachmentReceiveError, AuthorizationLease, BinaryError,
    BootstrapIdentityBinding, BootstrapPskBinding, BoundedVec, COMPONENT_SESSION_MAJOR,
    COMPONENT_SESSION_MINOR, CancelAck, CancelRequest, CancelResult, ChannelClass, ChannelId,
    CloseReason, CloseRecord, ComponentSessionBoundary, ComponentSessionDescriptor,
    ComponentSessionPreface, ContractError, ENDPOINT_POLICY_IDENTITY_CANONICAL_LEN, EndpointPolicy,
    EndpointPolicyIdentity, EndpointPurpose, EndpointRole, FRAGMENT_HEADER_LEN, FragmentHeader,
    FragmentSequence, FragmentSequenceError, HANDSHAKE_OFFER_CANONICAL_LEN, HandshakeAccept,
    HandshakeOffer, HandshakeReject, HandshakeRejectReason, HealthState,
    IdentityEvidenceRequirement, KeepaliveRecord, KernelObjectType, LOCAL_HANDSHAKE_DEADLINE_MS,
    LOCAL_RECONNECT_DEADLINE_MS, LimitProfile, MAX_ACTIVE_NAMED_STREAMS,
    MAX_AGGREGATE_NAMED_STREAM_QUEUE_BYTES, MAX_CLOCK_SKEW_MS, MAX_HANDSHAKE_OFFER_BYTES,
    MAX_HOST_ATTACHMENT_CREDITS, MAX_ID_BYTES, MAX_KEEPALIVE_INTERVAL_MS, MAX_KEEPALIVE_TIMEOUT_MS,
    MAX_LOGICAL_MESSAGE_BYTES, MAX_NAMED_STREAM_QUEUE_BYTES, MAX_OPERATION_ATTACHMENTS,
    MAX_PACKET_ATTACHMENTS, MAX_PROCESS_ATTACHMENT_CREDITS, MAX_PROTECTED_CIPHERTEXT_BYTES,
    MAX_PROTECTED_PLAINTEXT_BYTES, MAX_RECONNECT_ATTEMPTS, MAX_RECONNECT_WINDOW_MS,
    MAX_REQUEST_ATTACHMENTS, MAX_REQUEST_LIFETIME_MS, MAX_SESSION_ATTACHMENTS,
    MAX_SESSION_CONTROL_QUEUE_BYTES, MAX_TTRPC_CONTROL_QUEUE_BYTES, MetricLabels, MetricReason,
    MetricResult, NOISE_TAG_BYTES, NoiseProfile, OperationClass, PREFACE_LEN, PREFACE_MAGIC,
    PrefaceError, PurposeClass, RECORD_HEADER_LEN, RECORD_LENGTH_BYTES,
    REMOTE_HANDSHAKE_DEADLINE_MS, REMOTE_RECONNECT_DEADLINE_MS, RESERVED_CONTROL_FDS,
    ReceiveSequence, RecordHeader, RecordKind, Remediation, RequestEnvelope, SendSequence,
    SequenceError, ServicePackage, SessionErrorCode, TransportClass,
};
pub use emergency_policy::*;
pub use resource_export::*;
pub use resource_import::*;
pub use role::{
    PrincipalRef, RoleConditionType, RoleMount, RoleMountPath, RoleNamespaces, RolePosture,
    RoleResourceVerb, RoleRule, RoleSessionVerb, RoleSpec,
};
pub use role_binding::{
    BINDABLE_SUBJECT_TYPES, ExternalPrincipalSelector, RelayAuthority, RoleBindingConditionType,
    RoleBindingSpec, ScopeNarrowing,
};
pub use services::*;
pub use zone::*;
pub use zone_link::*;
pub use zone_routing::*;
pub use zone_session::*;

// The service-to-provider catalog is generated from the per-crate
// `service-catalog.json` declarations;the layout check's parity and drift
// gates own it (`cargo xtask check-provider-crate-layout`),and `--fix`
// regenerates it.

include!("../generated/service_provider_catalog.rs");
