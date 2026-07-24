# ADR 0046 Provider dossier: `Provider/system-core`

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-system-core` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `packages/d2b-provider-system-core`, fixed core-controller process |
| Depends on | `ADR-046-provider-model-and-packaging`, `ADR-046-core-controllers`, `ADR-046-resources-host-guest-process-user`, `ADR-046-resources-zone-control`, `ADR-046-componentsession-and-bus`, `ADR-046-nix-configuration`, `ADR-046-telemetry-audit-and-support`, `ADR-046-current-code-migration-map` |
| Supersedes | Current `WorkloadProviderKind::UnsafeLocal`, host/User grouping helpers, `HostJson`/`VmRuntimeRow` host-management paths in `d2bd` and `d2b-realm-core` |

---

## 1. Scope and bootstrap exception

`Provider/system-core` owns exactly two ResourceTypes:

- `Host` — physical/local host execution, policy, and budget parent;
- `User` — named host identity, UID/session observation, ACL/process subject.

It is one of the two bootstrap exceptions defined in
[`ADR-046-provider-model-and-packaging`](../ADR-046-provider-model-and-packaging.md):
`Provider/system-core` and `Provider/system-minijail` are the only Providers
that do not run as Process resources. `Provider/system-core` Host and User
handlers run inside the fixed `d2b-core-controller` process. `Provider/system-minijail`
is a separate fixed bootstrap controller process; its own dossier and package
define its binary name and placement. Neither is a handler inside the other's
process. Both fixed processes start before any other Process resource exists.
Every other Provider controller is a Process resource launched after
`Provider/system-core` creates the first Host.

This is only a **Process-resource bootstrap exception**. It is not a state
storage, status-ownership, or process-effect exception:
`Provider/system-core` declares no Provider state Volume, its
`ProviderStateSet` is empty, and its bounded non-secret operational state is
reconstructible from resource status, the core Operation ledger, and a resource
store relist. There is no hidden bootstrap store or bootstrap-state Volume.
Core-controller infrastructure creates the runtime-owned Provider resource and
writes `Provider.status`; the system-core handlers write only `Host.status` and
`User.status`. Process Providers remain the sole owners of Process launch,
stop, adoption, quarantine, and their `ProcessEffect` records.

`Provider/system-core` does **not** own:

| ResourceType | Owner |
| --- | --- |
| `Process` / `EphemeralProcess` | `Provider/system-systemd`, `Provider/system-minijail` |
| `Volume` | `Provider/volume-local`, `Provider/volume-virtiofs` |
| `Network` | `Provider/network-local` |
| `Device` | Device Providers |
| `Credential` | Credential Providers |
| Any semantic runtime/desktop/cloud type | Respective Provider |

`unsafe-local` is **not** a Provider. It is a user-only `Host` resource
reconciled by `Provider/system-core` with `defaultDomain=user`,
`allowedDomains=[user]`, and an explicit `defaultUserRef`. See §9 for the
complete no-isolation posture contract.

---

## 2. Provider identity and package

### 2.1 Canonical ResourceRef

```text
Provider/system-core
```

### 2.2 Crate location

```text
packages/d2b-provider-system-core/
├── src/
├── tests/
├── integration/
└── README.md
```

The crate produces:

| Output | Role |
| --- | --- |
| `libsystem_core.rlib` | Provider library: Host/User controller handlers, manifest, and component descriptors |
| `provider-system-core-manifest.json` | Compiled provider manifest installed into the private artifact catalog |

There is no `src/main.rs` in this crate. The binary `d2b-core-controller` is
owned by the separate `packages/d2b-core-controller` crate, which links
`libsystem_core.rlib` (and the other core handler libraries) into one fixed
process. The system-core handler pair must be in-process with the core-controller
runtime because they are available before the first Host resource exists.

### 2.3 Provider resource spec

> **Bootstrap note**: `Provider/system-core` and the `Zone` self-resource are
> `managedBy=controller` — they are created by the Zone runtime at bootstrap and
> are **never** Nix-authored in the resource bundle. Operators do not declare a
> `Provider/system-core` resource in `d2b.zones.<z>.resources`. See §8 for the
> Nix authoring surface (artifact catalog entry and Host/User resource authoring
> only).

```yaml
apiVersion: resources.d2bus.org/v3
type: Provider
metadata:
  name: system-core
  zone: dev
  uid: <store-generated>
  generation: 1
  revision: <opaque>
  ownerRef: null
  finalizers:
    - core.provider-api-binding
  deletionRequestedAt: null
  createdAt: 2026-07-22T00:00:00Z
  updatedAt: 2026-07-22T00:00:00Z
spec:
  artifactId: system-core
  config: {}
# Derived by core-controller infrastructure; not authored in Provider.spec.
status:
  phase: Ready
  observedGeneration: 1
  packageDigest: "sha256:<hex>"
  manifestDigest: "sha256:<hex>"
  configSchemaDigest: "sha256:<hex>"
  signatureId: "d2b-official:<fingerprint>"
  trustEpoch: 1
  conformanceAttestationDigest: "sha256:<hex>"
  exports:
    resourceTypes:
      - name: Host
        version: 1
        schemaDigest: "sha256:<hex>"
      - name: User
        version: 1
        schemaDigest: "sha256:<hex>"
  components:
    controllers:
      - id: host-controller
        phase: Ready
      - id: user-controller
        phase: Ready
  dependencyHealth: []
  disabledCondition: null
```

> **Provider status ownership**: `Provider/system-core` is the fixed core
> bootstrap provider. Its `Provider.status` fields (`phase`, `observedGeneration`,
> `packageDigest`, `manifestDigest`, component phases, etc.) are derived and
> written entirely by the core-controller infrastructure. The system-core
> handlers do **not** write `Provider.status`. system-core handlers own only
> `Host.status` and `User.status` update calls via ResourceClient.

---

## 3. Root configuration schema

`Provider/system-core` has no operator-tunable root configuration. Its behavior
is fully determined by the Host and User resources it reconciles. The config
schema is an empty closed object:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "d2b-provider-system-core/config/v1",
  "type": "object",
  "additionalProperties": false,
  "properties": {},
  "required": []
}
```

All fields that influence Host and User behavior live in those resource specs,
not in Provider config. An operator-supplied `config` field with any non-empty
content is rejected at admission with a structured error:
`spec-validation-error: config must be an empty object for Provider/system-core`.

No secrets are accepted in Provider config. Credential material is a
`Credential` ResourceRef on a Process, not a Provider config field.

---

## 4. Owned ResourceTypes

### 4.1 Host

Complete normative contract: [`ADR-046-resources-host-guest-process-user`](../ADR-046-resources-host-guest-process-user.md).
This section records the system-core–specific controller behavior.

#### 4.1.1 Spec schema (normative summary)

```yaml
apiVersion: resources.d2bus.org/v3
type: Host
metadata:
  name: host-system        # ^[a-z][a-z0-9-]*$; max 63
  zone: dev
spec:
  providerRef: Provider/system-core   # fixed; any other value rejected at admission
  defaultDomain: system               # system|user; default system
  allowedDomains: [system, user]      # [system|user]; 1..2 unique items
  defaultUserRef: null                # User/<name>; required when user ∈ allowedDomains
  budget: {}                          # BudgetSpec aggregate across all Processes
  networkAttachments: []              # 0..64 NetworkAttachmentList entries
  deviceAttachments:  []              # 0..64 DeviceAttachmentList entries
  volumeAttachmentDefaults: []        # 0..64 VolumeAttachmentDefaultList entries
  isolationPosture: null              # promoted base field; null | "none"; see §9
  provider:
    schemaId: system-core.d2bus.org/Host/spec
    schemaVersion: 1.0.0
    settings:                         # system-core Host implementation schema
      kernelVersionMin: null          # min kernel semver string; null = no requirement
      capabilities: []                # HostCapabilityClass[] claimed; verified at reconcile
```

`spec.providerRef` must be exactly `Provider/system-core`. Any other providerRef
on a Host resource is rejected at admission with:
`spec-validation-error: Host.spec.providerRef must be Provider/system-core`.

**D089 spec extension contract:** this Provider's implementation-only desired
configuration is carried in `spec.provider.settings` under
`system-core.d2bus.org/Host/spec`; the schema is registered/signed in the manifest,
deny-unknown, bounded, versioned, and validated against `spec.providerRef` at Nix
build and API admission. Base fields stay at `spec.*`; shared semantics are
promoted to the Host/User base and never placed in `spec.provider`. This
Provider implements the exact base spec/status schema version/fingerprint,
accepts the canonical minimal valid base Spec, and rejects an unsupported
optional base capability only through its signed capability matrix plus
provider-neutral `unsupported-capability`. `spec.provider` aligns with
`status.provider` for `Provider/system-core`.

`isolationPosture` is a promoted Host base field because admission and status use
it across Host implementations; it is not a Provider extension field.

#### 4.1.2 Status schema (normative summary)

```yaml
status:
  observedGeneration: 1
  phase: Ready            # Pending|Ready|Succeeded|Degraded|Failed|Unknown|Deleted
  conditions: []
  lastReconciledAt: "2026-07-22T00:00:01Z"
  startedAt: "2026-07-22T00:00:00Z"
  completedAt: null
  outcome: null
  resource:
    capabilities: []        # HostCapabilityClass[] observed
    kernelRelease: ""       # bounded 64 chars; diagnostic only; not in audit payloads
    osName: ""              # bounded 128 chars
    userManagerAvailable: false
    isolationPosture: null  # null | "none"
    activeProcessCount: 0
```

Per D088, the universal `ResourceStatus` fields remain at top-level
`status.*`; Host ResourceType-common observation written by system-core lives
only in `status.resource` and is identical across Host implementations. Any
future bounded, non-secret system-core-only Host observation must use
`status.provider` with `providerRef: Provider/system-core`, a qualified
`schemaId` such as `system-core.d2bus.org/Host/status`, `schemaVersion` (semver MAJOR.MINOR),
`observedProviderGeneration`, and a strict unknown-field-denied, ≤32 KiB,
redacted `details` object registered and signed in the Provider manifest. A
Host status write updates all present layers atomically in one mutation; shared
fields are promoted to `status.resource` and never duplicated into
`status.provider`.

No PID, pidfd, socket path, internal node name, raw diagnostic, cgroup path,
or host filesystem path appears in public status or audit.

#### 4.1.3 Conditions

| Condition type | Ready=True when | reason codes |
| --- | --- | --- |
| `HostAvailable` | Host OS reachable; kernel version requirement met | `host-unreachable`, `kernel-too-old`, `cgroup-unavailable` |
| `CapabilitiesVerified` | All spec.capabilities observed | `capability-absent-<class>` |
| `UserManagerReady` | user domain allowed and user manager reachable | `user-manager-unavailable`, `user-manager-unknown` |
| `PolicyValid` | spec invariants pass admission | `spec-invalid-domains`, `spec-missing-default-user-ref`, `spec-isolation-posture-conflict` |
| `BudgetAdmitted` | aggregate budget within Zone capacity | `budget-overcommit` |
| `NoIsolation` | always False for user-only Hosts (status=True means no isolation) | — |

`NoIsolation` condition is set to `True` on every user-only Host
(`defaultDomain=user`, `allowedDomains=[user]`). It is never absent or
clearable. The condition message is fixed:
`"This host resource runs processes as the authenticated user with no isolation boundary. All child processes share the host user environment."`.

Phase is `Ready` only when `HostAvailable` and `CapabilitiesVerified` are True.
`UserManagerReady=False` produces `Degraded` (not `Failed`) for system-only
Hosts; it produces `Failed` for user-domain Hosts after the bounded retry
threshold.

#### 4.1.4 Reconcile algorithm

Trigger classes: `spec-generation-changed`, `dependency-changed`,
`startup-relist`, `scheduled-observe`.

```
1. Read fresh Host spec snapshot via ResourceClient (expected revision pinned).
2. Validate spec invariants:
   a. providerRef == Provider/system-core (hard reject otherwise).
   b. allowedDomains validity (no duplicates; known values only).
   c. defaultUserRef set iff user ∈ allowedDomains.
   d. isolationPosture consistency (§9 bidirectional validation).
   e. BudgetSpec field bounds.
3. Probe local OS availability:
   a. uname(2): record kernelRelease.
   b. cgroup v2 mount check.
   c. Unprivileged user namespace check (for capability user-namespace claim).
   d. Check each HostCapabilityClass in spec.capabilities with a bounded
      provider-specific probe (timeout 5 s per probe; total probe budget 15 s).
4. If user ∈ allowedDomains and defaultUserRef is set:
   a. Contact fixed user supervisor via local ComponentSession.
   b. Verify user manager is reachable for defaultUserRef UID (bounded 3 s timeout).
   c. Record userManagerAvailable.
5. List non-terminal Processes targeting this Host:
     aggregate = sum(p.spec.budget for p in List(executionRef=Host/<name>))
6. Validate aggregate budget <= Host spec budget (and <= Zone capacity from policy).
7. Write Host status via UpdateStatus(expectedRevision):
   - capabilities observed, kernelRelease, osName, userManagerAvailable,
     isolationPosture, activeProcessCount, conditions.
8. If spec-generation-changed and budget reduced below current aggregate:
   emit reconcile hints for over-limit Processes.
9. Return converged or pending(requeueAt).

Reconcile must complete within 5 s. Synchronous kernel/libc calls — uname(2),
cgroup stat, capability probes — run in explicit bounded blocking adapters
(e.g., `tokio::task::spawn_blocking`) with per-call deadlines and cancellation.
No synchronous probe is called directly in an async task without a blocking
adapter wrapper.
```

#### 4.1.5 Finalize

No finalizer from system-core. Deletion is blocked by structural check when any
non-terminal Process or EphemeralProcess has `executionRef: Host/<name>`.
Operators must delete all targeting Processes before Host deletion proceeds.
The store transaction writes the Deleted revision and removes the row and index
entry; the `ResourceDeleted` audit event is appended afterward with
dedup/exactly-once recovery. There is no separately observable `phase: Deleted`
state between structural-check clearance and row removal.

#### 4.1.6 RBAC

| Verb | Restriction |
| --- | --- |
| `get` | Standard grant |
| `list` | Standard grant |
| `watch` | Credit-bounded stream |
| `create` | Config publication controller only; subsequent changes via config publication |
| `update-spec` | Config publication controller only |
| `update-status` | `Provider/system-core` controller only; requires expected revision and observedGeneration |
| `update-metadata` | Bounded labels/annotations; no ownerRef changes |
| `delete` | Blocked while non-terminal Process or EphemeralProcess targets this Host |

---

### 4.2 User

Complete normative contract: [`ADR-046-resources-host-guest-process-user`](../ADR-046-resources-host-guest-process-user.md).

#### 4.2.1 Spec schema (normative summary)

```yaml
apiVersion: resources.d2bus.org/v3
type: User
metadata:
  name: alice          # ^[a-z][a-z0-9-]*$; max 63; Zone-local identity
  zone: dev
spec:
  osUsername: alice    # OS username for NSS getpwnam; 1..255 bytes; no NUL/control/slash
  displayName: ""      # max 128 chars; not used for NSS lookup
  groups: []           # 0..64 OS group names to verify; each max 63 chars
```

`metadata.name` and `spec.osUsername` may differ. `metadata.name` must
satisfy `^[a-z][a-z0-9-]*$`; `spec.osUsername` follows OS username rules
(may contain underscores, etc.).

No credential material, SSH key, PAM configuration, or token appears in User
spec. Credentials are `Credential` resources.

#### 4.2.2 Status schema (normative summary)

```yaml
status:
  observedGeneration: 1
  phase: Ready          # Pending|Ready|Succeeded|Degraded|Failed|Unknown|Deleted
  conditions: []
  lastReconciledAt: "2026-07-22T00:00:01Z"
  resource:
    uid: null             # u32; discovered OS UID; diagnostic; not an authz input
    gid: null             # u32; discovered primary OS GID
    homeExists: false
    shellValid: false
    sessionManagerAvailable: false
    groupMembershipVerified: false
    observedGroups: []    # 0..256 OS group names; max 63 chars each
```

Per D088, User ResourceType-common observation written by system-core lives only
in `status.resource`. Any future bounded, non-secret system-core-only User
observation uses `status.provider` with `providerRef: Provider/system-core`, a
qualified `schemaId` such as `system-core.d2bus.org/User/status`, `schemaVersion` (semver MAJOR.MINOR),
`observedProviderGeneration`, and a strict unknown-field-denied, ≤32 KiB,
redacted `details` object registered and signed in the Provider manifest. The
controller writes the universal base, `status.resource`, and any
`status.provider` layer atomically; shared User fields are never duplicated into
the Provider extension.

Numeric UIDs and GIDs are diagnostic only and must not be used as authorization
inputs. Authorization uses the canonical `User/<name>` ResourceRef.

#### 4.2.3 Conditions

| Condition type | Ready=True when | reason codes |
| --- | --- | --- |
| `UserFound` | NSS getpwnam returns a record | `nss-lookup-failed`, `nss-lookup-timeout`, `user-not-found` |
| `HomeReady` | homeExists is true | `home-directory-missing`, `home-directory-inaccessible` |
| `ShellValid` | shellValid is true | `login-shell-missing`, `login-shell-invalid` |
| `GroupsVerified` | spec.groups present in observed membership | `group-membership-missing`, `group-not-found` |
| `SessionManagerReady` | sessionManagerAvailable is true | `session-manager-unavailable`, `session-manager-unknown` |

Phase is `Ready` when `UserFound` and `HomeReady` are both True.
`SessionManagerReady=False` produces `Degraded`. `GroupsVerified=False`
produces `Degraded` when `spec.groups` is non-empty. `Failed` requires
persistent unrecoverable NSS failure beyond the consecutive-failure threshold.

#### 4.2.4 Reconcile algorithm

Trigger classes: `spec-generation-changed`, `dependency-changed`,
`startup-relist`, `scheduled-observe`.

```
1. Read fresh User spec snapshot.
2. Perform NSS getpwnam(spec.osUsername) with 5 s bounded timeout.
3. If NSS lookup fails:
   a. Increment consecutive-failure counter (stored in handler checkpoint).
   b. Write UserFound=False, phase=Degraded (Failed after threshold 5).
   c. Return requeue-at (exponential backoff, max 60 s).
4. Record uid, gid from NSS record; reset consecutive-failure counter.
5. Check home directory existence (blocking stat in bounded adapter, 2 s timeout).
6. Check login shell existence (blocking stat in bounded adapter, 2 s timeout).
7. If spec.groups non-empty: check group membership via getgrouplist or
   /proc/groups; bounded 3 s timeout. Record observedGroups.
8. If allowedDomains on any referencing Host contains user:
   check systemd user manager reachability for this uid (bounded 3 s).
9. Write User status via UpdateStatus(expectedRevision):
   uid, gid, homeExists, shellValid, sessionManagerAvailable,
   groupMembershipVerified, observedGroups, conditions.
10. Return converged or pending(requeueAt).
```

Reconcile must complete within 5 s. All filesystem and IPC calls — NSS
`getpwnam`, `stat(2)`, user-manager socket IPC — are synchronous kernel/libc
APIs called in explicit bounded blocking adapters with per-call deadlines and
cancellation. No synchronous call is made directly in an async task without a
blocking adapter wrapper.

#### 4.2.5 Finalize

No finalizer from system-core. Deletion is blocked by structural check when
any Process has `userRef: User/<name>` or any Volume has `ownerRef: User/<name>`.
Operators must remove all such references before deletion proceeds. The store
The store transaction writes the Deleted revision and removes the row and index
entry; the `ResourceDeleted` audit event is appended afterward with
dedup/exactly-once recovery. There is no separately observable `phase: Deleted`
state between structural-check clearance and row removal.

#### 4.2.6 RBAC

| Verb | Restriction |
| --- | --- |
| `get` | Numeric UID/GID fields in status may require elevated role per Zone policy |
| `list` | Standard grant |
| `watch` | Standard grant |
| `create` | Config publication controller or Provider/system-core bootstrap only |
| `update-spec` | Config publication controller |
| `update-status` | `Provider/system-core` controller only |
| `delete` | Blocked while any Process has `userRef: User/<name>` or any Volume has `ownerRef: User/<name>` |

---

## 5. Controller components

### 5.1 Component table

`Provider/system-core` declares two controller components:

| Component ID | Type | Owned ResourceTypes | Binary | Placement |
| --- | --- | --- | --- | --- |
| `host-controller` | controller | `Host` | `d2b-core-controller` | Fixed; part of core-controller process |
| `user-controller` | controller | `User` | `d2b-core-controller` | Fixed; part of core-controller process |

Both components run as handlers inside the single fixed core-controller process.
The `d2b-core-controller` binary is built by `packages/d2b-core-controller` and
links `libsystem_core.rlib` from this crate. This is the bootstrap exception:
the handlers are in-process with the core-controller runtime and do not require
a Host resource to exist before starting.

### 5.2 Component descriptors

#### host-controller

```yaml
id: host-controller
type: controller
binaryRef: d2b-core-controller
resourceTypes: [Host]
allowedDomains: [system]        # host-controller only runs in system domain
cardinality: 1                  # one instance in the fixed process
config:                         # host-controller inherits Provider config (empty)
  projection: {}
dependencies: []
watchSelectors:
  - resourceType: Host
  - resourceType: User          # User changes trigger Host reconcile for budget/userManager checks
ownerChildTriggers: []
finalizerIds: []                # No finalizers; deletion blocked by structural check
reconcileConcurrency: 8
observeConcurrency: 4
maxPendingResources: 256
observeIntervalSeconds: null
processConstraints: null        # Not a Process resource; no sandbox/budget spec
permissionClaims:
  - resourceType: Host
    verbs: [create, update-status, update-metadata]
  - resourceType: User
    verbs: [get, list, watch]
  - resourceType: Process
    verbs: [list]               # for aggregate budget computation
```

#### user-controller

```yaml
id: user-controller
type: controller
binaryRef: d2b-core-controller
resourceTypes: [User]
allowedDomains: [system]
cardinality: 1
dependencies: []
watchSelectors:
  - resourceType: User
  - resourceType: Process       # Process userRef changes trigger User reconcile
  - resourceType: Volume        # structural-check watch: ownerRef=User/<name> changes trigger User reconcile for deletion-blocking
ownerChildTriggers: []
finalizerIds: []
reconcileConcurrency: 8
observeConcurrency: 4
maxPendingResources: 256
observeIntervalSeconds: null
processConstraints: null
permissionClaims:
  - resourceType: User
    verbs: [create, update-status, update-metadata]
  - resourceType: Process
    verbs: [get, list]
  - resourceType: Volume
    verbs: [get, list]          # structural-check reads only; user-controller does NOT own Volume and does NOT create/delete Volume resources
```

### 5.3 Services

`Provider/system-core` exposes only its fixed host/user controller service to
the Zone resource dispatcher. Operators and other controllers still interact
with Host/User state through the resource API (`d2b.resource.v3` service on
d2b-bus); they do not receive a provider-specific public method surface.

## 5.4 Endpoint resources (D092)

`Provider/system-core` conforms to the standard `Endpoint` base schema for its
fixed bootstrap controller service. Stable service identity is represented as an
owned `Endpoint` resource with `producerRef`; consumers use `Endpoint/<name>`.
Because `d2b-core-controller` is a bootstrap fixed process rather than a
`Process` resource, the producer is the qualified fixed-controller resource below
instead of an inline `Process.spec` field. Endpoint spec/status never carries raw
socket paths, peer credentials, fd numbers, host paths, PIDs, or credentials.
Resolution occurs only through an authorized EffectPort/LaunchTicket;
unauthorized resolution returns `endpoint-resolve-denied`. Producer restart
bumps `Endpoint.status.endpointGeneration`, causing dependents to observe
`dependency-changed`.

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: system-core-host-user-control
  zone: dev
  ownerRef: Provider/system-core
spec:
  providerRef: Provider/system-core
  producerRef: system-core.d2bus.org/FixedController/d2b-core-controller
  endpointClass: control
  transport: unix
  purpose: system-core.d2bus.org/host-user-control
  serviceFingerprint: system-core.d2bus.org/HostUserControl.v3
  locality: host-local
  visibility: provider
  attachmentPolicy: component-session
  consumerPolicy:
    allowedProviderComponents: [system-core.d2bus.org/zone-runtime]
    allowedOperations: [resolve]
  lifecyclePolicy: recycle-with-producer
status:
  readiness: Ready
  observedProducerGeneration: 1
  observedResourceGeneration: 1
  endpointGeneration: 1
  connectionAvailability: available
  leaseAvailability: lease-required
```

## 5.5 Retained opaque handles (D092 promotion test)

- Inherited d2b-bus socketpair fd indexes are bootstrap attachment slots and stay
  opaque.
- `OwnedTransport` and ComponentSession generation handles are in-memory bus
  capabilities behind Endpoint resolution, not addressable resources.
- Resource revision cursors, checkpoints, and `operationId` values remain opaque
  status/ledger correlation handles.
- system-core is not a Process Provider and holds no pidfds across reconcile
  calls; any observed PID remains a transient diagnostic, not an Endpoint.

### 5.6 Workers

`Provider/system-core` declares no worker Process templates. Workers are not
required because:

- Host probes (uname, cgroup, capability checks) run inside the reconcile
  handler in explicit bounded blocking adapters, not a separate worker process.
- User NSS lookups (`getpwnam`, group membership) run inside the reconcile
  handler in explicit bounded blocking adapters, not a separate worker.

Isolated remote NSS (e.g. for LDAP/SSSD polling) is excluded from the initial
v3 catalog.

---

## 6. d2b-bus interactions

`Provider/system-core` handlers connect to the Zone runtime's local d2b-bus
endpoint over an inherited local Unix socketpair (NN profile, system domain,
trusted endpoint policy). They use:

| Service | Purpose | Direction |
| --- | --- | --- |
| `d2b.resource.v3` | ResourceClient — list, watch, update-status | outbound from handlers |
| `d2b.resource.v3` | Receive reconcile hints from store post-commit dispatcher | inbound to handlers |

No public external service endpoint is registered on d2b-bus. The fixed
host/user control Endpoint above has `visibility: provider` and admits only the
Zone-runtime component through `consumerPolicy`; it does not expose
provider-specific methods to operators or other controllers.

### 6.1 ResourceClient usage

Both handlers use the standard `d2b-provider-toolkit` async `ResourceClient`:

```
ResourceClient.watch(Host)   — host-controller watch loop
ResourceClient.watch(User)   — user-controller watch loop
ResourceClient.watch(User)   — host-controller cross-watch for UserManagerReady
ResourceClient.watch(Process) — host-controller cross-watch for budget aggregation
ResourceClient.watch(Process) — user-controller cross-watch for structural check
ResourceClient.watch(Volume)  — user-controller cross-watch for structural check
ResourceClient.update_status(Host, expectedRevision)
ResourceClient.update_status(User, expectedRevision)
```

### 6.2 Reconcile hint path

Store post-commit dispatcher → d2b-bus hint stream → handler queue.

Performance contract (from [`ADR-046-core-controllers`](../ADR-046-core-controllers.md)):
- p95 durable commit → handler start: ≤5 ms.
- p95 durable commit → launch attempt start (Process after all deps Ready): ≤20 ms.

The system-core handler pair contributes to the overall core-controller
reserved capacity on d2b-bus.

---

## 7. Dependencies and RBAC

### 7.1 Provider dependencies

`Provider/system-core` has no Provider dependencies. It is the first Provider
ready in the Zone and cannot depend on other Providers for its bootstrap.

The following Providers consume system-core's exported ResourceTypes:

| Consumer Provider | Consumes |
| --- | --- |
| `Provider/system-systemd` | `Host` (executionRef), `User` (userRef) |
| `Provider/system-minijail` | `Host` (executionRef), `User` (userRef) |
| Every runtime Provider | `Host` (capability queries via HostCapabilityClass) |
| Every Provider with worker processes on Host | `Host`, `User` |

### 7.2 Bootstrap authorization

At Zone startup, before any Role/RoleBinding resources are processed, the
compiled bootstrap authorization grants these verbs to the exact
`Provider/system-core` subject:

```yaml
- resourceTypes: [Host, User]
  verbs: [create, update-status, update-metadata, get, list, watch]
- resourceTypes: [Process, EphemeralProcess]
  verbs: [get, list, watch]
- resourceTypes: [Volume]
  verbs: [get, list, watch]
```

No verb not in this set is granted through bootstrap. All other verbs require
a Role/RoleBinding resource created by config publication.

### 7.3 Broker operations

`Provider/system-core` uses no privileged broker operations. It does not invoke
`SpawnRunner`, `StoreSync`, `UsbipBindFirewallRule`, or any other broker op.

The host OS probes it performs (uname, cgroup check, stat, NSS lookup) are
unprivileged calls executed inside the fixed core-controller process in explicit
bounded blocking adapters, not broker-mediated privileged operations.

---

## 8. Nix configuration and artifact emission

> **Bootstrap resources are runtime-created**: `Provider/system-core` and the
> `Zone` self-resource are `managedBy=controller`. They are created by the Zone
> runtime at bootstrap and are **never** Nix-authored in the resource bundle.
> Operators do not declare a `Provider/system-core` entry in
> `d2b.zones.<z>.resources`. The Nix surface for system-core is the artifact
> catalog entry (§8.1) and the Host/User resource authoring described in §8.2–§8.4.

### 8.1 Artifact catalog entry

```nix
d2b.artifacts.system-core = {
  package = pkgs.d2b-provider-system-core;
  type    = "provider";
};
```

`pkgs.d2b-provider-system-core` is the Nix derivation that builds
`packages/d2b-provider-system-core`. It produces the provider library
(`libsystem_core.rlib`), the compiled provider manifest
(`provider-system-core-manifest.json`), and component descriptors. The manifest
store path is recorded in the private `artifact-catalog.json` (`root:d2bd` 0640);
it never appears in any public ResourceSpec, status field, audit record, or OTEL
telemetry. The `d2b-core-controller` binary is built by the separate
`packages/d2b-core-controller` derivation.

### 8.2 Host resource authoring

> **No implicit Zone-level primary Host**: there is no Zone-wide default or
> primary Host concept. Process resources reference an exact Host by
> `executionRef: Host/<resource-name>`. The `defaultDomain` and `defaultUserRef`
> fields are per-Host spec fields that govern Processes targeting *that specific
> Host* when they do not supply their own overrides. Nix declarations always name
> exact Host resources.

Standard isolated Host (system and user domains):

```nix
d2b.zones.dev.resources.host-system = {
  type = "Host";
  spec = {
    providerRef    = "Provider/system-core";
    defaultDomain  = "system";
    allowedDomains = ["system" "user"];
    defaultUserRef = "User/alice";
    budget         = {};
    isolationPosture = null;
    provider = {
      schemaId = "system-core.d2bus.org/Host/spec";
      schemaVersion = "1.0.0";
      settings.capabilities = ["kvm" "pidfd" "cgroup-v2" "virtiofs"];
    };
  };
};
```

Rendered canonical JSON:

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type": "Host",
  "metadata": {
    "name": "host-system",
    "zone": "dev"
  },
  "spec": {
    "providerRef": "Provider/system-core",
    "defaultDomain": "system",
    "allowedDomains": ["system", "user"],
    "defaultUserRef": "User/alice",
    "budget": {},
    "networkAttachments": [],
    "deviceAttachments": [],
    "volumeAttachmentDefaults": [],
    "isolationPosture": null,
    "provider": {
      "schemaId": "system-core.d2bus.org/Host/spec",
      "schemaVersion": "1.0.0",
      "settings": {
        "kernelVersionMin": null,
        "capabilities": ["kvm", "pidfd", "cgroup-v2", "virtiofs"]
      }
    }
  }
}
```

### 8.3 Unsafe-local (user-only) Host authoring

```nix
d2b.zones.dev.resources.host-user-shell = {
  type = "Host";
  spec = {
    providerRef     = "Provider/system-core";
    defaultDomain   = "user";
    allowedDomains  = ["user"];
    defaultUserRef  = "User/alice";
    isolationPosture = "none";     # required; cannot be null for user-only
    provider = {
      schemaId = "system-core.d2bus.org/Host/spec";
      schemaVersion = "1.0.0";
      settings = {};
    };
  };
};
```

Eval assertions:
- `isolationPosture` set to any value other than `"none"` for a user-only
  Host (`defaultDomain=user`, `allowedDomains=["user"]`) is rejected at eval
  time with:
  `spec-validation-error: Host.spec.isolationPosture must be "none" for user-only hosts`.
- A user-only Host with `isolationPosture=null` is rejected with:
  `spec-validation-error: Host.spec.isolationPosture must be set to "none" when allowedDomains=["user"]`.
- A `Process` with `executionRef: Host/host-user-shell` and `domain: system`
  is rejected at eval time.
- No `Guest` ref is emitted for a user-only Host declaration.

### 8.4 User resource authoring

```nix
d2b.zones.dev.resources.alice = {
  type = "User";
  spec = {
    osUsername  = "alice";
    displayName = "Alice Example";
    groups      = ["d2b" "audio" "video"];
  };
};
```

Rendered canonical JSON:

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type": "User",
  "metadata": {
    "name": "alice",
    "zone": "dev"
  },
  "spec": {
    "osUsername": "alice",
    "displayName": "Alice Example",
    "groups": ["d2b", "audio", "video"]
  }
}
```

Eval assertion: `spec.osUsername` must match the OS username regex
`^[a-zA-Z_][a-zA-Z0-9_.-]*$` (POSIX portable character class); violation
produces a structured eval error identifying the resource name and field.

### 8.5 Nix schema drift gate

The spec fields and default values described in §8.2–§8.4 are the authoritative
source. Generated Nix option types and documentation are derived from the same
ResourceTypeSchema and Provider schema used for build-time validation. The
`make test-drift` gate (`xtask gen-nix-options` + `git diff --exit-code`)
enforces that the two sources never diverge.

---

## 9. No-isolation Host posture

### 9.1 Semantic property

The user-only `Host` (`defaultDomain=user`, `allowedDomains=[user]`) with
`isolationPosture="none"` is the v3 successor to `kind="unsafe-local"` in
the current Nix module. It is not an implementation gap or a transitional state.
User processes share the host UID, filesystem, and environment. This posture is
explicit, persistent, and non-suppressible across four surfaces:

| Surface | Required behavior |
| --- | --- |
| **Host.status** | `isolationPosture: "none"` always set by reconciler; never absent or clearable. `NoIsolation` condition: `status=True`, `message` fixed text (see §4.1.3). |
| **CLI/UI** | `d2b zone list` and `d2b zone inspect` always render `⚠ no isolation boundary (user domain)` beside user-only Hosts. Warning is not suppressible via flag or environment variable. |
| **Audit** | Process Providers (`Provider/system-systemd`, `Provider/system-minijail`) emit `ProcessEffect` audit records for every process launch/stop/adopt/quarantine. For processes under a user-only Host, they query `Host.status.isolationPosture` and embed `no_isolation: true` in the record. This field is required and never absent for such records. system-core does not emit `ProcessEffect` records; it supplies the `isolationPosture` data by keeping `Host.status` accurate. |
| **OTEL** | `no_isolation=true` is NOT a metric label, span attribute, or log field. It must not appear in any telemetry dimension. |

### 9.2 Spec admission invariants

The following combinations are hard-rejected at spec admission:

| Condition | Error |
| --- | --- |
| `defaultDomain=user` + `allowedDomains=["user"]` + `isolationPosture=null` | `spec-validation-error: isolationPosture must be "none" for user-only hosts` |
| `isolationPosture="none"` + `system` ∈ `allowedDomains` | `spec-validation-error: isolationPosture cannot be "none" when system domain is allowed` |
| `isolationPosture="none"` + `defaultDomain=system` | `spec-validation-error: isolationPosture cannot be "none" for system-domain hosts` |
| `allowedDomains=["user"]` + `defaultUserRef=null` | `spec-validation-error: defaultUserRef required when allowedDomains contains user` |
| Process with `executionRef=Host/<user-only>` + `domain=system` | `spec-validation-error: system-domain processes cannot target a user-only host` |

### 9.3 Migration from unsafe-local

| Current baseline field | v3 target |
| --- | --- |
| `nixos-modules/options-realms-workloads.nix` `kind = "unsafe-local"` | `Host` resource with `isolationPosture="none"` |
| `WorkloadProviderKind::UnsafeLocal` | Host resource; no Provider variant |
| `IsolationPosture::UnsafeLocal` | `Host.status.isolationPosture="none"` + `NoIsolation` condition |
| `nixos-modules/unsafe-local-workloads-json.nix` | Nix emitter producing `Host` resource spec |
| `nixos-modules/unsafe-local-helper.nix` | Retired after Process Provider supervisor ticket migration |
| `d2b-unsafe-local-helper` binary | Retired after Process Provider supervisor ticket migration |
| `DaemonToUnsafeLocalHelper`/`UnsafeLocalHelperToDaemon` protocol | Retired |

---

## 10. Provider state (ProviderStateSet)

A **ProviderStateSet** is the optional, query-time grouping of the *declared*
`Volume` resources in a Zone whose `metadata.ownerRef` resolves to
`Provider/system-core`. It is **not a ResourceType**, no `ProviderStateSet`
resource is created in the store, and it is empty for a Provider that declares
no state Volume. The normative contract is
[`ADR-046-provider-state`](../ADR-046-provider-state.md).

### 10.1 No component state Volume

`Provider/system-core` and its `host-controller` and `user-controller`
components declare **no** Provider state Volume; its `ProviderStateSet` is
empty. Bounded non-secret operational state belongs in the owning resource's
`status` subresource and the core Operation ledger by default (D087). The Host
and User resources in the Zone resource store are the sole durable state for
Host and User concerns; handler checkpoints (watch high-water revision,
consecutive-failure counters) live in the core-controller's embedded
reconcile-engine checkpoint table and are fully reconstructible from a relist
after restart. system-core is not a Process Provider and holds no pidfds or open
FDs across reconcile calls.

Because system-core's operational state is fully derivable from spec, `status`,
the core Operation ledger, and a resource-store relist, it fails the
storage-need test: there is no `host-controller`/`user-controller` state
namespace, no state Volume, no state-view mount, and no dedicated state-layout
`User/<name>` principal. There is no empty identity-only Volume.

### 10.2 No bootstrap-state exception

`Provider/system-core` is a fixed bootstrap component. Because it declares no
state Volume and reaches Ready from resource `status`, the core Operation
ledger, and a resource-store relist, it needs no state Volume before
`Provider/volume-local` is ready — so there is no bootstrap state-Volume cycle,
no closed bootstrap storage mechanism, and no bootstrap-storage exception (D086,
superseded by D087; see "No bootstrap state Volume" in
`ADR-046-components-processes-and-sandbox`). No separate bootstrap Provider
process, public resource type, d2b-bus service, or broker operation is
introduced. There is no hidden bootstrap store.

---

## 11. Lifecycle, upgrade, drain, and restart

### 11.1 Startup sequencing

From [`ADR-046-core-controllers`](../ADR-046-core-controllers.md):

```
1. Zone runtime opens redb store.
2. Resource API and local d2b-bus endpoint start.
3. Fixed core-controller process (system-core handlers) and fixed
   system-minijail controller process start concurrently as separate processes.
4. Bootstrap authorization grants exact verbs to Provider/system-core subject.
5. Handlers list/recover checkpoints concurrently.
6. Configuration publication handler activates the current generation.
7. system-core Host handler: relist Hosts, reconcile each.
8. system-core User handler: relist Users, reconcile each.
9. Zone readiness publishes after mandatory handlers are current.
```

The first Host reconcile may discover no Hosts (first-time activation). This
is not an error; the Zone proceeds to Ready with zero active Hosts until the
first Host resource is created by config publication.

### 11.2 Restart

On restart:

1. Core-controller authenticates a new ComponentSession generation.
2. Handlers relist owned resources (Host, User).
3. Handlers resume from durable checkpoints where valid.
4. No cleanup is performed before Host/User/Process owners observe/adopt.
5. Unknown/ambiguous conditions are preserved.
6. No pidfd authority exists (system-core is not a Process Provider).

### 11.3 Upgrade

Provider upgrade for `Provider/system-core` requires a NixOS generation switch
because the `d2b-core-controller` binary (built by `packages/d2b-core-controller`
which links `libsystem_core.rlib`) is the fixed process. The upgrade sequence:

```
1. New generation emitted by Nix build; artifact-catalog.json updated.
2. Configuration publication handler stages the new generation.
3. d2b-activation-helper installs new binary alongside current.
4. Zone performs controlled drain of in-flight reconcile tasks.
5. core-controller process restarts with new binary (systemd unit restart).
6. Handlers relist and resume from stored checkpoints.
7. Prior generation retained for bounded rollback (default 3 generations).
```

No data migration is required for Host and User resources between minor
versions. A major-version bump requires a schema migration plan in the
`src/migration.rs` module.

### 11.4 Drain

When the Zone is drained for shutdown or reset:

1. System-core handlers stop accepting new reconcile work.
2. In-flight reconcile tasks are allowed to complete up to `drainTimeout`.
3. No Host or User resources are deleted by system-core during drain; that
   is the core-controller ownership/finalizer handler's responsibility.
4. After drain, the core-controller process exits cleanly.

---

## 12. Status, errors, audit, and OTEL

### 12.1 Handler health and status

> **Status ownership boundary**: `Provider/system-core` is a fixed core
> bootstrap provider. The `Provider.status` fields on the `Provider/system-core`
> resource (phase, observedGeneration, packageDigest, component phases, etc.)
> are derived and written by the core-controller infrastructure. The system-core
> handlers write only `Host.status` and `User.status` via ResourceClient
> `update_status` calls. No system-core code path calls `update_status` on the
> `Provider/system-core` resource itself.

**Currency and upgrade (D091).** The Host/User handlers implement
`assess_update`, `plan_upgrade`, and `execute_upgrade` for Host and User
currency and populate only the universal `status.update`, never
`status.provider`, with
`state: Current|UpdateAvailable|UpgradeRequired|Upgrading|Blocked|Unknown`,
`reasons` from `CoreGenerationChanged`, `ProviderGenerationChanged`,
`ArtifactChanged`, `ImageOrSystemGenerationChanged`, `SpecChanged`,
`DependencyChanged`, or `SecurityPolicyChanged`, observed/target
generation/digest IDs, `disruption: None|Reload|Restart|Recycle|Replace`,
`preserveState`, optional `operationId`, `lastAssessedAt`, and
`owned`/`dependencies` refs. It honors base `spec.updatePolicy` (manual
disruptive default; auto non-disruptive), while the Core Operation ledger owns
upgrade operation, idempotency, and progress. A core/provider-generation rollout
is expressed as currency; disruptive changes return `UpgradeRequired` rather
than being applied in place, while non-disruptive changes reconcile normally.
`system-core` reaches `Ready` without a state Volume, its own upgrade recycles
only the controller realization, and Host/User identity is preserved.

**Expedited reconcile (D090).** For `Create`, `UpdateSpec`, or `Delete` with
`waitForReconcile`, the Host/User handlers perform no external effect, finalizer
mutation, or status mutation until Core supplies
`CommittedRevisionProof {resourceUid,generation,revision,operationId}`. `Abort`
means no effect; a durable commit is never rolled back after a later reconcile
timeout. The response contains the committed object, one-pass projected layered
status, `disposition: Converged|Progressing|Blocked|UpgradeRequired|Failed`,
and `statusPersistence: pending|committed`; effect idempotency keys derive from
`(UID,generation,revision,operationId)` in the same per-resource single-flight
using a bounded priority lane.

Each system-core handler reports to the core-controller aggregate:

```yaml
handler: system_core_host    # stable closed name
phase: Ready
lastReconciledAt: "2026-07-22T00:00:01Z"
queueDepth: 0
runningCount: 0
lastWatchRevision: 42
```

```yaml
handler: system_core_user
phase: Ready
lastReconciledAt: "2026-07-22T00:00:01Z"
queueDepth: 0
runningCount: 0
lastWatchRevision: 38
```

These appear in `Zone.status.handlers` as name/phase/lastReconciledAt entries.
No resource names, counts by type, or provider diagnostics appear in that list.

### 12.2 Stable error codes

All error reason codes produced by system-core are stable and form a closed set:

| Error code | Surface | Meaning |
| --- | --- | --- |
| `host-unreachable` | Host condition | OS kernel/cgroup probe failed |
| `kernel-too-old` | Host condition | Observed kernel older than spec.kernelVersionMin |
| `cgroup-unavailable` | Host condition | cgroup v2 delegation not available |
| `capability-absent-<class>` | Host condition | Claimed HostCapabilityClass not observed |
| `user-manager-unavailable` | Host/User condition | systemd user manager not running |
| `user-manager-unknown` | Host/User condition | User manager status indeterminate |
| `spec-invalid-domains` | Host condition | allowedDomains/defaultDomain/defaultUserRef inconsistency |
| `spec-missing-default-user-ref` | Host condition | user ∈ allowedDomains without defaultUserRef |
| `spec-isolation-posture-conflict` | Host condition | isolationPosture/allowedDomains combination invalid |
| `budget-overcommit` | Host condition | Aggregate Process budget exceeds Host budget |
| `nss-lookup-failed` | User condition | NSS getpwnam returned error |
| `nss-lookup-timeout` | User condition | NSS getpwnam exceeded 5 s timeout |
| `user-not-found` | User condition | NSS getpwnam returned no entry |
| `home-directory-missing` | User condition | home directory not found |
| `home-directory-inaccessible` | User condition | home directory stat failed |
| `login-shell-missing` | User condition | login shell path not found |
| `login-shell-invalid` | User condition | login shell is empty or non-executable |
| `group-membership-missing` | User condition | One or more spec.groups not in observed membership |
| `group-not-found` | User condition | OS group name from spec.groups not in /etc/group |
| `session-manager-unavailable` | User condition | systemd user manager not reachable for this user |
| `session-manager-unknown` | User condition | User manager status indeterminate |
| `spec-validation-error` | Admission | Provider config or spec field validation failed |

No error message carries a filesystem path, PID, UID, GID, cgroup path, socket
path, executable path, argv, or operator-chosen identifier beyond the resource
name. The condition `message` field is a bounded bounded human-readable
description capped at 256 chars; it uses generic descriptions only.

### 12.3 Audit records

#### system-core audit responsibility

`Provider/system-core` does **not** emit `ProcessEffect` audit records.
`ProcessEffect` (launch, stop, adoption, quarantine) records are owned by the
Process Providers — `Provider/system-systemd` and `Provider/system-minijail`.
Those providers are required to query `Host.status.isolationPosture` before
emitting each `ProcessEffect` record and to embed `no_isolation: true` in the
record when the parent Host has `isolationPosture="none"`. Audit consumers must
not need to join resource status to determine isolation posture.

system-core's audit responsibilities are:

- **`ResourceReconciled` events** for every Host and User reconcile (see below).
- **`Host.status` accuracy**: keeping `isolationPosture` and the `NoIsolation`
  condition always current so Process Providers can rely on them at launch time.

#### ResourceReconciled

Every Host and User reconcile emits one `ResourceReconciled` event (kind=info
if converged, kind=warning if Degraded, kind=error if Failed):

```json
{
  "record_class": "resource-reconciled",
  "zone": "<zone_name>",
  "resource_type": "Host",
  "resource_name_digest": "sha256:<hex>",
  "resource_uid": "<opaque>",
  "generation": 1,
  "observed_generation": 1,
  "outcome": "converged",
  "handler": "system_core_host",
  "conditions_summary": "HostAvailable=True CapabilitiesVerified=True"
}
```

`resource_name_digest` is the SHA-256 of `ResourceType/resource_name`. The
raw resource name does not appear in audit records.

### 12.4 OTEL telemetry

#### Resource attributes

All core-controller processes (including system-core handlers) stamp:

```
service.name   = "d2b-core-controller"
service.version = "<CARGO_PKG_VERSION>"
d2b.zone        = "<zone_name>"              # resource attribute; not a metric label
d2b.provider    = "system-core"
d2b.component   = "host-controller" | "user-controller"
```

No user name, UID, GID, OS username, cgroup path, or executable path is
stamped in resource attributes.

#### Metric labels (closed set)

`handler` values from the core-controller metrics (see
[`ADR-046-telemetry-audit-and-support`](../ADR-046-telemetry-audit-and-support.md)):

```
system_core_host
system_core_user
```

These are the only two `handler` label values system-core contributes to
`d2b_controller_*` metrics.

`no_isolation` is not a metric label. It must not appear in any telemetry
dimension.

`provider="unsafe-local"` (current baseline metric label in
`packages/d2bd/src/shell_handler.rs` tracing) is removed entirely in v3.
No metric carries this label value. See removal proof in §14.

#### Required instruments

| Metric | Labels | Notes |
| --- | --- | --- |
| `d2b_controller_reconcile_total` | `handler=system_core_host\|system_core_user`, `outcome` | Standard controller instrument |
| `d2b_controller_reconcile_duration_seconds` | `handler`, `outcome` | p95 target ≤5 s |
| `d2b_controller_queue_depth` | `handler` | Instantaneous queue size |
| `d2b_controller_hint_to_handler_seconds` | `handler` | p95 target ≤5 ms |
| `d2b_controller_watch_revision_lag` | `handler` | Revision lag vs store head |

---

## 13. Performance bounds

All bounds are normative and subject to the conformance test matrix.

| Bound | Value | Source |
| --- | --- | --- |
| Durable trigger commit → handler start (p95) | ≤5 ms | ADR-046-core-controllers §controller-registration-and-hints |
| Durable commit → launch attempt (p95) | ≤20 ms | ADR-046-resources-host-guest-process-user §fast-path-contract |
| Host reconcile wall clock | ≤5 s | §4.1.4 above |
| User reconcile wall clock | ≤5 s | §4.2.4 above |
| NSS getpwnam timeout | 5 s | §4.2.4 above |
| Per-HostCapabilityClass probe timeout | 5 s | §4.1.4 above |
| Total capability probe budget per reconcile | 15 s | §4.1.4 above |
| User manager reachability check timeout | 3 s | §4.1.4 above |
| Group membership check timeout | 3 s | §4.2.4 above |
| Home/shell stat timeout | 2 s per call | §4.2.4 above |
| Reconcile concurrent handlers (Host) | 8 | §5.2 above |
| Reconcile concurrent handlers (User) | 8 | §5.2 above |
| Max pending resources per handler | 256 | §5.2 above |
| Handler drainTimeout | 30 s | §11.4 above |

Bounded parallel background work: each handler may have at most
`reconcileConcurrency` (8) concurrent reconcile tasks active at once.
All background tasks are tracked; none may escape the bounded task pool.
Status writes are async and do not hold the watch loop.

---

## 14. Pre-ADR45 and main reuse ledger

**Reuse policy**: The pre-ADR45 v3 baseline (`b5ddbed6`) is the factual anchor
for current behavior. Main commit `a1cc0b2d` is evidence of reusable
implementation patterns only. No current v3 behavior is assumed from main.

### 14.1 Current baseline sources (production-reachable at `b5ddbed6`)

| Current source | Evidence class | Disposition | Target |
| --- | --- | --- | --- |
| `packages/d2bd/src/lib.rs` — daemon startup, Host grouping | production-reachable | ADAPT | `d2b-provider-system-core/src/host.rs` |
| `packages/d2b-realm-core/src/node.rs` — `NodeKind::FullHost` | production-reachable | EXTRACT/ADAPT | `Host` ResourceType schema and reconcile |
| `packages/d2b-realm-core/src/workload.rs` — `WorkloadProviderKind::UnsafeLocal`, `IsolationPosture::UnsafeLocal` | production-reachable | ADAPT | `Host.spec.isolationPosture="none"` + `Host.status.isolationPosture="none"` |
| `packages/d2b-realm-core/src/workload.rs` — `WorkloadExecutionPosture` | production-reachable | ADAPT | `Host.spec` (ExecutionPolicy fields) |
| `packages/d2b-core/src/host.rs` — `HostJson`, `VmRuntimeRow` | production-reachable | ADAPT | `Host` ResourceSpec fields; Network/Guest attachments |
| `packages/d2bd/src/unsafe_local_helper.rs` — `HelperRegistry` | production-reachable | ADAPT → RETIRE | `Host` user-domain supervisor; `HelperRegistry::allowed_uids` → `defaultUserRef` |
| `packages/d2b-contracts/src/unsafe_local_wire.rs` — `DaemonToUnsafeLocalHelper`/`UnsafeLocalHelperToDaemon` | production-reachable | DELETE after migration | No v3 equivalent; retired when Process Provider supervisor ticket replaces direct launch |
| `packages/d2b-unsafe-local-helper/src/{main,protocol,runtime,systemd}.rs` | production-reachable | REPLACE → DELETE | User-domain Process; v3 uses normal Process Provider supervisor ticket |
| `nixos-modules/unsafe-local-workloads-json.nix` | nix-emitted | ADAPT | Emitter for `Host` resource spec (user-only variant) |
| `nixos-modules/unsafe-local-helper.nix` — service unit | nix-emitted | DELETE after migration | Fixed user-supervisor unit; retired after Process Provider supervisor ticket migration |
| `packages/d2b-realm-core/src/ids.rs` — `NodeId`, `ProviderId` | production-reachable | EXTRACT/ADAPT | Resource identity types in `d2b-contracts/src/v3/identity.rs` |
| `packages/d2b-realm-core/src/realm.rs` — `RealmControllerPlacement::HostLocal` | production-reachable | ADAPT | Zone runtime bootstrap; `HostLocal` → Zone runtime on `Host` |
| `packages/d2b-contract-tests/tests/policy_observability.rs` — `loki_native_otel_resource_attributes` | test-only | ADAPT | Extend OTEL attribute allowlist with `d2b.zone`, `d2b.provider`, `d2b.component`; add `system_core_host`, `system_core_user` to `handler` closed set |
| `packages/d2bd/src/metrics.rs` — `d2b_daemon_vm_*` with `vm=<name>` label | production-reachable | DELETE from v3 metrics | VM-name labels not carried forward; v3 uses closed `handler` label set |

### 14.2 Main reuse (copy/adapt; NOT current-state evidence)

Selected from main `a1cc0b2d`:

| Main source | Selected behavior | Reuse action |
| --- | --- | --- |
| `packages/d2b-provider-toolkit/src/reconciler.rs` | Async ResourceClient/Reconciler loop; bounded task pool; hint handling | copy and adapt |
| `packages/d2b-provider-toolkit/src/fake_store.rs` | Fake store/bus/supervisor for hermetic tests | copy and adapt |
| `packages/d2b-provider-toolkit/src/conformance.rs` | Provider conformance kit invoked from `tests/` | copy and adapt |
| `packages/d2b-session/src/{handshake,bootstrap,record,engine,scheduler,streams,lifecycle,transport}.rs` | ComponentSession (NN/KK/IKpsk2); basis for local bus connection | copy and adapt (per ADR-046-session-001) |
| `packages/d2b-session-unix/src/{adapter,socket,pidfd}.rs` | Unix peer identity, SO_PEERCRED validation | copy and adapt (per ADR-046-session-002) |
| `packages/d2b-provider/src/lib.rs` — provider trait skeleton | Provider resource/manifest/components model | extract and adapt |

Excluded main assumptions: v2 EndpointRole/Realm/service inventory, Provider
registry/process model v2, delivery tooling, and generated v2 DTO names are
not carried into v3.

### 14.3 Removal proof requirements

The following current artifacts must be retired before work items in this dossier
are considered complete:

| Artifact | Retirement condition | Canonical owner |
| --- | --- | --- |
| `nixos-modules/unsafe-local-helper.nix` service unit | After Process Provider supervisor ticket migration complete; all user-domain Processes launch via normal Process Provider | `ADR046-nix-010` |
| `packages/d2b-unsafe-local-helper/` binary | Same condition as above | `ADR046-exec-009` |
| `packages/d2b-contracts/src/unsafe_local_wire.rs` | After no live caller remains | `ADR046-exec-009` |
| `d2bd` `HelperRegistry::dispatch_launch` path | After Process Provider supervisor ticket migration complete | `ADR046-exec-009` |
| `d2b_daemon_vm_*` metrics with `vm=<name>` label | After v3 controller metrics are verified in integration tests | `ADR046-telem-002` and `ADR046-telem-005` own their respective metric replacements; `ADR046-telem-008` owns the absence proof |
| `provider="unsafe-local"` metric/span label | After shell_handler.rs migrated to Host-aware audit | `ADR046-telem-008` |

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass —
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.

---

## 15. Implementation work items

The former `SC-001` through `SC-006` labels were dossier-local aliases, not
canonical work-item IDs. They are retired and MUST NOT be emitted into the
work-item graph. The following table is normative. An ID in the owner column is
the exclusive owner of the stated destination or proof; dependencies and parent
items do not acquire co-ownership.

| Retired declaration | Canonical implementation ownership | Exact scope and non-ownership boundary | Removal/migration ownership |
| --- | --- | --- | --- |
| `SC-001` | `ADR046-exec-001` | Owns the Host/User DTOs, schemas, bounds, admission vectors, and shared execution-policy extraction. `ADR046-core-002` is the coordination parent only; it owns no duplicate contract destination. | `ADR046-exec-001` owns removal of the Host/User/ExecutionPolicy portions of `HostJson`, `VmRuntimeRow`, and `WorkloadExecutionPosture` after all consuming resource slices reach parity. |
| `SC-002` | `ADR046-exec-003` (Host), `ADR046-exec-004` (User), `ADR046-exec-005` (bootstrap ordering), and `ADR046-system-core-001` (Provider-specific manifest and audit boundary) | `ADR046-core-001` owns only the fixed core-controller process frame. Host and User handlers remain library code in their `ADR046-exec-*` destinations; the parent frame does not reimplement them. | `ADR046-exec-003` and `ADR046-exec-004` own host/group and UID/NSS parity; `ADR046-exec-005` owns retirement of the old daemon initialization sequence. |
| `SC-003` | `ADR046-exec-009` (user-only Host migration and status posture), `ADR046-exec-006`/`ADR046-exec-007` (Process Provider `ProcessEffect` emission), `ADR046-host-posture-001` (CLI/doctor warning), and `ADR046-telem-008` (OTEL absence gate) | system-core owns `Host.status.isolationPosture` and `NoIsolation`, but never emits `ProcessEffect`. The Process Providers query that status and own launch/stop/adopt/quarantine records. The telemetry gate owns no runtime emitter. | `ADR046-exec-009` owns helper binary/wire/dispatch retirement; `ADR046-nix-010` owns unsafe-local-specific Nix removal. |
| `SC-004` | `ADR046-exec-012` (Nix resource authoring and eval rules) and `ADR046-exec-014` (schema-generated option modules and Zone bundle emission) | These items own the Nix destinations. system-core consumes the emitted Host/User resources and does not maintain a second emitter or schema vocabulary. | `ADR046-exec-012` owns Realm/Workload option removal; `ADR046-nix-010` owns the unsafe-local-specific Nix migration gate. |
| `SC-005` | `ADR046-provider-002` (Provider package shape), `ADR046-exec-003`/`ADR046-exec-004` (system-core crate tests and conformance invocation), `ADR046-exec-020` (shared conformance toolkit), and `ADR046-pstate-011` (workspace layout gate) | The toolkit and policy gate are shared infrastructure. They are not reimplemented in `d2b-provider-system-core`; this crate only supplies its Host/User fixtures and required package paths. | `ADR046-pstate-011` owns the permanent layout gate; there is no system-core removal destination. |
| `SC-006` | `ADR046-telem-004` (core-controller instruments), `ADR046-telem-008` (allowlist/cardinality/redaction policy), `ADR046-audit-001` (shared audit sink and record machinery), and `ADR046-system-core-001` (Host/User `ResourceReconciled` adapter) | system-core contributes only the two closed handler values and its reconcile events. It does not own shared telemetry machinery, policy tests, or `ProcessEffect`. | `ADR046-telem-002` and `ADR046-telem-005` own replacement of their legacy metric families; `ADR046-telem-008` owns the final no-`vm`/no-`unsafe-local` label proof. |

Test ownership is enumerated per test in §16. The §14.3 table and the final
column above are the complete removal-proof assignment for this dossier.

### ADR046-system-core-001 — Provider boundary, manifest, and reconcile audit adapter

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-system-core-001` |
| Dependency/owner | `ADR046-provider-001`, `ADR046-exec-003`, `ADR046-exec-004`, `ADR046-exec-005`, `ADR046-pstate-012`, `ADR046-telem-001`, and `ADR046-audit-001`; `Provider/system-core` owner |
| Current source | No canonical v3 equivalent. Adapt the Provider descriptor pattern from `packages/d2b-realm-provider/src/provider.rs` and the bounded audit-envelope pattern from `packages/d2bd/src/daemon_audit.rs`; do not carry forward daemon topology or unsafe-local helper protocol types. |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-system-core/src/manifest.rs`, `packages/d2b-provider-system-core/src/audit.rs`, and `packages/d2b-provider-system-core/tests/provider_boundary.rs` |
| Detailed design | Compile the system-core Provider manifest, empty closed config schema, Host/User component descriptors, and empty state-namespace declaration. The manifest binds both library handlers to the fixed `d2b-core-controller` bootstrap process without declaring either handler as a Process resource. The audit adapter emits one bounded, redacted `ResourceReconciled` record after each Host/User reconcile. The boundary rejects Provider config fields and proves that handler call paths neither write `Provider.status` nor emit `ProcessEffect`; core-controller infrastructure owns the former and `ADR046-exec-006`/`ADR046-exec-007` own the latter. `ADR046-pstate-012` remains the owner of generic optional-state admission; this item only declares system-core's empty state set. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt descriptor/audit patterns; implement the v3 Provider-specific boundary. |
| Integration | `ADR046-exec-003` and `ADR046-exec-004` call the audit adapter after reconcile; `ADR046-exec-005` and core-controller infrastructure load the manifest and derive the runtime-owned `Provider/system-core` resource/status. |
| Data migration | Full d2b 3.0 reset; no Provider config, handler checkpoint, or audit state is imported. |
| Validation | `config_schema_empty_only`, `provider_status_not_written_by_handlers`, `provider_state_set_empty`, `host_no_process_effect_emitted`, `host_resource_reconciled_audit`, and `user_resource_reconciled_audit`; manifest golden vector proves no Process descriptor and no state namespace for either handler. |
| Removal proof | No independent destination removal. `ADR046-exec-009` owns unsafe-local helper/wire retirement, and `ADR046-telem-008` owns removal proofs for legacy unsafe-local and VM-name telemetry labels. |

---

## 16. Required tests

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has p95 ≤50 ms with no wall-clock
sleep, and `cargo test -p d2b-provider-system-core --lib --tests` completes in
≤2 s warm-cache execution time (compilation excluded). They use a deterministic
fake clock/RNG and the toolkit fakes/FakeEffectPort only — no process spawn,
container, network, DBus, systemd, broker daemon, Nix eval/build, KVM,
USB/GPU/TPM hardware, or live cloud, and no filesystem tree beyond tiny temp
fixtures. Any scenario needing those lives only in `integration/`, which keeps
a lane timeout/budget, parallel isolation, and fake external services by
default; such a need is re-placed into `integration/`, never given a sleep,
larger timeout, or `#[ignore]`. Bounded crypto/property tests are the only
classified exception, each named with a capped case count and a declared higher
per-test budget.

### 16.1 `tests/` (hermetic Cargo integration, invoked by `cargo test`)

| Test | Assertion | Canonical owner |
| --- | --- | --- |
| `host_spec_admission_valid` | All valid Host spec combinations pass admission without error | `ADR046-exec-001` |
| `host_spec_admission_isolation_posture_bidirectional` | All six invalid isolationPosture combinations in §9.2 produce the exact expected error code and message | `ADR046-exec-009` |
| `host_spec_admission_capability_class_bounds` | Unknown HostCapabilityClass rejected; each known class accepted | `ADR046-exec-001` |
| `host_reconcile_converged` | Fake OS probes → all conditions True → phase Ready | `ADR046-exec-003` |
| `host_reconcile_capability_absent` | Probe returns absent → `capability-absent-<class>` condition | `ADR046-exec-003` |
| `host_reconcile_user_manager_unavailable` | User manager IPC fails → `UserManagerReady=False`, phase Degraded (not Failed) for system-only Host | `ADR046-exec-003` |
| `host_reconcile_user_manager_unavailable_user_domain` | User manager IPC fails → phase Failed for user-domain Host after threshold | `ADR046-exec-003` |
| `host_reconcile_noisolution_condition_always_set` | User-only Host always has NoIsolation condition True after reconcile | `ADR046-exec-009` |
| `host_reconcile_budget_overcommit` | Aggregate Process budget exceeds Host budget → `budget-overcommit` condition | `ADR046-exec-003` |
| `host_reconcile_performance_bounds` | Reconcile completes under 5 s on fake probes; hint-to-handler under 5 ms | `ADR046-exec-003` |
| `user_spec_admission_valid` | Valid User specs pass admission | `ADR046-exec-001` |
| `user_spec_admission_os_username_invalid` | NUL byte, slash, control char in osUsername rejected | `ADR046-exec-001` |
| `user_reconcile_converged` | Fake NSS → uid/gid/home/shell all valid → phase Ready | `ADR046-exec-004` |
| `user_reconcile_nss_failure_degraded` | NSS timeout → `nss-lookup-timeout`; accumulates to Failed after threshold | `ADR046-exec-004` |
| `user_reconcile_group_membership_missing` | Missing group → GroupsVerified=False, phase Degraded | `ADR046-exec-004` |
| `user_reconcile_session_manager_unavailable` | Fake user manager IPC fails → SessionManagerReady=False, phase Degraded | `ADR046-exec-004` |
| `user_reconcile_structural_check_blocks_delete` | User with active Process userRef cannot be deleted | `ADR046-exec-004` |
| `host_isolation_posture_set_for_user_only` | User-only Host: reconcile sets `isolationPosture="none"` and `NoIsolation=True` in Host.status | `ADR046-exec-009` |
| `host_isolation_posture_absent_for_system_host` | System-domain Host: reconcile does not set `isolationPosture="none"` or `NoIsolation` condition | `ADR046-exec-009` |
| `host_isolation_posture_available_for_lookup` | `Host.status.isolationPosture` field is present and accurate after reconcile for Process Provider consumption | `ADR046-exec-009` |
| `host_no_process_effect_emitted` | system-core reconcile emits no `ProcessEffect` audit records for any Host reconcile scenario | `ADR046-system-core-001` |
| `host_resource_reconciled_audit` | Host reconcile emits one bounded, redacted `ResourceReconciled` record with the canonical handler value | `ADR046-system-core-001` |
| `user_resource_reconciled_audit` | User reconcile emits one bounded, redacted `ResourceReconciled` record with the canonical handler value | `ADR046-system-core-001` |
| `otel_no_isolation_not_a_label` | Metric/span labels for user-only Host reconcile contain no `no_isolation` dimension | `ADR046-telem-008` |
| `provider_state_set_empty` | `Provider/system-core` declares no Provider state Volume; `ProviderStateSet(zone, "system-core")` is empty; neither controller Process mounts a state Volume; bounded non-secret operational state is written to `status`/the core Operation ledger and handler checkpoints are reconstructible from a resource-store relist; no bootstrap-state pre-provisioning path exists | `ADR046-system-core-001` |
| `provider_status_not_written_by_handlers` | system-core handler code paths contain no `update_status(Provider/system-core, ...)` calls; Provider.status updates are absent from the handler call graph | `ADR046-system-core-001` |
| `provider_conformance_host` | `d2b-provider-toolkit::conformance::check_provider_conformance(Host)` returns zero errors | `ADR046-exec-003` |
| `provider_conformance_user` | `d2b-provider-toolkit::conformance::check_provider_conformance(User)` returns zero errors | `ADR046-exec-004` |
| `config_schema_empty_only` | Non-empty Provider config rejected with exact error | `ADR046-system-core-001` |
| `nix_schema_roundtrip_host` | Rendered Host JSON passes ResourceTypeSchema validation | `ADR046-exec-014` |
| `nix_schema_roundtrip_user` | Rendered User JSON passes ResourceTypeSchema validation | `ADR046-exec-014` |
| `nix_eval_unsafe_local_host_invariants` | nix-unit: unsafe-local Host Nix authoring emits correct JSON; invalid isolationPolicy rejected at eval | `ADR046-exec-012` |

> **Note**: Tests asserting that `ProcessEffect` records carry `no_isolation: true`
> belong to the `Provider/system-systemd` and `Provider/system-minijail` test
> suites under `ADR046-exec-006` and `ADR046-exec-007`, not this crate's
> `tests/`. Those providers own `ProcessEffect` emission and must verify they
> correctly query `Host.status.isolationPosture`.

### 16.2 `integration/` (invoked by `make test-integration` / `make test-host-integration`)

| Test | Assertion | Canonical owner |
| --- | --- | --- |
| `host_reconcile_real_zone` | Provider/system-core controller reconciles a real Host resource in a container Zone runtime; phase reaches Ready | `ADR046-exec-003` |
| `host_capability_probes_real_host` | Real `kvm`, `pidfd`, `cgroup-v2` capability probes succeed on a KVM-capable test host | `ADR046-exec-003` |
| `user_reconcile_real_nss` | Real NSS getpwnam lookup for a declared test user; User reaches Ready | `ADR046-exec-004` |
| `unsafe_local_host_warning_cli` | `d2b zone inspect` renders the no-isolation warning for a user-only Host; warning absent for system-domain Host | `ADR046-host-posture-001` |
| `user_only_host_isolation_posture_stable` | Under a real Zone runtime, a user-only Host consistently reports `isolationPosture="none"` in status after restart and reconcile cycles | `ADR046-exec-009` |
| `provider_system_core_bootstrap_failure_blocks_readiness` | If core cannot create or verify the runtime-owned `Provider/system-core` bootstrap resource, Zone reports Failed with a mandatory-provider condition; no Nix bundle declaration is expected | `ADR046-exec-005` |
| `generation_cleanup_host_deleted` | Removing Host from Nix config triggers async Delete; ResourceDeletionRequested audit event present; store transaction removes row/index and writes Deleted revision; ResourceDeleted audit event appended with exactly-once recovery | `ADR046-exec-015` |

---

## 17. Appendix: HostCapabilityClass enumeration

| Value | Linux feature | Probe method |
| --- | --- | --- |
| `kvm` | `/dev/kvm` accessible | `open("/dev/kvm", O_RDWR)` with bounded timeout |
| `pidfd` | `pidfd_open(2)` / `CLONE_PIDFD` | `pidfd_open(getpid(), 0)` syscall |
| `cgroup-v2` | cgroup v2 delegation | stat `/sys/fs/cgroup/d2b.slice` or check unified hierarchy |
| `user-namespace` | Unprivileged user namespaces | `clone(CLONE_NEWUSER)` with bounded test |
| `virtiofs` | virtiofsd runnable | check `virtiofsd` binary existence in artifact catalog |
| `audio-pipewire` | PipeWire session manager running | `pipewire` D-Bus/socket probe |
| `wayland` | Wayland compositor socket | stat `$WAYLAND_DISPLAY` socket |
| `gpu-render` | Render node present | stat `/dev/dri/renderD128` (or first present) |
| `gpu-drm` | DRM primary node present | stat `/dev/dri/card0` (or first present) |
| `tpm2` | TPM 2.0 device present | stat `/dev/tpm0` or `/dev/tpmrm0` |
| `usbip` | USBIP kernel module loadable | `modinfo vhci-hcd` (non-root) or kernel module check |

All probes use bounded syscall timeouts per §13. A probe that times out
reports the capability as absent (not unknown). The operator claim in
`spec.provider.settings.capabilities` is advisory; the reconciler always
reports the observed set and conditions reflect any mismatch between claimed
and observed.
