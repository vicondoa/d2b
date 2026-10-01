//! Recording test doubles shared with downstream crates' unit tests.
//!
//! Gated behind the `test-support` Cargo feature so production
//! consumers never pull this in.
//!
//! The double's ordered log is the toolkit's `SharedLog`
//! (`d2b_provider_toolkit::testing`), the canonical recorder shape every
//! family crate's test-support module shares. The scripted observation is
//! the Endpoint provider's own [`EndpointAccessObservation`], so a test
//! scripts what the KERNEL applies - the effective rights, the effective
//! traverse bit, the listable parent, the accepting endpoint, and the pinned
//! `(dev, ino)` - rather than a boolean that stands in for them.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use d2b_provider_endpoint::{EndpointAccessObservation, EndpointSocketIdentity};
use d2b_provider_toolkit::testing::SharedLog;

use crate::driver::{
    EndpointBindingDelivery, EndpointBindingDriverEffects, EndpointDeliveryTarget,
};

/// The POSIX bits a fully effective principal applies: read and write on the
/// socket, traverse on every ancestor.
const EFFECTIVE_RIGHTS: u32 = 0o6;
/// The effective traverse bit every ancestor directory applies.
const EFFECTIVE_TRAVERSE: u32 = 0o1;

/// Scripted serving port over the caller's ordered log, so the tests assert
/// one sequence across manager calls and serving effects.
pub struct FakeEndpointEffects {
    log: SharedLog,
    device: AtomicU64,
    inode: AtomicU64,
    rights: AtomicU32,
    traverse: AtomicU32,
    listable: AtomicBool,
    accepting: AtomicBool,
    attached: AtomicBool,
    fail_verify: AtomicBool,
    fail_deliver: AtomicBool,
    fail_fence: AtomicBool,
    fail_attached: AtomicBool,
    fail_release: AtomicBool,
}

impl FakeEndpointEffects {
    /// A fresh double with its own ordered log, a ready and accepting
    /// endpoint, and no attachment.
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            log: SharedLog::new(),
            device: AtomicU64::new(0xfd00),
            inode: AtomicU64::new(0x1a2b),
            rights: AtomicU32::new(EFFECTIVE_RIGHTS),
            traverse: AtomicU32::new(EFFECTIVE_TRAVERSE),
            listable: AtomicBool::new(false),
            accepting: AtomicBool::new(true),
            attached: AtomicBool::new(false),
            fail_verify: AtomicBool::new(false),
            fail_deliver: AtomicBool::new(false),
            fail_fence: AtomicBool::new(false),
            fail_attached: AtomicBool::new(false),
            fail_release: AtomicBool::new(false),
        })
    }

    /// A double whose ordered log is shared with the caller's manager
    /// logger, so manager calls and serving effects read as one sequence.
    pub fn shared(log: SharedLog) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            log,
            device: AtomicU64::new(0xfd00),
            inode: AtomicU64::new(0x1a2b),
            rights: AtomicU32::new(EFFECTIVE_RIGHTS),
            traverse: AtomicU32::new(EFFECTIVE_TRAVERSE),
            listable: AtomicBool::new(false),
            accepting: AtomicBool::new(true),
            attached: AtomicBool::new(false),
            fail_verify: AtomicBool::new(false),
            fail_deliver: AtomicBool::new(false),
            fail_fence: AtomicBool::new(false),
            fail_attached: AtomicBool::new(false),
            fail_release: AtomicBool::new(false),
        })
    }

    /// The identity the adapter currently resolves the exact endpoint to.
    pub fn pinned_socket(&self) -> EndpointSocketIdentity {
        EndpointSocketIdentity::new(
            self.device.load(Ordering::SeqCst),
            self.inode.load(Ordering::SeqCst),
        )
    }

    /// The exact endpoint's inode was replaced: a later pass observes a
    /// different identity than the one this row delivered.
    pub fn replace_inode(&self) {
        self.inode.fetch_add(1, Ordering::SeqCst);
    }

    /// Script the effective rights the kernel applies to short of what the
    /// admitted right needs.
    pub fn set_effective_rights(&self, rights: u32) {
        self.rights.store(rights & 0o7, Ordering::SeqCst);
    }

    /// Script the effective traverse bit every ancestor applies.
    pub fn set_effective_traverse(&self, traverse: u32) {
        self.traverse.store(traverse & 0o7, Ordering::SeqCst);
    }

    /// Script the socket's containing directory to be listable by the
    /// consumer principal.
    pub fn set_parent_listable(&self, listable: bool) {
        self.listable.store(listable, Ordering::SeqCst);
    }

    /// Script whether the exact endpoint is accepting connections or
    /// attachments.
    pub fn set_accepting(&self, accepting: bool) {
        self.accepting.store(accepting, Ordering::SeqCst);
    }

    /// Script the consumer to still hold the delivered endpoint: the
    /// teardown gate must block.
    pub fn set_attached(&self, attached: bool) {
        self.attached.store(attached, Ordering::SeqCst);
    }

    /// Script `verify` to fail with a serving error.
    pub fn set_fail_verify(&self, fail: bool) {
        self.fail_verify.store(fail, Ordering::SeqCst);
    }

    /// Script `deliver` to fail with a serving error.
    pub fn set_fail_deliver(&self, fail: bool) {
        self.fail_deliver.store(fail, Ordering::SeqCst);
    }

    /// Script `fence` to fail with a serving error.
    pub fn set_fail_fence(&self, fail: bool) {
        self.fail_fence.store(fail, Ordering::SeqCst);
    }

    /// Script `consumer_attached` to fail with a serving error.
    pub fn set_fail_attached(&self, fail: bool) {
        self.fail_attached.store(fail, Ordering::SeqCst);
    }

    /// Script `release` to fail with a serving error.
    pub fn set_fail_release(&self, fail: bool) {
        self.fail_release.store(fail, Ordering::SeqCst);
    }

    /// The ordered serving-effect log, shared with any manager logger.
    pub fn call_order(&self) -> Vec<String> {
        self.log.entries()
    }

    /// How many times one effect was called.
    pub fn count_of(&self, effect: &str) -> usize {
        self.log.entries().iter().filter(|entry| *entry == effect).count()
    }

    /// The facet set the plane and this crate's tests build the driver and
    /// the effects service from: the scripted serving double behind the five
    /// facets.
    pub fn facet_set(self: &std::sync::Arc<Self>) -> crate::facets::EndpointBindingEffectFacets {
        crate::facets::EndpointBindingEffectFacets {
            verify: std::sync::Arc::new(ScriptedVerify(std::sync::Arc::clone(self))),
            deliver: std::sync::Arc::new(ScriptedDeliver(std::sync::Arc::clone(self))),
            fence: std::sync::Arc::new(ScriptedFence(std::sync::Arc::clone(self))),
            attached: std::sync::Arc::new(ScriptedAttached(std::sync::Arc::clone(self))),
            release: std::sync::Arc::new(ScriptedRelease(std::sync::Arc::clone(self))),
        }
    }

    /// The observation the adapter reports for this exact endpoint right
    /// now.
    fn observe(&self) -> EndpointAccessObservation {
        EndpointAccessObservation::new(
            self.pinned_socket(),
            self.rights.load(Ordering::SeqCst),
            self.traverse.load(Ordering::SeqCst),
            self.listable.load(Ordering::SeqCst),
            self.accepting.load(Ordering::SeqCst),
        )
    }
}

/// The scripted verification facet: records the call and answers with what
/// the scripted kernel state implies.
struct ScriptedVerify(std::sync::Arc<FakeEndpointEffects>);

#[async_trait::async_trait]
impl crate::facets::EndpointVerifySource for ScriptedVerify {
    async fn verify(
        &self,
        _target: &EndpointDeliveryTarget,
    ) -> Result<EndpointAccessObservation, String> {
        self.0.log.record("verify".to_owned());
        if self.0.fail_verify.load(Ordering::SeqCst) {
            return Err("scripted verify failure".to_owned());
        }
        Ok(self.0.observe())
    }
}

/// The scripted delivery facet: records the call and pins the identity the
/// adapter resolved.
struct ScriptedDeliver(std::sync::Arc<FakeEndpointEffects>);

#[async_trait::async_trait]
impl crate::facets::EndpointDeliverSource for ScriptedDeliver {
    async fn deliver(
        &self,
        _target: &EndpointDeliveryTarget,
        _delivery: &EndpointBindingDelivery,
    ) -> Result<EndpointSocketIdentity, String> {
        self.0.log.record("deliver".to_owned());
        if self.0.fail_deliver.load(Ordering::SeqCst) {
            return Err("scripted deliver failure".to_owned());
        }
        Ok(self.0.pinned_socket())
    }
}

/// The scripted attachment-observation facet: records the call and answers
/// the scripted attachment state.
struct ScriptedAttached(std::sync::Arc<FakeEndpointEffects>);

#[async_trait::async_trait]
impl crate::facets::EndpointAttachmentSource for ScriptedAttached {
    async fn attached(
        &self,
        _target: &EndpointDeliveryTarget,
    ) -> Result<bool, String> {
        self.0.log.record("attached".to_owned());
        if self.0.fail_attached.load(Ordering::SeqCst) {
            return Err("scripted attached failure".to_owned());
        }
        Ok(self.0.attached.load(Ordering::SeqCst))
    }
}


/// The scripted pre-drain fence facet: records the call on the shared double.
struct ScriptedFence(std::sync::Arc<FakeEndpointEffects>);

/// The scripted release facet: records the call on the shared double.
struct ScriptedRelease(std::sync::Arc<FakeEndpointEffects>);
#[async_trait::async_trait]
impl crate::facets::EndpointFenceSource for ScriptedFence {
    async fn fence(&self, target: &EndpointDeliveryTarget) -> Result<(), String> {
        self.0.log.record("fence".to_owned());
        if self.0.fail_fence.load(Ordering::SeqCst) {
            return Err("scripted fence failure".to_owned());
        }
        let _ = target;
        Ok(())
    }
}

#[async_trait::async_trait]
impl crate::facets::EndpointReleaseSource for ScriptedRelease {
    async fn release(&self, target: &EndpointDeliveryTarget) -> Result<(), String> {
        self.0.log.record("release".to_owned());
        if self.0.fail_release.load(Ordering::SeqCst) {
            return Err("scripted release failure".to_owned());
        }
        let _ = target;
        Ok(())
    }
}

#[async_trait::async_trait]
impl EndpointBindingDriverEffects for FakeEndpointEffects {
    async fn verify(
        &self,
        _target: &EndpointDeliveryTarget,
    ) -> Result<EndpointAccessObservation, String> {
        self.log.record("verify".to_owned());
        if self.fail_verify.load(Ordering::SeqCst) {
            return Err("scripted verify failure".to_owned());
        }
        Ok(self.observe())
    }

    async fn deliver(
        &self,
        _target: &EndpointDeliveryTarget,
        _delivery: &EndpointBindingDelivery,
    ) -> Result<EndpointSocketIdentity, String> {
        self.log.record("deliver".to_owned());
        if self.fail_deliver.load(Ordering::SeqCst) {
            return Err("scripted deliver failure".to_owned());
        }
        Ok(self.pinned_socket())
    }

    async fn fence(&self, _target: &EndpointDeliveryTarget) -> Result<(), String> {
        self.log.record("fence".to_owned());
        if self.fail_fence.load(Ordering::SeqCst) {
            return Err("scripted fence failure".to_owned());
        }
        Ok(())
    }

    async fn consumer_attached(&self, _target: &EndpointDeliveryTarget) -> Result<bool, String> {
        self.log.record("attached".to_owned());
        if self.fail_attached.load(Ordering::SeqCst) {
            return Err("scripted attached failure".to_owned());
        }
        Ok(self.attached.load(Ordering::SeqCst))
    }

    async fn release(&self, _target: &EndpointDeliveryTarget) -> Result<(), String> {
        self.log.record("release".to_owned());
        if self.fail_release.load(Ordering::SeqCst) {
            return Err("scripted release failure".to_owned());
        }
        Ok(())
    }
}
