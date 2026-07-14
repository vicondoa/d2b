#![forbid(unsafe_code)]
//! Audited Unix transport substrate for ComponentSession.
//!
//! The default feature set intentionally exports no host socket implementation.

#[cfg(all(feature = "host-socket", not(target_os = "linux")))]
compile_error!("the host-socket feature requires Linux");

#[cfg(feature = "host-socket")]
mod credit;
#[cfg(feature = "host-socket")]
mod descriptor;
#[cfg(feature = "host-socket")]
mod error;
#[cfg(feature = "host-socket")]
mod socket;

#[cfg(feature = "host-socket")]
pub use credit::{
    CreditBundle, CreditError, CreditPool, CreditScope, CreditScopeSet, ProcessCreditLimit,
};
#[cfg(feature = "host-socket")]
pub use descriptor::ReceivedPacket;
#[cfg(feature = "host-socket")]
pub use descriptor::{
    AcceptedAttachment, DescriptorPolicy, ObjectIdentity, PeerCredentials, VerifiedPacket,
};
#[cfg(feature = "host-socket")]
pub use error::UnixSessionError;
#[cfg(feature = "host-socket")]
pub use socket::{
    AncillaryCapacity, OutboundPacket, PacketBurst, SendBurst, SentPacket, SeqpacketSocket,
    StreamRead, StreamSocket, prearmed_seqpacket_pair,
};
