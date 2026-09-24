//! The Host provider crate: the `Host` resource type's driver, its spec
//! decoder, its driver declaration, and the implementation of the family's
//! driver effects.
//!
//! The crate owns the Host type's complete resource knowledge: the closed
//! Host base contract, the admission fence that pins `spec.providerRef` to
//! `Provider/system-core`, the driver's validate, recover, reconcile,
//! finalize, and delete verbs, and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! The family's driver effects (U5) are implemented by this crate itself
//! ([`crate::effects_service`]): the bounded capability/platform/proc probe
//! ([`crate::probe`]) with its preserved degraded fallback runs inside the
//! crate over the preserved `HostReconciler`, and the one daemon-owned read
//! (the minijail platform gate) crosses the provider boundary as the
//! declared [`crate::facets::MinijailPlatformGateSource`] facet the
//! composition root supplies. The daemon hosts the family's declared
//! effects service ([`crate::effects_service::HOST_EFFECTS_SERVICE`]) per
//! zone from the family's registered factory; no externally built port
//! appears at any construction site (R2).

#![deny(missing_docs)]

mod driver;

mod effects_service;
mod facets;
mod probe;

// The test-support doubles: the scripted HostDriverEffects recording
// double, the scripted HostProbeEffectPort probe double, and the scripted
// minijail gate source the production probe is built from. Needed both by
// external crates (d2bd's plane tests, which opt in via the `test-support`
// feature) and by this crate's own tests. Gating on
// `any(test, feature = "test-support")` makes it available automatically to
// this crate's unit tests. Integration tests that need it declare
// `required-features`, so run those with `--features test-support` (or let
// the Bazel `*_test_support` target compile them).
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use driver::host_descriptor;
pub use effects_service::{HOST_EFFECTS_SERVICE, HostEffectsServiceFactory};
pub use facets::{HostEffectFacets, MinijailPlatformGateSource};
pub use probe::{PIPEWIRE_RUNTIME_SOCKET, USBIP_CORE_MODULE, USBIP_HOST_MODULE, production_probe};
// The gate type the family's facets carry: re-exported through the owning
// crate so a daemon composition module can name it without importing the
// system-core crate path (the same re-export shape the process crate uses
// for its conformance vocabulary).
pub use d2b_provider_system_core::MinijailPlatformGate;
