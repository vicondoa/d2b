//! Recording test doubles shared with downstream crates' unit tests.
//!
//! Gated behind the `test-support` Cargo feature so production
//! consumers never pull this in.

use std::sync::Arc;

use d2b_provider_volume_virtiofs::{SocketIdentity, StoredBinding};
use d2b_resource_runtime::identity::ResourceKey;
use parking_lot::Mutex;

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
        self.log.lock().clone()
    }
}

#[async_trait::async_trait]
impl BindingDriverEffects for FakeServingEffects {
    async fn socket_ready(&self, _socket: &SocketIdentity) -> bool {
        self.log.lock().push("socket-ready".to_owned());
        self.ready.load(std::sync::atomic::Ordering::SeqCst)
    }

    async fn remove_socket(&self, _socket: &SocketIdentity) -> Result<(), String> {
        self.log.lock().push("remove-socket".to_owned());
        Ok(())
    }

    async fn guest_mount_ready(
        &self,
        _key: &ResourceKey,
        _binding: &StoredBinding,
    ) -> Result<bool, String> {
        self.log.lock().push("guest-mount".to_owned());
        Ok(self.mounted.load(std::sync::atomic::Ordering::SeqCst))
    }
}
