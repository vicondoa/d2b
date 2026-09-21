//! The declared facets the provider-owned Host effects service reaches
//! daemon state through (U5).
//!
//! The Host family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]) over the bounded host
//! probe this crate owns (see [`crate::probe`]). The family reaches only
//! host state: the probe reads `/proc`, `/etc/os-release`, `/dev`,
//! `/sys/fs/cgroup`, and `/run/user` directly, and the one daemon-owned
//! read - the minijail platform gate the daemon's own `system-minijail`
//! Provider is constructed from - crosses the provider boundary as a
//! declared facet rather than a daemon call: the facet is a type this crate
//! declares, the daemon host supplies its implementation through the
//! composition root (never derived from caller input), and the family crate
//! holds no daemon state type.

use std::sync::Arc;

use d2b_provider_system_core::{HostProbeEffectPort, MinijailPlatformGate};

/// The daemon-supplied facet set the provider-owned Host effects are built
/// from (U5).
///
/// The composition root supplies the objects; the driver never holds a
/// daemon state type (R2). The one facet is the bounded probe the family's
/// effects run over: the crate's production probe, built by the composition
/// root from the daemon's minijail platform gate source (see
/// [`crate::production_probe`]), or a scripted double in tests. Every other
/// probe input is host state the crate reads itself.
#[derive(Clone)]
pub struct HostEffectFacets {
    /// The bounded probe the family's effects run over: the same
    /// [`HostProbeEffectPort`] surface the crate's production `HostProbe`
    /// implements.
    pub probe: Arc<dyn HostProbeEffectPort>,
}

/// The daemon-supplied minijail platform gate source (U5).
///
/// The daemon implements this trait in its composition root over its own
/// `detect_minijail_platform_gate` probe; the family crate receives the
/// bounded gate snapshot, never a daemon function or host path.
pub trait MinijailPlatformGateSource: Send + Sync + 'static {
    /// The current bounded platform gate snapshot.
    fn platform_gate(&self) -> MinijailPlatformGate;
}
