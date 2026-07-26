# ADR 0046 Provider payload state

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-state` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-volume-local`, `d2b-contracts` v3 Volume/schema extensions, Provider toolkit, `nixos-modules` Zone bundle emitter |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-primitive-resource-composition`, `ADR-046-provider-model-and-packaging`, `ADR-046-resource-store-redb`, `ADR-046-componentsession-and-bus` |
| Supersedes | `d2b-core/src/storage.rs` generated contract, ADR 0034 storage/sync/adoption model for v3 |

## Purpose

This spec governs how Provider components record, own, initialize, and recover their durable operational state in d2b 3.0. The default surface for bounded non-secret operational state is the owning resource's `status` subresource. A Provider component declares durable payload state as a Volume resource **only** when a specific payload passes the [storage-need test](#storage-need-test); such state Volumes are always Volume resources, and there is no `ProviderState` ResourceType.

## Status-first state model (D087)

Bounded non-secret controller/Provider operational state belongs in the owning resource's `status` subresource whenever possible (D087). Resource `status` is the default durable observation and recovery surface. It is:

- **Revisioned** - every status write advances the resource revision and is retained in `revision_log` until compaction (D028);
- **Optimistic-status-writer controlled** - a status write carries an expected revision and a stale writer receives a conflict (D005);
- **RBAC-readable and redacted** - status is a separately authorized subresource (D027) readable by authorized API subjects only, and every field is bounded and redacted;
- **Reverified after restart** - status records observation, never authority; after a Zone or controller restart the owning controller reverifies status against external reality (running processes, cgroup leaves, external cloud/handle state) before relying on it.

The spec remains the desired-state authority. Status is observation only and is **never** a host-mutation or repair authority: a controller may not treat a status field as a substitute for reverifying external reality, and status can never grant, carry, or stand in for a privileged effect.

Bounded non-secret operational state that belongs in `status` includes:

- reconcile stage and phase detail (closed enums / conditions);
- opaque, non-authorizing external handles, IDs, and digests (cloud operation handles, external resource IDs, content digests) that are safe for authorized API readers and independently revalidated;
- adoption observations after restart (e.g. observed running-process identity re-derived from cgroup leaves);
- bounded counters and last-successful checkpoints;
- dependency-readiness observations.

Reconcile writes status only on a **material change**; there are no high-frequency byte streams, logs, metrics, command output, or ring buffers in status. Watches, revision compaction, and backpressure remain bounded per `ADR-046-resource-store-redb` and `ADR-046-resource-reconciliation`.

For `ResourceExport`/`ResourceImport` (D096), per-session lease handles, stream
session state, credits, and payload bytes are high-churn runtime state, not
resource status or Provider payload state. Their statuses carry only bounded
lease summaries, counts, generation digests, readiness, and D091 currency; they
never carry raw bytes, paths, device handles, FDs, tokens, or authority-conferring
session handles.

### Status prohibitions

Resource `status` MUST NOT contain:

- secrets, raw tokens, keys, or PSKs, or any credential source handle that confers authority;
- private endpoint, path, argv, environment, PID, or systemd-unit data;
- terminal, clipboard, or CTAP byte content;
- raw cloud error bodies or other unbounded provider diagnostics;
- large binary blobs;
- unbounded collections;
- any content whose churn would bloat revision history.

An opaque handle may appear in status only if it is bounded, non-secret, non-authorizing, safe for authorized API readers, and independently revalidated by the owning controller against external reality.

### State-authority split

Each kind of state has exactly one owner; no owner duplicates another's data:

| State | Sole owner |
| --- | --- |
| In-flight idempotency, retry, and transaction progress | Core Operation ledger |
| Latest bounded result / checkpoint / observation | Owning resource `status` |
| Security history | Authoritative audit stream |
| Metrics and traces | OTEL |
| High-frequency content streams / rings | Owning process memory |
| Filesystem paths and artifact integrity | Nix / private artifact catalog |
| Secret, large, binary, or revision-unsuitable durable payload with a recovery need | An explicitly declared Provider state Volume |

Provider state Volume payloads never duplicate resource refs, generation, backoff, idempotency, or session state held by resource rows, resource status, or the core Operation ledger.

## Storage-need test

A component declares a Provider state Volume only when a concrete payload passes an explicit storage-need test. A payload qualifies when at least one of the following holds:

1. **Secret / sensitive private recovery data** - the payload is or seals a secret (key material, PSK, sealed enrollment/admission record) and cannot enter status;
2. **Large / binary / file content** - the payload is a large or binary file that is unsuitable for the bounded status API;
3. **Private data unsafe for status readers** - the payload is private data that must not be visible to authorized status readers under RBAC;
4. **Bounded but revision-unsuitable data with a demonstrated recovery need** - the payload is bounded but its churn would bloat revision history, and there is a demonstrated recovery need that status cannot satisfy.

A component whose durable operational state is fully derivable from spec, status, the core Operation ledger, or independent external observation declares **no** state Volume. There is no empty, identity-only state Volume, and no state Volume is created for a stateless component.

## No ProviderState ResourceType

Decision D032 folds file, directory, ACL, and filesystem-view concerns into one `Volume` ResourceType. A declared Provider payload state is a Volume with a per-component view and a `stateSchema` extension in its spec. A separate `ProviderState` ResourceType would duplicate the owner/finalizer/lifecycle model already present in Volume and add a second governance layer for the same physical bytes. The Volume ResourceType is extended instead.

This decision is final within this spec and does not require a new decision register entry.

## ProviderStateSet (optional logical concept)

A **ProviderStateSet** is the OPTIONAL, query-time grouping of the declared Volume resources in a Zone whose `metadata.ownerRef` resolves to `Provider/<name>`. It is a query-time grouping, not a ResourceType or stored artifact, and it is **empty** for a Provider that declares no state Volume.

```text
ProviderStateSet(zone, provider-name) =
  { v : Volume | v.metadata.zone == zone
              && v.metadata.ownerRef == "Provider/<provider-name>" }
```

**Core ProviderDeployment ownership.** Core ProviderDeployment creates only the declared state Volumes in the ProviderStateSet from the signed state declarations in the Provider manifest, before launching the owning component's Process. The Provider controller does not invoke Volume creation APIs; operators and Nix never author component state Volumes.

**Export children excluded.** `virtiofs.d2bus.org.Export` children have `ownerRef: Volume/<source>` (not `ownerRef: Provider/<name>`) and are excluded from the ProviderStateSet. Only source Volumes are included.

**Only declared components included.** ProviderStateSet includes only the Volumes of components that declared one under the [storage-need test](#storage-need-test). A component that keeps its bounded non-secret operational state in `status`/the core Operation ledger contributes no Volume. There is never an empty identity-only Volume.

The controller deletes all ProviderStateSet Volumes when the Provider is removed, after finalizers on any owning Process or EphemeralProcess complete.

## Provider component state declaration

A semantic Provider component declares a state namespace in its component descriptor **only** when a payload passes the [storage-need test](#storage-need-test). Components whose operational state fits in `status`/the core Operation ledger declare no `stateNamespaces` entry, receive no state Volume, and contribute no Volume to the ProviderStateSet. A component never declares an empty-payload (`schemaId: null`) identity-only namespace.

```yaml
stateNamespaces:
  - id: main-state
    kind: state                   # always "state" for component state Volumes
    schemaId: example-provider.d2bus.org/controller/main-state
    schemaVersion: "1.0"
    schemaDigest: sha256:<hex>
    persistenceClass: persistent    # required; ephemeral/cache/config rejected
    sensitivityClass: private       # private | internal | shared-read
    migrationPolicy: pre-launch-required  # pre-launch-required | online-optional | none
    quotaBytes: 104857600           # required nonzero
    storageNeed: secret             # required: secret | large-binary | private-unsafe-for-status | revision-unsuitable
    sealingRequired: false
    placementMode: null             # omitted for Host-targeted; guest-local or host-backed-guest for Guest
    hostCustodyPermitted: false     # required true only for host-backed-guest
    views:
      main:
        rights: [read, write, create, delete, traverse]
      worker-read:
        path: public
        rights: [read, traverse]
```

Fields:

| Field | Rules |
| --- | --- |
| `id` | Stable component-local alphanumeric namespace identifier |
| `kind` | Always `state` for component state Volumes; `staging` for migration staging Volumes |
| `schemaId` | Qualified immutable schema name: `<provider-crate>/<component>/<namespace>`; a declared namespace always carries a non-null payload schema |
| `schemaVersion` | Semver `MAJOR.MINOR`; major increment requires migration |
| `schemaDigest` | Exact SHA-256 hex of the canonical schema definition |
| `persistenceClass` | Must be `persistent`; `ephemeral`, `cache`, and `config` are rejected with `component-persistence-class-forbidden` |
| `sensitivityClass` | `private`: single-process; `internal`: same-Provider multi-component; `shared-read`: cross-Provider read-only |
| `migrationPolicy` | `pre-launch-required`: component Process is not started until migration completes; `online-optional`: Provider may start while migration runs; `none`: no migration logic |
| `quotaBytes` | Required nonzero; minimum 4096 bytes enforced (zero rejected with `component-quota-zero`) |
| `storageNeed` | Required justification the payload satisfies: `secret`, `large-binary`, `private-unsafe-for-status`, or `revision-unsuitable`; a namespace whose payload is fully derivable from spec/status/core ledger/external observation is rejected with `component-state-not-justified` |
| `sealingRequired` | If true the Provider controller must bind a `sealingCredentialRef` before the Volume is marked Ready |
| `placementMode` | For Guest-targeted components: `guest-local` (source inside Guest, Host never holds bytes/paths/dirfds) or `host-backed-guest` (source on Host with virtiofs Export, requires `hostCustodyPermitted: true`). Omitted for Host-targeted components. Frozen in signed manifest; no fallback. |
| `hostCustodyPermitted` | Required `true` for `host-backed-guest`; must be absent or `false` for `guest-local`. Core ProviderDeployment rejects `host-backed-guest` without `hostCustodyPermitted: true` with `placement-host-custody-violation`. |
| `views` | Named views declared in the component descriptor; subset is also declared in the Volume spec |

`schemaId`, `schemaVersion`, `schemaDigest`, `kind`, `persistenceClass`, `storageNeed`, `placementMode`, `hostCustodyPermitted`, and `views` are signed into the component descriptor and the Provider package digest. Any change increments the component descriptor version and the Provider resource generation.

## Volume creation and ownership

Core ProviderDeployment creates one Volume per **declared** component state namespace per execution target from the signed state declarations in the Provider manifest, before launching the owning component's Process. Components that declare no state namespace get no Volume. The Provider controller does not invoke Volume creation APIs. Operators and Nix never author component state Volumes. Each declared Volume:

```yaml
apiVersion: resources.d2bus.org/v3
type: Volume
metadata:
  name: example-provider--controller--main-state--host-system
  zone: dev
  ownerRef: Provider/example-provider
spec:
  providerRef: Provider/volume-local          # all source Volumes use volume-local; volume-virtiofs owns only Export children
  kind: state                                 # required for component state Volumes
  persistenceClass: persistent               # required; ephemeral/cache/config rejected
  sensitivityClass: private
  stateSchema:
    schemaId: example-provider.d2bus.org/controller/main-state
    schemaVersion: "1.0"
    schemaDigest: sha256:<hex>
    migrationPolicy: pre-launch-required
  quotaBytes: 104857600
  quota:
    maxBytes: 104857600
    maxInodes: 4096
    enforcement: none
  sealingCredentialRef: null
  source:
    executionRef: Host/host-system
    settings:
      kind: local-path
      sourcePolicyId: provider-state-persistent  # opaque source policy; no host path (D082)
  layout:
    - path: state
      type: directory
      ownerRef: User/example-provider-system
      groupRef: User/example-provider-system
      mode: "0700"
      sensitivity: private
      createPolicy: create-if-never-provisioned
      repairPolicy: exact-owner
      cleanupPolicy: owner-controlled
      noFollow: true
  views:
    main:
      path: state
      rights: [read, write, create, delete, traverse]
    worker-read:
      path: state/public
      rights: [read, traverse]
  identityMarker:
    class: broker-maintained
    markerRoot: provider-state-markers
  snapshotPolicy: null
  retentionPolicy: null
```

Volume naming convention: `<provider-name>--<component-id>--<namespace-id>--<execution-ref-short>`. The convention is enforced at runtime by checking that every Volume in the ProviderStateSet matches a declared namespace and execution target.

Process resources mount the Volume using the declared view:

```yaml
mounts:
  - volumeRef: Volume/example-provider--controller--main-state--host-system
    view: main
    mountPath: /state
    access: read-write
    required: true
```

**Admission invariants.** Core ProviderDeployment enforces these constraints when creating a Volume from a declared component descriptor state namespace:

| Constraint | Violation error |
| --- | --- |
| `kind: state` required for all component state Volumes | `component-kind-invalid` |
| declared namespace payload must pass the storage-need test (a `storageNeed` justification of `secret`, `large-binary`, `private-unsafe-for-status`, or `revision-unsuitable`); state fully derivable from spec/status/core ledger/external observation is rejected | `component-state-not-justified` |
| `persistenceClass: persistent` required; `ephemeral`, `cache`, and `config` rejected | `component-persistence-class-forbidden` |
| `quotaBytes ≥ 4096`; zero or smaller values are rejected; base `quota.maxBytes` equals `quotaBytes` and `quota.maxInodes` is nonzero | `component-quota-too-small` |
| `host-backed-guest` requires `hostCustodyPermitted: true` in signed descriptor | `placement-host-custody-violation` |
| Credential, audit, remote-node, or cloud-control schemas require `guest-local` | `guest-local-required` |
| layout `ownerRef`/`groupRef` must reference a Nix-preprovisioned User principal or bounded system pool; runtime-created principals rejected | `volume-principal-not-preprovisioned` |

## No bootstrap state Volume

Because every fixed bootstrap component - `system-core`, `system-minijail`, and the first `volume-local` controller instance on each execution target - keeps its bounded non-secret operational state in resource `status` and the core Operation ledger and declares **no** Provider state Volume, no component requires a state Volume before a `volume-local` instance is Ready. The previously specified mandatory-state bootstrap cycle and its per-execution-target local bootstrap storage mechanism are therefore removed entirely (D086, superseded by D087). There is no hidden bootstrap store, no closed bootstrap cycle, and no implicit framework-created bootstrap Volume.

A fixed bootstrap component that reaches Ready, adopts running processes, and re-derives its observed state from the core Operation ledger, resource `status`, and independent external observation (cgroup-leaf scanning, fresh pidfds, marker reverification against external reality) needs no durable payload Volume. If a future bootstrap component needs secret or large private recovery state that cannot enter status, it requires a new reviewed design that declares an ordinary optional state Volume under the [storage-need test](#storage-need-test); it does not reintroduce a bootstrap-storage exception.

## State placement under Host/Guest/user execution

### System-domain Host process

A stateful system-domain Process under `Host/<name>` uses a Volume backed by `Provider/volume-local` with `source.executionRef` pointing to the same Host. The volume-local Provider manages all layout, ACL, and lifecycle operations on the host filesystem under the anchored Volume root.

### User-domain Host process

A stateful user-domain Process under `Host/<name>` with `domain: user` and a resolved `userRef` uses a Volume whose source is the per-user subtree. The Volume layout owner/group entries bind `User/<username>` references. The volume-local Provider resolves these at provision time through the Host's User resource. A user-domain Volume must not be shared with a system-domain Volume or any other user's Volume.

### Guest (VM/sandbox/remote) process

A stateful Provider component whose Process runs under `Guest/<name>` uses one of two explicit placement modes, frozen in the signed component descriptor. There is no fallback or runtime selection.

#### guest-local

The source Volume lives inside the Guest; the Host never holds bytes, dirfds, or identity markers for it:

```yaml
spec:
  providerRef: Provider/volume-local
  kind: state
  persistenceClass: persistent
  source:
    executionRef: Guest/<name>
    settings:
      sourcePolicyId: <policy-id>   # opaque ID into volume-local's allowedHostPaths catalog for this Guest domain
  placementMode: guest-local        # Host never holds bytes, dirfds, or paths for this Volume
```

- Reconciled by the volume-local controller running inside the Guest.
- The Guest-local `volume-local` instance reaches Ready without a state Volume (it keeps its bounded operational state in `status`/the core Operation ledger); all declared Volumes for that Guest domain are created by Core ProviderDeployment after volume-local is Ready in that domain.
- The Host volume-local controller holds no dirfd, path, byte content, or identity marker for this Volume.
- **Mandatory for** components carrying gateway or realm credentials, remote-node registration, audit state, or cloud control plane state. Core ProviderDeployment rejects `placementMode: host-backed-guest` for these schema categories with `guest-local-required`.

#### host-backed-guest

The source Volume lives on the Host; volume-local creates a `virtiofs.d2bus.org.Export` child resource per attachment to provide Guest access:

```yaml
spec:
  providerRef: Provider/volume-local
  kind: state
  persistenceClass: persistent
  source:
    executionRef: Host/<name>
    settings:
      sourcePolicyId: <policy-id>   # opaque ID into volume-local's allowedHostPaths catalog
  placementMode: host-backed-guest
  hostCustodyPermitted: true        # required; absent or false → placement-host-custody-violation
```

- volume-local creates one `virtiofs.d2bus.org.Export` child (ownerRef: Volume/<source>, providerRef: volume-virtiofs) per `attachments[]` entry. volume-virtiofs owns only the virtiofsd worker Process and Export lifecycle; it does not own or replicate source bytes.
- Permitted only when the signed descriptor explicitly carries `hostCustodyPermitted: true`.
- All lifecycle operations (migration, sealing, snapshots, relocation, incident hold, destruction) apply to the source Volume.

**ProviderStateSet.** In both modes the source Volume has `ownerRef: Provider/<provider>` and is included in the ProviderStateSet. `virtiofs.d2bus.org.Export` children have `ownerRef: Volume/<source>` and are excluded. For `guest-local` there are no Export children.

All lifecycle decisions in this spec - migration, sealing, snapshots, relocation, incident hold, and destruction - apply to the source Volume in both modes. For `host-backed-guest` relocation, Export children follow the source Volume per `ADR-046-primitive-resource-composition`.

## No cross-domain shared dirfd

The volume-local Provider never hands out a dirfd, file descriptor, or raw path that would be accessible to a process in a different domain or a different User. This invariant is enforced through:

1. Volume layout entries bind exact `ownerRef`/`groupRef`/`mode` values; the provider validates inode owner against those references before exposing a view.
2. Process mount specs select a named view; the volume-local Provider validates that the mounting Process's domain and userRef are compatible with the declared Volume `sensitivityClass`.
3. A `private` Volume is mounted by exactly one Process at a time; the provider rejects concurrent mounts outside the same component instance.
4. An `internal` Volume is mountable only by Processes controlled by the same Provider, determined through the registered controller Provider/owner chain.
5. `shared-read` allows cross-Provider read-only view mounts but prohibits any write access to the shared path.
6. The volume-local Provider never passes a host filesystem fd to a process in a different domain. For `host-backed-guest` state, volume-local creates a `virtiofs.d2bus.org.Export` child (providerRef: volume-virtiofs) that exposes the declared view over virtiofs; the underlying dirfd is never passed across domains directly. For `guest-local` state, the Host volume-local controller holds no dirfd, path, byte content, or identity marker for the Volume; all filesystem operations are performed by the volume-local controller inside the Guest.

Any violation fails the Process launch with a typed `volume-domain-mismatch` error.

## Volume stateSchema extension

The `stateSchema` block in the Volume spec:

| Field | Rules |
| --- | --- |
| `schemaId` | Matches the exact `schemaId` in the component descriptor; immutable after first provision |
| `schemaVersion` | Current desired schema version; a change triggers migration |
| `schemaDigest` | Hash of the canonical schema definition; validated before migration |
| `migrationPolicy` | Inherited from component descriptor at creation; immutable per Volume |

The volume-local Provider stores the installed schema version in the Volume's identity marker file. On every open it compares the marker's `installedSchemaVersion` to the spec's `schemaVersion`. A mismatch is reflected in Volume status before any Process is allowed to mount the view.

### Extended Volume status for state

ResourceType-specific status additions:

| Field | Values and rules |
| --- | --- |
| `stateSchemaPhase` | `current` \| `migration-required` \| `migrating` \| `migration-committed` \| `migration-failed` |
| `installedSchemaVersion` | Semver string of the version currently on disk; null before first provision |
| `markerStatus` | `verified` \| `missing` \| `replaced` \| `unknown` |
| `sealingStatus` | `none` \| `sealed` \| `rotation-pending` \| `rotation-failed` |
| `quotaUsage` | Optional bounded current `{ usedBytes, inodeCount }` reported by the provider |
| `lastMigrationAt` | Optional RFC 3339 UTC completion timestamp of the last migration |

`stateSchemaPhase != current` and `markerStatus != verified` both block a `pre-launch-required` component Process from moving to Ready.

## Identity markers and fail-closed detection

Every persistent or cache-class Volume provisioned by the volume-local Provider has an identity marker. The marker:

1. is a regular file under a broker-maintained root outside the Volume's own tree (equivalent to the TPM marker root at `swtpm-markers`);
2. records the Volume's `(st_dev, st_ino)` at first provision, the schemaId, schemaVersion, and a tamper-evident digest;
3. is written by the broker at provision time (`class: broker-maintained`) or by the Provider controller at post-provision init (`class: provider-maintained`);
4. is checked on every daemon restart and on every Process launch that mounts the Volume.

Failure modes and responses:

| Condition | Response |
| --- | --- |
| Marker missing after prior provision | `markerStatus: missing`; Volume → `Failed`; blocked Processes → `Degraded` |
| `st_ino` mismatch (directory replaced) | `markerStatus: replaced`; Volume → `Failed`; never silently re-provision |
| Marker present but Volume root absent | `markerStatus: missing`; Volume → `Failed` |
| `installedSchemaVersion` > spec version | Volume → `Failed`; `stateSchemaPhase: migration-failed`; manual rollback required |

None of these cases auto-recover. Operator intervention clears the condition after confirming integrity.

## Quota enforcement

When `quotaBytes > 0`, the volume-local Provider:

1. Checks the available space on the backing filesystem before provisioning and reports `quota-insufficient` if the requested quota cannot be reserved.
2. Enforces a per-Volume maximum using the Volume root's filesystem quota where the backing filesystem supports it, or a soft-check on write by the Provider.
3. Reports `quotaUsage` in Volume status at a bounded polling interval (max every 60 s).
4. Rejects mounts whose views have `rights: [write, create]` when the Volume is at or above quota, returning a typed `volume-quota-exceeded` error.

Quota metadata lives in the Volume spec and is validated against the component descriptor at creation. A mismatch between descriptor and Volume spec fails Volume admission with `quota-mismatch`.

## Within-Volume transactions

Provider components may write structured state atomically using the toolkit's Volume transaction helpers (backed by `d2b-state`'s `AtomicFilesystem`). The write protocol:

1. Writer opens a temp file in the Volume view's anchored root with `O_CLOEXEC | O_TMPFILE`.
2. Writer serializes payload as canonical JSON and writes a `StateEnvelope` wrapping the digest and generation.
3. Writer calls `fsync` on the temp fd.
4. Writer calls `linkat` to rename the temp fd into the target relative path.
5. Writer calls `fsync` on the parent directory fd (anchored, never caller-controlled).

Step 4 is atomic on Linux. A crash between steps 3 and 5 leaves the old file intact. The toolkit validates the StateEnvelope digest and generation bound before exposing the payload to the caller.

The `generation` field in `StateEnvelope` is the component's own optimistic version counter, not the Zone resource generation. It is used for expected-previous generation checks in the component's own application logic.

No cross-Volume, cross-process, or cross-schema transaction is defined. Multi-object consistency uses the cross-component migration protocol.

## Schema migration

### Pre-launch migration

A Volume whose `migrationPolicy: pre-launch-required` and `stateSchemaPhase: migration-required` (i.e., `installedSchemaVersion != spec.stateSchema.schemaVersion`) blocks the owning component's Process from starting.

The Provider controller's reconcile handler:

1. Detects `stateSchemaPhase: migration-required` on the Volume via watch.
2. Sets a `Migrating` condition on the Volume status.
3. Creates an EphemeralProcess with `ownerRef: Volume/<name>` and a signed migration template from the Provider package.
4. The EphemeralProcess runs the migration operator binary, which:
   a. opens the Volume view via its declared mount;
   b. reads the installed schema version from the marker;
   c. runs the schema-specific migration operator up to the target version;
   d. writes the new marker with `installedSchemaVersion = target`;
   e. exits 0.
5. On `EphemeralProcess.status.phase = Succeeded`, the Provider controller updates Volume status to `stateSchemaPhase: current` and clears the `Migrating` condition.
6. The component Process may now start.

### Online migration (online-optional)

The component Process starts while the EphemeralProcess migration runs concurrently. The component must be capable of handling the old schema version until migration completes. The controller coordinates the cutover by setting a `MigrationPending` condition; the component observes it through its ComponentSession service interface and switches to the new schema layout after the condition clears.

### Migration operator requirements

Migration operators must be:

- deterministic given the same source schema version and source data;
- idempotent (safe to re-run after a crash at any point);
- roll-forward only: an operator never downgrades data already at the target version.

## Cross-component migration coordination

When a Provider has N stateful components sharing a related schema (e.g., a controller and a service that jointly own a coordinated state layout), the migration involves all N Volumes together.

### Prepare phase

1. Provider controller sets a `PrepareMigration` condition on all N Volumes simultaneously via a ResourceMutationBatch.
2. All N component Processes acknowledge the condition through their ComponentSession and stop mutating their state views.
3. Each Process signals readiness by setting a `MigrationReady` condition on its own Process status.

### Staging Volume

The controller creates a staging Volume for each migrating Volume:

```yaml
type: Volume
metadata:
  name: example-provider--controller--main-state--host-system--staging
  ownerRef: Volume/example-provider--controller--main-state--host-system
spec:
  providerRef: Provider/volume-local
  persistenceClass: ephemeral
  source:
    executionRef: Host/host-system
```

The staging Volume has `ownerRef` pointing to its source Volume. It is used as the migration workspace and is destroyed after successful commit or rollback.

### Commit phase

1. All N migration EphemeralProcess jobs complete and report `Succeeded`.
2. Controller atomically swaps staging content into the primary Volumes using the toolkit's `AtomicFilesystem.rename_into` helper.
3. Controller updates all N Volume status entries to `stateSchemaPhase: current` and removes `PrepareMigration` conditions.
4. Staging Volumes are deleted (no finalizer required; ephemeral lifecycle).
5. Component Processes are unblocked to mount the migrated views.

### Precommit rollback

If any migration EphemeralProcess reports `Failed` before the commit swap:

1. Controller sets `MigrationAborted` condition on all N Volumes.
2. Staging Volumes are deleted.
3. All N Volumes remain at their pre-migration `installedSchemaVersion`.
4. Component Processes that were running under `online-optional` continue on the old schema.
5. Volume `stateSchemaPhase` is set to `migration-failed` with a bounded typed reason.

Rollback is only valid before the atomic commit swap. After commit, only roll-forward (a further migration to the target version) is valid. The spec is deterministic: the migration EphemeralProcess reports one of `Succeeded` or `Failed`; the controller never enters an ambiguous mid-swap state.

### Roll-forward after interrupted commit

If the Zone daemon crashes after the atomic file-level rename but before Volume status is updated:

1. On restart, the volume-local Provider detects `installedSchemaVersion == target` in the marker.
2. The Volume status is corrected to `stateSchemaPhase: current` by the Provider controller's startup reconcile.
3. The staging Volume is detected as orphaned and GC'd under the unclaimed cleanup policy.

## Secret sealing

A Volume whose `stateSchema.sensitivityClass: private` and `sealingRequired: true` requires a `sealingCredentialRef`:

```yaml
sealingCredentialRef: Credential/example-provider-state-key
```

The referenced Credential must be Ready before the Volume can be provisioned. The volume-local Provider:

1. reads the Credential lease to obtain the envelope encryption key material;
2. wraps each `StateEnvelope` under the envelope key before writing;
3. never stores the raw key material on disk;
4. updates `sealingStatus` to `sealed` in Volume status.

Key rotation is triggered by a Credential generation change:

1. Controller detects `Credential.status.observedGeneration` has advanced.
2. Controller sets `sealingStatus: rotation-pending` on the Volume.
3. A rotation EphemeralProcess re-encrypts the Volume content under the new key.
4. On success, `sealingStatus: sealed` is restored.
5. A rotation failure sets `sealingStatus: rotation-failed`; the Volume is still readable under the old key until resolved.

The sealed content format and KDF parameters are owned by the volume-local Provider. No raw credential bytes or key material enter Volume status, audit records, OTEL spans, or logs.

## Snapshots

A Volume snapshot is an immutable point-in-time copy of the Volume's active view, created by an EphemeralProcess snapshot job owned by the Provider controller.

Volume spec:

```yaml
snapshotPolicy:
  retainCount: 3
  retainDurationHours: 168   # 7 days; 0 = retain only by count
  triggerOnMigration: true   # automatically snapshot before every migration
  triggerOnRelocation: true
```

Volume status:

```yaml
snapshots:
  - id: snap-<opaque>
    createdAt: 2026-07-22T00:00:00.000Z
    schemaVersion: "1.0"
    sizeBytes: 12345678
    trigger: pre-migration
    phase: Ready   # Ready | Failed | Expired
```

Snapshots are stored in a Provider-private path under the Volume root (e.g., `.snapshots/`) and are never exposed through the component's own views. Snapshot retention uses the normal EphemeralProcess cleanup policy; expired snapshots are removed by the Provider controller.

## Staging Volumes

Staging Volumes have `persistenceClass: ephemeral` and `ownerRef` pointing to the parent Volume. Their lifecycle:

1. Created by the Provider controller for migration, relocation, or large atomic-swap operations.
2. Mounted by migration EphemeralProcess resources only; no long-lived component Process mounts a staging Volume.
3. Removed on successful commit or failed rollback before the component Process is unblocked.
4. Detected as unclaimed and GC'd under the unclaimed Volume cleanup policy if the owning Provider is removed before cleanup completes.

## Retention policy

Volume retention policy applies to `ephemeral` and `cache`-class Volumes after the owning Provider or component Process is deleted:

```yaml
retentionPolicy:
  successfulTtlHours: 1
  failedTtlHours: 24
  incidentHoldEnabled: true
```

`successfulTtlHours` / `failedTtlHours` start from `deletionRequestedAt`. A Volume under incident hold ignores both TTLs until the hold is cleared.

A `persistent` Volume is never auto-expired; it is only removed after the Provider controller's reconcile explicitly deletes it with expected revision.

## Destruction

The Provider controller deletes a Volume by:

1. Setting `deletionRequestedAt` through the resource API.
2. Waiting for all mounting Processes to stop (finalizer).
3. The volume-local Provider's controller handler: removes layout paths using fd-relative `unlinkat` operations anchored within the Volume root, followed by `fsync` on the parent directory.
4. Where `sensitivityClass: private` and `sealingRequired: true`, the provider shreds key material before layout removal.
5. Removes the identity marker file.
6. Removes the Volume root directory.
7. Commits the finalizer removal.
8. Core emits a `Deleted` event (event-only; no further resource row mutation occurs in this step).
9. Core removes the resource row and all index entries atomically.
10. Core appends a post-commit audit record using a dedup/exactly-once recovery key.

The layout removal is ordered leaf-first and parent-last. Partial removal is detected on restart by the marker check; a partially removed Volume that still has a valid marker is quarantined rather than silently re-provisioned.

## Relocation

State relocation moves a Volume's backing store from one Host or execution target to another. A relocation:

1. Provider controller sets a `Relocating` finalizer on the source Volume and stops component Processes that mount it.
2. Creates a destination Volume (may be in the same or a different Zone, if a future cross-Zone extension permits; otherwise same Zone).
3. Creates a relocation EphemeralProcess that copies the source Volume tree to the destination using anchored read and write operations.
4. On successful copy: controller mounts the destination Volume in place of the source; removes the source finalizer; deletes the source Volume.
5. On failed copy: source Volume and its finalizer remain; operator resolves.

Cross-Host relocation is a prerequisite for Guest migration (moving the source Volume that backs a Guest's virtiofs attachment from one host to another). The `virtiofs.d2bus.org.Export` child resources (owned by volume-virtiofs) are reconciled by volume-virtiofs to point to the new source after the copy completes; the exact protocol is governed by `ADR-046-primitive-resource-composition` Volume attachment spec.

## Incident hold

An incident hold blocks Volume destruction and any migration commit:

```yaml
# Held by an authorized operator or incident controller
conditions:
  - type: IncidentHold
    status: "True"
    reason: active-incident
    message: bounded operator description
    observedGeneration: 3
    lastTransitionAt: 2026-07-22T00:01:00.000Z
```

The `IncidentHold` condition:

- is set by an authorized administrative Role via the status subresource;
- blocks `deletionRequestedAt` processing, migration commit, and staging Volume removal;
- does not block read-only mounts or status observation;
- is cleared only by the same administrative Role;
- is preserved through daemon restart and Zone reconcile.

An EphemeralProcess that completes while under incident hold retains its terminal status and output beyond its normal TTL until the hold is cleared.

## Worker subviews

Worker Processes with narrow access receive a named Volume view with limited rights:

```yaml
mounts:
  - volumeRef: Volume/example-provider--controller--main-state--host-system
    view: worker-read
    mountPath: /read-state
    access: read-only
```

The `worker-read` view is declared in the Volume spec and the component descriptor. The volume-local Provider enforces the view rights at mount time: a worker mount that requests any right absent from the view declaration fails with `volume-view-rights-exceeded`.

Worker Process resources are not owners of the Volume and carry no finalizer. Stopping the worker does not affect the Volume lifecycle.

## Unclaimed Volume GC

A Volume is unclaimed when:

- its `metadata.ownerRef` resolves to a Provider resource that has been deleted; or
- the Provider resource's ProviderStateSet declaration no longer includes this Volume's `stateSchema.schemaId`; or
- the Volume was created by a migration workflow that was abandoned (staging Volume with a missing or deleted owning Volume).

The core cleanup controller identifies unclaimed Volumes via the owner_index on each garbage collection pass. Unclaimed persistent Volumes:

1. receive a `Unclaimed` condition from the core cleanup controller;
2. are reported in Zone status and operator-visible diagnostics;
3. are not automatically deleted: operator confirms via an explicit delete request.

Unclaimed ephemeral and staging Volumes are automatically deleted after a configurable unclaimed TTL (default 1 h).

## Status, audit, and OTEL

### Volume status (state extensions)

See the extended status fields table in the [Volume stateSchema extension](#volume-stateschema-extension) section.

### Audit events

Each state-lifecycle transition emits one audit record through the Zone audit stream. Event kinds for state:

| Event kind | Trigger | Payload |
| --- | --- | --- |
| `volume-provisioned` | First-provision complete | zone, volume-ref, schemaId, schemaVersion, persistenceClass |
| `volume-migration-start` | Migration EphemeralProcess created | zone, volume-ref, from-version, to-version, migration-policy |
| `volume-migration-committed` | Migration committed | zone, volume-ref, from-version, to-version |
| `volume-migration-failed` | Migration EphemeralProcess failed | zone, volume-ref, from-version, to-version, bounded-reason |
| `volume-migration-rolled-back` | Migration precommit rollback completed | zone, volume-ref |
| `volume-snapshot-created` | Snapshot EphemeralProcess succeeded | zone, volume-ref, snapshot-id, trigger |
| `volume-relocation-start` | Relocation EphemeralProcess created | zone, volume-ref, from-execution-ref |
| `volume-relocation-committed` | Relocation complete | zone, volume-ref, to-execution-ref |
| `volume-incident-hold-set` | IncidentHold condition added | zone, volume-ref, actor |
| `volume-incident-hold-cleared` | IncidentHold condition removed | zone, volume-ref, actor |
| `volume-sealing-rotation-start` | Credential rotation triggered | zone, volume-ref |
| `volume-sealing-rotation-committed` | Credential rotation complete | zone, volume-ref |
| `volume-destroyed` | Volume fully destroyed | zone, volume-ref, schemaId |

No bytes of Volume content, credential material, migration data, raw paths, or process argv enter audit records.

### OTEL metrics

Cardinality-bounded metric labels are: `provider`, `schema_id`,
`schema_version`, `persistence_class`, `operation`, `trigger`, and `outcome`.
Zone identity is carried only by the bounded `d2b.zone` OTEL resource
attribute, never by a metric label.

| Metric | Unit | Labels |
| --- | --- | --- |
| `d2b_volume_state_size_bytes` | Gauge, bytes | provider, schema_id |
| `d2b_volume_state_migration_total` | Counter | provider, schema_id, outcome |
| `d2b_volume_state_migration_duration_ms` | Histogram | provider, schema_id |
| `d2b_volume_state_snapshot_total` | Counter | provider, schema_id, trigger |
| `d2b_volume_state_marker_check_total` | Counter | provider, outcome |
| `d2b_volume_state_quota_exceeded_total` | Counter | provider |

No schema content, raw path, instance ID, process arguments, or credential identifier enters any metric label.

## Async controller integration

A Provider controller that declares one or more state Volumes follows this integration pattern in its async reconcile loop. A controller with no declared state Volume writes its bounded operational observations to resource `status` on material change and skips the Volume-specific steps below.

1. **Watch** the declared Volumes in the ProviderStateSet (possibly none) and all EphemeralProcess resources it owns.
2. On `spec-generation-changed` for the Provider resource: diff the new declared component descriptor state namespaces against existing Volumes; create, update, or delete Volumes as needed.
3. On `Volume.status.stateSchemaPhase = migration-required` for any declared Volume: dispatch the migration workflow (staging Volume, EphemeralProcess, commit/rollback).
4. On `Volume.status.markerStatus = missing | replaced`: set `Degraded` on affected component Processes and surface the typed condition.
5. Gate each component Process's desired phase on its mounted Volume's `phase == Ready` and `stateSchemaPhase == current`. Use a declarative dependency selector in the controller descriptor that includes every Volume the component's Process mounts.
6. On EphemeralProcess completion, apply the commit or rollback protocol, then delete the EphemeralProcess normally.

The reconcile context contains the authorized async ResourceClient provided by the d2b-bus / ComponentSession stack (see `ADR-046-componentsession-and-bus`). No handler holds a blocking filesystem call across an `await`; Volume layout operations run in a bounded blocking adapter. Async sub-operations (migration, sealing, snapshot, relocation EphemeralProcess coordination) carry a cancellation token derived from the same session so that controller shutdown propagates atomically.

A controller that creates a Volume it does not subsequently manage (e.g., a controller that delegates migration to a sibling) must not hold the Volume ownerRef; the actual Volume owner performs all status writes.

## Nix configuration surface

### Resource authoring shape

All Zone resources are authored in Nix using a single generic attrset that mirrors the canonical ResourceSpec schema:

```nix
d2b.zones.<zone>.resources.<name> = {
  type = "...";    # exact ResourceType name
  spec = {
    # Exact spec fields - same names and nesting as the canonical ResourceSpec JSON
    # No renaming, no alternative vocabulary
  };
  # status is omitted: it is read-only, filled by the Zone runtime at runtime
};
```

The NixOS module derives:
- `metadata.name` from the attr key (`<name>`)
- `metadata.zone` from the enclosing Zone attr key (`<zone>`)
- `metadata.apiVersion` as `"resources.d2bus.org/v3"` (defaulted; not written by the user)

Core fills `metadata.uid`, `metadata.generation`, `metadata.resourceVersion`, `metadata.creationTimestamp`, `metadata.managementFields`, and all `status.*` fields at runtime. None of these appear in the Nix source or the emitted bundle.

### Provider resource example

```nix
d2b.zones.dev.resources.example-provider = {
  type = "Provider";
  spec = {
    # spec.artifactId: plain ID into d2b.artifacts (type=provider); no store path in the ResourceSpec
    artifactId = "example-provider";
    # spec.config: exact field names from the Provider artifact's config schema
    config = {
      controller = {
        mainState = {
          quotaBytes = 104857600;
          snapshotPolicy = {
            retainCount = 3;
            retainDurationHours = 168;
            triggerOnMigration = true;
          };
          # Provider schema marks sealingCredentialRef as credentialRef: true
          # Only "Credential/<name>" accepted - raw key values fail eval
          sealingCredentialRef = "Credential/example-provider-state-key";
        };
      };
    };
  };
};
```

### Nix option types and schema origin

Nix option types, defaults, and documentation for every `spec.*` field are generated from two schema sources:

| Field scope | Schema source | Validation level |
| --- | --- | --- |
| `spec.*` core fields (`artifactId` and ResourceType-common fields) | Committed `ResourceTypeSchema` for `Provider` in `d2b-contracts` | Eval-time (Nix type system) + build-time (JSON Schema) |
| `spec.config.*` | Provider artifact's embedded config schema, exported as a Nix module by the Provider package; resolved at eval time via `spec.artifactId` looked up in `d2b.artifacts` | Eval-time (`lib.evalModules` against Provider config schema module) + build-time (JSON Schema from `provider.json`) |

The NixOS module resolves `spec.artifactId` against the `d2b.artifacts` entries at eval time to obtain the Provider package and load its config schema module. Every `spec.config.*` field type, default, and documentation is sourced directly from the Provider artifact's own config schema - no bespoke module code in d2b core is needed for Provider-specific config.

### Eval-time validation

During `nixos-rebuild eval`:

1. **ResourceTypeSchema conformance**: `spec.*` core fields are type-checked against Nix option types generated from the committed `ResourceTypeSchema` for `type`. Unknown `spec` keys fail the eval.
2. **Provider config conformance**: `spec.config.*` fields are validated inside `lib.evalModules` against the Provider artifact's config schema module (resolved via `spec.artifactId` from `d2b.artifacts`). Unknown keys, type mismatches, and out-of-bounds values fail the eval.
3. **Credential-ref guard**: any `spec.*` or `spec.config.*` field the schema marks `credentialRef: true` accepts only strings matching `Credential/[a-z][a-z0-9-]*`. A raw value (anything that does not match that pattern) fails with a typed `credential-value-must-be-ref` error. Secret material never enters the Nix store or the emitted bundle.
4. **Reference bounds**: `*Ref` fields that reference Zone resources by `<ResourceType>/<name>` are validated for syntactic correctness at eval time; referential existence is validated by the Zone runtime at bundle activation time.
5. **Zone-level conflict detection**: the module aggregates all resources of a given `type` in the Zone and enforces constraints published in the ResourceTypeSchema (e.g., uniqueness invariants among Providers in the same component scope).

### Build-phase validation and bundle emission

After eval, the build derivation:

1. Resolves `spec.artifactId` against the `d2b.artifacts` catalog to locate the Provider package; reads the Provider's signed `provider.json` manifest. The artifact catalog (installed root:d2bd 0640) carries the store path and content digests; no store path appears in the emitted ResourceSpec.
2. Validates the fully rendered `spec` object (including all `spec.config` fields) against the full JSON Schema in the manifest, rejecting any field not present in the Provider's `ResourceTypeSchema` or `configSchema`.
3. Emits the canonical sorted Zone resource bundle at `/etc/d2b/zones/<zone>/resource-bundle.json`.

The rendered resource JSON for the example above:

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type": "Provider",
  "metadata": {
    "name": "example-provider",
    "zone": "dev"
  },
  "spec": {
    "artifactId": "example-provider",
    "config": {
      "controller": {
        "mainState": {
          "quotaBytes": 104857600,
          "snapshotPolicy": {
            "retainCount": 3,
            "retainDurationHours": 168,
            "triggerOnMigration": true
          },
          "sealingCredentialRef": "Credential/example-provider-state-key"
        }
      }
    }
  }
}
```

The input bundle contains only `metadata.name`, `metadata.zone`, and `spec`. `metadata.managedBy` and `configurationGeneration` are **not** serialized in the bundle; the configuration service sets them in the resource store when it persists activated resources.

The Nix `spec` fields map to the JSON `spec` object with **no field renaming and no alternative nesting**: what the user writes under `spec` in Nix appears unchanged in the canonical JSON `spec`. Build validation compares this rendered JSON against the committed `ResourceTypeSchema` for `Provider` before writing the bundle.

Bundle envelope (the canonical shape is frozen in
`ADR-046-nix-configuration.md` section "Bundle contract (canonical)" (D119);
this spec consumes that shape and does not redefine it):

```json
{
  "schemaVersion": 3,
  "bundleVersion": 1,
  "zone": "dev",
  "contentHash": "sha256:<hex-of-canonical-sorted-resources-array>",
  "artifactCatalogDigest": "sha256:<hex>",
  "generatedAt": "1970-01-01T00:00:00.000Z",
  "resources": [ /* sorted rendered resource objects */ ],
  "providerSchemaDigests": { "Provider/<name>": "sha256:<hex>" }
}
```

Bundle properties:

| Property | Rules |
| --- | --- |
| `resources` order | Lexicographically sorted by `(type, metadata.zone, metadata.name)` |
| `contentHash` | The D101 `d2b:v3:resource-bundle` digest over the canonical-sorted UTF-8 `resources` array alone (excluding envelope fields); it is the generation identity (`generationId`) and is deterministic across independent builds of the same Nix inputs |
| `generatedAt` | Fixed Unix epoch `1970-01-01T00:00:00.000Z` for reproducibility; the Zone daemon records the real activation time in its own generation record, never in the bundle |
| Resource metadata | Input bundle contains only `metadata.name` and `metadata.zone` per resource; `managedBy`, `configurationGeneration`, `uid`, `generation`, `resourceVersion`, and timestamps are set by the configuration service / core when persisting activated resources, never by the bundle emitter |
| Credential refs | Remain as `"Credential/<name>"` strings in the bundle; raw key material never appears |
| Schema validation | Build fails if any rendered resource spec violates the committed `ResourceTypeSchema` for its type |

The build emitter recomputes `contentHash` over the final sorted `resources` array and writes it into the envelope. The Zone daemon recomputes `contentHash` on load and aborts activation with `bundle-integrity-failure` on mismatch.

## Generation-based cleanup contract

### Configuration-managed vs controller-created resources

The Zone runtime distinguishes two resource ownership classes:

| Class | Source | `metadata.managedBy` (set on activation) | Responsible for deletion |
| --- | --- | --- | --- |
| **Configuration-managed** | Emitted by Nix Zone resource bundle; configuration service sets `managedBy: configuration` when persisting activated resources | `"configuration"` | Zone runtime: resources absent from the new bundle receive an async Delete |
| **Controller-created** | Created by Provider controllers at runtime (migration EphemeralProcesses, staging Volumes, dynamic Guests, worker subview allocations, etc.); controller sets `managedBy: controller` | `"controller"` | Owning Provider controller via reconcile; Zone runtime never deletes them solely because they are absent from the bundle |

The `managedBy` enum also includes `"api"` for resources created directly through the resource API. Generation-diff cleanup targets only `managedBy: configuration` resources; `managedBy: controller` and `managedBy: api` resources are never deleted by the generation-diff path.

The Zone runtime never deletes a `managedBy: controller` resource because it is absent from the incoming bundle. A Provider controller is fully responsible for reconciling and deleting its controller-created children as parent resources change.

### Applying a new generation

When the Zone daemon reads a bundle whose `contentHash` (`generationId`) differs from the active generation:

1. **Integrity check**: recompute SHA-256 of the `resources` array; abort with `bundle-integrity-failure` on mismatch - prior generation is unchanged.
2. **Diff**: compute `(type, name)` diff between the persisted resource store entries where `managedBy: configuration` and the new canonical configured set.
3. **Atomic durable commit (the sole activation point)**: before any intent is queued, the sole durable writer ADR046-routing-013 atomically commits `/var/lib/d2b/zones/<zone>/configuration/generation.json` (new active `contentHash`, prior `contentHash`, `retainedGenerations`, retention-ring metadata) and stages the outgoing bundle into the retention ring, under the bundle-file OFD lock. The new generation is active only when this commit returns; provider-state does not write `generation.json` itself and defers the durable commit to ADR046-routing-013.
4. **Added or changed resources**: create new resources; update spec of existing `managedBy: configuration` resources and set their `configurationGeneration`. If a resource with the same `(type, name)` already exists with `managedBy: controller` or `managedBy: api`, the configuration service records a `configuration-name-conflict` error for that resource, marks that generation activation item `Degraded/name-conflict`, and leaves the existing resource untouched. It never seizes ownership by changing `managedBy`.
5. **Absent resources**: set `deletionRequestedAt` on every persisted `managedBy: configuration` resource absent from the new configured set. Deletion proceeds through owner-child/finalizer-safe ordering; the Zone runtime does not force-delete or skip finalizers.
6. **Unchanged resources**: resources whose spec is byte-identical to the persisted spec have their `configurationGeneration` updated to the current generation; no controller reconcile is triggered.

Steps 4-6 (intent application) run only after the step-3 commit returns.

### Activation does not block on cleanup

New generation activation is complete once the step-3 durable commit returns; it does **not** wait for cleanup completion:

- `Zone.status.observedGeneration` advances to the new bundle generation once the atomic `generation.json` commit returns, not after intent application completes.
- `Zone.status.phase` transitions to `Degraded` with a `pending-cleanup` condition while any prior-generation `managedBy: configuration` resources are still completing deletion.
- `Zone.status.phase` returns to its normal steady state once all pending cleanup completes.
- Providers registered in the new generation start immediately; their controllers may begin creating controller-created children against the new spec without waiting.

### Prior generation retention

The Zone daemon retains prior bundle files at `/var/lib/d2b/zones/<zone>/configuration/prior/<contentHash>.json`. The number of retained prior bundles is configurable per Zone in the range 1..16 (default: 3). A prior bundle cannot be pruned while any of the following hold for resources that were removed in the transition from that generation:

1. Any removed `managedBy: configuration` resource has not yet reached `phase: Deleted`.
2. Any `IncidentHold` condition applies to a removed resource.
3. A pending rollback depends on the prior generation.

There is no time-based TTL for prior generation retention; retention is count-bounded only. The Zone daemon also retains prior bundles in the resource store for rollback. Rollback replays the prior bundle through the same diff/activate path and is subject to the same integrity-check, diff, and cleanup semantics.

### Owner-controller child reconciliation

When a `managedBy: configuration` Provider receives a generation-diff Delete:

1. Zone runtime sets `deletionRequestedAt` on the Provider resource; Provider controller receives a watch event.
2. Controller's reconcile handler cleans up controller-created children in dependency order: stop component Processes → await mount finalizers → delete Volumes → release Credential leases.
3. Only after all resources with `ownerRef: Provider/<name>` reach `phase: Deleted` does the controller remove its own finalizer.
4. Zone runtime removes the Provider from the resource store and excludes it from future bundle diffs.

A Provider controller must complete finalizer cleanup within `maxFinalizerDurationSeconds` declared in its descriptor. Exceeding the bound transitions the Provider to `Degraded/finalizer-timeout` and emits the corresponding audit event; operator intervention resolves.

### Cleanup status, audit, and OTEL

The following events extend the audit event table in [Status, audit, and OTEL](#status-audit-and-otel):

| Event kind | Trigger | Payload |
| --- | --- | --- |
| `zone-generation-activated` | Bundle diff applied | zone, generation, content-id, prior-generation |
| `zone-generation-pending-cleanup` | One or more prior-gen resources still deleting | zone, generation, pending-count |
| `zone-generation-cleanup-complete` | All absent resources deleted | zone, generation |
| `zone-generation-rollback` | Prior bundle replayed | zone, from-generation, to-generation |
| `resource-configuration-delete` | Async delete of a prior-gen `managedBy: configuration` resource | zone, resource-type, resource-name, configuration-generation |
| `configuration-name-conflict` | Bundle `(type, name)` collides with existing `managedBy: controller` or `managedBy: api` resource | zone, resource-type, resource-name, generation |
| `provider-finalizer-timeout` | Controller exceeded `maxFinalizerDurationSeconds` | zone, provider-name, elapsed-seconds |
| `bundle-integrity-failure` | `contentHash` mismatch on bundle load | zone, generation |

The following metrics extend the OTEL metric table in [Status, audit, and OTEL](#status-audit-and-otel):

| Metric | Unit | Labels |
| --- | --- | --- |
| `d2b_zone_generation_activation_total` | Counter | outcome (`applied` \| `integrity-failure`) |
| `d2b_zone_generation_cleanup_pending_resources` | Gauge | (none) |
| `d2b_zone_generation_cleanup_duration_ms` | Histogram | (none) |

### Required tests for removed-resource cleanup

Required by ADR046-pstate-010:

1. **Absent Volume**: new bundle omits a `managedBy: configuration` Volume → Volume receives `deletionRequestedAt`; new generation `status.observedGeneration` advances immediately; `pending-cleanup` condition present; Volume destroyed after mount finalizers clear; `zone-generation-cleanup-complete` audit event.
2. **Absent Provider, children retained**: new bundle omits a `managedBy: configuration` Provider → Provider controller cleans up controller-created children in order; only `managedBy: configuration` Volumes owned by that Provider receive the generation-diff Delete; unrelated controller-created resources in the same Zone are untouched.
3. **Incident hold defers cleanup**: absent `managedBy: configuration` Volume has `IncidentHold` → `deletionRequestedAt` set; `pending-cleanup` condition persists until hold is explicitly cleared; prior generation retained per count-based policy (not prunable while this resource is undeleted).
4. **Bundle integrity failure**: `contentHash` in bundle does not match recomputed hash → activation aborts; prior generation unchanged; `bundle-integrity-failure` audit event; no `status.observedGeneration` advance.
5. **Rollback**: prior generation replayed → diff re-creates absent resources; controller-created children with `ownerRef` pointing to restored Provider are retained without spurious deletion.
6. **Finalizer timeout**: Provider controller does not remove finalizer within `maxFinalizerDurationSeconds` → `Degraded/finalizer-timeout` condition set; `provider-finalizer-timeout` audit event. Timeout is stall detection only; no finalizer is force-cleared; operator intervention resolves the stall.
7. **Eval credential-ref guard**: `config` field with `credentialRef: true` receives a raw string → NixOS eval fails with `credential-value-must-be-ref`; no bundle is emitted; build derivation is never entered.
8. **Name conflict**: new bundle contains a resource `(type, name)` that collides with an existing `managedBy: controller` or `managedBy: api` resource → that activation item is `Degraded/name-conflict`; the existing resource and its `managedBy` are unchanged; a `configuration-name-conflict` audit event is emitted; all non-conflicting resources in the same generation activate normally.
9. **Metric identity absence**: every Volume-state and generation metric
   descriptor rejects `vm`, `zone`, `zone_id`, `zone_uid`, and every
   resource-name-derived label key; a Zone/resource-name canary is absent from
   emitted label values while `d2b.zone` remains in OTEL resource attributes.

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | Baseline v3 (`fd5b0067`): `packages/d2bd/src/supervisor/state.rs` (`RunnerSnapshotRecord`, keyed by `WorkloadId` + `RunnerRole`); `packages/d2b-contracts/src/broker_wire.rs` (`RunnerRole`: CloudHypervisor, Virtiofsd, Swtpm, SwtpmFlush, Gpu, QemuMedia); `packages/d2b-core/src/processes.rs` (`VmProcessDag`, `ProcessNode`, `ProcessRole`: Swtpm, Virtiofsd, StoreVirtiofsPreflight, CloudHypervisorRunner, etc.); `packages/d2b-priv-broker/src/ops/swtpm_dir.rs` (TPM dir/marker, per `WorkloadId` path); `nixos-modules/store.nix` (per-VM hardlink farm, `WorkloadId`-keyed); `packages/d2b-core/src/storage.rs` (`StorageRoot`, `StoragePathSpec`, `StorageRootClass`, `StorageAuthority`), `sync.rs`, `storage_lifecycle.rs`; `packages/d2b-realm-core/src/ids.rs` (`WorkloadId`, `RealmId`, `RealmPath`, `ProviderId`); `packages/d2b-realm-core/src/workload.rs` (`WorkloadProviderKind`: LocalVm, QemuMedia, ProviderManaged, UnsafeLocal; `IsolationPosture`); `packages/d2b-realm-core/src/realm.rs` (`EntrypointMode`: HostResident, GatewayBacked; `RealmControllerPlacement`: GatewayVm, CloudFullHost, ProviderController, etc.); `packages/d2b-realm-core/src/identity_store.rs` (`RealmIdentityStore`, `EnrollmentRecord`, `ControllerGenerationMetadata`); `packages/d2b-realm-provider/src/provider.rs` (`RuntimeProvider`, `WorkloadProvider`, `HostSubstrateProvider`); `packages/d2b-realm-router/src/session_lifecycle.rs` (`SessionPhase`: Allocating → Running → Stopped); `nixos-modules/unsafe-local-helper.nix`; `nixos-modules/gateway-vm.nix`; `packages/d2b-state/` (main branch 6faa5256 only; absent from v3 baseline) |
| Terminology note | Current v3 uses `WorkloadId`/`Workload` (→ target `Guest/<name>` for VM/sandbox/cloud; `WorkloadProviderKind::UnsafeLocal` → user-only Host, NOT Guest), `Realm`/`RealmId`/`RealmPath` (→ target `Zone`), `ProcessRole`/`ProcessNode`/`VmProcessDag` (→ target `Process` or `EphemeralProcess` resource), `RunnerRole` (→ specific Process resource under Guest/device Provider), `EntrypointMode::GatewayBacked`/`RealmControllerPlacement::GatewayVm` (→ `Guest` VM under `Provider/runtime-cloud-hypervisor`), `StorageRoot`/`StoragePathSpec` (→ Volume layout entries), `StorageAuthority::Daemon/Broker/NixModule` (→ Volume `providerRef` and broker-maintained/Nix-managed layout), `WorkloadProviderKind::LocalVm` (→ `Provider/runtime-cloud-hypervisor`), `WorkloadProviderKind::ProviderManaged` (→ `Provider/runtime-azure-container-apps` or similar). Main branch (6faa5256) uses ADR 0046 target names; none of those names exist in the v3 baseline. |
| Evidence class | `d2b-core` `StorageRoot`/`StoragePathSpec`/`StorageRootClass`/`StorageAuthority` (StorageJson): `generated-or-eval-contract`; swtpm tamper-guard marker (`swtpm_dir.rs`, per `WorkloadId`): `implemented-and-reachable`; daemon `RunnerSnapshotRecord` per `WorkloadId`/`RunnerRole`: `implemented-and-reachable`; ACA/Relay `SessionPhase` lifecycle (`d2b-realm-router/src/session_lifecycle.rs`): `implemented-and-reachable` (gateway paths only); `RealmIdentityStore` enrollment/key-rotation metadata: `implemented-and-reachable`; `RuntimeProvider`/`WorkloadProvider` traits: `implemented-and-reachable` (ACA/Relay/gateway), `test-only-or-preview` (host adapters); `VmProcessDag`/`ProcessRole`/`ProcessNode` generated contracts: `generated-or-eval-contract`; Volume `stateSchema` extension, `ProviderStateSet` query concept, migration EphemeralProcess, staging Volume, sealing, snapshots, relocation, incident hold: `ADR-only` |
| Behavior retained | Anchored path resolution (`AnchoredDir`/`AnchoredResource`, main 6faa5256; equivalent `openat2` pattern in v3 swtpm_dir.rs), OFD locks (`LockGuard`/`LockSet`, main), atomic rename/fsync (`AtomicFilesystem`, main), tamper-guard identity markers (v3 swtpm_dir.rs), fail-closed on missing/replaced state (v3 baseline), exact-owner ACL enforcement (`StoragePathSpec.owner/mode`, v3 baseline), no raw-path in public error envelopes (v3 baseline), generation-bound state envelopes (`StateEnvelope`, main), per-`WorkloadId` (→ per-`Guest`) storage scope, `ProcessRole`-keyed (→ component-descriptor-keyed) state namespace, `StorageLifecycle::Persistent`/`BootScopedReadoptable` classes mapped to Volume `persistenceClass` |
| Required delta | `stateSchema` Volume spec extension, `ProviderStateSet` query (owner-index based), broker-maintained marker root outside Volume tree, migration EphemeralProcess lifecycle, cross-component prepare/commit/rollback, staging Volumes, sealing credential integration, incident hold condition, relocation EphemeralProcess, unclaimed GC, OTEL metric set; `RunnerSnapshotRecord` pattern eliminated (replaced by cgroup-leaf scanning in Process Providers) |
| Reuse path | `d2b-state` from main (6faa5256): copy `AtomicFilesystem`, `AnchoredDir`, `AnchoredResource`, `LeafName`, `RelativePath`, `OfdTransfer`/`LockGuard`/`LockSet` into `d2b-provider-volume-local`; adapt `StateEnvelope`/`CanonicalJson`/`QuarantineRecord` in `d2b-contracts`; adapt `AuditAppender` for Zone audit stream; swtpm_dir.rs marker algorithm (v3 baseline) adapted into volume-local Provider marker module |
| Replacement/deletion | `d2b-core/src/storage.rs` (`StorageRoot`/`StoragePathSpec`) StorageJson, `sync.rs` SyncJson, `storage_lifecycle.rs`, and daemon `RunnerSnapshotRecord` (per `WorkloadId`/`RunnerRole`) removed only after owning Provider Volume specs are live; `VmProcessDag`/`ProcessRole` role branches in daemon removed after Process Provider successors pass parity tests; ACA/Relay `WorkloadProvider`/`RuntimeProvider` adapter removed after `Provider/runtime-azure-container-apps` and `Provider/transport-azure-relay` integrations pass; `RealmIdentityStore` in-memory state is superseded by Zone enrollment credential plane (not a Volume; separate scope from Provider payload state); `d2b-state` crate on main remains until v3 callers migrate |
| Feasibility proof | Pre-acceptance spike: provision one host-local Volume with a stateSchema (using adapted swtpm_dir.rs marker pattern), run a migration EphemeralProcess, verify marker identity check, mount a worker-read view, verify domain-isolation rejection |
| Future owner | ADR046-pstate-001 through ADR046-pstate-011 below |

## Provider state migration map

The table below maps each current state root in the v3 tree to its future Volume declaration in the d2b 3.0 Provider model.

**Terminology note.** In the current v3 baseline (`fd5b0067`), VM execution units are `Workload`/`WorkloadId` (`d2b-realm-core/src/ids.rs`) and `d2b.vms.<vm>` Nix options. Current per-VM runner processes are typed by `ProcessRole` (`d2b-core/src/processes.rs`) and `RunnerRole` (`d2b-contracts/src/broker_wire.rs`). In ADR 0046: `WorkloadId`/`Workload` → `Guest/<name>` (for VMs); `Realm`/`RealmId`/`RealmPath` → `Zone`; `ProcessRole`/`RunnerRole` → `Process`/`EphemeralProcess` resources under specific Providers; `WorkloadProviderKind::UnsafeLocal` → user-only Host (not Guest). Paths containing `<vm>` use the current `WorkloadId` label; the same label becomes the `Guest/<name>` ResourceName in v3.

| Current state | Current authority and symbol | v3 persistence | Future Volume owner | Future Volume name pattern | Notes |
| --- | --- | --- | --- | --- | --- |
| `/var/lib/d2b/daemon-state/<vm>/runtime.json` (`RunnerSnapshotRecord`; `<vm>` = `WorkloadId`; role = `RunnerRole` in `d2b-contracts/src/broker_wire.rs`) | Daemon (`d2bd`, `packages/d2bd/src/supervisor/state.rs`) | Superseded | - | - | In v3, Process Providers (system-minijail for `RunnerRole::CloudHypervisor`/`Virtiofsd`/`Swtpm`; system-systemd for `ProcessRole::HostReconcile`) adopt running processes via cgroup-leaf scanning + fresh pidfd; the persisted PID snapshot pattern is eliminated. `WorkloadId` → `Guest/<name>`; each `RunnerRole` variant → a Process resource owned by its Guest/device Provider. No Volume needed. |
| `/var/lib/d2b/vms/<vm>/swtpm/` (TPM NVRAM + EK seed; `<vm>` = `WorkloadId`; launched as `ProcessRole::Swtpm`/`RunnerRole::Swtpm`) | Broker (`d2b-priv-broker/src/ops/swtpm_dir.rs`) | `persistent` | Provider/device-tpm | `device-tpm--nvram--<vm>` (`WorkloadId` → `Guest/<vm>` name) | Broker-maintained identity marker; fail-closed replacement detection; `RunnerRole::Swtpm` → Process resource under Guest owned by device-tpm Provider |
| `/var/lib/d2b/swtpm-markers/<vm>` (TPM tamper-guard marker; `<vm>` = `WorkloadId`) | Broker | `persistent` | Provider/device-tpm | (marker root: broker-maintained, outside Volume root) | Marker root pattern adapts directly to broker-maintained `identityMarker.class: broker-maintained` |
| `/var/lib/d2b/vms/<vm>/store/` (Nix hardlink farm; `<vm>` = `WorkloadId`; served via `ProcessRole::Virtiofsd`/`RunnerRole::Virtiofsd`; preflight at `ProcessRole::StoreVirtiofsPreflight`) | NixOS activation (`nixos-modules/store.nix`) | `persistent` | Provider/runtime-cloud-hypervisor | `runtime-cloud-hypervisor--nix-store--<vm>` (one per Guest; `WorkloadId` → `Guest/<name>`) | `persistenceClass: config`; layout managed by NixOS activation; no `stateSchema` migration; `ProcessRole::Virtiofsd`/`StoreVirtiofsPreflight` → virtiofsd Process and preflight EphemeralProcess under Guest |
| `/var/lib/d2b/vms/<vm>/store-meta/` (generation pins + GC roots; `<vm>` = `WorkloadId`) | NixOS activation (`nixos-modules/store.nix`) | `persistent` | Provider/runtime-cloud-hypervisor | `runtime-cloud-hypervisor--nix-store-meta--<vm>` | `persistenceClass: config` |
| Unsafe-local user session state (helper runtime + shell supervisor; `WorkloadProviderKind::UnsafeLocal` in `d2b-realm-core/src/workload.rs`) | User daemon (`nixos-modules/unsafe-local-helper.nix`) | `ephemeral` user-domain | Provider/system-core (user-only Host, NOT Guest) | `system-core--unsafe-local-session--user-<username>` | User-domain Volume; no cross-uid access; no system-domain mount; maps from `WorkloadProviderKind::UnsafeLocal` → user-only Host ExecutionPolicy (D042) |
| Gateway realm VM config (`nixos-modules/gateway-vm.nix`; current `EntrypointMode::GatewayBacked`, `RealmControllerPlacement::GatewayVm` from `d2b-realm-core/src/realm.rs`; `WorkloadProviderKind::LocalVm`) | NixOS activation | `persistent` | Provider/runtime-cloud-hypervisor (gateway as Guest) | `runtime-cloud-hypervisor--gateway-config--<gateway>` (`WorkloadId` → `Guest/<gateway>`) | `persistenceClass: config`; gateway VM OS/data state only; current `RealmIdentityStore` enrollment records are Zone-level credential/enrollment data (not in this Volume; separate mapping below) |
| `d2b-core/src/storage.rs` `StorageJson`/`SyncJson` generated contract rows (per `ProcessRole`/`StorageRoot`/`StoragePathSpec.scope`) | Broker/daemon | varies by `StorageLifecycle` enum | Respective Provider per row (derived from `StorageAuthority` + `ProcessRole` → Provider owner mapping) | Named per Provider component descriptor `stateNamespace.id` | Full reset; no row-level import; `StoragePathSpec.scope` (current ProcessRole-keyed ID) → component-descriptor `stateNamespace.id`; `StorageRootClass::Persistent` → `persistenceClass: persistent`; `StorageRootClass::Config` → `persistenceClass: config`; `StorageLifecycle::BootScopedReadoptable` → `persistenceClass: ephemeral`; `StorageAuthority::Broker` → broker-maintained marker; `StorageAuthority::NixModule` → Nix-managed layout |
| ACA workload/display session lifecycle (`d2b-realm-router/src/session_lifecycle.rs` `SessionPhase`; `WorkloadProvider`/`RuntimeProvider` in `d2b-realm-provider/src/provider.rs`; live for ACA/Relay gateway paths) | ACA/Relay Provider adapters (`d2b-realm-router`) | In-memory | Not Volume - maps to Process/EphemeralProcess status conditions | - | `SessionPhase` (Allocating → TokenMinting → RelayConnecting → DisplayOpening → Running → Stopping → Stopped) is in-memory session state; no persistent file Volume. Maps to EphemeralProcess status under `Provider/runtime-azure-container-apps` or `Provider/transport-azure-relay`. `WorkloadProviderKind::ProviderManaged` → `Guest` under `Provider/runtime-azure-container-apps`. |
| Realm enrollment/key-rotation metadata (`d2b-realm-core/src/identity_store.rs` `RealmIdentityStore`; `EnrollmentRecord`, `ControllerGenerationMetadata`, `RevocationList`; `WorkloadId` in teardown directives) | Realm controller (in-memory, `d2b-realm-core`) | In-memory | Not Provider payload Volume - maps to Zone enrollment credential resources | - | Pure in-memory enrollment, generation, revocation, and recovery metadata; opaque refs and fingerprints only; no key material. In v3, controller static private key arrives via systemd credential (`d2b-controller-static-v2`); enrollment/recovery journal maps to Zone-level `Credential` resources. `RealmId`/`RealmPath` → `Zone`; `WorkloadId` in teardown → `Guest/<name>`. |
| Provider/workload registry (`d2b-realm-core/src/registry.rs` `ProviderRegistryEntry`, `WorkloadPlacement`; in-memory in realm controller) | Realm controller (in-memory) | In-memory | Not Volume - maps to Zone resource store Provider/Guest resources | - | Provider registry entries and workload placement are in-memory routing metadata. In v3, this becomes `Provider/<name>` resource status and `Guest/<name>` spec with `providerRef` in the Zone resource store. No persistent file Volume. |

All file-backed migrations are destructive v3 resets; no v2 state is imported. Persistent identity data (TPM NVRAM) must be backed up by the operator before the host is reset to v3.

## d2b-state reuse plan

Main-branch `d2b-state` (at commit `6faa5256`) is the primary reuse source for volume-local Provider filesystem primitives. This code does not exist in the v3 baseline. The following symbols are selected for copy/adaptation:

| Main symbol | Main path | Reuse action | v3 destination | Adaptation notes |
| --- | --- | --- | --- | --- |
| `AtomicFilesystem`, `RealAtomicFilesystem`, `AtomicWrite`, `CanonicalJson`, `DurableState`, `QuarantineRecord`, `GenerationPolicy`, `MetadataExpectation`, `ReadPolicy`, `WritePolicy` | `packages/d2b-state/src/atomic.rs` | adapt | `packages/d2b-provider-volume-local/src/atomic.rs` | Replace ADR 0045 `v2_state` contract imports with v3 `StateEnvelope`; retain all filesystem + fsync semantics unchanged |
| `AnchoredDir`, `AnchoredResource`, `LeafName`, `RelativePath` | `packages/d2b-state/src/path.rs` | copy-unchanged | `packages/d2b-provider-volume-local/src/path.rs` | No ADR 0045 contract dependencies; path resolution logic is self-contained |
| `LockGuard`, `LockSet`, `OfdTransfer`, `Cancellation`, `Clock`, `NeverCancelled`, `SystemClock` | `packages/d2b-state/src/lock.rs` | adapt | `packages/d2b-provider-volume-local/src/lock.rs` | Replace `v2_state` LockSpec/ResourceId with v3 typed lock IDs; retain OFD lock/CLOEXEC/fd-transfer semantics |
| `LeaseStatus`, `grant_lease`, `revoke_lease`, `validate_lease` | `packages/d2b-state/src/lease.rs` | adapt | `packages/d2b-contracts/src/v3/state_lease.rs` | Map to v3 Volume Credential rotation protocol; retain expiry/revocation semantics |
| `AuditAppender`, `AuditRecordInput`, `SegmentBuilder`, `checkpoint`, `decide_retention`, `detect_gap`, `read_audit_segment` | `packages/d2b-state/src/audit.rs` | adapt | `packages/d2b-provider-volume-local/src/audit.rs` | Adapt to Zone audit stream interface; retain segment builder, hash chain, and gap detection; replace stream/actor types with v3 equivalents |
| Integration tests (atomic write, OFD lock, quarantine, audit segment, lease) | `packages/d2b-state/tests/state.rs`, `async_state.rs` | adapt | `packages/d2b-provider-volume-local/tests/state.rs` | Port exact test scenarios; replace ADR 0045 contract setup with v3 Volume/StateEnvelope setup; retain all fault-injection and order-of-operations coverage |

The following ADR 0045 assumptions in `d2b-state` are explicitly excluded:

- `d2b-contracts::v2_state` envelope schema and authority refs;
- `AuthorityRef`/`OwnershipEpoch`/`v2_state::ResourceId` semantics;
- ADR 0045 broker-operation IDs embedded in `LockSpec.resource_id`;
- the `tokio_api` feature's dependency on the ADR 0045 broker transport.

## Bus / ComponentSession cross-reference

Provider state controllers and the volume-local Provider implementation depend on the d2b-bus / ComponentSession stack, but that stack's architecture, reuse plan, and implementation work items are owned by `ADR-046-componentsession-and-bus`. This spec does not duplicate or re-specify them.

The following state-relevant symbols from main commit `a1cc0b2d` are called out here because they appear directly in provider-state work items:

- **`Cancellation` / `RequestRegistry`** (`packages/d2b-session/src/cancellation.rs`, main `a1cc0b2d`): copy-unchanged into d2b-bus; used by migration EphemeralProcess coordination (ADR046-pstate-004) and sealing credential operations (ADR046-pstate-005) for atomic shutdown propagation.
- **`OwnedAttachment` / `AttachmentPayload`** (`packages/d2b-session/src/attachment.rs`, main `a1cc0b2d`): copy-unchanged into d2b-bus; volume-local Provider uses file-descriptor attachments to deliver a worker subview dirfd to requesting Processes (see [Worker subviews](#worker-subviews)). The descriptor-payload pairing and `validate_descriptor`-after-decrypt pattern are preserved unchanged.
- **`Fixture`, `FakeProvider`, `DeterministicClock`** (`packages/d2b-provider-toolkit/src/fixture.rs`, main `a1cc0b2d`): adapt into v3 toolkit; used by ADR046-pstate-009 integration tests as the fake Zone runtime without a live daemon. ADR 0045 credential lease formats excluded.
- **`StorageEffectPort` injection pattern** (`packages/d2b-provider-storage-local/src/lib.rs`, `packages/d2bd/src/provider_effects.rs`, main `a1cc0b2d`): adapt into `packages/d2b-provider-volume-local/src/effect_port.rs`; the Provider receives opaque Volume IDs and forwards filesystem operations to an injected async `VolumeEffectPort`; the daemon binds the port via `DaemonEffectAdapters`, never calling the Provider directly for filesystem operations (ADR046-pstate-003). ADR 0045 `v2_provider` operation types and plan/handle/lease protocol excluded.

## Provider crate layout

Every `packages/d2b-provider-<base>-<implementation>/` crate created by this or any other spec must include the following four paths. Absence of any path fails the workspace and package policy check (see ADR046-pstate-011):

| Path | Contents |
| --- | --- |
| `src/` | Implementation modules and binary entry points; colocated `#[cfg(test)]` unit tests for individual functions |
| `tests/` | Hermetic Cargo integration tests: ResourceType conformance tests, controller reconcile logic, fault-injection scenarios that do not require a live daemon (use `Fixture`/`FakeProvider`/`DeterministicClock` from the controller toolkit) |
| `integration/` | Heavier scenario tests requiring a container, real Host/Guest processes, cross-process coordination, or provider-system fixtures; invoked by the existing `make test-integration` / `make heavy-test-integration` orchestration rather than bare `cargo test` |
| `README.md` | Documents: Provider identity and artifact ID; config schema (`spec.config.*` with types and defaults); ResourceTypes owned; controllers/services/workers/binaries and their roles; placement requirements (Host/Guest/user); dependencies and RBAC; security model; state surfaces; telemetry surfaces; build, test, and integration command reference; and future standalone-repo usage guide |

`integration/` tests are not run by `cargo test` alone. Scenarios requiring a live daemon or a real mounted Volume belong in `integration/`; scenarios that can run against a fake Zone runtime belong in `tests/`. Both test trees are required; an empty `integration/` directory is not acceptable - at minimum it must contain a placeholder scenario and a `README.md` noting future scenarios.

## Implementation work items

### ADR046-pstate-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-pstate-001` |
| Dependency/owner | ADR046-primitives-001; v3 contracts owner |
| Current source | `packages/d2b-core/src/storage.rs` (`StoragePathSpec` with `scope: ContractId` currently keyed by `ProcessRole`/Workload; `SensitivityClass`; `StorageLifecycle`; `StorageRootClass`); `packages/d2b-state/src/atomic.rs` (main, 6faa5256; absent from v3 baseline) |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/volume_state.rs` |
| Detailed design | `VolumeStateSchema` struct (`schemaId`, `schemaVersion`, `schemaDigest`, `migrationPolicy`); `PersistenceClass` and `SensitivityClass` enums; `VolumeStateStatus` extension (`stateSchemaPhase`, `installedSchemaVersion`, `markerStatus`, `sealingStatus`, `quotaUsage`, `lastMigrationAt`); `StateEnvelope<T>` (replaces v2 `StateEnvelope`); canonical JSON serde and digest helpers Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Volume spec and status structs embed these types; Provider descriptor component stateNamespace declaration uses the same types |
| Data migration | Full v3 reset; no v2 state schema import |
| Validation | Schema golden vectors; phase/reason round-trip; StateEnvelope digest tests |
| Removal proof | `d2b-core/src/storage.rs` StoragePathSpec/SensitivityClass removed only after all Provider descriptor consumers are on v3 Volume spec |

### ADR046-pstate-002

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-pstate-002` |
| Dependency/owner | ADR046-pstate-001; Provider contracts owner |
| Current source | `packages/d2b-core/src/processes.rs` (`VmProcessDag`, `ProcessNode`, `ProcessRole`: each current `ProcessRole` variant maps to a `Process` or `EphemeralProcess` resource under its owning Provider; `ProcessRole::Swtpm`/`Virtiofsd`/`CloudHypervisorRunner` → Process resources under `Provider/device-tpm`/`Provider/volume-virtiofs`/`Provider/runtime-cloud-hypervisor`); Provider descriptor component model from ADR046-provider-001 |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/provider.rs` (component descriptor `stateNamespaces` field) |
| Detailed design | Add `stateNamespaces: Vec<ComponentStateNamespace>` to the component descriptor (zero or more entries; a component declares an entry only when a payload passes the storage-need test; stateless components declare none); each entry includes `id`, `kind` (always `state`), `schemaId` (non-null), `schemaVersion`, `schemaDigest`, `persistenceClass` (must be `persistent`; `ephemeral`/`cache` rejected), `sensitivityClass`, `migrationPolicy`, `quotaBytes` (nonzero; minimum 4096), `storageNeed` (`secret` \| `large-binary` \| `private-unsafe-for-status` \| `revision-unsuitable`), `sealingRequired`, `placementMode` (`guest-local` or `host-backed-guest` for Guest-targeted; omitted for Host-targeted), `hostCustodyPermitted` (required `true` for `host-backed-guest`; absent/false for `guest-local`), and `views`; there is no empty-payload (`schemaId: null`) namespace |
| Integration | Provider package build emits component descriptors with state namespaces; Provider controller creates Volumes from descriptors at install time |
| Data migration | Full reset |
| Validation | Descriptor schema golden vectors; descriptor-Volume consistency property test; stateless-component-declares-no-namespace round-trip; storage-need justification enforcement (namespace whose payload is derivable from spec/status/core ledger/external observation → `component-state-not-justified`); `kind != state` → `component-kind-invalid`; `persistenceClass: ephemeral` → `component-persistence-class-forbidden`; `quotaBytes: 0` or `1024` → `component-quota-too-small`; base `quota.maxBytes == quotaBytes` and `quota.maxInodes > 0`; Guest-targeted with `placementMode: guest-local` → source.executionRef=Guest; Guest-targeted with `host-backed-guest` + `hostCustodyPermitted: true` → source on Host, Export created; `host-backed-guest` without `hostCustodyPermitted: true` → `placement-host-custody-violation`; credential/audit schema with `host-backed-guest` → `guest-local-required`; `placementMode` change → descriptor version increment enforced |
| Removal proof | Not applicable (new) |

### ADR046-pstate-003

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-pstate-003` |
| Dependency/owner | ADR046-pstate-001; volume-local Provider owner |
| Current source | `packages/d2b-state/src/atomic.rs`, `path.rs`, `lock.rs` (main, 6faa5256); `packages/d2b-priv-broker/src/ops/swtpm_dir.rs` (marker algorithm, v3 baseline) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-volume-local/` (new crate, full scaffold required): `src/{atomic.rs, path.rs, lock.rs, marker.rs, effect_port.rs}`; `tests/volume_local.rs` (marker missing/replaced/mismatch, domain-isolation rejection, quota enforcement); `integration/volume_local.rs` (real Host filesystem provision, broker-maintained marker check, domain-isolation rejection cross-process); `README.md` |
| Crate layout | Creates full `d2b-provider-volume-local/` scaffold. `src/` owns filesystem primitives and effect port; `tests/` owns hermetic fault-injection and conformance tests (no live daemon); `integration/` owns real-Host provision, marker verification, and cross-process domain-isolation scenarios; `README.md` documents volume-local Provider identity, `spec.config.*`, Volume ResourceType ownership, broker placement requirements, OFD-lock security model, state and telemetry surfaces, and build/test/integration commands |
| Detailed design | Anchored Volume root provision, identity marker write/check, quota soft-check on write, domain-isolation validation, fd-relative layout creation/repair/cleanup, broker-maintained marker root protocol; layout `ownerRef`/`groupRef` must reference a Nix-preprovisioned User principal or bounded system pool - Volume admission rejects runtime-created principals; `VolumeEffectPort` returns opaque IDs and named view dirfds only - no raw host path returned by any EffectPort operation; volume-local must support `source.executionRef: Guest/<name>` for `guest-local` placement (controller running inside the Guest): when executing in a Guest domain, volume-local may not create, read, or hold dirfds/paths for Volumes sourced in another domain; `host-backed-guest` placement creates a `virtiofs.d2bus.org.Export` child per attachment entry and validates `hostCustodyPermitted: true` in the signed descriptor Primary reuse disposition: `adapt`. Preserved source-plan detail: copy-unchanged (path.rs) / adapt (atomic.rs, lock.rs) / adapt (swtpm_dir.rs marker algorithm). |
| Integration | `d2b-priv-broker` calls `volume_local::marker::provision_marker` at broker-maintained Volume creation; `d2b-provider-volume-local` controller calls `marker::verify_marker` on every daemon restart via reconcile startup relist |
| Data migration | New marker written for each Volume at v3 first-boot; TPM marker path adapted from current swtpm-markers root |
| Validation | Marker missing/replaced/mismatch tests; domain-isolation rejection tests; quota enforcement tests; crash at every provision step |
| Removal proof | `swtpm_dir.rs` marker implementation retired only after device-tpm Provider Volume is live and marker-check parity is confirmed |

### ADR046-pstate-004

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-pstate-004` |
| Dependency/owner | ADR046-reconcile-001, ADR046-pstate-003; volume-local Provider and controller-toolkit owners |
| Current source | `packages/d2b-core/src/storage_lifecycle.rs` (`StorageLifecycleReport` issue detection); `packages/d2b-state/src/atomic.rs` (main, 6faa5256) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-volume-local/src/migration.rs`; `packages/d2b-provider-volume-local/tests/migration_unit.rs` (hermetic staging Volume and prepare/commit/rollback); `packages/d2b-provider-volume-local/integration/migration.rs` (real Host crash-injection at each migration step, N-Volume cross-component coordination); `packages/d2b-controller-toolkit/src/state_migration.rs` |
| Crate layout | See [Provider crate layout](#provider-crate-layout). Migration crash-injection tests (OS-level kill between rename steps) and staged filesystem verification require real Host processes → `integration/migration.rs`; pure protocol and EphemeralProcess dispatch logic → `tests/migration_unit.rs` |
| Detailed design | Pre-launch migration EphemeralProcess template, staging Volume create/destroy lifecycle, prepare/commit/rollback protocol implementation, roll-forward on restart detection, migration idempotency |
| Integration | Provider controller's reconcile handler calls toolkit `state_migration::plan` and dispatches EphemeralProcess via ResourceClient; volume-local Provider reports `stateSchemaPhase` transitions |
| Data migration | None (new protocol) |
| Validation | Migration with and without crash at each step; rollback after failed EphemeralProcess; roll-forward after interrupted commit; cross-component N-Volume coordination |
| Removal proof | `StorageLifecycleReport` and storage contract validation in `d2b-core` removed only after v3 Volume migration path is live |

### ADR046-pstate-005

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-pstate-005` |
| Dependency/owner | ADR046-pstate-003; volume-local Provider and Credential Provider owners |
| Current source | `packages/d2b-state/src/lease.rs` (main, 6faa5256); Credential ResourceType from ADR046-primitives-001 |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-volume-local/src/sealing.rs`; `packages/d2b-provider-volume-local/tests/sealing_unit.rs` (hermetic seal/rotation state machine); `packages/d2b-provider-volume-local/integration/sealing.rs` (real Credential lease flow, interrupted key rotation with live Credential Provider) |
| Crate layout | See [Provider crate layout](#provider-crate-layout). Credential lease acquisition and re-encryption under rotation require a live Credential Provider process → `integration/sealing.rs`; pure sealing state-machine logic → `tests/sealing_unit.rs` |
| Detailed design | Envelope encryption on write using Credential lease key material; key rotation EphemeralProcess re-encrypt; no raw key on disk; `sealingStatus` transitions; `rotation-failed` fail-safe |
| Integration | Volume controller reads Credential status/lease before provisioning; sealing wraps `StateEnvelope` writes in atomic.rs |
| Data migration | Sealed Volumes are new (no existing sealing to migrate) |
| Validation | Seal/read/rotation tests; rotation interrupted at commit; credential revoked during runtime |
| Removal proof | Not applicable (new) |

### ADR046-pstate-006

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-pstate-006` |
| Dependency/owner | ADR046-pstate-003, ADR046-pstate-004; volume-local Provider and snapshot toolkit owners |
| Current source | `packages/d2b-state/src/atomic.rs` (main, 6faa5256): `AtomicFilesystem` read snapshot; no existing snapshot infrastructure in v3 |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-volume-local/src/snapshot.rs`; `packages/d2b-provider-volume-local/tests/snapshot_unit.rs` (hermetic `snapshotPolicy` enforcement, retention count and TTL logic); `packages/d2b-provider-volume-local/integration/snapshot.rs` (real Host filesystem snapshot creation, retention expiry, pre-migration auto-snapshot with interrupted migration) |
| Crate layout | See [Provider crate layout](#provider-crate-layout). Real-filesystem snapshot byte-equality verification and pre-migration crash recovery require real Host processes → `integration/snapshot.rs`; policy logic and EphemeralProcess lifecycle → `tests/snapshot_unit.rs` |
| Detailed design | Snapshot EphemeralProcess; bounded `.snapshots/` sub-tree; `snapshotPolicy` enforcement; retention count and TTL cleanup; snapshot status tracking in Volume status |
| Integration | Provider controller creates snapshot EphemeralProcess before migration and relocation based on `snapshotPolicy.triggerOnMigration` and `triggerOnRelocation`; status populated via Volume status update |
| Data migration | No existing snapshots; new infrastructure only |
| Validation | Create/read/expire/retention-limit tests; snapshot before interrupted migration; snapshot list in Volume status |
| Removal proof | Not applicable (new) |

### ADR046-pstate-007

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-pstate-007` |
| Dependency/owner | ADR046-pstate-003; volume-local Provider owner |
| Current source | `packages/d2b-priv-broker/src/ops/swtpm_dir.rs` (v3, marker pattern); `packages/d2b-state/src/path.rs` (main) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-volume-local/src/relocation.rs`; `packages/d2b-provider-volume-local/tests/relocation_unit.rs` (hermetic finalizer set/clear, commit/failure state machine); `packages/d2b-provider-volume-local/integration/relocation.rs` (real Host-to-Host anchored file copy, crash at copy midpoint, virtiofs source re-point after successful relocation) |
| Crate layout | See [Provider crate layout](#provider-crate-layout). Crash-at-midpoint relocation and virtiofs source re-point require real Host/Guest processes → `integration/relocation.rs`; finalizer and commit protocol logic → `tests/relocation_unit.rs` |
| Detailed design | Relocation EphemeralProcess; source finalizer; anchored copy; source Volume (volume-local) relocation for components with Guest attachment (the attachment Volume backed by volume-virtiofs is re-pointed to the new source after copy; see `ADR-046-primitive-resource-composition` Volume attachment spec); commit/failure handling |
| Integration | Provider controller adds `Relocating` finalizer before creating relocation EphemeralProcess; removes finalizer after successful destination Volume activation |
| Data migration | Not applicable |
| Validation | Relocation with crash at copy midpoint; failed relocation source preservation; virtiofs source relocation test |
| Removal proof | Not applicable (new) |

### ADR046-pstate-008

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-pstate-008` |
| Dependency/owner | ADR046-pstate-001; Zone audit stream, OTEL provider owners |
| Current source | `packages/d2b-state/src/audit.rs` (main, 6faa5256): `AuditAppender`, `AuditRecordInput`, `SegmentBuilder`, `checkpoint`, `decide_retention`, `detect_gap`, `read_audit_segment`; Zone audit stream interface from ADR046-bus contracts (see ADR-046-componentsession-and-bus); OTEL cardinality model from `packages/d2b-provider-observability-local/src/` (main, a1cc0b2d) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-volume-local/src/audit.rs`; `packages/d2b-provider-volume-local/src/otel.rs`; `packages/d2b-provider-volume-local/tests/audit_unit.rs` (hermetic audit golden records, OTEL cardinality label tests) |
| Crate layout | See [Provider crate layout](#provider-crate-layout). Hermetic golden-record and cardinality tests → `tests/audit_unit.rs`; live Zone audit stream emission and OTEL export against a running observability Provider → `integration/audit.rs` (added by ADR046-pstate-009) |
| Detailed design | Volume-state audit event types and Zone audit emission; OTEL metric definitions with closed semantic label sets and no Zone or resource-name-derived label keys; Zone identity remains in the `d2b.zone` OTEL resource attribute |
| Integration | Every state lifecycle transition calls `audit::emit_volume_event`; OTEL metrics exported via `observability-otel` Provider |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | Audit event golden records; no content/path/credential in audit payload; structural OTEL label-policy tests assert exact absence of `vm`, `zone`, `zone_id`, `zone_uid`, and resource-name-derived keys and preserve `d2b.zone` as a resource attribute |
| Removal proof | Not applicable |

### ADR046-pstate-009

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-pstate-009` |
| Dependency/owner | ADR046-pstate-001 through ADR046-pstate-008; integration test owner |
| Current source | `packages/d2b-state/tests/state.rs`, `async_state.rs` (main, 6faa5256): atomic, lock, quarantine, audit, lease tests; `packages/d2b-provider-toolkit/src/fixture.rs` (main, a1cc0b2d): `Fixture`, `FakeProvider`, `DeterministicClock`, `ProviderValues`, `Redacted`, `Secret`, `sample_lease_request` |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-volume-local/tests/state.rs` (ported hermetic atomic/lock/quarantine/lease tests); `packages/d2b-provider-volume-local/tests/migration.rs` (ported migration fault-injection, cross-component N-Volume coordination); `packages/d2b-provider-volume-local/integration/provider_state.rs` (end-to-end: live daemon, real Host Volume mount, cross-process worker subview, full audit stream); `packages/d2b-provider-volume-local/integration/audit.rs` (live Zone audit stream emission and OTEL export) |
| Crate layout | See [Provider crate layout](#provider-crate-layout). This work item populates both `tests/` (hermetic ported d2b-state tests using `FakeProvider`/`DeterministicClock`) and `integration/` (end-to-end provider-system scenarios requiring a live daemon + real Volume mount); must include a populated `integration/README.md` describing scenario prerequisites |
| Detailed design | Port all d2b-state integration tests replacing ADR 0045 contract setup with v3 Volume/StateEnvelope; add provider-state-specific migration, marker, quota, sealing, relocation, snapshot, incident-hold, and unclaimed-GC tests; include cross-component N-Volume coordination test |
| Integration | Tests run against the real volume-local Provider over a fake Zone runtime (no live daemon required) using the standard controller-toolkit fake clients |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | All ported tests pass under v3 contracts; test coverage includes every fault-injection scenario listed in d2b-state/tests/state.rs plus new provider-state cases; stateless-component-declares-no-Volume test passes; shared-Volume attempt rejected; `guest-local` Volume creation inside Guest domain (source.executionRef=Guest, no Export created, Host volume-local holds no dirfd/path); `host-backed-guest` Volume creation (source on Host, Export created, Export reaches Ready, Guest Process mounts source Volume view); `host-backed-guest` without `hostCustodyPermitted: true` → `placement-host-custody-violation`; credential/audit schema with `host-backed-guest` → `guest-local-required`; cross-domain isolation: Guest-local volume-local does not create or observe Host-domain Volumes |
| Removal proof | `d2b-state` crate retired from workspace only after every caller migrates to v3 Volume state helpers and all ported tests pass |

### ADR046-pstate-010

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-pstate-010` |
| Dependency/owner | ADR046-pstate-001; Zone resource bundle owner, NixOS module owner, `d2b-core-controller` owner (ADR-046-core-controllers) |
| Current source | `nixos-modules/manifest.nix` (current `manifest.json` emitter, v3 baseline `fd5b0067`); `packages/d2b-core/src/storage.rs` (`StorageAuthority::NixModule`-owned rows); `packages/xtask/src/main.rs` (`gen-schemas` command, same baseline) |
| Reuse action | adapt |
| Destination | `nixos-modules/zone-resources.nix` (per-Zone bundle emitter NixOS module); `packages/d2b-core/src/v3/zone_bundle.rs` (shared bundle DTOs: `ZoneResourceBundle`, `BundleResource`, `contentHash` computation); `packages/d2b-core-controller/src/configuration.rs` (diff/apply loop, name-conflict detection, `pending-cleanup` Zone status, `maxFinalizerDurationSeconds` stall detection - NOT in `d2b-provider-volume-local`); `packages/d2b-core-controller/tests/configuration.rs` (hermetic bundle diff, absent-resource Delete dispatch, name-conflict `Degraded/name-conflict` item, integrity-failure abort); `packages/d2b-core-controller/integration/configuration.rs` (container-based generation activation with running Providers, absent-resource cleanup, rollback, finalizer-timeout stall detection) |
| Crate layout | Configuration-publication diff/apply handler and its tests belong in `packages/d2b-core-controller` (ADR-046-core-controllers), not in `d2b-provider-volume-local` and not in the pre-ADR45 `d2bd` binary. `d2b-provider-volume-local` owns only Volume-behavior code (provision, marker, migration, sealing, snapshot, relocation, unclaimed GC). See [Provider crate layout](#provider-crate-layout) for Volume-behavior crate requirements. |
| Detailed design | Generic `d2b.zones.<zone>.resources.<name> = { type = "…"; spec = { …exact ResourceTypeSpec fields… }; }` attrset; `metadata.name` derived from attr key, `metadata.zone` from Zone key, `apiVersion` defaulted; `status` omitted (read-only); Nix option types for `spec.*` generated from committed `ResourceTypeSchema` for each `type`; Nix option types for `spec.config.*` generated from the Provider artifact's config schema module (resolved at eval time via `spec.artifactId` from `d2b.artifacts`); credential-ref guard (`credentialRef: true` schema fields accept only `Credential/[a-z][a-z0-9-]*`); build-phase full JSON Schema validation of rendered `spec` against Provider manifest; canonical sorted bundle emission with `contentHash` (SHA-256 of sorted `resources` array); configuration service sets `metadata.managedBy: configuration` and `configurationGeneration` in the resource store when persisting activated bundle resources (not in the bundle input; user authors only `type` + `spec`); diff compares new configured set against persisted resource store entries where `managedBy: configuration` (not against the prior bundle file); name-conflict detection: `(type, name)` collision with existing `managedBy: controller` or `managedBy: api` resource → `Degraded/name-conflict` activation item, existing resource untouched, `managedBy` never seized; unchanged-spec resources receive updated `configurationGeneration` with no controller reconcile triggered; absent-resource async Delete with owner-child/finalizer ordering; `Degraded/pending-cleanup` Zone status condition; per-Zone prior bundle count retention (range 1..16, default 3; no TTL); `maxFinalizerDurationSeconds` stall detection and `Degraded/finalizer-timeout` condition (no force-clear) |
| Integration | NixOS build emits `/etc/d2b/zones/<zone>/resource-bundle.json`; the Zone daemon (`d2bd`) watches that path in-process (inotify with a bounded polling fallback) and is also signalled through the activation protocol - `d2b-activation-helper` stages and installs the bundle, commits `generation.json` per D122/ADR046-routing-013, then notifies the daemon (SIGHUP or the activation notify socket). No systemd path unit is introduced, so the framework's three-root-visible-unit contract holds; reconcile loop runs the generation diff on change; Provider controller receives the `deletionRequestedAt` watch event when a configuration-owned Provider is absent from the new bundle |
| Data migration | `nixos-modules/manifest.nix` provider-registration and storage-authority rows superseded by `zone-resources.nix`; prior `manifest.json` format retired after Zone daemon migration to bundle format |
| Validation | All eight removed-resource cleanup tests enumerated in [Required tests for removed-resource cleanup](#required-tests-for-removed-resource-cleanup); eval credential-ref guard (test: raw value in `credentialRef` field → NixOS eval error, no bundle emitted); Provider schema conformance golden vector (test: unknown `config` key → build fails); `contentHash` determinism (test: two independent builds of identical Nix inputs produce byte-identical bundles) |
| Removal proof | `nixos-modules/manifest.nix` provider-registration rows retired only after all Provider registrations use the bundle format and all consumers (broker, Zone daemon) complete bundle-format migration |

### ADR046-pstate-011

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-pstate-011` |
| Dependency/owner | ADR046-pstate-003; workspace policy and xtask owners |
| Current source | `packages/xtask/src/main.rs` (`gen-schemas` and workspace-policy checks, v3 baseline `fd5b0067`); `tests/unit/gates/drift-check.sh` (schema drift gate, same baseline); `packages/d2b-contract-tests/` (contract-test policy, frozen per AGENTS.md) |
| Reuse action | adapt |
| Destination | `packages/xtask/src/provider_crate_policy.rs`; `tests/unit/gates/provider-crate-layout-check.sh` |
| Crate layout | Not applicable (this work item implements the policy gate itself) |
| Detailed design | `cargo xtask check-provider-crate-layout` command: walks every workspace member matching `d2b-provider-*`; for each, asserts presence of `src/`, `tests/`, `integration/`, and `README.md`; asserts `integration/` contains at least one `.rs` file and a `README.md`; fails closed with a typed `missing-provider-crate-path` error listing every absent path; wired into `make test-policy` via `tests/unit/gates/provider-crate-layout-check.sh`; output is machine-readable JSON (`{ "crate": "…", "missing": ["integration/"] }` per violation) Primary reuse disposition: `adapt`. Preserved source-plan detail: extend. |
| Integration | `make test-policy` runs `cargo xtask check-provider-crate-layout`; GitHub CI runs `make test-policy` on every PR; `make check` includes `test-policy` as a required Layer-1 shard; workspace policy tests in `packages/d2b-contract-tests/` are extended with a static manifest check that asserts `provider_crate_layout` policy version is current |
| Data migration | Not applicable |
| Validation | Policy gate detects missing `src/` → error; missing `tests/` → error; missing `integration/` → error; missing `README.md` → error; empty `integration/` (no `.rs` files) → error; all four paths present and non-empty → pass; existing non-provider `d2b-*` crates not flagged; gate is idempotent across re-runs |
| Removal proof | Not applicable (permanent gate) |

### ADR046-pstate-012

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-pstate-012` |
| Dependency/owner | ADR046-pstate-001, ADR046-pstate-002; Zone runtime owner (`d2b-core-controller`), volume-local Provider owner |
| Current source | `packages/d2b-core/src/status.rs` (v3 common status/observedGeneration/conditions); Provider descriptor state-namespace declaration from ADR046-pstate-002 |
| Reuse action | adapt |
| Destination | `packages/d2b-core-controller/src/optional_state_admission.rs` (storage-need admission: reject a declared namespace whose payload is derivable from spec/status/core ledger/external observation with `component-state-not-justified`; only declared namespaces produce a Volume; stateless components produce none); `packages/d2b-core-controller/tests/optional_state_admission.rs` (hermetic: stateless component → no Volume; declared `storageNeed` variants accepted; unjustified namespace rejected; status-first restart revalidation - controller re-derives observed state from status/core ledger/external observation after restart and never treats a status field as authority); `packages/d2b-provider-volume-local/tests/status_bounds.rs` (hermetic: total canonical serialized status cap and provider-specific detail cap enforced; oversize status write → typed rejection; status carries no secret/path/argv/PID/unit/stream/ring content) |
| Crate layout | See [Provider crate layout](#provider-crate-layout). Hermetic admission and status-bound round-trips → `tests/`; Zone-startup restart revalidation requiring real processes → `integration/`. |
| Detailed design | Optional state-Volume admission is a fixed step in Core ProviderDeployment: for each component, if it declares no `stateNamespaces` entry it gets no Volume; for each declared entry, verify the `storageNeed` justification (`secret` \| `large-binary` \| `private-unsafe-for-status` \| `revision-unsuitable`) and reject a namespace whose payload is fully derivable from spec/status/core ledger/external observation with `component-state-not-justified`. Fixed bootstrap components (`system-core`, `system-minijail`, first `volume-local` instance) declare no state Volume and reach Ready using resource `status`, the core Operation ledger, and external observation only; there is no bootstrap-storage mechanism. Status-bound enforcement: reject a status write whose total canonical serialized size exceeds the single canonical status cap, or whose provider-specific detail exceeds the detail cap, or whose condition/count/list/map entries exceed the bounded limits, with the typed status-oversize rejection; status writes occur only on material change. |
| Integration | `d2b-core-controller` runs optional state-Volume admission before creating any declared Volume and before launching a component Process; the status-bound check is applied on every status subresource write in the resource store. |
| Data migration | New; no prior bootstrap artifacts to migrate |
| Validation | Stateless component → no Volume created; each `storageNeed` variant accepted with a declared Volume; unjustified namespace → `component-state-not-justified`; status-first restart: controller re-derives observed state and reverifies against external reality, never treating status as authority; oversize/over-detail/over-cardinality status write → typed rejection; status contains no secret/path/argv/PID/unit/stream/ring content |
| Removal proof | Not applicable (permanent admission + status-bound enforcement) |
