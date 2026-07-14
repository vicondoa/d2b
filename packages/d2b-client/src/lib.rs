//! Portable, fail-closed client foundation for d2b v2 ComponentSession services.

mod client;
mod error;
mod service;
mod session;
mod target;

pub use client::{
    CallOptions, CancellationToken, Client, ConnectedClient, MetadataInput, Response, RetryPolicy,
    SystemClock, WallClock,
};
pub use error::{ClientError, RemoteErrorKind, RetryClass};
pub use service::{GeneratedClient, MethodHandle, ServiceHandle, ServiceKind};
pub use session::{
    ComponentSession, ComponentSessionConnector, ConnectedSession, NamedStream, SessionAttachment,
    SessionCall, SessionFailure, SessionReply, StreamId,
};
pub use target::{
    ResolvedTarget, RouteRecord, RouteTable, ServiceOwner, TargetInput, TargetResolver,
    TransportKind, TransportSelection,
};
