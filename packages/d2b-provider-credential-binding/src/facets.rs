//! The declared facets the provider-owned CredentialBinding effects reach
//! daemon state through.
//!
//! The `CredentialBinding` family's driver effects are served by this crate's
//! own implementation (see [`crate::effects_service`]) over the admitted
//! delivery session the credential provider already mints. The daemon-owned
//! reads and writes cross the provider boundary as declared facets rather
//! than daemon calls: the delivery of the credential to one exact
//! destination, the observation of whether that destination still holds the
//! delivery, the revocation that retires it, and the clock the delivery
//! window is bounded against. Nothing here names a daemon state type, and
//! nothing here carries credential material - the material is minted and
//! held inside the delivery session on the far side of
//! [`CredentialDeliverySource::deliver`], and no facet signature has a
//! parameter a secret could ride into a log or a status layer.

use std::sync::Arc;

use async_trait::async_trait;

use crate::driver::{CredentialDelivery, CredentialRevocation, DeliveredSession};

/// The daemon-supplied facet set the provider-owned CredentialBinding
/// effects are built from.
///
/// The composition root supplies the objects; the driver never holds a
/// daemon state type. The four facets are the delivery, the observation of a
/// live delivery, the revocation, and the clock.
#[derive(Clone)]
pub struct CredentialBindingEffectFacets {
    /// The delivery: mint the admitted session for one exact destination and
    /// observe whether that destination still holds it.
    pub delivery: Arc<dyn CredentialDeliverySource>,
    /// The revocation: retire whatever delivery this binding row holds.
    pub revocation: Arc<dyn CredentialRevocationSource>,
    /// The clock the delivery window is bounded against. A delivery is
    /// admitted against a wall-clock deadline and expiry, so the same read
    /// must come from the daemon's clock rather than from a second one here.
    pub clock: Arc<dyn CredentialClock>,
}

/// The daemon-supplied delivery facet: the mint that asks a credential
/// provider for material inside an admitted session, and the observation of
/// what that destination currently holds.
///
/// The request a delivery carries is identity, vocabulary, and bounds only.
/// The material itself is created behind this call and is never returned to
/// the driver, so the driver has nothing to persist and nothing to log.
#[async_trait]
pub trait CredentialDeliverySource: Send + Sync + 'static {
    /// Deliver the credential to the delivery's exact destination under its
    /// admitted audience, operation classes, and lifetime bounds, and answer
    /// the non-secret identity of the session it established.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon-supplied adapter cannot establish the
    /// delivery - a refused source policy, an unreachable destination, or a
    /// failed session establishment. The driver classifies the failure
    /// retryable and re-runs the pass; a delivery that cannot be established
    /// is never recorded as established.
    async fn deliver(&self, delivery: &CredentialDelivery) -> Result<DeliveredSession, String>;

    /// The live delivery this binding row holds at the delivery's exact
    /// destination, if there is one.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon-supplied adapter cannot complete the
    /// observation. The observation fails closed as "no delivery" only for
    /// the destinations the adapter genuinely holds nothing for; an
    /// unreachable target is an `Err`, so a driver never mistakes an
    /// unanswered question for a revoked credential.
    async fn observe(
        &self,
        delivery: &CredentialDelivery,
    ) -> Result<Option<DeliveredSession>, String>;
}

/// The daemon-supplied revocation facet: retire the delivery a binding row
/// holds.
///
/// Revocation is idempotent by contract. A row that never held a delivery,
/// or whose delivery this row's committed spec no longer names, answers
/// `Ok(())` immediately; a delivery that is still live answers `Err` rather
/// than claiming a release it could not prove, so a teardown that cannot
/// retire the credential keeps retrying instead of publishing `Released`.
#[async_trait]
pub trait CredentialRevocationSource: Send + Sync + 'static {
    /// Revoke the delivery this binding row holds.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the revocation could not be proven: the adapter
    /// could not reach the destination, or the protocol's own revocation is
    /// still unconfirmed. The driver defers retryably and keeps the row's
    /// durable deleting mark.
    async fn revoke(&self, revocation: &CredentialRevocation) -> Result<(), String>;
}

/// The daemon-supplied clock the delivery window is bounded against.
///
/// The bound matters: a delivery is authority for a bounded interval, and a
/// window computed against a second clock would be a different authority than
/// the one the delivery session itself is minted under.
pub trait CredentialClock: Send + Sync + 'static {
    /// The current wall-clock instant, in milliseconds since the Unix epoch.
    fn now_unix_ms(&self) -> u64;
}
