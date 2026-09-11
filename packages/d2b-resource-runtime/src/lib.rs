//! d2b-resource-runtime - the v3 resource execution runtime.
//!
//! Desired specs are durable rows in a minimal SQLite store; each desired
//! resource is driven by exactly one authoritative Ractor actor that realizes
//! effects through a `ResourceDriver` on an explicit Host or Guest target.
//! Runtime status, watches, retries, and queues are in-memory only.

/// Per-Zone runtime authority for desired specs and resource actors.
pub mod manager;
/// One authoritative actor per desired resource; owns logical live state.
pub mod resource;
/// Resource-type-specific validate/recover/reconcile/delete behavior.
pub mod driver;
/// The capability surface handed to drivers (ensure/get/delete/watch/...).
pub mod context;
/// Provider registry producing drivers per resource type.
pub mod provider;
/// Explicit Host vs Guest execution-target layer.
pub mod target;
/// Guest-side target actor and target-local runtime plumbing.
pub mod guest_target;
/// In-memory external/internal watch hub over runtime revisions.
pub mod watch;
/// Durable desired-spec store (SQLite, single writer).
pub mod schema;
pub mod spec_store;
/// Resource identity and ownership-edge types.
pub mod identity;
/// Runtime error taxonomy shared by actors, drivers, and the store.
pub mod error;
/// Runtime revisions (daemon epoch + sequence) for watch cursors.
pub mod revision;

// Public runtime surface (U3): the manager is the per-Zone authority, the
// resource actor is the per-resource authority.
pub use crate::manager::{
    AdmissionDecision, AdmissionOp, AllowAll, ChildrenDiff, DesiredResource, ManagerActorEndpoint,
    MutationAdmission, MutationRequest, MutationSubject, ResourceHandle, ResourceSelector,
    ResourceManager, ResourceManagerArgs, ResourceManagerClient, ResourceManagerMsg, ResourceView,
};
pub use crate::resource::{
    DEFAULT_REQUEUE_BACKOFF, ResourceActor, ResourceActorArgs, ResourceMsg, ResourceStatus,
};

// Target layer (U13): the Host/Guest directory, the generation-bound guest
// handle it mints, and the Guest-side target runtime behind the
// target-control port.
pub use crate::guest_target::{
    GuestAdoption, GuestRealizeRequest, GuestTargetControl, GuestTargetError, GuestTargetRuntime,
    SessionBoundGuestTargetControl, TargetResourceInstance, TargetInstanceState,
};
pub use crate::target::{
    GuestAdoptionOutcome, GuestConnectOutcome, GuestDisconnectOutcome, GuestTargetHandle,
    HostTargetHandle, ResolvedTarget, TargetAssignment, TargetAvailability, TargetDirectory,
    TargetError, TargetHandle, TargetKind, TargetObservation, TargetRef,
};

#[cfg(test)]
mod smoke_tests {
    /// Scaffold smoke test: the crate compiles and its module tree resolves.
    #[test]
    fn modules_resolve() {
        assert_eq!(crate::manager::MODULE_NAME, "manager");
        assert_eq!(crate::resource::MODULE_NAME, "resource");
        assert_eq!(crate::driver::MODULE_NAME, "driver");
        assert_eq!(crate::context::MODULE_NAME, "context");
        assert_eq!(crate::provider::MODULE_NAME, "provider");
        assert_eq!(crate::target::MODULE_NAME, "target");
        assert_eq!(crate::guest_target::MODULE_NAME, "guest_target");
        assert_eq!(crate::watch::MODULE_NAME, "watch");
        assert_eq!(crate::spec_store::MODULE_NAME, "spec_store");
        assert_eq!(crate::identity::MODULE_NAME, "identity");
        assert_eq!(crate::error::MODULE_NAME, "error");
        assert_eq!(crate::revision::MODULE_NAME, "revision");
    }
}
