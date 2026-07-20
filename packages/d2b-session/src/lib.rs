//! Portable ComponentSession v2 runtime.
//!
//! Transport-specific socket and descriptor handling is intentionally outside
//! this crate. Callers provide an owned [`OwnedTransport`] implementation.

#![forbid(unsafe_code)]

mod attachment;
mod bootstrap;
mod cancellation;
mod deadline;
mod driver;
mod engine;
mod error;
mod fragmentation;
mod handshake;
mod lifecycle;
mod metrics;
mod record;
mod scheduler;
mod server;
mod streams;
mod transport;

pub use bootstrap::{AdmittedBootstrapPsk, BootstrapAdmission, BootstrapPsk, Secret32};
pub use cancellation::{Cancellation, RequestRegistry};
pub use deadline::DeadlineBudget;
pub use driver::{ComponentSessionDriver, SessionDriverHandle};
pub use engine::{SessionEngine, SessionEvent};
pub use error::{Result, SessionError};
pub use fragmentation::{Fragment, Fragmenter, Reassembler};
pub use handshake::{
    EstablishedHandshake, GENERATION_DISCOVERY_REQUEST_LEN, GENERATION_DISCOVERY_RESPONSE_LEN,
    HandshakeCredentials, HandshakeRole, NegotiatedOffer, NoiseHandshake,
    accept_generation_discovery_request, decode_generation_discovery_response,
    encode_generation_discovery_request, encode_generation_discovery_response, encode_offer,
    is_generation_discovery_request, negotiate_offer, x25519_public_key,
};
pub use lifecycle::{KeepaliveAction, SessionLifecycle, SessionPhase};
pub use metrics::{MetricEvent, MetricsSink, NoopMetrics};
pub use record::{ProtectedRecord, RecordProtector};
pub use scheduler::{FairScheduler, OutboundFrame, QueueClass};
pub use server::{SessionServerError, serve_ttrpc_services};
pub use streams::{NamedStreamMux, StreamEvent, StreamId, StreamPhase};
pub use transport::{OwnedTransport, TransportDescriptor, TransportError, TransportPacket};

pub use attachment::{AttachmentPayload, AttachmentValidationError, OwnedAttachment};
pub use d2b_contracts::v2_component_session as contract;
