# ADR 0046 Provider/volume-local dossier

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-volume-local` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 3 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-volume-local` crate |
| Depends on | `ADR-046-resources-volume`, `ADR-046-provider-state`, `ADR-046-components-processes-and-sandbox`, `ADR-046-componentsession-and-bus`, `ADR-046-telemetry-audit-and-support`, `ADR-046-nix-configuration`, `ADR-046-resource-api-and-authorization`, `ADR-046-primitive-resource-composition` |
| Supersedes | Version 2 effect, audit, and telemetry authority where corrected below; `d2b-priv-broker/src/ops/{state_dir,store_sync,store_sync_audit,store_sync_export,store_view_farm,store_view_posture,swtpm_dir}.rs`; `d2b-core/src/{storage,sync,storage_lifecycle}.rs` StorageJson/SyncJson contract; `nixos-modules/store.nix` per-VM hardlink farm activation |

## Prospective Wave 6 authority correction

Version 3 is prospective Wave 6 authority and consumes Version 3 of
`ADR-046-telemetry-audit-and-support`. The following rules supersede every
later Version 2 example where they conflict:

1. `VolumeEffectPort` is a pure, total mapping from one closed Provider request
   type to one closed `VolumeBrokerOperation` variant and its typed result. The
   adapter may validate bounds, attach the committed operation proof, dispatch,
   and correlate an out-of-band FD transfer. It does not resolve a path or
   numeric identity, retain a trusted FD table, call a filesystem or key
   syscall, execute a command, choose audit fields, or append audit.
2. The privileged broker resolves opaque Volume, layout-entry, user, source
   policy, view, and sealing-policy IDs from its private bundle authority. Every
   host-authority `openat2`, `fstatat`, ACL, mount, unmount, `fallocate`,
   `linkat`, `renameat`, `unlinkat`, `read`, `write`, `fsync`, `statfs`, and key
   effect occurs inside the handler for a closed typed broker operation. There
   is no generic filesystem, command, path, ACL-string, mount-option-string, or
   key-handle operation. Adding a syscall effect requires adding a broker enum
   variant, bounded request/result DTOs, policy, audit class, and negative
   tests first.
3. Every privileged Volume effect durably commits immutable intent before
   effect release and durably records completion exactly once before returning
   success. The broker derives `PrivilegedEffectAuditDigest` from the Zone
   partition, `OperationCorrelationDigest`, effect ordinal, and closed
   `VolumeEffectClass`. If completion cannot become durable after a mutation,
   the broker returns `CommitPendingAudit`; the controller retries the
   byte-identical request and does not report success. Audit is never emitted
   by the Provider, rate-limited, lossy, informational, or best-effort.
4. The Zone selects the private audit partition and is not serialized. A
   required Volume join uses only `ResourceCorrelationDigest`; actor,
   execution, and operation joins use only `SubjectCorrelationDigest`,
   `ExecutionCorrelationDigest`, and `OperationCorrelationDigest`
   respectively. Audit payload values otherwise use closed enums and bounded
   non-identity scalars. Raw Zone, Volume, resource, user, Process, policy,
   snapshot, operation, or execution identity; `ResourceRef`; UID/GID; handle;
   and a general or Provider-local digest are forbidden.
5. Telemetry selects exact rows from the single
   `d2b-contracts::METRIC_DESCRIPTOR_REGISTRY`. Metrics and OTEL Resource
   attributes contain neither raw identity nor correlation digests; label
   values are closed enums from the selected row. A matching operation span
   may carry `OperationCorrelationDigest`, and span linkage may carry only
   `TraceCorrelationDigest` and `SpanCorrelationDigest`. The Provider owns no
   descriptor, label domain, bucket list, audit writer, or durability choice.
6. T608 owns only the feature-local storage and Host-global authority
   foundation assigned by the W6 local task contract: the shared Volume
   contract/Nix surfaces, the typed broker-effect foundation, and the serial
   foundation edits under the volume-local crate. It adopts the six retained
   W5 obligations named by that contract without changing their historical
   rows. It does not own or satisfy a manifest-backed `ADR046-vl-*` work item.
   After T608 is `Merged` with its required evidence, its files are handed off
   serially to the `wi:ADR-046-provider-volume-local` group. `ADR046-vl-001`
   through `ADR046-vl-013` remain the generated implementation identities and
   own the dossier deliverables below. T609 separately supplies the
   authoritative audit types/writer and central telemetry registry before
   `ADR046-vl-009` consumes them. No T608/T609 checkbox substitutes for a
   dossier work-item state or evidence row.

Current committed code is canon for its present reachability: the partial
volume-local crate still exposes `VolumeStateEffectPort`, carries raw
`ZonePath`/`ResourceRef` in `VolumeAuditEvent`, and defines Provider-local
metric descriptors. Those are recorded implementation gaps, not evidence that
this prospective correction has landed. W6 removes them only through the
manifest-backed dossier tasks and their named validation.

---

## Purpose

This dossier exhaustively specifies the `Provider/volume-local` controller: the
only Volume provider that owns source-side storage on the Host filesystem.
It covers source kinds and their source-config schema; LayoutEntry semantics
including every policy, invariant, ACL, and repair rule; quota enforcement; the
identity-marker contract; the same-filesystem hardlink farm (store-view); the
TPM Volume; named views and rights; Process volume mounts; broker/effect ops and
anchored-path security; the controller process and its EphemeralProcess workers;
schema migration (pre-launch and online); cross-component migration coordination;
secret sealing; snapshots; staging Volumes; relocation; retention, incident hold,
unclaimed GC, and destruction; within-Volume transactions; async reconciliation
with a blocking-thread adapter; d2b-bus/ComponentSession integration; RBAC;
status/conditions/error catalog; audit events and redaction; OTEL metrics; Nix
configuration including artifact catalog and Provider resource authoring; the
`d2b-state` main-branch reuse plan; current-code fit and migration map; the exact
`src/`/`tests/`/`integration/`/`README.md` crate layout; and implementation work
items with removal proofs.

`Provider/volume-virtiofs` owns the virtiofsd Process lifecycle and Guest-side
attachment transport. This dossier does not re-specify volume-virtiofs; it
documents the interface boundary where volume-local hands a validated Volume root
FD to volume-virtiofs for export.

---

## Terminology

Throughout this dossier, baseline symbols are cited with their current names.
The v3 target name appears in parentheses or an explicit mapping.

| Baseline name | v3 target name |
| --- | --- |
| `Realm` / `RealmId` / `RealmPath` | `Zone` / `ZoneId` |
| `WorkloadId` / `d2b.vms.<vm>` | `Guest/<name>` Resource |
| `ProcessRole` / `VmProcessDag` | `Process` / `EphemeralProcess` Resource |
| `StorageRoot` / `StoragePathSpec` | Volume LayoutEntry |
| `StorageAuthority::Broker` | `identityMarker.class: broker-maintained` |
| `StorageAuthority::NixModule` | Nix-managed Volume layout |
| `StorageLifecycle::Persistent` | `persistenceClass: persistent` |
| `StorageLifecycle::BootScopedReadoptable` | `persistenceClass: ephemeral` |
| `StorageRootClass::Config` | `persistenceClass: config` |
| `d2b-priv-broker` | Zone broker |
| `d2bd` | Zone runtime controller |

---

## Provider identity

| Field | Value |
| --- | --- |
| ResourceType | `Provider` |
| Name | `volume-local` (artifact ID `volume-local-provider`) |
| Crate | `packages/d2b-provider-volume-local/` |
| Reconciled ResourceType | `Volume` - exported as volume-local's primary ResourceType; reconciles physical state (layout, ACL, quota, identity marker) for all assigned Volumes; operators create/delete ordinary Volumes via Resource API; **core ProviderDeployment** creates/deletes component state Volumes before/after component Processes; volume-local never issues Volume create/delete API calls; component Processes consume their required view only |
| Source kinds | `local-path`, `block-image`, `tmpfs` |
| Controller component | `volume-local-controller`; `Process` under `Host/host-system`, `domain: system`; `controllerExecutionRef: Host/host-system` |
| Effect operations | Pure `VolumeEffectPort` mappings to the closed `VolumeBrokerOperation` catalogue: `ProvisionLayoutEntry`, `RepairLayoutEntry`, `CleanupLayoutEntry`, `PrepareSwtpmDir`, `CheckQuotaCapacity`, `PollQuotaUsage`, `StoreSyncComplete`, `MountTmpfs`, `UnmountTmpfs`, `ProvisionBlockImage`, `CommitVolumeTransaction`, `CreateVolumeSnapshot`, `ExpireVolumeSnapshot`, `RelocateVolumeContents`, `OpenVolumeMountToken`, `WriteVolumeMarker`, `VerifyVolumeMarker`, `CleanupVolumeMarker`, `CleanupVolumeRoot`, and `RotateSealingKey`; no direct broker connection or host syscall in Provider code |
| Permissions | No special host-path permission; all host path and numeric-identity resolution in closed broker handlers; the Provider has no direct broker connection |
| ProviderStateSet | Optional query-time logical grouping (not a ResourceType): `{ v : Volume \| ownerRef == "Provider/volume-local" }`; **empty** - volume-local declares no state Volume of its own (its bounded non-secret operational state lives in `status`/the core Operation ledger, D087). Volume-local is the **sole reconciler** for all assigned Volumes carrying `providerRef: Provider/volume-local` (operator-created Volumes and other Providers' *declared* state Volumes; Volume is its exported type) and never issues Volume create/delete API calls; **core ProviderDeployment** creates/deletes other Providers' declared state Volume instances before/after their component Processes; a declared component state Volume is created only when its payload passes the storage-need test; Nix-preprovisioned `User/<name>` layout principals; no cross-component sharing; no empty identity-only Volume |
| Finalizers | `volume-local/layout` |
| Supported Host variants | Local NixOS Host; bare-metal Host; ACA if backing filesystem is accessible |
| Guest capability | Not applicable - volume-local does not attach to Guests |
| Main reuse | `d2b-state` at commit `6faa5256` (copy/adapt); `d2b-priv-broker/src/ops/swtpm_dir.rs` (adapt marker algorithm) |

**D089 desired-spec shape.** `Provider/volume-local` owns the `Volume`
ResourceType base spec: fields such as `spec.providerRef`,
`spec.source.settings.kind`, and `spec.source.settings.sourcePolicyId` are base
Volume fields, not Provider extensions. It carries no optional
`spec.provider` payload today. If a future implementation-only desired setting
is required, it must use the canonical `spec.provider = { schemaId,
schemaVersion, settings }` envelope, registered/signed in the Provider
manifest, deny-unknown, bounded, versioned/digested, validated against
`spec.providerRef` at Nix build and API admission, and forbidden to shadow base
fields. Shared fields are promoted to the Volume base. The Provider implements
the exact base spec/status schema version/fingerprint, accepts the canonical
minimal base Spec, passes base conformance, and rejects an
unsupported optional base capability only via its signed capability matrix plus
provider-neutral `unsupported-capability`.
`spec.provider` aligns with `status.provider`. The `Provider` resource itself
keeps the D075 `spec.{artifactId, config}` exception.

### Endpoint resources (D092)

`Volume.source.settings`, layout entries, named views, and mount-token delivery
remain base Volume semantics, not endpoint declarations. `Provider/volume-local`
currently declares no independently consumed stable service portal; if a future
stable portal is visible, it MUST be an owned standard `Endpoint` resource with
`spec.producerRef: Process/<name>`, a `d2bus.org` purpose, and consumers using
`Endpoint/<name>`. `Endpoint.spec` and `Endpoint.status` MUST contain no raw host
path, address, port, FD number, or credential; authorized resolution happens only
through EffectPort/LaunchTicket, unauthorized callers receive
`endpoint-resolve-denied`, and producer restart bumps `endpointGeneration` so
consumers observe `dependency-changed`.

### Retained opaque handles (D092)

`VolumeMountToken`, per-session named streams, `OwnedTransport` byte-stream
handles, transport connection handles, pidfds, FD indexes, layout operation
handles, and `operationId` values remain controller-internal or high-churn
opaque handles. They are not promoted to `Endpoint` resources.

---

## Volume source kinds

### `local-path`

Backed by a Host directory resolved from an opaque `sourcePolicyId`. The
`sourcePolicyId` references a source policy declared in the Provider root config
(see §Source policies). The raw host-path prefix is private Nix/bundle authority
and is never projected into Provider config, Volume spec, status, or any public
DTO. The broker resolves the trusted path prefix and opens the directory via
the closed `OpenVolumeMountToken` operation using `openat2` anchored at the
prefix root. The pure adapter returns only an opaque `VolumeMountToken`
(authorization/correlation handle; no `OwnedFd` in Provider).

Source config schema:

```yaml
source:
  executionRef: Host/host-system
  settings:
    kind: local-path
    sourcePolicyId: default-state          # must match a declared source policy id
```

`sourcePolicyId` is validated against the Provider's `sourcePolicies` list at
Volume admission time. Raw host paths never appear in Volume spec, status,
resource list/watch responses, audit records, CLI output, OTEL spans, or logs.

Compatible `VolumeKind`: `durable`, `state`, `cache`, `ephemeral`, `tmp`.

### `block-image`

A raw or qcow2 disk-image file resolved from the `sourcePolicyId`. The broker
manages the image file through closed operations (create at provision via
`ProvisionBlockImage`, resize if declared, and delete at cleanup). The
Guest runtime provider (cloud-hypervisor or QEMU) receives a validated FD and
attaches the image as `virtio-blk`. The image is never passed by path; the FD
is transferred via the LaunchTicket for the Guest runtime process.

Source config schema:

```yaml
source:
  executionRef: Host/host-system
  settings:
    kind: block-image
    sourcePolicyId: default-block              # must match a declared source policy id
    imageFormat: raw                           # raw | qcow2; default raw
    preallocate: false                         # if true, fallocate at create via effect port
```

Constraints:

- `quota.maxBytes` is required.
- `VolumeKind` must be `durable` or `ephemeral`.
- `attachment[].transport` must be `virtio-blk` (handled by volume-local at the
  FD-handoff boundary; the Guest runtime provider owns mount semantics).
- At most one `read-write` attachment at any time; the single-writer constraint
  is enforced by the controller before handing off the FD.

### `tmpfs`

A memory-backed `tmpfs` mount. The controller sends a typed `MountTmpfs`
request to the injected `VolumeEffectPort`; the pure adapter maps it to the
closed broker operation. The broker derives `size=` and `nr_inodes=` from
trusted Volume quota fields and issues the mount syscall.
`quota.maxBytes` maps to `size=` and `quota.maxInodes` to `nr_inodes=`; both
are required. The kernel enforces these limits so enforcement is always
effectively `hard`. Cleanup unmounts the tmpfs via `umount_tmpfs` on the
`VolumeEffectPort`.

Source config schema:

```yaml
source:
  executionRef: Host/host-system
  settings:
    kind: tmpfs
    sourcePolicyId: default-tmpfs          # must match a declared tmpfs source policy
    mountFlags: []                         # optional extra mount flags; closed allowlist
```

Constraints:

- `quota.maxBytes` and `quota.maxInodes` are required.
- `VolumeKind` must be `ephemeral` or `tmp`.
- `tmpfs` Volumes are not persisted across Zone restart; `persistenceClass` is
  always `ephemeral` regardless of declared `kind`.
- No layout entries with `createPolicy: create-if-never-provisioned` or
  `restartPolicy: preserve-across-controller-restart` are valid for `tmpfs`;
  every entry is recreated on each mount.

### Source policies

The Provider root config declares the source policies the controller may
reference. A `sourcePolicyId` in a Volume spec must match one of these declared
IDs. The `class` determines which source-kind semantics the adapter applies. Raw
host path prefixes live only in the Nix-emitted private bundle and are never
projected into the Provider config or any public DTO.

```yaml
# Provider root config - validated against volume-local root-config.schema.json
sourcePolicies:
  - id: default-state
    class: local-path          # local-path | block-image | tmpfs
    volumeKinds: [durable, state, cache]
  - id: default-ephemeral
    class: local-path
    volumeKinds: [ephemeral, tmp]
  - id: default-tmpfs
    class: tmpfs
    volumeKinds: [ephemeral, tmp]
  - id: default-block
    class: block-image
    volumeKinds: [durable, ephemeral]
```

A `sourcePolicyId` that does not match a declared policy for the Volume's `kind`
fails Volume admission with `volume-source-policy-not-found`. This check is
performed at admit time, before any effect op. The private bundle records the
mapping from policy ID to actual host path prefix; the controller never reads
or stores that mapping.

---

## LayoutEntry schema

A `LayoutEntry` declares one managed path relative to the Volume root. Path
`""` is the Volume root itself. All paths must be:

- non-empty string except for root (`""`);
- no leading `/`;
- no `..` component;
- no null bytes;
- no Unicode homoglyphs of `/` or `\`;
- no drive-letter prefix (`C:`);
- evaluated without symlink traversal unless `noFollow: false` is explicit.

Maximum 1024 entries per Volume. Paths must be non-overlapping (no entry's path
is a prefix of another entry's path unless they differ only by the trailing
`/`-separated component).

### Full field reference

| Field | Type | Required | Default | Constraints |
| --- | --- | --- | --- | --- |
| `path` | string | Yes | - | Anchored relative path; `""` = Volume root |
| `type` | EntryType | Yes | - | `directory`, `file`, `symlink`, `unix-socket` |
| `ownerRef` | `User/<name>` ResourceRef | Yes | - | Same Zone; no numeric UID accepted |
| `groupRef` | `User/<name>` ResourceRef | Yes | - | Same Zone; no numeric GID accepted |
| `mode` | four-octet string | Yes | - | e.g. `"0700"`, `"0640"`, `"0660"` |
| `target` | string | Conditional | - | Required for `symlink` only; relative to Volume root; no `..`; no leading `/`; no null bytes; must resolve within Volume root |
| `accessAcl` | `AclGrant[]` | No | `[]` | Named access ACL; continuously reconciled |
| `defaultAcl` | `AclGrant[]` | No | `[]` | Default ACL for new children; continuously reconciled |
| `foreignChildPolicy` | `preserve` \| `fail` | No | `preserve` | Governs children not covered by `defaultAcl` |
| `noFollow` | bool | No | `true` | Reject symlink traversal during layout ops; may be `false` only for `symlink`-type entries |
| `recursive` | bool | No | `false` | Apply owner/mode/ACL recursively during repair; restricted by invariants |
| `sensitivity` | SensitivityClass | No | `private` | Governs audit redaction |
| `createPolicy` | CreatePolicy | No | `create-if-absent` | When to create |
| `repairPolicy` | RepairPolicy | No | `exact-owner` | How to reconcile drift |
| `cleanupPolicy` | CleanupPolicy | No | `never` | When to remove |
| `adoptionPolicy` | AdoptionPolicy | No | `adopt-with-live-owner-proof` | How an existing entry is treated on first bind |
| `restartPolicy` | RestartPolicy | No | `preserve-across-controller-restart` | Behavior across controller restart |
| `leaseClass` | LeaseClass | No | `none` | Live-ownership lease type |
| `invariants` | `Invariant[]` | No | `[no-symlink]` | Fail-closed additional checks |

### EntryType

| Value | Semantics | Current baseline analog |
| --- | --- | --- |
| `directory` | Standard directory | `StoragePathKind::Directory` |
| `file` | Regular file | `StoragePathKind::RegularFile` |
| `symlink` | Symbolic link; `noFollow: false` required; `target` required | `StoragePathKind::Symlink` |
| `unix-socket` | Unix domain socket; `mode: "0660"` default; process-scoped cleanup required | `StoragePathKind::UnixSocket` |

`DeviceNode` and `ExternalGrantOnly` from `StoragePathKind` are not LayoutEntry
types. Device nodes belong to Device Providers. `external-grant-only` maps to
an `observe-only` entry with `repairPolicy: none`.

### AclGrant schema

```yaml
principal:
  ref: User/example-system    # typed User/<name> ResourceRef; always same Zone
permissions: rwx               # POSIX ACL permission string: any combination of r/w/x/-
```

ACL principals are always typed `User/<name>` ResourceRefs. Numeric UID/GID
forms are rejected. The controller resolves the stable UID at reconciliation
time from the User resource and re-resolves on any User resource revision change
that affects the UID binding.

### CreatePolicy

| Value | Semantics | Baseline `StorageLifecycle` analog |
| --- | --- | --- |
| `create-if-absent` | Create if not present | `Config`, `Persistent` |
| `create-if-never-provisioned` | Create only if prior-provision marker absent; preserve existing content | swtpm/state hardening model |
| `always-recreate` | Always remove and recreate; use only for process-scoped entries | `ProcessScoped` |
| `observe-only` | Do not create; observe phase only | `ExternalObserveOnly` |

### RepairPolicy

| Value | Semantics | Baseline analog |
| --- | --- | --- |
| `none` | No repair; report drift as condition | `RepairPolicy::None` |
| `nix-activation` | NixOS activation is responsible | `RepairPolicy::NixActivation` |
| `exact-owner` | Broker reconciles owner/group/mode to exact declared values; non-recursive by default | `RepairPolicy::BrokerReconcile` |
| `fail-closed` | Any drift is a fatal condition; sets Degraded/Failed; no repair | `RepairPolicy::BrokerFailClosed` |
| `operator-only` | No automated repair; operator must intervene | `RepairPolicy::OperatorOnly` |

### CleanupPolicy

| Value | Semantics | Baseline analog |
| --- | --- | --- |
| `never` | Never removed by controller | `CleanupPolicy::Never` |
| `boot` | Removed on next host/Zone boot; entry is `/run/`-scoped | `CleanupPolicy::Boot` |
| `process-exit-with-proof` | Removed after owning Process exits (verified by pidfd) | `CleanupPolicy::ProcessExitWithProof` |
| `vm-stop-with-proof` | Removed when owning Guest stops (verified by controller) | `CleanupPolicy::VmStopWithProof` |
| `cutover-only` | Removed on cutover/generation switch | `CleanupPolicy::CutoverOnly` |
| `owner-controlled` | Lifecycle owned by the mounting controller | `CleanupPolicy::External` |

### AdoptionPolicy

| Value | Semantics | Baseline analog |
| --- | --- | --- |
| `adopt-with-live-owner-proof` | Adopt existing entry if owner proof (pidfd/cgroup) is live | `StorageAdoptionPolicy::AdoptWithLiveOwnerProof` |
| `recreate-from-persistent` | Delete existing and recreate from persistent state | `StorageAdoptionPolicy::RecreateFromPersistent` |
| `quarantine-on-ambiguity` | Quarantine existing; set Degraded; do not destroy | `StorageAdoptionPolicy::QuarantineOnAmbiguity` |
| `delete-if-owner-dead` | Delete existing if owner no longer live | `StorageAdoptionPolicy::DeleteIfOwnerDead` |
| `not-adoptable` | Always recreated on controller start | `StorageAdoptionPolicy::NotAdoptable` |

### RestartPolicy

| Value | Semantics | Baseline analog |
| --- | --- | --- |
| `preserve-across-controller-restart` | Retained across Volume controller restart | `StorageRestartPolicy::PreserveAcrossDaemonRestart` |
| `recreate-after-owner-death` | Recreated if owning process exits | `StorageRestartPolicy::RecreateAfterOwnerDeath` |
| `cleanup-after-owner-death` | Removed if owning process exits | `StorageRestartPolicy::CleanupAfterOwnerDeath` |
| `manual-recovery` | Requires operator action; controller sets Degraded | `StorageRestartPolicy::ManualRecovery` |
| `not-applicable` | No process owner; restart policy irrelevant | `StorageRestartPolicy::NotApplicable` |

### LeaseClass

| Value | Semantics | Baseline analog |
| --- | --- | --- |
| `none` | No live-ownership lease | `LeaseClass::None` |
| `process-pidfd` | Entry leased to process identified by pidfd | `LeaseClass::ProcessPidfd` |
| `cgroup-leaf` | Entry leased to cgroup leaf | `LeaseClass::CgroupLeaf` |
| `file-record` | Entry has OFD advisory lock (`O_RDWR | O_CREAT | O_CLOEXEC | O_NOFOLLOW`, never unlinked) | `LeaseClass::FileRecord` |

### SensitivityClass

| Value | Semantics | Baseline analog |
| --- | --- | --- |
| `public` | May be mentioned in status/logs at bounded granularity | `SensitivityClass::Public` |
| `private` | Path must not appear in public status/audit | `SensitivityClass::Private` |
| `secret-adjacent` | Path and size must not appear outside broker audit trail | `SensitivityClass::SecretAdjacent` |
| `audit` | Tamper-evident audit segment; special repair/cleanup rules | `SensitivityClass::Audit` |
| `zone-scoped` | Sensitivity bounded to Zone boundary | `SensitivityClass::RealmScoped` |

### Invariants

| Value | Semantics | Baseline analog |
| --- | --- | --- |
| `no-symlink` | Broker rejects symlinks during path walk | `StorageInvariant::NoSymlink` |
| `no-magic-link` | Broker rejects `/proc/self/...` magic links | `StorageInvariant::NoMagicLink` |
| `no-recursive-mutation` | Broker does not recurse into children | `StorageInvariant::NoRecursiveMutation` |
| `same-filesystem` | Entry must share `st_dev` with the Volume root (hardlink farm) | `StorageInvariant::SameFilesystem` |
| `hardlink-farm-no-recursion` | Entry is a hardlink farm node; broker does not recurse | `StorageInvariant::HardlinkFarmNoRecursion` |
| `broker-opaque-id-only` | Only broker-assigned identities may create children | `StorageInvariant::BrokerOpaqueIdOnly` |
| `scope-authorization-required` | Effect op on this entry requires confirmed `SourcePolicyId` authorization from the adapter; scope must be explicitly permitted for the requesting operation | - |
| `root-owned-parent` | Parent directory must be root-owned (TPM marker root) | - |

---

## User ACL, default ACL, view, repair, and foreign-child rules

### ACL reconciliation cycle

On every repair cycle (default 60 s for `durable`/`state` Volumes; on-start
only for `ephemeral`/`tmp`) the controller:

1. Resolves each `accessAcl` and `defaultAcl` entry's `User/<name>` to its
   stable UID via the Zone's User resource. Stale UID mappings (User revised)
   trigger an immediate re-resolve.
2. Calls `VolumeEffectPort::repair_layout_entry` with opaque User and
   LayoutEntry IDs. The pure adapter maps it to the closed broker
   `RepairLayoutEntry` operation; numeric IDs, mode, and ACL material are
   broker-resolved trusted inputs.
3. The broker applies `setfacl`/`acl_set_fd` on the anchored open FD, then
   compares the resulting `acl_get_fd` output with the declared set.
4. If the entry's `foreignChildPolicy == "fail"`, the broker reads the directory
   ACL and fails with `ForeignAclViolation` on any ACL entry not present in the
   declared `defaultAcl`. If `foreignChildPolicy == "preserve"`, surplus ACL
   entries are left unchanged.
5. The broker durably completes the `RepairLayoutEntry` audit record with
   `PrivilegedEffectAuditDigest`, optional `ResourceCorrelationDigest`, entry
   type, and repair action class - never with a raw Volume identity, entry path,
   or ACL value.

### View rights enforcement

A View grants rights that are the intersection of:

- the rights declared in the View spec, and
- the rights the LayoutEntry ACLs grant to the mounting Process's principal.

The controller validates the intersection at attach time. A mount that requests
any right absent from the intersection fails with `volume-view-rights-exceeded`.
Worker Processes with `access: read-only` that declare a `rights: [read,
traverse]` view cannot request `write` even if the Volume LayoutEntry permits
writes to their principal.

### Repair and drift detection

The controller maintains a per-entry drift state derived from the last completed
`RepairLayoutEntry` call. Drift is detected at the start of each reconcile cycle
by `fstatat`+`acl_get_fd` on the anchored FD. Detected conditions:

| Condition type | Trigger | Status effect |
| --- | --- | --- |
| `EntryMissing` | Entry absent and `createPolicy != observe-only` | `layoutPhase: Degraded` |
| `EntryDrift` | owner/mode/ACL diverged; `repairPolicy == none` | condition set; no repair |
| `EntryQuarantined` | Adoption ambiguity detected | `layoutPhase: Degraded`; quarantine record written |
| `InvariantViolated` | `no-symlink`, `no-magic-link`, or `same-filesystem` invariant failed | `layoutPhase: Failed` |
| `ForeignAclViolation` | `foreignChildPolicy: fail` with unlisted ACL entry | `layoutPhase: Degraded` |

---

## Quota specification and enforcement

```yaml
quota:
  maxBytes: 10737418240     # required when enforcement: hard; or when source.settings.kind: block-image or tmpfs
  maxInodes: 1000000         # required when source.settings.kind: tmpfs or enforcement: hard
  enforcement: none          # none | hard
```

### Enforcement behaviour by source kind

| Source kind | `enforcement: hard` effect | `enforcement: none` effect |
| --- | --- | --- |
| `local-path` | Controller checks backing FS quota capability at provision; sets `Failed` immediately if the FS cannot enforce byte/inode limits | Limits recorded for informational use; no rejection |
| `block-image` | Controller provisions the image at exactly `quota.maxBytes`; FS inside the image enforces limits | Image size is advisory; no enforcement |
| `tmpfs` | `quota.maxBytes` → `size=`; `quota.maxInodes` → `nr_inodes=` mount options; kernel enforces; always effectively `hard` | Not valid for `tmpfs`; both limits are always required and kernel-enforced |

When `quotaBytes > 0` in the Provider state schema:

1. The controller checks available space on the backing FS before provisioning
   and emits `quota-insufficient` if the requested quota cannot be reserved.
2. The controller reports `quotaUsage` in Volume status at a bounded polling
   interval (maximum every 60 s) using `statfs` on the anchored FD.
3. Mounts with `rights: [write, create]` are rejected when the Volume is at or
   above quota, returning `volume-quota-exceeded`.
4. A mismatch between component descriptor `quotaBytes` and Volume spec
   `quotaBytes` fails Volume admission with `quota-mismatch`.

---

## Identity markers and fail-closed detection

Every `persistent` or `cache`-class Volume provisioned by volume-local has an
identity marker file. The marker is maintained at a broker-private root
**outside** the Volume's own tree (analogous to `swtpm-markers/<vm>` in the
current baseline; `packages/d2b-priv-broker/src/ops/swtpm_dir.rs`).

### Marker anatomy

The marker root is `$stateDir/volume-local-markers/`. The marker file path is:

```
$stateDir/volume-local-markers/<volume-uid>
```

The marker file is:

- a regular file, root-owned, mode `0600`, created by the broker;
- single-link (nlink == 1); symlink and magic-link rejected;
- `invariants: [no-symlink, root-owned-parent, broker-opaque-id-only,
  scope-authorization-required]`;
- content: a canonical JSON structure containing:
  - `volumeUid`: the Volume's stable UID (opaque);
  - `stDev` / `stIno`: the Volume root directory inode identity at first
    provision;
  - `schemaId` / `schemaVersion` / `schemaDigest`: from the stateSchema block;
  - `provisionedAt`: RFC 3339 UTC timestamp;
  - `hmac`: SHA-256 HMAC over the above fields using a broker-private key.

The marker path itself is never included in any audit record, status field, or
CLI output. The broker identifies the marker by `volumeUid` through the marker
root directory FD; no caller supplies the path.

### Marker lifecycle

| Phase | Action |
| --- | --- |
| First provision | Broker writes marker with `(st_dev, st_ino)` at creation time |
| Controller restart | Controller calls `marker::verify_marker` for every served Volume (`providerRef: Provider/volume-local`) during startup relist |
| Daemon restart | Controller's startup reconcile re-reads every Volume status and re-verifies markers |
| Each Process launch | Controller verifies marker before handing FD to Process supervisor |

### Failure modes

| Condition | Response |
| --- | --- |
| Marker file missing after prior provision | `markerStatus: missing`; Volume → `Failed`; blocked Processes → `Degraded`; never auto-recover |
| `st_ino` mismatch (directory replaced) | `markerStatus: replaced`; Volume → `Failed`; never silently re-provision |
| Marker present but Volume root absent | `markerStatus: missing`; Volume → `Failed` |
| `installedSchemaVersion` > spec version | Volume → `Failed`; `stateSchemaPhase: migration-failed`; manual rollback required |
| HMAC validation failed | `markerStatus: tampered`; Volume → `Failed`; operator intervention required |

None of these cases auto-recover. Operator sets a dedicated condition-clear
operation after confirming integrity.

---

## Same-filesystem hardlink farm (store-view)

The per-VM Nix store hardlink farm is a Volume with `Provider/volume-local`,
`kind: durable`, `source.settings.kind: local-path`, and root at
`$stateDir/<vm>/store-view`. This corresponds to the current
`nixos-modules/store.nix` activation and
`packages/d2b-host/src/hardlink_farm.rs`.

### Store-view Volume spec summary

```yaml
type: Volume
spec:
  providerRef: Provider/volume-local
  source:
    executionRef: Host/host-system
    settings:
      kind: local-path
      sourcePolicyId: default-state          # references store-view source policy
  kind: durable
  persistenceClass: config
  layout:
    - path: ""
      type: directory
      ownerRef: User/d2bd
      groupRef: User/users
      mode: "0755"
      noFollow: true
      invariants: [no-symlink, scope-authorization-required]
      createPolicy: create-if-absent
      repairPolicy: exact-owner
    - path: live
      type: directory
      ownerRef: User/d2bd
      groupRef: User/users
      mode: "0755"
      noFollow: true
      invariants: [no-symlink, broker-opaque-id-only]
      createPolicy: create-if-absent
      cleanupPolicy: cutover-only
    - path: live/.d2b-marker-<vm>
      type: file
      ownerRef: User/d2bd
      groupRef: User/users
      mode: "0444"
      invariants: [no-symlink, same-filesystem, hardlink-farm-no-recursion, broker-opaque-id-only]
      createPolicy: create-if-absent
      repairPolicy: fail-closed
    - path: meta
      type: directory
      ownerRef: User/d2bd
      groupRef: User/users
      mode: "0755"
      invariants: [no-symlink, same-filesystem, hardlink-farm-no-recursion, broker-opaque-id-only]
      createPolicy: create-if-absent
    - path: meta/generations
      type: directory
      ownerRef: User/d2bd
      groupRef: User/users
      mode: "0755"
      invariants: [no-symlink, same-filesystem, hardlink-farm-no-recursion, broker-opaque-id-only]
      cleanupPolicy: cutover-only
    - path: meta/current
      type: symlink
      ownerRef: User/d2bd
      groupRef: User/users
      mode: "0777"
      noFollow: false
      invariants: [broker-opaque-id-only]
      target: generations/<current-N>     # relative; updated at each generation cutover
    - path: state
      type: directory
      ownerRef: User/d2bd
      groupRef: User/users
      mode: "0700"
      noFollow: true
      invariants: [no-symlink, broker-opaque-id-only]
      # host-only; NOT guest-served via virtiofsd
    - path: gcroots
      type: directory
      ownerRef: User/d2bd
      groupRef: User/users
      mode: "0755"
      invariants: [no-symlink, same-filesystem, hardlink-farm-no-recursion, broker-opaque-id-only]
      cleanupPolicy: cutover-only
      # host-only; at store-view root (NOT under meta/); see spec correction below
    - path: sync.lock
      type: file
      ownerRef: User/d2bd
      groupRef: User/users
      mode: "0640"
      invariants: [no-symlink, broker-opaque-id-only]
      leaseClass: file-record
      cleanupPolicy: never
      # OFD advisory lock; never unlinked (preserves OFD semantics across restart)
  views:
    ro-store:
      path: live
      rights: [read, traverse]
    meta:
      path: meta
      rights: [read, traverse]
    state:
      path: state
      rights: [read, write, create, delete, traverse]
  attachments:
    - executionRef: Guest/<vm>
      transport: virtiofs
      view: ro-store
      access: read-only
      mountPath: /nix/.ro-store
```

**Spec correction**: `nixos-modules/storage-json.nix` (baseline `b5ddbed6`)
declares `path:store-view-gcroots` at `store-view/meta/gcroots` and omits
`store-view/state/` entirely.
`packages/d2b-host/src/hardlink_farm.rs::gcroots_dir()` places gcroots at
`store-view/gcroots` (store-view root), confirmed by
`packages/d2b-priv-broker/src/ops/store_view_posture.rs`
(`posture_store_view_matrix_paths`). Code wins; the v3 LayoutEntry spec follows
`hardlink_farm.rs`. The `storage-json.nix` drift is resolved when Volume
resources replace the path rows.

### Hardlink farm invariants

1. **virtiofsd serves `store-view/live`, never host `/nix/store`**. The compile-time
   sentinel `share.source == "/nix/store"` in `processes-json.nix` triggers
   store-view substitution; virtiofsd's `--shared-dir` is `store-view/live`,
   not `/nix/store`.

2. **Same-filesystem requirement**: hardlinks require `st_dev` equality between
   `/nix/store` and `$stateDir`. If they differ, the controller fails closed
   with `storage-drift`.

3. **Private mount namespace**: the broker performs hardlink operations inside a
   private mount namespace where `/nix/store` is lazily detached from its
   bind-mount shadow (NixOS bind-mounts `/nix/store` on itself; a same-`st_dev`
   cross-vfsmount `link(2)` returns `EXDEV` - recoverable, distinct from a
   fatal different-filesystem `EXDEV`). An `EMLINK` fallback (saturated empty-file
   inode) copies the byte content.

4. **Broker no-recursion posture**: `store_view_posture.rs::posture_store_view_matrix_paths`
   applies `invariants: [hardlink-farm-no-recursion]` to `state/`, `gcroots/`,
   and `sync.lock`. The broker does not recurse into these nodes.

5. **Readiness marker**: `live/.d2b-marker-<vm>` is a zero-length file owned
   `d2bd:users 0444`. Its existence is checked by the virtiofsd readiness
   predicate before the virtiofsd worker is considered ready.

### Store sync broker op

The store-sync operation (`StoreSyncComplete`) runs the hardlink farm rebuild:

1. Broker acquires the `sync.lock` OFD write lock on the anchored FD.
2. Broker calls `d2b_host::hardlink_farm::build_store_view` with the
   `BuildStoreViewRequest` derived from the Volume's current generation metadata.
3. On completion, the broker releases the OFD lock, durably commits the
   exactly-once `StoreSyncComplete` record, and only then returns success. The
   record carries `PrivilegedEffectAuditDigest`, optional
   `ResourceCorrelationDigest`, and the generation number only (no raw Volume
   identity, paths, store-path list, or size).

---

## TPM Volume

The per-VM swtpm state directory is a Volume with `Provider/volume-local`,
`kind: state`, `source.settings.kind: local-path`.

### TPM Volume LayoutEntry

```yaml
layout:
  - path: ""
    type: directory
    ownerRef: User/d2b-<vm>-swtpm
    groupRef: User/d2b-<vm>-swtpm
    mode: "0700"
    createPolicy: create-if-never-provisioned
    repairPolicy: fail-closed
    cleanupPolicy: never
    adoptionPolicy: quarantine-on-ambiguity
    sensitivity: secret-adjacent
    invariants: [no-symlink, broker-opaque-id-only, scope-authorization-required]
```

### TPM invariants

1. **Fail-closed owner**: any mismatch between declared `ownerRef` UID and
   `st_uid` fails closed with a typed, path-free error. The controller never
   silently chowns existing NVRAM.

2. **Provisioning marker**: the TPM provisioning marker is a root-owned regular
   file at `$stateDir/swtpm-markers/<vm>` (`0600`), created by the broker with
   `invariants: [no-symlink, root-owned-parent, broker-opaque-id-only,
   scope-authorization-required]`. It records the trusted `(st_dev, st_ino)`
   plus first-provision stamp. If the swtpm directory is absent after the marker
   was written, the controller sets `Failed` with condition
   `previously-provisioned-swtpm-state-missing` and refuses to re-provision.

3. **Stale socket cleanup**: a stale `tpm.sock` under the Zone runtime directory
   is unlinked before the swtpm Process is started. The socket path belongs to
   the TPM Device Provider runtime, not the TPM Volume layout.

4. **Sensitivity**: `sensitivity: secret-adjacent`. The swtpm state path must
   never appear in public status, audit, CLI output, OTEL spans, or logs.

5. **Ancestor traverse ACL**: the broker sets a minimal `--x` traverse ACL on
   the Volume root's ancestor directories up to the per-Zone state root, allowing
   the `d2b-<vm>-swtpm` principal to reach the directory without exposing peers.
   This matches `swtpm_dir.rs::provision_ancestor_acls`.

---

## Named views and rights

A View maps a name to a subtree of the Volume root and a bounded rights set.
ViewName must match `^[a-z][a-z0-9-]*$`. Max 64 Views per Volume. A Volume must
have at least one View.

```yaml
views:
  controller:
    path: ""
    rights: [read, write, create, delete, traverse, execute]
  reader:
    path: data
    rights: [read, traverse]
  config:
    path: config
    rights: [read]
  worker-read:
    path: public
    rights: [read, traverse]
```

### Rights

| Right | Meaning |
| --- | --- |
| `read` | Read file contents and directory entries |
| `write` | Modify file contents; create, delete, rename within the subtree |
| `create` | Create new files/directories directly in this subtree |
| `delete` | Remove files/directories from this subtree |
| `traverse` | Enter directories (needed to reach sub-paths) |
| `execute` | Execute files; implies `traverse` on parent directories |

A View grants only rights that the Volume LayoutEntry ACLs permit for the
Process/Guest principal. The controller validates right intersection at attach
time.

---

## Process volume mounts

Process and EphemeralProcess specs inline their Volume mounts:

```yaml
mounts:
  - volumeRef: Volume/work-state
    view: controller
    mountPath: /state
    access: read-write    # read-only | read-write
    required: true
```

| Field | Type | Required | Default | Constraints |
| --- | --- | --- | --- | --- |
| `volumeRef` | ResourceRef | Yes | - | Must resolve to a Ready Volume in the same Zone |
| `view` | ViewId | Yes | - | Must exist in the Volume spec; bounded `^[a-z][a-z0-9-]*$`; max 63 chars |
| `mountPath` | absolute path string | Yes | - | Inside the Process sandbox; no overlap |
| `access` | `read-only` \| `read-write` | No | `read-only` | Must be compatible with View rights |
| `required` | bool | No | `true` | If `false`, absent/Degraded Volume does not prevent Process start |

The Process Provider (system-systemd or system-minijail) resolves the Volume
root at launch time through a broker FD delivered in the LaunchTicket. The raw
host path never appears in Process ResourceSpec, status, or audit.

### Volume root FD delivery

The controller's attach-time flow:

1. Verify marker (see §Identity markers).
2. Call `request_mount_token(vol, view_id, access)` on the `VolumeEffectPort`.
   The pure adapter maps the request to `OpenVolumeMountToken`. The broker
   opens the Volume root via `openat2` anchored at its private policy-prefix FD,
   verifies `st_dev`/`st_ino` against the marker record, durably completes the
   effect audit, and transfers the anchored FD out-of-band for direct routing
   to the target ProviderSupervisor.
3. The adapter returns a `VolumeMountToken` - an opaque authorization/correlation
   handle with a fully redacted `Debug` impl. The token contains no `OwnedFd`
   visible to the Provider process.
4. The controller embeds the token in the LaunchTicket sent to the Process
   supervisor. No raw path or FD crosses the controller→supervisor boundary
   in the LaunchTicket itself; the supervisor correlates the token with the
   pre-delivered FD using the token ID.

---

## Volume lifecycle phases

```text
Pending → Ready
       → Degraded (recoverable drift, quarantine, failed ACL repair, quota soft-exceeded)
       → Failed   (invariant violated, missing-after-provision, marker tampered/replaced)
       → Unknown  (controller/Host unreachable)
```

The `LayoutReady` condition on the Volume status summarizes the layout phase:

```yaml
conditions:
  - type: LayoutReady
    status: "True"
    reason: layout-reconciled
    observedGeneration: 1
  - type: AttachmentsReady
    status: "True"
    reason: all-attachments-ready
    observedGeneration: 1
```

Condition messages are bounded (512 bytes), UTF-8/control-character validated,
and must not contain host paths, secret content, process data, or terminal bytes.

---

## VolumeEffectPort - injected effect boundary

The `Provider/volume-local` controller process never opens host paths, calls
`openat2` or any syscall that takes a raw host path, issues `setfacl`,
`mount`/`umount`, `fallocate`, `unlinkat`, `write`, or a key operation, receives
numeric UIDs, or holds a direct connection to the Zone broker. The injected
`VolumeEffectPort` implementation is a pure core adapter outside the Provider
process. It maps typed requests to closed broker operations; the broker alone
resolves authority, performs host effects, and owns durable audit.

### VolumeEffectPort trait

Defined in `d2b-contracts/src/v3/effect_port.rs` (neutral contract crate shared
by the Provider and the pure core adapter). The core adapter implements the
trait; the Provider crate imports and uses it. The separate closed
`VolumeBrokerOperation` request/result sum types live in
`d2b-contracts/src/broker_wire.rs` and are imported only by the adapter and
broker surfaces, not re-exported by the Provider crate. The Provider crate must
not define the trait, and core must not import any Provider-implementation
crate. The Provider crate depends only on `d2b-contracts`, `d2b-provider`, and
`d2b-provider-toolkit`.

All opaque ID newtypes carry a custom redacted `Debug` implementation; they must
not derive `Debug` or print internal content. The two IDs carried by
`RotateSealingKeyRequest` use the same newtypes as the trait methods, derive the
wire traits directly, and deserialize through a shared bounded validator.
Their string fields remain crate-private. The canonical encoding is one
non-empty ASCII-graphic string of at most 128 bytes; rejection errors never echo
the input:

```rust
// Defined in d2b-contracts::v3::effect_port
// All IDs are opaque newtypes with custom redacted Debug.
const MAX_VOLUME_EFFECT_WIRE_ID_BYTES: usize = 128;

fn validate_volume_effect_wire_id(value: &str) -> Result<(), &'static str> {
    if value.is_empty() {
        return Err("opaque effect ID must not be empty");
    }
    if value.len() > MAX_VOLUME_EFFECT_WIRE_ID_BYTES {
        return Err("opaque effect ID exceeds 128 bytes");
    }
    if !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err("opaque effect ID contains an invalid byte");
    }
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct VolumeId(pub(crate) String);
pub struct SourcePolicyId(pub(crate) String);
pub struct LayoutEntryId(pub(crate) String);
pub struct UserId(pub(crate) String);     // resolves from User/<name> resource
pub struct ViewId(pub(crate) String);     // bounded view name; max 63 chars
#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct SealingPolicyId(pub(crate) String);

impl TryFrom<String> for VolumeId {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_volume_effect_wire_id(&value)?;
        Ok(Self(value))
    }
}

impl TryFrom<String> for SealingPolicyId {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_volume_effect_wire_id(&value)?;
        Ok(Self(value))
    }
}

impl fmt::Debug for VolumeId      { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("VolumeId([redacted])") } }
impl fmt::Debug for SourcePolicyId{ fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("SourcePolicyId([redacted])") } }
impl fmt::Debug for LayoutEntryId { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("LayoutEntryId([redacted])") } }
impl fmt::Debug for UserId        { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("UserId([redacted])") } }
impl fmt::Debug for ViewId        { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("ViewId([redacted])") } }
impl fmt::Debug for SealingPolicyId { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("SealingPolicyId([redacted])") } }

/// Closed, deny-unknown request. OperationId is the opaque identifier from the
/// committed Resource operation; none of these fields contains key bytes,
/// credential bytes, a host path, or a broker-resolved key handle.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotateSealingKeyRequest {
    pub volume: VolumeId,
    pub policy: SealingPolicyId,
    pub expected_volume_generation: u64,
    pub expected_resource_revision: u64,
    pub expected_current_key_generation: u64,
    pub target_key_generation: u64,
    pub operation_id: OperationId,
}

pub enum RotateSealingKeyDisposition {
    Rotated,
    AlreadyCommitted,
    RecoveredCommitted,
}

/// Returned only after the target generation is active, the previous
/// generation is retired, and the success audit record is durable.
pub struct RotateSealingKeyResult {
    pub disposition: RotateSealingKeyDisposition,
    pub volume_generation: u64,
    pub active_key_generation: u64,
}

pub enum RotationPrecondition {
    VolumeGeneration,
    ResourceRevision,
    PolicyBinding,
    CurrentKeyGeneration,
    TargetKeyGeneration,
}

/// All variants have bounded, path-free wire encodings and redacted Debug.
pub enum RotateSealingKeyError {
    Unauthorized,
    PreconditionFailed { precondition: RotationPrecondition },
    TargetKeyUnavailable { retry_after_ms: Option<u64> },
    TargetKeyRevoked,
    RotationConflict,
    IdempotencyConflict,
    IntegrityViolation,
    ResourceExhausted,
    BackendUnavailable,
    DeadlineExceeded,
    CommitPendingAudit,
}

/// Opaque mount authorization handle. Contains no OwnedFd visible to the
/// Provider. Core routes the anchored FD directly from the EffectPort adapter
/// to the target ProviderSupervisor out-of-band. The token is an authorization
/// and correlation handle only; its Debug output is fully redacted.
pub struct VolumeMountToken {
    pub(crate) token_id: String,      // opaque; never printed in logs
    pub(crate) view_id: ViewId,
    pub(crate) access: AccessClass,
    pub(crate) volume_id: VolumeId,
}
impl fmt::Debug for VolumeMountToken { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("VolumeMountToken([redacted])") } }

pub trait VolumeEffectPort: Send + Sync + 'static {
    /// Provision or reconcile a TPM state directory and fail-closed marker.
    async fn prepare_swtpm_dir(&self, vol: VolumeId)
        -> Result<SwtpmDisposition, EffectError>;

    /// Provision or verify-create a layout entry relative to the Volume root.
    async fn provision_layout_entry(&self, vol: VolumeId, entry: LayoutEntryId,
        owner: UserId, group: UserId) -> Result<ProvisionOutcome, EffectError>;

    /// Reconcile owner/group/mode and ACLs for an existing entry.
    async fn repair_layout_entry(&self, vol: VolumeId, entry: LayoutEntryId,
        owner: UserId, group: UserId) -> Result<RepairOutcome, EffectError>;

    /// Remove a layout entry ordered leaf-first.
    async fn cleanup_layout_entry(&self, vol: VolumeId, entry: LayoutEntryId,
        trigger: CleanupTrigger) -> Result<(), EffectError>;

    /// Verify the identity marker for a Volume.
    async fn verify_marker(&self, vol: VolumeId) -> Result<MarkerStatus, EffectError>;

    /// Write the identity marker for a Volume at first provision.
    async fn provision_marker(&self, vol: VolumeId) -> Result<(), EffectError>;

    /// Remove the identity marker (destruction only).
    async fn cleanup_marker(&self, vol: VolumeId) -> Result<(), EffectError>;

    /// Check that the backing FS can reserve the declared quota.
    /// The broker performs statfs(2) in its bounded blocking pool.
    async fn check_quota_capacity(&self, vol: VolumeId, max_bytes: u64,
        max_inodes: Option<u64>) -> Result<QuotaCapacityStatus, EffectError>;

    /// Poll current quota usage. The broker performs statfs(2).
    async fn poll_quota_usage(&self, vol: VolumeId) -> Result<QuotaUsage, EffectError>;

    /// Create or verify a block-image file at declared size; fallocate if preallocate.
    async fn provision_block_image(&self, vol: VolumeId,
        policy: SourcePolicyId) -> Result<BlockImageStatus, EffectError>;

    /// Mount a tmpfs at the Volume root with size= and nr_inodes= from quota.
    async fn mount_tmpfs(&self, vol: VolumeId,
        policy: SourcePolicyId) -> Result<(), EffectError>;

    /// Unmount the tmpfs at the Volume root.
    async fn umount_tmpfs(&self, vol: VolumeId) -> Result<(), EffectError>;

    /// Execute the hardlink farm sync cycle (acquire OFD lock, build farm, release).
    async fn run_store_sync(&self, vol: VolumeId,
        generation: u64) -> Result<StoreSyncOutcome, EffectError>;

    /// Durably replace one declared structured-state slot.
    async fn commit_volume_transaction(&self, request: CommitVolumeTransactionRequest)
        -> Result<CommitVolumeTransactionResult, EffectError>;

    /// Create or expire one snapshot through a closed broker operation.
    async fn create_volume_snapshot(&self, request: CreateVolumeSnapshotRequest)
        -> Result<CreateVolumeSnapshotResult, EffectError>;
    async fn expire_volume_snapshot(&self, request: ExpireVolumeSnapshotRequest)
        -> Result<(), EffectError>;

    /// Copy and durably commit one relocation generation.
    async fn relocate_volume_contents(&self, request: RelocateVolumeContentsRequest)
        -> Result<RelocateVolumeContentsResult, EffectError>;

    /// Atomically rewrap all sealed StateEnvelopes from the expected key
    /// generation to the target generation through the closed broker operation.
    async fn rotate_sealing_key(&self, request: RotateSealingKeyRequest)
        -> Result<RotateSealingKeyResult, RotateSealingKeyError>;

    /// Request a VolumeMountToken for delivery to a Process supervisor.
    /// Returns an opaque authorization handle; the anchored FD is routed
    /// directly by core from the adapter to the target ProviderSupervisor
    /// and is never returned to the Provider process.
    async fn request_mount_token(&self, vol: VolumeId,
        view: ViewId, access: AccessClass) -> Result<VolumeMountToken, EffectError>;

    /// Remove the Volume root directory after all entries are cleaned up.
    async fn cleanup_volume_root(&self, vol: VolumeId) -> Result<(), EffectError>;
}
```

The core adapter implementing this trait:

1. Receives opaque IDs and bounded semantic DTOs, never a raw path, numeric
   UID/GID, ACL string, mount option string, key handle, command, or FD.
2. Maps each trait method to exactly one `VolumeBrokerOperation` variant. The
   mapping is exhaustive in both directions; no generic or fall-through
   variant exists.
3. Attaches the committed operation proof and dispatches the typed request. It
   does not open the private bundle, maintain a trusted FD table, or perform a
   syscall.
4. For `OpenVolumeMountToken`, correlates the broker's audited SCM_RIGHTS
   transfer directly to the target ProviderSupervisor. The Provider receives
   only the opaque token.
5. Returns success only after the broker response proves durable completion
   audit. `CommitPendingAudit` remains pending and is retried byte-identically.

The broker independently resolves and authorizes every opaque ID, derives all
paths and numeric identities from private authority, performs every blocking
syscall in its bounded blocking pool, and owns intent/completion audit. No
`VolumeEffectPort` or `VolumeBrokerOperation` request accepts a caller-supplied
path, numeric identity, ACL string, mount option string, key material, or
broker-resolved handle.

### Closed broker operation catalogue

Every row is one `VolumeBrokerOperation` variant. The broker audit envelope
always carries `PrivilegedEffectAuditDigest`, closed `VolumeEffectClass`,
closed outcome, and closed failure reason. `ResourceCorrelationDigest` is
present only when the record class requires a Volume join.

| Broker operation | Broker-owned effect | Additional typed audit fields |
| --- | --- | --- |
| `ProvisionLayoutEntry` | Anchored `openat2`; create/write/link as required by closed `EntryType`; ownership and mode application | `EntryType`, `ProvisionDisposition` |
| `RepairLayoutEntry` | Anchored open/stat; resolve User IDs; apply and verify access/default ACL, owner, group, and mode | `EntryType`, `RepairActionClass` |
| `CleanupLayoutEntry` | Anchored open and leaf-first `unlinkat`/directory removal | `EntryType`, `CleanupTrigger` |
| `PrepareSwtpmDir` | Anchored provision/adopt plus fail-closed marker and ancestor ACL | `SwtpmDisposition` |
| `CheckQuotaCapacity` | Anchored `statfs` capability/capacity check | `QuotaCapacityClass` |
| `PollQuotaUsage` | Anchored `statfs` observation | `QuotaUsageClass` |
| `StoreSyncComplete` | OFD lock, anchored hardlink/copy/write/unlink, and generation commit | generation number, `StoreSyncDisposition` |
| `MountTmpfs` | Broker-derived flags/options and `mount` | `MountAction::Mount` |
| `UnmountTmpfs` | Anchored mount identity verification and `umount2` | `MountAction::Unmount` |
| `ProvisionBlockImage` | Anchored create/verify, size enforcement, optional `fallocate`, and durable metadata | `BlockImageAction` |
| `CommitVolumeTransaction` | Anchored bounded write, `fsync`, atomic link/rename, and parent `fsync` | `TransactionDisposition` |
| `CreateVolumeSnapshot` | Anchored snapshot generation create/copy/write and durable commit | `SnapshotTrigger`, `SnapshotDisposition` |
| `ExpireVolumeSnapshot` | Anchored snapshot generation `unlinkat` cleanup | `SnapshotExpiryReason` |
| `RelocateVolumeContents` | Broker-authorized anchored copy/write/fsync and target-generation commit | `RelocationDisposition` |
| `OpenVolumeMountToken` | Anchored `openat2`, marker/inode verification, audited SCM_RIGHTS transfer | `AccessClass`, `MountTokenDisposition` |
| `WriteVolumeMarker` | Anchored bounded marker write and durable rename | `MarkerWriteDisposition` |
| `VerifyVolumeMarker` | Anchored open/read/HMAC verification | `MarkerStatus` |
| `CleanupVolumeMarker` | Anchored marker `unlinkat` | `CleanupTrigger::Destruction` |
| `CleanupVolumeRoot` | Anchored final root removal after empty/posture verification | `CleanupTrigger` |
| `RotateSealingKey` | Journaled key-generation rewrap, atomic switch, retirement, and recovery | from/to generation, `RotateSealingKeyDisposition` |

An effect that cannot be expressed by exactly one row is denied until the
contract, broker policy, durable audit class, and tests add a new row.

---

## Controller process and EphemeralProcess catalog

### Controller: `volume-local-controller`

The controller is declared in `Provider.spec.config.controllerExecutionRef` as a
`Process` resource under `Host/host-system` with `domain: system`:

```yaml
# Provider.spec.config
controllerExecutionRef: Host/host-system

# Controller Process resource
type: Process
metadata:
  name: volume-local-controller
  ownerRef: Provider/volume-local
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  userRef: null
  processClass: controller
  template: volume-local-controller
  configRef: null
  credentialRefs: []
  mounts: []                # no Provider state Volume; operational state in status/core ledger (D087)
  sandbox:
    namespaceClasses: []
    capabilityClasses: []
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    environmentClass: minimal
    readOnlyRoot: true
    umask: "0022"
    oomScoreAdj: 0
    userNamespace: null
  budget: {}
  networkUsage: null
  deviceUsage: []
  telemetry: {}
  desiredLifecycle: running
  restartPolicy:
    class: on-failure
    backoffBase: "5s"
    backoffMax: "300s"
    backoffMultiplierMilli: 2000
    maxRestarts: null
    resetAfter: "300s"
  readiness:
    initialDelay: "0s"
    timeout: "30s"
    failureThreshold: 3
    successThreshold: 1
    class: ready-condition
  healthCheck:
    enabled: false
    interval: "30s"
    timeout: "5s"
    failureThreshold: 3
    class: provider-defined
  adoptionPolicy: adopt-on-restart
  drainTimeout: "30s"
```

The controller holds no ambient capabilities, performs no direct filesystem
mutations, and has no direct connection to the Zone broker. It receives the
`VolumeEffectPort` implementation injected by the Zone runtime via its
ComponentSession on startup.

### ProviderStateSet

A **ProviderStateSet** is the optional, query-time set of the *declared* Volume
resources in a Zone whose `metadata.ownerRef` resolves to `Provider/volume-local`:

```text
ProviderStateSet(zone, "volume-local") =
  { v : Volume | v.metadata.zone == zone
              && v.metadata.ownerRef == "Provider/volume-local" }
```

This is a query-time logical grouping, not a ResourceType or stored artifact,
and it is **empty** for `Provider/volume-local`.

`Provider/volume-local` declares **no** Provider state Volume of its own. Its
bounded non-secret operational state - reconcile stage, per-Volume layout/marker
observations, quota usage, adoption observations, and closed-enum error detail -
lives in the owning resource's `status` subresource and the core Operation
ledger (D087). Because that state is fully derivable from the Volume resources
it reconciles, their `status`, the core Operation ledger, and independent
external observation (marker re-verification against external reality), it fails
the storage-need test: the volume-local controller declares no state namespace,
no state Volume, no state-view mount, and no dedicated
`User/volume-local-system` state-layout principal. There is no empty
identity-only Volume.

`Volume` remains **volume-local's exported and reconciled ResourceType**.
Volume-local is the sole reconciler for all Volumes that carry
`providerRef: Provider/volume-local` and never issues Volume create/delete API
calls. Two populations of Volumes are assigned to volume-local (neither is owned
by `Provider/volume-local`, so neither is in its ProviderStateSet):

- **Operator-created Volumes** (`kind: durable`, `cache`, `ephemeral`, `tmp`,
  `state`): operators create and delete these via the Resource API; volume-local
  reconciles their physical state (layout, ACL, quota, identity marker).
- **Other Providers' declared state Volumes**: **core ProviderDeployment**
  creates one Volume per *declared* `stateNamespace` in a component descriptor
  (only when that payload passes the storage-need test) before the component's
  Process starts, and deletes those Volumes after Processes stop when the owning
  Provider is removed. Volume-local reconciles their physical state; it does not
  create or delete these instances, and they are owned by their own Provider,
  not by volume-local.

**Component Processes consume only their required view**: each Process receives a
`VolumeMountToken` for its declared named view; no Process creates, watches, or
manages any Volume resource.

Layout ACL principals are **Nix-preprovisioned `User/<name>` resources**
declared in the Provider's Nix configuration (or drawn from a bounded principal
pool declared in the bundle). Numeric UIDs never appear in Volume spec, layout
entries, or any public DTO. Each component's declared state Volume is strictly
private to that component; no such Volume is shared across components. A
component receives only its declared named view: the core routes the anchored
dirfd for that view to the requesting Process supervisor; no other component or
domain observes that dirfd.

The common `phase` field (`Pending`/`Ready`/`Degraded`/`Failed`/`Unknown`) for
`Provider/volume-local` is aggregated by the Zone core from the health of its
controller Process and the Volumes it reconciles, and reported in Provider
resource status; no custom provider-level status extension is defined.

### No bootstrap-state exception

Volume-local is itself the storage Provider, but because its controller declares
**no** state Volume, there is nothing to provision before the controller is
active - so there is no bootstrap state-Volume cycle, no closed bootstrap
storage sequence, no broker layout pre-provision, and no bootstrap-storage
exception (D086, superseded by D087; see "No bootstrap state Volume" in
`ADR-046-components-processes-and-sandbox`).

On first install and on every daemon restart the volume-local controller Process
starts and reaches `Ready` from its own resource `status`, the core Operation
ledger, and a resource-store relist; there is no hidden bootstrap store and no
pre-provisioned controller Volume. Once Ready, it reconciles every Volume
carrying `providerRef: Provider/volume-local` (operator-created Volumes and
other Providers' declared state Volumes) as they appear in its `providerRef`
watch - re-verifying identity markers against external reality - never creating
them itself. A Guest bootstraps its own Guest-local volume-local instance from
Guest-local primitives only, never a leaked parent-Host dirfd or resource
handle.

### EphemeralProcess templates

Only genuine Volume-operation workers that the canonical Volume spec semantically
owns are declared. Effect-port-only actions (layout cleanup, store-sync) and
framework-owned sealing are executed directly via the `VolumeEffectPort` and do
not need a separate EphemeralProcess. All templates are resolved through the
signed Provider manifest; no free-form argv is accepted. Each template declares
canonical EphemeralProcess fields with `successfulTtl: "1h"` and
`failedTtl: "24h"`.

#### `volume-migration-worker`

Runs a schema migration operator binary against a staging Volume. Triggered by
`stateSchemaPhase: migration-required`. No raw paths cross the worker boundary;
the staging and source Volume views are delivered as `VolumeMountToken`s.

```yaml
type: EphemeralProcess
metadata:
  ownerRef: Volume/<name>
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  userRef: null
  processClass: worker
  template: volume-migration-worker
  configRef: null
  credentialRefs: []
  mounts:
    - volumeRef: Volume/<name>
      view: current
      mountPath: /work/current
      access: read-only
      required: true
    - volumeRef: Volume/<name>--staging
      view: staging
      mountPath: /work/staging
      access: read-write
      required: true
  sandbox:
    namespaceClasses: []
    capabilityClasses: []
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    environmentClass: minimal
    readOnlyRoot: true
    umask: "0022"
  budget: {}
  networkUsage: null
  deviceUsage: []
  telemetry: {}
  startDeadline: "60s"
  runtimeDeadline: "3600s"
  successfulTtl: "1h"
  failedTtl: "24h"
  incidentHold: false
```

#### `volume-snapshot-worker`

Creates a point-in-time snapshot copy of the Volume's active view. Triggered by
`snapshotPolicy` (pre-migration, pre-relocation, or manual). Result stored in
the Volume's provider-private `.snapshots/` subtree, accessible to the controller
via a provider-private view only.

```yaml
type: EphemeralProcess
metadata:
  ownerRef: Volume/<name>
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  userRef: null
  processClass: worker
  template: volume-snapshot-worker
  mounts:
    - volumeRef: Volume/<name>
      view: snapshot-source
      mountPath: /work/source
      access: read-only
      required: true
  sandbox:
    namespaceClasses: []
    capabilityClasses: []
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    environmentClass: minimal
    readOnlyRoot: true
    umask: "0022"
  budget: {}
  networkUsage: null
  deviceUsage: []
  telemetry: {}
  startDeadline: "60s"
  runtimeDeadline: "3600s"
  successfulTtl: "1h"
  failedTtl: "24h"
  incidentHold: false
```

#### `volume-relocation-worker`

Copies a source Volume tree to a destination Volume on relocation. Uses anchored
read/write on `VolumeMountToken`-delivered FDs; no raw paths.

```yaml
type: EphemeralProcess
metadata:
  ownerRef: Volume/<name>
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  userRef: null
  processClass: worker
  template: volume-relocation-worker
  mounts:
    - volumeRef: Volume/<name>
      view: relocation-source
      mountPath: /work/source
      access: read-only
      required: true
    - volumeRef: Volume/<destination-name>
      view: relocation-dest
      mountPath: /work/dest
      access: read-write
      required: true
  sandbox:
    namespaceClasses: []
    capabilityClasses: []
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    environmentClass: minimal
    readOnlyRoot: true
    umask: "0022"
  budget: {}
  networkUsage: null
  deviceUsage: []
  telemetry: {}
  startDeadline: "60s"
  runtimeDeadline: "7200s"
  successfulTtl: "1h"
  failedTtl: "24h"
  incidentHold: false
```

---

## Schema migration

### Pre-launch migration

A Volume whose `migrationPolicy: pre-launch-required` and
`stateSchemaPhase: migration-required` blocks the owning component's Process:

1. Controller detects `stateSchemaPhase: migration-required` on the Volume via
   watch.
2. Sets a `Migrating` condition on the Volume status.
3. Creates a staging Volume (`persistenceClass: ephemeral`, `ownerRef:
   Volume/<name>`).
4. Creates a `volume-migration-worker` EphemeralProcess with `ownerRef:
   Volume/<name>` and a signed migration template. The EphemeralProcess mounts
   the staging Volume under the `staging` view and the source Volume under the
   `current` view.
5. The migration operator binary:
   a. opens the Volume view via its declared mount;
   b. reads `installedSchemaVersion` from the marker;
   c. runs the schema-specific migration operator up to the target version
      (idempotent; deterministic; roll-forward only);
   d. writes the new marker with `installedSchemaVersion = target`;
   e. exits 0.
6. On `EphemeralProcess.status.phase = Succeeded`: commit staging → primary
   using `AtomicFilesystem.rename_into`; update `stateSchemaPhase: current`;
   delete staging Volume.
7. Component Process is unblocked.

### Online migration (`online-optional`)

The component Process starts while the EphemeralProcess migration runs
concurrently. The component observes the `MigrationPending` condition through
its ComponentSession service interface and switches schema layout after the
condition clears.

### Cross-component migration coordination

For N stateful components sharing a related schema:

**Prepare phase**:
1. Controller sets `PrepareMigration` condition on all N Volumes via
   `ResourceMutationBatch`.
2. All N component Processes acknowledge via ComponentSession and stop mutating
   their state views.
3. Each Process sets a `MigrationReady` condition on its own Process status.

**Staging**:
Each migrating Volume gets a staging Volume (`persistenceClass: ephemeral`,
`ownerRef: Volume/<source-name>`).

**Commit phase**:
1. All N migration EphemeralProcess workers complete with `Succeeded`.
2. Controller atomically swaps staging content into primary Volumes using
   `AtomicFilesystem.rename_into` (one per Volume; ordered parent-before-child).
3. Updates all N Volumes to `stateSchemaPhase: current`; removes
   `PrepareMigration` conditions.
4. Deletes staging Volumes.
5. Unblocks component Processes.

**Precommit rollback** (any EphemeralProcess reports `Failed` before commit):
1. Sets `MigrationAborted` condition on all N Volumes.
2. Deletes staging Volumes.
3. All N Volumes remain at `installedSchemaVersion`; `stateSchemaPhase:
   migration-failed`.
4. Online-optional Processes continue on old schema.

**Roll-forward after interrupted commit**:
On daemon restart, if `installedSchemaVersion == target` in the marker, the
controller corrects `stateSchemaPhase: current` during startup reconcile.
Orphan staging Volumes are GC'd under the unclaimed cleanup policy.

### Migration operator requirements

Migration operators must be:
- deterministic given the same source schema version and source data;
- idempotent (safe to re-run after crash at any point);
- roll-forward only (never downgrade data at or above target version).

---

## Secret sealing

A Volume with `stateSchema.sensitivityClass: private` and `sealingRequired:
true` requires a `sealingCredentialRef`:

```yaml
sealingCredentialRef: Credential/example-provider-state-key
```

The referenced Credential must be `Ready` before the Volume is provisioned.
The controller observes only Credential identity, readiness, and generation. It
never reads a Credential lease or obtains envelope-key material. The admitted
Volume binds `sealingCredentialRef` to an opaque `SealingPolicyId`; the trusted
broker resolves that policy and performs initial wrapping. Raw keys
remain inside the Credential authority and closed broker operation.

### Canonical sealing-key rotation

Credential generation advance and a sealing-policy change both use only
`VolumeEffectPort::rotate_sealing_key(RotateSealingKeyRequest)`. There is no
direct Credential-lease read, filesystem rewrite, generic broker call, or
Provider-owned `volume-sealing-rotation-worker` fallback.

The request fields and preconditions are exact:

- `volume` must resolve to the admitted Volume UID, still assigned to
  `Provider/volume-local` in the caller's Zone.
- `policy` must be the `SealingPolicyId` bound to the Volume's admitted
  `sealingCredentialRef`; a caller cannot substitute another policy.
- `expected_volume_generation` must match the desired-object generation that
  caused reconcile. `expected_resource_revision` must be the new revision
  returned by the committed status-first mutation described below.
- `expected_current_key_generation` must match the authenticated sealing marker
  currently active on disk. `target_key_generation` must be greater, `Ready`,
  permitted by the policy, and not revoked.
- `operation_id` must be the one in the corresponding
  `CommittedRevisionProof`. It is opaque and persisted by the core Operation
  ledger, not invented by the Provider.

Before any external effect, the controller commits, with expected revision, a
status update containing `sealingStatus: rotation-pending`,
`sealingKeyGeneration: <from>`, and
`sealingRotation: { fromGeneration, toGeneration }`, plus a
`SealingReady=False/rotation-pending` condition at the Volume generation. Only
the proof for that committed status revision permits the adapter call. A status
conflict causes re-read and re-plan without an effect. This status-first rule
applies equally to normal reconcile and `execute_upgrade`.

The core authorization owner authorizes the controller ComponentSession
principal for the closed `volume.rotate-sealing-key` capability, checks
same-Zone ownership and `providerRef`, and verifies the committed revision
proof. The pure adapter then maps the method one-to-one to the closed broker
operation `RotateSealingKey`. The broker independently repeats the capability,
Volume/policy binding, generation, and revision checks against its signed
private policy table. The broker request
contains only opaque `VolumeId`, opaque `SealingPolicyId`, generation/revision
preconditions, opaque `operation_id`, and the derived idempotency key. No key
bytes, Credential bytes, KDF parameters, host paths, relative paths, key
handles, or caller-selected file descriptors cross either boundary.

The idempotency key is:

```text
SHA-256(
  "d2b.volume.rotate-sealing-key.v1" ||
  volume-uid || sealing-policy-id ||
  expected-volume-generation || expected-resource-revision ||
  expected-current-key-generation || target-key-generation ||
  operation-id
)
```

Fields use their canonical length-prefixed wire encodings. Core and the broker
independently derive the key from the canonical typed request; the adapter only
maps the request. Reusing a key with different request bytes is
`IdempotencyConflict`. A retry of byte-identical request fields returns
`AlreadyCommitted` or `RecoveredCommitted` without another rewrite or duplicate
success audit. A distinct in-flight request for the same Volume is
`RotationConflict`; rotation is single-flight per Volume.

Result dispositions have one meaning:

| Disposition | Meaning |
| --- | --- |
| `Rotated` | This call performed and committed the fresh rotation |
| `AlreadyCommitted` | The broker dedup record already proved the same request, retired prior generation, and durable success audit |
| `RecoveredCommitted` | Crash recovery proved/switched the target, completed retirement and the missing durable audit, if any |

All three return the matching Volume generation and active target key
generation; no partial or merely staged state returns success.

The broker uses an anchored, fsync'd rotation journal keyed by the idempotency
key and a staged sealing generation:

1. `Prepared`: authorize and lease the expected and target policy generations;
   persist only their opaque policy/generation references and request digest,
   then durably commit the immutable privileged-effect audit intent before
   releasing the effect.
2. `Staged`: rewrap every sealed `StateEnvelope` into an anchored staging
   generation, verify every authentication tag, then fsync files and directory.
3. `Committed`: atomically switch authenticated sealing metadata to the target
   generation and fsync its parent. Readers see either the complete old
   generation or the complete target generation, never a mixed generation.
4. `Audited`: append and durably commit the exactly-once broker success audit,
   then retire the previous generation's staged data and complete the journal.

No journal contains key material or paths. A crash before the metadata switch
discards or resumes staging while the old generation remains authoritative. A
crash after the switch is roll-forward only: startup verifies the authenticated
target marker, completes the missing success audit under the same idempotency
key, retires the old generation, and returns `RecoveredCommitted`. The broker
never switches back to the old generation. If the audit sink is unavailable
after the data commit, it returns retryable `CommitPendingAudit`; the controller
keeps `rotation-pending` and retries the identical request until recovery and
audit complete.

`Rotated`, `AlreadyCommitted`, and `RecoveredCommitted` results are returned
only when the requested Volume generation still matches, the target generation
is active, the prior generation is retired, and the success audit is durable.
The controller then atomically writes `sealingStatus: sealed`, advances
`sealingKeyGeneration`, clears `sealingRotation`, and sets
`SealingReady=True/rotation-complete`. A generation/revision/policy
precondition error causes re-read/re-plan and leaves the status pending until
the new plan is committed. `TargetKeyUnavailable`, `ResourceExhausted`,
`BackendUnavailable`, `DeadlineExceeded`, and `CommitPendingAudit` are retryable
with the identical request and bounded exponential backoff with full jitter
(250 ms initial, 30 s cap). `Unauthorized`, `TargetKeyRevoked`,
`RotationConflict`, `IdempotencyConflict`, and `IntegrityViolation` are not
blindly retried; the controller records `rotation-failed` and a bounded,
path-free reason. Revocation or a new committed Resource generation requires a
new plan and operation ID.

| Typed error | Retry semantics | Controller/status action |
| --- | --- | --- |
| `Unauthorized` | Never retry automatically | `rotation-failed`; require policy/authorization correction |
| `PreconditionFailed` | Do not retry the same request | Re-read; commit a new plan or clear a stale pending plan |
| `TargetKeyUnavailable` | Retry identical request after hint/backoff | Keep `rotation-pending` |
| `TargetKeyRevoked` | Never retry that target | `rotation-failed`; require a newer admitted target |
| `RotationConflict` | Do not race or replace in-flight work | `rotation-failed`; re-read broker/resource state |
| `IdempotencyConflict` | Never retry | `rotation-failed`; integrity/operator investigation |
| `IntegrityViolation` | Never retry or auto-repair | Volume `Failed`; preserve evidence |
| `ResourceExhausted` | Retry identical request with backoff | Keep `rotation-pending` |
| `BackendUnavailable` | Retry identical request with backoff | Keep `rotation-pending` |
| `DeadlineExceeded` | Commit outcome is unknown; retry identical request | Keep `rotation-pending` |
| `CommitPendingAudit` | Data is committed; retry identical request until audit completes | Keep `rotation-pending`; do not report success |

On controller or daemon restart, the startup relist finds
`rotation-pending`, obtains the original operation ID/request fingerprint from
the core Operation ledger, and retries the identical typed request before any
new rotation plan. If status says `sealed` but the broker reports a different
active generation, reconcile sets `SealingReady=False/sealing-generation-drift`
and fails closed rather than issuing an implicit rewrite.

The controller submits closed start/failure/commit observations to the typed
audit port; it opens no writer. The broker durably completes exactly one
`RotateSealingKey` record with `PrivilegedEffectAuditDigest`, optional
`ResourceCorrelationDigest`, `OperationCorrelationDigest`, from/to
generations, and `RotateSealingKeyDisposition`. No raw Zone, Volume, resource,
policy, operation, or credential identity; credential bytes; key material; KDF
parameters; data counts/sizes; paths; or content enters status, audit, OTEL,
errors, `Debug`, or logs at any cardinality.

---

## Snapshots

```yaml
snapshotPolicy:
  retainCount: 3
  retainDurationHours: 168    # 7 days; 0 = retain only by count
  triggerOnMigration: true    # auto-snapshot before every migration
  triggerOnRelocation: true
```

Volume status snapshot list:

```yaml
snapshots:
  - id: snap-<opaque>
    createdAt: 2026-07-22T00:00:00.000Z
    schemaVersion: "1.0"
    sizeBytes: 12345678
    trigger: pre-migration
    phase: Ready     # Ready | Failed | Expired
```

Implementation:
- Stored in a Provider-private path under the Volume root (`.snapshots/`);
  never exposed through component views.
- Created by a `volume-snapshot-worker` EphemeralProcess.
- Retained by `retainCount` and `retainDurationHours`; expired snapshots are
  removed by the controller.
- `sizeBytes` is an informational estimate; not a quota-deduction input.
- Snapshot `id` is opaque (not a path or generation number).

---

## Staging Volumes

Staging Volumes have `persistenceClass: ephemeral` and `ownerRef` pointing to
the parent Volume:

```yaml
type: Volume
metadata:
  name: <source-volume-name>--staging
  ownerRef: Volume/<source-volume-name>
spec:
  providerRef: Provider/volume-local
  persistenceClass: ephemeral
  source:
    executionRef: Host/host-system
```

Lifecycle rules:
- Mounted only by migration/snapshot/relocation EphemeralProcess resources.
- Removed on successful commit or failed rollback before component is unblocked.
- Detected as unclaimed and GC'd if owning Provider is removed before cleanup.

---

## Relocation

State relocation moves a Volume's backing store to another Host:

1. Controller sets `Relocating` finalizer on source Volume; stops component
   Processes that mount it.
2. Creates destination Volume (same Zone; different Host).
3. Creates `volume-relocation-worker` EphemeralProcess anchored at both source
   and destination Volume root FDs; copies tree using `read`/`write` on
   anchored FDs, never on paths.
4. On success: controller mounts destination Volume in place of source; removes
   source finalizer; deletes source Volume.
5. On failure: source Volume and finalizer remain; operator resolves.

The attachment Volume backed by `Provider/volume-virtiofs` is re-pointed to the
new source after copy; the re-point protocol is governed by
`ADR-046-primitive-resource-composition`.

---

## Retention, incident hold, unclaimed GC, and destruction

### Retention policy

```yaml
retentionPolicy:
  successfulTtlHours: 1
  failedTtlHours: 24
  incidentHoldEnabled: true
```

`persistent` Volumes are never auto-expired; deleted only by explicit controller
request with expected revision.

### Incident hold

```yaml
conditions:
  - type: IncidentHold
    status: "True"
    reason: active-incident
    message: bounded operator description      # ≤512 bytes; no paths
    observedGeneration: 3
```

- Set by an authorized administrative Role via the status subresource.
- Blocks `deletionRequestedAt` processing, migration commit, and staging removal.
- Does not block read-only mounts or status observation.
- Cleared only by the same administrative Role.
- Preserved through daemon restart and Zone reconcile.

### Unclaimed Volume GC

A Volume is unclaimed when:
- `metadata.ownerRef` resolves to a deleted Provider; or
- Provider's component descriptor no longer declares a stateNamespace matching this Volume's `stateSchema.schemaId` (Volume is no longer in the Provider's ProviderStateSet by design); or
- it is a staging Volume whose owning Volume has been deleted.

Unclaimed `persistent` Volumes:
1. Receive an `Unclaimed` condition.
2. Are reported in Zone status.
3. Are not automatically deleted; operator confirms via explicit delete.

Unclaimed `ephemeral` and staging Volumes are automatically deleted after the
Zone's configured unclaimed TTL (default 1 h).

### Destruction sequence

1. Set `deletionRequestedAt` via resource API.
2. Controller sets `volume-local/layout` finalizer and drains all outstanding
   `VolumeMountToken` handles (waits for all mounting Processes to stop).
3. Controller sends `CleanupLayoutEntry` effect requests to `VolumeEffectPort`
   for each layout entry ordered leaf-first then parent-last; adapter performs
   `unlinkat` anchored at the Volume root FD with `fsync` on the parent
   directory FD after each removal.
4. Where `sensitivityClass: private` and `sealingRequired: true`: controller
   sends a key-shred effect request before layout removal.
5. Controller sends `CleanupMarker` effect request.
6. Controller sends `CleanupVolumeRoot` effect request.
7. Controller clears the `volume-local/layout` finalizer; audit of each step
   is broker/core-owned and is never atomic with any redb write.
8. Core emits `Deleted` event and removes resource row.

Partial removal detected on restart by marker check; partially removed Volume
with valid marker is quarantined rather than silently re-provisioned.

---

## Within-Volume transactions

Provider components commit structured state through
`VolumeEffectPort::commit_volume_transaction`, a pure mapping to the closed
`CommitVolumeTransaction` broker operation:

1. The Provider serializes a bounded payload as canonical JSON and wraps it in
   `StateEnvelope` with digest and generation counter. The request identifies a
   declared state slot by opaque ID, never by path.
2. The broker resolves the slot and opens a temporary file in the anchored
   Volume root with `O_CLOEXEC | O_TMPFILE`.
3. The broker performs the bounded write and calls `fsync` on the temporary fd.
4. The broker links/renames the temporary fd into the broker-resolved target.
5. The broker calls `fsync` on the parent directory, durably completes the
   effect audit, and returns the typed disposition.

A crash between steps 3 and 5 leaves the old file intact. The toolkit validates
the `StateEnvelope` digest and generation bound before exposing payload.

No cross-Volume, cross-process, or cross-schema transaction is defined.
Multi-object consistency uses the cross-component migration protocol.

---

## Async reconciliation and blocking-thread adapter

Volume reconciliation follows `ADR-046-resource-reconciliation`:

1. On Volume spec `create`/`update`, volume-local receives
   `spec-generation-changed`.
2. Controller reads current Volume spec and evaluates all layout entries.
3. For each entry, resolves `ownerRef`/`groupRef` User UID/GID via Zone User
   resource watch and derives `UserId` opaque handles.
4. Sends `provision_layout_entry` or `repair_layout_entry` effect op to the
   injected `VolumeEffectPort`; concurrently dispatches per-resource effect calls
   while the watch loop remains responsive to new events.
5. Writes status batch with expected revision; conflict → re-read/retry.
6. Translate each virtiofs attachment into one
   `virtiofs.d2bus.org.Export` owned by the Volume; diff the desired Export set
   and let volume-virtiofs reconcile each Export independently.

Credential watches join the same per-Volume single-flight. An observed sealing
generation advance first commits the `rotation-pending` status transition, then
calls the typed `rotate_sealing_key` method with the resulting committed proof.
Restart, timeout, cancellation, and retry reuse the operation ID/request
fingerprint from the core Operation ledger. Reconcile never obtains keys or
performs a direct rewrite, and it never starts a newer rotation while an older
status-first operation is pending.

Owner triggers: every Volume spec/status/finalizer mutation produces an
`owned-resource-changed` hint for the Volume's `ownerRef` (typically a Guest).

External drift observation: volume-local declares a bounded observe interval
(default 60 s) for `durable`/`state` Volumes. `ephemeral`/`tmp` Volumes observe
only on start.

### Blocking syscalls in broker handlers

All blocking FS syscalls (`openat2`, `fstatat`, `acl_get_fd`, `acl_set_fd`,
`unlinkat`, `linkat`, `fsync`, `read`, `write`, `statfs`, `mount`, `umount2`,
`fallocate`) execute only in closed broker handlers and are dispatched to the
broker's bounded blocking thread pool (tokio `spawn_blocking` equivalent). No
broker async handler holds a blocking syscall across an `await`. The
`VolumeEffectPort` trait's `async fn` signatures expose the pure typed dispatch
interface to the controller; the adapter itself performs no blocking work.

Async sub-operations (migration, snapshot, relocation EphemeralProcess
coordination) carry a cancellation token derived from the ComponentSession so
that controller shutdown propagates atomically.

---

## d2b-bus/ComponentSession integration

The controller uses the Zone d2b-bus `ResourceClient` provided via its
ComponentSession for:

- `watch(Volume, providerRef: Provider/volume-local)` - observe all served
  Volumes (spec/status changes trigger layout/ACL/quota/marker reconciliation);
  ProviderDeployment creates and deletes Volume instances; volume-local
  reconciles physical state only and does not issue create/delete API calls
  for these Volumes;
- `update-status(Volume, with-expected-revision)` - write layout conditions;
- `create(EphemeralProcess)` - dispatch migration/snapshot/relocation workers;
- `watch(EphemeralProcess)` - observe worker completion;
- `watch(User)` - observe UID binding changes for ACL re-resolve;
- `watch(Credential)` - observe sealing key rotation;
- `update-finalizers(Volume)` - manage `volume-local/layout` finalizer.

No direct d2b-bus protocol details are specified here; they are governed by
`ADR-046-componentsession-and-bus`. The controller's session purpose is
`volume-local/controller`, authenticated via the Zone-issued controller
bootstrap token (IKpsk2 profile).

Cross-references to main `a1cc0b2d` symbols used by volume-local (per
`ADR-046-provider-state` §Bus/ComponentSession cross-reference):

- `Cancellation` / `RequestRegistry` from `d2b-session/src/cancellation.rs`:
  used by migration, snapshot, and relocation EphemeralProcess coordination.
- `OwnedAttachment` / `AttachmentPayload` from `d2b-session/src/attachment.rs`:
  used by volume-local to deliver a worker subview `VolumeMountToken` to
  requesting Processes.
- `Fixture` / `FakeProvider` / `DeterministicClock` from
  `d2b-provider-toolkit/src/fixture.rs`: used by `tests/` as the fake Zone
  runtime; no live daemon required.
- `VolumeEffectPort` trait from `d2b-contracts/src/v3/effect_port.rs`: imported
  by the Provider crate; implemented by the pure core adapter, which maps each
  method to one closed broker operation.

---

## RBAC

```yaml
# Controller role - creates and manages Volumes
rules:
  - resourceTypes: [Volume, EphemeralProcess]
    verbs: [create, update-spec, update-status, update-finalizers, get, list, watch, delete]
    zones: [dev]
  - resourceTypes: [User]
    verbs: [get, watch]
    zones: [dev]
  - resourceTypes: [Credential]
    verbs: [get, watch]
    zones: [dev]

# Process mounting read-only
rules:
  - resourceTypes: [Volume]
    verbs: [get]
    zones: [dev]

# Guest Provider reading Volumes
rules:
  - resourceTypes: [Volume]
    verbs: [get, list, watch]
    zones: [dev]
    executionRefs: [Guest/work-vm]
```

No special host-path permission claims. The controller process holds no claim
that grants access to raw host paths; path resolution is performed exclusively
inside the privileged broker. The `VolumeEffectPort` adapter performs no path
resolution or host syscall.

No subject may write spec for a Volume they do not own. Status may be written
only by the current controller lease for the declared `providerRef`.
`sourcePolicyId` in `source.settings` is validated against the Provider's
declared `sourcePolicies` at Volume admission time; no host path string is
accepted in any Volume spec field.

---

## Status fields and conditions

### Provider controller status

The `Provider/volume-local` controller itself uses only the common `phase` field
(`Pending`/`Ready`/`Degraded`/`Failed`/`Unknown`). Individual Volume resources
managed by the controller follow D088: Volume attachment base, layout phase,
marker base, quota base, and sealing/schema base are promoted to the
ResourceType-common `status.resource` shape shared by all Volume implementations.
Local filesystem-specific marker/quota/snapshot/migration observations live only
in `status.provider.details` with `providerRef: Provider/volume-local`,
qualified `schemaId` (`volume-local.d2bus.org/Volume/status`), `schemaVersion`, and
`observedProviderGeneration`. The controller writes all present layers atomically
in one status mutation; shared fields are never duplicated into
`status.provider`, and the strict, ≤32 KiB, redacted extension schema is
registered and signed in the Provider manifest. The controller watch loop remains
responsive to new Volume spec changes while per-resource effect calls run
concurrently.

### Currency and expedited reconcile (D091/D090)

D091 currency is universal status, not volume-local provider detail. The
controller implements `assess_update`, `plan_upgrade`, and `execute_upgrade`,
populates universal `status.update`, and keeps shared currency fields out of
`status.provider`; filesystem-specific observations may appear only under
`status.provider.details`. Provider/controller generation, schema, spec, or
security-policy changes that require disruption MUST set `status.update.state =
UpgradeRequired`, with `reasons = [ProviderGenerationChanged]`, `[SpecChanged]`,
`[ArtifactChanged]`, or `[SecurityPolicyChanged]`, `disruption = Recycle|Replace`,
and `preserveState = true` rather than applying disruption in place.
Non-disruptive changes reconcile normally. Durable and state Volumes are
preserved across any upgrade; volume-local's own controller upgrade recycles
only the controller `Process`, and `Replace` of a Volume row is allowed only
with explicit ownership/state transfer. No raw host path or secret enters
`status.update`.

A Credential or sealing-policy generation change is a non-disruptive,
state-preserving upgrade step, but not an in-place implicit rewrite:
`plan_upgrade` records the expected current/target sealing generations and
`execute_upgrade` commits `rotation-pending` status before invoking the same
typed `VolumeEffectPort::rotate_sealing_key` operation used by reconcile. An
upgrade completes only after its result and durable broker audit are reflected
in `sealed` status. Restart resumes the original idempotency key; a changed
Resource generation causes re-plan rather than reuse under different
preconditions.

D090 expedited `waitForReconcile` on `Create`/`UpdateSpec`/`Delete` performs no
external effect, finalizer change, or status mutation until Core supplies
`CommittedRevisionProof {resourceUid,generation,revision,operationId}`. The
one-pass response returns the committed object, projected layered status,
disposition `Converged|Progressing|Blocked|UpgradeRequired|Failed`, and
`statusPersistence = pending|committed`; the durable commit is never rolled back
after a reconcile timeout. Effect idempotency keys derive from
`(UID,generation,revision,operationId)`, and the expedited pass uses the bounded
priority lane inside the same per-resource single-flight.

### Core Volume status

```yaml
status:
  observedGeneration: 1
  phase: Ready         # Pending | Ready | Degraded | Failed | Unknown
  conditions:
    - type: LayoutReady
      status: "True" | "False" | "Unknown"
      reason: layout-reconciled | layout-error | layout-pending
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:00.000Z
    - type: AttachmentsReady
      status: "True"
      reason: all-attachments-ready
      observedGeneration: 1
    - type: SealingReady
      status: "True"
      reason: rotation-complete
      observedGeneration: 1
  resource:
    layoutPhase: Ready    # Pending | Ready | Degraded | Failed
    layoutConditions: []  # per-entry: EntryMissing | EntryDrift | EntryQuarantined | InvariantViolated | ForeignAclViolation
    attachmentStatuses: []
    stateSchemaPhase: current
    markerStatus: verified
    sealingStatus: sealed
    sealingKeyGeneration: 2
    sealingRotation: null
    quotaUsage: { usedBytes: 0, inodeCount: 0 }
  provider:
    providerRef: Provider/volume-local
    schemaId: volume-local.d2bus.org/Volume/status
    schemaVersion: 1.0.0
    observedProviderGeneration: 1
    details:
      localMarkerObservation: verified
      quotaBackend: project-quota
      snapshots: []
```

### State-extension fields

The following fields are `status.resource` base fields unless noted; provider-
specific filesystem observations remain in `status.provider.details` and never
carry raw host paths, secret bytes, or unbounded records.

| Field | Values | Notes |
| --- | --- | --- |
| `stateSchemaPhase` | `current`, `migration-required`, `migrating`, `migration-committed`, `migration-failed` | Blocks pre-launch Processes when not `current` |
| `installedSchemaVersion` | semver string or null | Version on disk at last verified marker read |
| `markerStatus` | `verified`, `missing`, `replaced`, `tampered`, `unknown` | Non-`verified` blocks pre-launch Processes |
| `sealingStatus` | `none`, `sealed`, `rotation-pending`, `rotation-failed` | The committed status-first gate for `rotate_sealing_key`; `rotation-pending` is durable before any effect |
| `sealingKeyGeneration` | integer or null | Last broker-confirmed active generation; never inferred only from Credential status |
| `sealingRotation` | `{ fromGeneration, toGeneration }` or null | Safe restart/reconcile projection; operation ID and idempotency key remain only in the core Operation ledger |
| `quotaUsage` | `{ usedBytes: N, inodeCount: N }` or null | Polled at max 60 s intervals |
| `lastMigrationAt` | RFC 3339 UTC timestamp or null | |
| `snapshots` | `SnapshotRecord[]` | Bounded list; each record: `id` (opaque), `createdAt`, `schemaVersion`, `sizeBytes`, `trigger`, `phase` |

`stateSchemaPhase != current` and `markerStatus != verified` both block a
`pre-launch-required` component Process from moving to `Ready`.

---

## Error catalog

| Error code | Trigger | Phase result |
| --- | --- | --- |
| `volume-source-policy-not-found` | `sourcePolicyId` does not match any declared source policy for the Volume's `kind` | Failed |
| `volume-domain-mismatch` | Mounting Process domain/userRef incompatible with Volume `sensitivityClass` | Process launch rejected |
| `volume-view-rights-exceeded` | Mount requests rights absent from declared View | Process launch rejected |
| `volume-quota-exceeded` | Write attempted at or above quota | Write rejected |
| `quota-insufficient` | Backing FS cannot reserve declared `quotaBytes` | Failed |
| `quota-mismatch` | Descriptor `quotaBytes` != Volume spec `quotaBytes` | Volume admission rejected |
| `storage-drift` | `st_dev` mismatch for same-filesystem invariant | Failed |
| `previously-provisioned-swtpm-state-missing` | TPM state absent after marker written | Failed |
| `foreign-acl-violation` | `foreignChildPolicy: fail` with unlisted ACL entry | Degraded |
| `invariant-violated` | `no-symlink`, `no-magic-link`, or `same-filesystem` invariant failed | Failed |
| `entry-quarantined` | Adoption ambiguity | Degraded |
| `volume-local/layout` finalizer timeout | Controller exceeded `maxFinalizerDurationSeconds` | Degraded/finalizer-timeout |
| `marker-tampered` | Marker HMAC validation failed | Failed |
| `volume-effect-audit-pending` | A non-sealing broker effect committed but durable completion audit is not yet confirmed | Operation remains pending; retry the byte-identical request |
| `sealing-rotation-unauthorized` | Caller lacks the closed `volume.rotate-sealing-key` capability or Volume/policy binding | rotation-failed |
| `sealing-rotation-precondition` | Volume generation, Resource revision, policy binding, or active/target key generation no longer matches | rotation-pending; re-read/re-plan |
| `sealing-target-unavailable` | Target policy generation is not yet available | rotation-pending; retry same request |
| `sealing-target-revoked` | Target policy generation is revoked | rotation-failed |
| `sealing-rotation-conflict` | A distinct rotation is already in flight for the Volume | rotation-failed |
| `sealing-idempotency-conflict` | One idempotency key is presented with different canonical request bytes | rotation-failed |
| `sealing-integrity-violation` | Existing envelope, marker, staging generation, or journal fails authentication | Failed |
| `sealing-rotation-retryable` | Resource exhaustion, backend unavailability, or deadline before known commit | rotation-pending; retry same request |
| `sealing-audit-pending` | Target committed but durable success audit is not yet confirmed | rotation-pending; retry same request |

All error messages are bounded (512 bytes), UTF-8/control-character validated,
and must not contain host paths, secret content, process data, or terminal bytes.

---

## Audit events and redaction

### Audit event table

The Provider opens no audit writer. It submits closed lifecycle observations to
the T609-owned typed audit port; the resource/operation owner decides and
durably writes the authoritative record. The Zone selects the private
partition and is never a serialized field.

| Event kind | Authoritative owner and trigger | Required payload fields |
| --- | --- | --- |
| `volume-provisioned` | Core resource lifecycle owner after first provision completes | `ResourceCorrelationDigest`, `PersistenceClass`, `SourceKind`, `AuditOutcome` |
| `volume-layout-repaired` | Broker after `RepairLayoutEntry` completes durably | `PrivilegedEffectAuditDigest`, optional `ResourceCorrelationDigest`, `EntryType`, `RepairActionClass`, `AuditOutcome` |
| `volume-migration-start` | Core operation owner after migration operation commit | `ResourceCorrelationDigest`, `OperationCorrelationDigest`, from/to version, `MigrationPolicy`, `AuditOutcome` |
| `volume-migration-committed` | Core operation owner after migration commit | `ResourceCorrelationDigest`, `OperationCorrelationDigest`, from/to version, `AuditOutcome` |
| `volume-migration-failed` | Core operation owner after terminal migration failure | `ResourceCorrelationDigest`, `OperationCorrelationDigest`, from/to version, `VolumeFailureReason` |
| `volume-migration-rolled-back` | Core operation owner after precommit rollback | `ResourceCorrelationDigest`, `OperationCorrelationDigest`, `AuditOutcome` |
| `volume-snapshot-created` | Core operation owner after snapshot commit | `ResourceCorrelationDigest`, `OperationCorrelationDigest`, `SnapshotTrigger`, `AuditOutcome` |
| `volume-relocation-start` | Core operation owner after relocation operation commit | `ResourceCorrelationDigest`, `OperationCorrelationDigest`, `ExecutionCorrelationDigest` for source, `AuditOutcome` |
| `volume-relocation-committed` | Core operation owner after relocation commit | `ResourceCorrelationDigest`, `OperationCorrelationDigest`, `ExecutionCorrelationDigest` for destination, `AuditOutcome` |
| `volume-incident-hold-set` | Core authorization/resource owner after condition commit | `ResourceCorrelationDigest`, `SubjectCorrelationDigest`, `IncidentHoldAction::Set`, `AuditOutcome` |
| `volume-incident-hold-cleared` | Core authorization/resource owner after condition commit | `ResourceCorrelationDigest`, `SubjectCorrelationDigest`, `IncidentHoldAction::Clear`, `AuditOutcome` |
| `volume-sealing-rotation-start` | Core operation owner after status-first pending commit | `ResourceCorrelationDigest`, `OperationCorrelationDigest`, from/to generation, `AuditOutcome` |
| `volume-sealing-rotation-failed` | Core operation owner after terminal typed result | `ResourceCorrelationDigest`, `OperationCorrelationDigest`, from/to generation, `VolumeFailureReason` |
| `volume-sealing-rotation-committed` | Core operation owner after broker durable completion is confirmed | `ResourceCorrelationDigest`, `OperationCorrelationDigest`, from/to generation, `RotateSealingKeyDisposition` |
| `volume-destroyed` | Core resource lifecycle owner after finalizer completion | `ResourceCorrelationDigest`, `AuditOutcome` |
| `volume-marker-check` | Broker after `VerifyVolumeMarker` completes durably | `PrivilegedEffectAuditDigest`, optional `ResourceCorrelationDigest`, `MarkerStatus` |
| `volume-quota-exceeded` | Core resource owner after typed write denial | `ResourceCorrelationDigest`, `QuotaOutcome::Exceeded` |
| `volume-store-sync-complete` | Broker after `StoreSyncComplete` completes durably | `PrivilegedEffectAuditDigest`, optional `ResourceCorrelationDigest`, generation number, `StoreSyncDisposition` |
| Every other `VolumeBrokerOperation` variant | Broker before effect release and after terminal completion | `PrivilegedEffectAuditDigest`, optional `ResourceCorrelationDigest`, `VolumeEffectClass`, closed outcome/reason, and only the additional closed fields in the broker catalogue |

### Excluded from all audit records

The following are explicitly prohibited from appearing in any audit record,
status field, log line, OTEL span attribute, or metric label:

- raw Zone, Volume, resource, user, Process, policy, snapshot, operation, or
  execution identity;
- `ZonePath`, `ResourceRef`, `ResourceUid`, UID/GID, a Provider-local
  `VolumeAuditDigest`, or a general digest string;
- `source.settings.sourcePolicyId` resolved host path (the path exists only in the private bundle, never in any public field or record);
- layout entry relative paths;
- ACL grant values or permission strings;
- Volume root or staging path;
- virtiofsd export socket path;
- migration data or snapshot content;
- credential or key material;
- process argv, environment, or stdout/stderr bytes;
- `st_dev` / `st_ino` raw values;
- `sizeBytes` for `secret-adjacent` Volumes.

A required audit join uses only the approved correlation newtype for that
record class. A digest is absent when the join is unnecessary.

---

## OTEL metrics

`d2b-contracts::METRIC_DESCRIPTOR_REGISTRY` owns the descriptors, complete
label value domains, aggregation scopes, and buckets. The table below is the
exact row set volume-local requests from that registry; it is not a
Provider-local descriptor authority. Label values are closed enums. Raw
identity, ResourceRefs, names, handles, paths, schema IDs, view IDs, and every
correlation digest are forbidden from metric labels and OTEL Resource
attributes.

| Metric | Unit | Labels |
| --- | --- | --- |
| `d2b_volume_provision_total` | Counter | provider, persistence_class, source_kind, outcome |
| `d2b_volume_provision_duration_ms` | Histogram | provider, source_kind |
| `d2b_volume_layout_repair_total` | Counter | provider, outcome |
| `d2b_volume_state_size_bytes` | Histogram | provider, source_kind |
| `d2b_volume_state_migration_total` | Counter | provider, outcome |
| `d2b_volume_state_migration_duration_ms` | Histogram | provider |
| `d2b_volume_state_snapshot_total` | Counter | provider, trigger |
| `d2b_volume_state_marker_check_total` | Counter | provider, outcome |
| `d2b_volume_state_quota_exceeded_total` | Counter | provider |
| `d2b_volume_store_sync_total` | Counter | provider, outcome |
| `d2b_volume_store_sync_duration_ms` | Histogram | provider |
| `d2b_volume_relocation_total` | Counter | provider, outcome |
| `d2b_volume_sealing_rotation_total` | Counter | provider, outcome |
| `d2b_volume_unclaimed_gc_total` | Counter | provider, persistence_class |
| `d2b_volume_fd_handoff_total` | Counter | provider, access, outcome |

The only OTEL Resource attributes are the central fixed
`service.name`, `service.version`, `d2b.provider`, and `d2b.component`
semantic/build classes. A matching operation span may carry only
`OperationCorrelationDigest`; parent/child linkage uses only
`TraceCorrelationDigest` and `SpanCorrelationDigest`. Every admitted span has
one terminal transition. Metric, trace, and log buffers enforce byte bounds and
the central 60-second, 300-second, and 120-second age ceilings respectively.
Telemetry failure updates only the bounded closed diagnostic accumulator and
never changes a Volume operation.

---

## Nix configuration

### Artifact catalog

```nix
d2b.artifacts."volume-local-provider" = {
  package = pkgs.d2b-provider-volume-local;
  type = "provider";
};
```

Store paths are private catalog data; they never appear in any ResourceSpec,
status field, or audit record.

### Provider resource

```nix
d2b.zones."dev".resources."volume-local" = {
  type = "Provider";
  spec = {
    artifactId = "volume-local-provider";
    config = {
      controllerExecutionRef = "Host/host-system";
      # Root config validated against volume-local root-config.schema.json.
      # Raw host path prefixes are private bundle authority; they are NOT in this
      # config block. sourcePolicies declares opaque IDs that Volumes reference.
      sourcePolicies = [
        { id = "default-state";     class = "local-path";  volumeKinds = [ "durable" "state" "cache" ]; }
        { id = "default-ephemeral"; class = "local-path";  volumeKinds = [ "ephemeral" "tmp" ]; }
        { id = "default-tmpfs";     class = "tmpfs";       volumeKinds = [ "ephemeral" "tmp" ]; }
        { id = "default-block";     class = "block-image"; volumeKinds = [ "durable" "ephemeral" ]; }
      ];
      # No secrets in Provider root config; any credential must use Credential refs.
    };
  };
};
```

### Volume resource - minimal state Volume

```nix
d2b.zones."dev".resources."work-state" = {
  type = "Volume";
  spec = {
    providerRef = "Provider/volume-local";
    source = {
      executionRef = "Host/host-system";
      settings = {
        kind = "local-path";
        sourcePolicyId = "default-state";  # must match a declared source policy
      };
    };
    kind = "state";
    layout = [
      {
        path = "";
        type = "directory";
        ownerRef = "User/d2b-work-vm-runner";
        groupRef = "User/d2b-work-vm-runner";
        mode = "0700";
        sensitivity = "private";
        createPolicy = "create-if-never-provisioned";
        repairPolicy = "fail-closed";
        cleanupPolicy = "never";
      }
    ];
    views.controller = {
      path = "";
      rights = [ "read" "write" "create" "delete" "traverse" ];
    };
  };
};
```

### Volume resource - block-image

```nix
d2b.zones."dev".resources."work-disk" = {
  type = "Volume";
  spec = {
    providerRef = "Provider/volume-local";
    source = {
      executionRef = "Host/host-system";
      settings = {
        kind = "block-image";
        sourcePolicyId = "default-block";  # must match a declared source policy
        imageFormat = "raw";
        preallocate = false;
      };
    };
    kind = "durable";
    layout = [
      { path = ""; type = "directory"; ownerRef = "User/d2bd"; groupRef = "User/d2bd"; mode = "0700"; }
    ];
    views.controller = { path = ""; rights = [ "read" "write" "create" "delete" "traverse" ]; };
    quota = { maxBytes = 21474836480; enforcement = "hard"; };  # 20 GiB
  };
};
```

### Volume resource - tmpfs

```nix
d2b.zones."dev".resources."work-tmp" = {
  type = "Volume";
  spec = {
    providerRef = "Provider/volume-local";
    source = {
      executionRef = "Host/host-system";
      settings = {
        kind = "tmpfs";
        sourcePolicyId = "default-tmpfs";  # must match a declared source policy
      };
    };
    kind = "tmp";
    layout = [
      { path = ""; type = "directory"; ownerRef = "User/d2b-work-vm-runner"; groupRef = "User/d2b-work-vm-runner"; mode = "0700"; }
    ];
    views.controller = { path = ""; rights = [ "read" "write" "create" "delete" "traverse" ]; };
    quota = { maxBytes = 104857600; maxInodes = 10000; enforcement = "hard"; };
  };
};
```

### Eval-time validation rules specific to volume-local

1. `source.settings.hostPath` must not appear in any Volume spec; it is
   forbidden in Nix source and fails eval with `operator-supplied-host-path`.
   The private bundle records the mapping from `sourcePolicyId` to host path
   prefix; that mapping is injected by the Nix resource compiler and is not
   operator-authored.
2. `source.settings.sourcePolicyId` must match one of the IDs declared in the
   Provider's `sourcePolicies` list. A non-matching ID fails eval with
   `volume-source-policy-not-found`.
3. The `class` of the matched source policy must be compatible with
   `source.settings.kind`; a mismatch fails eval.
4. `source.settings.kind == "block-image"` requires `quota.maxBytes != null`.
5. `source.settings.kind == "tmpfs"` requires both `quota.maxBytes` and
   `quota.maxInodes`.
6. `source.settings.kind == "tmpfs"` with any `layout` entry using
   `createPolicy: create-if-never-provisioned` or
   `restartPolicy: preserve-across-controller-restart` fails eval.
7. Every `layout[].ownerRef` and `layout[].groupRef` must resolve to a `User`
   resource in the same Zone.
8. Every `accessAcl[].principal.ref` and `defaultAcl[].principal.ref` must
   resolve to a `User` resource.
9. `sourcePolicies` in Provider root config must not be empty; at least one
   entry is required.
10. Two Volumes in the same Zone referencing the same `sourcePolicyId` must not
    produce overlapping resolved paths; the resource compiler detects overlap
    and fails eval.
11. Any Volume with a `stateSchema` block (a declared component state Volume)
    must have `kind: state`, `persistenceClass: persistent`,
    `quota.maxBytes > 0`, and `quota.maxInodes > 0`. Specifying
    `kind: ephemeral`, `kind: tmp`, `persistenceClass: ephemeral`,
    `quota.maxBytes: 0`, or omitting `quota.maxInodes` in a stateSchema Volume
    fails eval with `invalid-provider-state-namespace`. A component declares a
    state Volume only when its payload passes the D087 storage-need test; there
    is no empty identity-only state Volume.

---

## d2b-state reuse plan

Main-branch `d2b-state` at commit `6faa5256` is the primary reuse source.
This code is absent from the v3 baseline. Selected symbols:

| Main symbol | Main path | Reuse action | v3 destination | Adaptation notes |
| --- | --- | --- | --- | --- |
| `AtomicFilesystem`, `RealAtomicFilesystem`, `AtomicWrite`, `CanonicalJson`, `DurableState`, `QuarantineRecord`, `GenerationPolicy`, `MetadataExpectation`, `ReadPolicy`, `WritePolicy` | `packages/d2b-state/src/atomic.rs` | adapt | Provider-side canonical request builder in `packages/d2b-provider-volume-local/src/atomic.rs`; filesystem implementation in the closed broker `CommitVolumeTransaction` handler | Replace ADR 0045 `v2_state` imports with v3 `StateEnvelope`; retain ordering semantics while moving every open/write/link/rename/fsync effect and durable audit into the broker |
| `AnchoredDir`, `AnchoredResource`, `LeafName`, `RelativePath` | `packages/d2b-state/src/path.rs` | adapt | Broker-private Volume path resolver; Provider keeps opaque `LayoutEntryId`/state-slot bindings only | Do not copy raw path-resolution types into the Provider; retain validation and anchored-resolution algorithms behind closed broker operations |
| `LockGuard`, `LockSet`, `OfdTransfer`, `Cancellation`, `Clock`, `NeverCancelled`, `SystemClock` | `packages/d2b-state/src/lock.rs` | adapt | Broker-private Volume operation handlers plus Provider cancellation/clock abstractions | Replace `v2_state` LockSpec/ResourceId with v3 typed lock IDs; OFD lock/CLOEXEC/fd-transfer effects remain broker-owned |
| `LeaseStatus`, `grant_lease`, `revoke_lease`, `validate_lease` | `packages/d2b-state/src/lease.rs` | adapt | `packages/d2b-contracts/src/v3/state_lease.rs` | Map to v3 Volume Credential rotation protocol; retain expiry/revocation semantics |
| `AuditAppender`, `AuditRecordInput`, `SegmentBuilder`, `checkpoint`, `decide_retention`, `detect_gap`, `read_audit_segment` | `packages/d2b-state/src/audit.rs` | adapt | T609-owned `d2b-audit`; `packages/d2b-provider-volume-local/src/audit.rs` keeps closed observation construction only | Provider opens no writer and chooses no durability; retain hash-chain/gap behavior only in the authoritative audit owner |
| Integration tests | `packages/d2b-state/tests/{state,async_state}.rs` | adapt | Provider fake-port tests plus broker Volume-operation integration tests | Retain crash/fault ordering while proving pure Provider mapping, broker-only syscalls, intent-before-effect, and durable exactly-once completion |

Excluded ADR 0045 assumptions:
- `d2b-contracts::v2_state` envelope schema and authority refs;
- `AuthorityRef`/`OwnershipEpoch`/`v2_state::ResourceId` semantics;
- ADR 0045 broker-operation IDs embedded in `LockSpec.resource_id`;
- the `tokio_api` feature's dependency on the ADR 0045 broker transport.

---

## Current-code fit

| Item | Evidence class | Treatment |
| --- | --- | --- |
| Current `packages/d2b-provider-volume-local/src/{effect_port,audit,otel}.rs` after the dossier baseline | `implemented-and-reachable` partial scaffold | Keep as current-code evidence only: `VolumeStateEffectPort` is not the closed W6 port, raw `ZonePath`/`ResourceRef` audit fields violate Version 3, and Provider-local metric descriptors violate central registry ownership. `ADR046-vl-001`, `ADR046-vl-009`, and `ADR046-vl-012` replace these prospectively; no historical work-item state changes. |
| `d2b-core/src/storage.rs`: `StorageJson`, `StoragePathSpec`, all policy enums (`CleanupPolicy`, `RepairPolicy`, `StorageRestartPolicy`, `StorageAdoptionPolicy`, `LeaseClass`, `SensitivityClass`, `StorageInvariant`, `StoragePathKind`, `PrincipalRef`, `ActorRef`, `AclGrant`) | `generated-or-eval-contract` | Extract and adapt to Volume LayoutEntry; enum values preserved with renames where noted |
| `d2b-core/src/sync.rs`: `SyncJson`, `LockSpec` | `generated-or-eval-contract` | OFD lock rows become Volume layout entries with `leaseClass: file-record` |
| `d2b-core/src/storage_lifecycle.rs`: `StorageLifecycleReport`, `StorageLifecycleIssue`, `StorageContractValidationReason` | `production-reachable` | Daemon startup lifecycle report; migrated to Volume controller phase/condition reporting |
| `d2b-priv-broker/src/ops/swtpm_dir.rs`: swtpm provisioning, fail-closed marker, reconcile-in-place, ancestor ACL, `seccomp_policy_ref: "w1-swtpm"` | `production-reachable` | Migrated to volume-local `create-if-never-provisioned` + fail-closed repair for TPM Volume; marker algorithm adapted to `marker.rs` |
| `d2b-priv-broker/src/ops/store_sync.rs`: `run_store_sync`, `StoreSyncOutcome`, `cleanup_store_view`, `prune_gcroots` | `production-reachable` | Migrated to volume-local `StoreSyncComplete` broker op |
| `d2b-priv-broker/src/ops/store_sync_audit.rs`, `store_sync_export.rs` | `production-reachable` | Migrated to volume-local audit/export ops |
| `d2b-priv-broker/src/ops/store_view_posture.rs`: `posture_store_view_matrix_paths`, `plant_live_marker_with_matrix_posture` | `production-reachable` | No-recursion posture for `state/`, `gcroots/`, `sync.lock`; migrated to volume-local repair policy and LayoutEntry invariants |
| `d2b-priv-broker/src/ops/state_dir.rs`: `PrepareStateDir`, `PrepareRuntimeDir`, `PrepareDirRequest`, `DirKind` | `production-reachable` | Migrated to volume-local `ProvisionLayoutEntry` op |
| `d2b-priv-broker/src/ops/store_view_farm.rs` | `production-reachable` | Migrated to volume-local store-view mode with hardlink farm invariants |
| `d2b-priv-broker/src/ops/store_verify.rs` | `production-reachable` | Migrated to volume-local marker verification flow |
| `d2b-priv-broker/src/ops/storage_contract.rs`: `reconcile_storage_scope`, `validate_lock_spec`, `validate_storage_scope` | `production-reachable` | Live broker handler; migrated to volume-local broker ops; path-hash in errors (never raw path) preserved |
| `d2b-host/src/hardlink_farm.rs`: `build_store_view`, `build_farm`, `GenerationMarker`, `BuildStoreViewRequest`, `gcroots_dir`, `state_dir`, `meta_dir`, `live_dir`, `sync_lock_path`, `generation_id` | `production-reachable` | Canonical store-view layout; migrates to volume-local store-view mode; confirms `gcroots/` and `state/` at store-view root (not under `meta/`) |
| `d2b-host/src/virtiofsd_argv.rs`: `VirtiofsdArgvInput`, `generate_virtiofsd_argv` | `production-reachable` | Extracted to `d2b-provider-volume-virtiofs`; not a volume-local concern |
| `nixos-modules/storage-json.nix`: all path rows with `scope:"vm:<vm>"` / `scope:"host"` | `nix-emitted` | Each path row maps to a Volume LayoutEntry or non-Volume host path (see migration table in `ADR-046-resources-volume.md` §Current storage.json path rows → Volume migration) |
| `nixos-modules/store.nix`: per-VM hardlink farm activation, private-NS sync algorithm | `nix-emitted` | Extracted to volume-local store-view mode; sync algorithm and private-mount-NS invariant preserved |
| `d2b-contract-tests/tests/storage_sync_contracts.rs` | `production-reachable` | Live gate asserting storage.json/sync.json wiring and opaque-id contract; adapted to Volume resource parity gate in v3 |
| `tests/unit/nix/cases/per-vm-state-ownership.nix` | `production-reachable` | Adapted to v3 Volume LayoutEntry matrix |
| `tests/unit/smoke/smoke-eval-tpm.nix` | `production-reachable` | Migrated to volume-local TPM Volume conformance test |
| `d2b-state/src/` (main, `6faa5256`) | `main-reuse-only` | Copy/adapt as documented in §d2b-state reuse plan; absent from v3 baseline |

---

## Crate layout

The `packages/d2b-provider-volume-local/` crate must contain exactly the
following four paths. Absence of any path fails the workspace/package policy
check (`cargo xtask check-provider-crate-layout`, wired into `make test-policy`).

### `src/`

Implementation modules and binary entry points. Colocated `#[cfg(test)]` unit
tests for individual functions.

Required modules:

| Module | Contents |
| --- | --- |
| `src/main.rs` | Controller binary entry point; bootstrap ComponentSession; register as volume-local Provider |
| `src/controller.rs` | Async reconcile loop; Volume watch; layout dispatch; EphemeralProcess dispatch |
| `src/source.rs` | Source-kind resolution: `LocalPath`, `BlockImage`, `Tmpfs`; `sourcePolicyId` validation against declared policies; semantic `VolumeEffectPort` operation dispatch |
| `src/layout.rs` | LayoutEntry evaluation; topological sort; parent-before-child ordering |
| `src/acl.rs` | ACL policy logic: translates `AclGrant` entries and `foreignChildPolicy` rules into `repair_layout_entry` calls on the `VolumeEffectPort`; no ACL values, numeric identities, or syscall bindings |
| `src/marker.rs` | Marker status state machine and typed write/verify/cleanup request mapping; HMAC, path, and bytes remain broker-private |
| `src/quota.rs` | Quota policy and write-reject gate; maps checks/observations to closed broker operations; no direct `statfs` |
| `src/migration.rs` | Pre-launch and online migration dispatch; staging Volume lifecycle; cross-component prepare/commit/rollback |
| `src/sealing.rs` | Status-first sealing state machine; constructs canonical `RotateSealingKeyRequest`; dispatches only `VolumeEffectPort::rotate_sealing_key`; classifies typed result/errors and retry; no key lease, direct rewrite, generic broker call, or EphemeralProcess worker |
| `src/snapshot.rs` | Snapshot EphemeralProcess dispatch; `.snapshots/` subtree; `snapshotPolicy` enforcement |
| `src/relocation.rs` | Relocation EphemeralProcess coordination, source finalizer, typed `RelocateVolumeContents` mapping, and commit/failure handling; no copy syscall |
| `src/store_view.rs` | Store-view policy: layout, generation, gcroots, and lock semantics mapped to `StoreSyncComplete`; no hardlink, mount-namespace, write, or unlink implementation |
| `src/swtpm_volume.rs` | TPM Volume policy: `create-if-never-provisioned`, fail-closed marker, and typed `PrepareSwtpmDir` mapping; no ACL or filesystem implementation |
| `src/effect_port.rs` | Re-exports the shared `VolumeEffectPort` and Provider-side opaque IDs; constructs bounded semantic requests only; does not import/re-export broker-wire DTOs and contains no adapter, syscall, broker, or audit implementation |
| `src/atomic.rs` | `CanonicalJson`/`StateEnvelope` validation and typed `CommitVolumeTransactionRequest` construction; no `RealAtomicFilesystem`, path, fd, write, link, rename, or fsync |
| `src/path.rs` | Opaque declared layout/state-slot ID bindings and lexical schema validation only; broker-private anchored path types do not enter this crate |
| `src/lock.rs` | Cancellation and logical lock-order policy only; OFD lock and FD-transfer effects remain broker-owned |
| `src/audit.rs` | Closed lifecycle observation enums and typed audit-port requests; no writer, segment, raw identity, general digest constructor, or durability choice |
| `src/otel.rs` | Exact central metric-registry row selection, closed enum values, complete span terminalization, and bounded diagnostics; no descriptors |
| `src/status.rs` | Volume status builders; condition constructors; phase transition |
| `src/error.rs` | Typed error catalog; bounded message construction; no-path invariant |

**Cargo.toml dependencies**: `d2b-contracts`, `d2b-provider`, `d2b-provider-toolkit`.
Must NOT depend on `d2b-priv-broker`, `d2bd`, or any crate that provides broker
implementation, `openat2`/`nix`/`libc` syscall bindings, or raw host-path types.

### `tests/`

Hermetic Cargo integration tests. Use `Fixture`/`FakeProvider`/`DeterministicClock`
from the controller toolkit; no live daemon required.

Required test files and minimum coverage:

| Test file | Required scenarios |
| --- | --- |
| `tests/layout_provision.rs` | Create directory entry; create file entry; create symlink entry with relative target; symlink target with `..` rejected; `broker-opaque-id-only` entry rejects non-broker child; Unicode path-separator homoglyph rejected; `noFollow: true` rejects symlink path |
| `tests/layout_repair.rs` | Drift detected on owner/mode/ACL; `repairPolicy: exact-owner` corrects; `repairPolicy: fail-closed` sets Failed; `repairPolicy: none` sets condition; ACL re-reconcile after User revision |
| `tests/layout_adopt.rs` | `adopt-with-live-owner-proof` with live pidfd; `quarantine-on-ambiguity` with no proof; `not-adoptable` always recreated |
| `tests/acl.rs` | `accessAcl` applied and reconciled; `defaultAcl` applied to new children; `foreignChildPolicy: preserve` retains surplus; `foreignChildPolicy: fail` sets `ForeignAclViolation` |
| `tests/quota.rs` | `enforcement: hard` - FS supports quota: admitted; FS does not support quota: `Failed`; write above quota: rejected; `quotaUsage` reported; descriptor-Volume mismatch: `quota-mismatch` |
| `tests/marker.rs` | First-provision write; restart verify: `verified`; marker missing: `missing` → Failed; `st_ino` mismatch: `replaced` → Failed; HMAC tampered: `tampered` → Failed; `installedSchemaVersion` > spec: migration-failed |
| `tests/store_view.rs` | Hardlink farm LayoutEntry matrix validated; `gcroots/` at root (not under `meta/`); `state/` at root; `sync.lock` never unlinked; `live/.d2b-marker-<vm>` zero-length; same-filesystem invariant: `st_dev` mismatch → Failed |
| `tests/swtpm_volume.rs` | TPM `create-if-never-provisioned`: existing → preserve; marker absent after provision → Failed; owner mismatch → Failed; quarantine on ambiguity; ancestor traverse ACL applied |
| `tests/source.rs` | `local-path` `sourcePolicyId` matched and unmatched policy; `block-image` without quota maxBytes fails eval; `tmpfs` without maxBytes/maxInodes fails eval; `tmpfs` with `create-if-never-provisioned` entry fails eval |
| `tests/symlink.rs` | Valid relative target; `..` component rejected; absolute target rejected; null byte rejected; target resolves outside Volume root rejected |
| `tests/domain_isolation.rs` | `private` Volume: concurrent mount from different domain rejected; `internal` Volume: cross-Provider mount rejected; `shared-read` Volume: read-only cross-Provider permitted; `shared-read` write rejected |
| `tests/view_rights.rs` | Mount with rights subset of View: admitted; mount with extra right: `volume-view-rights-exceeded`; `read-write` access on `read-only` View: rejected; single-writer constraint: second `read-write` rejected |
| `tests/effect_port_contract.rs` | Existing Provider-side ID/wire/redacted-Debug bounds and semantic request coverage; Provider dependency/source scan proves no broker-wire import, syscall, path resolver, command, or audit writer. The exhaustive trait-method-to-`VolumeBrokerOperation` bijection is tested with the adapter under `ADR046-vl-012` |
| `tests/state.rs` | Provider-side StateEnvelope/generation/quarantine validation and fake-port request mapping; real atomic write/fsync/rename/OFD-lock crash ordering lives in broker Volume-operation integration tests |
| `tests/migration_unit.rs` | Pre-launch migration dispatch; staging Volume create/destroy; EphemeralProcess succeeded → commit; EphemeralProcess failed → rollback; N-Volume cross-component prepare/commit/rollback protocol; roll-forward on restart detection |
| `tests/sealing_unit.rs` | Initial seal/read without exposing a key lease; status CAS commits `rotation-pending` before the first effect; exact request fields and generation/revision/policy preconditions using the canonical validated effect-port ID newtypes; operation ID comes from committed proof; deterministic idempotency vector; byte-identical timeout/restart retry; duplicate success → `AlreadyCommitted`; recovered commit → `RecoveredCommitted`; changed bytes under one key → `IdempotencyConflict`; concurrent different rotation → `RotationConflict`; retryable/terminal error table; new generation re-plan; success → `sealed`; integrity failure → Failed; no key/path/handle in DTO, status, error, Debug, log, audit, or OTEL; no direct rewrite/generic broker/EphemeralProcess dispatch |
| `tests/snapshot_unit.rs` | `snapshotPolicy` enforcement; retention count; retention TTL; `triggerOnMigration` auto-snapshot; snapshot EphemeralProcess dispatch; list in Volume status |
| `tests/relocation_unit.rs` | Finalizer set; EphemeralProcess created; commit: source deleted; failure: source retained; state machine round-trip |
| `tests/audit_unit.rs` | Golden typed observation for each event kind; authoritative records use only approved correlation newtypes and closed enums; exact absence of raw Zone/Volume/resource identity, `ResourceRef`, UID/GID, Provider-local/general digests, paths, credentials, and identity-bearing OTEL labels/attributes; central registry row equality and complete span outcomes |
| `tests/error_messages.rs` | Every error code emitted with bounded (≤512 byte) message; no host path in any error message |

### `integration/`

Heavier scenario tests requiring a container, real Host processes, real
filesystem mounts, cross-process coordination, or provider-system fixtures.
Invoked by `make test-integration` / `make heavy-test-integration`, not by bare
`cargo test`.

Required files:

| File | Required scenarios |
| --- | --- |
| `integration/README.md` | Prerequisites (container runtime, real Host filesystem); how to run (`make test-integration`); environment variables; expected setup/teardown; scenario descriptions |
| `integration/provision.rs` | Real Host filesystem provision: `local-path` Volume create, marker written, FD handed to fake Process, marker verified on restart; `block-image` image file created at declared size; `tmpfs` mounted with quota limits, unmounted on cleanup |
| `integration/store_view.rs` | Same-filesystem boundary enforcement: `/nix/store` and `$stateDir` on same FS → hardlink created; on different FS → `storage-drift`; `live/.d2b-marker-<vm>` zero-length confirmed; `gcroots/` at store-view root confirmed; private-NS sync with concurrent reader |
| `integration/quota_fs.rs` | Real FS quota fixture: quota-capable FS → `enforcement: hard` admitted; quota-incapable FS → Volume set to `Failed` immediately; write above quota → `volume-quota-exceeded` |
| `integration/swtpm_marker.rs` | Real broker-maintained marker: provision, restart verify, inject `st_ino` mismatch → Failed; remove marker → Failed; marker HMAC tampered → Failed; ancestor traverse ACL applied to real inode |
| `integration/block_image.rs` | Block-image lifecycle: create raw image at size, verify `fallocate` when `preallocate: true`, FD transfer to fake Guest runtime process, cleanup removes image file |
| `integration/migration.rs` | Real Host crash-injection at each migration step (OS-level `SIGKILL` between `rename` steps); roll-forward on restart; N-Volume cross-component coordination with real staging Volume; staging orphan GC after Provider removal |
| `integration/sealing.rs` | Live Credential authority plus real adapter and planned closed broker `RotateSealingKey`: Provider never receives a lease; same-Zone capability/policy authorization and denials; crash injection at Prepared, Staged, metadata switch, audit append, and retirement; old-or-target atomic visibility; roll-forward recovery; exactly-once success audit; revoked/unavailable target behavior; controller restart resumes pending operation with identical idempotency key; precondition drift re-plans; request/broker journal/audit contain no key bytes or paths |
| `integration/snapshot.rs` | Real Host filesystem snapshot byte-equality verification; retention expiry removes snapshot directory; pre-migration auto-snapshot with interrupted migration; snapshot list in status |
| `integration/relocation.rs` | Real Host-to-Host anchored file copy; crash at copy midpoint → source preserved; successful relocation → source deleted; virtiofsd source re-point after relocation (via volume-virtiofs stub) |
| `integration/domain_isolation.rs` | Cross-process domain-isolation rejection: two fake Processes in different domains attempt same Volume mount; `volume-domain-mismatch` returned |
| `integration/audit.rs` | Live typed audit-port and broker audit path: intent is durable before every privileged effect, completion is durable exactly once before success, `CommitPendingAudit` recovers by byte-identical retry, and records contain only approved typed digests/enums; central-registry OTEL export carries no identity |
| `integration/provider_state.rs` | End-to-end served-Volume lifecycle: live daemon, volume-local controller starts and reaches Ready with no Provider state Volume of its own (bounded operational state in status/core ledger, D087); another Provider's *declared* state Volume is created by core ProviderDeployment, reconciled by volume-local, marker verified on restart, and removed on Provider delete; no cross-component dirfd sharing; full served-Volume lifecycle (provision → migrate → snapshot → destroy) |

An empty `integration/` directory is not acceptable. The `README.md` and at
least one `.rs` scenario file are required for the workspace policy check to
pass.

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has an advisory wall-clock p95
diagnostic threshold of <=50 ms; gate enforcement is aggregate per-crate
process CPU only. There is no wall-clock
sleep, and `cargo test -p d2b-provider-volume-local --lib --tests` completes in ≤3 s warm-cache
execution time (compilation excluded). They use a deterministic fake clock/RNG
and the toolkit fakes/FakeEffectPort only - no process spawn, container,
network, DBus, systemd, broker daemon, Nix eval/build, KVM, USB/GPU/TPM
hardware, or live cloud, and no filesystem tree beyond tiny temp fixtures. Any
scenario needing those lives only in `integration/`, which keeps a lane
timeout/budget, parallel isolation, and fake external services by default; such
a need is re-placed into `integration/`, never given a sleep, larger timeout,
or `#[ignore]`. Bounded crypto/property tests are the only classified
exception, each named with a capped case count and a declared higher per-test
budget.

### `README.md`

Documents:

1. **Provider identity**: artifact ID `volume-local-provider`; ResourceTypes
   owned; crate location.
2. **Config schema**: `spec.config.*` fields with types, defaults, and
   constraints:
   - `sourcePolicies`: list of `{ id: string, class: "local-path" | "block-image" | "tmpfs", volumeKinds: string[] }`;
     required; minimum one entry; IDs are opaque references used in Volume
     `source.settings.sourcePolicyId` fields; raw host path prefixes are private
     bundle authority and are not present in this config block.
   - `controllerExecutionRef`: execution ref for the controller process;
     defaults to `Host/host-system`.
3. **Owned ResourceTypes**: `Volume`; supported source kinds; VolumeKind matrix.
4. **Controllers/services/workers/binaries**: `volume-local-controller` binary;
   EphemeralProcess template names and their triggers.
5. **Placement**: system-domain Host process only; no Guest attachment.
6. **Dependencies and RBAC**: Zone User resource; Zone Credential resource;
   all host-authority effects map one-to-one through the injected
   `VolumeEffectPort` to closed broker operations; the pure core adapter has no
   syscall or audit writer; no direct broker connection from Provider process;
   no host-path permission claim.
7. **Security model**: no ambient host capabilities; no path in status/audit;
   all path/numeric-ID resolution and openat2/ACL/mount/fallocate/unlink/write/
   key effects inside closed broker handlers with durable intent/completion
   audit; marker fail-closed; same-filesystem invariant; ADR 0021 inapplicable
   (virtiofsd is volume-virtiofs's concern).
8. **State surfaces**: identity marker at `$stateDir/volume-local-markers/<uid>`;
   staging Volumes; snapshot `.snapshots/` subtree; never in status/audit.
9. **Audit and telemetry surfaces**: approved typed audit digests and closed
   enums only; exact central metric-registry rows; no raw identity,
   Provider-local descriptor/digest, identity label/attribute, or unbounded
   cardinality.
10. **Build, test, and integration command reference**:
    ```bash
    cargo build -p d2b-provider-volume-local
    cargo test -p d2b-provider-volume-local
    make test-integration                        # runs integration/
    make heavy-test-integration                  # with heavy-gate semaphore
    cargo xtask check-provider-crate-layout      # workspace policy check
    ```
11. **Future standalone-repo usage**: how to vendor or depend on this crate
    outside the d2b workspace; which ADR 0046 contracts are required.

---

## Implementation work items

### W6 foundation ownership and serial handoff

The feature-local T608 task is a readiness foundation, not a manifest work-item
identity. Its owned-file list is authoritative while T608 is active. No
`ADR046-vl-*` task writes one of those files concurrently. After T608 reaches
`Merged`, ownership transfers serially as follows:

| T608/T609 foundation surface | Manifest-backed receiving tasks | Handoff rule |
| --- | --- | --- |
| Shared Volume contracts and typed broker-effect foundation | `ADR046-vl-001`, `ADR046-vl-012` | Provider contracts consume the merged semantic DTOs without re-exporting broker wire; the adapter consumes both sides and extends them only through a new typed variant, policy, durable audit class, and negative tests |
| Foundational volume-local crate edits | `ADR046-vl-002` through `ADR046-vl-009`, `ADR046-vl-013` | T608 supplies no completion state for these rows; each task implements and validates its full dossier deliverable against the merged foundation |
| Volume Nix generator/compiler/assertion surfaces | `ADR046-vl-010` | Consume T608's strict generated base; preserve generator/module/assertion/schema bijection and follow the W6 shared-file order |
| `packages/d2bd/src/volume_effect_adapter.rs` | `ADR046-vl-012` | Preserve the adapter as a pure exhaustive mapping; every syscall and durable audit implementation remains in closed broker handlers |
| T609 typed audit writer and central telemetry registry | `ADR046-vl-009`, `ADR046-vl-012` | Consume only typed ports/registry rows; do not add a Provider writer, descriptor, general digest, or durability policy |

File ownership may hand off after T608 merges, but the receiving group becomes
Ready only after the feature task contract proves T606, T607, T608, and T609
`Merged`. This handoff changes no retained W5 checkbox, implementation state,
evidence, or delivery row.

### ADR046-vl-001 - Volume contracts and state schema

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-vl-001` |
| Dependency/owner | ADR046-primitives-001; v3 contracts owner |
| Depends on | `ADR046-pstate-001` (VolumeStateSchema/PersistenceClass/SensitivityClass/StateEnvelope in `d2b-contracts/src/v3/volume_state.rs`) |
| Current source | `d2b-core/src/storage.rs` (`StoragePathSpec`, `StoragePathKind`, policy enums); `d2b-core/src/sync.rs` (`SyncJson`, `LockSpec`) |
| Reuse action | adapt |
| Destination | `d2b-contracts/src/v3/volume_layout.rs` (LayoutEntry, EntryType, all policy enums, AclGrant, Invariant, SensitivityClass); `d2b-contracts/src/v3/volume_spec.rs` (VolumeSpec, ViewSpec, Attachment, QuotaSpec, SourceKind, `SourcePolicyId` opaque newtype); `d2b-contracts/src/v3/effect_port.rs` (`VolumeEffectPort`, opaque ID newtypes `VolumeId`/`LayoutEntryId`/`UserId`/`ViewId`/`SealingPolicyId`, `VolumeMountToken`, transaction/snapshot/relocation DTOs, and canonical `RotateSealingKeyRequest`/`Result`/`Error` types); consumes the T608-frozen broker-wire variants without redefining them |
| Detailed design | All LayoutEntry fields as documented in this dossier; enum value names preserved from `StoragePathKind`/policy enums with renames where noted; `User/<name>` ACL principal (no numeric UID); `sourcePolicyId` opaque newtype replaces raw `hostPath`; semantic requests accept no path, numeric identity, ACL/mount string, FD, key bytes, broker handle, or broker-wire DTO; deny-unknown sealing rotation retains canonical bounded opaque IDs and redacted Debug. The adapter-owned one-to-one mapping remains an `ADR046-vl-012` obligation Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Volume spec and status structs; Provider descriptor component stateNamespace; Nix resource compiler schema validation |
| Data migration | Full v3 reset; no row-level import |
| Validation | Existing schema/serde/opaque-ID/redacted-Debug vectors; compile-fail tests deny a generic Provider request, raw path, numeric identity, ACL/mount string, FD, key bytes, broker handle, and broker-wire import; compile-time trait conformance includes transaction/snapshot/relocation and `rotate_sealing_key`; emit the exact port-method census consumed by the executable adapter/broker bijection gate in `ADR046-vl-012` |
| Removal proof | `d2b-core/src/storage.rs` StoragePathSpec/policy enums removed only after all Provider descriptor consumers are on v3 Volume spec |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-vl-002 - Crate scaffold and filesystem primitives

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-vl-002` |
| Dependency/owner | ADR046-vl-001; volume-local Provider owner |
| Depends on | `ADR046-pstate-003` |
| Current source | `d2b-state/src/{atomic,path,lock}.rs` (main `6faa5256`); `d2b-priv-broker/src/ops/swtpm_dir.rs` (marker algorithm) |
| Reuse action | adapt |
| Destination | Full `packages/d2b-provider-volume-local/` scaffold per §Crate layout: `src/`, `tests/`, `integration/`, `README.md`; crate `Cargo.toml` depends only on `d2b-contracts`, `d2b-provider`, `d2b-provider-toolkit` |
| Detailed design | Provider keeps only canonical `StateEnvelope` validation, opaque layout/state-slot bindings, logical lock order, marker status policy, and bounded semantic effect requests. Anchored path, real atomic filesystem, OFD lock, HMAC, ACL, mount, fallocate, unlink, write, fsync, and key implementations are adapted behind closed broker operations. `src/effect_port.rs` re-exports only the shared Provider trait/DTOs and provides no adapter, broker-wire import, syscall, broker, or audit implementation; `sourcePolicyId` validation remains Provider-side Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt `atomic.rs`, `path.rs`, `lock.rs`, and the swtpm marker algorithm across the sealed boundary. |
| Integration | Controller binary receives `VolumeEffectPort` via ComponentSession injection; its pure adapter maps `provision_marker` and `verify_marker` to closed broker operations when a Volume first appears or restart relist runs |
| Data migration | New marker written for each Volume at v3 first-boot |
| Validation | All `tests/marker.rs`, `tests/state.rs`, and `integration/provision.rs` scenarios; dependency/source checks prove no `d2b-priv-broker`/`d2bd`, syscall binding, command execution, raw host-path type, audit writer, or Provider-local descriptor in the Provider crate |
| Removal proof | `swtpm_dir.rs` marker implementation retired only after device-tpm Provider Volume is live and marker-check parity is confirmed |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-vl-003 - Controller reconcile loop and layout engine

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-vl-003` |
| Dependency/owner | ADR046-vl-002; ADR046-reconcile-001; d2b-bus/ComponentSession owner |
| Current source | `d2b-priv-broker/src/ops/{state_dir,storage_contract}.rs` (broker layout ops); `d2b-core/src/storage_lifecycle.rs` (lifecycle report) |
| Reuse action | adapt |
| Destination | `src/controller.rs`, `src/layout.rs`, `src/acl.rs`, `src/source.rs` |
| Detailed design | Async reconcile loop; topological LayoutEntry evaluation; semantic `VolumeEffectPort` dispatch through the injected pure adapter; ACL reconciliation by opaque IDs; drift detection; expected-revision status; source-policy validation; responsive concurrent per-resource calls; **single watch scope** `providerRef: Provider/volume-local`; ProviderDeployment owns Volume resource create/delete; no direct broker connection, broker-wire import, host syscall, path/numeric-ID resolution, audit writer, or generic effect; Nix-preprovisioned User principals; no cross-component Volume sharing; empty-payload stateNamespace Volumes use `migrationPolicy: none` |
| Integration | Controller binary instantiated by Zone runtime after Provider Ready; receives `VolumeEffectPort` implementation and d2b-bus `ResourceClient` via ComponentSession |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | `tests/layout_provision.rs`, `tests/layout_repair.rs`, `tests/layout_adopt.rs`, `tests/acl.rs`, `tests/view_rights.rs`, `tests/source.rs`, `integration/provision.rs` |
| Removal proof | `d2b-priv-broker/src/ops/storage_contract.rs` `reconcile_storage_scope` retired only after Volume controller parity confirmed |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-vl-004 - Store-view Volume

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-vl-004` |
| Dependency/owner | ADR046-vl-003; runtime-cloud-hypervisor Provider owner |
| Current source | `d2b-host/src/hardlink_farm.rs`; `d2b-priv-broker/src/ops/{store_sync,store_view_posture,store_view_farm,store_sync_audit,store_sync_export}.rs`; `nixos-modules/store.nix` |
| Reuse action | adapt |
| Destination | `src/store_view.rs`; `tests/store_view.rs`; `integration/store_view.rs` |
| Detailed design | Store-view LayoutEntry matrix; Provider maps `run_store_sync` to closed `StoreSyncComplete`; broker owns private-NS sync, OFD lock, hardlink/copy/write/unlink, durable generation commit, and privileged-effect audit; `gcroots/` and `state/` remain at the store-view root; enforce the spec correction |
| Integration | `runtime-cloud-hypervisor` Provider declares store-view Volume in its ProviderStateSet; volume-local controller handles sync |
| Data migration | None (format preserved; activation changed from Nix to Volume controller) |
| Validation | `tests/store_view.rs` all invariants; `integration/store_view.rs` same-filesystem boundary; private-NS sync with concurrent reader |
| Removal proof | `nixos-modules/store.nix` activation and `d2b-priv-broker/src/ops/store_sync.rs` retired only after store-view Volume controller is live and passes all parity tests |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-vl-005 - TPM Volume

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-vl-005` |
| Dependency/owner | ADR046-vl-002; device-tpm Provider owner |
| Current source | `d2b-priv-broker/src/ops/swtpm_dir.rs` |
| Reuse action | adapt |
| Destination | `src/swtpm_volume.rs`; `tests/swtpm_volume.rs`; `integration/swtpm_marker.rs` |
| Detailed design | TPM LayoutEntry matrix; `create-if-never-provisioned` + fail-closed repair; broker-maintained provisioning marker; ancestor traverse ACL; `previously-provisioned-swtpm-state-missing` fail-closed detection; `secret-adjacent` sensitivity enforcement |
| Integration | `device-tpm` Provider declares TPM Volume in ProviderStateSet; volume-local handles layout/marker lifecycle |
| Data migration | None (full v3 reset; TPM NVRAM must be backed up by operator) |
| Validation | `tests/swtpm_volume.rs` all scenarios; `integration/swtpm_marker.rs` real broker-maintained marker |
| Removal proof | `d2b-priv-broker/src/ops/swtpm_dir.rs` retired only after device-tpm Provider TPM Volume is live and fail-closed tests pass |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-vl-006 - Block-image and tmpfs source kinds

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-vl-006` |
| Dependency/owner | ADR046-vl-003 |
| Current source | No equivalent in baseline; new |
| Reuse action | create |
| Destination | `src/source.rs` (block-image and tmpfs branches); `tests/source.rs`; `integration/block_image.rs` |
| Detailed design | `block-image`: pure mapping from `provision_block_image` to closed `ProvisionBlockImage`; broker performs anchored create/verify and optional `fallocate`; `OpenVolumeMountToken` audits and transfers the FD out-of-band; `tmpfs`: pure mappings to separate `MountTmpfs`/`UnmountTmpfs` variants, with broker-derived size/inode options and broker-owned mount/unmount |
| Integration | Guest runtime Provider (cloud-hypervisor) receives block-image FD from volume-local via LaunchTicket; no path crosses the boundary |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | `tests/source.rs` allowlist pass/fail; block-image/tmpfs eval constraints; `integration/block_image.rs` real image lifecycle |
| Removal proof | Not applicable (new) |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-vl-007 - Migration and snapshots

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-vl-007` |
| Dependency/owner | ADR046-vl-002; ADR046-vl-003; ADR046-pstate-004 through ADR046-pstate-006 |
| Current source | `d2b-state/src/atomic.rs` (main); no existing migration/snapshot infrastructure in v3 |
| Reuse action | adapt |
| Destination | `src/{migration,snapshot,sealing}.rs`; `tests/{migration_unit,snapshot_unit,sealing_unit}.rs`; `integration/{migration,snapshot,sealing}.rs` |
| Detailed design | Schema migration and snapshot workers coordinate policy/status only; every host-authority transaction/snapshot effect maps through `CommitVolumeTransaction`, `CreateVolumeSnapshot`, or `ExpireVolumeSnapshot`. Sealing uses status-first `rotation-pending` and only `VolumeEffectPort::rotate_sealing_key`; it persists/resumes the core Operation-ledger fingerprint and has no key lease, direct rewrite, generic broker call, or sealing worker. Broker effects commit audit intent before release and completion exactly once before success. |
| Integration | Controller's reconcile handler dispatches migration/snapshot EphemeralProcess via d2b-bus `ResourceClient`; sealing reconcile/upgrade commits status before effect and maps typed results to `sealed`/`rotation-failed`; volume-local reports state schema, snapshots, and safe sealing generations in Volume status |
| Data migration | None (new protocol) |
| Validation | All `tests/migration_unit.rs`, `tests/snapshot_unit.rs`, `tests/sealing_unit.rs`, `integration/migration.rs`, `integration/snapshot.rs`, and `integration/sealing.rs` scenarios, including restart/idempotency and status-before-effect assertions |
| Removal proof | Not applicable (new) |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-vl-008 - Relocation, retention, incident hold, unclaimed GC, destruction

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-vl-008` |
| Dependency/owner | ADR046-vl-003; ADR046-vl-007 |
| Current source | No equivalent; new |
| Reuse action | create |
| Destination | `src/relocation.rs`; `tests/relocation_unit.rs`; `integration/relocation.rs` |
| Detailed design | As documented in §Relocation, §Retention, §Incident hold, §Unclaimed Volume GC, §Destruction |
| Integration | Controller adds `Relocating` finalizer and coordinates the relocation worker; every copy/write/fsync/unlink step is expressed by `RelocateVolumeContents`, `CleanupLayoutEntry`, `CleanupVolumeMarker`, or `CleanupVolumeRoot`, with broker-enforced order and durable effect audit |
| Data migration | Not applicable |
| Validation | All `tests/relocation_unit.rs`, `integration/relocation.rs` scenarios; destruction ordering under fault injection |
| Removal proof | Not applicable (new) |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-vl-009 - Audit, OTEL, and error catalog

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-vl-009` |
| Dependency/owner | ADR046-vl-001; authoritative audit owner; central telemetry registry owner |
| Current source | `d2b-state/src/audit.rs` (main `6faa5256`); OTEL cardinality model from `d2b-provider-observability-local/src/` (main `a1cc0b2d`) |
| Reuse action | adapt |
| Destination | `src/audit.rs`; `src/otel.rs`; `src/error.rs`; `tests/audit_unit.rs`; `integration/audit.rs` |
| Detailed design | Consume the T609 typed audit port and central `METRIC_DESCRIPTOR_REGISTRY`. Provider code constructs closed lifecycle observations but opens no writer and chooses no durability; broker operations own durable intent/completion. Audit uses only approved `ResourceCorrelationDigest`, `SubjectCorrelationDigest`, `ExecutionCorrelationDigest`, `OperationCorrelationDigest`, and `PrivilegedEffectAuditDigest` in their allowed record classes plus closed enums. Metrics/Resource attributes carry no raw identity or digest, and operation/trace span linkage uses only approved typed digests. Remove raw `ZonePath`/`ResourceRef`, Provider-local `VolumeAuditDigest`, local metric descriptors, and `d2b.zone`. |
| Integration | Every lifecycle transition submits to the typed audit port; every broker effect proves durable intent/completion; telemetry selects central rows exported via `observability-otel` |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | Golden typed records reject raw Zone/Volume/resource/user/process/policy/snapshot/operation/execution identity, `ResourceRef`, UID/GID, general/provider-local digests, paths, keys, and handles; central registry equality and identity-free label/Resource-attribute tests; complete span terminal outcomes; broker intent-before-effect, exactly-once completion, `CommitPendingAudit`, restart, and concurrency fault injection; bounded errors; live integration stream |
| Removal proof | Not applicable |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-vl-010 - Nix configuration and resource compiler integration

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-vl-010` |
| Dependency/owner | ADR046-vl-001; ADR046-pstate-010; NixOS module owner |
| Current source | `nixos-modules/storage-json.nix`; `nixos-modules/store.nix`; `packages/xtask/src/main.rs` (`gen-schemas`) |
| Reuse action | adapt |
| Destination | `nixos-modules/zone-resources.nix` (per §ADR046-pstate-010); `root-config.schema.json` in the Provider package |
| Detailed design | `sourcePolicies` in Provider root config (opaque IDs; no raw host paths); path prefix injection by resource compiler into private bundle (never into ResourceSpec or operator-authored Nix); `controllerExecutionRef` in Provider config; all eval-time validation rules per §Nix configuration including `sourcePolicyId` validation; artifact catalog entry; Provider and Volume resource authoring shapes |
| Integration | NixOS build emits `/etc/d2b/zones/<zone>/resource-bundle.json`; Zone daemon activates bundle and creates Volume resources; volume-local controller reconciles |
| Data migration | `nixos-modules/storage-json.nix` path rows superseded by Volume resources; `nixos-modules/store.nix` activation superseded by store-view Volume |
| Validation | All Nix eval-time validation rules; `contentHash` determinism; credential-ref guard; unknown Provider config key → build fail |
| Removal proof | `nixos-modules/storage-json.nix` and `nixos-modules/store.nix` per-VM rows retired only after Volume resources replace every path row and all consumers complete bundle-format migration |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-vl-011 - Workspace policy gate

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-vl-011` |
| Dependency/owner | ADR046-vl-002; ADR046-pstate-011; workspace policy owner |
| Current source | `packages/xtask/src/main.rs` (`gen-schemas`, workspace-policy checks); `tests/unit/gates/drift-check.sh` |
| Reuse action | adapt |
| Destination | `packages/xtask/src/provider_crate_policy.rs`; `tests/unit/gates/provider-crate-layout-check.sh` |
| Detailed design | `cargo xtask check-provider-crate-layout` gate asserts `src/`, `tests/`, `integration/` (with at least one `.rs` file and a `README.md`), and `README.md` for every `packages/d2b-provider-*` workspace member; fails closed with typed `missing-provider-crate-path` error Primary reuse disposition: `adapt`. Preserved source-plan detail: extend (per ADR046-pstate-011). |
| Integration | `make test-policy` runs the gate; GitHub CI runs `make test-policy` on every PR |
| Data migration | Not applicable |
| Validation | Gate detects each missing path; idempotent across re-runs; existing non-provider `d2b-*` crates not flagged |
| Removal proof | Not applicable (permanent gate) |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-vl-012 - Pure core adapter and closed Volume broker operations

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-vl-012` |
| Dependency/owner | ADR046-vl-001; ADR046-vl-002; Zone broker/core owner |
| Depends on | `ADR046-pstate-003`; `ADR-046-provider-model-and-packaging` (generic effect-port injection contract) |
| Current source | `d2b-priv-broker/src/ops/{state_dir,storage_contract,swtpm_dir,store_sync,store_view_posture}.rs`; `d2b-host/src/hardlink_farm.rs` |
| Reuse action | adapt |
| Destination | T608-handoff `packages/d2bd/src/volume_effect_adapter.rs` implementing the pure `VolumeEffectPort` mapping; closed Volume request/result types in `d2b-contracts`; broker handlers under `packages/d2b-priv-broker/src/ops/volume/`, including rotation |
| Detailed design | Adapter exhaustively maps each trait method to one closed `VolumeBrokerOperation`, attaches committed proof, dispatches, and correlates audited FD transfer; it has no bundle/path/FD table, syscall, command, key operation, audit writer, or durability policy. Broker independently resolves opaque IDs from private authority; performs every openat2/ACL/mount/fallocate/link/rename/unlink/read/write/fsync/statfs/key effect in bounded blocking handlers; durably commits intent before effect release and completion exactly once before success; and returns `CommitPendingAudit` after a committed mutation whose completion audit is pending. Requests carry no raw path, numeric identity, ACL/mount string, key/credential bytes, broker handle, or caller FD. Rotation retains canonical idempotency and roll-forward recovery Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt behind the sealed broker boundary. |
| Integration | Zone runtime injects the pure adapter through ComponentSession bootstrap; broker alone receives its private bundle/FD authority; controller remains generic over `P: VolumeEffectPort` with no trait object or `async-trait` dependency |
| Data migration | None (pure typed mappings replace direct or adapter-local effect call sites) |
| Validation | Exhaustive mapping tests and planted unknown/generic-op negatives; source/dependency checks prove zero adapter/provider syscall, path resolver, command, audit-writer, and broker-implementation access; broker tests cover every closed effect class, private resolution, authorization, intent-before-effect, durable exactly-once completion, byte-identical `CommitPendingAudit` retry, restart/concurrency, and path/ACL/mount/key rejection; rotation crash injection retains old-or-target visibility and roll-forward; audit wire contains only approved typed digests/enums; `integration/{provision,sealing,audit}.rs` exercise the real boundary |
| Removal proof | Baseline broker op handlers (`state_dir.rs`, `storage_contract.rs`, `swtpm_dir.rs`, `store_sync.rs`, `store_view_posture.rs`) retired only after Volume controller parity is confirmed and all callers are on the adapter |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

### ADR046-vl-013 - No bootstrap-state exception (status-first controller start)

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-vl-013` |
| Dependency/owner | ADR046-vl-001; ADR046-vl-012; Zone broker/core owner |
| Depends on | `ADR-046-provider-model-and-packaging` (Provider install sequencing) |
| Current source | No equivalent; new |
| Reuse action | create |
| Destination | Zone core ProviderDeployment controller-start path (outside `d2b-provider-volume-local`) |
| Detailed design | The volume-local controller declares no Provider state Volume, so there is no bootstrap Volume, no `BootstrapProviderStateVolume` broker op, no pre-provisioned controller Volume, and no bootstrap-storage exception (D086, superseded by D087). On first install and on every daemon restart, core ProviderDeployment starts the volume-local controller Process directly; the controller reaches `Ready` from its own resource `status`, the core Operation ledger, and a resource-store relist. Once Ready, it reconciles every Volume carrying `providerRef: Provider/volume-local` (operator-created Volumes and other Providers' declared state Volumes) as they appear in its `providerRef` watch, re-verifying identity markers against external reality, never creating them itself. A Guest bootstraps its own Guest-local volume-local instance from Guest-local primitives only. |
| Integration | Core ProviderDeployment spawns the controller Process with no state-Volume prerequisite; the controller's startup relist reconciles served Volumes and re-verifies markers |
| Data migration | None; pre-existing baseline `StorageRoot` rows for the volume-local controller are superseded on v3 reset |
| Validation | `integration/provider_state.rs`: controller starts and reaches Ready with no state Volume; served Volumes reconciled and markers re-verified after restart; no bootstrap Volume and no bootstrap Provider Process in the resource list |
| Removal proof | Not applicable (new) |
| Implementation state | Planned |
| Evidence | The complete Destination and Validation obligations above have not both been verified in the indexed tree. |

---

## Security invariants

1. **No raw host path in any surface**: Raw host paths never appear in Volume
   `source.settings`, Volume status, resource list/watch responses, audit
   records, OTEL spans, logs, or CLI output. The `source.settings.sourcePolicyId`
   is the only source reference in the public schema. The broker privately
   resolves the matching path prefix from trusted bundle authority at effect
   time. The pure `VolumeEffectPort` adapter and controller never hold a raw
   path string or numeric UID.

2. **Anchored relative paths**: all LayoutEntry paths are validated as relative
   with no `..`, no leading `/`, no null bytes, no Unicode homoglyphs of path
   separators. Validation runs at schema-validation time (Nix eval), at
   controller admit time, and again after broker-private resolution in the
   closed operation handler.

3. **`noFollow: true` default**: symlink traversal is disabled by default.
   Only `symlink`-type entries with explicit `noFollow: false` traverse.

4. **`broker-opaque-id-only`**: entries with this invariant reject children
   created by non-broker actors, preventing arbitrary file injection into
   controlled subtrees.

5. **No recursive mutation without explicit flag**: `recursive: false` is the
   default. Enabling recursion requires explicit `recursive: true` with
   `repairPolicy: exact-owner` or `fail-closed`.

6. **TPM never re-provisioned**: after the provisioning marker exists, a missing
   or replaced TPM directory is a hard failure. The controller never silently
   creates a new empty TPM directory.

7. **Store isolation**: virtiofsd serving `access: read-only` for the ro-store
   attachment always uses `store-view/live` as `--shared-dir`, never the host's
   `/nix/store`. The compile-time sentinel `share.source == "/nix/store"` in
   `processes-json.nix` triggers store-view substitution only.

8. **No cross-domain dirfd**: volume-local never hands out a dirfd or raw path
   accessible to a process in a different domain or a different User. Domain
   isolation is enforced through `sensitivityClass` validation at attach time.

9. **Typed identity only in audit; no identity in metrics**: the Zone selects
   the private audit partition but is not serialized. Required joins use only
   the approved record-specific correlation newtype, and all other values are
   closed enums or bounded non-identity scalars. Raw Zone/Volume/resource
   identity, ResourceRef, UID/GID, Provider-local/general digests, and paths are
   denied. Metrics and OTEL Resource attributes contain no correlation digest
   or identity. Errors are bounded (≤512 bytes) and contain no path, secret,
   process argument, or terminal byte.

10. **Marker HMAC**: every identity marker's integrity is verified by HMAC before
    any Volume FD is handed out. A tampered marker fails closed with no
    auto-recovery path.

11. **No bootstrap Volume**: the volume-local controller declares no state
    Volume; there is no closed bootstrap sequence, no bootstrap-provisioned
    controller Volume, and no bootstrap-storage exception (D086, superseded by
    D087). The controller reaches Ready from resource `status`/the core
    Operation ledger; a Guest bootstraps its own Guest-local volume-local
    instance from Guest-local primitives only.

12. **Declared component state Volumes are durable when justified**: a component
    declares a state Volume only when its payload passes the storage-need test
    (secret/large/private/revision-unsuitable). Every *declared* state Volume in
    a ProviderStateSet must have `kind: state`, `persistenceClass: persistent`,
    and a nonzero `quota.maxBytes` and `quota.maxInodes`. A component descriptor
    declaring a stateNamespace with `persistenceClass: ephemeral`, `kind: tmp`,
    or `quota.maxBytes: 0` fails Provider admission with a typed
    `invalid-provider-state-namespace` error, and a namespace whose payload is
    fully derivable from spec/status/core ledger/external observation fails with
    `component-state-not-justified`. There is no empty identity-only state
    Volume.

13. **Every host-authority effect is closed and durably audited**: every
    openat2/ACL/mount/fallocate/link/rename/unlink/read/write/fsync/statfs/key
    effect runs only inside a closed broker handler. The adapter is a pure
    one-to-one mapping. Immutable intent is durable before effect release and
    completion is durable exactly once before success; pending completion audit
    returns `CommitPendingAudit`.

14. **Sealing rotation is one closed, status-first effect**: the Provider commits
    `rotation-pending` before calling only
    `VolumeEffectPort::rotate_sealing_key`; the adapter maps it one-to-one to the
    closed broker `RotateSealingKey` operation. Core and broker authorization
    owners verify the opaque Volume/policy binding and committed preconditions.
    Key bytes, Credential leases, handles, and paths never cross those
    boundaries; crash recovery is roll-forward, retries reuse the canonical
    idempotency key, and success requires durable intent and exactly-once
    completion audit.

---

## Removal proof table

| Baseline artifact | Condition for removal |
| --- | --- |
| `d2b-core/src/storage.rs` (StorageJson contract, all policy enums) | All Provider descriptor consumers on v3 Volume spec; Volume resource parity gate passes |
| `d2b-core/src/sync.rs` (SyncJson, LockSpec) | OFD lock rows replaced by Volume LayoutEntry `leaseClass: file-record`; all callers migrated |
| `d2b-core/src/storage_lifecycle.rs` | Volume controller phase/condition reporting live; daemon startup lifecycle report superseded |
| `d2b-priv-broker/src/ops/swtpm_dir.rs` | device-tpm Provider TPM Volume live; fail-closed tests pass |
| `d2b-priv-broker/src/ops/store_sync.rs`, `store_sync_audit.rs`, `store_sync_export.rs` | volume-local `StoreSyncComplete` broker op live; store-view Volume controller active |
| `d2b-priv-broker/src/ops/store_view_posture.rs` | volume-local store-view LayoutEntry invariants live; posture matrix superseded |
| `d2b-priv-broker/src/ops/store_view_farm.rs` | volume-local store-view Volume controller live |
| `d2b-priv-broker/src/ops/state_dir.rs` | volume-local `ProvisionLayoutEntry` op live; all per-VM state directories managed by Volume controller |
| `d2b-priv-broker/src/ops/storage_contract.rs` | Volume resource parity gate passes; `reconcile_storage_scope` superseded |
| `d2b-priv-broker/src/ops/store_verify.rs` | volume-local marker verification live; store verify superseded |
| `nixos-modules/store.nix` (per-VM hardlink farm activation) | store-view Volume controller live and all parity tests pass |
| `nixos-modules/storage-json.nix` (all path rows) | Volume resources replace every path row; all consumers on bundle format |
| `d2b-state` crate (main `6faa5256`) | All v3 callers migrated to volume-local filesystem primitives; no remaining consumers |
| `d2b-host/src/hardlink_farm.rs` | volume-local `StoreSyncComplete` op fully supersedes; all 14 tests migrated |
| `d2b-contract-tests/tests/storage_sync_contracts.rs` | Superseded by Volume resource parity gate; new gate includes opaque-id contract coverage |
| `tests/unit/nix/cases/per-vm-state-ownership.nix` | Superseded by v3 Volume LayoutEntry matrix test |
| `tests/unit/smoke/smoke-eval-tpm.nix` (swtpm layout) | Superseded by volume-local TPM Volume conformance test |

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass -
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.
