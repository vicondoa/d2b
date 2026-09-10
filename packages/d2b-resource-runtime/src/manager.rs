//! Per-Zone ResourceManager: the single runtime authority for desired
//! resources (U3; spec sections 4, 6, 7, 16, 17, 20, 21, 33).
//!
//! The manager owns durable spec persistence (the only writer to the spec
//! store, R2), resource identity, actor lifecycle (one authoritative actor
//! per resource, R1), provider lookup, ownership edges, internal-watch
//! routing, the runtime resource index, and external watches. It never
//! executes provider reconciliation logic.
//!
//! ## The durability boundary (F1, AE1, R7)
//!
//! Every `Ensure` runs: admission -> persist (commit-before-return) ->
//! spawn-or-update the actor -> reply. Identical specs return the current
//! actor handle; changed specs persist a new generation and only then send
//! `SpecChanged`; absent resources persist before their actor is spawned. A
//! spawn that fails after the commit (e.g. no provider factory for the type)
//! leaves the row durable; a later Ensure or manager restart recovers it.
//!
//! ## Admission hook (KTD2 review finding)
//!
//! The redb `SealedMutation` admission-seal contract was cut consciously:
//! admission happens at the manager boundary, before any persistence. The
//! constructor takes a [`MutationAdmission`] invoked on every `Apply` /
//! `Ensure` / `Remove` entry point with the caller's subject context and the
//! desired mutation. U8/U10 wire the real subjects (API caller identity,
//! bundle identity, owning-resource identity); tests default to
//! [`AllowAll`].
//!
//! ## Supervision (R17, spec sections 16, 33)
//!
//! Resource actors are spawned linked; a crash is respawened with the same
//! durable row (the actor's `Start` sequence recovers/adopts from the
//! target). The manager keeps the ephemeral dependency graph (who watches
//! whom) and notifies dependents of a death via `DependencyChanged`, so
//! they reconcile and re-register watches.
//!
//! ## Runtime revisions (R23, F4)
//!
//! The [`crate::watch::WatchHub`] owns the epoch + sequence; the manager
//! publishes every desired and runtime change into it. Status transitions
//! never touch disk (AE6, R11): they only update the in-memory view and
//! publish.

pub const MODULE_NAME: &str = "manager";

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use ractor::{Actor, ActorCell, ActorProcessingErr, ActorRef};
use tokio::sync::oneshot;
use crate::context::{
    ChildEnsure, ManagerEndpoint, SpecDecoder, WatchId,
    WatchRegistration as InternalWatchRegistration,
};
use crate::spec_store::EnsureOutcome;
use crate::error::ResourceError;
use crate::identity::{ResourceKey, ResourceProvenance, ResourceTypeName, StoredDesiredResource};
use crate::provider::ProviderDirectory;
use crate::resource::{ResourceActor, ResourceActorArgs, ResourceMsg, ResourceStatus};
use crate::revision::RuntimeRevision;
use crate::spec_store::{SpecStore, SpecStoreError};
use crate::watch::{
    ChangeKind, ChangeNotice, ChangeSource, WatchHub, WatchRegistration as ExternalWatchRegistration,
    WatchSelector as ExternalWatchSelector,
};

// ---------------------------------------------------------------------------
// Admission hook: the manager boundary replaces the cut redb seal (KTD2)
// ---------------------------------------------------------------------------

/// Who is asking for a durable mutation (U8/U10 wire the real subjects: API
/// caller identity, bundle identity, owning-resource identity).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationSubject {
    /// Human/credential-readable principal: the API caller subject, `nix`
    /// for bundle materialization, or the owning resource's display key.
    pub principal: String,
    /// Which surface the mutation came from (persisted provenance).
    pub origin: ResourceProvenance,
}

/// Which mutation admission is asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionOp {
    Ensure,
    Remove,
}

/// The mutation admission evaluates, before any persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationRequest {
    pub key: ResourceKey,
    pub op: AdmissionOp,
    pub spec: Vec<u8>,
    pub metadata: Vec<u8>,
}

/// Admission verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionDecision {
    Allow,
    Deny(String),
}

/// The manager-boundary admission hook (U3): invoked on EVERY `Apply` /
/// `Ensure` / `Remove` entry point before persisting. U8/U10 wire the real
/// admission (API caller subject / bundle identity / owning-resource
/// identity); tests default to [`AllowAll`].
pub trait MutationAdmission: Send + Sync + 'static {
    fn admit(&self, subject: &MutationSubject, request: &MutationRequest) -> AdmissionDecision;
}

/// Default admission: allow everything (tests, and callers that authorize
/// upstream).
pub struct AllowAll;

impl MutationAdmission for AllowAll {
    fn admit(&self, _subject: &MutationSubject, _request: &MutationRequest) -> AdmissionDecision {
        AdmissionDecision::Allow
    }
}

// ---------------------------------------------------------------------------
// Public value types
// ---------------------------------------------------------------------------

/// A desired resource arriving from Nix, the API, or a parent actor (R2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredResource {
    /// Durable identity `(zone, type, name)`. Owned children derive their
    /// zone from the owner.
    pub key: ResourceKey,
    /// Opaque encoded spec envelope (decoded by the wired per-type hook).
    pub spec: Vec<u8>,
    /// Opaque encoded metadata envelope (finalizers, annotations, ...).
    pub metadata: Vec<u8>,
    /// Where this desired resource came from (persisted provenance).
    pub provenance: ResourceProvenance,
}

/// Handle to a resource's authoritative actor, returned by `Ensure` (R7).
#[derive(Debug, Clone)]
pub struct ResourceHandle {
    pub key: ResourceKey,
    /// Stable 16-byte identity; survives generation changes.
    pub uid: [u8; 16],
    pub generation: u64,
    /// The one authoritative actor for this resource (R1).
    pub actor: ActorRef<ResourceMsg>,
}

/// In-memory runtime view of one resource: durable envelope minus status
/// plus the actor's in-memory status (R11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceView {
    pub key: ResourceKey,
    pub uid: [u8; 16],
    pub generation: u64,
    pub deleting: bool,
    pub provenance: ResourceProvenance,
    pub spec: Vec<u8>,
    pub metadata: Vec<u8>,
    pub status: Option<ResourceStatus>,
}

/// Filter for [`ResourceManagerMsg::List`]. Absent fields are wildcards.
#[derive(Debug, Clone, Default)]
pub struct ResourceSelector {
    pub zone: Option<String>,
    pub type_name: Option<String>,
    pub owner: Option<ResourceKey>,
}

/// Outcome of declarative owned-child reconciliation (R9, spec section 21).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChildrenDiff {
    pub created: Vec<ResourceKey>,
    pub updated: Vec<ResourceKey>,
    pub retained: Vec<ResourceKey>,
    pub obsolete: Vec<ResourceKey>,
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// The manager's message protocol (spec section 4). Every durable mutation
/// is request/reply; replies fire only after the commit (F1, AE1).
#[derive(Debug)]
pub enum ResourceManagerMsg {
    /// Top-level desired-resource apply (Nix materialization, A1).
    Apply {
        subject: MutationSubject,
        desired: DesiredResource,
        reply: oneshot::Sender<Result<ResourceHandle, ResourceError>>,
    },
    /// Idempotent ensure (R7, AE1). `owner` marks an actor-created child
    /// (R8): the child persists ownership and derives its zone from it.
    Ensure {
        subject: MutationSubject,
        owner: Option<ResourceKey>,
        desired: DesiredResource,
        reply: oneshot::Sender<Result<ResourceHandle, ResourceError>>,
    },
    /// Durable deletion (R10, F3): mark deleting (commit), cascade to owned
    /// children, then run driver cleanup through the actor.
    Remove {
        subject: MutationSubject,
        key: ResourceKey,
        reply: oneshot::Sender<Result<(), ResourceError>>,
    },
    /// Current in-memory runtime view (spec section 4 `Get`).
    Get {
        key: ResourceKey,
        reply: oneshot::Sender<Result<Option<ResourceView>, ResourceError>>,
    },
    /// Runtime index listing (spec section 4 `List`).
    List {
        selector: ResourceSelector,
        reply: oneshot::Sender<Result<Vec<ResourceView>, ResourceError>>,
    },
    /// External API watch (R23): served from the in-memory hub; replay plus
    /// live delivery is gap-free within the daemon epoch.
    Watch {
        selector: ExternalWatchSelector,
        after: Option<RuntimeRevision>,
        reply: oneshot::Sender<Result<ExternalWatchRegistration, ResourceError>>,
    },
    /// A resource actor transitioned its in-memory status (R11): update the
    /// runtime view and publish to the hub; zero persistent writes (AE6).
    RuntimeChanged {
        key: ResourceKey,
        status: ResourceStatus,
    },
    /// Bookkeeping from spawn flows (spec section 4).
    ActorStarted {
        key: ResourceKey,
        actor: ActorCell,
    },
    /// Bookkeeping: the actor stopped. The row may still be durable (crash)
    /// or the deletion completed; supervision decides respawn vs cleanup.
    ActorStopped { key: ResourceKey },
    /// Direct dependency-edge registration (spec section 16): the manager
    /// holds the ephemeral graph so actor deaths notify dependents.
    DependencyAdded {
        dependent: ResourceKey,
        dependency: ResourceKey,
    },
    DependencyRemoved {
        dependent: ResourceKey,
        dependency: ResourceKey,
    },

    // ---- Driver-context routing surface (R2; context.rs ManagerCall) ----
    /// A parent actor ensures an owned child through its context; the reply
    /// fires only after the child's row committed (F1, AE1).
    ChildEnsure {
        parent: ResourceKey,
        child: ChildEnsure,
        reply: oneshot::Sender<Result<EnsureOutcome, ResourceError>>,
    },
    /// Stored-row lookup for drivers (`ResourceContext::get`).
    GetRow {
        key: ResourceKey,
        reply: oneshot::Sender<Result<Option<StoredDesiredResource>, ResourceError>>,
    },
    /// Rows owned by a resource uid (`ResourceContext::children`).
    ListOwned {
        owner_uid: [u8; 16],
        reply: oneshot::Sender<Result<Vec<StoredDesiredResource>, ResourceError>>,
    },
    /// Internal watch registration routed to the target actor (R12, spec
    /// section 15); the manager records the dependency edge (spec 16).
    RegisterWatch {
        registration: InternalWatchRegistration,
        subscriber: ResourceKey,
        reply: oneshot::Sender<Result<WatchId, ResourceError>>,
    },
    CancelWatch {
        watch: WatchId,
        reply: oneshot::Sender<Result<(), ResourceError>>,
    },
    /// Declarative owned-child reconciliation (R9, spec section 21): diff
    /// currently owned children against desired children.
    ReconcileChildren {
        owner: ResourceKey,
        desired: Vec<ChildEnsure>,
        reply: oneshot::Sender<Result<ChildrenDiff, ResourceError>>,
    },
    /// The actor finished driver cleanup; the manager removes the spec row
    /// and the actor stops (F3).
    DeletionComplete { key: ResourceKey },
}

/// The manager's durable + runtime state (spec section 4).
pub struct ResourceManagerState {
    zone: String,
    store: Arc<SpecStore>,
    providers: Arc<ProviderDirectory>,
    hub: Arc<WatchHub>,
    admission: Arc<dyn MutationAdmission>,
    decoders: HashMap<ResourceTypeName, Arc<dyn SpecDecoder>>,
    default_decoder: Arc<dyn SpecDecoder>,
    backoff: Duration,
    my_cell: ActorCell,

    /// Runtime resource index (spec section 4).
    rows: HashMap<ResourceKey, StoredDesiredResource>,
    statuses: HashMap<ResourceKey, ResourceStatus>,
    actors: HashMap<ResourceKey, ActorRef<ResourceMsg>>,
    actors_by_id: HashMap<ractor::ActorId, ResourceKey>,
    by_uid: HashMap<[u8; 16], ResourceKey>,
    by_owner: HashMap<ResourceKey, std::collections::HashSet<ResourceKey>>,
    by_type: HashMap<ResourceTypeName, std::collections::HashSet<ResourceKey>>,

    /// Ephemeral dependency graph (spec section 16): who watches whom. Held
    /// across actor restarts so dependents can be notified and resubscribe.
    dependents: HashMap<ResourceKey, std::collections::HashSet<ResourceKey>>,
    /// Live internal-watch registrations routed to target actors.
    watch_registry: HashMap<WatchId, WatchEntry>,
    next_watch_id: u64,

    /// The hub owns the authoritative epoch + sequence; this mirrors the
    /// epoch for quick reference (single source of truth is the hub).
    revision_epoch: u64,
}

struct WatchEntry {
    subscriber: ResourceKey,
    target: ResourceKey,
}

/// Deterministic stable uid for a key (R8): owned-child graphs and adoption
/// identities reconstruct identically after restart.
pub(crate) fn deterministic_uid(key: &ResourceKey) -> [u8; 16] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"d2b-resource-uid/v1\x00");
    hasher.update(key.zone.as_bytes());
    hasher.update([0u8]);
    hasher.update(key.type_name.as_bytes());
    hasher.update([0u8]);
    hasher.update(key.name.as_bytes());
    let digest = hasher.finalize();
    let mut uid = [0u8; 16];
    uid.copy_from_slice(&digest[..16]);
    uid
}

impl ResourceManagerState {
    fn view(&self, key: &ResourceKey) -> Option<ResourceView> {
        let row = self.rows.get(key)?;
        Some(ResourceView {
            key: row.key.clone(),
            uid: row.uid,
            generation: row.generation,
            deleting: row.deleting,
            provenance: row.provenance,
            spec: row.spec.clone(),
            metadata: row.metadata.clone(),
            status: self.statuses.get(key).copied(),
        })
    }

    /// Insert/refresh the index entries for a row.
    fn index_row(&mut self, row: StoredDesiredResource) {
        let key = row.key.clone();
        self.by_uid.insert(row.uid, key.clone());
        self.by_type
            .entry(ResourceTypeName::new(key.type_name.clone()))
            .or_default()
            .insert(key.clone());
        if let Some(owner_uid) = row.owner_uid {
            if let Some(owner_key) = self.by_uid.get(&owner_uid).cloned() {
                self.by_owner.entry(owner_key).or_default().insert(key.clone());
            }
        }
        self.rows.insert(key, row);
    }

    /// Drop all index entries for a key (deletion completed).
    fn unindex_row(&mut self, key: &ResourceKey) {
        self.statuses.remove(key);
        if let Some(row) = self.rows.remove(key) {
            self.by_uid.remove(&row.uid);
            self.by_type
                .entry(ResourceTypeName::new(key.type_name.clone()))
                .or_default()
                .remove(key);
            if let Some(owner_uid) = row.owner_uid {
                if let Some(owner_key) = self.by_uid.get(&owner_uid).cloned() {
                    if let Some(children) = self.by_owner.get_mut(&owner_key) {
                        children.remove(key);
                    }
                }
            }
        }
        // Watch edges owned by the deleted resource die with it.
        let stale: Vec<WatchId> = self
            .watch_registry
            .iter()
            .filter(|(_, entry)| entry.subscriber == *key)
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            if let Some(entry) = self.watch_registry.remove(&id) {
                if let Some(dependents) = self.dependents.get_mut(&entry.target) {
                    dependents.remove(&entry.subscriber);
                }
                if let Some(target) = self.actors.get(&entry.target) {
                    let _ = target.send_message(ResourceMsg::Unwatch { id });
                }
            }
        }
        self.dependents.remove(key);
    }

    /// Resolves the per-type decoder (falling back to the configured
    /// default) and spawns the resource actor linked to this manager (R17).
    /// The spawn happens only after the caller committed the row (F1).
    async fn spawn_resource_actor(
        &mut self,
        manager: ActorRef<ResourceManagerMsg>,
        row: StoredDesiredResource,
    ) -> Result<ActorRef<ResourceMsg>, ResourceError> {
        let type_name = ResourceTypeName::new(row.key.type_name.clone());
        let decoder =
            self.decoders.get(&type_name).cloned().unwrap_or_else(|| self.default_decoder.clone());
        let args = ResourceActorArgs {
            row: row.clone(),
            target: crate::target::TargetHandle::Host,
            providers: self.providers.clone(),
            manager,
            decoder,
            backoff: self.backoff,
        };
        match ResourceActor::spawn_linked(None, ResourceActor::new(), args, self.my_cell.clone())
            .await
        {
            Ok((actor, _join)) => {
                self.actors.insert(row.key.clone(), actor.clone());
                self.actors_by_id.insert(actor.get_id(), row.key);
                Ok(actor)
            }
            Err(error) => Err(ResourceError::Provider {
                type_name: type_name.to_string(),
                message: error.to_string(),
            }),
        }
    }


    async fn notify_dependents(&mut self, key: &ResourceKey) {
        let Some(dependents) = self.dependents.get(key).cloned() else {
            return;
        };
        for dependent in dependents {
            if let Some(actor) = self.actors.get(&dependent) {
                let _ = actor.send_message(ResourceMsg::DependencyChanged { key: key.clone() });
            }
        }
    }

    /// Admission check shared by all mutating entry points.
    fn admit(
        &self,
        subject: &MutationSubject,
        request: &MutationRequest,
    ) -> Result<(), ResourceError> {
        match self.admission.admit(subject, request) {
            AdmissionDecision::Allow => Ok(()),
            AdmissionDecision::Deny(reason) => Err(ResourceError::AdmissionDenied {
                principal: subject.principal.clone(),
                zone: request.key.zone.clone(),
                type_name: request.key.type_name.clone(),
                name: request.key.name.clone(),
                reason,
            }),
        }
    }

    /// The durability boundary (F1, AE1, R7): admission -> commit ->
    /// spawn-or-update. Returns the committed outcome and the (existing or
    /// freshly spawned) actor. A spawn failure leaves the row durable.
    async fn ensure_internal(
        &mut self,
        manager: ActorRef<ResourceManagerMsg>,
        subject: &MutationSubject,
        row: StoredDesiredResource,
    ) -> Result<(EnsureOutcome, ActorRef<ResourceMsg>), ResourceError> {
        self.admit(
            subject,
            &MutationRequest {
                key: row.key.clone(),
                op: AdmissionOp::Ensure,
                spec: row.spec.clone(),
                metadata: row.metadata.clone(),
            },
        )?;
        let outcome = self.store.ensure(row).await.map_err(ResourceError::from)?;
        let committed = outcome.row().clone();
        let changed = matches!(outcome, EnsureOutcome::Created(_) | EnsureOutcome::Updated(_));
        self.index_row(committed.clone());
        if changed {
            // Desired-state change publishes into the hub (F4); status stays
            // in memory (AE6).
            self.hub.publish(ChangeNotice {
                key: committed.key.clone(),
                kind: ChangeKind::Upsert,
                source: ChangeSource::Desired,
            });
        }
        let key = committed.key.clone();
        let actor = match self.actors.get(&key).cloned() {
            Some(actor) => {
                if let EnsureOutcome::Updated(_) = outcome {
                    // AE1: the actor receives the change only after the new
                    // generation committed.
                    let _ = actor.send_message(ResourceMsg::SpecChanged {
                        generation: committed.generation,
                        spec: committed.spec.clone(),
                        metadata: committed.metadata.clone(),
                    });
                }
                actor
            }
            None => self.spawn_resource_actor(manager, committed).await?,
        };
        Ok((outcome, actor))
    }

    /// Durable deletion (R10, F3): mark deleting (commit), cascade to owned
    /// children, then run cleanup through the actor. Absent resources are a
    /// no-op; resources that never realized effects are cleaned up inline.
    async fn remove_internal(
        &mut self,
        subject: &MutationSubject,
        key: &ResourceKey,
    ) -> Result<(), ResourceError> {
        let (spec, metadata) = match self.rows.get(key) {
            Some(row) => (row.spec.clone(), row.metadata.clone()),
            None => (Vec::new(), Vec::new()),
        };
        self.admit(
            subject,
            &MutationRequest { key: key.clone(), op: AdmissionOp::Remove, spec, metadata },
        )?;
        // Owned children are removed with the parent unless the type
        // supports orphaning (F3; orphaning support lands with type
        // contracts, not in the manager).
        let children: Vec<ResourceKey> =
            self.by_owner.get(key).map(|set| set.iter().cloned().collect()).unwrap_or_default();
        let child_subject = MutationSubject {
            principal: key.to_string(),
            origin: ResourceProvenance::Resource,
        };
        for child in children {
            let _ = Box::pin(self.remove_internal(&child_subject, &child)).await;
        }
        match self.store.mark_deleting(key.clone()).await {
            Ok(row) => {
                self.rows.insert(key.clone(), row);
                self.statuses.insert(key.clone(), ResourceStatus::Deleting);
                self.hub.publish(ChangeNotice {
                    key: key.clone(),
                    kind: ChangeKind::Upsert,
                    source: ChangeSource::Desired,
                });
            }
            Err(SpecStoreError::NotFound { .. }) => return Ok(()),
            Err(error) => return Err(error.into()),
        }
        match self.actors.get(key).cloned() {
            Some(actor) => {
                let _ = actor.send_message(ResourceMsg::Delete);
            }
            None => {
                // Nothing was ever realized on a target: cleanup is
                // trivially complete; retire the row now.
                let _ = self.store.remove_after_cleanup(key.clone()).await;
                self.unindex_row(key);
                self.hub.publish(ChangeNotice {
                    key: key.clone(),
                    kind: ChangeKind::Delete,
                    source: ChangeSource::Desired,
                });
            }
        }
        Ok(())
    }
}

/// Construction arguments for the per-Zone manager (KTD5).
pub struct ResourceManagerArgs {
    /// The Zone this manager is the runtime authority for (KTD5).
    pub zone: String,
    pub store: Arc<SpecStore>,
    pub providers: ProviderDirectory,
    pub hub: Arc<WatchHub>,
    /// Manager-boundary admission hook (KTD2 execution decision).
    pub admission: Arc<dyn MutationAdmission>,
    /// Per-type spec decode hooks; unmatched types use the default.
    pub decoders: HashMap<ResourceTypeName, Arc<dyn SpecDecoder>>,
    pub default_decoder: Arc<dyn SpecDecoder>,
    /// Fixed reconcile backoff for retryable driver failures (R13).
    pub backoff: Duration,
}

/// The per-Zone manager actor (spec section 4).
pub struct ResourceManager;

impl ResourceManager {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for ResourceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Actor for ResourceManager {
    type Msg = ResourceManagerMsg;
    type State = ResourceManagerState;
    type Arguments = ResourceManagerArgs;

    async fn pre_start(
        &self,
        myself: ActorRef<ResourceManagerMsg>,
        args: ResourceManagerArgs,
    ) -> Result<ResourceManagerState, ActorProcessingErr> {
        let hub = args.hub.clone();
        let mut state = ResourceManagerState {
            zone: args.zone,
            store: args.store,
            providers: Arc::new(args.providers),
            hub,
            admission: args.admission,
            decoders: args.decoders,
            default_decoder: args.default_decoder,
            backoff: args.backoff,
            my_cell: myself.get_cell(),
            rows: HashMap::new(),
            statuses: HashMap::new(),
            actors: HashMap::new(),
            actors_by_id: HashMap::new(),
            by_uid: HashMap::new(),
            by_owner: HashMap::new(),
            by_type: HashMap::new(),
            dependents: HashMap::new(),
            watch_registry: HashMap::new(),
            next_watch_id: 1,
            revision_epoch: 0,
        };
        state.revision_epoch = state.hub.snapshot_revision().epoch;
        // Restart recovery (F2, R15): load durable specs and spawn one actor
        // per row; each actor reconstructs observed state by discovery and
        // adoption on its target. Rows without a registered provider factory
        // (e.g. a spawn that failed after commit) stay durable and are
        // picked up by the next Ensure or restart.
        let rows = state
            .store
            .list(crate::spec_store::SpecSelector::default())
            .await
            .map_err(|error| ActorProcessingErr::from(error.to_string()))?;
        for row in rows {
            state.index_row(row.clone());
            let _ = state.spawn_resource_actor(myself.clone(), row).await;
        }
        Ok(state)
    }

    async fn handle(
        &self,
        myself: ActorRef<ResourceManagerMsg>,
        message: ResourceManagerMsg,
        state: &mut ResourceManagerState,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            ResourceManagerMsg::Apply { subject, desired, reply } => {
                let row = top_level_row(&desired);
                let result = state
                    .ensure_internal(myself, &subject, row)
                    .await
                    .map(|(outcome, actor)| handle_from_outcome(&outcome, actor));
                reply.send(result).ok();
            }
            ResourceManagerMsg::Ensure { subject, owner, desired, reply } => {
                let result = if desired.key.zone != state.zone {
                    Err(ResourceError::ManagerRpc(format!(
                        "resource zone {} does not belong to manager zone {}",
                        desired.key.zone, state.zone
                    )))
                } else {
                    match resolve_owner(state, owner.as_ref()) {
                        Ok(owner_uid) => {
                            let row = owned_row(&desired, owner.as_ref(), owner_uid);
                            state
                                .ensure_internal(myself, &subject, row)
                                .await
                                .map(|(outcome, actor)| handle_from_outcome(&outcome, actor))
                        }
                        Err(error) => Err(error),
                    }
                };
                reply.send(result).ok();
            }
            ResourceManagerMsg::Remove { subject, key, reply } => {
                let result = state.remove_internal(&subject, &key).await;
                reply.send(result).ok();
            }
            ResourceManagerMsg::Get { key, reply } => {
                reply.send(Ok(state.view(&key))).ok();
            }
            ResourceManagerMsg::List { selector, reply } => {
                let views = state
                    .rows
                    .keys()
                    .filter(|key| selector_matches(state, &selector, key))
                    .filter_map(|key| state.view(key))
                    .collect();
                reply.send(Ok(views)).ok();
            }
            ResourceManagerMsg::Watch { selector, after, reply } => {
                // One manager serializes list snapshots, revisions, and watch
                // registration, so the list/watch handoff stays gap-free
                // within the epoch (F4, R23).
                reply.send(Ok(state.hub.register(selector, after))).ok();
            }
            ResourceManagerMsg::RuntimeChanged { key, status } => {
                // Status is in-memory only (R11): update the view model and
                // publish; no store write (AE6).
                if state.rows.contains_key(&key) {
                    state.statuses.insert(key.clone(), status);
                    state.hub.publish(ChangeNotice {
                        key,
                        kind: ChangeKind::Upsert,
                        source: ChangeSource::RuntimeStatus,
                    });
                }
            }
            ResourceManagerMsg::ActorStarted { key, actor } => {
                // Bookkeeping for asynchronously-started actors; the
                // Ensure-path spawns already inserted their handles.
                if state.rows.contains_key(&key) {
                    let typed = ActorRef::<ResourceMsg>::from(actor.clone());
                    state.actors.insert(key.clone(), typed);
                    state.actors_by_id.insert(actor.get_id(), key);
                }
            }
            ResourceManagerMsg::ActorStopped { key } => {
                state.actors.remove(&key);
            }
            ResourceManagerMsg::DependencyAdded { dependent, dependency } => {
                state.dependents.entry(dependency).or_default().insert(dependent);
            }
            ResourceManagerMsg::DependencyRemoved { dependent, dependency } => {
                if let Some(dependents) = state.dependents.get_mut(&dependency) {
                    dependents.remove(&dependent);
                }
            }
            ResourceManagerMsg::ChildEnsure { parent, child, reply } => {
                // Child identity derives deterministically from the child
                // key (R8); ownership carries the parent's stable uid so
                // the owned-child graph reconstructs after restart.
                let result = match state.rows.get(&parent).cloned() {
                    Some(parent_row) => {
                        let subject = MutationSubject {
                            principal: parent.to_string(),
                            origin: ResourceProvenance::Resource,
                        };
                        let key = ResourceKey::new(
                            parent.zone.clone(),
                            child.type_name.as_str().to_owned(),
                            child.name.clone(),
                        );
                        let row = StoredDesiredResource {
                            key,
                            uid: deterministic_uid(&ResourceKey::new(
                                parent.zone.clone(),
                                child.type_name.as_str().to_owned(),
                                child.name.clone(),
                            )),
                            generation: 1,
                            owner_uid: Some(parent_row.uid),
                            provenance: ResourceProvenance::Resource,
                            deleting: false,
                            spec: child.spec.clone(),
                            metadata: child.metadata.clone(),
                            created_at: 0,
                        };
                        state
                            .ensure_internal(myself, &subject, row)
                            .await
                            .map(|(outcome, _actor)| outcome)
                    }
                    None => Err(ResourceError::ManagerRpc(format!(
                        "parent {parent} is not known to the manager"
                    ))),
                };
                reply.send(result).ok();
            }
            ResourceManagerMsg::GetRow { key, reply } => {
                reply.send(Ok(state.rows.get(&key).cloned())).ok();
            }
            ResourceManagerMsg::ListOwned { owner_uid, reply } => {
                let rows = state
                    .rows
                    .values()
                    .filter(|row| row.owner_uid == Some(owner_uid))
                    .cloned()
                    .collect();
                reply.send(Ok(rows)).ok();
            }
            ResourceManagerMsg::RegisterWatch { registration, subscriber, reply } => {
                let result = register_watch(state, registration, subscriber);
                reply.send(result).ok();
            }
            ResourceManagerMsg::CancelWatch { watch, reply } => {
                if let Some(entry) = state.watch_registry.remove(&watch) {
                    if let Some(dependents) = state.dependents.get_mut(&entry.target) {
                        dependents.remove(&entry.subscriber);
                    }
                    if let Some(target) = state.actors.get(&entry.target) {
                        let _ = target.send_message(ResourceMsg::Unwatch { id: watch });
                    }
                }
                reply.send(Ok(())).ok();
            }
            ResourceManagerMsg::ReconcileChildren { owner, desired, reply } => {
                let result = reconcile_children(state, myself, &owner, desired).await;
                reply.send(result).ok();
            }
            ResourceManagerMsg::DeletionComplete { key } => {
                // Cleanup finished (F3): remove the spec row, retire the
                // index entries, publish the deletion. The actor stops
                // itself; supervision sees no entry and does not respawn.
                let _ = state.store.remove_after_cleanup(key.clone()).await;
                state.unindex_row(&key);
                state.actors.remove(&key);
                // Drop the id mapping too: supervision events are delivered
                // ahead of regular mailbox traffic, so an ActorTerminated
                // racing ahead of this message must not respawn the
                // finished resource.
                let dead: Vec<ractor::ActorId> = state
                    .actors_by_id
                    .iter()
                    .filter(|(_, mapped)| **mapped == key)
                    .map(|(id, _)| *id)
                    .collect();
                for id in dead {
                    state.actors_by_id.remove(&id);
                }
                state.hub.publish(ChangeNotice {
                    key,
                    kind: ChangeKind::Delete,
                    source: ChangeSource::Desired,
                });
            }
        }
        Ok(())
    }

    async fn handle_supervisor_evt(
        &self,
        myself: ActorRef<ResourceManagerMsg>,
        message: ractor::SupervisionEvent,
        state: &mut ResourceManagerState,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            ractor::SupervisionEvent::ActorFailed(who, _panic) => {
                supervise_exit(state, who, myself).await
            }
            ractor::SupervisionEvent::ActorTerminated(who, _last_state, _reason) => {
                supervise_exit(state, who, myself).await
            }
            _ => {}
        }
        Ok(())
    }
}

/// Supervision (R17): a crashed (or unexpectedly stopped) resource actor is
/// respawned from its durable row with recover/adopt; if the row is gone
/// (deletion completed), just clean up.
async fn supervise_exit(
    state: &mut ResourceManagerState,
    who: ActorCell,
    manager: ActorRef<ResourceManagerMsg>,
) {
    let Some(key) = state.actors_by_id.remove(&who.get_id()) else {
        return;
    };
    state.actors.remove(&key);
    if let Some(row) = state.rows.get(&key).cloned() {
        let _ = state.spawn_resource_actor(manager, row).await;
    }
    state.notify_dependents(&key).await;
}

/// Build the durable row for a top-level ensure (R7). The uid derives
/// deterministically from the stable key.
fn top_level_row(desired: &DesiredResource) -> StoredDesiredResource {
    StoredDesiredResource {
        key: desired.key.clone(),
        uid: deterministic_uid(&desired.key),
        generation: 1,
        owner_uid: None,
        provenance: desired.provenance,
        deleting: false,
        spec: desired.spec.clone(),
        metadata: desired.metadata.clone(),
        created_at: 0,
    }
}

/// Resolve the owner's stable uid for an owned-child ensure (R8).
fn resolve_owner(
    state: &ResourceManagerState,
    owner: Option<&ResourceKey>,
) -> Result<Option<[u8; 16]>, ResourceError> {
    match owner {
        None => Ok(None),
        Some(owner_key) => match state.rows.get(owner_key) {
            Some(row) => Ok(Some(row.uid)),
            None => Err(ResourceError::ManagerRpc(format!(
                "owner {owner_key} is not known to the manager"
            ))),
        },
    }
}

fn owned_row(
    desired: &DesiredResource,
    owner: Option<&ResourceKey>,
    owner_uid: Option<[u8; 16]>,
) -> StoredDesiredResource {
    StoredDesiredResource {
        key: desired.key.clone(),
        uid: deterministic_uid(&desired.key),
        generation: 1,
        owner_uid,
        // Owned children always carry Resource provenance (R8).
        provenance: if owner.is_some() { ResourceProvenance::Resource } else { desired.provenance },
        deleting: false,
        spec: desired.spec.clone(),
        metadata: desired.metadata.clone(),
        created_at: 0,
    }
}

fn handle_from_outcome(outcome: &EnsureOutcome, actor: ActorRef<ResourceMsg>) -> ResourceHandle {
    let committed = outcome.row();
    ResourceHandle {
        key: committed.key.clone(),
        uid: committed.uid,
        generation: committed.generation,
        actor,
    }
}

/// Internal watch registration routed to the target actor (R12, spec 15).
/// The manager records the dependency edge so a target death notifies the
/// subscriber (spec 16). The WatchId is manager-allocated.
fn register_watch(
    state: &mut ResourceManagerState,
    registration: InternalWatchRegistration,
    subscriber: ResourceKey,
) -> Result<WatchId, ResourceError> {
    let target = registration.target.clone();
    let Some(target_actor) = state.actors.get(&target).cloned() else {
        return Err(ResourceError::ManagerRpc(format!(
            "watch target {target} has no running actor"
        )));
    };
    let id = WatchId(state.next_watch_id);
    state.next_watch_id += 1;
    state.watch_registry.insert(
        id,
        WatchEntry { subscriber: subscriber.clone(), target: target.clone() },
    );
    state.dependents.entry(target).or_default().insert(subscriber);
    let _ = target_actor.send_message(ResourceMsg::Watch {
        id,
        condition: registration.condition,
        subscriber: registration.notify,
    });
    Ok(id)
}

/// Declarative owned-child reconciliation (R9, spec section 21): missing
/// children are created, matching children retained/updated, obsolete
/// children marked deleting. Every mutation routes through `Ensure` /
/// `Remove` with the owner as the admission subject.
async fn reconcile_children(
    state: &mut ResourceManagerState,
    manager: ActorRef<ResourceManagerMsg>,
    owner: &ResourceKey,
    desired: Vec<ChildEnsure>,
) -> Result<ChildrenDiff, ResourceError> {
    let Some(parent_row) = state.rows.get(owner).cloned() else {
        return Err(ResourceError::ManagerRpc(format!("owner {owner} is not known to the manager")));
    };
    let subject =
        MutationSubject { principal: owner.to_string(), origin: ResourceProvenance::Resource };
    let mut diff = ChildrenDiff::default();
    for child in &desired {
        let key = ResourceKey::new(owner.zone.clone(), child.type_name.as_str().to_owned(), child.name.clone());
        let row = StoredDesiredResource {
            key: key.clone(),
            uid: deterministic_uid(&key),
            generation: 1,
            owner_uid: Some(parent_row.uid),
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: child.spec.clone(),
            metadata: child.metadata.clone(),
            created_at: 0,
        };
        match state.ensure_internal(manager.clone(), &subject, row).await {
            Ok((outcome, _actor)) => match outcome {
                EnsureOutcome::Created(_) => diff.created.push(key),
                EnsureOutcome::Updated(_) => diff.updated.push(key),
                EnsureOutcome::Unchanged(_) => diff.retained.push(key),
            },
            // A child already deleting is obsolete by definition.
            Err(ResourceError::DeletingConflict { .. }) => diff.obsolete.push(key),
            Err(error) => return Err(error),
        }
    }
    // Obsolete: owned, not deleting, not desired anymore.
    let desired_names: std::collections::HashSet<String> =
        desired.iter().map(|child| child.name.clone()).collect();
    let owned: Vec<StoredDesiredResource> = state
        .rows
        .values()
        .filter(|row| row.owner_uid == Some(parent_row.uid) && !row.deleting)
        .cloned()
        .collect();
    for child in owned {
        if !desired_names.contains(&child.key.name) {
            state.remove_internal(&subject, &child.key).await?;
            diff.obsolete.push(child.key.clone());
        }
    }
    Ok(diff)
}


/// The manager endpoint injected into every driver context (R2): all
/// durable child mutations and internal-watch registrations ride the manager
/// mailbox; replies fire only after the commit (F1, AE1).
#[derive(Debug, Clone)]
pub struct ManagerActorEndpoint {
    manager: ActorRef<ResourceManagerMsg>,
}

impl ManagerActorEndpoint {
    pub fn new(manager: ActorRef<ResourceManagerMsg>) -> Self {
        Self { manager }
    }

    async fn rpc<T>(
        &self,
        build: impl FnOnce(oneshot::Sender<Result<T, ResourceError>>) -> ResourceManagerMsg,
    ) -> Result<T, ResourceError> {
        let (reply, rx) = oneshot::channel();
        self.manager
            .send_message(build(reply))
            .map_err(|_| ResourceError::ManagerRpc("manager channel closed".into()))?;
        rx.await.map_err(|_| ResourceError::ManagerRpc("manager dropped the request".into()))?
    }
}

#[async_trait::async_trait]
impl ManagerEndpoint for ManagerActorEndpoint {
    async fn ensure_child(
        &self,
        parent: &ResourceKey,
        child: ChildEnsure,
    ) -> Result<EnsureOutcome, ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::ChildEnsure {
            parent: parent.clone(),
            child,
            reply,
        })
        .await
    }

    async fn get(&self, key: &ResourceKey) -> Result<Option<StoredDesiredResource>, ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::GetRow { key: key.clone(), reply }).await
    }

    async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
        let subject = MutationSubject {
            principal: key.to_string(),
            origin: ResourceProvenance::Resource,
        };
        self.rpc(|reply| ResourceManagerMsg::Remove {
            subject,
            key: key.clone(),
            reply,
        })
        .await
    }

    async fn list_owned(
        &self,
        owner_uid: [u8; 16],
    ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::ListOwned { owner_uid, reply }).await
    }

    async fn register_watch(
        &self,
        subscriber: &ResourceKey,
        registration: InternalWatchRegistration,
    ) -> Result<WatchId, ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::RegisterWatch {
            registration,
            subscriber: subscriber.clone(),
            reply,
        })
        .await
    }

    async fn cancel_watch(&self, watch: WatchId) -> Result<(), ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::CancelWatch { watch, reply }).await
    }
}

/// Caller-side facade over the manager mailbox (U8 wires the Resource API
/// onto this surface; U9 composes the manager).
#[derive(Debug, Clone)]
pub struct ResourceManagerClient {
    actor: ActorRef<ResourceManagerMsg>,
}

impl ResourceManagerClient {
    pub fn new(actor: ActorRef<ResourceManagerMsg>) -> Self {
        Self { actor }
    }

    pub fn actor(&self) -> &ActorRef<ResourceManagerMsg> {
        &self.actor
    }

    async fn rpc<T>(
        &self,
        build: impl FnOnce(oneshot::Sender<Result<T, ResourceError>>) -> ResourceManagerMsg,
    ) -> Result<T, ResourceError> {
        let (reply, rx) = oneshot::channel();
        self.actor
            .send_message(build(reply))
            .map_err(|_| ResourceError::ManagerRpc("manager channel closed".into()))?;
        rx.await.map_err(|_| ResourceError::ManagerRpc("manager dropped the request".into()))?
    }

    pub async fn apply(
        &self,
        subject: MutationSubject,
        desired: DesiredResource,
    ) -> Result<ResourceHandle, ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::Apply { subject, desired, reply }).await
    }

    pub async fn ensure(
        &self,
        subject: MutationSubject,
        owner: Option<ResourceKey>,
        desired: DesiredResource,
    ) -> Result<ResourceHandle, ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::Ensure { subject, owner, desired, reply }).await
    }

    pub async fn remove(
        &self,
        subject: MutationSubject,
        key: ResourceKey,
    ) -> Result<(), ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::Remove { subject, key, reply }).await
    }

    pub async fn get(&self, key: ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::Get { key, reply }).await
    }

    pub async fn list(
        &self,
        selector: ResourceSelector,
    ) -> Result<Vec<ResourceView>, ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::List { selector, reply }).await
    }

    pub async fn watch(
        &self,
        selector: ExternalWatchSelector,
        after: Option<RuntimeRevision>,
    ) -> Result<ExternalWatchRegistration, ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::Watch { selector, after, reply }).await
    }

    pub async fn get_row(
        &self,
        key: ResourceKey,
    ) -> Result<Option<StoredDesiredResource>, ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::GetRow { key, reply }).await
    }

    pub async fn list_owned(
        &self,
        owner_uid: [u8; 16],
    ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::ListOwned { owner_uid, reply }).await
    }

    pub async fn ensure_child(
        &self,
        parent: ResourceKey,
        child: ChildEnsure,
    ) -> Result<EnsureOutcome, ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::ChildEnsure { parent, child, reply }).await
    }

    pub async fn register_watch(
        &self,
        subscriber: ResourceKey,
        registration: InternalWatchRegistration,
    ) -> Result<WatchId, ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::RegisterWatch { registration, subscriber, reply }).await
    }

    pub async fn cancel_watch(&self, watch: WatchId) -> Result<(), ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::CancelWatch { watch, reply }).await
    }

    pub async fn reconcile_children(
        &self,
        owner: ResourceKey,
        desired: Vec<ChildEnsure>,
    ) -> Result<ChildrenDiff, ResourceError> {
        self.rpc(|reply| ResourceManagerMsg::ReconcileChildren { owner, desired, reply }).await
    }

}

/// Owner filtering resolves the owner key to its stable uid against the
/// in-memory row index.
fn selector_matches(
    state: &ResourceManagerState,
    selector: &ResourceSelector,
    key: &ResourceKey,
) -> bool {
    if let Some(zone) = &selector.zone {
        if *zone != key.zone {
            return false;
        }
    }
    if let Some(type_name) = &selector.type_name {
        if *type_name != key.type_name {
            return false;
        }
    }
    if let Some(owner) = &selector.owner {
        let Some(owner_row) = state.rows.get(owner) else {
            return false;
        };
        let Some(row) = state.rows.get(key) else {
            return false;
        };
        if row.owner_uid != Some(owner_row.uid) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering as AtomicOrdering;
    use std::time::Duration;

    use tokio::sync::mpsc;

    use crate::context::{ChildEnsure, WatchCondition, WatchSatisfied};
    use crate::error::{DriverFailure, DriverOp};
    use crate::error::ResourceError;
    use crate::identity::ResourceTypeName;
    use crate::resource::test_support::{
        desired, harness, harness_over, harness_with, key, subject, until, wait_row_gone,
        wait_status,
    };
    use crate::resource::test_support::ReconcileMode;
    use crate::resource::{ResourceMsg, ResourceStatus};
    use crate::watch::ChangeSource;

    /// R7/AE1: identical ensure returns the same handle and spawns exactly
    /// one actor; a changed spec advances the durable generation exactly
    /// once and the actor observes the new generation only after the commit.
    #[tokio::test]
    async fn duplicate_ensure_same_handle_and_one_actor() {
        let h = harness(&["Test"]).await;
        let k = key("test", "Test", "data");
        let first = h
            .client
            .ensure(subject(), None, desired("Test", "data", b"one"))
            .await
            .expect("first ensure");
        let second = h
            .client
            .ensure(subject(), None, desired("Test", "data", b"one"))
            .await
            .expect("second ensure");
        assert_eq!(first.actor, second.actor, "duplicate ensure returns the same handle");
        assert_eq!(first.generation, 1);
        assert_eq!(second.generation, 1);
        let shared = h.factory.shared(&k);
        until(|| shared.recover_calls.load(AtomicOrdering::SeqCst) == 1).await;

        // Changed spec: generation N+1 persists before the actor receives
        // the change (AE1); the driver observes the new generation.
        let updated = h
            .client
            .ensure(subject(), None, desired("Test", "data", b"two"))
            .await
            .expect("changed ensure");
        assert_eq!(updated.generation, 2);
        assert_eq!(updated.actor, first.actor);
        until(|| shared.generations_seen.lock().contains(&2)).await;
        let row = h.client.get_row(k.clone()).await.expect("get_row").expect("row");
        assert_eq!(row.generation, 2);
        assert_eq!(row.spec, b"two");
    }

    /// F1/AE1: ensure commits before spawn - a poisoned spawn still leaves
    /// the spec row present, and a restart (fresh manager over the same
    /// durable store) recovers the resource.
    #[tokio::test]
    async fn ensure_commits_before_spawn_and_restart_recovers() {
        let h = harness(&["Other"]).await; // factory does NOT cover "Ghost"
        let k = key("test", "Ghost", "g");
        let error = h
            .client
            .ensure(subject(), None, desired("Ghost", "g", b"spec"))
            .await
            .expect_err("spawn fails after commit");
        assert!(matches!(error, ResourceError::Provider { .. }), "got {error}");
        let row = h.client.get_row(k.clone()).await.expect("get_row");
        assert!(row.is_some(), "spec row committed before the poisoned spawn");
        let row = row.unwrap();
        assert_eq!(row.generation, 1);
        assert_eq!(row.spec, b"spec");

        // Restart: a fresh manager over the same store spawns the actor for
        // the orphaned row, which recovers and reconciles.
        let restarted =
            harness_over(h.store.clone(), "test", &["Ghost"], Duration::from_millis(200)).await;
        let shared = restarted.factory.shared(&k);
        until(|| shared.recover_calls.load(AtomicOrdering::SeqCst) == 1).await;
        wait_status(&restarted.client, &k, ResourceStatus::Ready).await;
    }

    /// R17/spec section 16: an actor crash triggers a supervised respawn
    /// with recover/adopt, and dependents receive `DependencyChanged`, so
    /// they reconcile and re-register their watches.
    #[tokio::test]
    async fn actor_crash_respawns_and_notifies_dependents() {
        let h = harness(&["Target", "Dep"]).await;
        let tkey = key("test", "Target", "t");
        let dkey = key("test", "Dep", "d");
        let tshare = h.factory.shared(&tkey);
        let dshare = h.factory.shared(&dkey);
        // The dependent registers a watch on the target in every reconcile.
        *dshare.watch_target.lock() = Some(tkey.clone());

        let target =
            h.client.ensure(subject(), None, desired("Target", "t", b"t")).await.expect("target");
        let _dependent =
            h.client.ensure(subject(), None, desired("Dep", "d", b"d")).await.expect("dependent");
        wait_status(&h.client, &tkey, ResourceStatus::Ready).await;
        until(|| dshare.watch_calls.load(AtomicOrdering::SeqCst) >= 1).await;
        let watch_calls_before = dshare.watch_calls.load(AtomicOrdering::SeqCst);

        // Crash the target through its driver (panic in reconcile).
        *tshare.reconcile_mode.lock() = ReconcileMode::PanicOnce;
        target.actor.send_message(ResourceMsg::Reconcile).expect("cast reconcile");
        until(|| tshare.recover_calls.load(AtomicOrdering::SeqCst) == 2).await;

        // The dependent was notified: it reconciled again and re-registered
        // its watch on the respawned target.
        until(|| dshare.watch_calls.load(AtomicOrdering::SeqCst) > watch_calls_before).await;
        wait_status(&h.client, &tkey, ResourceStatus::Ready).await;
    }

    /// R9/spec section 21: declarative owned-child diff creates missing
    /// children, retains matching ones, and marks obsolete children
    /// deleting (their rows retire after cleanup).
    #[tokio::test]
    async fn owned_children_diff_creates_retains_marks_obsolete() {
        let h = harness(&["Volume", "Worker"]).await;
        let parent = key("test", "Volume", "data");
        h.client.ensure(subject(), None, desired("Volume", "data", b"vol")).await.expect("parent");
        let child = |name: &str, spec: &[u8]| ChildEnsure {
            type_name: ResourceTypeName::new("Worker"),
            name: name.to_string(),
            spec: spec.to_vec(),
            metadata: Vec::new(),
        };
        let a = child("helper", b"a");
        let b = child("extra", b"b");

        let first =
            h.client.reconcile_children(parent.clone(), vec![a.clone(), b.clone()]).await.expect("diff");
        assert_eq!(first.created.len(), 2);

        let second =
            h.client.reconcile_children(parent.clone(), vec![a.clone(), b.clone()]).await.expect("diff");
        assert_eq!(second.retained.len(), 2, "matching children are retained, not re-created");
        assert!(second.created.is_empty());

        // Parent config drops `b`: marked deleting, row retired after cleanup.
        let akey = key("test", "Worker", "helper");
        let bkey = key("test", "Worker", "extra");
        let bshare = h.factory.shared(&bkey);
        let third = h.client.reconcile_children(parent.clone(), vec![a]).await.expect("diff");
        assert_eq!(third.obsolete, vec![bkey.clone()]);
        wait_row_gone(&h.client, &bkey).await;
        assert!(bshare.delete_calls.load(AtomicOrdering::SeqCst) >= 1);
        assert!(h.client.get_row(akey.clone()).await.expect("get_row").is_some());
        assert!(h.client.get_row(parent.clone()).await.expect("get_row").is_some());
    }

    /// R10 + the Ensure-during-deleting rejection: a resource with the
    /// durable deleting mark committed rejects a late Ensure with the typed
    /// deleting conflict, surfaced through the manager.
    #[tokio::test]
    async fn ensure_against_deleting_is_rejected() {
        let h = harness(&["Test"]).await;
        let k = key("test", "Test", "data");
        let shared = h.factory.shared(&k);
        shared.delete_blocked.store(true, AtomicOrdering::SeqCst);
        h.client.ensure(subject(), None, desired("Test", "data", b"one")).await.expect("ensure");
        wait_status(&h.client, &k, ResourceStatus::Ready).await;

        h.client.remove(subject(), k.clone()).await.expect("remove");
        for _ in 0..500 {
            if let Ok(Some(row)) = h.client.get_row(k.clone()).await {
                if row.deleting {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let error = h
            .client
            .ensure(subject(), None, desired("Test", "data", b"one"))
            .await
            .expect_err("ensure against deleting");
        assert!(matches!(error, ResourceError::DeletingConflict { .. }), "got {error}");

        // Cleanup resumes once the blocked driver delete is released.
        shared.open_gate();
        wait_row_gone(&h.client, &k).await;
    }

    /// R14: two `Reconcile` messages on one resource while an effect is in
    /// flight never overlap - the guard coalesces them, and the driver is
    /// entered exactly once more after the effect completes.
    #[tokio::test]
    async fn same_resource_reconcile_never_overlaps() {
        let h = harness(&["Test"]).await;
        let k = key("test", "Test", "data");
        let shared = h.factory.shared(&k);
        *shared.reconcile_mode.lock() = ReconcileMode::GatedEffectOnce;
        let handle =
            h.client.ensure(subject(), None, desired("Test", "data", b"one")).await.expect("ensure");
        eprintln!("DBG after ensure: reconcile_calls={}", shared.reconcile_calls.load(AtomicOrdering::SeqCst));
        until(|| {
            eprintln!("DBG poll reconcile_calls={}", shared.reconcile_calls.load(AtomicOrdering::SeqCst));
            shared.reconcile_calls.load(AtomicOrdering::SeqCst) == 1
        })
        .await;

        handle.actor.send_message(ResourceMsg::Reconcile).expect("cast 1");
        handle.actor.send_message(ResourceMsg::Reconcile).expect("cast 2");
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            shared.reconcile_calls.load(AtomicOrdering::SeqCst),
            1,
            "no reconcile may enter the driver while the effect is in flight"
        );

        shared.open_gate();
        until(|| shared.reconcile_calls.load(AtomicOrdering::SeqCst) == 2).await;
        assert_eq!(shared.max_concurrent_reconcile.load(AtomicOrdering::SeqCst), 1);
        wait_status(&h.client, &k, ResourceStatus::Ready).await;
    }

    /// R14: distinct resources are independent actors; their reconciles run
    /// concurrently.
    #[tokio::test]
    async fn distinct_resources_reconcile_concurrently() {
        let h = harness(&["Test"]).await;
        let ak = key("test", "Test", "a");
        let bk = key("test", "Test", "b");
        let ashared = h.factory.shared(&ak);
        let bshared = h.factory.shared(&bk);
        *ashared.reconcile_mode.lock() = ReconcileMode::BlockedInline;
        *bshared.reconcile_mode.lock() = ReconcileMode::BlockedInline;
        h.client.ensure(subject(), None, desired("Test", "a", b"a")).await.expect("ensure a");
        h.client.ensure(subject(), None, desired("Test", "b", b"b")).await.expect("ensure b");
        until(|| {
            ashared.active_reconcile.load(AtomicOrdering::SeqCst) == 1
                && bshared.active_reconcile.load(AtomicOrdering::SeqCst) == 1
        })
        .await;
        assert_eq!(ashared.max_concurrent_reconcile.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(bshared.max_concurrent_reconcile.load(AtomicOrdering::SeqCst), 1);
        ashared.open_gate();
        bshared.open_gate();
        wait_status(&h.client, &ak, ResourceStatus::Ready).await;
        wait_status(&h.client, &bk, ResourceStatus::Ready).await;
    }

    /// AE2/R12: a watch registered against a condition that is already true
    /// notifies immediately, from the same mailbox handler.
    #[tokio::test]
    async fn watch_when_already_true_notifies_immediately() {
        let h = harness(&["Test"]).await;
        let tkey = key("test", "Test", "data");
        h.client.ensure(subject(), None, desired("Test", "data", b"one")).await.expect("ensure");
        wait_status(&h.client, &tkey, ResourceStatus::Ready).await;

        let (notify, mut rx) = mpsc::unbounded_channel::<WatchSatisfied>();
        let watcher = key("test", "Test", "watcher");
        let id = h
            .client
            .register_watch(
                watcher,
                crate::context::WatchRegistration {
                    target: tkey.clone(),
                    condition: WatchCondition::Ready,
                    notify,
                },
            )
            .await
            .expect("register watch");
        let satisfied = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("immediate notification")
            .expect("satisfied");
        assert_eq!(satisfied.watch, id);
        assert_eq!(satisfied.target, tkey);
    }

    /// AE2/R12: a condition that flips while the watch registration message
    /// is still queued still notifies exactly once, through the real actor
    /// mailbox.
    #[tokio::test]
    async fn condition_flip_while_watch_queued_notifies_exactly_once() {
        let h = harness(&["Test"]).await;
        let tkey = key("test", "Test", "data");
        let shared = h.factory.shared(&tkey);
        *shared.reconcile_mode.lock() = ReconcileMode::GatedEffectOnce;
        h.client.ensure(subject(), None, desired("Test", "data", b"one")).await.expect("ensure");
        until(|| shared.reconcile_calls.load(AtomicOrdering::SeqCst) == 1).await;

        let (notify, mut rx) = mpsc::unbounded_channel::<WatchSatisfied>();
        let watcher = key("test", "Test", "watcher");
        let id = h
            .client
            .register_watch(
                watcher,
                crate::context::WatchRegistration {
                    target: tkey.clone(),
                    condition: WatchCondition::Ready,
                    notify,
                },
            )
            .await
            .expect("register watch");
        // The condition is false while the registration is processed.
        assert!(rx.try_recv().is_err(), "no notification while not ready");

        // The condition flips: the completed effect continues reconcile,
        // which satisfies the desired state and notifies the queued watcher
        // exactly once in the transition handler.
        shared.open_gate();
        let satisfied = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("notification after flip")
            .expect("satisfied");
        assert_eq!(satisfied.watch, id);
        wait_status(&h.client, &tkey, ResourceStatus::Ready).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(rx.try_recv().is_err(), "exactly one notification per registration");
    }

    /// Spec section 32: the delete path cancels the pending requeue timer -
    /// after cleanup, no further reconcile is ever delivered.
    #[tokio::test]
    async fn delete_cancels_pending_requeue_timer() {
        let h = harness_with(&["Test"], Duration::from_millis(300)).await;
        let k = key("test", "Test", "data");
        let shared = h.factory.shared(&k);
        *shared.reconcile_mode.lock() = ReconcileMode::FailRetryable;
        h.client.ensure(subject(), None, desired("Test", "data", b"one")).await.expect("ensure");
        wait_status(
            &h.client,
            &k,
            ResourceStatus::Failed(DriverFailure::retryable(DriverOp::Reconcile)),
        )
        .await;
        assert_eq!(shared.reconcile_calls.load(AtomicOrdering::SeqCst), 1);

        // Delete well before the 300ms requeue timer would fire.
        h.client.remove(subject(), k.clone()).await.expect("remove");
        wait_row_gone(&h.client, &k).await;
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(
            shared.reconcile_calls.load(AtomicOrdering::SeqCst),
            1,
            "the cancelled requeue timer must never deliver a reconcile"
        );
        // The termination event can race ahead of DeletionComplete in the
        // manager mailbox; the durable deleting row then makes the manager
        // respawn once more, and the idempotent driver delete runs again.
        // Both passes are cleanup; the invariant under test is that the
        // cancelled requeue never delivers a reconcile.
        assert!(
            shared.delete_calls.load(AtomicOrdering::SeqCst) >= 1,
            "driver delete ran to completion"
        );
    }

    /// R11/AE6: status transitions publish `RuntimeChanged` to the manager
    /// (which feeds the watch hub) and perform zero persistent writes.
    #[tokio::test]
    async fn status_transitions_publish_and_write_zero_store_rows() {
        let h = harness(&["Test"]).await;
        let k = key("test", "Test", "data");
        let shared = h.factory.shared(&k);
        let handle =
            h.client.ensure(subject(), None, desired("Test", "data", b"one")).await.expect("ensure");
        wait_status(&h.client, &k, ResourceStatus::Ready).await;
        let snapshot = h.hub.snapshot_revision();

        // Status churn: repeated reconcile cycles through the in-memory
        // transitions only; no spec change is involved.
        for _ in 0..3 {
            handle.actor.send_message(ResourceMsg::Reconcile).expect("cast reconcile");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;

        let history = h.store.history(100).await.expect("history");
        assert_eq!(
            history.len(),
            1,
            "status churn must not produce persistent writes (AE6): only the ensure.create audit row exists"
        );
        let view = h.client.get(k.clone()).await.expect("get").expect("view");
        assert_eq!(view.status, Some(ResourceStatus::Ready));
        let events = h.hub.events_after(snapshot).expect("events after snapshot");
        assert!(
            events.iter().any(|event| event.source == ChangeSource::RuntimeStatus),
            "status transitions publish runtime events to the hub"
        );
        assert!(
            events.iter().all(|event| event.source == ChangeSource::RuntimeStatus),
            "nothing after the snapshot is a desired write: status is memory-only"
        );
        assert_eq!(shared.reconcile_calls.load(AtomicOrdering::SeqCst), 4, "three churn passes ran");
    }
}

