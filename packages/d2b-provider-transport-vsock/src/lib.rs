//! Canonical `Provider/transport-vsock` implementation.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod audit;
mod auth;
mod bridge;
mod effect_port;
mod errors;
mod framing;
mod limits;
mod metrics;
mod relay;
mod relay_argv;
mod service;
mod settings;
mod state_volume;
mod topology;

pub use audit::{TransportAuditEvent, TransportAuditOperation, TransportAuditOutcome};
pub use auth::{
    GuestIdentity, PeerCid, ReadySession, SessionAuthority, SessionKey, SessionProof,
    SessionRejectReason, SessionState,
};
pub use bridge::{
    BridgeControl, BridgeExit, BridgeStats, NamedStreamError, NamedStreamId, NamedStreamPort,
    TransportHandle,
};
pub use effect_port::{OpaqueBindingId, OpaqueEndpointId, TransportRole, VsockEffectPort};
pub use errors::{ServiceError, TransportError, VsockEffectError};
pub use framing::{FramedVsockTransport, VsockTransportDescriptor};
pub use limits::{
    CLOSE_GRACE_MS, MAX_ACTIVE_TRANSPORTS, MAX_FRAME_BYTES, MAX_OPEN_DEADLINE_MS,
    MAX_REPLAY_ENTRIES, MIN_OPEN_DEADLINE_MS,
};
pub use metrics::{TransportMetricLabels, TransportMetricOperation, TransportMetricOutcome};
pub use relay::{
    NativeGuestRelay, RelayBinding, RelayEffectError, RelayEffectPort, RelayObservation, RelayPhase,
};
pub use relay_argv::{
    SocatEndpoint, VsockRelayArgvError, VsockRelayArgvInput, exec_arg0, generate_vsock_relay_argv,
};
pub use service::{
    CloseTransportRequest, ObserveTransportRequest, OpenTransportRequest, OpenTransportResponse,
    ServicePhase, TransportEvent, TransportObservation, TransportPhase, VsockTransportService,
};
pub use settings::{PortClass, SettingsError, VsockTransportSettings};
pub use state_volume::{EMPTY_STATE_SCHEMA, STATE_LAYOUT_USER, StateVolumeSpec};
pub use topology::{ParentStoreResourceCensus, TopologyError, TransportLimits, ZoneLinkSpec};

/// Stable Provider implementation identifier.
pub const VSOCK_IMPLEMENTATION_ID: &str = "vsock";
/// Stable Provider resource reference.
pub const PROVIDER_REF: &str = "Provider/transport-vsock";
