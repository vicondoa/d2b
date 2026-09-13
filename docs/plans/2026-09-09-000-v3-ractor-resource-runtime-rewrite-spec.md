# d2b v3 Resource Runtime Rewrite
## Replace redb + store-driven controllers with Ractor resource actors

This document is intended to be handed directly to an implementation agent working in the `vicondoa/d2b` repository on the `v3` branch.

The goal is a one-shot architectural rewrite. It is acceptable for the branch to be broken during the work. Do not build compatibility shims, dual-write paths, or temporary adapters unless they are strictly necessary to finish the rewrite.

---

## 1. Objective

Replace the current resource-plane architecture:

```text
Nix / Resource API
        |
        v
Resource Store
        |
        +-- redb persistence
        +-- revisions
        +-- watches
        +-- replay
        +-- status persistence
        |
        v
Controller Runner
        |
        +-- PendingQueue
        +-- fresh reads
        +-- retries
        +-- conflict handling
        +-- checkpoints
        +-- scheduled requeues
        |
        v
ResourceReconciler
        |
        v
Provider-specific effect logic
```

with:

```text
                         Nix
                          |
                          | desired resources
                          v
                  +------------------+
                  | ResourceManager  |<--------- Resource API
                  |      Actor       |
                  +--------+---------+
                           |
             +-------------+-------------+
             |                           |
             | durable specs             | logical runtime ownership
             v                           v
       +------------+           +-------------------+
       | Spec Store |           | Resource Actors   |
       |  SQLite    |           |                   |
       |            |           | Volume/foo        |
       | spec       |           | Process/bar       |
       | owner      |           | Network/work      |
       | target     |           | Guest/dev         |
       | generation |           +---------+---------+
       | deleting   |                     |
       +------------+                     | realize on exact target
                                          v
                                  +----------------+
                                  | TargetDirectory|
                                  +-------+--------+
                                          |
                         +----------------+----------------+
                         |                                 |
                         v                                 v
                    HostTarget                       GuestTarget/A
                         |                                 |
                  Provider/Driver                    ComponentSession
                         |                                 |
                         v                                 v
                       Linux                      Guest TargetRuntime
                                                           |
                                                    target-local effects
                                                           |
                                                           v
                                                        Linux
```

The new architecture has these properties:

1. Desired resource specs are durable.
2. Resource status and observed state are in memory only.
3. Every desired resource has one authoritative actor in the Zone that owns the resource's logical live state.
4. A resource's Zone and its execution target are independent. A resource stored in a Host Zone may be realized on the Host or inside a Guest without moving into the Guest's namespace.
5. Resource actors reconcile themselves and use a target abstraction for physical realization.
6. Child resource creation goes through `ResourceManager`.
7. Internal watches are actor subscriptions, not store watches.
8. External API watches are served from an in-memory watch hub.
9. On restart, actors are reconstructed from specs and recover observed state by discovery/adoption from the target on which they are realized.
10. Existing provider-specific effect code should be preserved wherever possible.
11. `ZoneLink` remains the resource that links Zone namespaces. It is not the generic mechanism for running a Host-owned resource inside a Guest.
12. The existing redb resource database, controller runner, store-driven watch system, and revision-conflict machinery should be removed.

---

## 2. Architectural principle

The system has three sources of truth, each with a distinct responsibility:

```text
Desired truth:   Spec Store / Nix
Observed truth:  the actual target machine
Runtime model:   Ractor actors
```

The resource plane also has three orthogonal identity/placement concepts. Do not collapse them:

```text
Zone       = where the resource exists, is named, authorized, and persisted
Target     = where the resource is physically realized
ZoneLink   = a resource representing an edge between Zone namespaces
```

Examples:

```text
Zone: /
Resource: Process/foo
Target: Guest/work

/Process/foo remains a resource in Zone /.
It does not become /work/Process/foo.
It does not require ZoneLink to mean "run this in Guest/work".
```

A `ZoneLink` may itself target a Guest because its implementation needs to run there, but that is placement of the ZoneLink resource. Its semantic purpose remains linking Zones.

Do not persist runtime status merely so it can be restored.

On restart:

```text
load desired specs
      |
      v
spawn resource actors
      |
      v
discover actual resources
      |
      +-- exact match -> adopt
      +-- missing     -> create
      +-- unexpected  -> cleanup/quarantine according to resource policy
      |
      v
reconstruct runtime status
      |
      v
reconcile desired vs observed
```

Actors should be intentionally disposable.

---

## 3. New crate and core types

Create a new crate:

```text
packages/d2b-resource-runtime
```

Recommended initial layout:

```text
packages/d2b-resource-runtime/
  src/
    lib.rs
    manager.rs
    resource.rs
    driver.rs
    context.rs
    provider.rs
    target.rs
    guest_target.rs
    watch.rs
    spec_store.rs
    identity.rs
    error.rs
```

Add `ractor` and `rusqlite` to the workspace dependencies.

The major runtime types should be:

```text
ResourceManager
ResourceManagerMsg
ResourceActor
ResourceMsg
ResourceDriver
ResourceDriverFactory
ResourceContext
ResourceHandle
ResourceAddress
ResourceTarget
TargetDirectory
TargetHandle
GuestTargetActor
WatchCondition
WatchId
WatchRegistration
SpecStore
StoredDesiredResource
```

---

## 4. ResourceManager

`ResourceManager` is the single runtime authority for desired resources.

It owns:

- durable spec persistence
- logical resource identity
- actor creation
- actor lookup
- provider/driver lookup
- execution-target lookup
- ownership edges
- external API watches
- runtime resource index
- runtime revision sequence

It does **not** execute provider reconciliation logic.

Suggested state:

```rust
struct ResourceManagerState {
    specs: SpecStore,

    resources: HashMap<ResourceKey, ResourceHandle>,
    by_ref: HashMap<ResourceRef, ResourceKey>,
    by_owner: HashMap<ResourceKey, HashSet<ResourceKey>>,
    by_type: HashMap<ResourceTypeName, HashSet<ResourceKey>>,

    providers: ProviderDirectory,
    targets: TargetDirectory,

    watches: WatchHub,

    revision_epoch: u64,
    revision_seq: u64,
}
```

Suggested protocol:

```rust
enum ResourceManagerMsg {
    Apply {
        desired: DesiredResource,
        reply: RpcReplyPort<Result<ResourceHandle, ResourceError>>,
    },

    Ensure {
        owner: Option<ResourceKey>,
        desired: DesiredResource,
        reply: RpcReplyPort<Result<ResourceHandle, ResourceError>>,
    },

    Remove {
        key: ResourceKey,
        reply: RpcReplyPort<Result<(), ResourceError>>,
    },

    Get {
        reference: ResourceRef,
        reply: RpcReplyPort<Result<Option<ResourceView>, ResourceError>>,
    },

    List {
        selector: ResourceSelector,
        reply: RpcReplyPort<Result<Vec<ResourceView>, ResourceError>>,
    },

    Watch {
        selector: ResourceSelector,
        after: Option<RuntimeRevision>,
        subscriber: ExternalWatchSubscriber,
        reply: RpcReplyPort<Result<WatchRegistration, ResourceError>>,
    },

    RuntimeChanged {
        key: ResourceKey,
        status: ResourceStatus,
    },

    ActorStarted {
        key: ResourceKey,
        actor: ResourceAddress,
    },

    ActorStopped {
        key: ResourceKey,
    },

    DependencyAdded {
        dependent: ResourceKey,
        dependency: ResourceKey,
    },

    DependencyRemoved {
        dependent: ResourceKey,
        dependency: ResourceKey,
    },
}
```

---

## 5. Desired resource persistence

Replace the current redb resource database with a much smaller SQLite desired-state store.

The store persists only information required to reconstruct desired resources.

Suggested schema:

```sql
CREATE TABLE resources (
    zone        TEXT NOT NULL,
    type        TEXT NOT NULL,
    name        TEXT NOT NULL,

    uid         BLOB NOT NULL,
    generation  INTEGER NOT NULL,

    owner_uid   BLOB,
    spec        BLOB NOT NULL,

    deleting    INTEGER NOT NULL DEFAULT 0,

    PRIMARY KEY(zone, type, name)
);
```

Optional indexes:

```sql
CREATE INDEX resources_owner_uid
    ON resources(owner_uid);

CREATE INDEX resources_type
    ON resources(zone, type);
```

Do not persist:

```text
runtime status
observed status
PID
pidfd state
actor references
watch registrations
controller checkpoints
retry attempts
reconcile queue state
dependency readiness
health transitions
temporary handles
current sockets/fds
```

`deleting` is durable desired state and should remain persisted until cleanup finishes.

---

## 6. Resource creation invariant

All resource creation, regardless of origin, must pass through `ResourceManager`.

The paths are:

```text
Nix
 |
 | Apply(spec)
 v
ResourceManager
 |
 +--> SpecStore
 +--> ResourceActor

Resource API
 |
 | Apply(spec)
 v
ResourceManager
 |
 +--> SpecStore
 +--> ResourceActor

VolumeActor
 |
 | Ensure(ProcessSpec, owner=Volume/foo)
 v
ResourceManager
 |
 +--> SpecStore
 +--> ProcessActor
```

The critical invariant is:

```text
                 DURABILITY BOUNDARY

Ensure()
   |
   v
persist desired spec
   |
 COMMIT
   |
   v
spawn/update ResourceActor
   |
   v
return ResourceHandle
```

Never spawn or realize a child resource before its desired spec has committed.

---

## 7. Ensure semantics

`Ensure` replaces controller-emitted create/update mutations.

It must be idempotent.

Behavior:

```text
same resource + same spec
    -> return current ResourceHandle

same resource + different spec
    -> persist new generation
    -> send SpecChanged to existing actor

resource absent
    -> persist new desired spec
    -> create actor
    -> return ResourceHandle
```

For actor-created children, identity should be deterministic whenever possible.

Example:

```text
owner: Volume/data
child logical name: mount-helper

=> Process/data.mount-helper
```

The stored child row should retain ownership:

```text
Process/data.mount-helper
owner = Volume/data
spec = ...
```

This lets ownership reconstruct automatically after restart.

---

## 8. ResourceActor

Each desired resource is represented by one Ractor actor.

That actor is the exclusive owner of the live resource state.

Suggested state:

```rust
struct ResourceState {
    key: ResourceKey,

    // desired
    generation: ResourceGeneration,
    spec: ResourceSpec,
    target: ResourceTarget,
    deleting: bool,

    // observed/runtime only
    status: ResourceStatus,

    // actor relationships
    dependencies: HashMap<ResourceKey, DependencyRegistration>,
    watchers: HashMap<WatchId, Watcher>,

    // scheduling
    reconcile_pending: bool,
    effect_running: bool,
}
```

Suggested message protocol:

```rust
enum ResourceMsg {
    Start,

    SpecChanged {
        generation: ResourceGeneration,
        spec: ResourceSpec,
    },

    Reconcile,

    DependencyChanged {
        key: ResourceKey,
    },

    DependencySatisfied {
        key: ResourceKey,
        condition: WatchCondition,
    },

    Watch {
        id: WatchId,
        condition: WatchCondition,
        subscriber: ResourceAddress,
    },

    Unwatch {
        id: WatchId,
    },

    Delete,

    EffectCompleted {
        operation: OperationId,
        result: EffectResult,
    },
}
```

---

## 9. ResourceActor startup

On actor start:

```text
ResourceActor::Start
       |
       v
driver.recover(ctx.target())
       |
       | discover real resources on the exact execution target
       | adopt exact matches
       | reconstruct observed status
       v
driver.reconcile()
```

The actor should not trust persisted runtime state because there is none.

Recovery must derive observed state from reality.

---

## 10. Resource identity and adoption

Every realized external resource should carry enough D2B identity to be discovered after restart.

Prefer stable identifiers such as:

```text
systemd unit name / properties
cgroup path or metadata
process environment marker
VM UUID
Cloud Hypervisor API socket path
tap alias
network namespace name
mount marker
filesystem xattr / marker file
device binding metadata
vsock identity
```

A generic adoption identity should include enough information to distinguish resource incarnation:

```rust
struct AdoptionIdentity {
    zone: ZoneId,
    resource_type: ResourceTypeName,
    resource_name: ResourceName,
    resource_uid: ResourceUid,
    desired_generation: ResourceGeneration,
}
```

Every `ResourceDriver` should be able to support the equivalent of:

```text
discover
adopt
create
reconcile
delete
```

The existing process subsystem already contains useful probe/adoption/quarantine behavior. Preserve and reuse it rather than replacing it with persisted process status.

---

## 11. Replace ResourceReconciler with ResourceDriver

Delete the current `ResourceReconciler` interface.

Replace it with a smaller runtime-oriented contract.

Suggested shape:

```rust
trait ResourceDriver: Send + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn validate(
        &self,
        ctx: &ResourceContext,
    ) -> Result<(), Self::Error>;

    async fn recover(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<(), Self::Error>;

    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<(), Self::Error>;

    async fn delete(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<(), Self::Error>;
}
```

The actor owns scheduling, retry behavior, status publication, dependencies, and lifecycle.

The driver owns resource-specific behavior.

---

## 12. ResourceContext

`ResourceContext` should expose the primitive operations provider code actually needs.

Suggested surface:

```rust
impl ResourceContext {
    fn key(&self) -> &ResourceKey;
    fn spec<T>(&self) -> Result<&T, ResourceError>;
    fn status<T>(&self) -> Result<&T, ResourceError>;

    fn set_status<T>(&mut self, status: T);

    fn target(&self) -> &TargetHandle;

    async fn get<T>(
        &self,
        reference: &ResourceRef,
    ) -> Result<Option<ResourceHandle>, ResourceError>;

    async fn ensure<T>(
        &self,
        name: impl Into<ResourceName>,
        spec: T,
    ) -> Result<ResourceHandle, ResourceError>;

    async fn delete(
        &self,
        resource: &ResourceHandle,
    ) -> Result<(), ResourceError>;

    async fn watch(
        &mut self,
        resource: &ResourceHandle,
        condition: WatchCondition,
    ) -> Result<WatchRegistration, ResourceError>;

    fn requeue_after(&mut self, delay: Duration);

    fn owner(&self) -> Option<&ResourceKey>;
    async fn children(&self) -> Result<Vec<ResourceHandle>, ResourceError>;
}
```

Provider code should not write the SpecStore directly.

All child resource mutations go through `ResourceManager`.

---

## 13. Mechanical controller conversion

Do not redesign every provider independently.

Use the same conversion pattern for every current `impl ResourceReconciler`.

```text
Current                           New
----------------------------------------------------------------
describe()                        DriverFactory registration
validate_spec()                   validate()
plan()                            fold into reconcile()
reconcile()                       reconcile()
execute_effect()                  direct driver/effect-port call
observe()                         recover() + reconcile probing
prepare_finalize()                delete()
execute_finalize()                delete()
finalize()                        delete()
health()                          provider/actor supervision
drain()                           actor shutdown
assess_update()                   compare current desired state in reconcile()
plan_upgrade()                    reconcile()
execute_upgrade()                 reconcile()
MutationIntent::Create            ctx.ensure()
MutationIntent::UpdateSpec        ctx.ensure() / manager update
MutationIntent::UpdateStatus      ctx.set_status()
MutationIntent::Delete            ctx.delete()
RequeueAt                         ctx.requeue_after()
DependencySnapshot                ctx.get() / ctx.watch()
```

The compiler should be used as the migration guide:

1. Delete old traits and DTOs.
2. Build.
3. Fix every provider error by applying this table.
4. Continue until all providers implement `ResourceDriver`.

Do not introduce a compatibility adapter for old controllers.

---

## 14. Long-running effects

Do not block a resource actor's mailbox on long-running external work.

Pattern:

```text
ResourceActor
    |
    | plan effect
    |
    +---- launch/call effect ----------------+
    |                                       |
    | return to mailbox                     |
    |                                       |
    |<------ EffectCompleted(result) --------+
    |
    v
continue reconcile
```

The actor must remain responsive to:

```text
Delete
SpecChanged
DependencyChanged
provider termination
cancellation
```

while an external operation is running.

Use Ractor forwarding/RPC patterns or spawned tasks that send a typed completion message back to the actor.

---

## 15. Internal watches

Internal watches are actor subscriptions.

Do not persist them.

Example:

```text
Volume/foo
    needs
Process/foo-mounter Ready
```

Runtime:

```text
VolumeActor
     |
     | Watch {
     |   condition = Ready
     |   subscriber = Volume/foo
     | }
     v
ProcessActor
```

The target actor performs the check and registration atomically in the same mailbox handler:

```rust
if state.matches(condition) {
    notify(subscriber, Satisfied);
} else {
    watchers.insert(watch_id, subscriber);
}
```

This prevents the lost-wakeup race:

```text
check status
     |
resource becomes ready
     |
register watch
```

because status transitions and watch registration are serialized by the target actor.

When the target changes state:

```text
ProcessActor
     |
     | status = Ready
     |
     +--------> VolumeActor
                  DependencySatisfied
                         |
                         v
                    Reconcile
```

No database access is involved.

---

## 16. Watch routing and actor restarts

Prefer routing watch registration through `ResourceManager`, even if the target subscription ultimately lives in the resource actor.

This lets `ResourceManager` retain an ephemeral dependency graph:

```text
Process/foo
    -> Volume/bar
    -> Guest/work
```

If `ProcessActor` dies:

```text
ResourceManager
    |
    +--> respawn/rebind Process/foo
    |
    +--> notify dependents:
         DependencyChanged(Process/foo)
```

Dependents reconcile and re-register watches.

Do not try to preserve watch subscriptions across daemon restarts.

---

## 17. External Resource API watches

External watches should be managed by an in-memory `WatchHub` owned by `ResourceManager`.

Every desired or runtime change increments a runtime revision.

Example:

```text
epoch 73

73:1 Volume/foo Created
73:2 Process/foo Created
73:3 Process/foo Ready
73:4 Volume/foo Ready
```

Flow:

```text
ResourceActor
     |
     | RuntimeChanged(status)
     v
ResourceManager
     |
     +--> update current in-memory view
     |
     +--> increment runtime revision
     |
     +--> append ResourceChange to bounded ring buffer
     |
     +--> fan out matching external watches
```

`LIST` should return:

```text
current matching resources
snapshot revision
```

Then:

```text
WATCH(after=snapshot_revision)
```

replays newer ring-buffer events and transitions to live delivery.

Because one `ResourceManager` serializes list snapshots, revisions, and watch registration, the list/watch handoff can remain gap-free within one daemon lifetime.

---

## 18. External watch behavior across restart

Do not persist status events or watch replay logs.

Use an epoch plus sequence.

For example:

```text
boot 73:
73:1
73:2
73:3

restart

boot 74:
74:1
```

A cursor from an older epoch is invalid:

```text
WATCH(after=73:3)
    |
    v
RevisionExpired
```

Client behavior:

```text
RevisionExpired
      |
      v
LIST
      |
      v
WATCH from returned revision
```

A bounded in-memory ring buffer should support short disconnect/reconnect within the same daemon epoch.

Desired result:

```text
live replay within daemon lifetime       YES
resume after brief client disconnect     YES
resume through d2bd restart              NO -> relist
persistent writes for status changes     ZERO
```

---

## 19. Status publication

Status is in memory only.

When a driver calls:

```rust
ctx.set_status(new_status);
```

The ResourceActor should:

```text
update local actor state
        |
        +--> evaluate internal watchers
        |
        +--> ResourceManager::RuntimeChanged
                    |
                    +--> update current ResourceView
                    +--> external watch fanout
```

Do not write status to SQLite.

A test should explicitly verify that repeated status transitions generate zero persistent writes.

---

## 20. Deletion

Deletion is desired state and must survive crashes.

Flow:

```text
API / Nix / parent removes resource
           |
           v
ResourceManager
           |
           | UPDATE resources SET deleting = 1
           v
COMMIT
           |
           v
ResourceActor::Delete
           |
           v
driver.delete()
           |
           v
actual resource removed
           |
           v
ResourceManager removes spec row
           |
           v
actor stops
```

If d2bd crashes after `deleting=1` but before cleanup completes, restart reconstructs the actor in deleting mode and retries cleanup.

Owned children should be removed automatically when their parent is deleted, unless a resource type explicitly supports orphaning/reparenting.

---

## 21. Owned child reconciliation

For controllers that derive multiple child resources, prefer declarative child reconciliation over imperative create/delete calls.

Expose something like:

```rust
ctx.reconcile_children([
    DesiredChild::new(...),
    DesiredChild::new(...),
]).await?;
```

`ResourceManager` compares:

```text
currently owned children
vs
new desired children
```

and performs:

```text
missing child       -> create
matching child      -> retain/update
obsolete child      -> mark deleting
```

This prevents stale children when parent configuration changes.

---

## 22. Provider model

Replace the controller registry/execution model with provider driver factories.

Suggested interface:

```rust
trait ResourceDriverFactory: Send + Sync + 'static {
    fn resource_types(&self) -> &[ResourceTypeName];

    async fn create(
        &self,
        key: ResourceKey,
    ) -> Result<Box<dyn ResourceDriver>, ResourceError>;
}
```

`ProviderDirectory` maps resource type/provider selection to a factory.

Possible shape:

```text
ProviderSupervisor
       |
       +-- volume-local factory
       +-- cloud-hypervisor factory
       +-- network-local factory
       +-- security-key factory
       +-- ...
```

If a provider requires shared singleton state, keep that state behind another actor or shared runtime service.

Examples:

```text
ProcessRuntimeActor
PipeWireActor
UdevActor
GpuOwnershipActor
NetlinkActor
```

Resource actors can call those shared actors.

---

## 23. Execution targets, Guests, and ZoneLink

This distinction is a hard architectural invariant of the rewrite.

A resource's **Zone authority** and **execution target** are separate dimensions.

A resource created in a Host Zone can target a Guest and run inside that Guest while remaining owned, named, persisted, authorized, and externally visible in the Host Zone.

Example:

```text
Host Zone /
    |
    +-- Process/foo
    |      target = Guest/A
    |
    +-- Endpoint/bar
    |      target = Guest/A
    |
    +-- Volume/data
           target = Host
```

`Process/foo` is still `/Process/foo`. Do not synthesize a second authoritative `/guest-a/Process/foo` resource merely because the process runs inside Guest/A.

### 23.1 Target abstraction

Introduce an explicit target layer. The exact names may vary, but the responsibilities must remain separate from Zone routing:

```rust
enum ResourceTarget {
    Host(ResourceRef),
    Guest(ResourceRef),
}

struct TargetDirectory {
    // exact Host/Guest target -> runtime handle
}

enum TargetHandle {
    Host(HostTargetHandle),
    Guest(GuestTargetHandle),
}
```

`ResourceManager` resolves the target named by the desired resource and gives the `ResourceActor` a `TargetHandle` through `ResourceContext`.

The realization path is:

```text
ResourceActor for /Process/foo
        |
        | target = Guest/A
        v
GuestTargetActor(A)
        |
        | authenticated ComponentSession
        v
Guest d2bd TargetRuntime
        |
        | realize / observe / delete
        v
target-local provider/effect code
        |
        v
Guest Linux
```

For a Host target:

```text
ResourceActor
     |
     v
HostTarget
     |
     v
local provider/effect code
```

The target abstraction decides **where effects occur**. It does not change the resource's Zone identity.

### 23.2 Guest-side realization is not a second resource authority

The Host-side `ResourceActor` remains authoritative for the logical resource if the resource belongs to the Host Zone.

The Guest may use an internal actor/object to manage the target-local realization:

```rust
struct TargetResourceInstance {
    source: ResourceKey,
    assignment_generation: u64,
    // target-local handles and observed state
}
```

That object is an implementation detail of the target runtime. It is not another API-visible resource, does not get an independent desired spec, and does not become part of the Guest Zone's resource namespace.

This prevents split-brain ownership where Host and Guest both believe they own the same desired resource.

### 23.3 ComponentSession / TargetRuntime is the generic Guest control path

Do not make `ZoneLink` the transport abstraction for all Host-to-Guest resource operations.

The generic path for a Host-owned resource targeting a Guest is:

```text
Host ResourceActor
      |
      v
GuestTargetActor
      |
      v
ComponentSession / target-control protocol
      |
      v
Guest TargetRuntime
```

The current v3 `d2bd-runtime` already distinguishes Host and Guest daemon modes and gives Guest mode a parent ComponentSession rather than a local Zone store. Preserve that separation in the Ractor runtime.

If existing protocol or variable names describe the generic Guest ComponentSession as `zone-link`, rename them where practical so the implementation does not encode the wrong semantic relationship. Prefer names such as `target-session`, `component-session`, `guest-control`, or `target-runtime` when the session carries generic target realization traffic.

Do not rename the actual `ZoneLink` resource or its namespace semantics.

### 23.4 ZoneLink remains a Zone topology resource

`ZoneLink` represents a relationship between Zones:

```text
Zone /  <-------- ZoneLink -------->  Zone /work
```

It is not synonymous with:

```text
"execute this resource inside Guest/A"
```

A ZoneLink may require a Guest and may itself use:

```text
ZoneLinkActor
      |
      | target = Guest/A
      v
GuestTargetActor
      |
      v
Guest TargetRuntime
```

That means ZoneLink is a **consumer of the same target mechanism** used by Process, Endpoint, or any other targetable resource. ZoneLink does not own that mechanism.

Ordinary resources may target Guest/A without creating a Zone edge. Conversely, a ZoneLink changes Zone topology even though its physical implementation may happen to run in a Guest.

### 23.5 Target failure and Zone failure are different events

If Guest/A disconnects or restarts:

```text
GuestTargetActor(A)
       |
       +--> mark target unavailable
       +--> reconnect/authenticate ComponentSession
       +--> notify affected ResourceActors
       +--> target-local discovery/adoption
       +--> reconcile
```

Do not delete or move Host-zone desired resources merely because their target is temporarily unavailable.

Likewise, failure or deletion of a ZoneLink must not implicitly delete unrelated Host-zone resources that happen to target the same Guest unless an explicit ownership relationship says so.

### 23.6 Remote Ractor is optional, not the initial requirement

The initial rewrite does not require the authoritative resource actor itself to move into the Guest or require `ractor_cluster` for Host-to-Guest execution.

Keep the actor that owns the logical resource in its authority process and put remote execution behind `TargetHandle` / `GuestTargetActor` and the existing authenticated D2B ComponentSession boundary.

A later implementation may replace the internals of that target path with Ractor remoting, but resource identity and Zone semantics must not depend on that choice.

---

## 24. Security boundaries

Do not remove D2B authentication/session boundaries merely because the execution model becomes actor-based.

Within one trusted process:

```text
ActorRef / ResourceHandle
```

can replace many controller assignment fences.

Across processes, guests, remote hosts, or providers:

```text
existing authenticated D2B session / bus
```

must still authenticate the peer and authorize allowed resource operations.

For Guest targets, the ComponentSession is a target-control security boundary. Bind target assignments to the expected Guest identity/session generation so a reconnect cannot accidentally inherit stale realization authority.

Do not infer Zone membership or ZoneLink authority merely because a Guest target session exists.

If Ractor cluster/remoting is used later, place D2B routing/authentication beneath or around it. Do not treat raw actor remoting as the security model.

---

## 25. Existing code to preserve

Keep or adapt:

```text
d2b-contracts*
resource schemas
RBAC/authentication
zone/session boundaries
target-runtime and ComponentSession security boundaries
Host/Guest target distinction
provider-specific effect ports
process conformance/adoption
provider supervisor process handling
audit logic that represents real security/accountability requirements
Nix resource compilation
ownership semantics
resource-specific discovery/adoption
broker privilege separation
```

The process provider/supervisor already contains important behavior around:

```text
probe
observe
adoption candidates
pidfd reconstruction
ambiguous launch quarantine
retained process handles
stop/finalize
```

Preserve that behavior.

---

## 26. Existing code to remove

The following should disappear or be radically reduced because they exist mainly to coordinate controllers through a persistent resource database:

```text
packages/d2b-resource-store-redb
packages/d2b-resource-store
most/all of packages/d2b-controller-toolkit

ResourceStoreBackend
RedbBackend
CheckedResourceStore

ControllerSource
Runner
PendingQueue

ResourceSnapshot
DependencySnapshot
ReconcileContext
ReconcilePlan
ReconcileResult

MutationIntent
ResourceMutationBatch

store-driven dependency watches
durable revision log
watch replay database
controller checkpoints
fresh reads before reconcile
status persistence
status mutation transactions
per-resource optimistic revision conflicts
controller worker queue
controller requeue persistence
store backpressure retry logic
redb read pool
redb watch coordinator
redb revision compaction
```

Only retain DTOs that still represent meaningful resource-domain contracts. Move those into `d2b-contracts-resource` or the new runtime crate.

---

## 27. d2bd-runtime integration

The existing `d2bd-runtime` composition currently assumes a Zone resource store and imports redb-specific resource runtime pieces.

Replace the runtime construction path so that an authority-bearing Zone runtime owns:

```text
ResourceManager actor
SpecStore
ProviderDirectory
TargetDirectory
Resource actors
external WatchHub
```

Host mode should register a local Host target plus Guest targets as Guests become available.

Guest mode is different: it should expose a target-local `TargetRuntime` over its authenticated parent ComponentSession. It must not create a second authoritative SpecStore/ResourceManager for Host-owned resources merely to realize them locally.

Remove the concept that readiness depends on a redb resource store being ready.

Runtime readiness should instead reflect:

```text
spec store opened
resource manager started
resource API started
providers registered
target directory initialized
required Guest target sessions admitted or marked unavailable
initial desired resources loaded
initial actors spawned
required recovery completed
```

Do not preserve a `store_ready` flag solely for compatibility.

---

## 28. Nix integration

Nix should feed top-level desired resources directly into `ResourceManager`, including their exact Host/Guest execution target where the resource contract supports targeting.

On startup:

```text
Nix materialization
      |
      v
ResourceManager::Apply
      |
      v
SpecStore
      |
      v
ResourceActor
```

On configuration changes:

```text
new Nix desired set
       |
       v
compare against Nix-owned stored specs
       |
       +-- add missing
       +-- update changed
       +-- mark removed as deleting
```

Track provenance/ownership if necessary:

```rust
enum DesiredOwner {
    Nix,
    Api,
    Resource(ResourceKey),
}
```

Actor-emitted children should use:

```text
DesiredOwner::Resource(parent)
```

---

## 29. Runtime revision type

Introduce a runtime-only revision type.

For example:

```rust
struct RuntimeRevision {
    epoch: u32,
    sequence: u64,
}
```

or encode it into a `u64` if wire compatibility requires it.

One possible compact encoding:

```text
[ 24-bit epoch ][ 40-bit sequence ]
```

The epoch only needs to advance once per daemon startup.

It is not a resource generation.

Keep these concepts separate:

```text
ResourceGeneration
    durable desired-spec version

RuntimeRevision
    ephemeral watch ordering
```

---

## 30. Resource generation

Resource generation should advance only when durable desired state changes.

Do not increment it on status changes.

Possible future simplification:

```text
generation = hash(canonical desired spec)
```

but do not require that in the first rewrite unless it naturally fits current contracts.

Stable `ResourceUid` should remain distinct from generation so delete/recreate semantics can still be represented.

---

## 31. Concurrency

One actor per resource removes the need for most per-resource concurrency control.

Still preserve explicit shared limits where the external backend requires them.

Examples:

```text
maximum concurrent process launches
maximum VM creations
serialized PipeWire changes
serialized device ownership changes
bounded broker calls
```

Implement those as:

```text
shared semaphore
shared runtime actor
provider actor
```

Do not reintroduce a global controller worker queue.

---

## 32. Retry behavior

Retry state is runtime-only.

Pattern:

```text
reconcile fails retryably
        |
        v
ctx.requeue_after(backoff)
        |
        v
Ractor timer sends Reconcile
```

If d2bd crashes while waiting for retry:

```text
restart
   |
   v
recover/adopt
   |
   v
reconcile immediately
```

There is no reason to restore the retry timer.

---

## 33. Provider/resource failure

Use Ractor supervision for runtime failures.

If a ResourceActor crashes:

```text
ResourceManager / supervisor
       |
       v
spawn replacement
       |
       v
load desired spec from memory/store
       |
       v
recover/adopt
       |
       v
reconcile
```

If a provider runtime actor crashes:

```text
ProviderSupervisor
       |
       v
restart provider
       |
       v
notify affected ResourceActors
       |
       v
DependencyChanged / ProviderChanged
       |
       v
reconcile
```

Do not persist failure state merely to recover it.

---

## 34. Cross-resource operations

Most current multi-resource mutation batches should disappear because actors communicate directly.

If a true durable multi-resource invariant remains, perform it through `ResourceManager`, which is the only writer to the SpecStore.

Example:

```text
ResourceManager::ApplyBatch([
    update Volume/foo,
    create Process/helper,
    delete Process/old-helper,
])
        |
        v
one SQLite transaction
        |
       COMMIT
        |
        +--> update in-memory catalog
        +--> notify actors
        +--> emit external watch events
```

Do not implement transactions across actor mailboxes.

---

## 35. Implementation order

This is a one-shot rewrite, but execute the work in a disciplined order.

The branch may remain broken until the later steps.

### Step 1: Add the new runtime

Add:

```text
ractor
rusqlite
packages/d2b-resource-runtime
```

Implement:

```text
ResourceManager
ResourceActor
ResourceDriver
ResourceDriverFactory
ResourceContext
SpecStore
WatchHub
ResourceHandle
ResourceTarget
TargetDirectory
HostTarget
GuestTargetActor
runtime revisions
```

Write focused unit tests for these primitives immediately.

### Step 2: Introduce Host/Guest target routing

Before converting providers, make target placement explicit.

Implement:

```text
ResourceTarget
TargetDirectory
HostTarget
GuestTargetActor
Guest TargetRuntime binding
ComponentSession-backed target operations
```

Prove these invariants with a minimal resource before proceeding:

```text
Host-zone resource + Host target
    -> realized locally

Host-zone resource + Guest target
    -> remains in Host ResourceManager/SpecStore
    -> realized through GuestTargetActor + ComponentSession
    -> no duplicate API-visible resource appears in the Guest
```

Do not use ZoneLink as the generic Host-to-Guest routing mechanism.

### Step 3: Replace d2bd Zone resource runtime construction

Remove redb resource-store opening and construct the new runtime.

Load desired specs.

Spawn resource actors.

Do not attempt compatibility with the old store.

### Step 4: Rewrite Resource API directly against ResourceManager

Delete:

```text
ResourceStoreBackend
RedbBackend
CheckedResourceStore
```

Keep external authorization/admission semantics.

Map API operations to ResourceManager RPC.

### Step 5: Delete old store crates

Remove:

```text
d2b-resource-store
d2b-resource-store-redb
```

Move surviving domain types to the correct contract/runtime crates.

Expect the build to break widely.

### Step 6: Delete the controller runtime

Delete:

```text
Runner
ControllerSource
PendingQueue
old watch loop
fresh read loop
checkpoint logic
conflict retry logic
status persistence
ReconcileResult mutation protocol
```

Replace with `ResourceDriver` and `ResourceContext`.

### Step 7: Convert every provider

Search for:

```text
impl ResourceReconciler
ResourceReconciler
ReconcileResult
MutationIntent
ResourceMutationBatch
DependencySnapshot
RequeueAt
UpdateStatus
```

Convert every occurrence mechanically using the mapping in section 13.

For providers that currently execute on a Guest target, preserve target-local privileged/effect behavior behind `TargetHandle` and the Guest `TargetRuntime`; do not turn the Guest realization into a second desired resource.

Do not add an adapter layer.

### Step 8: Replace dependency delivery

Convert store-watch-driven dependency triggers to:

```text
ctx.watch()
DependencySatisfied
DependencyChanged
```

Keep an ownership/dependency index only where useful for graph inspection, cycle detection, teardown, or resubscription.

### Step 9: Replace provider/controller registry execution

Remove controller assignment as the in-process execution mechanism.

Introduce:

```text
ProviderDirectory
ResourceDriverFactory
TargetDirectory
GuestTargetActor
provider/shared runtime actors
```

Keep authenticated cross-process session admission.

### Step 10: Implement full recovery/adoption

For every resource type, the equivalent of:

```text
discover
adopt
create
delete
```

must be correct enough that the daemon can restart without persisted status.

Recovery must run against the resource's execution target. Guest-targeted resources wait for/reconnect to the Guest target, then discover and adopt their target-local realizations without changing Zone ownership.

### Step 11: Rewrite integration tests around the new invariants

Do not try to preserve redb/store implementation tests.

---

## 36. Required tests

### Desired state

```text
API create persists before actor creation
Nix create persists before actor creation
actor Ensure persists before child actor creation
same Ensure is idempotent
Ensure with changed spec advances generation
delete marks durable deleting state before cleanup
```

### Recovery

```text
restart reconstructs all actors from stored specs
existing process is adopted
existing VM is adopted
existing volume/mount is adopted
missing desired resource is recreated
deleting resource resumes cleanup
owned-child graph reconstructs correctly
```

### Internal watches

```text
watch when condition already true -> immediate notification
condition changes before watch message is processed -> still no lost wakeup
target resource becomes Ready -> dependent reconciles
target actor crashes -> dependent is notified/reconciles/resubscribes
watch registrations are not persisted
restart rebuilds dependency relationships through reconciliation
```

### Targeting and ZoneLink separation

```text
Host-zone resource targeting Host is realized locally
Host-zone resource targeting Guest stays in Host ResourceManager and SpecStore
Host-zone resource targeting Guest is realized through GuestTargetActor + ComponentSession
Guest realization does not create a second API-visible desired resource
multiple resource types can target the same Guest
Guest disconnect leaves desired resources intact and marks target-dependent observed state unavailable
Guest reconnect causes discovery/adoption and reconciliation
ordinary Guest-targeted resource does not create or modify a ZoneLink
ZoneLink can itself target a Guest through the same TargetDirectory path
ZoneLink deletion does not delete unrelated resources targeting the same Guest
ZoneLink topology state and Guest target availability can change independently
```

### External watches

```text
LIST returns snapshot revision
WATCH(after=snapshot) receives all later matching events
ring-buffer replay works within one epoch
stale cursor from old epoch -> RevisionExpired
RevisionExpired -> relist works
slow subscriber is bounded/evicted
status change generates no disk write
```

### Provider failure

```text
resource actor restarts and adopts
shared provider actor restarts
affected resources reconcile after provider restart
```

### Ownership

```text
parent creation creates owned children
parent spec change removes obsolete children
parent deletion cleans up children
child cannot silently change owner
```

### Concurrency

```text
same resource never reconciles concurrently
different resource actors can reconcile concurrently
shared backend limits are honored
long-running effect does not block actor mailbox
```

---

## 37. Definition of done

The rewrite is complete when all of the following are true:

```text
[ ] d2b-resource-store-redb is gone
[ ] d2b-resource-store is gone or reduced to zero runtime/store responsibility
[ ] Runner is gone
[ ] ControllerSource is gone
[ ] PendingQueue is gone
[ ] ResourceReconciler is gone
[ ] ReconcileResult mutation protocol is gone
[ ] status persistence is gone
[ ] internal store watches are gone
[ ] persistent watch revision log is gone
[ ] all desired resources are represented by one authoritative resource actor in their owning Zone
[ ] Host-zone resources can target Guests without moving into a Guest Zone namespace
[ ] Guest target realizations are not exposed as duplicate desired resources
[ ] TargetDirectory cleanly separates Host and Guest realization paths
[ ] ZoneLink is used only for Zone topology semantics, not generic Guest targeting
[ ] all providers implement ResourceDriver or ResourceDriverFactory
[ ] all resource creation goes through ResourceManager
[ ] child specs are persisted before child actors are created
[ ] runtime watches are actor-based
[ ] external API watches use in-memory revisions/ring buffer
[ ] stale external watch cursors force relist after daemon restart
[ ] actors recover observed state through discovery/adoption
[ ] deleting resources survive restart
[ ] repeated status transitions perform zero persistent writes
[ ] current end-to-end host and guest scenarios work
[ ] Guest reconnect/adoption works for Host-owned Guest-targeted resources
```

---

## 38. Rules for the implementation agent

Follow these rules during the rewrite:

1. Do not preserve old abstractions solely to keep intermediate builds green.
2. Do not create a compatibility `ResourceStoreBackend` over `ResourceManager`.
3. Do not dual-write to redb and SQLite.
4. Do not persist runtime status.
5. Do not persist watch registrations.
6. Do not persist reconcile queues or retry timers.
7. Do not introduce event sourcing unless a concrete requirement proves necessary.
8. Do not reimplement Kubernetes-style resource revisions for runtime state.
9. Do not let provider code write the SpecStore directly.
10. All durable desired resource mutations must pass through ResourceManager.
11. All actor-created child resources must be persisted before realization.
12. Every external resource must be discoverable/adoptable after restart.
13. Preserve D2B security/session boundaries across trust domains.
14. Preserve provider-specific effect and adoption code when it remains useful.
15. Prefer deletion of obsolete infrastructure over adapting it.
16. Use compiler failures as the checklist for converting providers.
17. Keep resource contracts separate from runtime implementation details.
18. Keep `ResourceGeneration` and runtime watch revisions conceptually separate.
19. Never block a resource actor mailbox on an indefinitely long external operation.
20. At the end, there must be one resource execution model, not an old and new model side by side.
21. Never infer a resource's Zone from its execution target.
22. Never use ZoneLink as the generic abstraction for Host-to-Guest resource realization.
23. A Guest-side realization of a Host-owned resource is target-runtime state, not a second desired resource authority.
24. Keep ComponentSession/TargetRuntime generic enough to realize any permitted Guest-targeted resource, including but not limited to ZoneLink.

---

## 39. Design heuristic

When deciding whether an old subsystem survives, ask:

```text
Does this exist mainly because controllers communicate
through a persistent resource database?

               YES
                |
                v
             DELETE IT
```

Likely deletions:

```text
PendingQueue
store-driven dependency watches
fresh reads before reconcile
status persistence
revision-conflict retries
controller checkpoints
watch replay database
controller worker scheduling
same-process controller assignment mutation fences
```

Likely survivors:

```text
resource contracts
authorization
provider isolation
effect ports
adoption
process quarantine
auditing
ownership
Nix compilation
broker privilege separation
cross-domain authenticated sessions
Host/Guest target abstraction
Guest TargetRuntime / ComponentSession realization path
ZoneLink as Zone topology
```

---

## 40. Final mental model

```text
                     DURABLE AUTHORITY
                           |
                 +---------+---------+
                 |                   |
             Nix config          Spec Store
                 |                   |
                 +---------+---------+
                           |
                     desired state
                           |
                           v

                     RESOURCE ACTORS
                           |
             logical live resource state
                           |
                           v
                    TargetDirectory
                           |
                 +---------+---------+
                 |                   |
             HostTarget          GuestTarget
                 |                   |
                 |             ComponentSession
                 |                   |
                 |             Guest TargetRuntime
                 |                   |
                 +---------+---------+
                           |
                           v

                      ACTUAL SYSTEM
              processes / VMs / mounts /
              devices / networks / etc.

Separate plane:

Zone A  <------------ ZoneLink ------------>  Zone B

Zone topology is not execution placement.
```

The central rules are:

> Specs survive a crash. Reality survives a crash. Everything between those two is disposable and reconstructed.

> A resource belongs to a Zone, but it may be realized on a different Host or Guest target. `ZoneLink` links namespaces; it does not define execution placement.

That should be the organizing principle for the entire rewrite.
