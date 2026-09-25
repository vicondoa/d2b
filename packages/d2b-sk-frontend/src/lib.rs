//! d2b guest-side virtual FIDO/security-key frontend.
//!
//! The crate owns exactly the type-specific half of the frontend: the
//! `/dev/uhid` virtual FIDO2 CTAPHID device and the report translation between
//! that device and the enrolled Guest session. Everything around it - the link
//! to the allocator, the `zone-bootstrap`/`zone-enroll` enrollment, the
//! admitted session loop, its disconnect and reconnect bounds, and drain
//! ordering - is the toolkit's [`d2b_provider_toolkit::GuestAgent`] base, so
//! this crate holds no session, admission, or service loop of its own.
//!
//! # Trust boundary
//!
//! The binary is a Guest agent. It holds no Zone authority, no host path, no
//! credential, and no key byte: it is started with the placement the allocator
//! minted for it, it enrolls with the two landed Zone service methods, and it
//! serves CTAPHID report frames on the enrolled session. No host `Principal`
//! row, host user, or provider-internal state crosses into this crate.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod agent;
mod config;
mod link;
mod uhid;

pub use agent::{HidDevice, SecurityKeyFrontend};
pub use config::{Config, PlacementConfig};
pub use link::VsockAllocatorLink;
pub use uhid::{UhidDevice, UhidEvent};
