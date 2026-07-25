# ADR 0046 Provider dossier: activation-nixos

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-activation-nixos` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 3 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `packages/d2b-provider-activation-nixos/`, `d2b activation` CLI namespace, `activation-nixos.d2bus.org.NixosGeneration` ResourceType |
| Depends on | `ADR-046-provider-model-and-packaging`, `ADR-046-nix-configuration`, `ADR-046-core-controllers`, `ADR-046-resource-object-model`, `ADR-046-resource-api-and-authorization`, `ADR-046-componentsession-and-bus`, `ADR-046-resources-zone-control`, `ADR-046-resources-host-guest-process-user`, `ADR-046-provider-state`, `ADR-046-telemetry-audit-and-support`, `ADR-046-cli-and-operations` |
| Supersedes | Current `d2b switch`/`boot`/`test`/`rollback`/`build`/`generations`/`gc`/`migrate` top-level verbs in `packages/d2b/src/lib.rs` and hardlink-farm activation in `packages/d2b-host/src/hardlink_farm.rs` |

---

## 1. Purpose and ownership boundary

`Provider/activation-nixos` owns the NixOS system generation plan/apply/status/
adopt/rollback lifecycle for every Guest or Host whose Nix configuration
declares it. It reconciles exactly one vendor-qualified ResourceType,
`activation-nixos.d2bus.org.NixosGeneration`, and owns the `d2b activation` CLI
namespace.

### 1.1 What activation-nixos owns

| Surface | Owned behavior |
| --- | --- |
| `activation-nixos.d2bus.org.NixosGeneration` | Plan, apply, status, adopt, and rollback of a NixOS system activation on a Host or Guest |
| `d2b activation` CLI projection | `build`, `switch`, `boot`, `test`, `rollback`, `adopt`, `generations`, `gc`, `migrate` subcommands |
| Per-generation activation-runner | One `EphemeralProcess` per reconcile cycle; runs the integrity-bound activation operation on the target execution context |
| Generation sequence and retention | Tracks active/superseded generation ordering per target; prunes surplus `NixosGeneration` resources to stay within `retainedGenerations` |

### 1.2 What activation-nixos does NOT own

| Surface | Owner |
| --- | --- |
| Zone bundle compilation, `bundle.json`, `artifact-catalog.json` | Nix compiler (`nixos-modules/bundle-zones.nix`, `nixos-modules/artifact-catalog.nix`); spec `ADR-046-nix-configuration` |
| Internal store paths for artifact IDs | Private `artifact-catalog.json`; read by the activation-runner only, never surfaced |
| Zone configuration generation pointer swap, `managedBy=configuration` classification | Core-controller configuration-publication handler; spec `ADR-046-core-controllers` |
| Name conflict resolution and retained-bundle pruning at Zone level | Core-controller; spec `ADR-046-core-controllers` |
| Provider state Volume declaration | None. Under D087, the ProviderStateSet is empty because controller operational state is bounded, non-secret, and derivable from resource status, the core Operation ledger, and external observation. |
| Provider state reconciliation | No Provider state Volume is reconciled. `Provider/volume-local` remains the owner for unrelated store/Volume authority surfaces only. |
| Hardlink farm layout, content operations, store-view GC | `Provider/volume-local`; spec `ADR-046-provider-state` |
| SSH key lifecycle | Out of scope; belongs to a separate identity Provider |
| Guest-editable config editing | Out of scope; belongs to a separate config-management Provider |
| `d2b-activation-helper` binary implementation | Reused from `packages/d2b-host/src/bin/d2b-activation-helper.rs`; adapted, not re-implemented |
| Guest runtime lifecycle (start/stop) | `Provider/runtime-cloud-hypervisor` and peer runtime Providers |
| Credential lifecycle | `Provider/credential-*` family |

### 1.3 Core-controller boundary (do not duplicate)

The core-controller configuration-publication handler is the sole authority for:

1. Advancing the Zone's active configuration generation pointer.
2. Classifying bundle resources as `managedBy=configuration` with `configurationGeneration`.
3. Enqueuing absent-resource cleanup (resources absent from the new bundle).
4. Resolving `(type, name)` name conflicts.
5. Pruning retained bundle slots beyond `retainedGenerations`.

`d2b-activation-helper` verifies artifact integrity and stages closures only.
It does **not** write `managedBy`, `configurationGeneration`, or any resource
field.

---

## 2. Crate and package identity

| Field | Value |
| --- | --- |
| Crate name | `d2b-provider-activation-nixos` |
| Artifact ID | `activation-nixos` |
| Provider resource name | `activation-nixos` |
| Install domain | `system` |
| Controller execution target | `Host/<zone-host>` (from `spec.config.controllerExecutionRef`) |
| Package workspace | `packages/d2b-provider-activation-nixos/` |
| Nix emit location | `nixos-modules/providers/activation-nixos.nix` |

The crate must contain `src/`, `tests/`, `integration/`, and `README.md`.
Absence of any of these paths fails `make test-policy` via
`packages/d2b-contract-tests/tests/policy_provider_crate_layout.rs`
(`ADR-046-resources-zone-control` §4.8.2).

---

## 3. Provider resource spec

### 3.1 Canonical spec shape

```yaml
apiVersion: resources.d2bus.org/v3
type: Provider
metadata:
  name: activation-nixos
  zone: dev
  managedBy: configuration
  configurationGeneration: 7
spec:
  artifactId: activation-nixos
  config:
    controllerExecutionRef: Host/host-system   # required; where the controller Process runs
    retainedGenerations: 3                     # default 3; range [1, 16]; no TTL
```

`spec.config` is validated at build time and re-validated at admission against
the JSON Schema whose digest is stored in the private artifact catalog under
`configSchemaDigest`. The schema definition is part of the signed Provider
package.

### 3.2 spec.config fields

| Field | Type | Default | Bound | Description |
| --- | --- | --- | --- | --- |
| `controllerExecutionRef` | ResourceRef | required | `Host/<name>` | Execution target for the controller Process. Must resolve to an existing Host in the same Zone. |
| `retainedGenerations` | integer | 3 | [1, 16] | Maximum number of `activation-nixos.d2bus.org.NixosGeneration` resources retained per target execution context, spanning all terminal phases. Surplus records (oldest `Succeeded` or `Failed`) are deleted when this bound is exceeded. No TTL fields. |

### 3.3 Fields that are NOT in spec.config

| Field | Location | Reason |
| --- | --- | --- |
| `packageDigest` | Private artifact catalog | Integrity pin; never in public spec |
| `executableDigests` | Private artifact catalog | Integrity pin; never in public spec |
| `manifestDigest` | Private artifact catalog | Integrity pin; never in public spec |
| `configSchemaDigest` | Private artifact catalog | Schema pin; never in public spec |
| Store paths (`systemStorePath`) | Controller process memory only | Private; resolved at activation time from `artifact-catalog.json` |

---

## 4. ResourceType: activation-nixos.d2bus.org.NixosGeneration

### 4.1 Type identity and qualification

Provider-specific semantic ResourceTypes extend the standard catalog through
signed schemas and API bindings (`ADR-046-resource-object-model` §"Minimal
standard ResourceType catalog"). The activation-nixos Provider declares one
vendor-qualified ResourceType:

```
activation-nixos.d2bus.org.NixosGeneration
```

The vendor qualifier (`activation-nixos.d2bus.org`) distinguishes it from any
unqualified standard kind. All resource metadata, spec, and status fields
follow the universal resource envelope contract (`ADR-046-resource-object-model`).

`Volume` is **not** an exported ResourceType for this Provider. Under D087 the
activation-nixos Provider declares no Provider state Volume, so it neither
declares, creates, mounts, nor manages Volumes for controller state.

### 4.2 Spec schema

```yaml
apiVersion: resources.d2bus.org/v3
type: activation-nixos.d2bus.org.NixosGeneration
metadata:
  name: dev-vm--gen-7                        # bounded name; unique per Zone
  zone: dev
  managedBy: configuration                   # or controller for CLI/rollback-created records
  configurationGeneration: 7                 # present when managedBy=configuration
  ownerRef: null                             # Nix-authored: no ownerRef; controller-created: ownerRef=Provider/activation-nixos
spec:
  providerRef: Provider/activation-nixos     # required; immutable
  executionRef: Guest/dev-vm                 # required; Host/<name> or Guest/<name>; immutable
  systemArtifactId: dev-vm-system            # required; bounded ID from d2b.artifacts catalog; immutable
  activationMode: switch                     # required; switch|boot|test|adopt; immutable
  priorGenerationRef: null                   # optional; activation-nixos.d2bus.org.NixosGeneration/<name>; points to the generation being superseded or rolled back from; immutable
```

#### 4.2.1 spec fields

| Field | Type | Required | Bound | Description |
| --- | --- | --- | --- | --- |
| `providerRef` | ResourceRef | yes | `Provider/<name>` | Must resolve to `Provider/activation-nixos` in the same Zone. Immutable after creation. |
| `executionRef` | ResourceRef | yes | `Host/<name>\|Guest/<name>` | Target execution context for the NixOS system. Immutable. |
| `systemArtifactId` | bounded string | yes | `^[a-z][a-z0-9-]{0,127}$` | Global artifact catalog ID from `d2b.artifacts`. The controller resolves the internal store path from the private `artifact-catalog.json` at activation time. No store path appears in any spec or status field. Immutable. |
| `activationMode` | enum | yes | `switch\|boot\|test\|adopt` | `switch`: activate immediately and switch running services. `boot`: set as next-boot default without switching running services. `test`: activate temporarily; reverts on next boot. `adopt`: record an existing active generation without dispatching a new activation. Immutable. |
| `priorGenerationRef` | ResourceRef? | no | `activation-nixos.d2bus.org.NixosGeneration/<name>` | The generation being superseded or rolled back from. Informational; the controller validates that the referenced resource exists and is in a terminal phase. Immutable. |

### 4.3 Status schema

```yaml
status:
  phase: Pending                   # common framework phase; see §4.4
  conditions: []                   # common Condition list
  outcome:
    code: null                     # bounded outcome code on terminal phase
    message: null                  # bounded operator-visible message; no paths/digests
  lastReconciledAt: "2026-07-22T12:00:00.000Z"
  observedGeneration: 1
  resource:
    activationDetail: planning     # typed NixosGeneration detail enum; see §4.3.1
```

Per D088, `NixosGeneration` status uses the universal `ResourceStatus` base at
top-level `status.*`; activation-specific typed fields are the
ResourceType-common `status.resource` object for
`activation-nixos.d2bus.org.NixosGeneration`. Optional `status.provider` carries
only implementation-only observation (`providerRef`, qualified immutable
`schemaId`, semver `schemaVersion`, numeric `observedProviderGeneration`,
strict unknown-field-denied redacted `details` ≤32 KiB registered/signed in the
Provider manifest); shared fields are never duplicated there. The controller
writes all present layers atomically in one status mutation.

D091 currency and upgrade: the activation-nixos controller implements
`assess_update`, `plan_upgrade`, and `execute_upgrade` for
`activation-nixos.d2bus.org.NixosGeneration`. A new NixOS/system generation is
the prototypical `ImageOrSystemGenerationChanged` and `ArtifactChanged` trigger:
the `NixosGeneration` resource populates universal `status.update` with
`UpdateAvailable` or `UpgradeRequired`, and disruptive changes MUST return
`UpgradeRequired` rather than being applied in place, while non-disruptive
changes reconcile normally. These currency fields are universal/ResourceType
base fields, never `status.provider`. Operators apply the planned change
through `d2b upgrade` or activation apply; durable state is preserved across
activation, and `Replace` is used only with ownership/state transfer.

D090 expedited reconcile: Create, UpdateSpec, and Delete requests that set
`waitForReconcile` perform no external effect, finalizer mutation, or status
mutation until Core supplies a typed `CommittedRevisionProof`
`{resourceUid,generation,revision,operationId}`. Abort produces no effect; a
durable commit is never rolled back on later reconcile timeout. The response is
the committed object plus one-pass projected layered status, a disposition
(`Converged`, `Progressing`, `Blocked`, `UpgradeRequired`, or `Failed`), and
`statusPersistence` (`pending` or `committed`); effect idempotency keys derive
from `(UID,generation,revision,operationId)` in the same per-resource
single-flight priority lane.

#### 4.3.1 activationDetail enum

`activationDetail` is a typed, bounded resource detail field. It
carries provider-internal progress that the common `phase` does not encode.
Operators use it to distinguish intermediate states within a phase.

| Value | Meaning |
| --- | --- |
| `planning` | Controller has received the resource; preparing to dispatch activation runner |
| `staged` | Activation runner EphemeralProcess has been created; waiting for the process provider to start it |
| `applying` | Activation runner is executing the activation operation inside the target |
| `applied` | Activation runner completed; for `switch` mode: generation is active. For `boot` mode: generation is set as boot default |
| `boot-default` | `boot` mode only: generation is committed as the next-boot default; not yet confirmed active |
| `adopted` | `adopt` mode: existing active generation recorded without dispatching a new activation |
| `rolled-back` | `switch` mode with `priorGenerationRef`: rollback activation completed; prior generation superseded |
| `superseded` | A newer generation has become active for the same `executionRef`; this generation is now historical |

`activationDetail` is a `status.resource` field only. It never appears in spec.

### 4.4 Phase transitions

The common framework `phase` follows `ADR-046-resource-object-model`. For
`activation-nixos.d2bus.org.NixosGeneration`, the expected transitions are:

| From | To | Trigger |
| --- | --- | --- |
| (none) | `Pending` | Resource created (Nix bundle activate or CLI create) |
| `Pending` | `Pending` | Controller reconciling; `activationDetail` advances through `planning` → `staged` → `applying` |
| `Pending` | `Ready` | Activation runner completes successfully; generation is active (`switch`/`adopt`) or confirmed boot-default (`boot`) |
| `Pending` | `Succeeded` | `test` mode runner completes; generation applied temporarily |
| `Pending` | `Failed` | Activation runner reached `Failed`; or `startDeadline` exceeded |
| `Ready` | `Degraded` | Post-activation health probe failed; running services not healthy |
| `Ready` or `Succeeded` | `Ready` | Superseded by a newer `NixosGeneration`; `activationDetail` set to `superseded` (phase stays `Ready` to indicate last-known-good) |
| `Ready` or `Failed` or `Succeeded` | `Deleted` | Retention window exceeded and controller requests deletion (see §4.5) |
| any | `Unknown` | Controller cannot determine activation state; boot probe inconclusive |

`Deleted` is an event-only terminal phase: the Zone runtime atomically emits
a `phase=Deleted` revision event and removes the resource row and all index
entries. No tombstone row is retained (`ADR-046-resource-object-model`
§"Deletion protocol").

### 4.5 Finalizer protocol

The activation-nixos controller holds a finalizer on every
`activation-nixos.d2bus.org.NixosGeneration` resource it owns. When
`deletionRequestedAt` is set, the controller executes this ordered sequence
before releasing the finalizer:

1. **Drain active activation operation.** If the owned activation-runner
   `EphemeralProcess` is in a non-terminal phase (`Pending` or running),
   update its `runtimeDeadline` to now and wait for it to reach `Succeeded`,
   `Failed`, or `Unknown`. Do not issue `SIGKILL` directly; the EphemeralProcess
   provider owns process termination.

2. **Delete owned EphemeralProcess.** Once the runner reaches a terminal phase,
   set `deletionRequestedAt` on it. Wait for the EphemeralProcess resource to
   be removed (it has no further finalizers of its own).

3. **Release typed generation/profile ownership.** Remove any GC-root
   finalizers or lease records the controller holds that pin the generation's
   store closure. This signals to the store/Volume authority
   (`Provider/volume-local`) that it may collect the associated closure when
   no remaining ownership reference exists. The activation-nixos controller
   does not directly invoke `nix-collect-garbage` or any store-path operation;
   ownership release is the only mechanism.

4. **Clear activation finalizer.** Remove the controller's finalizer from the
   `activation-nixos.d2bus.org.NixosGeneration` resource.

5. **Core commits event-only Deleted/removal.** The Zone runtime atomically
   emits `phase=Deleted` in the revision log and removes the resource row.

6. **Post-commit audit.** The Zone audit chain fires
   `NixosGenerationDeleted` after the commit is durable. No store path, artifact
   digest, or private material appears in the audit record.

### 4.6 Generation retention

The controller maintains at most `spec.config.retainedGenerations` resources per
`executionRef` target across all non-`Deleted` phases. When a new generation
reaches `Ready` or `Succeeded`:

1. Count all existing `activation-nixos.d2bus.org.NixosGeneration` resources for the
   same `executionRef` that are in `Ready`, `Succeeded`, `Failed`, or
   `Degraded` phase.
2. If the count exceeds `retainedGenerations`, identify the oldest surplus
   records (by `metadata.createdAt`, ascending) that are not the current
   `Ready` generation.
3. Set `deletionRequestedAt` on each surplus record, triggering the finalizer
   protocol above.

No TTL fields participate in retention. The `retainedGenerations` count is the
sole pruning criterion.

---

## 5. Components and framework-created resources

### 5.1 Component declarations

The Provider package descriptor declares two components:

| Component ID | Kind | processClass | Role |
| --- | --- | --- | --- |
| `controller` | `Process` (long-lived) | `controller` | Owns the `activation-nixos.d2bus.org.NixosGeneration` reconcile loop via d2b-bus |
| `activation-runner` | `EphemeralProcess` template | `worker` | One-shot per generation; dispatches the integrity-bound activation operation on the target execution context |

### 5.2 Controller Process

The core ProviderDeployment handler creates the controller `Process` resource
from the signed component descriptor. The activation-nixos Provider does **not**
author the Process resource itself, and declares no Provider state Volume for
the controller.

Canonical `Process` resource shape (as created by ProviderDeployment):

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: activation-nixos--controller
  zone: dev
  ownerRef: Provider/activation-nixos
spec:
  providerRef: Provider/system-minijail
  executionRef: <spec.config.controllerExecutionRef>
  domain: system
  userRef: null
  processClass: controller
  template: activation-nixos-controller
  configRef: null
  credentialRefs: []
  mounts: []
  sandbox:
    namespaceClasses: [pid, mount, ipc]
    capabilityClasses: []
    seccompClass: activation-nixos-controller
    noNewPrivileges: true
    startRoot: false
    environmentClass: provider-defined
    readOnlyRoot: true
  budget:
    cpu:
      request: "100m"
      limit: "1000m"
    memory:
      request: "32Mi"
      limit: "64Mi"
    pids:
      limit: 64
    fds:
      limit: 256
  networkUsage: null
  deviceUsage: []
  telemetry: {}
  desiredLifecycle: running
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
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

### 5.3 ProviderStateSet and status-first state

The **ProviderStateSet** for `Provider/activation-nixos` is the set of all
`Volume` resources in the Zone whose `metadata.ownerRef` resolves to
`Provider/activation-nixos`. It is a logical, query-time grouping - not a
ResourceType or stored artifact:

```text
ProviderStateSet(zone, "activation-nixos") =
  { v : Volume | v.metadata.zone == zone
              && v.metadata.ownerRef == "Provider/activation-nixos" }
```

The activation-nixos Provider declares **no Provider state Volume**. Its
ProviderStateSet is therefore empty:

```text
ProviderStateSet(zone, "activation-nixos") = {}
```

This passes the D087 storage-need test without a durable Volume: active
generation references, reconcile checkpoints, retention decisions, and adoption
observations are bounded, non-secret operational state derivable from
`NixosGeneration.status`, the core Operation ledger, and re-verification against
the target execution context after restart.

Status is observation only. The optimistic status writer records only bounded,
redacted, revisioned status fields and material changes; it never stores
secrets, authority-conferring handles, host paths, argv/env, PIDs, unit names,
store paths, raw command output, terminal/clipboard/notification bytes, large
blobs, or unbounded collections. Oversize status is rejected with
`status-oversize` and restart recovery re-verifies the status against external
reality before treating it as current.

There is no controller state mount, identity-only state layout principal,
migration worker, Provider state reset/destroy path, or bootstrap state-Volume
mechanism. The previous bootstrap exception (D086, superseded by D087) does not
apply.

### 5.4 Activation-runner EphemeralProcess

For each `activation-nixos.d2bus.org.NixosGeneration` that requires reconciliation, the
controller creates one owned `EphemeralProcess` resource. This is the canonical
shape the controller emits:

```yaml
apiVersion: resources.d2bus.org/v3
type: EphemeralProcess
metadata:
  name: activation-nixos--runner--dev-vm--gen-7--<run-id>
  zone: dev
  ownerRef: activation-nixos.d2bus.org.NixosGeneration/dev-vm--gen-7
spec:
  providerRef: Provider/system-minijail
  executionRef: <NixosGeneration.spec.executionRef>   # same Host or Guest as the generation; runner executes locally on that target
  domain: system
  userRef: null
  processClass: worker
  template: activation-nixos-runner
  configRef: null
  credentialRefs: []
  mounts: []
  sandbox:
    namespaceClasses: [pid, mount, ipc]
    capabilityClasses: []
    seccompClass: activation-nixos-runner
    noNewPrivileges: true                             # see §7.3
    startRoot: true                                   # see §7.3
    environmentClass: provider-defined
    readOnlyRoot: true                                # rootfs read-only; all target mutation via LaunchTicket effect resources
  budget:
    cpu:
      request: "100m"
      limit: "2000m"
    memory:
      request: "32Mi"
      limit: "128Mi"
    pids:
      limit: 128
    fds:
      limit: 512
  networkUsage: null
  deviceUsage: []
  telemetry: {}
  startDeadline: "120s"
  runtimeDeadline: "600s"
  successfulTtl: "1h"
  failedTtl: "24h"
  incidentHold: false
```

The runner's `template` identifier resolves to the `activation-nixos-runner`
component in the Provider's signed descriptor. The descriptor specifies the
exact binary path, allowed arguments, and environment whitelist. No raw
command string, `argv`, environment variable, or store path is composed by
the controller or written into the EphemeralProcess spec.

#### 5.4.1 What the runner does

The runner executes locally on the target (`spec.executionRef`), whether that
is a Host or a Guest. `Provider/system-minijail` on the target itself starts
the runner binary. Host and Guest targets use the identical contract with no
bypass or fallback path (OQ-1 closed).

The runner receives only fixed inherited operation attachments from its
`LaunchTicket`: a pre-opened activation portal, dirfd(s) for any required
integrity-bound inputs, and the manifest-declared system-manager effect
resources. It does not call d2b-bus (processClass worker carries no bus
authority), and it does not resolve, construct, or traverse any host paths.

Steps the runner performs using its inherited attachments:

1. Invokes the integrity-bound `d2b-activation-helper` through the pre-opened
   activation portal FD, passing the activation mode. The helper operates
   entirely within the inherited FD set; no path construction.
2. All target profile and system mutation occurs only through the
   manifest-declared, pre-opened system-manager effect resources routed in
   the LaunchTicket. No ambient host paths are accessed.
3. Exits with a structured outcome (bounded outcome code) returned through the
   activation portal. No store path, no d2b-bus call, no path in output.

The controller never directly reads store paths, mutates profiles, or calls
`nix-collect-garbage`.

### 5.5 Endpoint resources (D092)

`Provider/activation-nixos` declares standard `Endpoint` base-schema
conformance but does not expose a stable cross-boundary service endpoint in this
dossier. Controller health and activation-control channels are
controller-internal ComponentSession details, and activation runners are
one-shot workers with no inbound service. Therefore no activation-nixos
`Endpoint` child resource is created for the Process examples above. If a
future stable managed activation service is introduced, it must be an owned
`Endpoint` resource with `producerRef`; consumers must use `Endpoint/<name>`,
and raw locators must stay out of spec, status, CLI, audit, and telemetry.
Resolution must go through an authorized EffectPort/LaunchTicket, unauthorized
resolution must return `endpoint-resolve-denied`, and producer restarts must
bump `Endpoint.status.endpointGeneration` to trigger `dependency-changed`.

### 5.6 Retained opaque handles

- pidfds: Process supervision handles, not stable service identities.
- Per-connection/session handles: reconcile operation IDs and ComponentSession
  handles are scoped to one activation or controller session.
- Named streams: none are stable service identities in this dossier; any future
  operation stream carries payload only.
- `OwnedTransport`: authenticated bus transport ownership remains an in-memory
  capability.
- fd indexes: LaunchTicket effect descriptors are per-run slots and stay
  opaque under D092.

---

## 6. Nix authoring

### 6.1 Provider install resource

`nixos-modules/providers/activation-nixos.nix` emits the Provider install
resource using the canonical `spec.artifactId` + `spec.config` shape:

```nix
# nixos-modules/providers/activation-nixos.nix
{ config, lib, ... }:
let
  zone = config.d2b.zone.name;
  controllerHost = config.d2b.zones.${zone}.controllerHost;
  retained = config.d2b.zones.${zone}.retainedGenerations;
in {
  d2b.zones.${zone}.resources.Provider.activation-nixos = {
    spec = {
      artifactId = "activation-nixos";
      config = {
        controllerExecutionRef = "Host/${controllerHost}";
        retainedGenerations = retained;
      };
    };
  };
}
```

Rendered resource:

```yaml
apiVersion: resources.d2bus.org/v3
type: Provider
metadata:
  name: activation-nixos
  zone: dev
  managedBy: configuration
  configurationGeneration: 7
spec:
  artifactId: activation-nixos
  config:
    controllerExecutionRef: Host/host-system
    retainedGenerations: 3
```

The artifact catalog entry for `activation-nixos` is emitted separately into
the private `artifact-catalog.json`. It contains `packageDigest`,
`executableDigests`, `manifestDigest`, and `configSchemaDigest`. None of these
appear in the Provider spec.

### 6.2 NixosGeneration resource authoring

`activation-nixos.d2bus.org.NixosGeneration` resources are authored by the Nix module
for each Guest or Host that enables activation-nixos. They reference artifact
IDs from the global `d2b.artifacts` catalog:

```nix
# nixos-modules/providers/activation-nixos.nix (continued)
let
  # d2b.artifacts.${vmName}-system is declared by the Zone's VM Nix config
  genName = "${vmName}--gen-${toString configGen}";
in {
  d2b.zones.${zone}.resources."activation-nixos.d2bus.org.NixosGeneration".${genName} = {
    spec = {
      providerRef = "Provider/activation-nixos";
      executionRef = "Guest/${vmName}";
      systemArtifactId = "${vmName}-system";   # references d2b.artifacts.${vmName}-system
      activationMode = cfg.vms.${vmName}.activationMode or "switch";
      priorGenerationRef = null;
    };
  };
}
```

Rendered:

```yaml
apiVersion: resources.d2bus.org/v3
type: activation-nixos.d2bus.org.NixosGeneration
metadata:
  name: dev-vm--gen-7
  zone: dev
  managedBy: configuration
  configurationGeneration: 7
spec:
  providerRef: Provider/activation-nixos
  executionRef: Guest/dev-vm
  systemArtifactId: dev-vm-system
  activationMode: switch
  priorGenerationRef: null
```

### 6.3 Artifact catalog binding

The `systemArtifactId` field (`dev-vm-system` above) names an entry in
`d2b.artifacts` declared elsewhere in the Zone's Nix configuration:

```nix
d2b.artifacts."dev-vm-system" = {
  system = config.d2b.zones.dev.vms.dev-vm.config.system.build.toplevel;
};
```

The Nix build emits the store path and its SHA-256 digest into the private
`artifact-catalog.json`. Consumers of `systemArtifactId` (the controller and
runner) look up the artifact there. The store path never reaches any public
resource field.

### 6.4 retainedGenerations Nix option

`d2b.zones.<zone>.retainedGenerations` is a Zone-level Nix option (default 3,
range 1-16). It is the **sole** location where the retention bound is declared.
It flows into `Provider.spec.config.retainedGenerations` only. It is not
duplicated in any `NixosGeneration` spec, nor in any other resource field.

---

## 7. Generation lifecycle

### 7.1 Overview

```
Nix build / CLI create
      │
      ▼
activation-nixos.d2bus.org.NixosGeneration
  phase: Pending
  activationDetail: planning
      │
      ▼ controller reconciles
  creates EphemeralProcess activation-runner
  activationDetail: staged
      │
      ▼ runner starts
  activationDetail: applying
      │
      ├─ runner Succeeded ──► phase: Ready (switch/adopt/boot) or Succeeded (test)
      │                        activationDetail: applied | boot-default | adopted | rolled-back
      │
      └─ runner Failed ─────► phase: Failed
                               outcome.code: <bounded error code>
```

### 7.2 Controller reconcile loop

The controller watches all `activation-nixos.d2bus.org.NixosGeneration` resources in
the Zone whose `spec.providerRef` resolves to `Provider/activation-nixos`. For
each resource:

1. **Admission check.** Validate `spec.executionRef` resolves to an existing
   Host or Guest; validate `systemArtifactId` appears in the private
   `artifact-catalog.json`; validate `priorGenerationRef`, if set, exists.
   Any failure sets `phase: Failed` with an appropriate bounded `outcome.code`.

2. **Idempotent runner dispatch.** If no owned `EphemeralProcess` exists for
   this generation (or the previous one reached a terminal phase and a retry
   is warranted), create the activation-runner EphemeralProcess (§ 5.4).
   Set `activationDetail: staged`.

3. **Runner observation.** Watch the EphemeralProcess status. On
   `EphemeralProcess.phase=Pending` → running: set `activationDetail:
   applying`. On `EphemeralProcess.phase=Succeeded`: transition
   `NixosGeneration` phase as described in §4.4 and set the appropriate
   `activationDetail`.

4. **Supersession.** When a new `NixosGeneration` for the same `executionRef`
   reaches `Ready`, set `activationDetail: superseded` on all prior `Ready`
   records for the same target. Prior `Failed` and `Succeeded` records are
   not superseded; they retain their phase for operator visibility.

5. **Retention pruning.** After each phase transition, apply the retention
   window (§ 4.6). Initiate deletion of surplus records via the finalizer
   protocol (§ 4.5).

### 7.3 startRoot justification

The activation-runner EphemeralProcess requires `startRoot: true`. The
explicit Provider descriptor justification is:

> The `activation-nixos-runner` template requires in-namespace root
> (`startRoot: true`) to operate the integrity-bound activation helper, which
> requires the fixed root identity to apply the system profile through the
> pre-opened system-manager effect resources. `noNewPrivileges: true` and
> `readOnlyRoot: true` are set; no writable ambient root paths or ambient host
> paths are accessible. All target profile and system mutation flows only
> through the manifest-declared, pre-opened activation portal and system-manager
> effect resources routed in the LaunchTicket. No persistent root process
> results; the runner is a one-shot EphemeralProcess that exits after the
> activation operation completes.

### 7.4 Rollback

A rollback is a new `activation-nixos.d2bus.org.NixosGeneration` resource created by
the CLI or controller with:

- `spec.activationMode: switch`
- `spec.systemArtifactId`: the artifact ID of the generation to roll back to
- `spec.priorGenerationRef`: the `NixosGeneration` name of the failed or
  unwanted current generation

The controller reconciles a rollback generation identically to a normal switch.
On success, `activationDetail` is set to `rolled-back`. The prior generation
(pointed to by `priorGenerationRef`) is not automatically deleted; it remains
available for inspection until pruned by the retention window.

### 7.5 Adopt

`activationMode: adopt` records an existing active generation without
applying a new activation. The activation-runner executes target-locally,
reads the active system profile generation, verifies it matches
`systemArtifactId`, and exits with a structured outcome. On success,
`phase: Ready`, `activationDetail: adopted`. Useful for migrations from
pre-ADR-0046 activation paths.

### 7.6 Retention and store GC

When a surplus `NixosGeneration` is deleted per §4.6, step 3 of the finalizer
protocol releases the controller's typed generation/profile ownership. The
actual store closure GC is performed exclusively by the store/Volume authority
(`Provider/volume-local`) through its own resource ownership tracking. The
activation-nixos controller:

- Does **not** invoke `nix-collect-garbage` or any store-path deletion command.
- Does **not** submit explicit `VolumeGcRequest` messages.
- Releases ownership references only (finalizer and lease removal).

The store/Volume authority observes the released ownership and performs
store-view GC through its own reconcile loop, which tracks which closures
still have live ownership references.

---

## 8. d2b activation CLI namespace

`d2b activation` is the sole operator surface for activation operations.
It is the projection of `ADR-046-cli-and-operations` work item `ADR046-cli-007`.

### 8.1 Subcommand summary

| Subcommand | Primary effect | Auth |
| --- | --- | --- |
| `d2b activation build [--zone Z] [--target T]` | Evaluate Zone flake; produce bundle and artifact catalog | admin |
| `d2b activation switch --target T` | Create/update `NixosGeneration(switch)` for target T | admin |
| `d2b activation boot --target T` | Create/update `NixosGeneration(boot)` for target T | admin |
| `d2b activation test --target T` | Create `NixosGeneration(test)` for target T | admin |
| `d2b activation rollback --target T [--to G]` | Create `NixosGeneration(switch)` with `priorGenerationRef` pointing to G (or current active) | admin |
| `d2b activation adopt --target T` | Create `NixosGeneration(adopt)` for target T | admin |
| `d2b activation generations --target T` | List `activation-nixos.d2bus.org.NixosGeneration` resources for target T | read |
| `d2b activation gc --target T` | Release ownership references for surplus generations (triggers retention pruning immediately) | admin |
| `d2b activation migrate --target T --destination H` | Request execution target relocation (future; not in v1 scope) | admin |

`read` auth requires `d2b` group membership with read role on the Zone.
`admin` auth requires admin role on the Zone.

`--target T` accepts `Host/<name>` or `Guest/<name>`. Short form `--target
dev-vm` resolves to `Guest/dev-vm` if unambiguous within the Zone.

### 8.2 Output

All subcommands support `--output json` for machine-readable output. No store
path, private key material, artifact digest, or internal file path appears in
any CLI output in either format.

### 8.3 Current-verb migration

Top-level `d2b switch`, `d2b boot`, `d2b test`, `d2b rollback`, `d2b build`,
`d2b generations`, `d2b gc`, `d2b migrate` are superseded by the
`d2b activation *` group. Removal gated on work item `ADR046-activation-007`.

---

## 9. Bus integration

### 9.1 ComponentSession usage

| Session purpose | Who | Operation |
| --- | --- | --- |
| Zone resource API | Controller | Create, update, and watch `activation-nixos.d2bus.org.NixosGeneration` and `EphemeralProcess` resources |
| Zone resource API | Activation runner (target-local) | Report structured activation outcome; update generation status |

The runner executes locally on the target and uses the Zone resource API to
report its outcome. It does not open a ComponentSession to a remote Host or
Guest. The controller holds no long-lived session to any execution target; all
coordination is through Zone resource state.

### 9.2 Bus endpoint declarations

Component descriptor endpoint declarations:

```yaml
busEndpoints:
  - name: activation-control
    transport: unix
    purpose: activation-nixos-control
```

The runner does not declare endpoints; it is a worker with no inbound
ComponentSession.

---

## 10. RBAC and security

### 10.1 Resource-level access

| ResourceType | Create | Read | Update | Delete |
| --- | --- | --- | --- | --- |
| `activation-nixos.d2bus.org.NixosGeneration` | configuration (bundle) / admin (CLI) | read | admin | admin (retention via controller) |
| `Volume` | - | - | - | - |

The activation-nixos controller has **no** `Volume` resource rights. Volume
create/delete and reconciliation are not part of this Provider's state model:
under D087 it declares no Provider state Volume and mounts none. Bounded
non-secret operational observations are written only to resource status and the
core Operation ledger.

Direct creation of `NixosGeneration` by operators (CLI `switch`/`rollback`/etc.)
results in `managedBy: controller` resources owned by the controller.
Bundle-created resources are `managedBy: configuration`.

The controller creates and manages `EphemeralProcess` resources under its own
ownerRef. Operators do not directly create or delete EphemeralProcess activation
runners.

### 10.2 Controller Process sandbox

The controller runs under:

- `User/d2b-activation-nixos`; system domain; `noNewPrivileges: true`
- `seccompClass: activation-nixos-controller` - allowlist defined in the
  signed Provider package; no syscalls that operate outside the controller's
  state `dirfd` or the d2b-bus socket
- No host network; no devices; no store-path access

### 10.3 Runner sandbox

The activation-runner runs with `startRoot: true` and the provider-declared
justification (§ 7.3). After the activation operation completes, the runner
process exits entirely. No persistent root process results.

The runner's `seccompClass: activation-nixos-runner` allowlist is defined in
the signed Provider package. It permits only the syscalls required to:
- Read the private artifact catalog via the framework-provided integrity channel.
- Invoke the `d2b-activation-helper` subprocess.
- Return the activation outcome through the pre-opened activation portal from
  the LaunchTicket. The runner is `processClass: worker` and has no d2b-bus
  authority; it does not open any ComponentSession.

### 10.4 Store path confidentiality

Internal store paths (`systemStorePath`) are resolved from `artifact-catalog.json`
into the runner's process memory at activation time. They do not appear in:

- Any `activation-nixos.d2bus.org.NixosGeneration` spec or status field
- Any EphemeralProcess spec, status, or output field
- Any CLI output
- Any audit record or OTEL span attribute
- Any log message at any severity level

---

## 11. Conditions and error codes

### 11.1 Conditions

Standard conditions on `activation-nixos.d2bus.org.NixosGeneration`:

| Condition type | Meaning |
| --- | --- |
| `RunnerReady` | Activation-runner EphemeralProcess is in a non-terminal phase |
| `ActivationComplete` | Activation runner reached `Succeeded`; generation active |
| `ArtifactResolvable` | `systemArtifactId` found in private artifact catalog |

### 11.2 Bounded outcome codes

| Code | Terminal phase | Meaning |
| --- | --- | --- |
| `activation-succeeded` | Ready / Succeeded | Activation completed normally |
| `artifact-not-found` | Failed | `systemArtifactId` absent from private artifact catalog |
| `start-deadline-exceeded` | Failed | Runner did not start within `startDeadline` |
| `runtime-deadline-exceeded` | Failed | Runner exceeded `runtimeDeadline` |
| `activation-helper-nonzero` | Failed | `d2b-activation-helper` returned non-zero; artifact integrity check or staging failed |
| `switch-to-configuration-nonzero` | Failed | `switch-to-configuration` on target returned non-zero |
| `prior-generation-invalid` | Failed | `priorGenerationRef` does not resolve to a valid terminal-phase generation |
| `adoption-mismatch` | Failed | `adopt` mode: active profile does not match `systemArtifactId` |

Outcome codes appear only in `status.outcome.code`. They are not used as log
tokens, OTEL span attributes, or audit record payload keys.

---

## 12. Telemetry, audit, and OTEL

### 12.1 OTEL spans

The controller emits one span per generation reconcile cycle. Attributes:

| Attribute | Type | Notes |
| --- | --- | --- |
| `d2b.activation.mode` | string | `switch\|boot\|test\|adopt` |
| `d2b.activation.detail` | string | Current `activationDetail` value |
| `d2b.activation.outcome` | string | `succeeded\|failed\|timeout` on terminal transition |

Zone and target identity are available only in bounded OTEL resource attributes
and permitted audit fields, never as span attributes.
No `systemArtifactId` value, store path, digest, or artifact catalog field
appears as a span attribute.

### 12.2 Audit records

Audit events are emitted by the Zone's post-commit audit chain after each
store transaction is durable. The controller declares the event shape; the
runtime fires it post-commit. No raw path, store path, digest, or private
material appears in any audit record.

| Event | Trigger | Bounded fields |
| --- | --- | --- |
| `NixosGenerationCreated` | Resource created | zone, target, activationMode |
| `NixosGenerationActivated` | Phase → Ready/Succeeded | zone, target, activationMode, activationDetail |
| `NixosGenerationFailed` | Phase → Failed | zone, target, activationMode, outcomeCode |
| `NixosGenerationDeleted` | Deleted revision committed | zone, target, activationMode |
| `ProviderActivationNixosInstalled` | Provider phase → Ready | zone, version |
| `ProviderActivationNixosUninstalled` | Provider Deleted | zone |

### 12.3 Metrics

| Metric | Type | Labels (closed enumerations only) |
| --- | --- | --- |
| `d2b_activation_nixos_generations_total` | Counter | mode, outcome |
| `d2b_activation_nixos_generation_duration_seconds` | Histogram | mode |
| `d2b_activation_nixos_runner_total` | Counter | mode, outcome |

No Zone/guest/resource name, artifact ID, store path, or file path appears as
a metric label. Zone identity remains in the bounded `d2b.zone` OTEL resource
attribute.

---

## 13. Provider lifecycle

### 13.1 Install

1. Core-controller commits the Provider resource with `managedBy=configuration`.
2. ProviderDeployment handler verifies package trust and conformance.
3. ProviderDeployment creates the controller `Process` with no Provider state
   Volume mount (see §5.2 and §5.3).
4. Controller starts; opens health socket.
5. Provider transitions to `Ready`.

### 13.2 Update

A Provider update changes `spec.artifactId` to a new version. ProviderDeployment
re-stages the artifact and restarts the controller Process under the new binary.

### 13.3 Uninstall

1. Core-controller sets `deletionRequestedAt` on the Provider.
2. Controller runs the finalizer protocol for every owned `NixosGeneration`
   (§ 4.5), in parallel across all targets.
3. Controller gracefully stops; ProviderDeployment stops the controller Process
   and waits for the Process finalizer to drain.
4. No Provider state Volume finalizer runs because none is declared.
5. Zone runtime emits event-only `phase=Deleted` for the Provider; no tombstone.
6. Post-commit: `ProviderActivationNixosUninstalled` audit event fires.

---

## 14. Reuse of existing binaries

### 14.1 d2b-activation-helper

`packages/d2b-host/src/bin/d2b-activation-helper.rs` is reused unchanged in
ownership. In the ADR 0046 model, the runner binary invokes it via subprocess.
Required adaptations (work item `ADR046-activation-001`):

- Accept a structured JSON input descriptor (`systemArtifactId`, `activationMode`)
  instead of CLI flags.
- Emit structured JSON result (outcome, bounded error code) to stdout.
- Write **no** resource metadata; no call to any resource API.

### 14.2 chgrp-by-numeric-gid helper

`packages/d2b-host-activation-helper/` is reused without ownership change.

---

## 15. Work items

### ADR046-activation-001: Adapt d2b-activation-helper for structured invocation
| Field | Value |
| --- | --- |
| Dependency/owner | Provider/activation-nixos runner owner; reused helper owner in d2b-host |
| Current source | packages/d2b-host/src/bin/d2b-activation-helper.rs |
| Reuse action | adapt |
| Destination | packages/d2b-host/src/bin/d2b-activation-helper.rs |
| Detailed design | Replace the helper CLI flag interface with structured JSON input and JSON output, accept bounded systemArtifactId and activationMode, resolve store path internally, emit bounded outcome code, write no resource metadata, and preserve the no-bash-fallback invariant. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt in place. |
| Integration | activation-runner invokes the helper through the pre-opened activation portal and system-manager effect resources, then reports structured outcome to activation-nixos status. |
| Data migration | Full d2b 3.0 reset; no v2 activation-helper invocation compatibility |
| Validation | Unit tests for JSON protocol, bounded outcomes, no resource metadata writes, and no Command::new bash fallback. |
| Removal proof | Legacy flag-based helper invocation is removed from activation-nixos paths once runner JSON protocol tests pass. |

**Scope:** `packages/d2b-host/src/bin/d2b-activation-helper.rs`

- Replace CLI-flag interface with JSON-in / JSON-out protocol.
- Accept `systemArtifactId` (bounded string); resolve store path internally.
- Emit bounded outcome code; no resource metadata writes.
- Preserve no-bash-fallback invariant.

### ADR046-activation-002: Implement activation-nixos.d2bus.org.NixosGeneration ResourceType schema
| Field | Value |
| --- | --- |
| Dependency/owner | ADR-046-resource-object-model and ADR-046-core-controllers; d2b-contracts activation-nixos owner |
| Current source | None - net-new v3 ResourceType; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | docs/reference/schemas/v3/activation-nixos.d2bus.org.NixosGeneration.json and packages/d2b-contracts/src/activation_nixos.rs |
| Detailed design | Define JSON schema and Rust DTOs for activation-nixos.d2bus.org.NixosGeneration, with systemArtifactId as the only artifact reference, no store path fields, activationDetail as a closed status.resource enum absent from spec, and phase constrained to the common framework enum values. Primary reuse disposition: `create`. Preserved source-plan detail: net-new schema and DTOs. |
| Integration | Resource API, resource store, Nix compiler, activation-nixos controller, and CLI projections consume the schema and DTOs. |
| Data migration | Full d2b 3.0 reset; no v2 generation resource import |
| Validation | Schema golden vectors, serde unknown-field rejection, phase enum tests, activationDetail-not-in-spec test, and no-store-path-in-spec-or-status test. |
| Removal proof | None - net-new; no prior owner to remove |

**Scope:** `docs/reference/schemas/v3/activation-nixos.d2bus.org.NixosGeneration.json`,
`packages/d2b-contracts/src/activation_nixos.rs`

Define JSON schema and Rust DTOs. Enforce:
- `systemArtifactId` is the only artifact reference (no store path fields).
- `activationDetail` is a closed enum; not present in spec.
- `phase` uses only the common framework enum values.

### ADR046-activation-003: Implement controller crate
| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-activation-002; activation-nixos controller owner |
| Current source | Current top-level activation behavior in packages/d2b/src/lib.rs and hardlink-farm/store ownership split described in this dossier |
| Reuse action | replace |
| Destination | packages/d2b-provider-activation-nixos/src/controller/ |
| Detailed design | Implement the reconcile loop for activation-nixos.d2bus.org.NixosGeneration: validate executionRef, systemArtifactId, and priorGenerationRef; dispatch one activation-runner EphemeralProcess with canonical startRoot=true shape; observe runner status; mark superseded generations; prune by retainedGenerations through the finalizer protocol; emit §12.3 metrics with fixed `mode`/`outcome` semantics and no Zone or resource-name-derived labels; never perform direct store-path operations, nix-collect-garbage, explicit VolumeGcRequest, raw argv composition, or store path writes to resources. Primary reuse disposition: `replace`. Preserved source-plan detail: replace top-level imperative activation flow with resource controller logic. |
| Integration | Controller watches NixosGeneration resources through Zone resource API, creates activation-runner EphemeralProcesses, releases ownership references for Provider/volume-local, and writes bounded status. |
| Data migration | Full d2b 3.0 reset; adopt mode records an existing active generation but does not import v2 controller state |
| Validation | Controller tests for retention, finalizer sequence, no TTL retention, no direct store ops, no store path in status, deleted event-only removal, runner shape, and a structural metric descriptor assertion that `vm`, `zone`, `zone_id`, `zone_uid`, and resource-name-derived keys are absent and a generation/Zone-name canary never enters labels. |
| Removal proof | Direct hardlink-farm and garbage-collection calls from activation-nixos reachable paths are absent after controller and runner tests pass. |

**Scope:** `packages/d2b-provider-activation-nixos/src/controller/`

Reconcile loop for `activation-nixos.d2bus.org.NixosGeneration`. Key invariants:

- No direct store-path operations; no `nix-collect-garbage` invocation.
- EphemeralProcess dispatch creates runner with startRoot=true and the exact
  spec shape from §5.4; no raw command/argv composition.
- Retention pruning via finalizer protocol; no explicit VolumeGcRequest or
  store-path deletion.
- Store path never written to any resource field.

### ADR046-activation-004: Implement activation-runner binary
| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-activation-003; activation-runner owner |
| Current source | packages/d2b-host/src/bin/d2b-activation-helper.rs for helper invocation; runner process is net-new |
| Reuse action | adapt |
| Destination | packages/d2b-provider-activation-nixos/src/runner/ |
| Detailed design | Implement target-local activation-runner worker that executes on NixosGeneration.spec.executionRef for Host and Guest targets using the same contract, reads private artifact-catalog.json through the integrity channel, resolves systemArtifactId to a store path in memory only, invokes d2b-activation-helper through structured JSON, executes target-local switch-to-configuration through typed helper dispatch with no raw exec or SSH, emits structured outcome JSON, and never outputs store paths. Primary reuse disposition: `adapt`. Preserved source-plan detail: net-new runner invoking adapted helper. |
| Integration | Controller-created EphemeralProcess runs under Provider/system-minijail on the target execution context and returns outcome through the activation portal for status update. |
| Data migration | Full d2b 3.0 reset; no v2 runner state import |
| Validation | Runner tests for artifact lookup, JSON helper invocation, Host and Guest target parity, no raw argv, no SSH, no store path in output, and terminal nonzero handling. |
| Removal proof | No old SSH or raw command fallback path exists in activation-runner after tests assert typed helper dispatch only. |

**Scope:** `packages/d2b-provider-activation-nixos/src/runner/`

Target-local binary invoked by the EphemeralProcess template. Executes on the
target (`spec.executionRef`), not on the controller host. Host and Guest targets
use the same contract with no bypass.

- Read private `artifact-catalog.json` via framework-provided integrity channel;
  resolve `systemArtifactId` to store path in memory only.
- Invoke `d2b-activation-helper` via structured JSON protocol.
- Execute target-local `switch-to-configuration` through the helper's typed
  dispatch; no raw exec, no SSH.
- Emit structured outcome JSON; exit 0 on success, non-zero on failure.
- No raw command composition; no store path in any output.

### ADR046-activation-005: Implement d2b activation CLI projection
| Field | Value |
| --- | --- |
| Dependency/owner | ADR-046-cli-and-operations; activation CLI owner |
| Current source | packages/d2b/src/lib.rs top-level cmd_switch, cmd_boot, cmd_test, cmd_rollback, cmd_build, cmd_generations, cmd_gc, and cmd_migrate |
| Reuse action | replace |
| Destination | packages/d2b/src/activation.rs |
| Detailed design | Implement d2b activation build, switch, boot, test, rollback, adopt, generations, gc, and migrate subcommands, projecting ADR-046 CLI and Operations behavior and ensuring no store path, digest, or artifact catalog field appears in JSON or human output. Primary reuse disposition: `replace`. Preserved source-plan detail: replace with grouped CLI projection. |
| Integration | d2b CLI dispatcher calls resource API and activation-nixos controller by creating or listing NixosGeneration resources; legacy top-level verbs are removed by ADR046-activation-007 after integration tests pass. |
| Data migration | Full d2b 3.0 reset; CLI command surface changes with no runtime state import |
| Validation | CLI integration tests for subcommand parsing, authorization, resource creation/listing, rollback priorGenerationRef, gc ownership release, and output redaction. |
| Removal proof | Legacy top-level verb removal is gated on ADR046-activation-007 after the d2b activation integration matrix passes. |

**Scope:** `packages/d2b/src/activation.rs` (new subcommand module)

Implement all `d2b activation *` subcommands (§ 8.1). No store path, digest, or
artifact catalog field in any CLI output. Gated: remove top-level legacy verbs
only after this lands (work item ADR046-activation-007).

### ADR046-activation-006: Nix module for activation-nixos Provider
| Field | Value |
| --- | --- |
| Dependency/owner | ADR-046-nix-configuration; activation-nixos Nix owner |
| Current source | Current VM Nix configuration emits activation inputs implicitly; this item creates the explicit Provider and NixosGeneration resource emitter |
| Reuse action | adapt |
| Destination | nixos-modules/providers/activation-nixos.nix |
| Detailed design | Emit Provider spec and activation-nixos.d2bus.org.NixosGeneration resources per target, flow retainedGenerations only through Provider.spec.config.retainedGenerations, reference systems by systemArtifactId only, omit store paths from all emitted resources, and avoid dedicated state-layout User or ComponentPrincipal because ProviderStateSet is empty. Primary reuse disposition: `adapt`. Preserved source-plan detail: net-new resource emitter adapted from existing Nix activation inputs. |
| Integration | Nix compiler emits Provider and NixosGeneration resources plus private artifact catalog entries consumed by core configuration publication and the activation-nixos controller. |
| Data migration | Full d2b 3.0 reset; existing d2b.vms activation settings are reauthored as Zone resources rather than imported |
| Validation | Nix eval tests for Provider config, NixosGeneration shape, retainedGenerations source, no systemStorePath in bundle, no state Volume or state-layout principal, and artifact ID resolution. |
| Removal proof | Old implicit activation Nix paths are unused by activation-nixos once resource emitter parity tests pass. |

**Scope:** `nixos-modules/providers/activation-nixos.nix`

Emit Provider spec (§ 6.1) and `activation-nixos.d2bus.org.NixosGeneration` resources
(§ 6.2) per declared target. `retainedGenerations` flows only through
`spec.config.retainedGenerations`. No store path in any emitted resource.
Does not declare a dedicated state-layout `User/<name>` principal: the Provider
has no Provider state Volume, and process identity remains owned by the Process
Provider's normal principal model.

### ADR046-activation-007: Remove legacy top-level activation verbs
| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-activation-005; d2b CLI dispatcher owner |
| Current source | packages/d2b/src/lib.rs cmd_switch, cmd_boot, cmd_test, cmd_rollback, cmd_build, cmd_generations, cmd_gc, cmd_migrate and dispatcher registrations |
| Reuse action | delete-after-cutover |
| Destination | packages/d2b/src/lib.rs |
| Detailed design | Remove the legacy top-level activation command functions and their dispatcher registrations after the grouped d2b activation namespace passes integration tests. Primary reuse disposition: `delete-after-cutover`. Preserved source-plan detail: delete legacy top-level commands. |
| Integration | CLI dispatcher routes only d2b activation subcommands for activation operations; documentation and tests use the new namespace. |
| Data migration | Full d2b 3.0 reset; no command alias compatibility window |
| Validation | CLI integration matrix for d2b activation passes; grep or contract test confirms old cmd_* symbols and dispatcher registrations are absent. |
| Removal proof | cmd_switch, cmd_boot, cmd_test, cmd_rollback, cmd_build, cmd_generations, cmd_gc, cmd_migrate and their registrations are deleted from packages/d2b/src/lib.rs. |

**Scope:** `packages/d2b/src/lib.rs`

Remove `cmd_switch`, `cmd_boot`, `cmd_test`, `cmd_rollback`, `cmd_build`,
`cmd_generations`, `cmd_gc`, `cmd_migrate` and their dispatcher registrations.
Gated on ADR046-activation-005 passing integration tests.

---

## 16. Tests

### 16.1 Unit tests (`tests/`)

| Test | Verifies |
| --- | --- |
| `generation::test_spec_no_store_path` | `systemArtifactId` resolves to store path in memory only; no store path written to spec/status |
| `generation::test_phase_common_enum` | phase only uses Pending/Ready/Succeeded/Degraded/Failed/Deleted/Unknown |
| `generation::test_activation_detail_not_in_spec` | `activationDetail` absent from spec schema |
| `generation::test_retention_from_config` | Retention reads `retainedGenerations` from Provider `spec.config`; ignores any other source |
| `generation::test_no_ttl_in_retention` | Retention pruning uses count only; no TTL field applied |
| `generation::test_finalizer_sequence` | Finalizer steps execute in order; runner deleted before ownership released |
| `generation::test_deleted_event_only` | Deletion emits event-only `phase=Deleted`; no tombstone |
| `runner::test_no_raw_argv` | Runner binary creates no raw command string; invokes helper via JSON protocol |
| `runner::test_storepath_not_in_output` | Runner output contains no store path in any field |
| `runner::test_nix_bundle_no_generation` | Compiled bundle contains no `activation-nixos.d2bus.org.NixosGeneration` resources with `systemStorePath` |
| `state::test_provider_state_set_empty` | Provider declares no Provider state Volume; ProviderStateSet query returns empty for `Provider/activation-nixos` |
| `state::test_no_state_layout_principal` | No dedicated state-layout `User/<name>` or ComponentPrincipal reference is emitted for controller state |
| `state::test_status_first_operational_state` | Bounded non-secret controller operational observations are stored in revisioned status/core Operation ledger and re-verified after restart |
| `metrics::test_identity_labels_absent` | Metric descriptors use only closed `mode`/`outcome` semantics; exact absence of `vm`, `zone`, `zone_id`, `zone_uid`, and resource-name-derived keys; Zone/generation-name canaries absent from values; `d2b.zone` resource attribute retained |

### 16.2 Integration tests (`integration/`)

| Test | Verifies |
| --- | --- |
| `test_switch_creates_generation` | `d2b activation switch` creates `NixosGeneration` with correct activationMode |
| `test_rollback_prior_generation_ref` | `d2b activation rollback` sets `priorGenerationRef`; activationDetail → rolled-back |
| `test_runner_ephemeral_process_shape` | EphemeralProcess resource matches canonical spec shape including startRoot and successfulTtl/failedTtl |
| `test_no_nixossshkey_in_bundle` | Compiled bundle contains no NixosSshKey or NixosConfigEdit resources |
| `test_provider_config_retained_generations` | `spec.config.retainedGenerations` is present; no other activation config fields |
| `test_storepath_absent_all_surfaces` | Store path absent from all resource spec/status, CLI output, audit records, OTEL spans |
| `test_adopt_records_existing_generation` | `activationMode: adopt` transitions to Ready/adopted without dispatching switch-to-configuration |
| `test_retention_prunes_surplus_generations` | After `retainedGenerations+1` generations, oldest is deleted |
| `test_activation_helper_no_resource_writes` | `d2b-activation-helper` completes without writing any resource metadata |
| `test_gc_no_direct_store_ops` | Retention deletion does not invoke `nix-collect-garbage`; ownership reference released only |
| `test_no_state_volume_provisioned_on_install` | Provider install creates no Provider state Volume and no `/state` Process mount |
| `test_provider_state_set_logical_query` | ProviderStateSet query returns an empty set for `Provider/activation-nixos` |
| `test_status_bounds_and_redaction` | Status rejects oversize/provider-detail overflow and never includes store paths, argv/env, PIDs, unit names, raw output, or authority handles |

### 16.3 Conformance

`packages/d2b-contract-tests/tests/policy_provider_crate_layout.rs` enforces
`src/`, `tests/`, `integration/`, `README.md` presence for this crate. That
test is the sole enforcement point; this dossier does not duplicate the
requirement.

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has p95 ≤50 ms with no wall-clock
sleep, and `cargo test -p d2b-provider-activation-nixos --lib --tests`
completes in ≤2 s warm-cache execution time (compilation excluded). They use a
deterministic fake clock/RNG and the toolkit fakes/FakeEffectPort only - no
process spawn, container, network, DBus, systemd, broker daemon, Nix eval/build,
KVM, USB/GPU/TPM hardware, or live cloud, and no filesystem tree beyond tiny
temp fixtures. Any scenario needing those lives only in `integration/`, which
keeps a lane timeout/budget, parallel isolation, and fake external services by
default; such a need is re-placed into `integration/`, never given a sleep,
larger timeout, or `#[ignore]`. Bounded crypto/property tests are the only
classified exception, each named with a capped case count and a declared higher
per-test budget.

---

## 17. Migration and removal

### 17.1 Current-code mapping

| Current symbol | ADR 0046 replacement |
| --- | --- |
| `cmd_switch`/`cmd_boot`/`cmd_test`/`cmd_rollback` (`packages/d2b/src/lib.rs`) | `d2b activation switch/boot/test/rollback` (ADR046-activation-005/008) |
| `cmd_build` | `d2b activation build` (ADR046-activation-005) |
| `cmd_generations` | `d2b activation generations` (ADR046-activation-005) |
| `cmd_gc` | `d2b activation gc` (ADR046-activation-005); ownership release only, no store-path ops |
| `cmd_migrate` | `d2b activation migrate` (future; out of v1 scope) |
| `hardlink_farm::build_farm`, `swap_current_symlink` (`packages/d2b-host/src/hardlink_farm.rs`) | Owned by `Provider/volume-local`; activation-nixos never references these primitives |
| `packages/d2b-host/src/bin/d2b-activation-helper.rs` | Adapted in place (ADR046-activation-001); JSON protocol interface added |

Full mapping table is in `ADR-046-current-code-migration-map`.

### 17.2 Removal conditions

Legacy top-level verbs are removed after ADR046-activation-005 and
ADR046-activation-007 land and the `d2b activation` integration test matrix
passes.

Direct hardlink-farm calls from any code path reachable from this Provider are
removed after ADR046-activation-003 and ADR046-activation-004 land.

No new `Command::new("bash")` sites may be introduced. The activation-helper
adaptation preserves the no-bash-fallback invariant
(`ADR-046-current-code-migration-map` §"No bash fallbacks").

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass -
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.
