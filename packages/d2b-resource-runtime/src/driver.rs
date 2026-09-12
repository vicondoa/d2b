//! ResourceDriver contract (U4, KTD3; spec sections 11, 13, 14).

pub const MODULE_NAME: &str = "driver";

use async_trait::async_trait;

use crate::context::{OperationId, ResourceContext};
use crate::error::DriverFailure;
use crate::error::DriverOp;
use crate::error::FailureKinds;
use crate::identity::{ResourceKey, ResourceTypeName};

/// Outcome of driver recovery (F2; spec sections 9-10): discovery and
/// adoption on the resource's realization target. The actor publishes the
/// result as in-memory status; recovery never persists (R6, R11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// A live target resource matched the adoption identity; adopted.
    Adopted,
    /// Nothing on the target matched the adoption identity; the resource
    /// must be created on the next reconcile. This is the target-probe
    /// analogue of an `Absent` [`crate::context::RowLookup`] (issue #511):
    /// absence waits for reconcile, it never fails the pass.
    Missing,
    /// Target state of this type exists but matches no adoption identity;
    /// quarantined per resource policy (R15).
    Quarantined,
}

/// Outcome of one reconcile pass (spec section 11). The actor owns
/// scheduling: `Satisfied` lets it go ready, `InProgress` leaves the mailbox
/// responsive while the effect runs (R5); completion arrives as
/// `EffectCompleted { operation }` (spec section 14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileOutcome {
    /// Desired state satisfied; the actor may go ready.
    Satisfied,
    /// A long effect was spawned with the operation id from
    /// [`crate::context::ResourceContext::begin_operation`].
    InProgress { operation: OperationId },
}

/// Provider-facing driver contract (KTD3, R3; spec section 11).
///
/// One implementation per resource type, produced by its
/// [`ResourceDriverFactory`] through the
/// [`crate::provider::ProviderDirectory`]. The actor owns scheduling,
/// retries, status publication, dependencies, and lifecycle; the driver owns
/// resource-specific validate/recover/reconcile/delete behavior.
///
/// Error redaction mirrors `HandlerFailure` in
/// `packages/d2b-controller-toolkit/src/runner.rs:400-437`, extended for issue
/// #508: `type Error` stays inside the provider and
/// [`ResourceDriver::classify_error`] maps it onto the structured
/// [`DriverFailure`] surface (registered kind, verdict, stage, compared
/// values), which is all the erased boundary ([`DynResourceDriver`], what the
/// actor holds) reports. Drivers report failures only through that surface,
/// so the actor owns retry/backoff and retry state stays runtime-only (R13).
///
/// Conversion template for a current `ResourceReconciler` implementor (spec
/// section 13):
///
/// ```ignore
/// #[async_trait::async_trait]
/// impl ResourceDriver for ProcessDriver {
///     type Error = ProcessDriverError;
///
///     fn classify_error(&self, error: &ProcessDriverError) -> DriverFailure {
///         match error {
///             ProcessDriverError::NotReady => DriverFailure::not_yet(
///                 DriverOp::Reconcile,
///                 FailureKinds::PROCESS_DRAIN_PENDING,
///             ),
///             ProcessDriverError::Refused => DriverFailure::refused(
///                 DriverOp::Reconcile,
///                 FailureKinds::PROCESS_SPEC_INVALID,
///             ),
///         }
///     }
///
///     async fn reconcile(&mut self, ctx: &mut ResourceContext)
///         -> Result<ReconcileOutcome, Self::Error> { ... }
/// }
/// ```
#[async_trait]
pub trait ResourceDriver: Send + 'static {
    /// Driver-internal error type; stays inside the provider. Mapped onto the
    /// structured failure surface at the erased boundary through
    /// [`ResourceDriver::classify_error`].
    type Error: std::error::Error + Send + Sync + 'static;

    /// Map implementation errors onto the structured failure surface (issue
    /// #508): the registered [`crate::error::FailureKind`], the structural
    /// [`crate::error::DriverVerdict`], the stage, and the compared values
    /// where a comparison happened. The driver knows which operations can
    /// fail how and which values were compared; classify accordingly.
    ///
    /// The verdict is the retry contract:
    ///
    /// - [`crate::error::DriverVerdict::NotYet`] - defer and requeue, never
    ///   terminal. Issue #511's default applies here: absence (`Absent`) and
    ///   an unanswerable plane (`Unavailable`) are not terminal.
    /// - [`crate::error::DriverVerdict::Refused`] - a terminal decision
    ///   against the row, requiring named terminal evidence (a committed row
    ///   that cannot decode, a structurally invalid spec, a refused
    ///   resolution). Refusals surface through the driver's status
    ///   projection or the actor's failure log.
    /// - [`crate::error::DriverVerdict::Error`] - an operational failure
    ///   carrying the driver's own [`crate::error::FailureClass`].
    ///
    /// The actor prints the returned failure's log line and publishes its
    /// wire layer; both are projections of the same structured detail.
    fn classify_error(&self, error: &Self::Error) -> DriverFailure;

    /// Structural validation of the current desired spec (spec section 13:
    /// `validate_spec()`).
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error>;

    /// Discovery and adoption on the resource's target (F2, R15-R16): exact
    /// match adopts, missing creates on the next reconcile, unexpected
    /// quarantines per resource policy.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error>;

    /// One reconcile pass (spec section 13: `plan()`, `reconcile()`, and
    /// `assess_update()` fold in here). Long effects spawn and return
    /// [`ReconcileOutcome::InProgress`]; the driver must never block the
    /// actor mailbox on external work (R5, KTD12).
    async fn reconcile(&mut self, ctx: &mut ResourceContext) -> Result<ReconcileOutcome, Self::Error>;

    /// Drain step: called by the actor immediately before
    /// [`ResourceDriver::delete`] for EVERY resource whose durable deleting
    /// mark is already committed (R10, F3). The driver performs the
    /// resource's drain work here - waiting for dependents to release what
    /// the deletion must not cut through - and returns `Ok(())` once it is
    /// safe to tear down. A failure the classification calls retryable
    /// requeues another delete pass, so `finalize` must be idempotent under
    /// retry.
    ///
    /// Default: no drain work. A driver whose type has nothing to drain
    /// converges in `delete` exactly as before this step existed.
    async fn finalize(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Teardown (spec section 13: `prepare_finalize`, `execute_finalize`,
    /// `finalize` fold in here). Idempotent under retry: the durable
    /// deleting mark is already committed when this runs (R10).
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error>;
}

/// Object-erased driver surface: what the resource actor holds as
/// `Box<dyn DynResourceDriver>` (U3). Typed errors stop at this boundary -
/// they cannot cross `dyn` - so failures are redacted into
/// [`crate::error::DriverFailure`] via [`ResourceDriver::classify_error`].
///
/// The two-trait split is deliberate: spec section 11's single
/// `Box<dyn ResourceDriver>` is not expressible once methods return
/// `Self::Error` (object-safety), and the actor must see only the closed
/// classification to own retry/backoff (R13).
#[async_trait]
pub trait DynResourceDriver: Send + 'static {
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), DriverFailure>;
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, DriverFailure>;
    async fn reconcile(&mut self, ctx: &mut ResourceContext) -> Result<ReconcileOutcome, DriverFailure>;
    /// Drain step; the actor runs it immediately before
    /// [`DynResourceDriver::delete`] on every resource.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), DriverFailure>;
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), DriverFailure>;
}

#[async_trait]
impl<D: ResourceDriver> DynResourceDriver for D {
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
        self.validate(ctx).await.map_err(|error| self.classify_error(&error))
    }

    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, DriverFailure> {
        self.recover(ctx).await.map_err(|error| self.classify_error(&error))
    }

    async fn reconcile(&mut self, ctx: &mut ResourceContext) -> Result<ReconcileOutcome, DriverFailure> {
        self.reconcile(ctx).await.map_err(|error| self.classify_error(&error))
    }

    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
        // EVERY resource finalizes what it owns before its own drain work
        // (owner directive 2026-09-11): the call lives here, at the erased
        // boundary the actor drives, so no driver implementation can skip
        // it - a driver's own `finalize` body always runs after its owned
        // children have been driven through their finalize-before-delete
        // pass. The failure is a registered `NotYet` (`children-draining`):
        // the actor defers and requeues, so a parent whose children are
        // still retiring requeues instead of blocking.
        ctx.finalize_owned_resources().await.map_err(|_| {
            DriverFailure::not_yet(DriverOp::Delete, FailureKinds::CHILDREN_DRAINING)
        })?;
        ResourceDriver::finalize(self, ctx).await.map_err(|error| self.classify_error(&error))
    }

    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
        self.delete(ctx).await.map_err(|error| self.classify_error(&error))
    }
}

/// Produces drivers for the resource types it covers (KTD3). Registered
/// into the [`crate::provider::ProviderDirectory`] under
/// [`ResourceDriverFactory::resource_types`].
///
/// Construction is infallible by contract: anything that can fail is
/// resource-specific behavior and surfaces through the driver's
/// `validate`/`recover`, where the actor's scheduling and retry policy owns
/// it (R3).
#[async_trait]
pub trait ResourceDriverFactory: Send + Sync + 'static {
    /// Resource types this factory produces drivers for; must match the
    /// `type_name` component of the keys it accepts.
    fn resource_types(&self) -> &[ResourceTypeName];

    /// Build the erased driver for one desired resource.
    async fn create(&self, key: &ResourceKey) -> Box<dyn DynResourceDriver>;
}

 #[cfg(test)]
 mod tests {
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::time::Duration;

    use super::{
        DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver,
    };
    use crate::context::test_support::{fixture, test_row, DeadManager, FailingDecoder, NullRequeue};
    use crate::context::{EffectCompleted, EffectResult, ResourceContext};
    use crate::error::{DriverFailure, DriverOp, FailureClass};

    // -- Erased boundary classification ---------------------------------------

    /// Converted-provider shape: typed errors stay inside the driver and are
    /// mapped through `classify_error` at the erased boundary; the actor sees
    /// only the closed retryable/terminal class (R13: it owns retry/backoff).
    #[derive(Clone)]
    struct ClassifyingDriver {
        failure: Option<ClassifiedFailure>,
    }
    #[derive(Debug, Clone, Copy, thiserror::Error)]
    enum ClassifiedFailure {
        #[error("transient provider failure")]
        Transient,
        #[error("fatal provider failure")]
        Fatal,
    }

    #[async_trait::async_trait]
    impl ResourceDriver for ClassifyingDriver {
        type Error = ClassifiedFailure;

        fn classify_error(&self, error: &ClassifiedFailure) -> DriverFailure {
            match error {
                ClassifiedFailure::Transient => DriverFailure::retryable(DriverOp::Reconcile),
                ClassifiedFailure::Fatal => DriverFailure::terminal(DriverOp::Reconcile),
            }
        }

        async fn validate(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn recover(&mut self, _ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
            Ok(RecoveryOutcome::Missing)
        }

        async fn reconcile(&mut self, _ctx: &mut ResourceContext) -> Result<ReconcileOutcome, Self::Error> {
            match self.failure.as_ref() {
                None => Ok(ReconcileOutcome::Satisfied),
                Some(ClassifiedFailure::Transient) => Err(ClassifiedFailure::Transient),
                Some(ClassifiedFailure::Fatal) => Err(ClassifiedFailure::Fatal),
            }
        }

        async fn delete(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn erased_driver_boundary_reports_retryable_and_terminal_classes() {
        let fixture = fixture(
            test_row("z", "Process", "worker-0"),
            DeadManager,
            NullRequeue,
            Arc::new(FailingDecoder),
        );
        let mut ctx = fixture.ctx;

        let mut transient: Box<dyn DynResourceDriver> = Box::new(ClassifyingDriver {
            failure: Some(ClassifiedFailure::Transient),
        });
        let failure = transient.reconcile(&mut ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.op(), DriverOp::Reconcile);

        let mut fatal: Box<dyn DynResourceDriver> = Box::new(ClassifyingDriver {
            failure: Some(ClassifiedFailure::Fatal),
        });
        let failure = fatal.reconcile(&mut ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Reconcile);

        // The satisfied path crosses the boundary unchanged.
        let mut ok: Box<dyn DynResourceDriver> = Box::new(ClassifyingDriver { failure: None });
        assert_eq!(ok.reconcile(&mut ctx).await.unwrap(), ReconcileOutcome::Satisfied);
        assert_eq!(ok.recover(&mut ctx).await.unwrap(), RecoveryOutcome::Missing);
        ok.validate(&mut ctx).await.unwrap();
        ok.delete(&mut ctx).await.unwrap();
    }

    // -- Long effects (R5; spec section 14) -----------------------------------

    /// Long-effect contract: reconcile spawns the external work and returns
    /// `InProgress` immediately - the driver never blocks on the effect - and
    /// completion arrives as the typed `EffectCompleted` message with the
    /// operation id from `begin_operation`. U3 wires the actor side (mailbox
    /// delivery); this proves the shapes and the non-blocking return with
    /// tokio paused time.
    struct EffectDriver;

    #[async_trait::async_trait]
    impl ResourceDriver for EffectDriver {
        type Error = Infallible;

        fn classify_error(&self, _error: &Infallible) -> DriverFailure {
            match *_error {}
        }

        async fn validate(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn recover(&mut self, _ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
            Ok(RecoveryOutcome::Missing)
        }

        async fn reconcile(&mut self, ctx: &mut ResourceContext) -> Result<ReconcileOutcome, Self::Error> {
            let operation = ctx.begin_operation();
            let effects = ctx.effect_sender();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(600)).await;
                let _ = effects.send(EffectCompleted {
                    operation,
                    result: EffectResult::Completed,
                });
            });
            Ok(ReconcileOutcome::InProgress { operation })
        }

        async fn delete(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn long_effect_returns_in_progress_and_completes_as_a_message() {
        let fixture = fixture(
            test_row("z", "Process", "worker-0"),
            DeadManager,
            NullRequeue,
            Arc::new(FailingDecoder),
        );
        let mut ctx = fixture.ctx;
        let mut effects = fixture.effects;

        let mut driver: Box<dyn DynResourceDriver> = Box::new(EffectDriver);
        let outcome = driver.reconcile(&mut ctx).await.expect("reconcile");
        let operation = match outcome {
            ReconcileOutcome::InProgress { operation } => operation,
            other => panic!("expected InProgress, got {other:?}"),
        };

        // The effect task must register its timer before time advances.
        tokio::task::yield_now().await;

        // The spawned effect has not completed yet: reconcile returned while
        // the effect was still running.
        tokio::time::advance(Duration::from_secs(599)).await;
        tokio::task::yield_now().await;
        assert!(effects.try_recv().is_err(), "no completion before the effect finishes");

        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        let completed = effects.try_recv().expect("typed completion message");
        assert_eq!(completed.operation, operation, "the completion carries the operation id");
        assert!(matches!(completed.result, EffectResult::Completed));
        assert!(effects.try_recv().is_err(), "exactly one completion");
    }
}