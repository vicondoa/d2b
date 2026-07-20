//! Portable, fail-closed client foundation for d2b v2 ComponentSession services.

mod client;
pub mod daemon_service;
mod error;
mod guest_service;
#[cfg(feature = "host-socket")]
mod host_socket;
mod service;
mod session;
mod target;

pub use client::{
    CallOptions, CancellationToken, Client, ConnectedClient, MetadataInput, Response, RetryPolicy,
    SystemClock, WallClock,
};
pub use d2b_session::{
    AttachmentPayload, AttachmentValidationError, ComponentSessionDriver, HandshakeCredentials,
    OwnedAttachment, OwnedTransport, StreamId, TransportPacket,
};
pub use daemon_service::{
    DaemonClient, DaemonLifecycleRequest, DaemonMethod, DaemonTerminal, daemon_call_options,
};
pub use error::{ClientError, RemoteErrorKind, RetryClass};
pub use guest_service::{
    GuestCancelCall, GuestClient, GuestInspectCall, GuestOperation, GuestRetainedLogCall,
};
#[cfg(feature = "host-socket")]
pub use host_socket::{HostSocketConnector, local_daemon_endpoint_identity};
pub use service::{GeneratedClient, MethodHandle, ServiceHandle, ServiceKind};
pub use session::{
    ComponentSessionConnector, ConnectedSession, NamedStream, SessionCall, SessionFailure,
    SessionReply, SharedDriver,
};
pub use target::{
    ResolvedTarget, RouteRecord, RouteTable, ServiceOwner, TargetInput, TargetResolver,
    TransportKind, TransportSelection,
};
