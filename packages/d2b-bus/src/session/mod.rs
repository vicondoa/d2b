//! Zone bus session handling: the v3 ComponentSession runtime as the bus uses
//! it, plus the ZoneLink enrollment and session state machine.
//!
//! # What is ported and how
//!
//! The work item's detailed design says to carry the ComponentSession runtime
//! into this crate "keeping the Noise profiles (Nn/Kk/IKpsk2), generation
//! discovery, record/fragment/keepalive/credit/cancellation/attachment logic
//! verbatim". Every one of those surfaces is re-exported from the audited
//! implementation rather than copied into a second set of source files.
//!
//! This is the same spelling `d2b_contracts::v3::zone_session` chose for the
//! wire contract, and for the same reasons. A byte-exact copy of a Noise
//! driver is not more verbatim than the original; it is a second definition
//! that can drift, a second thing to fuzz, and it silently invalidates the
//! golden vectors already proved against the first. Re-exporting keeps exactly
//! one definition of the handshake transcript, the record header, the replay
//! window, the fragmenter, the fair scheduler, and the named-stream credit
//! accounting. Nothing is forked, and nothing is re-derived.
//!
//! Two surfaces named by the work item are deliberately *not* re-exported:
//!
//! - `serve_ttrpc_services`, the fixed-endpoint ttrpc binding. A v3 Zone
//!   transport descriptor arrives from the allocator through
//!   [`crate::transport::unix`], which consults no activation protocol at all.
//!   Re-exporting a fixed-endpoint binder from here would put a second,
//!   unaudited way to acquire a transport back into the crate.
//! - The guest-session credential types. They are absent from the v3 contract
//!   and this module declares no replacement.
//!
//! # The Zone taxonomy, and why the offer types are local
//!
//! [`crate::session::contract`] defines Zone-typed endpoint policy over the
//! extended `zone_session` enumerations, and lowers fail-closed into the
//! component-session policy the handshake encoder consumes. The alternative -
//! widening `component_session`'s own `HandshakeOffer` and `EndpointPolicy`
//! fields to the Zone enumerations - is a change to a frozen canonical byte
//! encoding with committed golden vectors, in a file this work item does not
//! own. See that module for the full argument and the resulting gap.
//!
//! # No authority is minted here
//!
//! Nothing in this module resolves a subject, admits a peer, or mints a
//! capability. [`crate::session::prologue`] digests an *already authenticated*
//! subject context that the registrar resolved; it borrows and never stores
//! one, constructs none, and accepts no caller-supplied subject reference,
//! uid, or claim. [`crate::session::zone_link`] holds a driver handle and an
//! enrollment state, never admission evidence. The sealed `SessionAuthority`
//! supertrait, the single-owner admission consumption, and the registrar's
//! exclusive ownership of subject resolution are untouched: this module adds
//! no implementation of any of them, no clone of an admission, and no
//! accessor that would let a caller reuse one.

pub mod contract;
pub mod enrollment;
pub mod prologue;
pub mod zone_link;

#[cfg(test)]
mod noise_vectors;

pub use contract::{ZoneEndpointPolicy, ZoneEndpointPolicyIdentity, ZonePolicyError};
pub use enrollment::{
    BOOTSTRAP_PSK_TTL_MS_DEFAULT, BOOTSTRAP_PSK_TTL_MS_MAX, BOOTSTRAP_PSK_TTL_MS_MIN,
    BootstrapPskIssuance, EnrollmentFingerprint, EnrollmentRecord,
    KK_SESSION_MAX_LIFETIME_MS_DEFAULT, KK_SESSION_MAX_LIFETIME_MS_MAX,
    KK_SESSION_MAX_LIFETIME_MS_MIN, LinkEpoch, ZoneLinkEnrollment, ZoneLinkEnrollmentError,
    ZoneLinkState,
};
pub use prologue::{SubjectContextDigest, ZoneLinkPrologue};
pub use zone_link::{ZoneLinkSession, ZoneLinkSessionError};

/// The v3 Zone session wire contract.
///
/// This is the extended taxonomy plus the frozen protocol constants and binary
/// types. Import the endpoint enumerations from here, never from the
/// component-session module, so the un-extended taxonomy cannot be picked up
/// by accident.
pub use d2b_contracts::v3::zone_session as wire;

// The ported ComponentSession runtime. Every item below is the audited
// definition, re-exported unchanged; this module adds no wrapper, no default,
// and no alternative constructor for any of them.

pub use d2b_session::{
    // Bootstrap PSK admission: single-use, operation-bound, expiring, zeroized.
    AdmittedBootstrapPsk,
    // Attachment carriage.
    AttachmentPayload,
    AttachmentValidationError,
    BootstrapAdmission,
    BootstrapPsk,
    // Cancellation and the request registry.
    Cancellation,
    // The session drive loop.
    ComponentSessionDriver,
    DeadlineBudget,
    // Handshake: three Noise profiles and generation discovery.
    EstablishedHandshake,
    FairScheduler,
    // Fragmentation and reassembly.
    Fragment,
    Fragmenter,
    GENERATION_DISCOVERY_REQUEST_LEN,
    GENERATION_DISCOVERY_RESPONSE_LEN,
    HandshakeCredentials,
    HandshakeRole,
    // Keepalive and lifecycle.
    KeepaliveAction,
    MetricEvent,
    MetricsSink,
    NamedStreamMux,
    NegotiatedOffer,
    NoiseHandshake,
    NoopMetrics,
    OperationKind,
    OutboundFrame,
    OwnedAttachment,
    OwnedTransport,
    // Record protection.
    ProtectedRecord,
    QueueClass,
    Reassembler,
    RecordProtector,
    RequestRegistry,
    Result as SessionResult,
    Secret32,
    SessionDriverHandle,
    SessionEngine,
    SessionError,
    SessionErrorClass,
    SessionEvent,
    SessionLifecycle,
    SessionOperation,
    SessionPhase,
    StreamEvent,
    StreamId,
    StreamPhase,
    TransportDescriptor,
    TransportError,
    TransportPacket,
    TransportReader,
    TransportWriter,
    accept_generation_discovery_request,
    decode_generation_discovery_response,
    encode_generation_discovery_request,
    encode_generation_discovery_response,
    encode_offer,
    is_generation_discovery_request,
    negotiate_offer,
    serialized_transport_split,
    x25519_public_key,
};
