//! Portable, fail-closed client foundation for d2b v2 ComponentSession services.

mod client;
mod error;
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
    AttachmentPayload, AttachmentValidationError, ComponentSessionDriver, OwnedAttachment,
    OwnedTransport, StreamId, TransportPacket,
};
pub use error::{ClientError, RemoteErrorKind, RetryClass};
#[cfg(feature = "host-socket")]
pub use host_socket::HostSocketConnector;
pub use service::{GeneratedClient, MethodHandle, ServiceHandle, ServiceKind};
pub use session::{
    ComponentSessionConnector, ConnectedSession, NamedStream, SessionCall, SessionFailure,
    SessionReply, SharedDriver,
};
pub use target::{
    ResolvedTarget, RouteRecord, RouteTable, ServiceOwner, TargetInput, TargetResolver,
    TransportKind, TransportSelection,
};
