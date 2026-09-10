//! Endpoint resource driver (U7): the v3 `ResourceDriver` conversion of the
//! daemon-owned Endpoint realization path (R4, R8, R9; F4).
//!
//! The driver covers the transport-unix / purpose `virtiofsd` realization
//! case per preserved behavior (the binding-owned virtiofsd socket, old
//! `binding_child_resource_runtime`): recover probes the socket on the
//! host target, reconcile realizes the socket through the provider port as
//! a long effect, and delete participates in the preserved endpoint-first
//! teardown ordering - the endpoint is removed BEFORE the worker Process
//! child (the binding driver deletes its own endpoint child first, then
//! the worker; the drain finalizer and recycle-with-producer semantics are
//! preserved: an endpoint with `recycle-with-producer` lifecycle goes away
//! with its producer and nothing outlives it).
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> [`EndpointDriverFactory`] registration under `Endpoint`.
//! - `validate_spec` -> [`ResourceDriver::validate`].
//! - `observe` -> [`ResourceDriver::recover`].
//! - socket realization -> [`ResourceDriver::reconcile`].
//! - socket removal -> [`ResourceDriver::delete`].
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only, R11).
#![allow(dead_code)]

use std::sync::Arc;

use d2b_contracts_resource::v3::{
    endpoint::{EndpointClass, EndpointLifecyclePolicy, EndpointSpec, EndpointTransport},
    ResourceRef, ResourceSpec,
};
use d2b_resource_runtime::context::{ResourceContext, SpecDecoder, typed_spec_decoder};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{DriverFailure, DriverOp, FailureClass};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};

/// The one resource type this factory serves (KTD4 Phase A).
pub(crate) const ENDPOINT_TYPE_NAME: &str = "Endpoint";

/// The frozen purpose of the binding-owned virtiofsd socket.
const VIRTIOFSD_PURPOSE: &str = "virtiofsd";

// ---------------------------------------------------------------------------
// Driver error and status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointDriverErrorKind {
    /// The durable spec did not decode as the closed Endpoint contract.
    SpecInvalid,
    /// The spec is an Endpoint shape this driver does not realize.
    ShapeUnsupported,
    /// A provider socket effect failed transiently.
    SocketEffect,
}

impl EndpointDriverErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::SocketEffect => FailureClass::Retryable,
            Self::SpecInvalid | Self::ShapeUnsupported => FailureClass::Terminal,
        }
    }
}

/// Typed driver failure; redacted at the erased boundary through
/// [`ResourceDriver::classify_error`] (R13).
#[derive(Debug, Clone, Copy)]
pub(crate) struct EndpointDriverError {
    kind: EndpointDriverErrorKind,
    op: DriverOp,
}

impl EndpointDriverError {
    fn new(kind: EndpointDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op }
    }
}

impl core::fmt::Display for EndpointDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            EndpointDriverErrorKind::SpecInvalid => "endpoint-spec-invalid",
            EndpointDriverErrorKind::ShapeUnsupported => "endpoint-shape-unsupported",
            EndpointDriverErrorKind::SocketEffect => "endpoint-socket-effect-failed",
        })
    }
}

impl std::error::Error for EndpointDriverError {}

/// Typed in-memory status projection (R11: never persisted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndpointDriverStatus {
    /// A socket realization effect is in flight.
    Realizing,
    /// The endpoint is realized and observable.
    Realized,
}

// ---------------------------------------------------------------------------
// Decoded spec envelope
// ---------------------------------------------------------------------------

/// The spec-store envelope for one Endpoint row (KTD2), exactly as
/// persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EndpointSpecEnvelope {
    raw: Vec<u8>,
    base: d2b_contracts_resource::v3::CanonicalJsonObject,
}

/// The manager-wired decode hook for Endpoint rows. The Endpoint base
/// carries `providerRef` inside its typed contract, so the decoder
/// reconstructs the complete typed object.
pub(crate) fn endpoint_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        serde_json::from_slice::<ResourceSpec>(bytes).map(|spec| EndpointSpecEnvelope {
            raw: bytes.to_vec(),
            base: spec.base_with_provider_ref(),
        })
    })
}

// ---------------------------------------------------------------------------
// Provider effect port
// ---------------------------------------------------------------------------

/// The provider-facing socket effect surface the Endpoint driver needs.
/// The production implementation delegates to the preserved virtiofs
/// endpoint realization; test doubles implement the same seam (R4).
#[async_trait::async_trait]
pub(crate) trait EndpointDriverEffects: Send + Sync + 'static {
    /// Whether the endpoint's socket is currently realized and observable.
    async fn socket_present(&self, producer_ref: &ResourceRef, purpose: &str) -> bool;

    /// Realize the endpoint's socket (transport-unix virtiofsd case).
    async fn ensure_socket(&self, producer_ref: &ResourceRef, purpose: &str)
        -> Result<(), String>;

    /// Remove the endpoint realization - endpoint-first teardown. Idempotent
    /// under retry (R10).
    async fn remove_socket(&self, producer_ref: &ResourceRef, purpose: &str)
        -> Result<(), String>;
}

/// Production effects over the preserved endpoint realization. U9 wires the
/// adapter construction (the same inputs the old serving effect adapter
/// assembled).
pub(crate) struct ProductionEndpointDriverEffects {
    present: Arc<dyn Fn(&ResourceRef, &str) -> bool + Send + Sync>,
    ensure: Arc<dyn AsyncSocketEffect + Send + Sync>,
    remove: Arc<dyn AsyncSocketEffect + Send + Sync>,
}

/// A boxed async socket effect (ensure or remove).
#[async_trait::async_trait]
pub(crate) trait AsyncSocketEffect: Send + Sync {
    async fn run(&self, producer_ref: &ResourceRef, purpose: &str) -> Result<(), String>;
}

impl ProductionEndpointDriverEffects {
    pub(crate) fn new(
        present: Arc<dyn Fn(&ResourceRef, &str) -> bool + Send + Sync>,
        ensure: Arc<dyn AsyncSocketEffect + Send + Sync>,
        remove: Arc<dyn AsyncSocketEffect + Send + Sync>,
    ) -> Self {
        Self { present, ensure, remove }
    }
}

#[async_trait::async_trait]
impl EndpointDriverEffects for ProductionEndpointDriverEffects {
    async fn socket_present(&self, producer_ref: &ResourceRef, purpose: &str) -> bool {
        (self.present)(producer_ref, purpose)
    }

    async fn ensure_socket(
        &self,
        producer_ref: &ResourceRef,
        purpose: &str,
    ) -> Result<(), String> {
        self.ensure.run(producer_ref, purpose).await
    }

    async fn remove_socket(
        &self,
        producer_ref: &ResourceRef,
        purpose: &str,
    ) -> Result<(), String> {
        self.remove.run(producer_ref, purpose).await
    }
}

// ---------------------------------------------------------------------------
// Factory (U9 wiring shape)
// ---------------------------------------------------------------------------

/// Everything the composition unit (U9) must construct to instantiate the
/// Endpoint driver factory for one zone.
pub(crate) struct EndpointDriverArgs {
    pub(crate) zone: String,
    pub(crate) effects: Arc<dyn EndpointDriverEffects>,
}

/// [`ResourceDriverFactory`] for the `Endpoint` resource type. Construction
/// is infallible by contract (R3).
pub(crate) struct EndpointDriverFactory {
    types: [ResourceTypeName; 1],
    args: EndpointDriverArgs,
}

impl EndpointDriverFactory {
    pub(crate) fn new(args: EndpointDriverArgs) -> Self {
        Self {
            types: [ResourceTypeName::new(ENDPOINT_TYPE_NAME)],
            args,
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for EndpointDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(EndpointDriver::new(EndpointDriverArgs {
            zone: self.args.zone.clone(),
            effects: Arc::clone(&self.args.effects),
        }))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One Endpoint resource's driver.
#[derive(Clone)]
pub(crate) struct EndpointDriver {
    zone: String,
    effects: Arc<dyn EndpointDriverEffects>,
}

impl EndpointDriver {
    pub(crate) fn new(args: EndpointDriverArgs) -> Self {
        Self {
            zone: args.zone,
            effects: args.effects,
        }
    }

    fn error(&self, kind: EndpointDriverErrorKind, op: DriverOp) -> EndpointDriverError {
        EndpointDriverError::new(kind, op)
    }

    /// Decode the stored envelope into the strict typed Endpoint contract.
    fn decoded_spec(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<EndpointSpec, EndpointDriverError> {
        let envelope = ctx
            .spec::<EndpointSpecEnvelope>()
            .map_err(|_| self.error(EndpointDriverErrorKind::SpecInvalid, op))?;
        serde_json::from_slice::<EndpointSpec>(&envelope.base.to_canonical_bytes())
            .map_err(|_| self.error(EndpointDriverErrorKind::SpecInvalid, op))
    }

    /// The daemon-owned shape this driver realizes: the binding-owned
    /// virtiofsd socket (transport unix, purpose virtiofsd, recycle with
    /// its producer). Any other shape stays on the old reconciler until
    /// its conversion unit.
    fn check_shape(&self, spec: &EndpointSpec, op: DriverOp) -> Result<(), EndpointDriverError> {
        if spec.transport() != EndpointTransport::Unix
            || spec.purpose().as_str() != "virtiofsd"
            || spec.endpoint_class() != EndpointClass::Service
            || spec.lifecycle_policy() != EndpointLifecyclePolicy::RecycleWithProducer
        {
            return Err(self.error(EndpointDriverErrorKind::ShapeUnsupported, op));
        }
        Ok(())
    }

    fn producer_ref(&self, ctx: &ResourceContext, op: DriverOp) -> Result<ResourceRef, EndpointDriverError> {
        let envelope = ctx
            .spec::<EndpointSpecEnvelope>()
            .map_err(|_| self.error(EndpointDriverErrorKind::SpecInvalid, op))?;
        let spec = serde_json::from_slice::<EndpointSpec>(&envelope.base.to_canonical_bytes())
            .map_err(|_| self.error(EndpointDriverErrorKind::SpecInvalid, op))?;
        Ok(spec.producer_ref().clone())
    }
}

#[async_trait::async_trait]
impl ResourceDriver for EndpointDriver {
    type Error = EndpointDriverError;

    fn classify_error(&self, error: &EndpointDriverError) -> DriverFailure {
        match error.kind.class() {
            FailureClass::Retryable => DriverFailure::retryable(error.op),
            FailureClass::Terminal => DriverFailure::terminal(error.op),
        }
    }

    /// Spec decode plus the daemon-owned shape check (old `validate_spec`).
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let spec = self.decoded_spec(ctx, DriverOp::Validate)?;
        self.check_shape(&spec, DriverOp::Validate)?;
        Ok(())
    }

    /// Probe the socket on the host target: present adopts the realized
    /// endpoint, absent waits for reconcile.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let spec = self.decoded_spec(ctx, DriverOp::Recover)?;
        self.check_shape(&spec, DriverOp::Recover)?;
        if self
            .effects
            .socket_present(spec.producer_ref(), spec.purpose().as_str())
            .await
        {
            ctx.set_status(EndpointDriverStatus::Realized);
            Ok(RecoveryOutcome::Adopted)
        } else {
            Ok(RecoveryOutcome::Missing)
        }
    }

    /// One reconcile pass: the socket present converges; otherwise the
    /// realization effect spawns as a long effect (R5: the mailbox never
    /// blocks on it).
    async fn reconcile(&mut self, ctx: &mut ResourceContext) -> Result<ReconcileOutcome, Self::Error> {
        let spec = self.decoded_spec(ctx, DriverOp::Reconcile)?;
        self.check_shape(&spec, DriverOp::Reconcile)?;
        if self
            .effects
            .socket_present(spec.producer_ref(), spec.purpose().as_str())
            .await
        {
            ctx.set_status(EndpointDriverStatus::Realized);
            return Ok(ReconcileOutcome::Satisfied);
        }
        let operation = ctx.begin_operation();
        let effects = Arc::clone(&self.effects);
        let effect_sender = ctx.effect_sender();
        let producer_ref = spec.producer_ref().clone();
        let purpose = spec.purpose().as_str().to_owned();
        tokio::spawn(async move {
            let result = effects.ensure_socket(&producer_ref, &purpose).await;
            let effect_result = match result {
                Ok(()) => d2b_resource_runtime::context::EffectResult::Completed,
                Err(_) => d2b_resource_runtime::context::EffectResult::Failed(
                    DriverFailure::retryable(DriverOp::Reconcile),
                ),
            };
            let _ = effect_sender.send(d2b_resource_runtime::context::EffectCompleted {
                operation,
                result: effect_result,
            });
        });
        ctx.set_status(EndpointDriverStatus::Realizing);
        Ok(ReconcileOutcome::InProgress { operation })
    }

    /// Teardown: remove the socket realization. Endpoint-first ordering is
    /// preserved: the endpoint driver's own effect runs BEFORE the worker
    /// Process child is deleted (the binding driver's delete encodes the
    /// full ordering; this driver supplies the endpoint leg of it).
    /// Idempotent under retry (R10).
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let Ok(spec) = self.decoded_spec(ctx, DriverOp::Delete) else {
            // Nothing durable to clean up; converged without effects.
            return Ok(());
        };
        self.effects
            .remove_socket(spec.producer_ref(), spec.purpose().as_str())
            .await
            .map_err(|_| self.error(EndpointDriverErrorKind::SocketEffect, DriverOp::Delete))
    }
}

// ---------------------------------------------------------------------------
// Tests: driver unit tests over a scripted socket port (R4; ordering and
// idempotence observed through the recorded calls).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use d2b_contracts_resource::v3::{
        endpoint::{
            EndpointClass, EndpointConsumerPolicy,
            EndpointLifecyclePolicy, EndpointLocality, EndpointSpec, EndpointTransport,
            EndpointVisibility,
        },
        execution_policy::BoundedToken,
        ResourceRef,
    };
    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext,
        WatchId, WatchRegistration,
    };
    use d2b_resource_runtime::driver::{
        DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriverFactory,
    };
    use d2b_resource_runtime::error::{FailureClass, ResourceError};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::target::TargetHandle;

    use super::{EndpointDriverArgs, EndpointDriverFactory, endpoint_spec_decoder};

    // -- fakes ---------------------------------------------------------------

    /// Scripted socket port: records every call in order.
    struct FakeSocketEffects {
        calls: parking_lot::Mutex<Vec<&'static str>>,
        present: std::sync::atomic::AtomicBool,
    }

    impl FakeSocketEffects {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: parking_lot::Mutex::new(Vec::new()),
                present: std::sync::atomic::AtomicBool::new(false),
            })
        }

        fn call_order(&self) -> Vec<&'static str> {
            self.calls.lock().clone()
        }

        fn make_present(&self) {
            self.present.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl super::EndpointDriverEffects for FakeSocketEffects {
        async fn socket_present(&self, _producer_ref: &ResourceRef, _purpose: &str) -> bool {
            self.calls.lock().push("socket-present");
            self.present.load(std::sync::atomic::Ordering::SeqCst)
        }

        async fn ensure_socket(
            &self,
            _producer_ref: &ResourceRef,
            _purpose: &str,
        ) -> Result<(), String> {
            self.calls.lock().push("ensure-socket");
            self.make_present();
            Ok(())
        }

        async fn remove_socket(
            &self,
            _producer_ref: &ResourceRef,
            _purpose: &str,
        ) -> Result<(), String> {
            self.calls.lock().push("remove-socket");
            self.present.store(false, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    /// Dead manager: these Endpoint flows make no manager calls.
    struct DeadManager;

    #[async_trait::async_trait]
    impl ManagerEndpoint for DeadManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            _child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            Err(ResourceError::ManagerRpc("dead".into()))
        }

        async fn get(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Err(ResourceError::ManagerRpc("dead".into()))
        }

        async fn delete(&self, _key: &ResourceKey) -> Result<(), ResourceError> {
            Err(ResourceError::ManagerRpc("dead".into()))
        }

        async fn list_owned(
            &self,
            _owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Err(ResourceError::ManagerRpc("dead".into()))
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            Err(ResourceError::ManagerRpc("dead".into()))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    struct NullRequeue;

    impl RequeueScheduler for NullRequeue {
        fn schedule(&self, _key: ResourceKey, _after: std::time::Duration) -> RequeueId {
            RequeueId(0)
        }

        fn cancel(&self, _id: RequeueId) {}
    }

    // -- fixtures ------------------------------------------------------------


    fn test_row(spec: &EndpointSpec) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", "Endpoint", "endpoint"),
            uid: [0x42; 16],
            generation: 1,
            owner_uid: Some([0x43; 16]),
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: serde_json::to_vec(spec).expect("endpoint spec bytes"),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    fn fixture(row: StoredDesiredResource) -> ResourceContext {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        ResourceContext::new(
            row,
            TargetHandle::Host,
            endpoint_spec_decoder(),
            Arc::new(DeadManager),
            Arc::new(NullRequeue),
            effects_tx,
            notify_tx,
        )
    }

    async fn driver(effects: Arc<FakeSocketEffects>) -> Box<dyn DynResourceDriver> {
        let factory = EndpointDriverFactory::new(EndpointDriverArgs {
            zone: "work".to_owned(),
            effects,
        });
        factory
            .create(&ResourceKey::new("work", "Endpoint", "endpoint"))
            .await
    }

    // -- factory -------------------------------------------------------------

    #[tokio::test]
    async fn factory_registers_only_the_endpoint_resource_type() {
        let factory = EndpointDriverFactory::new(EndpointDriverArgs {
            zone: "work".to_owned(),
            effects: FakeSocketEffects::new(),
        });
        assert_eq!(factory.resource_types().len(), 1);
        assert_eq!(factory.resource_types()[0].as_str(), "Endpoint");
        factory
            .create(&ResourceKey::new("work", "Endpoint", "endpoint"))
            .await;
    }

    // -- realize happy path ----------------------------------------------------

    #[tokio::test]
    async fn reconcile_realizes_the_socket_through_a_long_effect() {
        let fake = FakeSocketEffects::new();
        let mut ctx = fixture(test_row(&virtiofsd_endpoint_spec()));
        let mut d = driver(fake.clone()).await;

        d.validate(&mut ctx).await.expect("validate");
        assert_eq!(
            d.recover(&mut ctx).await.expect("recover"),
            RecoveryOutcome::Missing,
            "socket absent: nothing to adopt"
        );

        // Pass one spawns the realization effect; the mailbox never blocks
        // (R5).
        match d.reconcile(&mut ctx).await.expect("reconcile") {
            ReconcileOutcome::InProgress { .. } => {}
            other => panic!("expected InProgress, got {other:?}"),
        }
        // Let the spawned effect task reach its send.
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
        assert!(fake.call_order().contains(&"ensure-socket"));

        // Pass two observes the realized socket and reports satisfied.
        assert_eq!(
            d.reconcile(&mut ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            ctx.status::<super::EndpointDriverStatus>(),
            Some(&super::EndpointDriverStatus::Realized)
        );
    }

    fn virtiofsd_endpoint_spec() -> EndpointSpec {
        let producer = ResourceRef::parse("Process/vol-worker").expect("producer");
        EndpointSpec::new(
            ResourceRef::parse("Provider/volume-virtiofs").expect("provider"),
            producer,
            EndpointClass::Service,
            EndpointTransport::Unix,
            BoundedToken::parse("virtiofsd").expect("purpose"),
            None,
            EndpointLocality::HostLocal,
            EndpointVisibility::Provider,
            d2b_contracts_resource::v3::endpoint::EndpointAttachmentPolicy::new(false, 0)
                .expect("attachment policy"),
            EndpointConsumerPolicy::new(
                vec![ResourceRef::parse("Provider/volume-virtiofs").expect("subject")],
                vec![],
                vec![
                    d2b_contracts_resource::v3::endpoint::EndpointOperation::Resolve,
                    d2b_contracts_resource::v3::endpoint::EndpointOperation::Observe,
                ],
            )
            .expect("consumer policy"),
            EndpointLifecyclePolicy::RecycleWithProducer,
        )
        .expect("endpoint spec")
    }

    // -- recover: adopt a realized socket ----------------------------------------

    #[tokio::test]
    async fn recover_adopts_a_realized_socket() {
        let fake = FakeSocketEffects::new();
        fake.make_present();
        let mut ctx = fixture(test_row(&virtiofsd_endpoint_spec()));
        let mut d = driver(fake).await;
        assert_eq!(
            d.recover(&mut ctx).await.expect("recover"),
            RecoveryOutcome::Adopted
        );
    }

    // -- delete: endpoint-first teardown leg --------------------------------------

    #[tokio::test]
    async fn delete_removes_the_socket_before_any_worker_teardown() {
        let fake = FakeSocketEffects::new();
        let mut ctx = fixture(test_row(&virtiofsd_endpoint_spec()));
        let mut d = driver(fake.clone()).await;

        d.delete(&mut ctx).await.expect("delete");
        assert_eq!(
            fake.call_order(),
            vec!["remove-socket"],
            "endpoint driver removes the socket; the binding driver's delete runs \
             this endpoint leg BEFORE the worker Process deletion"
        );
        // Retry is idempotent (R10).
        d.delete(&mut ctx).await.expect("delete retry");
        assert_eq!(
            fake.call_order(),
            vec!["remove-socket", "remove-socket"]
        );
    }

    // -- shape guard -----------------------------------------------------------------

    #[tokio::test]
    async fn non_virtiofsd_shapes_are_rejected_at_validate() {
        let mut spec = virtiofsd_endpoint_spec();
        let replacement = EndpointSpec::new(
            ResourceRef::parse("Provider/volume-virtiofs").expect("provider"),
            ResourceRef::parse("Process/vol-worker").expect("producer"),
            EndpointClass::Service,
            EndpointTransport::Tcp,
            BoundedToken::parse("virtiofsd").expect("purpose"),
            None,
            EndpointLocality::HostLocal,
            EndpointVisibility::Provider,
            d2b_contracts_resource::v3::endpoint::EndpointAttachmentPolicy::new(false, 0)
                .expect("attachment policy"),
            EndpointConsumerPolicy::new(
                vec![ResourceRef::parse("Provider/volume-virtiofs").expect("subject")],
                vec![],
                vec![d2b_contracts_resource::v3::endpoint::EndpointOperation::Resolve],
            )
            .expect("consumer policy"),
            EndpointLifecyclePolicy::RecycleWithProducer,
        )
        .expect("endpoint spec");
        let _ = spec;
        let mut ctx = fixture(test_row(&replacement));
        let mut d = driver(FakeSocketEffects::new()).await;
        let failure = d.validate(&mut ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal);
    }

    #[tokio::test]
    async fn malformed_spec_decodes_to_a_terminal_failure() {
        let row = StoredDesiredResource {
            spec: serde_json::json!({ "nonsense": true }).to_string().into_bytes(),
            ..test_row(&virtiofsd_endpoint_spec())
        };
        let mut ctx = fixture(row);
        let mut d = driver(FakeSocketEffects::new()).await;
        let failure = d.reconcile(&mut ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal);
    }
}
