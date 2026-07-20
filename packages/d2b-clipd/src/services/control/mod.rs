//! Clipboard control-plane policy behind the frozen ComponentSession service.
//!
//! The composition layer decodes the generated `d2b.clipboard.v2` request and
//! admits its ComponentSession before constructing the types in this module.
//! Consequently this module has no socket, framing, credential, or fallback
//! path of its own.

mod service;

pub use service::{
    AdmittedCall, ClipboardControl, ClipboardControlConfig, ControlError, ControlInput,
    ControlMethod, ControlObservation, ControlOperation, ControlOutcome, ControlPeer,
    ControlResponse, ControlSession, ControlTransport, OfferIntent, OfferState,
};

pub const SERVICE_PACKAGE: &str = "d2b.clipboard.v2";
pub const ENDPOINT_PURPOSE: &str = "clipboard-control";
pub const ENDPOINT_ROLE: &str = "clipboard-daemon";

pub const SERVICE_NAME: &str = "ClipboardService";

pub const METHODS: &[ControlMethod] = &[
    ControlMethod::Offer,
    ControlMethod::InspectOffer,
    ControlMethod::AcceptTransfer,
    ControlMethod::CompleteTransfer,
    ControlMethod::CancelTransfer,
    ControlMethod::BridgeReady,
    ControlMethod::Cancel,
];
