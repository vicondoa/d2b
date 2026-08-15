//! Authenticated local Unix transport for Zone-local Provider requests.

#![deny(missing_docs)]

/// Socket admission rules for local transport requests.
pub mod admission;
/// Bounded audit records for transport lifecycle events.
pub mod audit;
/// Attachment-credit types shared with the Unix session substrate.
pub mod credit;
/// Descriptor validation types shared with the Unix session substrate.
pub mod descriptor;
/// Accepted-socket request binding.
pub mod identity;
/// Bounded transport telemetry.
pub mod metrics;
/// The local transport portal and its owned monitor table.
pub mod portal;
/// Seqpacket transport types shared with the Unix session substrate.
pub mod seqpacket;
/// Service lifecycle façade for the local transport portal.
pub mod service;
/// Socket primitives shared with the Unix session substrate.
pub mod socket;
/// Stream transport types shared with the Unix session substrate.
pub mod stream;

pub use admission::{OpenTransportRequest, RouteClass, SocketKind, TransportAdmissionError};
pub use identity::{BrokerRole, TransportRequestBinding};
pub use portal::{
    OpenedTransport, PortalError, TransportDescriptor, TransportHandle, TransportObservation,
    TransportPortal,
};
