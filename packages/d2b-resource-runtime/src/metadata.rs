//! The declaration-only metadata driver.
//!
//! A declaration-only metadata type is a resource type whose rows are
//! admitted, read, and governed, but which realizes nothing on a target: the
//! row converges as metadata, and whatever state it stands for lives either
//! in the controller session (the quota authority index) or in the family
//! crate that materializes the rows (the Zone status projection, the ZoneLink
//! enrollment-and-cursor machine). The v3 rewrite converted every one of
//! those types the same way, so the conversion lives here and no two types
//! can diverge on it:
//!
//! - `describe` -> the declaration the per-type crate builds through
//!   `d2b-resource-types`, which names the driver, the decoder, and the verbs
//!   declared here;
//! - `validate_spec` -> [`ResourceDriver::validate`]: the stored spec
//!   envelope must decode as the JSON spec object every core row stores. The
//!   old fence was "the canonical row JSON is non-empty", so a row whose spec
//!   is absent or undecodable is the same corrupt row;
//! - `observe` -> [`ResourceDriver::recover`]: the old `ObservationResult`
//!   was converged, so recovery adopts without effects;
//! - `plan` -> folded into [`ResourceDriver::reconcile`]: the old plan was an
//!   empty effect list, and the manager's durable deleting mark (R10) owns
//!   the bookkeeping the old finalizer window held;
//! - `prepare_finalize`/`execute_finalize` -> [`ResourceDriver::finalize`]:
//!   owned children retire first through their own finalize-before-delete
//!   pass (F3), so the type itself holds no drain state;
//! - `finalize` -> [`ResourceDriver::delete`]: the durable deleting mark is
//!   already committed and the type realizes no target-local state;
//! - `assess_update`/`plan_upgrade`/`execute_upgrade` -> no KTD3 equivalent:
//!   the old handler assessed every row `Current` with preserve-state and
//!   planned a no-op restart, and the runner's upgrade path is gone with the
//!   runner (R30);
//! - `UpdateStatus` -> [`ResourceContext::set_status`] (in-memory only, R11):
//!   the type publishes no status of its own.
//!
//! The per-type crate keeps the part that is not shared: the type's identity,
//! the crate's documentation, and the one-line declaration the plane registers
//! the type by.

use std::sync::Arc;

use async_trait::async_trait;
/// The module declared name, asserted by the crate smoke test.
pub const MODULE_NAME: &str = "metadata";

use crate::context::{ResourceContext, SpecDecoder, typed_spec_decoder};
use crate::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
};
use crate::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKinds,
    ResourceError,
};
use crate::identity::{ResourceKey, ResourceTypeName};

use serde_json::Value;

/// The execution domains every declaration-only metadata type is reconciled
/// in.
///
/// Derived from the placement contract: none of these types names a placement
/// anchor (`PlacementAnchor::canonical_for` resolves none), so a row never
/// carries the canonical `spec.executionRef` and the plane reconciles it on
/// its own Host domain.
pub const METADATA_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The manager-wired decode hook for a declaration-only metadata type: the
/// stored spec envelope is the JSON spec object the core types store (they
/// have no typed core spec, and the old handler worked from canonical JSON).
pub fn metadata_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| serde_json::from_slice::<Value>(bytes))
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// [`ResourceDriverFactory`] for one declaration-only metadata type.
///
/// Construction is infallible by contract (R3): the driver carries no
/// fallible setup and no target-local state.
pub struct MetadataDriverFactory {
    types: [ResourceTypeName; 1],
}

impl MetadataDriverFactory {
    /// Construct the one-type factory of the type it serves.
    pub fn new(resource_type: ResourceTypeName) -> Self {
        Self {
            types: [resource_type],
        }
    }
}

#[async_trait]
impl ResourceDriverFactory for MetadataDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(MetadataDriver)
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One declaration-only metadata resource's driver.
pub struct MetadataDriver;

#[async_trait]
impl ResourceDriver for MetadataDriver {
    type Error = DriverFailure;

    fn classify_error(&self, error: &DriverFailure) -> DriverFailure {
        error.clone()
    }

    /// The stored spec object fence (old `validate_spec`).
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        spec_object(ctx, DriverOp::Validate)?;
        Ok(())
    }

    /// The old `observe` was converged with nothing to adopt: the type
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
// Shared fences
// ---------------------------------------------------------------------------

/// The stored spec object fence every declaration-only metadata type shares:
/// validates that the manager's decode hook produced a JSON object - exactly
/// what the old `validate_spec` required of the canonical row JSON - without
/// materializing the decoded value.
fn spec_object(ctx: &ResourceContext, op: DriverOp) -> Result<(), DriverFailure> {
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
        return Err(
            DriverFailure::refused(op, FailureKinds::CORE_SPEC_INVALID).with_detail(
                FailureDetail::at("spec/shape").comparison(FailureComparison::new(
                    "spec.shape",
                    "object",
                    shape,
                )),
            ),
        );
    }
    Ok(())
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
//

// ---------------------------------------------------------------------------
// Tests: driver behavior over a scripted manager.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Mutex;

    use crate::context::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext, WatchId,
        WatchRegistration,
    };
    use crate::driver::{
        DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriverFactory,
    };
    use crate::error::{DriverFailure, DriverOp, FailureClass, ResourceError};
    use crate::identity::{
        ResourceKey, ResourceProvenance, ResourceTypeName, StoredDesiredResource,
    };
    use crate::manager::ResourceView;
    use crate::spec_store::EnsureOutcome;

    use serde_json::json;

    use super::{MetadataDriverFactory, metadata_spec_decoder};

    /// The type the scripted driver serves. The driver is type-agnostic, so
    /// any declaration-only type exercises it.
    const TYPE_NAME: &str = "Role";

    fn type_name() -> ResourceTypeName {
        ResourceTypeName::new(TYPE_NAME)
    }

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

        async fn call_order(&self) -> Vec<&'static str> {
            self.calls.lock().await.clone()
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
            Err(ResourceError::ManagerRejected { reason: "unexpected ensure_child".into() })
        }

        async fn get(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Ok(None)
        }

        async fn view(&self, _key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
            Ok(None)
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.calls.lock().await.push("delete");
            self.owned.lock().await.retain(|row| row.key != *key);
            Ok(())
        }

        async fn list_owned(
            &self,
            _owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            self.calls.lock().await.push("list-owned");
            if self.fail_reads.load(Ordering::SeqCst) {
                return Err(ResourceError::ManagerRejected { reason: "scripted read failure".into() });
            }
            Ok(self.owned.lock().await.clone())
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
            key: ResourceKey::new("work", TYPE_NAME, "sample"),
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

    fn context(target: StoredDesiredResource, manager: Arc<ScriptedManager>) -> ResourceContext {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        ResourceContext::new(
            target,
            metadata_spec_decoder(),
            manager,
            Arc::new(NullRequeue),
            effects_tx,
            notify_tx,
        )
    }

    async fn driver(
        spec: serde_json::Value,
        manager: Arc<ScriptedManager>,
    ) -> (ResourceContext, Box<dyn DynResourceDriver>) {
        let ctx = context(target_row(spec), manager);
        let driver = MetadataDriverFactory::new(type_name())
            .create(&ResourceKey::new("work", TYPE_NAME, "sample"))
            .await;
        (ctx, driver)
    }

    /// The stored spec envelope must be the JSON spec object every core row
    /// stores; anything else is the same corrupt row the old `validate_spec`
    /// refused.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn validate_refuses_a_spec_that_is_not_an_object() {
        let manager = ScriptedManager::new();
        let (mut ctx, mut driver) = driver(json!("not-an-object"), Arc::clone(&manager)).await;
        let failure = driver
            .validate(&mut ctx)
            .await
            .expect_err("terminal refusal");
        assert_eq!(failure.kind().code(), "core-spec-invalid");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Validate);
        assert!(
            manager.call_order().await.is_empty(),
            "validate must not touch the manager"
        );
    }

    /// The bootstrap rows store `{}`: the old fence was "the row is not
    /// empty", not "the spec has fields".
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn validate_admits_the_empty_spec_object() {
        let manager = ScriptedManager::new();
        let (mut ctx, mut driver) = driver(json!({}), Arc::clone(&manager)).await;
        driver
            .validate(&mut ctx)
            .await
            .expect("the empty spec object is admitted");
    }

    /// Recovery and deletion realize nothing on a target and read no rows.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn recover_and_delete_converge_without_manager_calls() {
        let manager = ScriptedManager::new();
        let (mut ctx, mut driver) = driver(json!({}), Arc::clone(&manager)).await;
        assert_eq!(
            driver.recover(&mut ctx).await.expect("adopted"),
            RecoveryOutcome::Adopted
        );
        driver.delete(&mut ctx).await.expect("converged");
        assert!(manager.call_order().await.is_empty());
    }

    /// One reconcile pass converges as metadata: no manager read, no effect,
    /// and no status the type would claim.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn reconcile_converges_without_effects() {
        let manager = ScriptedManager::new();
        let (mut ctx, mut driver) =
            driver(json!({ "artifactId": "sample" }), Arc::clone(&manager)).await;
        assert_eq!(
            driver.reconcile(&mut ctx).await.expect("converged"),
            ReconcileOutcome::Satisfied
        );
        assert!(manager.call_order().await.is_empty());
        assert!(ctx.status::<serde_json::Value>().is_none());
    }

    /// The drain is child-first: a live owned child defers the row, and once
    /// the children are gone the drain converges.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn finalize_drains_owned_children_first() {
        let manager = ScriptedManager::with_owned(vec![owned_child("Process")]);
        let (mut ctx, mut driver) = driver(json!({}), Arc::clone(&manager)).await;
        let failure = driver
            .finalize(&mut ctx)
            .await
            .expect_err("a live owned child defers");
        assert_eq!(failure.kind().code(), "children-draining");
        assert_eq!(failure.op(), DriverOp::Delete);
        assert!(
            manager.call_order().await.contains(&"delete"),
            "the child is nudged through its own finalize-before-delete pass"
        );

        manager.owned.lock().await.clear();
        driver
            .finalize(&mut ctx)
            .await
            .expect("no owned child remains");
    }

    /// A manager read the drain cannot answer is retryable, never terminal.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn a_failed_child_read_is_retryable() {
        let manager = ScriptedManager::new();
        manager.fail_reads();
        let (mut ctx, mut driver) = driver(json!({}), Arc::clone(&manager)).await;
        let failure: DriverFailure = driver.finalize(&mut ctx).await.expect_err("read failure");
        assert_eq!(failure.class(), FailureClass::Retryable);
    }
}
