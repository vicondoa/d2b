//! The declared facets the provider-owned VolumeBinding effects service
//! reaches daemon state through (U6).
//!
//! The VolumeBinding family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]) over the preserved
//! virtiofs serving adapter. The daemon-owned reads cross the provider
//! boundary as declared facets rather than daemon calls: the serving-socket
//! probe (resolve the binding's private socket target and probe the bound
//! socket on the host target), the socket removal (the endpoint-first half
//! of the teardown), and the guest-mount observation the drain gate reads
//! from the Zone target directory - never from a second channel.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_provider_volume_virtiofs::SocketIdentity;
use d2b_resource_runtime::identity::ResourceKey;

/// The daemon-supplied facet set the provider-owned VolumeBinding effects
/// are built from (U6).
///
/// The composition root supplies the objects; the driver never holds a
/// daemon state type (R2). The three facets are the serving-socket probe,
/// the socket removal, and the guest-mount observation the preserved
/// serving adapter owns.
#[derive(Clone)]
pub struct BindingEffectFacets {
    /// The serving-socket probe: whether one binding's private socket is
    /// bound on the host target (child-phase evidence).
    pub ready: Arc<dyn SocketReadySource>,
    /// The socket removal: the endpoint-first half of the teardown,
    /// idempotent under retry.
    pub remove: Arc<dyn SocketRemoveSource>,
    /// The guest-mount observation: whether the target Guest currently
    /// observes the row's mount, read from the Zone target directory.
    pub guest_mount: Arc<dyn GuestMountSource>,
}

/// The daemon-supplied serving-socket probe (U6): whether one binding's
/// private socket is resolved and bound on the host target.
#[async_trait]
pub trait SocketReadySource: Send + Sync + 'static {
    /// Whether the worker's private socket is ready (child-phase evidence).
    async fn ready(&self, socket: &SocketIdentity) -> bool;
}

/// The daemon-supplied socket removal (U6): the endpoint-first half of the
/// teardown, idempotent under retry (R10).
#[async_trait]
pub trait SocketRemoveSource: Send + Sync + 'static {
    /// Remove the endpoint realization (socket).
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon adapter fails to remove the realized
    /// socket endpoint. A socket that was never realized, or whose file is
    /// already gone, answers `Ok(())` - the removal is idempotent under
    /// retry.
    async fn remove(&self, socket: &SocketIdentity) -> Result<(), String>;
}

/// The daemon-supplied guest-mount observation (U6): whether the target
/// Guest currently observes the mount.
///
/// The evidence is the target layer's, never a second channel (U13): the
/// owning row's assignment in the Zone target directory is asked through
/// the live authenticated ComponentSession, and only a target-local
/// realization the Guest reports `ready` for that source answers `true`.
#[async_trait]
pub trait GuestMountSource: Send + Sync + 'static {
    /// Whether the target Guest observes the row's mount.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon adapter cannot complete the
    /// observation (the target layer is unreachable). A target the
    /// directory cannot reach, a loose row, and a source the Guest holds
    /// no realization for all answer `Ok(false)` - the observation fails
    /// closed.
    async fn guest_mount_ready(&self, key: &ResourceKey) -> Result<bool, String>;
}