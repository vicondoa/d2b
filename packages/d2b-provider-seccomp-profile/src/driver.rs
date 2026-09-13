//! The SeccompProfile resource driver: the v3 `ResourceDriver` conversion of the Core
//! baseline reconciler for `SeccompProfile` rows.
//!
//! `SeccompProfile` is a posture row: it declares the device-node binds and the posture a role references. The type is declared here and its rows materialize in the policy-rows unit, so this crate ships the declaration and the driver shell.
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> [`SeccompProfileDriverFactory`] registration through the type's
//!   descriptor ([`seccomp_profile_descriptor`]).
//! - `validate_spec` -> [`ResourceDriver::validate`]: the stored spec envelope
//!   must decode as the JSON spec object every core row stores. The old fence
//!   was "the canonical row JSON is non-empty", so a row whose spec is absent
//!   or undecodable is the same corrupt row.
//! - `plan` -> folded into [`ResourceDriver::reconcile`]: the old plan was an
//!   empty effect list (the `!ensure_finalizer` flag only selected the legacy
//!   constructor `CoreResourceReconciler::new`, which production never used).
//! - `observe` -> [`ResourceDriver::recover`]: the old `ObservationResult` was
//!   converged, so recovery adopts without effects.
//! - `prepare_finalize`/`execute_finalize` -> [`ResourceDriver::finalize`]:
//!   owned children retire first (their own finalize-before-delete pass, F3).
//!   The type's descriptor finalizer was released unconditionally on the old
//!   delete path, so the type itself holds no drain state.
//! - `finalize` -> [`ResourceDriver::delete`]: the manager committed the
//!   durable deleting mark (R10) and the type realizes no target-local state.
//! - `assess_update`/`plan_upgrade`/`execute_upgrade` -> no KTD3 equivalent:
//!   the old handler assessed every row `Current` with preserve-state and
//!   planned a no-op restart; the runner's upgrade path is gone with the
//!   runner (R30).
//! - `UpdateStatus` -> [`ResourceContext::set_status`] (in-memory only, R11):
//!   the type publishes no status of its own.
//! - `DependencySnapshot` -> the manager's owned children, read by the drain
//!   step exactly as the old finalizer window held them.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_resource_runtime::context::{ResourceContext, SpecDecoder, typed_spec_decoder};
use d2b_resource_runtime::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKinds,
    ResourceError,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_types::{AllowedSources, DriverDescriptor, WellKnownType};
use serde_json::Value;

/// The one resource type this driver serves.
pub const SECCOMP_PROFILE_TYPE_NAME: &str = "SeccompProfile";

/// The resource verbs the `SeccompProfile` type supports.
///
/// Derived from the v3 resource plane's converted-type verb surface: the
/// closed `RoleResourceVerb` set minus the two Credential-scoped credential
/// verbs (`use-credential`, `admin-credential`), which the plane gates to the
/// `Credential` type. Every converted type is served by the same manager
/// verbs, and Role rules and the typed CLI nouns resolve their gating from
/// this declaration.
const SECCOMP_PROFILE_VERBS: &[&str] = &[
    "get",
    "list",
    "watch",
    "create",
    "update-spec",
    "update-status",
    "update-metadata",
    "update-finalizers",
    "delete",

];

/// The execution domains the `SeccompProfile` type can be reconciled in.
///
/// Derived from the placement contract: `SeccompProfile` names no placement anchor
/// (`PlacementAnchor::canonical_for` resolves none), so a `SeccompProfile` row never
/// carries the canonical `spec.executionRef` and the plane reconciles it on
/// its own Host domain.
const SECCOMP_PROFILE_EXECUTION_DOMAINS: &[&str] = &["host"];
/// The resource types the `SeccompProfile` driver reads while reconciling.
///
/// The type realizes no target-local state and observes no other row: it
/// converges as metadata, so the declaration names no read.
const SECCOMP_PROFILE_READS: &[WellKnownType] = &[];


/// The manager-wired decode hook for `SeccompProfile` rows: the stored spec envelope is
/// the JSON spec object the core types store (they have no typed core spec,
/// and the old handler worked from canonical JSON).
pub fn seccomp_profile_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| serde_json::from_slice::<Value>(bytes))
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// [`ResourceDriverFactory`] for the `SeccompProfile` resource type. Construction is
/// infallible by contract (R3): the driver carries no fallible setup and no
/// target-local state.
pub struct SeccompProfileDriverFactory {
    types: [ResourceTypeName; 1],
}

impl SeccompProfileDriverFactory {
    /// Construct the one-type factory.
    pub fn new() -> Self {
        Self {
            types: [ResourceTypeName::new(SECCOMP_PROFILE_TYPE_NAME)],
        }
    }
}

impl Default for SeccompProfileDriverFactory {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ResourceDriverFactory for SeccompProfileDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(SeccompProfileDriver)
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One `SeccompProfile` resource's driver.
pub struct SeccompProfileDriver;

#[async_trait]
impl ResourceDriver for SeccompProfileDriver {
    type Error = DriverFailure;

    fn classify_error(&self, error: &DriverFailure) -> DriverFailure {
        error.clone()
    }

    /// The stored spec object fence (old `validate_spec`).
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        spec_object(ctx, DriverOp::Validate)?;
        Ok(())
    }

    /// The old `observe` was converged with nothing to adopt: the family
    /// realizes no target-local state.
    async fn recover(
        &mut self,
        _ctx: &mut ResourceContext,
    ) -> Result<RecoveryOutcome, Self::Error> {
        Ok(RecoveryOutcome::Adopted)
    }

    /// One reconcile pass. The old handler converged as soon as its finalizer
    /// bookkeeping was current, which the manager's durable deleting mark now
    /// owns.
    async fn reconcile(
        &mut self,
        _ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        Ok(ReconcileOutcome::Satisfied)
    }

    /// The drain step before [`ResourceDriver::delete`] (R10, F3): owned
    /// children retire first, and the actor requeues while any of them is
    /// still live.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        drain_owned_children(ctx).await
    }

    /// Teardown: the durable deleting mark is already committed (R10) and the
    /// type realizes nothing on a target.
    async fn delete(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Shared core-type fences
// ---------------------------------------------------------------------------

/// The stored spec object fence every core type shares: the manager's decode
/// hook must have produced a JSON object, which is exactly what the old
/// `validate_spec` required of the canonical row JSON.
fn spec_object(ctx: &ResourceContext, op: DriverOp) -> Result<Value, DriverFailure> {
    let spec = ctx.spec::<Value>().map_err(|error| {
        DriverFailure::refused(op, FailureKinds::CORE_SPEC_INVALID)
            .with_detail(FailureDetail::at("spec/decode").with_note(error.to_string()))
    })?;
    if !spec.is_object() {
        let shape = match &spec {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        };
        return Err(DriverFailure::refused(op, FailureKinds::CORE_SPEC_INVALID).with_detail(
            FailureDetail::at("spec/shape").comparison(FailureComparison::new(
                "spec.shape",
                "object",
                shape,
            )),
        ));
    }
    Ok(spec.clone())
}

/// The child-first drain: every owned child is nudged through its own
/// finalize-before-delete pass, and the row requeues while any child row is
/// still live. The erased boundary runs the same pass before the driver's own
/// drain, so this call is the ordering guarantee the driver owns rather than
/// the only one.
async fn drain_owned_children(ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
    match ctx.finalize_owned_resources().await {
        Ok(()) => Ok(()),
        Err(ResourceError::ChildrenDraining { .. }) => Err(DriverFailure::not_yet(
            DriverOp::Delete,
            FailureKinds::CORE_DRAIN_PENDING,
        )),
        Err(error) => Err(DriverFailure::error(
            DriverOp::Delete,
            FailureKinds::CORE_DEPENDENCY_READ_FAILED,
            FailureClass::Retryable,
        )
        .with_detail(
            FailureDetail::at("finalize/children")
                .comparison(FailureComparison::new(
                    "owned.children",
                    "finalize answered",
                    "read failed",
                ))
                .with_note(error.to_string()),
        )),
    }
}

// ---------------------------------------------------------------------------
// Registration: the type's driver declaration
// ---------------------------------------------------------------------------

/// The `SeccompProfile` type's driver declaration.
///
/// The type is declared but not yet materialized: this crate ships the type's
/// declaration and its driver shell, and the type's rows commit in the
/// policy-rows unit. The `BUILTIN` mask carries the presence obligation -
/// only a declared row may ever satisfy the type - and no other driver can
/// ever claim it.
///
/// The type is not exportable: `ResourceExport` admits only qualified
/// `*.d2bus.org.*Service` types, so a `SeccompProfile` row is never an export subject.
pub fn seccomp_profile_descriptor() -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::SECCOMP_PROFILE,
        allowed_sources: AllowedSources::BUILTIN,
        verbs: SECCOMP_PROFILE_VERBS,
        execution: SECCOMP_PROFILE_EXECUTION_DOMAINS,
        exportable: false,
        reads: SECCOMP_PROFILE_READS,
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: seccomp_profile_spec_decoder(),
        factory: Arc::new(SeccompProfileDriverFactory::new()),
    }
}

// ---------------------------------------------------------------------------
// Tests: driver behavior over a scripted manager.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext, WatchId,
        WatchRegistration,
    };
    use d2b_resource_runtime::driver::{
        DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriverFactory,
    };
    use d2b_resource_runtime::error::{DriverFailure, DriverOp, FailureClass, ResourceError};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::target::TargetHandle;
    use serde_json::json;

    use super::{
        SECCOMP_PROFILE_TYPE_NAME, SeccompProfileDriverFactory, seccomp_profile_spec_decoder,
    };

    /// The target row's deterministic uid; its owned children carry the same
    /// owner uid, which is what the drain's owned-child read selects on.
    const TARGET_UID: [u8; 16] = [
        0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x41, 0x11, 0x81, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
        0x11,
    ];

    /// Scripted manager: the owned-child set the drain reads, the calls the
    /// driver made, and a switch that fails every read.
    #[derive(Default)]
    struct ScriptedManager {
        owned: Mutex<Vec<StoredDesiredResource>>,
        calls: Mutex<Vec<&'static str>>,
        fail_reads: AtomicBool,
    }

    impl ScriptedManager {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        fn with_owned(rows: Vec<StoredDesiredResource>) -> Arc<Self> {
            Arc::new(Self {
                owned: Mutex::new(rows),
                calls: Mutex::new(Vec::new()),
                fail_reads: AtomicBool::new(false),
            })
        }

        fn call_order(&self) -> Vec<&'static str> {
            self.calls.lock().expect("calls").clone()
        }

        fn fail_reads(&self) {
            self.fail_reads.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl ManagerEndpoint for ScriptedManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            _child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            Err(ResourceError::ManagerRpc("unexpected ensure_child".into()))
        }

        async fn get(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Ok(None)
        }

        async fn view(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ResourceError> {
            Ok(None)
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.calls.lock().expect("calls").push("delete");
            self.owned.lock().expect("owned").retain(|row| row.key != *key);
            Ok(())
        }

        async fn list_owned(
            &self,
            _owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            self.calls.lock().expect("calls").push("list-owned");
            if self.fail_reads.load(Ordering::SeqCst) {
                return Err(ResourceError::ManagerRpc("scripted read failure".into()));
            }
            Ok(self.owned.lock().expect("owned").clone())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            Ok(WatchId(1))
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

    fn spec_bytes(value: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&value).expect("spec bytes")
    }

    fn target_row(spec: serde_json::Value) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", SECCOMP_PROFILE_TYPE_NAME, "sample"),
            uid: TARGET_UID,
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Nix,
            deleting: false,
            spec: spec_bytes(spec),
            metadata: serde_json::to_vec(&json!({ "generation": 1 })).expect("metadata"),
            created_at: 0,
        }
    }

    fn owned_child(type_name: &str) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", type_name, "child"),
            uid: [0x22; 16],
            generation: 1,
            owner_uid: Some(TARGET_UID),
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: spec_bytes(json!({})),
            metadata: serde_json::to_vec(&json!({ "generation": 1 })).expect("metadata"),
            created_at: 0,
        }
    }

    fn context(
        target: StoredDesiredResource,
        manager: Arc<ScriptedManager>,
    ) -> ResourceContext {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        ResourceContext::new(
            target,
            TargetHandle::Host,
            seccomp_profile_spec_decoder(),
            manager,
            Arc::new(NullRequeue),
            effects_tx,
            notify_tx,
        )
    }

    async fn driver(spec: serde_json::Value, manager: Arc<ScriptedManager>) -> (ResourceContext, Box<dyn DynResourceDriver>) {
        let ctx = context(target_row(spec), manager);
        let driver = SeccompProfileDriverFactory::new()
            .create(&ResourceKey::new("work", SECCOMP_PROFILE_TYPE_NAME, "sample"))
            .await;
        (ctx, driver)
    }

    /// The stored spec envelope must be the JSON spec object every core row
    /// stores; anything else is the same corrupt row the old `validate_spec`
    /// refused.
    #[tokio::test]
    async fn validate_refuses_a_spec_that_is_not_an_object() {
        let manager = ScriptedManager::new();
        let (mut ctx, mut driver) = driver(json!("not-an-object"), Arc::clone(&manager)).await;
        let failure = driver.validate(&mut ctx).await.expect_err("terminal refusal");
        assert_eq!(failure.kind().code(), "core-spec-invalid");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Validate);
        assert!(
            manager.call_order().is_empty(),
            "validate must not touch the manager"
        );
    }

    /// The bootstrap rows store `{}`: the old fence was "the row is not
    /// empty", not "the spec has fields".
    #[tokio::test]
    async fn validate_admits_the_empty_spec_object() {
        let manager = ScriptedManager::new();
        let (mut ctx, mut driver) = driver(json!({}), Arc::clone(&manager)).await;
        driver.validate(&mut ctx).await.expect("the empty spec object is admitted");
    }

    /// Recovery and deletion realize nothing on a target and read no rows.
    #[tokio::test]
    async fn recover_and_delete_converge_without_manager_calls() {
        let manager = ScriptedManager::new();
        let (mut ctx, mut driver) = driver(json!({}), Arc::clone(&manager)).await;
        assert_eq!(driver.recover(&mut ctx).await.expect("adopted"), RecoveryOutcome::Adopted);
        driver.delete(&mut ctx).await.expect("converged");
        assert!(manager.call_order().is_empty());
    }

    /// One reconcile pass converges as metadata: no manager read, no effect,
    /// and no status the type would claim.
    #[tokio::test]
    async fn reconcile_converges_without_effects() {
        let manager = ScriptedManager::new();
        let (mut ctx, mut driver) = driver(json!({ "artifactId": "sample" }), Arc::clone(&manager)).await;
        assert_eq!(driver.reconcile(&mut ctx).await.expect("converged"), ReconcileOutcome::Satisfied);
        assert!(manager.call_order().is_empty());
        assert!(ctx.status::<serde_json::Value>().is_none());
    }

    /// The drain is child-first: a live owned child defers the row, and once
    /// the children are gone the drain converges.
    #[tokio::test]
    async fn finalize_drains_owned_children_first() {
        let manager = ScriptedManager::with_owned(vec![owned_child("Process")]);
        let (mut ctx, mut driver) = driver(json!({}), Arc::clone(&manager)).await;
        let failure = driver.finalize(&mut ctx).await.expect_err("a live owned child defers");
        assert_eq!(failure.kind().code(), "children-draining");
        assert_eq!(failure.op(), DriverOp::Delete);
        assert!(
            manager.call_order().iter().any(|call| *call == "delete"),
            "the child is nudged through its own finalize-before-delete pass"
        );

        manager.owned.lock().expect("owned").clear();
        driver.finalize(&mut ctx).await.expect("no owned child remains");
    }

    /// A manager read the drain cannot answer is retryable, never terminal.
    #[tokio::test]
    async fn a_failed_child_read_is_retryable() {
        let manager = ScriptedManager::new();
        manager.fail_reads();
        let (mut ctx, mut driver) = driver(json!({}), Arc::clone(&manager)).await;
        let failure: DriverFailure = driver.finalize(&mut ctx).await.expect_err("read failure");
        assert_eq!(failure.class(), FailureClass::Retryable);
    }
}
