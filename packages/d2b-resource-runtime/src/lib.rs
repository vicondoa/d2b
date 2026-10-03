//! d2b-resource-runtime - the v3 resource execution runtime.
//!
//! Desired specs are durable rows in a minimal SQLite store; each desired
//! resource is driven by exactly one authoritative Ractor actor that realizes
//! effects through a `ResourceDriver` on an explicit Host or Guest target.
//! Runtime status, watches, retries, and queues are in-memory only.

/// An in-process authority publisher, for composition that has no broker.
pub mod test_support;
/// Per-Zone runtime authority for desired specs and resource actors.
pub mod manager;
/// One authoritative actor per desired resource; owns logical live state.
pub mod resource;
/// Resource-type-specific validate/recover/reconcile/delete behavior.
pub mod driver;
/// The shared declaration-only metadata driver: the conversion every
/// declaration-only metadata type's rows converge through.
pub mod metadata;
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
/// Durable desired revisions, the publication outbox, and the recovery table
/// for staged, prepared, and committed-but-unacknowledged transactions (U5,
/// KTD5-KTD6).
pub mod authority_journal;
/// The broker half of the freeze / commit / publish / acknowledge order: the
/// seam the manager drives one durable Zone transaction through.
pub mod authority_publish;
/// Typed relation indexes derived from accepted desired rows (U6; R2-R4).
pub mod relations;
/// Resource identity and ownership-edge types.
pub mod identity;
/// Runtime error taxonomy shared by actors, drivers, and the store.
pub mod error;
/// Runtime revisions (daemon epoch + sequence) for watch cursors.
pub mod revision;

// Public runtime surface (U3): the manager is the per-Zone authority, the
// resource actor is the per-resource authority.
pub use crate::manager::{
    authority_subject_kind, resource_ref, source_controller_kind, AdmissionDecision, AdmissionOp, AllowAll,
    AuthenticatedIdentity, AuthenticatedMutation, ChildrenDiff, DesiredResource,
    ManagerActorEndpoint, MutationAdmission, MutationRequest, MutationSubject, ResourceHandle,
    ResourceSelector, ResourceManager, ResourceManagerArgs, ResourceManagerClient,
    ResourceManagerMsg, ResourceView,
};
pub use crate::resource::{
    DEFAULT_REQUEUE_BACKOFF, ResourceActor, ResourceActorArgs, ResourceMsg, ResourceStatus,
};

// Lookup classification (issue #511): the canonical classified row-read
// result every driver and effect maps onto.
pub use crate::context::{LookupPlane, RowLookup};

// Authority journal (U5, KTD5-KTD6): the durable identity a desired
// authority change carries, and the explicit recovery decision for each
// outstanding publication transaction.
pub use crate::authority_journal::{
    AcceptedCursor, AcceptedPublication, CommitOutcome, CommittedPublication, DesiredMutation,
    DesiredRow, OutboxEntry, ProjectedAudit, ProjectedRow, Projection, PublishedRow, RetiredRow,
    StagedMutation, TransactionRecovery, ZoneRecovery,
};
// Authority publication (KTD6-KTD7): the two broker-side calls a durable Zone
// transaction makes, and the owned facts each one carries.
pub use crate::authority_publish::{
    AcceptedRevision, AuthorityPublisher, FencedTransaction, MutationKind, PublicationCandidate,
    PublicationRefusal, PublicationRows, PublishError, PublishOutcome, ZoneProjection,
    adopt_outstanding, publish, resynchronize,
};
// Relation index (U6, KTD2-KTD4): the six distinct graph relationship classes
// derived from committed desired rows, and the per-type projections that read
// them. Nothing here is separately authored: every edge comes from a row.
pub use crate::relations::{
    AuthorizationRelation, BindingRequestRelations, BindingSlotConflict, ConsumptionRelation,
    DecodedBindingRequest, ImplementationRelation, ObservationRelation,
    OperationImplementationRelations, OwnershipRelation, PlacementRelation, RelationClass,
    RelationEdge, RelationError, RelationExtractors, RelationExtractor, RelationIndex,
    RelationResolver, RelationRow, UnresolvedRelation, BINDING_RESOURCE_TYPES,
};
pub use crate::identity::TransactionId;
pub use crate::schema::AUTHORITY_JOURNAL_USER_VERSION;

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

