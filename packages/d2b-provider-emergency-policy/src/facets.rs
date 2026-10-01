//! The per-Zone EmergencyPolicy runtime the driver is built over (U40, R36).
//!
//! The EmergencyPolicy family's driver effects are served by this crate's own
//! implementation. Two of the facts a reduction acts on are not derivable from
//! the policy row, and both are daemon-owned:
//!
//! - the Zone's **open use** - which reservations are held, which consumers
//!   sit on them, and which helper legs still exist - lives in the committed
//!   rows and the broker, never in the policy;
//! - the Zone's **live reduction**, which the manager-boundary admission reads
//!   on every mutation, is contributed by every `EmergencyPolicy` row.
//!
//! Both reach the driver through the per-Zone runtime this module defines. The
//! composition root builds one runtime per Zone and installs it here; the
//! driver looks its own Zone's runtime up. That is the whole cross-boundary
//! surface, and it is declared by this crate - the family holds no daemon
//! state type (R2), and a daemon that has not installed a Zone's runtime gets
//! a driver that refuses rather than one that silently admits.
//!
//! # Why the runtime is looked up rather than injected
//!
//! The Zone is a property of the durable row, not of the driver's
//! construction: the same descriptor registers the type for every Zone a plane
//! serves, and the row it reconciles names its own Zone. Keying the runtime by
//! Zone is therefore what makes one registered driver correct for all of them,
//! and it is why the lookup cannot return another Zone's answer.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};


use crate::{EmergencyDrainPlan, EmergencyReduction, OpenUseCensus, plan_drain};

/// The daemon-supplied open-use read (U40, R36).
///
/// # Errors
///
/// Returns `Err` when the read itself failed. That is distinct from
/// `Ok(None)`, which is the honest "the broker could not answer" answer:
/// [`plan_drain`] turns `None` into a fenced Zone, never a converged one.
#[async_trait::async_trait]
pub trait OpenUseSource: Send + Sync + 'static {
    /// What the Zone can prove about its own open use.
    async fn census(&self) -> Result<Option<OpenUseCensus>, String>;
}

/// One Zone's EmergencyPolicy runtime, installed by the composition root.
pub struct ZoneEmergencyRuntime {
    zone: d2b_contracts_resource::v3::ZoneId,
    open_use: Arc<dyn OpenUseSource>,
    reduction: RwLock<EmergencyReduction>,
}

impl ZoneEmergencyRuntime {
    /// Build the runtime for one Zone over the daemon's open-use read.
    ///
    /// The runtime starts with no reduction: a Zone whose `EmergencyPolicy`
    /// rows have not committed has no emergency, which is the honest prior
    /// state rather than a fabricated one.
    pub fn new(
        zone: d2b_contracts_resource::v3::ZoneId,
        open_use: Arc<dyn OpenUseSource>,
    ) -> Self {
        Self { zone, open_use, reduction: RwLock::new(EmergencyReduction::NONE) }
    }

    /// The Zone this runtime answers for.
    pub const fn zone(&self) -> &d2b_contracts_resource::v3::ZoneId {
        &self.zone
    }

    /// The reduction this Zone currently has published.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    pub fn reduction(&self) -> EmergencyReduction {
        self.reduction
            .read().map(|current| current.clone()).unwrap_or(EmergencyReduction::NONE)
    }

    /// Publish the Zone's effective reduction.
    ///
    /// A poisoned lock publishes [`EmergencyReduction::NONE`]: a reduction
    /// that cannot be recorded must not be the one left in force, and the
    /// failure direction that matters here is refusing new use, which the
    /// driver reaches through the row it reconciles.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    pub fn publish(&self, reduction: &EmergencyReduction) {
        match self.reduction.write() {
            Ok(mut current) => *current = reduction.clone(),
            Err(poisoned) => *poisoned.into_inner() = reduction.clone(),
        }
    }

    /// The plan this Zone's reduction drives against its open use.
    ///
    /// A census the daemon could not answer yields a fenced plan, so this
    /// returns a plan rather than an option: the plan's own state is what says
    /// whether anything may be released.
    pub async fn plan(&self, reduction: &EmergencyReduction) -> EmergencyDrainPlan {
        let census = self.open_use.census().await.unwrap_or(None);
        plan_drain(reduction, census.as_ref())
    }
}

/// The installed per-Zone runtimes, one per Zone per process.
///
/// A daemon process serves each Zone through its own plane and its own
/// manager, and a Zone's reduction is a fact about that Zone alone, so the
/// runtime is keyed by the Zone and can never answer for another one.
fn installed() -> &'static RwLock<BTreeMap<String, Arc<ZoneEmergencyRuntime>>> {
    static INSTALLED: OnceLock<RwLock<BTreeMap<String, Arc<ZoneEmergencyRuntime>>>> =
        OnceLock::new();
    INSTALLED.get_or_init(|| RwLock::new(BTreeMap::new()))
}

/// Install one Zone's runtime, replacing any earlier one.
///
/// The composition root calls this while it builds the Zone's plane, before
/// that plane's manager spawns, so the runtime exists before the first
/// `EmergencyPolicy` row of the Zone can reconcile.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub fn install(runtime: Arc<ZoneEmergencyRuntime>) {
    let mut installed = installed().write().unwrap_or_else(|poisoned| poisoned.into_inner());
    installed.insert(runtime.zone().as_str().to_owned(), runtime);
}

/// The runtime for one Zone, when the composition root installed it.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub fn runtime(zone: &d2b_contracts_resource::v3::ZoneId) -> Option<Arc<ZoneEmergencyRuntime>> {
    installed()
        .read()
        .ok()
        .and_then(|installed| installed.get(zone.as_str()).cloned())
}

/// Forget one Zone's runtime.
///
/// The plane's shutdown path calls this so a Zone whose plane closed cannot
/// keep publishing a reduction into the next plane for the same Zone.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub fn remove(zone: &d2b_contracts_resource::v3::ZoneId) {
    let mut installed = installed().write().unwrap_or_else(|poisoned| poisoned.into_inner());
    installed.remove(zone.as_str());
}
