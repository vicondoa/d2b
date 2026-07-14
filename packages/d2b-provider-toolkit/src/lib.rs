//! Provider-agent server, registration, redaction, fixture, and conformance
//! toolkit for the canonical d2b provider contracts.
//!
//! The toolkit is internal, has no default features, and exposes no ambient
//! credential, dynamic-loading, or fallback mechanism.

mod adapter;
mod conformance;
mod fixture;
mod redaction;
mod registration;
mod server;
mod session_seam;

pub use adapter::ProviderAgentAdapter;
pub use conformance::{ConformanceError, check_descriptor_conformance, check_provider_conformance};
pub use fixture::{DeterministicClock, FakeProvider, Fixture, sample_lease_request};
pub use redaction::{Redacted, Secret};
pub use registration::{ToolkitError, register_exact_instances};
pub use server::GeneratedProviderServiceServer;
pub use session_seam::{
    AuthenticatedSessionState, ClosedProviderMethod, ComponentSessionDriver, OwnedAttachment,
    SessionDriverError, TransportPacket,
};
