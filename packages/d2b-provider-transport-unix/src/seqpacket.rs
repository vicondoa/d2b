//! Seqpacket transport reused from the authenticated session layer.

pub use d2b_session_unix::{
    AncillaryCapacity, OutboundPacket, PacketBurst, PeerIdentityPolicy, SeqpacketSocket,
    UnixSeqpacketTransport,
};
