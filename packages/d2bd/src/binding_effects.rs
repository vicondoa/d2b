//! Production VolumeBinding effects: the serving effect port the binding
//! driver runs behind.
//!
//! The port is three closures the plane assembles: the serving-socket probe,
//! the socket removal (the endpoint-first half of the teardown), and the
//! guest-mount observation the drain gate reads from the Zone target
//! directory - never from a second channel. This module only adapts them onto
//! the family's port.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use d2b_provider_volume_binding::BindingDriverEffects;
use d2b_provider_volume_virtiofs::{SocketIdentity, StoredBinding};
use d2b_resource_runtime::identity::ResourceKey;

/// Boxed future returned by one production serving-socket probe: resolving
/// the socket target is store-backed (the registry loads derived-child rows
/// from the authority on a miss), so the port cannot be a sync closure.
pub(crate) type ServingEffectFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Boxed serving-socket probe closure (one socket's ready state).
pub(crate) type SocketReadyEffect =
    Arc<dyn for<'a> Fn(&'a SocketIdentity) -> ServingEffectFuture<'a, bool> + Send + Sync>;

/// Boxed socket-removal closure (the endpoint-first half of the teardown).
pub(crate) type SocketRemoveEffect = Arc<
    dyn for<'a> Fn(&'a SocketIdentity) -> ServingEffectFuture<'a, Result<(), String>> + Send + Sync,
>;

/// Boxed guest-mount observation closure over the Zone target directory.
pub(crate) type GuestMountEffect =
    Arc<dyn for<'a> Fn(&'a ResourceKey) -> ServingEffectFuture<'a, bool> + Send + Sync>;

/// Production effects over the preserved virtiofs serving adapter, plus the
/// guest-mount observation over the Zone target directory, so the drain gate
/// reads the target layer's own evidence instead of the old
/// Endpoint-published state.
pub(crate) struct ProductionBindingDriverEffects {
    ready: SocketReadyEffect,
    remove: SocketRemoveEffect,
    /// Guest mount observation: one row key resolved through the Zone target
    /// directory - a target-local realization the Guest reports `ready` for
    /// that source is the only `true` answer.
    guest_mount: GuestMountEffect,
}

impl ProductionBindingDriverEffects {
    pub(crate) fn new(
        ready: SocketReadyEffect,
        remove: SocketRemoveEffect,
        guest_mount: GuestMountEffect,
    ) -> Self {
        Self {
            ready,
            remove,
            guest_mount,
        }
    }
}

#[async_trait::async_trait]
impl BindingDriverEffects for ProductionBindingDriverEffects {
    async fn socket_ready(&self, socket: &SocketIdentity) -> bool {
        (self.ready)(socket).await
    }

    async fn remove_socket(&self, socket: &SocketIdentity) -> Result<(), String> {
        (self.remove)(socket).await
    }

    async fn guest_mount_ready(
        &self,
        key: &ResourceKey,
        _binding: &StoredBinding,
    ) -> Result<bool, String> {
        Ok((self.guest_mount)(key).await)
    }
}
