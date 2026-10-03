//! The per-Zone Quota runtime the driver is built over (U40, R8).
//!
//! The Quota family's driver is served by this crate's own implementation.
//! One of the two facts a ceiling is measured against is not derivable from
//! the ceiling row: the Zone's **committed usage**. Counting it means reading
//! the committed rows of every type in the Zone and resolving their owner
//! chains, which is the manager's and the store's state, never the policy
//! row's.
//!
//! That census crosses the provider boundary as the per-Zone runtime this
//! module defines. The composition root builds one runtime per Zone and
//! installs it here; the driver looks its own Zone's runtime up. The family
//! holds no daemon state type (R2), and a Zone with no installed runtime gets
//! a driver that publishes nothing rather than one that invents a usage.
//!
//! # Why the runtime is looked up rather than injected
//!
//! The Zone is a property of the durable row, not of the driver's
//! construction: the same descriptor registers the type for every Zone a plane
//! serves, and the row it reconciles names its own Zone. Keying the runtime by
//! Zone is what makes one registered driver correct for all of them, and it is
//! why the lookup cannot return another Zone's census.

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, RwLock};

use crate::quota::{QuotaPolicy, ZoneUsage};

/// The daemon-supplied committed-usage read (U40, R8).
///
/// The census is counted from committed rows alone. A read that failed is not
/// a Zone with no rows: it is a Zone whose usage is unknown, and the driver
/// reports that rather than publishing a ceiling measured against a guess.
#[async_trait::async_trait]
pub trait UsageSource: Send + Sync + 'static {
    /// Count the Zone's committed usage.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the read itself failed. That is distinct from
    /// `Ok(None)`, which is the honest "the rows could not be counted"
    /// answer: a Zone whose usage is unknown is not a Zone with none.
    async fn usage(&self) -> Result<Option<ZoneUsage>, String>;
}

/// One Zone's Quota runtime, installed by the composition root.
pub struct ZoneQuotaRuntime {
    zone: d2b_contracts_resource::v3::ZoneId,
    source: Arc<dyn UsageSource>,
    published: RwLock<Option<QuotaPolicy>>,
    usage: RwLock<ZoneUsage>,
}

impl ZoneQuotaRuntime {
    /// Build the runtime for one Zone over the daemon's usage read.
    ///
    /// The runtime starts with no policy and no usage: a Zone whose `Quota`
    /// row has not committed has no ceiling, which is the honest prior state
    /// rather than a fabricated one.
    pub fn new(zone: d2b_contracts_resource::v3::ZoneId, source: Arc<dyn UsageSource>) -> Self {
        Self {
            zone,
            source,
            published: RwLock::new(None),
            usage: RwLock::new(ZoneUsage::default()),
        }
    }

    /// The Zone this runtime answers for.
    pub const fn zone(&self) -> &d2b_contracts_resource::v3::ZoneId {
        &self.zone
    }

    /// The Zone's committed usage as of the last read.
    ///
    /// A read that could not be counted leaves the previous census in place
    /// rather than zeroing it: dropping to zero would make a Zone that cannot
    /// be measured look like one with nothing in it, and the next mutation
    /// would be admitted against a ceiling it never earned.
    pub async fn usage(&self) -> ZoneUsage {
        match self.source.usage().await {
            Ok(Some(usage)) => {
                self.store_usage(&usage);
                usage
            }
            // A read that could not be counted, and a read that failed
            // outright, both leave the previous census in force rather than
            // zeroing it: dropping to zero would make a Zone that cannot be
            // measured look like one with nothing in it, and the next mutation
            // would be admitted against a ceiling it never earned.
            Ok(None) | Err(_) => self.stored_usage(),
        }
    }

    /// The last census that could be counted.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    pub fn stored_usage(&self) -> ZoneUsage {
        self.usage
            .read()
            .map(|usage| usage.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }

    /// Record the census the next admission measures against.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn store_usage(&self, usage: &ZoneUsage) {
        match self.usage.write() {
            Ok(mut current) => *current = usage.clone(),
            Err(poisoned) => *poisoned.into_inner() = usage.clone(),
        }
    }

    /// Publish the Zone's accepted ceilings and the census they measure
    /// against.
    ///
    /// A poisoned lock is written through rather than skipped: a runtime that
    /// dropped a ceiling because an earlier writer panicked would admit past
    /// the limit the Zone set.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    pub fn publish(&self, policy: &Option<QuotaPolicy>, usage: &ZoneUsage) {
        match self.published.write() {
            Ok(mut current) => *current = policy.clone(),
            Err(poisoned) => *poisoned.into_inner() = policy.clone(),
        }
        self.store_usage(usage);
    }

    /// The ceilings this Zone currently has published.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    pub fn policy(&self) -> Option<QuotaPolicy> {
        self.published
            .read()
            .ok()
            .and_then(|current| current.clone())
    }
}

/// The installed per-Zone runtimes, one per Zone per process.
///
/// A daemon process serves each Zone through its own plane and its own
/// manager, and a Zone's ceiling is a fact about that Zone alone, so the
/// runtime is keyed by the Zone and can never answer for another one.
fn installed() -> &'static RwLock<BTreeMap<String, Arc<ZoneQuotaRuntime>>> {
    static INSTALLED: LazyLock<RwLock<BTreeMap<String, Arc<ZoneQuotaRuntime>>>> =
        LazyLock::new(|| RwLock::new(BTreeMap::new()));
    &INSTALLED
}

/// Install one Zone's runtime, replacing any earlier one.
///
/// The composition root calls this while it builds the Zone's plane, before
/// that plane's manager spawns, so the runtime exists before the first `Quota`
/// row of the Zone can reconcile.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub fn install(runtime: Arc<ZoneQuotaRuntime>) {
    let mut installed = installed().write().unwrap_or_else(|poisoned| poisoned.into_inner());
    installed.insert(runtime.zone().as_str().to_owned(), runtime);
}

/// The runtime for one Zone, when the composition root installed it.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub fn runtime(zone: &d2b_contracts_resource::v3::ZoneId) -> Option<Arc<ZoneQuotaRuntime>> {
    installed()
        .read()
        .ok()
        .and_then(|installed| installed.get(zone.as_str()).cloned())
}

/// Forget one Zone's runtime.
///
/// The plane's shutdown path calls this so a Zone whose plane closed cannot
/// keep a published ceiling alive into the next plane for the same Zone.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub fn remove(zone: &d2b_contracts_resource::v3::ZoneId) {
    let mut installed = installed().write().unwrap_or_else(|poisoned| poisoned.into_inner());
    installed.remove(zone.as_str());
}
