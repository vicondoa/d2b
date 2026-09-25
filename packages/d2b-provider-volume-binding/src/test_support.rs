//! Recording test doubles shared with downstream crates' unit tests.
//!
//! Gated behind the `test-support` Cargo feature so production
//! consumers never pull this in.

use std::sync::{Arc, Mutex};

use d2b_provider_volume_virtiofs::{SocketIdentity, StoredBinding};
use d2b_resource_runtime::identity::ResourceKey;

use crate::driver::BindingDriverEffects;

/// Scripted serving port over the caller's ordered log, so the tests
/// assert one sequence across manager calls and serving effects.
pub struct FakeServingEffects {
    log: Arc<Mutex<Vec<String>>>,
    ready: std::sync::atomic::AtomicBool,
    mounted: std::sync::atomic::AtomicBool,
}

impl FakeServingEffects {
    /// A fresh double with its own ordered log.
    pub fn new() -> Arc<Self> {
        Self::shared(Arc::new(Mutex::new(Vec::new())))
    }

    /// A double whose ordered log is shared with the caller's manager
    /// logger, so manager calls and serving effects read as one sequence.
    pub fn shared(log: Arc<Mutex<Vec<String>>>) -> Arc<Self> {
        Arc::new(Self {
            log,
            ready: std::sync::atomic::AtomicBool::new(false),
            mounted: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// Script `socket_ready` to report the serving socket present.
    pub fn make_ready(&self) {
        self.ready.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// The guest observes the mount: the drain gate must block.
    pub fn make_mounted(&self) {
        self.mounted.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// The ordered serving-effect log, shared with any manager logger.
    pub fn call_order(&self) -> Vec<String> {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        self.log.lock().unwrap().clone()
    }

    /// The facet set the plane and this crate's tests build the driver and
    /// the effects service from: the scripted serving double behind the
    /// three facets.
    pub fn facet_set(self: &Arc<Self>) -> crate::facets::BindingEffectFacets {
        crate::facets::BindingEffectFacets {
            ready: Arc::new(ScriptedReady(Arc::clone(self))),
            remove: Arc::new(ScriptedRemove(Arc::clone(self))),
            guest_mount: Arc::new(ScriptedGuestMount(Arc::clone(self))),
        }
    }
}

/// The scripted serving-socket probe facet: records the call on the shared
/// double and answers its scripted readiness.
struct ScriptedReady(Arc<FakeServingEffects>);

#[async_trait::async_trait]
impl crate::facets::SocketReadySource for ScriptedReady {
    async fn ready(&self, _socket: &SocketIdentity) -> bool {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        self.0.log.lock().unwrap().push("socket-ready".to_owned()); // async-gate-allow: test-support recorder lock
        self.0.ready.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// The scripted socket-removal facet: records the call on the shared
/// double.
struct ScriptedRemove(Arc<FakeServingEffects>);

#[async_trait::async_trait]
impl crate::facets::SocketRemoveSource for ScriptedRemove {
    async fn remove(&self, _socket: &SocketIdentity) -> Result<(), String> {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        self.0.log.lock().unwrap().push("remove-socket".to_owned()); // async-gate-allow: test-support recorder lock
        Ok(())
    }
}

/// The scripted guest-mount observation facet: records the call on the
/// shared double and answers its scripted mount state.
struct ScriptedGuestMount(Arc<FakeServingEffects>);

#[async_trait::async_trait]
impl crate::facets::GuestMountSource for ScriptedGuestMount {
    async fn guest_mount_ready(&self, _key: &ResourceKey) -> Result<bool, String> {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        self.0.log.lock().unwrap().push("guest-mount".to_owned()); // async-gate-allow: test-support recorder lock
        Ok(self.0.mounted.load(std::sync::atomic::Ordering::SeqCst))
    }
}

#[async_trait::async_trait]
impl BindingDriverEffects for FakeServingEffects {
    async fn socket_ready(&self, _socket: &SocketIdentity) -> bool {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        self.log.lock().unwrap().push("socket-ready".to_owned()); // async-gate-allow: test-support recorder lock
        self.ready.load(std::sync::atomic::Ordering::SeqCst)
    }

    async fn remove_socket(&self, _socket: &SocketIdentity) -> Result<(), String> {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        self.log.lock().unwrap().push("remove-socket".to_owned()); // async-gate-allow: test-support recorder lock
        Ok(())
    }

    async fn guest_mount_ready(
        &self,
        _key: &ResourceKey,
        _binding: &StoredBinding,
    ) -> Result<bool, String> {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        self.log.lock().unwrap().push("guest-mount".to_owned()); // async-gate-allow: test-support recorder lock
        Ok(self.mounted.load(std::sync::atomic::Ordering::SeqCst))
    }
}
