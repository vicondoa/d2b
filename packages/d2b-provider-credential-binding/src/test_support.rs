//! Recording test doubles shared with this crate's and downstream crates'
//! unit tests.
//!
//! Gated behind the `test-support` Cargo feature so production consumers
//! never pull them in.
//!
//! The double is the scripted delivery port: it answers `observe` with the
//! session it was constructed to hold, mints a session from the request on
//! `deliver`, and counts revocations. It also holds the credential material a
//! real adapter would mint inside the session and never records it anywhere,
//! so a test can assert that no log line, status projection, or committed row
//! carries it.
//!
//! The double's ordered log is the toolkit's `SharedLog`
//! (`d2b_provider_toolkit::testing`), the canonical recorder shape every
//! family crate's test-support module shares.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use d2b_provider_toolkit::testing::SharedLog;

use crate::driver::{
    CredentialBindingDriverEffects, CredentialDelivery, CredentialRevocation, DeliveredSession,
};
use crate::facets::{
    CredentialBindingEffectFacets, CredentialClock, CredentialDeliverySource,
    CredentialRevocationSource,
};

/// The credential material a real delivery adapter mints inside the admitted
/// session.
///
/// The double holds it so a test can prove the material never reaches a log
/// line, a status projection, or a committed row. It is deliberately a value
/// the double can see and the driver cannot: the driver holds identity,
/// vocabulary, counters, and bounds only.
pub const SCRIPTED_MATERIAL: &str = "material::scripted-credential-material";

/// Scripted delivery port over the caller's ordered log, so a test asserts one
/// sequence across manager calls and delivery effects.
///
/// The double is immutable after construction apart from its counters, its
/// clock, and its failure switches: what `observe` answers is fixed when the
/// double is built, which is what makes "nothing is live" and "this exact
/// session is live" two distinguishable scenarios rather than a mutable state
/// a test has to unwind.
pub struct FakeDeliveryEffects {
    log: SharedLog,
    now_unix_ms: AtomicU64,
    /// The live session this double holds, if it was built to hold one.
    held: Option<DeliveredSession>,
    minted: AtomicU64,
    observed: AtomicU64,
    revoked: AtomicU64,
    fail_deliver: AtomicBool,
    fail_observe: AtomicBool,
    fail_revoke: AtomicBool,
    /// The credential material a real adapter mints inside the session.
    ///
    /// The double holds it so a test can assert that nothing the driver
    /// publishes carries it. It never reaches the ordered log, which is why
    /// the log is the surface a leak check reads.
    material: String,
}

impl FakeDeliveryEffects {
    /// A fresh double whose destination holds no delivery.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::with_log(SharedLog::new()))
    }

    /// A fresh double whose destination holds no delivery, sharing the
    /// caller's ordered log with a manager recorder so manager calls and
    /// delivery effects read as one sequence.
    pub fn with_log(log: SharedLog) -> Self {
        Self {
            log,
            now_unix_ms: AtomicU64::new(1_760_000_000_000),
            held: None,
            minted: AtomicU64::new(0),
            observed: AtomicU64::new(0),
            revoked: AtomicU64::new(0),
            fail_deliver: AtomicBool::new(false),
            fail_observe: AtomicBool::new(false),
            fail_revoke: AtomicBool::new(false),
            material: SCRIPTED_MATERIAL.to_owned(),
        }
    }

    /// A double whose destination already holds `session`.
    ///
    /// This is the pre-restart shape a recovering driver adopts: the delivery
    /// is live, so nothing may mint a second one.
    pub fn holding(session: DeliveredSession) -> Arc<Self> {
        Arc::new(Self {
            held: Some(session),
            ..Self::with_log(SharedLog::new())
        })
    }

    /// Script the clock the delivery window is bounded against.
    pub fn set_now_unix_ms(&self, now_unix_ms: u64) {
        self.now_unix_ms.store(now_unix_ms, Ordering::SeqCst);
    }

    /// Script `deliver` to fail with a delivery error.
    pub fn set_fail_deliver(&self, fail: bool) {
        self.fail_deliver.store(fail, Ordering::SeqCst);
    }

    /// Script `observe` to fail: an unanswerable target, never a revoked one.
    pub fn set_fail_observe(&self, fail: bool) {
        self.fail_observe.store(fail, Ordering::SeqCst);
    }

    /// Script `revoke` to fail with a delivery error.
    pub fn set_fail_revoke(&self, fail: bool) {
        self.fail_revoke.store(fail, Ordering::SeqCst);
    }

    /// The credential material this double's deliveries mint.
    pub fn material(&self) -> &str {
        &self.material
    }

    /// The ordered delivery-effect log, shared with any manager logger.
    pub fn call_order(&self) -> Vec<String> {
        self.log.entries()
    }

    /// How many deliveries this double established.
    pub fn deliveries(&self) -> u64 {
        self.minted.load(Ordering::SeqCst)
    }

    /// How many observations answered a live delivery.
    pub fn observations(&self) -> u64 {
        self.observed.load(Ordering::SeqCst)
    }

    /// How many revocations this double performed.
    pub fn revocations(&self) -> u64 {
        self.revoked.load(Ordering::SeqCst)
    }

    /// The facet set the plane and this crate's tests build the driver and the
    /// effects service from: this scripted double behind the four facets.
    pub fn facet_set(self: &Arc<Self>) -> CredentialBindingEffectFacets {
        CredentialBindingEffectFacets {
            delivery: Arc::new(ScriptedDelivery(Arc::clone(self))),
            revocation: Arc::new(ScriptedRevocation(Arc::clone(self))),
            clock: Arc::new(ScriptedClock(Arc::clone(self))),
        }
    }

    /// The session one delivery request establishes.
    ///
    /// The identity is the request's own: the port answers with what it
    /// delivered to, never with a session for some other destination.
    fn mint(delivery: &CredentialDelivery, sequence: u64) -> DeliveredSession {
        DeliveredSession::new(
            delivery.destination().clone(),
            delivery.source_uid().clone(),
            delivery.destination_uid().clone(),
            delivery.source_generation(),
            delivery.destination_generation(),
            sequence,
            delivery.expires_unix_ms(),
        )
    }
}

/// The scripted delivery facet: records the call on the shared double and
/// answers from the double's script.
struct ScriptedDelivery(Arc<FakeDeliveryEffects>);

#[async_trait::async_trait]
impl CredentialDeliverySource for ScriptedDelivery {
    async fn deliver(&self, delivery: &CredentialDelivery) -> Result<DeliveredSession, String> {
        self.0.deliver(delivery).await
    }

    async fn observe(
        &self,
        delivery: &CredentialDelivery,
    ) -> Result<Option<DeliveredSession>, String> {
        self.0.observe(delivery).await
    }
}

/// The scripted revocation facet: records the call on the shared double.
struct ScriptedRevocation(Arc<FakeDeliveryEffects>);

#[async_trait::async_trait]
impl CredentialRevocationSource for ScriptedRevocation {
    async fn revoke(&self, revocation: &CredentialRevocation) -> Result<(), String> {
        self.0.revoke(revocation).await
    }
}

/// The scripted clock facet: the double's scripted instant.
struct ScriptedClock(Arc<FakeDeliveryEffects>);

impl CredentialClock for ScriptedClock {
    fn now_unix_ms(&self) -> u64 {
        self.0.now_unix_ms.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl CredentialBindingDriverEffects for FakeDeliveryEffects {
    async fn deliver(&self, delivery: &CredentialDelivery) -> Result<DeliveredSession, String> {
        self.log
            .record(format!("deliver:{}", delivery.destination()));
        if self.fail_deliver.load(Ordering::SeqCst) {
            return Err("scripted delivery failure".to_owned());
        }
        let sequence = self.minted.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(Self::mint(delivery, sequence))
    }

    async fn observe(
        &self,
        delivery: &CredentialDelivery,
    ) -> Result<Option<DeliveredSession>, String> {
        self.log
            .record(format!("observe:{}", delivery.destination()));
        if self.fail_observe.load(Ordering::SeqCst) {
            return Err("scripted observation failure".to_owned());
        }
        let Some(held) = &self.held else {
            return Ok(None);
        };
        self.observed.fetch_add(1, Ordering::SeqCst);
        Ok(Some(held.clone()))
    }

    async fn revoke(&self, revocation: &CredentialRevocation) -> Result<(), String> {
        self.log.record(match revocation.destination() {
            Some(destination) => format!("revoke:{destination}"),
            None => format!("revoke-row:{}", revocation.binding()),
        });
        if self.fail_revoke.load(Ordering::SeqCst) {
            return Err("scripted revocation failure".to_owned());
        }
        self.revoked.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn now_unix_ms(&self) -> u64 {
        self.now_unix_ms.load(Ordering::SeqCst)
    }
}
