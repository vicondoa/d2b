//! Canonical `Provider/transport-vsock` implementation.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod auth;
mod bridge;
mod errors;
mod framing;
mod graph_binding;
mod limits;
mod relay;
mod service;
mod settings;
mod state_volume;
mod topology;

pub use auth::{
    GraphBoundSession, GuestIdentity, PeerCid, ReadySession, SessionAuthority, SessionKey,
    SessionProof, SessionRejectReason, SessionState,
};
pub use bridge::{
    BridgeControl, BridgeExit, BridgeStats, CarriageClass, ControlPlaneInjectionRefusal,
    ControlPlaneRequest, ControlRouteToken, NamedStreamError, NamedStreamId, NamedStreamPort,
    TransportControlOperation, TransportHandle, classify_carriage,
};
pub use errors::{ServiceError, TransportError, VsockEffectError};
pub use framing::{FramedVsockTransport, VsockTransportDescriptor};
pub use graph_binding::{
    AdmittedTransportBinding, AdmittedTransportRoute, RelationshipFence, RelationshipPhase,
    TransportAttachEvidence, TransportAttachRefusal, TransportBindingRefusal,
    TransportBindingRegistry, admit_attach, admit_route, MAX_ADMITTED_TRANSPORT_BINDINGS,
};
pub use limits::{
    CLOSE_GRACE_MS, MAX_ACTIVE_TRANSPORTS, MAX_FRAME_BYTES, MAX_OPEN_DEADLINE_MS,
    MAX_REPLAY_ENTRIES, MIN_OPEN_DEADLINE_MS,
};
pub use relay::{
    NativeGuestRelay, RelayBinding, RelayEffectError, RelayEffectPort, RelayObservation, RelayPhase,
};
pub use service::{
    CloseTransportRequest, ObserveTransportRequest, OpaqueBindingId, OpaqueEndpointId,
    OpenTransportRequest, OpenTransportResponse, ServicePhase, TransportEvent, TransportObservation,
    TransportPhase, TransportRole, VsockEffectPort, VsockTransportService,
};
pub use settings::{PortClass, SettingsError, VsockTransportSettings};
pub use state_volume::{EMPTY_STATE_SCHEMA, STATE_LAYOUT_USER, StateVolumeSpec};
pub use topology::{ParentStoreResourceCensus, TopologyError, TransportLimits, ZoneLinkSpec};

/// Stable Provider implementation identifier.
pub const VSOCK_IMPLEMENTATION_ID: &str = "vsock";
/// Stable Provider resource reference.
pub const PROVIDER_REF: &str = "Provider/transport-vsock";
/// The role id the vsock relay process carries in launcher rows.
pub const VSOCK_RELAY_ROLE: &str = "vsock-relay";
