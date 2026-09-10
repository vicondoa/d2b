//! Volume resource driver (U7): the v3 `ResourceDriver` conversion of the
//! daemon-owned shared-Volume path (R4, R8, R9; KTD7, F1, F4).
//!
//! The driver keeps the old `SharedVolumeResourceReconciler` Volume leg
//! (`packages/d2bd/src/resource_runtime/volume_provider_runtime.rs`,
//! `reconcile_volume`) and nothing else: recover discovers existing
//! volume-local layout state on the host target, reconcile runs the
//! preserved volume-local layout effect and then derives one deterministic
//! `VolumeBinding` child per virtiofs attachment through the manager-routed
//! ensure (the child spec is committed BEFORE the child actor exists, F1),
//! and delete removes the Volume's own layout state. Child teardown on
//! parent delete is the manager's reconcile_children diff (R9/F3): the
//! manager marks children deleting and drives bindings -> endpoint ->
//! process-last ordering; this driver's delete covers only the Volume's own
//! effect, with the drain finalizer preserved behind the provider port.
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> [`VolumeDriverFactory`] registration under `Volume`.
//! - `validate_spec` -> [`ResourceDriver::validate`].
//! - `observe` -> [`ResourceDriver::recover`].
//! - layout effect + `volume_children` ensure -> [`ResourceDriver::reconcile`].
//! - volume-local cleanup -> [`ResourceDriver::delete`].
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only, R11).
//!
//! U9 wires this factory into the composition; until the cutover the
//! module surface is only exercised from its tests.
#![allow(dead_code)]
use std::sync::Arc;

use d2b_contracts_resource::v3::{
    ResourceName, ResourceRef, ResourceSpec, ResourceTypeName as ContractResourceTypeName,
    ResourceUid,
    volume::{AttachmentAccess, VolumeSpec},
};
use d2b_provider_volume_local::{LayoutPhase, VolumeLocalController, desired_binding_intents};
use d2b_resource_runtime::context::{
    ChildEnsure, ResourceContext, SpecDecoder, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{DriverFailure, DriverOp, FailureClass};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};

/// The one resource type this factory serves (KTD4 Phase A).
pub(crate) const VOLUME_TYPE_NAME: &str = "Volume";

/// The fixed Provider this driver serves (old `SharedVolumeResourceKind`).
const VOLUME_PROVIDER_NAME: &str = "volume-local";

/// The serving Provider the derived `VolumeBinding` rows select.
const BINDING_PROVIDER_REF: &str = "Provider/volume-virtiofs";

/// Deterministic `VolumeBinding` child type (old
/// `VOLUME_BINDING_RESOURCE_TYPE`).
const VOLUME_BINDING_TYPE: &str = "VolumeBinding";

// ---------------------------------------------------------------------------
// Driver error and status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VolumeDriverErrorKind {
    /// The durable spec did not decode as the closed Volume contract.
    SpecInvalid,
    /// The spec selected a Provider this driver does not own.
    ProviderUnsupported,
    /// A provider layout effect failed transiently.
    LayoutEffect,
    /// The manager refused a child ensure/delete.
    ChildMutation,
    /// Deterministic child derivation failed (invalid attachment).
    ChildDerivation,
}

impl VolumeDriverErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::LayoutEffect | Self::ChildMutation => FailureClass::Retryable,
            Self::SpecInvalid | Self::ProviderUnsupported | Self::ChildDerivation => {
                FailureClass::Terminal
            }
        }
    }
}

/// Typed driver failure; redacted at the erased boundary through
/// [`ResourceDriver::classify_error`] (R13).
#[derive(Debug, Clone, Copy)]
pub(crate) struct VolumeDriverError {
    kind: VolumeDriverErrorKind,
    op: DriverOp,
}

impl VolumeDriverError {
    fn new(kind: VolumeDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op }
    }
}

impl core::fmt::Display for VolumeDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            VolumeDriverErrorKind::SpecInvalid => "volume-spec-invalid",
            VolumeDriverErrorKind::ProviderUnsupported => "volume-provider-unsupported",
            VolumeDriverErrorKind::LayoutEffect => "volume-layout-effect-failed",
            VolumeDriverErrorKind::ChildMutation => "volume-child-mutation-failed",
            VolumeDriverErrorKind::ChildDerivation => "volume-child-derivation-invalid",
        })
    }
}

impl std::error::Error for VolumeDriverError {}

/// Typed in-memory status projection (R11: never persisted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VolumeDriverStatus {
    /// The layout effect is in flight.
    EnsuringLayout,
    /// Binding children derived; readiness aggregates the child rows this
    /// pass converged (R8/R9: child phases, never a parent-side override).
    ServingChildren { desired: usize, converged: bool },
}

// ---------------------------------------------------------------------------
// Decoded spec envelope
// ---------------------------------------------------------------------------

/// The spec-store envelope for one Volume row (KTD2), exactly as persisted.
/// `raw` keeps the exact stored bytes so audits can assert the driver never
/// mutates the durable envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VolumeSpecEnvelope {
    pub(crate) raw: Vec<u8>,
    provider_ref: Option<ResourceRef>,
    base: d2b_contracts_resource::v3::CanonicalJsonObject,
}

/// The manager-wired decode hook for Volume rows.
pub(crate) fn volume_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        serde_json::from_slice::<ResourceSpec>(bytes).map(|spec| VolumeSpecEnvelope {
            raw: bytes.to_vec(),
            provider_ref: spec.provider_ref().cloned(),
            base: spec.base().clone(),
        })
    })
}

// ---------------------------------------------------------------------------
// Provider effect port
// ---------------------------------------------------------------------------

/// The provider-facing layout effect surface the Volume driver needs. The
/// production implementation drives the already-preserved
/// `VolumeLocalController`; test doubles implement the same seam (R4).
///
/// Object-erased on purpose: the driver holds the port as
/// `Arc<dyn VolumeDriverEffects>` so one factory serves every Volume row.
#[async_trait::async_trait]
pub(crate) trait VolumeDriverEffects: Send + Sync + 'static {
    /// Run the preserved volume-local layout reconcile and report whether
    /// the layout phase reached `Ready`.
    async fn ensure_layout(
        &self,
        volume_uid: &ResourceUid,
        spec: &VolumeSpec,
        provider: Option<&serde_json::Value>,
        owner_ref: Option<&ResourceRef>,
    ) -> Result<bool, String>;

    /// Remove the Volume's own layout state (drain finalizer preserved);
    /// idempotent under retry (R10).
    async fn remove_layout(&self, volume_uid: &ResourceUid, spec: &VolumeSpec)
        -> Result<(), String>;

    /// Discover existing volume-local layout state for this exact uid
    /// (recover probe).
    fn has_layout(&self, volume_uid: &ResourceUid) -> bool;
}

/// Production effects over the preserved `VolumeLocalController`. U9 wires
/// the root resolver/adapter construction (the same inputs the old
/// reconciler assembled inline per call).
pub(crate) struct ProductionVolumeDriverEffects<S, L> {
    controller: Arc<dyn Fn() -> VolumeLocalController<S, L> + Send + Sync>,
    state: Arc<dyn Fn(&ResourceUid) -> bool + Send + Sync>,
}

impl<S, L> ProductionVolumeDriverEffects<S, L> {
    pub(crate) fn new(
        controller: Arc<dyn Fn() -> VolumeLocalController<S, L> + Send + Sync>,
        state: Arc<dyn Fn(&ResourceUid) -> bool + Send + Sync>,
    ) -> Self {
        Self { controller, state }
    }
}

#[async_trait::async_trait]
impl<
    S: d2b_provider_volume_local::VolumeSourceEffectPort + 'static,
    L: d2b_provider_volume_local::VolumeLayoutEffectPort + 'static,
> VolumeDriverEffects for ProductionVolumeDriverEffects<S, L> {
    async fn ensure_layout(
        &self,
        volume_uid: &ResourceUid,
        spec: &VolumeSpec,
        provider: Option<&serde_json::Value>,
        owner_ref: Option<&ResourceRef>,
    ) -> Result<bool, String> {
        let report = (self.controller)()
            .reconcile(volume_uid, spec, provider, owner_ref)
            .await
            .map_err(|error| error.to_string())?;
        Ok(report.layout_phase == LayoutPhase::Ready)
    }

    async fn remove_layout(
        &self,
        volume_uid: &ResourceUid,
        spec: &VolumeSpec,
    ) -> Result<(), String> {
        (self.controller)()
            .cleanup(volume_uid, spec)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn has_layout(&self, volume_uid: &ResourceUid) -> bool {
        (self.state)(volume_uid)
    }
}

// ---------------------------------------------------------------------------
// Factory (U9 wiring shape)
// ---------------------------------------------------------------------------

/// Everything the composition unit (U9) must construct to instantiate the
/// Volume driver factory for one zone.
pub(crate) struct VolumeDriverArgs {
    pub(crate) zone: String,
    pub(crate) effects: Arc<dyn VolumeDriverEffects>,
}

/// [`ResourceDriverFactory`] for the `Volume` resource type. Construction is
/// infallible by contract (R3).
pub(crate) struct VolumeDriverFactory {
    types: [ResourceTypeName; 1],
    args: VolumeDriverArgs,
}

impl VolumeDriverFactory {
    pub(crate) fn new(args: VolumeDriverArgs) -> Self {
        Self {
            types: [ResourceTypeName::new(VOLUME_TYPE_NAME)],
            args,
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for VolumeDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(VolumeDriver::new(VolumeDriverArgs {
            zone: self.args.zone.clone(),
            effects: Arc::clone(&self.args.effects),
        }))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One Volume resource's driver.
#[derive(Clone)]
pub(crate) struct VolumeDriver {
    zone: String,
    effects: Arc<dyn VolumeDriverEffects>,
    /// In-memory layout phase (spec section 32: nothing persisted; recover
    /// re-probes the host state through [`VolumeDriverEffects::has_layout`]).
    layout_ready: Arc<std::sync::atomic::AtomicBool>,
}

/// One desired `VolumeBinding` child derived from a Volume attachment
/// (old `volume_children`). The child key is deterministic from the parent
/// plus the attachment tuple (volume, execution target, view, mount path) -
/// never from the attachment index - so reordering declared attachments
/// never churns identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DesiredBindingChild {
    pub(crate) name: String,
    pub(crate) volume_ref: ResourceRef,
    pub(crate) execution_ref: ResourceRef,
    pub(crate) view: String,
    pub(crate) access: AttachmentAccess,
    pub(crate) mount_path: String,
    /// Exact child spec envelope bytes (neutral binding + serving Provider
    /// reference; no provider extension, KTD1).
    pub(crate) spec: Vec<u8>,
}

impl VolumeDriver {
    pub(crate) fn new(args: VolumeDriverArgs) -> Self {
        Self {
            zone: args.zone,
            effects: args.effects,
            layout_ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn error(&self, kind: VolumeDriverErrorKind, op: DriverOp) -> VolumeDriverError {
        VolumeDriverError::new(kind, op)
    }

    /// Decode the stored envelope and the typed spec in one step.
    fn decoded_spec(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<(VolumeSpecEnvelope, VolumeSpec), VolumeDriverError> {
        let envelope = ctx
            .spec::<VolumeSpecEnvelope>()
            .map_err(|_| self.error(VolumeDriverErrorKind::SpecInvalid, op))?;
        let spec = serde_json::from_slice::<VolumeSpec>(&envelope.base.to_canonical_bytes())
            .map_err(|_| self.error(VolumeDriverErrorKind::SpecInvalid, op))?;
        Ok((envelope.clone(), spec))
    }

    /// Provider check (old `validate_spec`): the Volume must select the
    /// fixed daemon-owned volume-local Provider.
    fn check_provider(&self, envelope: &VolumeSpecEnvelope, op: DriverOp) -> Result<(), VolumeDriverError> {
        match envelope.provider_ref.as_ref() {
            Some(provider_ref)
                if provider_ref.resource_type().as_str() == "Provider"
                    && provider_ref.name().as_str() == VOLUME_PROVIDER_NAME => {}
            _ => return Err(self.error(VolumeDriverErrorKind::ProviderUnsupported, op)),
        }
        Ok(())
    }

    fn volume_ref(&self, ctx: &ResourceContext, op: DriverOp) -> Result<ResourceRef, VolumeDriverError> {
        let resource_type = ContractResourceTypeName::parse(VOLUME_TYPE_NAME.to_owned())
            .map_err(|_| self.error(VolumeDriverErrorKind::SpecInvalid, op))?;
        let name = ResourceName::parse(&ctx.key().name)
            .map_err(|_| self.error(VolumeDriverErrorKind::SpecInvalid, op))?;
        Ok(ResourceRef::new(resource_type, name))
    }

    /// Derive the deterministic `VolumeBinding` children per virtiofs
    /// attachment (old `volume_children`).
    fn desired_children(
        &self,
        volume_ref: &ResourceRef,
        spec: &VolumeSpec,
        op: DriverOp,
    ) -> Result<Vec<DesiredBindingChild>, VolumeDriverError> {
        let intents = desired_binding_intents(volume_ref.clone(), spec, false)
            .map_err(|_| self.error(VolumeDriverErrorKind::ChildDerivation, op))?;
        intents
            .into_iter()
            .map(|intent| {
                // Neutral binding payload only (KTD1): access mode and mount
                // intent. The envelope carries no provider extension or
                // attachment settings; the serving posture is the frozen
                // default.
                let binding = d2b_contracts_resource::v3::volume_binding::VolumeBindingSpec::new(
                    intent.volume_ref().clone(),
                    intent.execution_ref().clone(),
                    intent.view().as_str(),
                    intent.access(),
                    intent.mount_path(),
                )
                .map_err(|_| self.error(VolumeDriverErrorKind::ChildDerivation, op))?;
                let mut binding_spec = serde_json::to_value(&binding)
                    .map_err(|_| self.error(VolumeDriverErrorKind::ChildDerivation, op))?
                    .as_object_mut()
                    .ok_or_else(|| self.error(VolumeDriverErrorKind::ChildDerivation, op))?
                    .clone();
                binding_spec.insert(
                    "providerRef".to_owned(),
                    serde_json::Value::String(BINDING_PROVIDER_REF.to_owned()),
                );
                Ok(DesiredBindingChild {
                    name: intent.name().as_str().to_owned(),
                    volume_ref: intent.volume_ref().clone(),
                    execution_ref: intent.execution_ref().clone(),
                    view: intent.view().as_str().to_owned(),
                    access: intent.access(),
                    mount_path: intent.mount_path().to_owned(),
                    spec: serde_json::to_vec(&serde_json::Value::Object(binding_spec))
                        .map_err(|_| self.error(VolumeDriverErrorKind::ChildDerivation, op))?,
                })
            })
            .collect()
    }

    /// Ensure every desired binding child and retire any owned child this
    /// pass no longer derives (old `reconcile_owned_children` diff, R8/R9).
    /// Each manager reply fires only after the child row commit (F1/AE1).
    async fn reconcile_children(
        &self,
        ctx: &mut ResourceContext,
        desired: &[DesiredBindingChild],
        op: DriverOp,
    ) -> Result<(), VolumeDriverError> {
        for child in desired {
            let ensure = ChildEnsure {
                type_name: ResourceTypeName::new(VOLUME_BINDING_TYPE),
                name: child.name.clone(),
                spec: child.spec.clone(),
                metadata: Vec::new(),
            };
            ctx.ensure_child(ensure)
                .await
                .map_err(|_| self.error(VolumeDriverErrorKind::ChildMutation, op))?;
        }
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(VolumeDriverErrorKind::ChildMutation, op))?;
        for row in owned {
            if row.key.type_name != VOLUME_BINDING_TYPE
                || desired.iter().any(|child| child.name == row.key.name)
            {
                continue;
            }
            // Obsolete child: the manager retires it and owns its own
            // teardown (endpoint -> process last), R9/F3.
            ctx.delete(&row.key)
                .await
                .map_err(|_| self.error(VolumeDriverErrorKind::ChildMutation, op))?;
        }
        Ok(())
    }

    /// Spawn the preserved layout effect as a long effect (R5, KTD12): the
    /// mailbox never blocks on it; completion arrives as
    /// [`d2b_resource_runtime::context::EffectCompleted`].
    fn spawn_layout(
        &mut self,
        ctx: &mut ResourceContext,
        uid: ResourceUid,
        spec: VolumeSpec,
        provider: Option<serde_json::Value>,
    ) -> Result<ReconcileOutcome, VolumeDriverError> {
        let operation = ctx.begin_operation();
        let effects = Arc::clone(&self.effects);
        let effect_sender = ctx.effect_sender();
        let layout_ready = Arc::clone(&self.layout_ready);
        tokio::spawn(async move {
            let result = effects.ensure_layout(&uid, &spec, provider.as_ref(), None).await;
            let effect_result = match result {
                Ok(ready) => {
                    layout_ready.store(ready, std::sync::atomic::Ordering::SeqCst);
                    d2b_resource_runtime::context::EffectResult::Completed
                }
                Err(_) => d2b_resource_runtime::context::EffectResult::Failed(
                    DriverFailure::retryable(DriverOp::Reconcile),
                ),
            };
            let _ = effect_sender.send(d2b_resource_runtime::context::EffectCompleted {
                operation,
                result: effect_result,
            });
        });
        ctx.set_status(VolumeDriverStatus::EnsuringLayout);
        Ok(ReconcileOutcome::InProgress { operation })
    }
}

fn resource_uid(bytes: &[u8; 16]) -> Result<ResourceUid, ()> {
    let mut bytes = *bytes;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let text = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    );
    ResourceUid::parse(text).map_err(|_| ())
}

#[async_trait::async_trait]
impl ResourceDriver for VolumeDriver {
    type Error = VolumeDriverError;

    fn classify_error(&self, error: &VolumeDriverError) -> DriverFailure {
        match error.kind.class() {
            FailureClass::Retryable => DriverFailure::retryable(error.op),
            FailureClass::Terminal => DriverFailure::terminal(error.op),
        }
    }

    /// Spec decode plus provider reference check (old `validate_spec`).
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let (envelope, _) = self.decoded_spec(ctx, DriverOp::Validate)?;
        self.check_provider(&envelope, DriverOp::Validate)?;
        Ok(())
    }

    /// Discover existing volume-local layout state on the host target
    /// (preserved recover behavior): found layout adopts, absent waits for
    /// reconcile.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let (envelope, _) = self.decoded_spec(ctx, DriverOp::Recover)?;
        self.check_provider(&envelope, DriverOp::Recover)?;
        let uid = resource_uid(ctx.uid())
            .map_err(|_| self.error(VolumeDriverErrorKind::SpecInvalid, DriverOp::Recover))?;
        if self.effects.has_layout(&uid) {
            self.layout_ready.store(true, std::sync::atomic::Ordering::SeqCst);
            ctx.set_status(VolumeDriverStatus::ServingChildren { desired: 0, converged: false });
            Ok(RecoveryOutcome::Adopted)
        } else {
            Ok(RecoveryOutcome::Missing)
        }
    }

    /// One reconcile pass: pass one spawns the preserved layout effect
    /// (R5); after its completion the actor re-reconciles and this pass
    /// derives the deterministic binding children and ensures each through
    /// the manager (F1); readiness aggregates the child rows this pass
    /// converged.
    async fn reconcile(&mut self, ctx: &mut ResourceContext) -> Result<ReconcileOutcome, Self::Error> {
        let (envelope, spec) = self.decoded_spec(ctx, DriverOp::Reconcile)?;
        self.check_provider(&envelope, DriverOp::Reconcile)?;
        let uid = resource_uid(ctx.uid())
            .map_err(|_| self.error(VolumeDriverErrorKind::SpecInvalid, DriverOp::Reconcile))?;
        let volume_ref = self.volume_ref(ctx, DriverOp::Reconcile)?;

        if !self.layout_ready.load(std::sync::atomic::Ordering::SeqCst) {
            return self.spawn_layout(ctx, uid, spec, envelope.base.get("provider").map(|value| {
                serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
            }));
        }

        let desired = self.desired_children(&volume_ref, &spec, DriverOp::Reconcile)?;
        self.reconcile_children(ctx, &desired, DriverOp::Reconcile).await?;
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(VolumeDriverErrorKind::ChildMutation, DriverOp::Reconcile))?;
        let converged = desired.iter().all(|child| {
            owned
                .iter()
                .any(|row| row.key.type_name == VOLUME_BINDING_TYPE && row.key.name == child.name)
        });
        ctx.set_status(VolumeDriverStatus::ServingChildren {
            desired: desired.len(),
            converged,
        });
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Teardown of the Volume's own layout effect (idempotent under retry,
    /// R10; the durable deleting mark is already committed). Child teardown
    /// (bindings -> endpoint -> process last) is the manager's
    /// reconcile_children diff on parent delete (R9/F3).
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let Ok((_, spec)) = self.decoded_spec(ctx, DriverOp::Delete) else {
            // Nothing durable to clean up; converged without effects.
            return Ok(());
        };
        let Ok(uid) = resource_uid(ctx.uid()) else {
            return Ok(());
        };
        self.effects
            .remove_layout(&uid, &spec)
            .await
            .map_err(|_| self.error(VolumeDriverErrorKind::LayoutEffect, DriverOp::Delete))
    }
}

// ---------------------------------------------------------------------------
// Tests: driver unit tests over a scripted layout port and a recording
// manager endpoint (R4; F1/AE1 ordering observed as the manager records it).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::AtomicBool};

    use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
    use d2b_contracts_resource::v3::volume::VolumeSpec;
    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext,
        SpecDecoder, WatchId, WatchRegistration,
    };
    use d2b_resource_runtime::driver::{
        DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriverFactory,
    };
    use d2b_resource_runtime::error::{DriverFailure, FailureClass, ResourceError};
    use d2b_resource_runtime::identity::{
        ResourceKey, ResourceProvenance, StoredDesiredResource,
    };
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::target::TargetHandle;
    use super::{VolumeDriverArgs, VolumeDriverFactory, VolumeDriverStatus, volume_spec_decoder};

    // -- fakes ---------------------------------------------------------------

    /// Scripted layout port: records every call in order.
    struct FakeLayoutEffects {
        calls: parking_lot::Mutex<Vec<&'static str>>,
        ready: AtomicBool,
    }

    impl FakeLayoutEffects {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: parking_lot::Mutex::new(Vec::new()),
                ready: AtomicBool::new(false),
            })
        }

        fn call_order(&self) -> Vec<&'static str> {
            self.calls.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl super::VolumeDriverEffects for FakeLayoutEffects {
        async fn ensure_layout(
            &self,
            _volume_uid: &ResourceUid,
            _spec: &VolumeSpec,
            _provider: Option<&serde_json::Value>,
            _owner_ref: Option<&ResourceRef>,
        ) -> Result<bool, String> {
            self.calls.lock().push("ensure-layout");
            self.ready.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(true)
        }

        async fn remove_layout(
            &self,
            _volume_uid: &ResourceUid,
            _spec: &VolumeSpec,
        ) -> Result<(), String> {
            self.calls.lock().push("remove-layout");
            Ok(())
        }

        fn has_layout(&self, _volume_uid: &ResourceUid) -> bool {
            self.calls.lock().push("has-layout");
            self.ready.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    /// Recording manager endpoint over one shared ordered log so tests can
    /// assert commit-before-spawn (F1) and retire/retain behavior. The
    /// in-memory row set emulates the manager's store.
    #[derive(Clone)]
    struct RecordingManager {
        zone: String,
        log: Arc<parking_lot::Mutex<Vec<String>>>,
        rows: Arc<parking_lot::Mutex<Vec<StoredDesiredResource>>>,
        next_uid: Arc<std::sync::atomic::AtomicU64>,
    }

    impl RecordingManager {
        fn new() -> Self {
            Self {
                zone: "work".to_owned(),
                log: Arc::new(parking_lot::Mutex::new(Vec::new())),
                rows: Arc::new(parking_lot::Mutex::new(Vec::new())),
                next_uid: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            }
        }

        fn order(&self) -> Vec<String> {
            self.log.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl ManagerEndpoint for RecordingManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            let id = format!("{}/{}", child.type_name.as_str(), child.name);
            // Record the ensure request first; the commit (this function's
            // row write) happens before the reply, and the spawn
            // notification is recorded only after it.
            self.log.lock().push(format!("ensure:{id}"));
            let next = self
                .next_uid
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut uid = [0u8; 16];
            uid[..8].copy_from_slice(&next.to_be_bytes());
            let row = StoredDesiredResource {
                key: ResourceKey::new(&self.zone, child.type_name.as_str(), &child.name),
                uid,
                generation: 1,
                owner_uid: Some([0x42; 16]),
                provenance: ResourceProvenance::Resource,
                deleting: false,
                spec: child.spec,
                metadata: child.metadata,
                created_at: 0,
            };
            let mut rows = self.rows.lock();
            let outcome = match rows.iter_mut().find(|existing| existing.key == row.key) {
                Some(existing) => {
                    if existing.spec == row.spec {
                        EnsureOutcome::Unchanged(existing.clone())
                    } else {
                        *existing = row.clone();
                        EnsureOutcome::Updated(row.clone())
                    }
                }
                None => {
                    rows.push(row.clone());
                    EnsureOutcome::Created(row.clone())
                }
            };
            drop(rows);
            self.log.lock().push(format!("spawned:{id}"));
            Ok(outcome)
        }

        async fn get(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Ok(self.rows.lock().iter().find(|row| row.key == *key).cloned())
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.log
                .lock()
                .push(format!("delete:{}/{}", key.type_name, key.name));
            self.rows.lock().retain(|row| row.key != *key);
            Ok(())
        }

        async fn list_owned(
            &self,
            owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Ok(self
                .rows
                .lock()
                .iter()
                .filter(|row| row.owner_uid.as_ref() == Some(&owner_uid))
                .cloned()
                .collect())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            Err(ResourceError::ManagerRpc("watches unused in this unit".into()))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    /// Dead requeue: these Volume flows never schedule a requeue.
    struct NullRequeue;

    impl RequeueScheduler for NullRequeue {
        fn schedule(&self, _key: ResourceKey, _after: std::time::Duration) -> RequeueId {
            RequeueId(0)
        }

        fn cancel(&self, _id: RequeueId) {}
    }

    // -- fixtures ------------------------------------------------------------

    /// A minimal valid Volume spec: one localPath source, one write-capable
    /// view, one virtiofs attachment at `mount_path`.
    fn volume_spec_json(mount_path: &str, extra_attachment: bool) -> serde_json::Value {
        let mut attachments = vec![serde_json::json!({
            "executionRef": "Guest/guest-a",
            "transport": "virtiofs",
            "view": "root",
            "access": "read-only",
            "mountPath": mount_path,
            "settings": {},
        })];
        if extra_attachment {
            attachments.push(serde_json::json!({
                "executionRef": "Guest/guest-b",
                "transport": "virtiofs",
                "view": "root",
                "access": "read-only",
                "mountPath": "/mnt/second",
                "settings": {},
            }));
        }
        serde_json::json!({
            "source": {
                "executionRef": "Host/host-system",
                "settings": { "kind": "local-path", "sourcePolicyId": "policy-default" },
            },
            "kind": "durable",
            "layout": [],
            "views": { "root": { "path": "data", "rights": ["read", "traverse", "write"] } },
            "attachments": attachments,
        })
    }

    fn spec_bytes(mount_path: &str, extra_attachment: bool) -> Vec<u8> {
        let mut spec = volume_spec_json(mount_path, extra_attachment);
        let object = spec.as_object_mut().expect("spec object");
        object.insert(
            "providerRef".to_owned(),
            serde_json::Value::String("Provider/volume-local".to_owned()),
        );
        serde_json::to_vec(&spec).expect("canonical volume spec")
    }

    fn test_row(spec: &[u8]) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", "Volume", "data"),
            uid: [0x42; 16],
            generation: 3,
            owner_uid: None,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: spec.to_vec(),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    struct Fixture {
        ctx: ResourceContext,
        effects: tokio::sync::mpsc::UnboundedReceiver<
            d2b_resource_runtime::context::EffectCompleted,
        >,
        manager: RecordingManager,
    }

    fn fixture(row: StoredDesiredResource, manager: RecordingManager) -> Fixture {
        let (effects_tx, effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = ResourceContext::new(
            row,
            TargetHandle::Host,
            volume_spec_decoder(),
            Arc::new(manager.clone()),
            Arc::new(NullRequeue),
            effects_tx,
            notify_tx,
        );
        Fixture { ctx, effects: effects_rx, manager }
    }

    async fn driver(effects: Arc<FakeLayoutEffects>) -> Box<dyn DynResourceDriver> {
        let factory = VolumeDriverFactory::new(VolumeDriverArgs {
            zone: "work".to_owned(),
            effects,
        });
        factory
            .create(&ResourceKey::new("work", "Volume", "data"))
            .await
    }

    async fn settle() {
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    async fn reconcile_to_children(
        d: &mut Box<dyn DynResourceDriver>,
        f: &mut Fixture,
    ) -> ReconcileOutcome {
        // Pass one spawns the layout effect; the mailbox never blocks (R5).
        let first = d.reconcile(&mut f.ctx).await.expect("reconcile one");
        assert!(matches!(first, ReconcileOutcome::InProgress { .. }), "{first:?}");
        let completed = f.effects.recv().await.expect("typed completion");
        assert!(matches!(
            completed.result,
            d2b_resource_runtime::context::EffectResult::Completed
        ));
        // The actor re-reconciles on the effect completion.
        d.reconcile(&mut f.ctx).await.expect("reconcile two")
    }

    // -- factory -------------------------------------------------------------

    #[tokio::test]
    async fn factory_registers_only_the_volume_resource_type() {
        let factory = VolumeDriverFactory::new(VolumeDriverArgs {
            zone: "work".to_owned(),
            effects: FakeLayoutEffects::new(),
        });
        assert_eq!(factory.resource_types().len(), 1);
        assert_eq!(factory.resource_types()[0].as_str(), "Volume");
    }

    // -- ensure: layout effect, then children ---------------------------------

    #[tokio::test]
    async fn ensure_creates_binding_children_after_the_layout_effect() {
        let fake = FakeLayoutEffects::new();
        let manager = RecordingManager::new();
        let mut f = fixture(test_row(&spec_bytes("/mnt/data", false)), manager.clone());
        let mut d = driver(fake.clone()).await;

        d.validate(&mut f.ctx).await.expect("validate");
        assert_eq!(
            d.recover(&mut f.ctx).await.expect("recover"),
            RecoveryOutcome::Missing,
            "no layout yet: nothing to adopt"
        );

        let outcome = reconcile_to_children(&mut d, &mut f).await;
        assert_eq!(outcome, ReconcileOutcome::Satisfied);
        let order = manager.order();
        assert_eq!(
            fake.call_order(),
            vec!["has-layout", "ensure-layout"],
            "recover probe then exactly one layout effect"
        );
        assert!(
            order.iter().any(|entry| entry.starts_with("ensure:VolumeBinding/")),
            "binding child ensured through ctx, order: {order:?}"
        );
        // F1: every child ensure is recorded (committed) BEFORE its spawn
        // notification.
        let binding_ensure = order
            .iter()
            .position(|entry| entry.starts_with("ensure:VolumeBinding/"))
            .expect("ensure recorded");
        let binding_spawn = order
            .iter()
            .position(|entry| entry.starts_with("spawned:VolumeBinding/"))
            .expect("spawn notification recorded");
        assert!(binding_ensure < binding_spawn_index(&order, binding_ensure));
        let status = f
            .ctx
            .status::<super::VolumeDriverStatus>()
            .expect("status");
        assert_eq!(
            *status,
            super::VolumeDriverStatus::ServingChildren { desired: 1, converged: true }
        );
    }

    fn binding_spawn_index(order: &[String], ensure_index: usize) -> usize {
        order[ensure_index..]
            .iter()
            .position(|entry| entry.starts_with("spawned:"))
            .map(|offset| ensure_index + offset)
            .expect("spawn notification after ensure")
    }

    // -- deterministic child identity -----------------------------------------

    #[tokio::test]
    async fn same_parent_and_attachment_derive_the_same_child_key() {
        let manager = RecordingManager::new();
        {
            let mut f = fixture(test_row(&spec_bytes("/mnt/data", false)), manager.clone());
            let mut d = driver(FakeLayoutEffects::new()).await;
            reconcile_to_children(&mut d, &mut f).await;
        }
        let first = manager.order();
        // Same parent + attachment -> exactly one child key, ensured again
        // as Unchanged (no duplicate identity, no churn).
        {
            let mut f = fixture(test_row(&spec_bytes("/mnt/data", false)), manager.clone());
            let mut d = driver(FakeLayoutEffects::new()).await;
            d.recover(&mut f.ctx).await.expect("recover");
            reconcile_to_children(&mut d, &mut f).await;
        }
        let second = manager.order();
        let ensure_count = first
            .iter()
            .filter(|entry| entry.starts_with("ensure:VolumeBinding/"))
            .count();
        let unchanged_count = second
            .iter()
            .filter(|entry| entry.starts_with("ensure:VolumeBinding/"))
            .count();
        assert_eq!(ensure_count, 1);
        assert_eq!(unchanged_count, 2, "same attachment -> same child key, no churn");
        let child_name = first
            .iter()
            .find_map(|entry| entry.strip_prefix("ensure:VolumeBinding/"))
            .expect("binding name recorded")
            .to_owned();
        assert_eq!(
            second
                .iter()
                .find_map(|entry| entry.strip_prefix("ensure:VolumeBinding/")),
            Some(child_name.as_str()),
            "child identity is deterministic"
        );
    }

    // -- parent spec change: retire obsolete, retain matching ------------------

    #[tokio::test]
    async fn parent_spec_change_retires_obsolete_children_and_retains_matching() {
        let manager = RecordingManager::new();
        let mut f = fixture(test_row(&spec_bytes("/mnt/data", false)), manager.clone());
        let mut d = driver(FakeLayoutEffects::new()).await;
        d.recover(&mut f.ctx).await.expect("recover");
        reconcile_to_children(&mut d, &mut f).await;
        let first_child = manager
            .order()
            .iter()
            .find_map(|entry| entry.strip_prefix("ensure:VolumeBinding/"))
            .expect("first binding name")
            .to_owned();

        // Grow the spec: a second attachment. The first child must be
        // retained, a second child created, nothing deleted.
        let grown = test_row(&spec_bytes("/mnt/data", true));
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut ctx2 = ResourceContext::new(
            grown,
            TargetHandle::Host,
            volume_spec_decoder(),
            Arc::new(manager.clone()),
            Arc::new(NullRequeue),
            effects_tx,
            notify_tx,
        );
        d.reconcile(&mut ctx2).await.expect("reconcile grown");
        let ensured = manager
            .order()
            .iter()
            .filter(|entry| entry.starts_with("ensure:VolumeBinding/"))
            .count();
        assert_eq!(ensured, 3, "first retained + two passes over two children");
        assert!(
            !manager
                .order()
                .iter()
                .any(|entry| entry.starts_with("delete:")),
            "matching child retained, no delete on growth"
        );

        // Shrink the spec back: the second child is retired through the
        // manager; the matching child stays.
        let shrunk = test_row(&spec_bytes("/mnt/data", false));
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut ctx3 = ResourceContext::new(
            shrunk,
            TargetHandle::Host,
            volume_spec_decoder(),
            Arc::new(manager.clone()),
            Arc::new(NullRequeue),
            effects_tx,
            notify_tx,
        );
        d.reconcile(&mut ctx3).await.expect("reconcile shrunk");
        let deletes = manager
            .order()
            .iter()
            .filter(|entry| entry.starts_with("delete:VolumeBinding/"))
            .cloned()
            .collect::<Vec<String>>();
        assert_eq!(deletes.len(), 1, "exactly the obsolete child retired");
        assert!(
            !deletes[0].contains(&first_child),
            "matching child never retired, order: {:?}",
            manager.order()
        );
    }

    // -- delete: the Volume's own layout effect -------------------------------

    #[tokio::test]
    async fn delete_removes_the_volume_layout_exactly() {
        let fake = FakeLayoutEffects::new();
        let manager = RecordingManager::new();
        let mut f = fixture(test_row(&spec_bytes("/mnt/data", false)), manager);
        let mut d = driver(fake.clone()).await;
        d.recover(&mut f.ctx).await.expect("recover");
        d.delete(&mut f.ctx).await.expect("delete");
        assert_eq!(
            fake.call_order(),
            vec!["has-layout", "remove-layout"],
            "recover probe then drain-finalizer-gated cleanup"
        );
        // Retry is idempotent (R10).
        d.delete(&mut f.ctx).await.expect("delete retry");
        assert_eq!(
            fake.call_order(),
            vec!["has-layout", "remove-layout", "remove-layout"]
        );
    }

    // -- spec guards -----------------------------------------------------------

    #[tokio::test]
    async fn wrong_provider_is_rejected_at_validate() {
        let mut spec = volume_spec_json("/mnt/data", false);
        spec.as_object_mut().expect("spec object").insert(
            "providerRef".to_owned(),
            serde_json::Value::String("Provider/volume-virtiofs".to_owned()),
        );
        let bytes = serde_json::to_vec(&spec).expect("spec");
        let mut f = fixture(test_row(&bytes), RecordingManager::new());
        let mut d = driver(FakeLayoutEffects::new()).await;
        let failure = d.validate(&mut f.ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal, "provider mismatch is terminal");
    }

    #[tokio::test]
    async fn malformed_spec_decodes_to_a_terminal_failure() {
        let bytes = serde_json::json!({
            "providerRef": "Provider/volume-local",
            "nonsense": true,
        })
        .to_string()
        .into_bytes();
        let mut f = fixture(test_row(&bytes), RecordingManager::new());
        let mut d = driver(FakeLayoutEffects::new()).await;
        let failure = d.reconcile(&mut f.ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal);
    }

}
