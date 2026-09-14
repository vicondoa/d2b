//! Cause-carrying reconcile: why a pass runs, and what it may do.
//!
//! Reconciling without the cause is what forces a driver to poll and forces
//! platform layers to encode readiness for it. Every pass here carries the
//! reason it was requested - its own spec changed, a resource it targets
//! changed, or a resource whose spec targets it changed - and the context
//! carries the reverse-target list the driver would otherwise scan for.
//!
//! The authoritative driver contract lives in `d2b-resource-runtime`
//! (`ResourceDriver`, invoked by the resource actor with its own context and
//! cause vocabulary). This module is the toolkit-facing seam at the same
//! shape: the base's test harness drives a [`ReconcileTarget`] with these
//! causes, and the two vocabularies converge when the runtime's cause type is
//! re-exported to the toolkit.

use std::fmt;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{CanonicalJsonObject, ResourceRef, ZoneId};
use d2b_resource_types::ChildCreation;

use super::ChildCreationFailure;

/// The longest requeue delay a driver may ask for.
pub const MAX_REQUEUE_AFTER_MS: u64 = 60_000;

/// Why one reconcile pass was requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileCause {
    /// The resource's own committed desired spec changed.
    OwnedSpecChanged,
    /// The resource's own status changed.
    OwnedStatusChanged,
    /// A resource this resource's spec targets changed.
    TargetChanged(ResourceRef),
    /// A resource whose spec targets this resource changed.
    DependentChanged(ResourceRef),
    /// The driver asked to be called again.
    Requeue,
    /// A declared watch fired.
    WatchFired,
}

impl ReconcileCause {
    /// The stable lower-kebab code for this cause.
    pub fn code(&self) -> &'static str {
        match self {
            Self::OwnedSpecChanged => "owned-spec-changed",
            Self::OwnedStatusChanged => "owned-status-changed",
            Self::TargetChanged(_) => "target-changed",
            Self::DependentChanged(_) => "dependent-changed",
            Self::Requeue => "requeue",
            Self::WatchFired => "watch-fired",
        }
    }
}

impl fmt::Display for ReconcileCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

/// The child-creation seam one reconcile pass creates children through.
///
/// The seam is async and fallible because a creation crosses the declaration
/// fence and the resource store; a driver never retries a refusal.
#[async_trait]
pub trait CreateChild: Send + Sync {
    /// Create one declared child, or refuse terminally.
    async fn create_child(
        &self,
        declaration: &ChildCreation,
        name: &str,
        spec: CanonicalJsonObject,
    ) -> Result<(), ChildCreationFailure>;
}

/// Everything one reconcile pass may read.
pub struct ReconcileCtx<'a> {
    /// The zone the resource lives in.
    pub zone: &'a ZoneId,
    /// The resource being reconciled.
    pub resource: &'a ResourceRef,
    /// The committed desired spec.
    pub spec: &'a CanonicalJsonObject,
    /// Every resource whose declared refs point at `resource`.
    pub targets: &'a [ResourceRef],
    /// The child-creation fence for the declaring driver.
    pub creations: &'a dyn CreateChild,
    /// The current time, from the plane's clock.
    pub now_unix_ms: u64,
}

impl fmt::Debug for ReconcileCtx<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReconcileCtx")
            .field("zone", &self.zone)
            .field("resource", &self.resource)
            .field("target_count", &self.targets.len())
            .field("now_unix_ms", &self.now_unix_ms)
            .finish_non_exhaustive()
    }
}

/// The result of one reconcile pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileOutcome {
    /// The resource is satisfied; the runtime publishes Ready.
    Ready,
    /// The resource is not satisfied yet, and the driver asks for another
    /// pass after a bounded delay.
    NotYet {
        /// The bounded requeue delay.
        requeue_after_ms: u64,
    },
    /// The pass failed, with a closed operator-facing code.
    Failed {
        /// The closed failure code.
        code: &'static str,
    },
}

impl ReconcileOutcome {
    /// The stable lower-kebab class for this outcome.
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotYet { .. } => "not-yet",
            Self::Failed { .. } => "failed",
        }
    }

    /// The bounded requeue delay, or zero when the pass does not requeue.
    pub const fn requeue_after_ms(&self) -> u64 {
        match self {
            Self::NotYet { requeue_after_ms } => {
                if *requeue_after_ms > MAX_REQUEUE_AFTER_MS {
                    MAX_REQUEUE_AFTER_MS
                } else {
                    *requeue_after_ms
                }
            }
            Self::Ready | Self::Failed { .. } => 0,
        }
    }
}

/// One driver's reconcile body, as the base drives it.
#[async_trait]
pub trait ReconcileTarget: Send + Sync {
    /// Run one cause-carrying reconcile pass.
    async fn reconcile(&self, ctx: ReconcileCtx<'_>, cause: &ReconcileCause) -> ReconcileOutcome;
}

/// Why the harness refused to run a reconcile pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileRefusal {
    /// The resource was never admitted, so there is no committed spec to
    /// reconcile.
    UnadmittedRow(ResourceRef),
    /// A cause named a resource the store does not hold.
    UnresolvedTarget(ResourceRef),
}

impl ReconcileRefusal {
    /// The stable lower-kebab code for this refusal.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnadmittedRow(_) => "unadmitted-row",
            Self::UnresolvedTarget(_) => "unresolved-target",
        }
    }
}

impl fmt::Display for ReconcileRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ReconcileRefusal {}
