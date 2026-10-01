//! Recording test doubles shared with this crate's and downstream crates'
//! unit tests.
//!
//! Gated behind the `test-support` Cargo feature so production consumers never
//! pull them in.
//!
//! The double's ordered log is the toolkit's `SharedLog`
//! (`d2b_provider_toolkit::testing`), the canonical recorder shape every family
//! crate's test-support module shares, so one sequence reads across manager
//! calls and fabric effects alike.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use d2b_contracts_resource::v3::ResourceGeneration;
use d2b_provider_toolkit::testing::SharedLog;

use crate::driver::{
    FabricDrain, FabricMembership, FabricMembershipState, FabricRelease,
    NetworkBindingDriverEffects,
};

/// What the scripted fabric holds for one consumer's membership.
#[derive(Clone, PartialEq, Eq)]
struct HeldMembership {
    /// The Network generation the membership was joined under.
    fabric_generation: u64,
    /// The interface this consumer's membership presented.
    presented_interface: String,
    /// Whether that interface exists yet.
    interface_ready: bool,
}

/// A scripted fabric over the caller's ordered log, holding one membership per
/// consumer identity, so a test can assert one sequence across manager calls
/// and fabric effects and can see exactly which interfaces the fabric holds.
pub struct FakeFabricEffects {
    log: SharedLog,
    held: tokio::sync::Mutex<BTreeMap<String, HeldMembership>>,
    interface_ready: AtomicBool,
    drained: AtomicBool,
    join_fails: AtomicBool,
    observe_fails: AtomicBool,
    release_fails: AtomicBool,
}

impl FakeFabricEffects {
    /// A fresh double with its own ordered log.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Self::build(SharedLog::new())
    }

    /// A double whose ordered log is shared with the caller's manager logger,
    /// so manager calls and fabric effects read as one sequence.
    #[must_use]
    pub fn shared(log: SharedLog) -> Arc<Self> {
        Self::build(log)
    }

    fn build(log: SharedLog) -> Arc<Self> {
        Arc::new(Self {
            log,
            held: tokio::sync::Mutex::new(BTreeMap::new()),
            interface_ready: AtomicBool::new(false),
            drained: AtomicBool::new(false),
            join_fails: AtomicBool::new(false),
            observe_fails: AtomicBool::new(false),
            release_fails: AtomicBool::new(false),
        })
    }

    /// Script the fabric to report a joined membership's presented interface as
    /// realized.
    pub fn make_interface_ready(&self) {
        self.interface_ready.store(true, Ordering::SeqCst);
    }

    /// Script the drain to report outstanding use already drained.
    pub fn make_drained(&self) {
        self.drained.store(true, Ordering::SeqCst);
    }

    /// Script the join to fail.
    pub fn set_fail_join(&self, fail: bool) {
        self.join_fails.store(fail, Ordering::SeqCst);
    }

    /// Script the observation to fail.
    pub fn set_fail_observe(&self, fail: bool) {
        self.observe_fails.store(fail, Ordering::SeqCst);
    }

    /// Script the release to fail.
    pub fn set_fail_release(&self, fail: bool) {
        self.release_fails.store(fail, Ordering::SeqCst);
    }

    /// The ordered effect log, shared with any manager logger.
    pub fn call_order(&self) -> Vec<String> {
        self.log.entries()
    }

    /// The interfaces the fabric currently holds, one per consumer.
    pub async fn held_interfaces(&self) -> Vec<String> {
        self.held
            .lock()
            .await
            .values()
            .map(|held| held.presented_interface.clone())
            .collect()
    }

    /// How many memberships the fabric currently holds.
    pub async fn membership_count(&self) -> usize {
        self.held.lock().await.len()
    }

    /// The facet set this crate's tests and the plane's registration build the
    /// driver from: the scripted fabric behind the four declared facets.
    #[must_use]
    pub fn facet_set(self: &Arc<Self>) -> crate::facets::NetworkBindingEffectFacets {
        crate::facets::NetworkBindingEffectFacets {
            observe: Arc::new(Scripted(Arc::clone(self))),
            join: Arc::new(Scripted(Arc::clone(self))),
            drain: Arc::new(Scripted(Arc::clone(self))),
            release: Arc::new(Scripted(Arc::clone(self))),
        }
    }

    /// The membership key one derived membership occupies: the consumer's own
    /// identity on the Network, never a name another consumer could collide
    /// with.
    fn key(membership: &FabricMembership) -> String {
        format!(
            "{}|{}",
            membership.network_uid.as_str(),
            membership.consumer_uid.as_str()
        )
    }

    /// What the fabric holds for one membership, without recording a call.
    async fn state(&self, membership: &FabricMembership) -> FabricMembershipState {
        let held = {
            let guard = self.held.lock().await;
            guard.get(&Self::key(membership)).cloned()
        };
        match held {
            Some(held) => FabricMembershipState {
                joined: true,
                fabric_generation: ResourceGeneration::new(held.fabric_generation).ok(),
                // The interface exists once it exists: a later pass observes
                // the same interface the join created, it does not mint
                // another one.
                interface_ready: held.interface_ready || self.interface_ready.load(Ordering::SeqCst),
            },
            None => FabricMembershipState::absent(),
        }
    }
}

#[async_trait::async_trait]
impl NetworkBindingDriverEffects for FakeFabricEffects {
    async fn observe_membership(
        &self,
        membership: &FabricMembership,
    ) -> Result<FabricMembershipState, String> {
        self.log.record("fabric-observe".to_owned());
        if self.observe_fails.load(Ordering::SeqCst) {
            return Err("scripted observe failure".to_owned());
        }
        Ok(self.state(membership).await)
    }

    async fn join_membership(
        &self,
        membership: &FabricMembership,
    ) -> Result<FabricMembershipState, String> {
        self.log.record(format!(
            "fabric-join:{}",
            membership.presented_interface.as_str()
        ));
        if self.join_fails.load(Ordering::SeqCst) {
            return Err("scripted join failure".to_owned());
        }
        let interface_ready = self.interface_ready.load(Ordering::SeqCst);
        // One record per consumer identity: a retried join rewrites that
        // consumer's own membership and can never add a second interface.
        self.held.lock().await.insert(
            Self::key(membership),
            HeldMembership {
                fabric_generation: membership.network_generation.get(),
                presented_interface: membership.presented_interface.as_str().to_owned(),
                interface_ready,
            },
        );
        Ok(self.state(membership).await)
    }

    async fn drain_membership(
        &self,
        membership: &FabricMembership,
    ) -> Result<FabricDrain, String> {
        self.log.record("fabric-drain".to_owned());
        if !self.state(membership).await.joined {
            // Nothing is held, so nothing can be outstanding: the drain is
            // already complete.
            return Ok(FabricDrain {
                fenced: true,
                drained: true,
            });
        }
        Ok(FabricDrain {
            fenced: true,
            drained: self.drained.load(Ordering::SeqCst),
        })
    }

    async fn leave_membership(
        &self,
        membership: &FabricMembership,
    ) -> Result<FabricRelease, String> {
        self.log.record("fabric-release".to_owned());
        if self.release_fails.load(Ordering::SeqCst) {
            return Err("scripted release failure".to_owned());
        }
        // One member's release never tears the shared fabric down, and a
        // membership the fabric no longer holds releases successfully too, so
        // a retried teardown converges.
        self.held.lock().await.remove(&Self::key(membership));
        Ok(FabricRelease {
            remaining_members: self.membership_count().await,
            fabric_retained: true,
        })
    }
}

/// The scripted facet: one value behind all four declared facets, so the
/// double's joined state is the single source every effect reads.
struct Scripted(Arc<FakeFabricEffects>);

#[async_trait::async_trait]
impl crate::facets::FabricObserveSource for Scripted {
    async fn observe(
        &self,
        membership: &FabricMembership,
    ) -> Result<FabricMembershipState, String> {
        self.0.observe_membership(membership).await
    }
}

#[async_trait::async_trait]
impl crate::facets::FabricJoinSource for Scripted {
    async fn join(&self, membership: &FabricMembership) -> Result<FabricMembershipState, String> {
        self.0.join_membership(membership).await
    }
}

#[async_trait::async_trait]
impl crate::facets::FabricDrainSource for Scripted {
    async fn drain(&self, membership: &FabricMembership) -> Result<FabricDrain, String> {
        self.0.drain_membership(membership).await
    }
}

#[async_trait::async_trait]
impl crate::facets::FabricReleaseSource for Scripted {
    async fn release(&self, membership: &FabricMembership) -> Result<FabricRelease, String> {
        self.0.leave_membership(membership).await
    }
}
