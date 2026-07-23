# ADR 0046 CLI and operations

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-cli-and-operations` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `packages/d2b` binary crate, Zone runtime client layer |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-resource-api-and-authorization`, `ADR-046-provider-model-and-packaging`, `ADR-046-components-processes-and-sandbox`, `ADR-046-componentsession-and-bus` |
| Supersedes | Current v3 `d2b` CLI contract (`packages/d2b/src/lib.rs` at baseline) |

## Purpose

This spec defines every public-facing CLI command, argument, output format, exit
code, stream/TTY contract, completion/help surface, and dynamic descriptor
behavior for the d2b 3.0 CLI binary. It maps every current v3 CLI verb and
target to its retained, successor, or deletion outcome. It specifies how the CLI
discovers the nearest Zone runtime, constructs canonical ResourceRefs, routes
through the resource API, handles async operations and their lifecycle, and
enforces all limits without embedding Provider logic.

## Guiding invariants

1. The CLI is a typed client. It constructs resource API requests and interprets
   resource/operation responses. It contains no Provider code, no controller
   logic, no broker operations, and no sandbox compilation.
2. Every mutating verb routes through the Zone resource API over ComponentSession
   and d2b-bus. Every read-only verb prefers the Zone resource API and documents
   its graceful fallback.
3. ResourceRefs are canonical Zone-local references of the form
   `<ResourceType>/<resource_name>`. The CLI parses, validates, and serializes
   them but does not invent, guess, or alias them.
4. Output is stable: `--json` emits newline-terminated JSON objects with a
   frozen schema; human output is for terminals only and is not machine-parseable.
5. Exit codes are stable. The guest process exit code passes through `vm exec`
   transparently; the `--json` envelope disambiguates collisions.
6. Dynamic descriptors (shell completion, help, Provider-advertised commands)
   are bounded and fetched with hard deadlines. A slow or absent Zone runtime
   never blocks the CLI binary startup.
7. Provider code never runs in the CLI binary or subprocess at CLI invocation
   time.
8. No SSH, no bash fallback, no realm/workload terminology in new surfaces.

## Baseline terminology mapping

The v3 codebase at baseline `b5ddbed6` uses pre-ADR 0046 names. This table
maps every cited current symbol to its ADR 0046 target. Current-source evidence
citations throughout this spec use the old symbol names; target names appear in
the "target:" annotation.

| Current baseline symbol | Package | Target name |
| --- | --- | --- |
| `RealmId`, `RealmPath`, `TargetName` | `d2b-realm-core/src/ids.rs`, `realm.rs`, `target.rs` | `Zone/<name>` ResourceRef; multi-level path → `ZoneLink` hierarchy |
| `RealmControllerPlacement`, `EntrypointMode` | `d2b-realm-core/src/realm.rs` | Zone runtime placement/mode (ZoneLink `mode` field) |
| `RealmEntrypointDocument`, `RealmEntrypointConfig` | `packages/d2b/src/lib.rs:5163` | Nix-generated static `realm-entrypoints.json`; target: Zone self resource + ZoneLink resources in redb store |
| `RealmPolicyOutputV1 { realm, mode, gateway_vm, gateway_target, gateway_state, cross_realm_policy, credential_boundary }` | `d2b-contracts/src/cli_output.rs:345` | ZoneLink resource fields; `gateway_vm` → ZoneLink `gatewayGuestRef`; `mode` → ZoneLink `placement` |
| `RealmListOutputV1 { realms: Vec<RealmPolicyOutputV1> }` | `d2b-contracts/src/cli_output.rs:285` | Target: `d2b zone list` response listing `ZoneLink` resources |
| `RealmInspectOutputV1 { realm: RealmPolicyOutputV1 }` | `d2b-contracts/src/cli_output.rs:292` | Target: `d2b zone get <name>` response |
| `WorkloadId`, `WorkloadTarget` (`= RealmTarget`) | `d2b-realm-core/src/ids.rs`, `d2b-core/src/workload_identity.rs` | `Guest/<name>` ResourceRef for VM/sandbox/cloud/remote; `Host/<name>` for unsafe-local (NEVER `Guest`) |
| `WorkloadProviderKind::LocalVm` | `d2b-realm-core/src/workload.rs:16` | `Guest` under `Provider/runtime-cloud-hypervisor` |
| `WorkloadProviderKind::QemuMedia` | `d2b-realm-core/src/workload.rs:19` | `Guest` under `Provider/runtime-qemu-media` |
| `WorkloadProviderKind::UnsafeLocal` | `d2b-realm-core/src/workload.rs:22`; Nix option `kind = "unsafe-local"` in `nixos-modules/options-realms-workloads.nix:221`; Nix emitter `nixos-modules/unsafe-local-workloads-json.nix`; helper crate `packages/d2b-unsafe-local-helper/`; wire protocol `packages/d2b-contracts/src/unsafe_local_wire.rs` | Target: `Host/<name>` resource with `defaultDomain=user`, `allowedDomains=[user]`, `defaultUserRef=User/<name>`; reconciled by `Provider/system-core` (NOT itself a Provider and NOT a Guest); child processes use normal Process Providers; no-isolation posture (`IsolationPosture::UnsafeLocal`) MUST be preserved as explicit warnings in Host status, CLI output, UI, and audit/telemetry |
| `WorkloadProviderKind::ProviderManaged` | `d2b-realm-core/src/workload.rs:21` | `Guest` or `Host` depending on Provider type; decision-required per provider dossier |
| `IsolationPosture::UnsafeLocal` | `d2b-realm-core/src/workload.rs:33`; `WorkloadExecutionPosture { isolation: IsolationPosture::UnsafeLocal, environment: EnvironmentPosture::SystemdUserManagerAmbient, execution_identity: ExecutionIdentityPosture::AuthenticatedRequesterUid }` (test at workload.rs:207); emitted in `WorkloadPublicSummary.execution_posture` via `public_wire.rs:267` | Target: `Host` resource `status.isolationPosture: "none"` field; this field MUST be present and non-empty in all Host status outputs, `--json` envelopes, CLI `--human` tables, and audit events for unsafe-local Hosts; a missing or silent posture field is a correctness violation |
| `WorkloadSummary`, `WorkloadState` | `d2b-realm-core/src/workload.rs:130,156` | Guest resource status/phase (LocalVm/QemuMedia/ProviderManaged); for UnsafeLocal → Host resource status/phase |
| `ListEntry { vm: String, lifecycle: VmLifecycle, workload_identity: Option<WorkloadIdentity>, … }` | `d2b-contracts/src/public_wire.rs:2495` | Guest resource envelope; `ListEntry.vm` = `WorkloadId`; `workload_identity` = current realm-native canonical target |
| `VmStatus { vm: String, lifecycle: VmLifecycle, … }` | `d2b-contracts/src/public_wire.rs:2530` | Guest resource status |
| `VmLifecycleState::Stopped/Starting/Booted/Running/Stopping/Restarting/Failed/Unknown` | `d2b-contracts/src/public_wire.rs:2605` | Guest resource `phase`; target phases are `Pending\|Ready\|Succeeded\|Degraded\|Failed\|Deleted\|Unknown`; `Starting`/`Stopping`/`Restarting` map to conditions/reasons on `Pending`/`Degraded`, not separate phases |
| `VmTargetRoute::Local { vm }` | `packages/d2b/src/lib.rs:5577` | `Guest/<vm>` ResourceRef in the current Zone |
| `VmTargetRoute::Gateway { realm, gateway_vm }` | `packages/d2b/src/lib.rs:5578` | Cross-Zone via `ZoneLink/<realm>` ComponentSession; `gateway_vm` = `Guest/<gateway_vm>` |
| `ProcessRole::Virtiofsd` | `d2b-core/src/processes.rs:199` | `Process` under volume-virtiofs Provider |
| `ProcessRole::Swtpm` | `d2b-core/src/processes.rs:203` | `Process` under device-tpm Provider |
| `ProcessRole::CloudHypervisorRunner` | `d2b-core/src/processes.rs:213` | `Process` under runtime-cloud-hypervisor Provider |
| `ProcessRole::QemuMediaRunner` | `d2b-core/src/processes.rs:215` | `Process` under runtime-qemu-media Provider |
| `ProcessRole::WaylandProxy` | `d2b-core/src/processes.rs:249` | `Process` under display-wayland Provider |
| `ProcessRole::Gpu`/`GpuRenderNode` | `d2b-core/src/processes.rs:209,211` | `Process` under `Provider/device-gpu` |
| `ProcessRole::Audio` | `d2b-core/src/processes.rs:213` | `Process` under audio-pipewire Provider |
| `ProcessRole::HostReconcile` | `d2b-core/src/processes.rs:196` | Realm controller bootstrap step; not a standalone CLI-visible Process |
| `ProcessRole::GuestControlHealth` | `d2b-core/src/processes.rs:235` | Guest readiness probe integrated into Guest controller reconcile loop; not a standalone Process ResourceType |
| `ProcessRole::GuestSshReadiness` | `d2b-core/src/processes.rs:228` | Deleted at v3 clean cutover; no compatibility retention |
| `VmProcessDag` | `d2b-core/src/processes.rs` | Per-Guest/Provider `Process` resource DAG; target managed by Provider controller |
| `RealmControllersJson`, `RealmControllerLocalWorkload { vm_name, identity }` | `d2b-core/src/realm_controller_config.rs` | Zone Nix-authored Guest resources; `vm_name` → `Guest/<name>` resource name; `identity.canonical_target` → `WorkloadId` = `Guest/<name>` resource name |
| `ShellAttachArgs { vm: String }`, `ShellOp`, `ShellOpResponse` | `d2b-contracts/src/public_wire.rs:1319,1394,1527` | Target: ShellSession resource + named stream over ComponentSession; `vm` arg = current `WorkloadId` (= target `executionRef: Host/<name>\|Guest/<name>`) |

## Zone context and nearest-runtime discovery

### Zone context selection

The CLI selects a Zone context in this priority order:

1. `--zone <zone-name>` flag on any command where it applies.
2. `D2B_ZONE` environment variable.
3. Nearest-runtime discovery: the CLI walks the runtime socket hierarchy
   (see below) and selects the first reachable Zone runtime whose
   `Zone/<name>` self resource is Ready.
4. If no Zone runtime is reachable, read-only verbs that accept static sources
   fall back to those sources with a visible degraded warning; mutating verbs
   fail closed with exit code 1 and a stable `zone-unavailable` error class.

The selected zone name and socket path are visible in every `--json` output
envelope under `zoneRef` (e.g. `"zoneRef": "Zone/dev"`).

### Nearest-runtime socket paths

For a system-domain (default) context, the CLI probes these paths in order,
stopping at the first socket that accepts a local ComponentSession connect:

```text
/run/d2b/zones/<zone-name>/public.sock   # per-Zone socket (multi-Zone host)
/run/d2b/public.sock                     # v3 canonical single-Zone socket
```

For a user-domain context (`--domain user` or inferred from `$XDG_RUNTIME_DIR`
availability and the target ResourceRef):

```text
$XDG_RUNTIME_DIR/d2b/zones/<zone-name>/public.sock
$XDG_RUNTIME_DIR/d2b/public.sock
```

Environment overrides (retained from v3 baseline for compatibility):

| Variable | Purpose |
| --- | --- |
| `D2B_ZONE` | Override zone name selection |
| `D2B_PUBLIC_SOCKET` | Override public socket path directly |
| `D2B_BROKER_SOCKET` | Override broker socket path (host prepare/destroy only) |

Discovery never falls back from a zone-qualified socket to the single-Zone
socket when `--zone` or `D2B_ZONE` is set.

`--zone` is a global option available to all commands. Purely local
host-maintenance commands (`host prepare`, `host destroy`, `host install`,
`host reconcile`, `host doctor`, `host check`) do not consult Zone context;
they operate on the local host only and explicitly document this in their
`--help` text. Providing `--zone` to these commands is not an error; the flag
is accepted and documented as having no effect on host-maintenance operations.

## Canonical ResourceRef argument parsing

### Accepted forms

Standard ResourceRef positional or `--<type>` flag argument:

```text
<ResourceType>/<resource_name>        # canonical full form
<resource_name>                       # short form for commands that declare a default ResourceType
```

Short form rules:

- The command declares one and only one default ResourceType. There is no
  implicit disambiguation across types.
- Short form is accepted only when the command's `--help` documents the default
  type.
- If the name contains a `/`, the full form is required.

Validation:

- `resource_name` matches `^[a-z][a-z0-9-]*$`.
- `ResourceType` is a known Zone-registered type or a qualified vendor name.
- Validation failure yields exit code 2 with a `ref-invalid` error class.

ResourceRef values that name resources that do not exist return exit code 1 with
a `resource-not-found` error class.

**ResourceType validation:** The frozen standard ResourceType set (Zone,
ZoneLink, Provider, Role, RoleBinding, Host, Guest, Process, EphemeralProcess,
Volume, Network, Device, User, Credential, Quota, EmergencyPolicy) is validated
locally at compile time against the names defined in this spec. A syntactically
valid type name that contains a vendor qualifier (e.g. `acme.corp/FooResource`)
is passed through to the live Zone catalog; the API returns
`resource-schema-invalid` if the Zone does not recognize it. Any other
unrecognized type name fails locally with exit code 2 and class `ref-invalid`
without a Zone round-trip.

## Nix configuration, ResourceSpec JSON, and generation lifecycle

> Full Nix option surface, eval/build validation rules, bundle emission format,
> schema-digest binding, and assertion catalogue are owned by the Nix
> configuration spec (ADR-046-nix-configuration.md). This section covers the
> canonical resource envelope shape as exposed by the CLI and the CLI-visible
> activation/cleanup commands.

### Resource shape and Nix authoring

Zone resources are declared in NixOS modules through a unified structure that
mirrors the canonical ResourceSpec schema directly:

```
d2b.zones.<zone-name>.resources.<resource-name> = {
  type    = "<ResourceType>";
  metadata = {          # optional user-authored fields only
    ownerRef = null;    # optional; names a declared resource in the same bundle
    labels   = { };     # optional; presentation/selector labels only
  };
  spec = { /* exact ResourceTypeSchema field names — no renaming or extra nesting */ };
};
```

**Derived on emit:** `metadata.name` from `<resource-name>` attr key;
`metadata.zone` from `<zone-name>` attr key; `apiVersion` defaults to
`"resources.d2bus.org/v3"`.

**Omitted in authoring (runtime-filled):** `status`, `uid`, `revision`,
`generation`, `finalizers`, `deletionRequestedAt`, `createdAt`, `updatedAt`,
`managedBy`, `configurationGeneration`. The Nix module rejects these as unknown
options if authored.

`spec` sub-options per `type` are generated from the same ResourceTypeSchema
(and signed Provider schema for `type = "Provider"`) that governs the API.
Field names in `spec` are identical to the schema; there is no second vocabulary
and no extra nesting. Vendor-qualified types (containing `.`) are admitted when
a matching schema is installed.

**Minimal multi-resource example:**

```nix
# Derivations are declared in d2b.artifacts; ResourceSpecs reference them by id only.
d2b.artifacts."cloud-hv-pkg" = { package = pkgs.d2b-provider-cloud-hv; type = "provider"; };

d2b.zones.main.resources = {
  "main"         = { type = "Zone";            spec = {}; };  # Zone.spec is empty
  "work-vm"      = { type = "Guest";           spec = { providerRef = "Provider/cloud-hv"; executionPolicy = { cores = 4; memoryMiB = 8192; }; networkRefs = [ "Network/default" ]; }; };
  "local-user"   = { type = "Host";            spec.defaultUserRef = "User/alice"; };
  "default"      = { type = "Network";         spec = { cidr = "10.100.0.0/24"; gatewayAddress = "10.100.0.1"; }; };
  "cloud-hv"     = { type = "Provider";        spec.artifactId = "cloud-hv-pkg"; };  # plain id, no derivation in spec
  "ssh-host-key" = { type = "Credential";      spec = { credentialType = "ssh-ed25519-host-key"; credentialSource = "systemd-credential:d2b-ssh-key"; }; };
  "alice"        = { type = "User";            spec = { uid = 1001; homeDir = "/home/alice"; }; };
  "default"      = { type = "Quota";           spec = { /* quota fields per Quota schema */ }; };
  "lockdown"     = { type = "EmergencyPolicy"; spec = { /* policy fields per EmergencyPolicy schema */ }; };
};
```

**Evidence class:** `ADR-only` — v3 Nix modules do not exist at baseline
`b5ddbed6`. Current source: `nixos-modules/options-realms-workloads.nix`,
`nixos-modules/options-realms.nix`, `nixos-modules/unsafe-local-workloads-json.nix`
(pre-ADR 0046 names). Target implemented by work item ADR046-cli-011.

### Canonical ResourceSpec JSON shape

Every `d2b get`, `d2b list` element, and `d2b watch` event carries the
canonical ResourceSpec envelope inside the outer `--json` response. The
exact shape (all fields always present; null where empty):

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type": "Guest",
  "metadata": {
    "name": "work-vm",
    "zone": "main",
    "uid": "01930a4f-2b3d-7e8f-9c1a-4d5e6f708192",
    "generation": 3,
    "revision": "<opaque-zone-local-token>",
    "ownerRef": null,
    "finalizers": [],
    "deletionRequestedAt": null,
    "createdAt": "2026-07-22T00:00:00Z",
    "updatedAt": "2026-07-22T10:30:00Z",
    "managedBy": "configuration",
    "configurationGeneration": 7,
    "labels": {}
  },
  "spec": {
    "providerRef": "Provider/cloud-hv",
    "executionPolicy": { "cores": 4, "memoryMiB": 8192 },
    "networkRefs": ["Network/default"],
    "provider": { "schemaId": "runtime-cloud-hypervisor.d2bus.org/Guest/spec", "schemaVersion": "1.0", "settings": {} }
  },
  "status": {
    "observedGeneration": 3,
    "phase": "Ready",
    "conditions": [],
    "lastReconciledAt": "2026-07-22T10:30:05Z",
    "startedAt": "2026-07-22T10:30:05Z",
    "completedAt": null,
    "outcome": null
  }
}
```

**Core management metadata** (runtime-filled; never authored in Nix):

| Field | Value | Meaning |
| --- | --- | --- |
| `managedBy` | `"configuration"` | Core-set closed enum; `"controller"` and `"api"` are the other values |
| `configurationGeneration` | `7` (integer) | Bundle generation that last confirmed this configuration-managed resource; absent on controller/API-managed resources |

**Cleanup authority:** Core identifies resources eligible for deletion from a
new bundle by `managedBy == "configuration"` AND absence from the new canonical
configured set. `managedBy == "controller"` and `managedBy == "api"` resources
are **never** deleted by bundle application. This invariant is a correctness
requirement.

**Owner controller responsibility:** When a configuration-managed parent resource is
deleted, its owning controller enqueues deletion of declared children (child
Processes, EphemeralProcess sessions, Volume attachments). It does not delete
unrelated resources.

**List response** (`d2b list Guest`):

```json
{
  "ok": true,
  "zoneRef": "Zone/main",
  "operationId": "<id>",
  "schemaVersion": 1,
  "items": [ { /* ResourceSpec envelope */ }, ... ],
  "nextPageToken": null,
  "totalCount": 3
}
```

**Status-only response** (`d2b status Guest/work-vm`):

```json
{
  "ok": true,
  "zoneRef": "Zone/main",
  "operationId": "<id>",
  "schemaVersion": 1,
  "resourceRef": "Guest/work-vm",
  "status": { /* status sub-object only */ }
}
```

### CLI-visible activation and cleanup

Full activation-provider command surface is in
[§ `d2b activation`](#d2b-activation--activation-nixos-provider-commands).

**Apply a new bundle generation:**

```
d2b activation apply [--zone <zone>] [--dry-run] [--json | --human]
```

Sends the built bundle (`/etc/d2b/zones/<zone>/resource-bundle.json`) to the
fixed core-controller configuration service.
Exits 0 when the apply phase completes (creates/updates present resources).
Does not wait for deletion of absent Nix resources. Output includes applied
resource count and pending-cleanup count.

**Absent-resource deletion:** Resources with `managedBy == "configuration"` absent from
the new bundle receive asynchronous Delete. The Zone enters
`Degraded/pending-cleanup` until all complete.

**Observe cleanup progress:**

```
d2b zone status [<name>] [--zone <zone>] [--watch] [--deadline <duration>]
```

With `--watch`, streams `pending-cleanup` condition transitions:

```
$ d2b zone status main --watch
Zone/main  Degraded  pending-cleanup: 2 resources pending deletion
Zone/main  Degraded  pending-cleanup: 1 remaining (Guest/old-vm done)
Zone/main  Ready     (cleanup complete)
```

**Per-resource deletion progress:**

```
d2b status <ResourceType>/<name> [--watch]
```

**Prior generation retention:** count-based; default 3; range 1–16. The
retention count is configured by the Zone-level Nix/compiler setting
`d2b.zones.<zone>.retainedGenerations`, outside the empty `Zone.spec`.
Generations beyond the count become eligible for pruning after cleanup
completes. Generations within the count are retained for rollback regardless of
elapsed time; there is no TTL.

```
d2b activation gc [--zone <zone>] [--dry-run] [--json | --human]
```

Prunes generation bundles and hardlink-farm snapshots beyond the retention count.

**Rollback:**

```
d2b activation rollback [--to-generation <n>] [--zone <zone>] [--dry-run] [--apply]
```

Re-applies a retained prior generation bundle as a new (higher)
`bundleGeneration`. The generation counter is never reversed.

**Audit:** Each deletion dispatched from bundle application emits
`resource-delete-dispatched`; each completion emits
`resource-delete-completed` or `resource-delete-failed`.

### CLI-visible tests for activation and cleanup

| Layer | Test | What it proves |
| --- | --- | --- |
| Runtime (integration) | Apply bundle → absent `managedBy=configuration` resource receives async Delete | Cleanup dispatch uses `managedBy` |
| Runtime (integration) | `managedBy=controller` and `managedBy=api` resources absent from bundle → NOT deleted | `managedBy` cleanup invariant |
| Runtime (integration) | Zone status = `Degraded/pending-cleanup` during outstanding deletions | Status accuracy |
| Runtime (integration) | Zone status = `Ready` after all cleanup completes | Status accuracy |
| Runtime (integration) | Generations within retention count retained (no TTL) | Count-based retention |
| Runtime (integration) | `d2b activation gc` prunes beyond retention count | GC correctness |
| Runtime (integration) | `d2b activation rollback` re-applies prior bundle as new bundleGeneration | Rollback increments counter |
| Runtime (integration) | Stuck finalizer leaves resource Degraded and requires controller repair or full Zone reset | No force-finalizer path |
| Runtime (integration) | Old `bundleGeneration` replay rejected | Generation monotonicity |

## Standard resource verbs

The following subcommand structure applies to all standard ResourceTypes that
declare a CLI surface. The exact set of verbs implemented per ResourceType is
specified in the per-ResourceType section below.

### `d2b get <ResourceType>/<name>`

Fetch the current resource envelope.

```
d2b get <ResourceType>/<name> [--zone <zone>] [--json | --human]
```

Returns the complete resource envelope (`metadata`, `spec`, `status`).

**Output format:** JSON envelope or human summary table row with phase/condition
summary.

**Exit codes:**
- 0: resource returned
- 1: resource not found or zone unavailable (`resource-not-found`,
  `zone-unavailable`)
- 2: usage/validation error

### `d2b list <ResourceType> [filters]`

List resources of one type.

```
d2b list <ResourceType> [--zone <zone>]
  [--phase <phase>]
  [--label-selector <k>=<v>]
  [--page-token <token>]
  [--limit <n>]
  [--json | --human]
```

Returns a bounded page of matching resources. Pagination uses `--page-token`
from the previous response.

**Output format:** JSON array envelope with `items`, `nextPageToken`
(absent if last page), `snapshotRevision`; human table with status column.

**Exit codes:**
- 0: list returned (may be empty)
- 1: zone unavailable or filter error
- 2: usage error

### `d2b watch <ResourceType> [filters]`

Stream resource change events after a starting revision.

```
d2b watch <ResourceType> [--zone <zone>]
  [--since-revision <revision>]
  [--phase <phase>]
  [--label-selector <k>=<v>]
  [--json]
  [--deadline <duration>]
```

Streams newline-separated JSON change envelopes (`type`, `object`) to stdout.

**Output format:** JSON change stream only (`--human` is not valid for watch).

**Deadline:** `--deadline <duration>` (e.g. `30s`, `5m`) sets a hard wall
deadline. Exit code 0 on clean close; 1 on deadline expiry with
`deadline-exceeded` class; 3 on signal/cancel.

**Exit codes:**
- 0: clean close by server
- 1: stream error, zone unavailable
- 2: usage error
- 3: cancelled by SIGINT/SIGTERM

### `d2b create <ResourceType> [--spec-file <path> | --spec-stdin]`

Create a resource from a JSON spec file or stdin.

```
d2b create <ResourceType> [--zone <zone>]
  [--spec-file <path> | --spec-stdin]
  [--json | --human]
```

**Exit codes:**
- 0: created
- 1: already exists (`resource-already-exists`), validation error, zone
  unavailable
- 2: usage error

### `d2b update-spec <ResourceType>/<name>`

Full spec replacement.

```
d2b update-spec <ResourceType>/<name> [--zone <zone>]
  [--revision <expected-revision>]
  [--spec-file <path> | --spec-stdin]
  [--json | --human]
```

**Exit codes:**
- 0: updated
- 1: conflict, not found, validation error, zone unavailable
- 2: usage error

### `d2b delete <ResourceType>/<name>`

Request deletion.

```
d2b delete <ResourceType>/<name> [--zone <zone>]
  [--revision <expected-revision>]
  [--json | --human]
```

Triggers deletion (sets `deletionRequestedAt`). Does not wait for finalizers to
complete unless `--wait` is given.

**Exit codes:**
- 0: deletion accepted
- 1: not found, conflict, zone unavailable
- 2: usage error

### `d2b status <ResourceType>/<name>`

Fetch the status subresource only.

```
d2b status <ResourceType>/<name> [--zone <zone>]
  [--watch]
  [--deadline <duration>]
  [--json | --human]
```

With `--watch`, streams status change events until Ready/Succeeded/Failed/
Deleted or deadline.

**Exit codes (without `--watch`):**
- 0: returned
- 1: not found, zone unavailable
- 2: usage error

**Exit codes (with `--watch`):**
- 0: phase reached Ready or Succeeded
- 1: phase reached Failed, Deleted, or zone unavailable
- 2: usage error
- 3: cancelled or deadline exceeded

## `d2b host` — Host resource commands

Maps to the `Host` ResourceType. Default ResourceType context for positional
name argument.

### `d2b host get <name>`

Equivalent to `d2b get Host/<name>`.

### `d2b host list`

Equivalent to `d2b list Host`.

**Current v3 source:** no current command lists physical/local Host contexts.
`cmd_list`/`cmd_status` list VM workloads and therefore inform the target
`Guest` command shape, not Host phase semantics. `Host` as a distinct
ResourceType and `d2b host list` are ADR-only.

### `d2b host status <name>`

Equivalent to `d2b status Host/<name>`.

### `d2b host prepare`

Reconcile host-side infrastructure (bridges, nftables, sysctls, cgroups).
Routes through the Zone resource API's Host reconcile operation. `--dry-run`
returns a plan; `--apply` executes.

```
d2b host prepare [--zone <zone>] [--dry-run | --apply] [--json | --human]
```

**Current v3 source:** `cmd_host_prepare` at
`packages/d2b/src/lib.rs:4468`; dispatches through `public.sock`/broker.
Evidence class: `implemented-and-reachable`.

**Retained behavior:** dry-run/apply distinction, broker-mediated mutation,
ownership markers, fail-closed on foreign markers.

**Required delta:** Route through Zone resource API Host reconcile operation
instead of raw broker seqpacket.

### `d2b host destroy`

```
d2b host destroy [--zone <zone>] [--dry-run | --apply] [--json | --human]
```

**Current v3 source:** `cmd_host_destroy` at `packages/d2b/src/lib.rs:4549`.
Evidence class: `implemented-and-reachable`.

### `d2b host doctor`

Read-only deep diagnostics.

```
d2b host doctor [--zone <zone>] [--read-only] [--json | --human]
```

**Current v3 source:** `cmd_host_doctor` at `packages/d2b/src/lib.rs:4752`.
Evidence class: `implemented-and-reachable`.

**Retained behavior:** reads daemon state dir, metrics URL reachability,
pidfd-table, kernel-module-report, autostart-report, storage-lifecycle-report.
Daemon-state-dir and metrics-URL overrides retained via `D2B_DAEMON_STATE_DIR`
and `D2B_METRICS_URL`.

**Required delta:** Read from Zone runtime status resources rather than local
files where available; fall back to local state files when Zone API is
unavailable.

### `d2b host check`

```
d2b host check [--read-only] [--json | --human]
```

**Current v3 source:** `cmd_host_check` at `packages/d2b/src/lib.rs:4271`.
Evidence class: `implemented-and-reachable`. Host-check exit code 3 retained
for compatibility.

### `d2b host install`

```
d2b host install [--dry-run | --apply] [--enable] [--start] [--no-start]
  [--json | --human]
```

**Current v3 source:** `cmd_host_install` at `packages/d2b/src/lib.rs:5007`.
Evidence class: `implemented-and-reachable`.

### `d2b host reconcile`

```
d2b host reconcile [--dry-run | --apply] [--json | --human]
```

**Current v3 source:** `cmd_host_reconcile` at `packages/d2b/src/lib.rs:5070`.
Evidence class: `implemented-and-reachable`.

### `d2b host validate`

```
d2b host validate [--dry-run | --apply] [--wave <wave>]
  [--evidence-dir <path>] [--scripts-dir <path>]
  [--operator-signature <sig>]
  [--json | --human]
```

**Current v3 source:** `cmd_host_validate` at `packages/d2b/src/lib.rs:4941`.
Evidence class: `implemented-and-reachable`.

### Unsafe-local Host resources — isolation posture requirements

`Host` resources with `WorkloadProviderKind::UnsafeLocal` lineage have
`defaultDomain=user`, `allowedDomains=[user]`, and `defaultUserRef=User/<name>`.
They are reconciled by `Provider/system-core`; they are NOT Provider resources
themselves, and they are NEVER `Guest` resources. Their child processes run
through normal Process Providers.

The no-isolation posture (`IsolationPosture::UnsafeLocal` in current baseline,
`status.isolationPosture: "none"` in target Host resource) MUST be preserved
explicitly in:

1. **Host status** (`d2b host status`, `d2b host get`, `d2b host list`): the
   `status.isolationPosture` field MUST be `"none"` in `--json` output and
   presented as a visible `[no isolation]` annotation in `--human` table rows.
2. **CLI/UI**: `d2b shell open Host/<name>` and `d2b exec run Host/<name> -- ...`
   MUST emit a one-line `warning: no isolation boundary — this process runs as
   your host user` line to stderr before attaching. The warning has NO
   suppression flag; it appears unconditionally in human output. JSON output
   carries `isolationPosture: "none"` in the response envelope.
3. **Audit**: every `Shell` and `EphemeralProcess` event on a Host resource
   MUST carry `isolationPosture: "none"` as a fixed closed label in audit
   records. This posture MUST NOT appear in OTEL metric labels, span attributes,
   or structured log fields; it is status/UI/audit-surface only.

A missing or silent posture field is a correctness violation. Reviewers MUST
reject any diff that adds a code path serving a Host/unsafe-local resource
without propagating the isolation posture through all three surfaces above.

**Current v3 evidence:**
- `IsolationPosture::UnsafeLocal` at `d2b-realm-core/src/workload.rs:33`
- `WorkloadExecutionPosture { isolation: IsolationPosture::UnsafeLocal, environment: EnvironmentPosture::SystemdUserManagerAmbient, execution_identity: ExecutionIdentityPosture::AuthenticatedRequesterUid }` at `workload.rs:207`
- `WorkloadPublicSummary.execution_posture: WorkloadExecutionPosture` at `packages/d2b-contracts/src/public_wire.rs:267` (public inventory carries isolation posture today)
- `nixos-modules/options-realms-workloads.nix:233` — "Host-user process runtime with no isolation boundary. Requires explicit realm policy opt-in."
- `nixos-modules/unsafe-local-workloads-json.nix` — emits `runtimeKind = "unsafe-local"` and `providerId = "unsafe-local"` in private bundle
- `packages/d2b-unsafe-local-helper/src/` — runtime helper; `packages/d2b-contracts/src/unsafe_local_wire.rs` — wire protocol

Evidence class: `implemented-and-reachable` for isolation posture data path;
`ADR-only` for the `Host` resource API shape and the `Provider/system-core` reconciler.

## `d2b guest` — Guest resource commands

Maps to the `Guest` ResourceType. Unsafe-local workloads are `Host` resources,
never `Guest` — see §Unsafe-local Host resources above.

### `d2b guest get <name>`

Equivalent to `d2b get Guest/<name>`.

### `d2b guest list`

Equivalent to `d2b list Guest`.

**Current v3 source:** `cmd_list`/`cmd_vm_list` at `packages/d2b/src/lib.rs:3560`; returns
`ListResponse { vms: Vec<ListEntry> }` from `packages/d2b-contracts/src/public_wire.rs:2152`.
Each `ListEntry` has `vm: String` (= current `WorkloadId`; target: `Guest/<name>` resource
name), `lifecycle.state: VmLifecycleState`, `workload_identity: Option<WorkloadIdentity>` (realm-
native canonical target, additive field tolerated absent by old daemons), and `runtime_capabilities:
Vec<String>` (current: `WorkloadProviderKind` encoded; target: Provider family). Evidence class:
`implemented-and-reachable` for the list/lifecycle data path; `ADR-only` for `Guest` resource API.

### `d2b guest status <name>`

Equivalent to `d2b status Guest/<name>`. With `--watch`, streams status events.

**Successor to:** `d2b vm status <name>` (see migration table below).

**Current v3 source:** `cmd_vm_status`/`cmd_status` at `packages/d2b/src/lib.rs:6868,3587`;
returns `StatusResponse { entries: Vec<VmStatus> }` from `public_wire.rs:2158`. `VmStatus.vm:
String` = current `WorkloadId`; `VmStatus.lifecycle.state: VmLifecycleState` maps to target
`Guest` phase (`Stopped` → `Pending`, `Booted`/`Running` → `Ready`, `Failed` → `Failed`,
`Starting`/`Stopping`/`Restarting` → conditions/reasons on `Pending`/`Degraded`,
not separate phases). Evidence class:
`implemented-and-reachable` for status/lifecycle data path; `ADR-only` for `Guest` resource API.

### `d2b guest start <name>`

Request that the Guest reach Ready phase.

```
d2b guest start <name> [--zone <zone>]
  [--dry-run | --apply]
  [--no-wait-ready]
  [--json | --human]
```

`--no-wait-ready` exits 0 on accepted (not waiting for Ready phase).
Default behavior waits for phase Ready or Failed with exit codes 0/1.

**Successor to:** `d2b vm start <vm>`, `d2b up <vm>`.

**Current v3 source:** `cmd_vm_start` at `packages/d2b/src/lib.rs:6675`;
dry-run/apply/no-wait-api pattern at `VmStartArgs`.
Evidence class: `implemented-and-reachable`.

**Required delta:** Route through Zone resource API Guest lifecycle instead
of raw daemon verb; replace `EXIT_API_TIMEOUT` (33) with `deadline-exceeded`
error class.

### `d2b guest stop <name>`

```
d2b guest stop <name> [--zone <zone>]
  [--dry-run | --apply]
  [-f | --force]
  [--json | --human]
```

**Successor to:** `d2b vm stop <vm>`, `d2b down <vm>`.

**Current v3 source:** `cmd_vm_stop` at `packages/d2b/src/lib.rs:6690`.
Evidence class: `implemented-and-reachable`.

### `d2b guest restart <name>`

```
d2b guest restart <name> [--zone <zone>]
  [--dry-run | --apply]
  [-f | --force]
  [--json | --human]
```

**Successor to:** `d2b vm restart <vm>`, `d2b restart <vm>`.

### `d2b guest create`

```
d2b guest create [--zone <zone>]
  [--spec-file <path> | --spec-stdin]
  [--json | --human]
```

### `d2b guest update-spec <name>`

### `d2b guest delete <name>`

**Note:** The following v3 Guest verbs (`build`, `generations`, `switch`,
`boot`, `test`, `rollback`) are Guest-activation operations that interact with
the activation Provider (`activation-nixos`). They are specified under
`d2b activation` below.

## `d2b process` — Process resource commands

Maps to the `Process` ResourceType.

### `d2b process get <name>`

Equivalent to `d2b get Process/<name>`.

### `d2b process list`

Equivalent to `d2b list Process [--execution-ref <Host|Guest>/<name>]
  [--domain <system|user>]`.

### `d2b process status <name>`

Equivalent to `d2b status Process/<name>`. With `--watch`, streams until Ready
or terminal.

### `d2b process start <name>`

Request that the Process reach Ready phase (sets desired lifecycle).

```
d2b process start <name> [--zone <zone>]
  [--dry-run | --apply]
  [--no-wait-ready]
  [--json | --human]
```

### `d2b process stop <name>`

```
d2b process stop <name> [--zone <zone>]
  [--dry-run | --apply]
  [-f | --force]
  [--json | --human]
```

### `d2b process create`, `update-spec`, `delete`

Standard resource verbs.

## `d2b exec` — EphemeralProcess (one-shot exec)

`d2b exec` creates an `EphemeralProcess` resource and manages its lifecycle.
This replaces the `d2b vm exec` sub-verb for the one-shot asynchronous exec
use case.

### `d2b exec run <executionRef> -- <cmd> [args...]`

Create a detached `EphemeralProcess` and print its resource name.

```
d2b exec run <executionRef> [--zone <zone>]
  [--name <name>]
  [--domain <system|user>]
  [--user <userRef>]
  [--provider <providerRef>]
  [--env KEY=VALUE]...
  [--cwd <dir>]
  [--deadline <duration>]
  [--json | --human]
  -- <cmd> [args...]
```

`executionRef` is a canonical ResourceRef of type `Host` or `Guest`.

Returns the created `EphemeralProcess/<name>` resource ref and initial status.

**Exit codes:**
- 0: EphemeralProcess created and accepted
- 1: execution target unavailable, authorization denied, zone unavailable
- 2: usage error

**Successor to:** `d2b vm exec -d <vm> -- <cmd>` (detached form).

**Current v3 source:** `VmExecArgs.detach` in `cmd_vm_exec` at
`packages/d2b/src/lib.rs:7166`; `exec_client::EXIT_EXEC_*` exit codes at
`packages/d2b/src/exec_client.rs`.
Evidence class: `implemented-and-reachable` for detached vm exec.

### `d2b exec attach <EphemeralProcess>/<name>`

Attach stdin/stdout/stderr to a running EphemeralProcess.

```
d2b exec attach <EphemeralProcess>/<name> [--zone <zone>]
  [-i | --interactive]
  [-t | --tty]
  [--json]
  [--deadline <duration>]
```

`-t` puts the host terminal in raw mode. `-i` forwards host stdin. `-it`
together yields an interactive shell session. `--tty` is incompatible with
`--json`.

**Successor to:** `d2b vm exec -it <vm> -- bash` (attached interactive form) and
`d2b vm exec <vm> -- <cmd>` (attached non-interactive form).

**Current v3 source:** `cmd_vm_exec` attached path at
`packages/d2b/src/lib.rs:7166`; `exec_client.rs` FSM; `terminal_client.rs`
TTY abstractions. Evidence class: `implemented-and-reachable`.

**Retained behavior:** host terminal raw mode; RAII termios restore on every
exit/error/panic; guest owns PTY; host only flips termios; `--json` envelope
with `source`/`reason`/`guestExitCode`/`signal`/`exitCode` fields;
`--json` is non-interactive (incompatible with `-t`).

### `d2b exec wait <EphemeralProcess>/<name>`

Wait for completion and exit with the process exit code.

```
d2b exec wait <EphemeralProcess>/<name> [--zone <zone>]
  [--deadline <duration>]
  [--json]
```

**Exit codes:** guest `WIFEXITED` code passes through (0–255). Reserved exit
codes (see table below) are only generated by transport/protocol failures.

### `d2b exec status <EphemeralProcess>/<name>`

```
d2b exec status <EphemeralProcess>/<name> [--zone <zone>]
  [--watch]
  [--deadline <duration>]
  [--json | --human]
```

**Successor to:** `d2b vm exec <vm> status <id>`.

### `d2b exec list [<executionRef>]`

```
d2b exec list [<executionRef>] [--zone <zone>]
  [--phase <phase>]
  [--json | --human]
```

**Successor to:** `d2b vm exec <vm> list`.

### `d2b exec logs <EphemeralProcess>/<name>`

```
d2b exec logs <EphemeralProcess>/<name> [--zone <zone>]
  [--stdout-offset <n>]
  [--stderr-offset <n>]
  [--max-len <n>]
  [--json]
```

**Successor to:** `d2b vm exec <vm> logs <id>`.

**Current v3 source:** `VmExecLogsArgs` in `packages/d2b/src/lib.rs:789`;
`cmd_vm_exec_management` at line 7397. Evidence class: `implemented-and-reachable`.

### `d2b exec kill <EphemeralProcess>/<name>`

```
d2b exec kill <EphemeralProcess>/<name> [--zone <zone>]
  [--signal <name>]
  [--json | --human]
```

Sends `ExecCancel` SIGTERM/grace/SIGKILL sequence.

**Successor to:** `d2b vm exec <vm> kill <id>`.

## `d2b shell` — ShellSession (persistent terminal sessions)

Shell commands interact with the `shell-terminal` Provider's `ShellSession`
ResourceType and attach to persistent named sessions.

### `d2b shell open <executionRef>`

Open a new persistent shell session or attach to an existing one.

```
d2b shell open <executionRef> [--zone <zone>]
  [--name <session-name>]
  [--force]
  [--json | --human]
```

Puts the host terminal in raw mode for the attached session. With `--json`,
emits the session resource ref and initial status without attaching.

**Successor to:** `d2b shell <target>` / `d2b shell <target> attach`.

**Current v3 source:** `cmd_shell` and `cmd_shell_attach` at
`packages/d2b/src/lib.rs:1788,1957`; shell FSM in `run_shell_fsm` at line
2092; `ShellOwnerTransport`/`TerminalTransport` traits; `exec_client.rs`
signal/TTY machinery. Evidence class: `implemented-and-reachable`.

**Current target argument:** The current `ShellArgs.vm: String` accepts a
`WorkloadId` (local VM name) or a `RealmTarget` of the form
`<workload>.<realm>.d2b`. The CLI dispatches through `route_vm_target()` (at
`lib.rs:5544`) which resolves to `VmTargetRoute::Local { vm }` (local shell) or
`VmTargetRoute::Gateway { realm, gateway_vm }` (cross-realm shell). The target
v3 arg is `executionRef: Host/<name> | Guest/<name>`.

**Gateway shell fails closed:** `cmd_gateway_shell` at `lib.rs:1770` returns
`shell_gateway_attach_failure()` (error class `gateway-shell-attach-unavailable`)
for `ShellAction::Attach`; only management verbs (list/detach/kill) route through
the gateway today. The v3 target removes this restriction via cross-Zone
ComponentSession.

**Retained behavior:** host terminal raw mode; RAII termios restore; SIGINT/
SIGTERM/SIGSTOP/SIGHUP signal forwarding; SIGWINCH window resize; shell session
supervision in a verified transient user scope; disconnect detaches but does not
kill; kill targets only the exact re-verified transient scope.

### `d2b shell attach <ShellSession>/<name>`

Attach to an existing named session.

```
d2b shell attach <ShellSession>/<name> [--zone <zone>]
  [--force]
```

**Successor to:** `d2b shell <target> attach --name <name>`.

### `d2b shell list [<executionRef>]`

```
d2b shell list [<executionRef>] [--zone <zone>]
  [--json | --human]
```

**Successor to:** `d2b shell <target> list`.

**Current v3 source:** `ShellAction::List` in `cmd_shell`; shell list dispatch
at `packages/d2b/src/lib.rs:1788`. Evidence class: `implemented-and-reachable`.

### `d2b shell detach <ShellSession>/<name>`

```
d2b shell detach <ShellSession>/<name> [--zone <zone>]
  [--json | --human]
```

**Successor to:** `d2b shell <target> detach --name <name>`.

### `d2b shell kill <ShellSession>/<name>`

```
d2b shell kill <ShellSession>/<name> [--zone <zone>]
  [--json | --human]
```

Terminates the session's transient user scope.

**Successor to:** `d2b shell <target> kill --name <name>`.

**Current v3 source:** `ShellAction::Kill` in `cmd_shell`. Exit code 1 when
`--name` is absent for kill retained. Evidence class: `implemented-and-reachable`.

### `d2b shell status <ShellSession>/<name>`

```
d2b shell status <ShellSession>/<name> [--zone <zone>]
  [--watch]
  [--deadline <duration>]
  [--json | --human]
```

## `d2b volume` — Volume resource commands

Maps to the `Volume` ResourceType.

```
d2b volume get <name>
d2b volume list
d2b volume status <name> [--watch] [--deadline <duration>]
d2b volume create [--spec-file <path> | --spec-stdin]
d2b volume update-spec <name> [--revision <rev>] [--spec-file <path> | --spec-stdin]
d2b volume delete <name> [--revision <rev>]
```

All accept `[--zone <zone>] [--json | --human]`.

### `d2b volume verify <name>`

Verify the Volume's backing state (hardlink farm integrity check for
`volume-local`).

```
d2b volume verify <name> [--zone <zone>]
  [--repair]
  [--json | --human]
```

**Successor to:** `d2b store verify <vm>`.

**Current v3 source:** `cmd_store_verify`; `StoreVerifyArgs` at
`packages/d2b/src/lib.rs:973`. Evidence class: `implemented-and-reachable`.

**Retained behavior:** non-destructive by default; `--repair` opts in to
repairs; exit code reflects verification result.

## `d2b network` — Network resource commands

Maps to the `Network` ResourceType.

```
d2b network get <name>
d2b network list
d2b network status <name> [--watch] [--deadline <duration>]
d2b network create [--spec-file <path> | --spec-stdin]
d2b network update-spec <name> [--revision <rev>] [--spec-file <path> | --spec-stdin]
d2b network delete <name> [--revision <rev>]
```

All accept `[--zone <zone>] [--json | --human]`.

## `d2b device` — Device resource commands

Maps to the `Device` ResourceType.

```
d2b device get <name>
d2b device list [--type <DeviceType>]
d2b device status <name> [--watch] [--deadline <duration>]
```

All accept `[--zone <zone>] [--json | --human]`.

### `d2b device usb attach <name> <busid>`

Bind a host USB busid to a declared Device resource.

```
d2b device usb attach <Device>/<name> <busid>
  [--zone <zone>]
  [--dry-run | --apply]
  [--json | --human]
```

**Successor to:** `d2b usb attach <vm> <busid>`.

**Current v3 source:** `cmd_usb_attach` at `packages/d2b/src/lib.rs`; UsbAttachArgs.
Evidence class: `implemented-and-reachable`.

### `d2b device usb detach <name> <busid>`

**Successor to:** `d2b usb detach <vm> <busid>`.

**Current v3 source:** `cmd_usb_detach`. Evidence class: `implemented-and-reachable`.

### `d2b device usb probe`

```
d2b device usb probe [--zone <zone>] [--json | --human]
```

**Successor to:** `d2b usb probe`.

**Current v3 source:** `cmd_usb_probe`. Evidence class: `implemented-and-reachable`.

### `d2b device security-key status`

```
d2b device security-key status [--zone <zone>] [--json | --human]
```

**Successor to:** `d2b usb security-key status`.

**Current v3 source:** `cmd_usb_sk_status`. Evidence class: `implemented-and-reachable`.

### `d2b device security-key sessions`

**Successor to:** `d2b usb security-key sessions`.

### `d2b device security-key cancel`

```
d2b device security-key cancel [<session-id> | --current]
  [--dry-run | --apply]
  [--json | --human]
```

**Successor to:** `d2b usb security-key cancel`.

### `d2b device security-key test <name>`

```
d2b device security-key test <Device>/<name> [--zone <zone>]
  [--dry-run] [--json | --human]
```

**Successor to:** `d2b usb security-key test <vm>`.

## `d2b user` — User resource commands

Maps to the `User` ResourceType.

```
d2b user get <name>
d2b user list
d2b user status <name>
```

All accept `[--zone <zone>] [--json | --human]`.

## `d2b credential` — Credential resource commands

Maps to the `Credential` ResourceType. Credential bytes are never surfaced
through the CLI; only opaque status/lease metadata is returned.

```
d2b credential get <name>
d2b credential list
d2b credential status <name> [--watch] [--deadline <duration>]
d2b credential delete <name>
```

All accept `[--zone <zone>] [--json | --human]`.

**Current v3 source:** No direct current CLI mapping.
Evidence class: `ADR-only`.

## `d2b provider` — Provider resource commands

### `d2b provider list`

```
d2b provider list [--zone <zone>]
  [--package-only]
  [--json | --human]
```

`--package-only` lists the offline package catalog without contacting the Zone
runtime.

**Current v3 source:** No direct mapping; closest is `cmd_list` which lists
runtime VMs. Evidence class: `ADR-only`.

### `d2b provider get <name>`

```
d2b provider get <name> [--zone <zone>]
  [--json | --human]
```

Returns the Provider resource envelope including component/service status.

### `d2b provider status <name>`

```
d2b provider status <name> [--zone <zone>]
  [--watch]
  [--deadline <duration>]
  [--json | --human]
```

### `d2b provider inspect <name>`

Return full Provider descriptor: exported ResourceTypes, schemas, services,
process templates, permission claims, CLI projection, dependency aliases.

```
d2b provider inspect <name> [--zone <zone>]
  [--json | --human]
```

**Relationship to dynamic descriptors:** The CLI fetches the CLI projection
sub-field of the Provider descriptor and renders the custom commands it
declares. See §Custom provider commands below.

**Current v3 source:** No direct mapping. Evidence class: `ADR-only`.

### Custom provider commands

A Provider may declare a bounded `cliProjection` in its descriptor. The
projection contains:

- a top-level subcommand name (e.g. `audio`, `display`);
- one or more sub-verb descriptors, each with: name, description, arguments
  (typed, bounded), required/optional flags, output schema;
- maximum total projection byte size: 64 KiB per Provider.

The CLI discovers projections by calling `InspectSchema` on each Ready
Provider with a `--deadline 2s` hard deadline per call. Missing or slow
Providers produce a visible warning but do not block CLI startup.

Custom commands are rendered as:

```
d2b <provider-name> <verb> [args...]
```

or, for Providers whose top-level name matches a built-in command name, as:

```
d2b provider run <name> <verb> [args...]
```

**Built-in name collision rule:** A Provider-projected command name that
collides with a built-in top-level verb is always rejected at Provider
install/bind time. Providers cannot shadow `get`, `list`, `watch`,
`create`, `update-spec`, `delete`, `status`, `host`, `guest`, `process`,
`exec`, `shell`, `volume`, `network`, `device`, `user`, `credential`,
`provider`, `zone`, `activation`, `audit`, `op`, `auth`, or `complete`.

**Retained built-in Provider commands:** The following current v3 Provider-
specific verbs are retained as first-class built-in commands because they have
stable existing tests and wire contracts:

| Retained built-in | Provider | Successor |
| --- | --- | --- |
| `d2b audio status/mic/speaker/off` | `audio-pipewire` | Retained via Provider CLI projection |
| `d2b clipboard arm` | `clipboard-wayland` | Retained via Provider CLI projection |
| `d2b console <name>` | Guest serial console | `d2b guest console <name>` |
| `d2b vm display list/close` | `display-wayland` | Retained via Provider CLI projection |

**Current v3 source (audio):** `cmd_audio` at `packages/d2b/src/lib.rs`; `AudioArgs`,
`AudioCommand`. Evidence class: `implemented-and-reachable`.

**Current v3 source (clipboard):** `cmd_clipboard_arm` at line 2555. Evidence
class: `implemented-and-reachable`.

**Provider projection loading:** Provider CLI projections are loaded lazily on
demand. A projection is fetched from the Provider's InspectSchema endpoint the
first time that Provider's projected subcommand is invoked or included in
`--help` output for `d2b provider`. Each fetch is bounded by the per-Provider
2-second deadline. The result is cached for the duration of the current CLI
invocation only. No disk cache or cross-invocation cache is maintained. Startup
latency for non-provider commands is zero.

## `d2b zone` — Zone resource commands

```
d2b zone get [<name>]           # omitting name fetches the current Zone self resource
d2b zone list                   # local ZoneLink resources (child Zones)
d2b zone status [<name>] [--watch] [--deadline <duration>]
```

All accept `[--json | --human]`.

**Successor to:** `d2b realm list`, `d2b realm inspect <realm>`.

**Current v3 source:** `cmd_realm_list`, `cmd_realm_inspect` at
`packages/d2b/src/lib.rs:5942,5958`. These read the **static Nix-generated file**
`realm-entrypoints.json` (path constant in `lib.rs`) via `realm_policy_rows_raw()` —
NOT a live daemon API call. The file contains a `RealmEntrypointDocument { entries:
BTreeMap<String, RealmEntrypointConfig> }` (from `d2b-realm-router` crate); entries
have `mode: host-resident|gateway-backed` and optional `gateway` VM name.
`cmd_realm_list` emits `RealmListOutputV1 { realms: Vec<RealmPolicyOutputV1> }`;
`cmd_realm_inspect` emits `RealmInspectOutputV1 { realm: RealmPolicyOutputV1 }`.
`RealmPolicyOutputV1` fields: `realm` (= `RealmId`), `mode`, `gateway_vm`, `gateway_target`,
`gateway_state`, `cross_realm_policy`, `credential_boundary` (from
`packages/d2b-contracts/src/cli_output.rs:345`).

Evidence class: `implemented-and-reachable` for realm list/inspect (static file read);
`ADR-only` for Zone/ZoneLink resource API (live daemon query replacing static file).

**Replacement/deletion:** `d2b realm enter` and `d2b realm run` are replaced by
`d2b guest start <gateway-guest>` plus `d2b exec run <gateway-guest> -- <cmd>`.
See migration table.

## `d2b quota` — Quota resource commands

Maps to the `Quota` ResourceType. Quota resources define resource-consumption
limits for a Zone or a subset of its resources. Quota and EmergencyPolicy are
separate resources; quota commands do not operate on `Zone.spec` (which is empty).

```
d2b quota get <name>          [--zone <zone>] [--json | --human]
d2b quota list                [--zone <zone>] [--json | --human]
d2b quota status <name>       [--zone <zone>] [--watch] [--deadline <dur>] [--json | --human]
d2b quota create              [--zone <zone>] [--spec-file <path> | --spec-stdin] [--json | --human]
d2b quota update-spec <name>  [--zone <zone>] [--revision <r>] [--spec-file <path> | --spec-stdin] [--json | --human]
d2b quota delete <name>       [--zone <zone>] [--revision <r>] [--json | --human]
```

**Successor to:** no current v3 equivalent.

**Evidence class:** `ADR-only`. Target implemented by standard resource verb
infrastructure (ADR046-cli-002).

## `d2b emergency-policy` — EmergencyPolicy resource commands

Maps to the `EmergencyPolicy` ResourceType. EmergencyPolicy resources define
Zone-wide emergency operational modes (e.g. forced shutdown, isolation,
credential revocation). They are separate resources from Zone, Host, and Guest;
`Zone.spec` is empty and carries no policy fields.

```
d2b emergency-policy get <name>          [--zone <zone>] [--json | --human]
d2b emergency-policy list                [--zone <zone>] [--json | --human]
d2b emergency-policy status <name>       [--zone <zone>] [--watch] [--deadline <dur>] [--json | --human]
d2b emergency-policy create              [--zone <zone>] [--spec-file <path> | --spec-stdin] [--json | --human]
d2b emergency-policy update-spec <name>  [--zone <zone>] [--revision <r>] [--spec-file <path> | --spec-stdin] [--json | --human]
d2b emergency-policy delete <name>       [--zone <zone>] [--revision <r>] [--json | --human]
```

**Successor to:** no current v3 equivalent.

**Evidence class:** `ADR-only`. Target implemented by standard resource verb
infrastructure (ADR046-cli-002).

**Help text** for both `d2b quota` and `d2b emergency-policy` MUST note that
`Zone.spec` is empty; quota limits and emergency policy are configured via
their own resource types, not through Zone fields.

**Tests:**

| Layer | Test | What it proves |
| --- | --- | --- |
| Unit | `d2b quota get/list/status/create/update-spec/delete` parse and route correctly | Quota resource noun wired |
| Unit | `d2b emergency-policy get/list/status/create/update-spec/delete` parse and route correctly | EmergencyPolicy resource noun wired |
| Unit | `Quota` and `EmergencyPolicy` accepted as frozen standard types (local validation) | Compile-time type list complete |
| Unit | `d2b get Quota/default` routes to Zone catalog without a round-trip for type validation | Local type validation path |
| Unit | `d2b get EmergencyPolicy/lockdown` routes correctly | Local type validation path |

## `d2b activation` — activation-nixos Provider commands

The `activation-nixos` Provider projects these CLI commands. They operate on
the `activation-nixos`-specific ResourceTypes.

### `d2b activation build <GuestRef>`

Non-destructive eval + build of the Guest NixOS toplevel.

```
d2b activation build <Guest>/<name> [--zone <zone>]
  [--json | --human]
```

**Successor to:** `d2b build <vm>`.

**Current v3 source:** `cmd_build` at `packages/d2b/src/lib.rs`. Evidence
class: `implemented-and-reachable`.

### `d2b activation generations <GuestRef>`

```
d2b activation generations <Guest>/<name> [--zone <zone>]
  [--json | --human]
```

**Successor to:** `d2b generations <vm>`.

### `d2b activation switch <GuestRef>`

Atomically activate a new closure.

```
d2b activation switch <Guest>/<name> [--zone <zone>]
  [--dry-run | --apply]
  [--json | --human]
```

**Successor to:** `d2b switch <vm>`.

### `d2b activation boot <GuestRef>`

Stage a closure for next boot only.

```
d2b activation boot <Guest>/<name> [--zone <zone>]
  [--dry-run | --apply]
  [--json | --human]
```

**Successor to:** `d2b boot <vm>`.

### `d2b activation test <GuestRef>`

Activate with rollback on reboot.

```
d2b activation test <Guest>/<name> [--zone <zone>]
  [--dry-run | --apply]
  [--json | --human]
```

**Successor to:** `d2b test <vm>`.

### `d2b activation rollback <GuestRef>`

Roll back to previous generation.

```
d2b activation rollback <Guest>/<name> [--zone <zone>]
  [--dry-run | --apply]
  [--json | --human]
```

**Successor to:** `d2b rollback <vm>`.

### `d2b activation gc`

Garbage-collect hardlink farms.

```
d2b activation gc [--zone <zone>]
  [--dry-run | --apply]
  [--json | --human]
```

**Successor to:** `d2b gc`.

### `d2b activation migrate`

```
d2b activation migrate [--zone <zone>]
  [--dry-run | --apply]
  [--json | --human]
```

**Successor to:** `d2b migrate`.

### `d2b activation keys` — managed SSH key lifecycle

```
d2b activation keys list [--json | --human]
d2b activation keys show <name> [--json | --human]
d2b activation keys rotate <name> [--dry-run | --apply] [--json | --human]
d2b activation trust <name> [--json | --human]
d2b activation rotate-known-host <name> [--json | --human]
```

**Successor to:** `d2b keys list/show/rotate`, `d2b trust <vm>`,
`d2b rotate-known-host <vm>`.

**Current v3 source:** `cmd_keys_list`, `cmd_keys_show`, `cmd_keys_rotate`,
`cmd_keys_trust`, `cmd_keys_rotate_known_host` at
`packages/d2b/src/lib.rs:986ff`. Evidence class: `implemented-and-reachable`.

### `d2b activation config` — guest-editable config lifecycle

```
d2b activation config sync <GuestRef> [--dry-run] [--json]
d2b activation config diff <GuestRef> --against <path> [--json]
d2b activation config approve <GuestRef> --to <path> [--json]
d2b activation config reject <GuestRef> [--json]
d2b activation config status <GuestRef> [--json]
```

**Successor to:** `d2b config sync/diff/approve/reject/status`.

**Current v3 source:** `ConfigCommand` variants; `cmd_config_sync` and
related at `packages/d2b/src/lib.rs:2699ff`. Evidence class:
`implemented-and-reachable`.

**Retained behavior:** guest-control transport for sync (no SSH); staging file
lifecycle; diff/approve/reject workflow.

### `d2b guest console <name>`

Foreground serial console bridge for headless Guests.

```
d2b guest console <name> [--zone <zone>]
```

**Successor to:** `d2b console <vm>`.

**Current v3 source:** `cmd_console` at `packages/d2b/src/lib.rs`. Evidence
class: `implemented-and-reachable`.

## `d2b audit` — audit log streaming

```
d2b audit [--zone <zone>]
  [--strict]
  [--deadline <duration>]
  [--json | --human]
```

**Successor to:** `d2b audit`.

**Current v3 source:** `cmd_audit` at `packages/d2b/src/lib.rs`. Evidence
class: `implemented-and-reachable`.

**Retained behavior:** streaming audit lines; `--strict` (exit non-zero on
parse error); bounded line lengths; redacted payloads.

## `d2b op` — operation inspection

```
d2b op inspect [--zone <zone>]
  [--operation-id <id>]
  [--trace-id <id>]
  [--json | --human]
```

**Successor to:** `d2b op inspect`.

**Current v3 source:** `cmd_op_inspect` at `packages/d2b/src/lib.rs:6098`.
Evidence class: `implemented-and-reachable`.

## `d2b auth` — authorization status

```
d2b auth status [--zone <zone>]
  [--json | --human]
```

**Successor to:** `d2b auth status`.

**Current v3 source:** `cmd_auth_status` at `packages/d2b/src/lib.rs`. Evidence
class: `implemented-and-reachable`.

**Retained behavior:** SO_PEERCRED group-membership check display; no test-uid
argument exposed in v3.

## `d2b complete` — shell completion

```
d2b complete <shell>         # emit completion script (bash | zsh | fish)
d2b complete --list-commands # emit available top-level commands as JSON
```

Completion discovery fetches Provider CLI projections with a hard 2s aggregate
deadline. Missing Providers produce a stable degraded list without error.
Completion output is bounded: maximum 256 KiB total.

**Current v3 source:** No shell completion implemented in v3 baseline
(`packages/d2b/src/lib.rs` has no `clap_complete` reference). Evidence
class: `ADR-only`.

## v2 command surface removed at 3.0 clean break

All v2 aliases and predecessor commands are deleted at the d2b 3.0 clean break.
There are no executable aliases in 3.0. A `d2b migrate-check` diagnostic
command may explain replacements, but it does not dispatch to v2 behavior.
The table below records each removed command and its v3 successor for
documentation and test-removal tracking only.

| Removed v2 command | v3 successor |
| --- | --- |
| `d2b up <name>` | `d2b guest start <name>` |
| `d2b down <name>` | `d2b guest stop <name>` |
| `d2b restart <name>` | `d2b guest restart <name>` |
| `d2b list` | `d2b guest list` |
| `d2b status [<name>]` | `d2b guest status <name>` |
| `d2b vm start/stop/restart/list/status` | `d2b guest start/stop/restart/list/status` |
| `d2b vm exec <vm> -- <cmd>` | `d2b exec run/attach` |
| `d2b vm exec <vm> list/logs/status/kill` | `d2b exec list/logs/status/kill` |
| `d2b realm list` | `d2b zone list` |
| `d2b realm inspect <r>` | `d2b zone get <r>` |
| `d2b realm enter <r>` | `d2b shell open <gateway-guest>` |
| `d2b realm run <r> -- <cmd>` | `d2b exec run <gateway-guest> -- <cmd>` |
| `d2b usb attach/detach/probe` | `d2b device usb attach/detach/probe` |
| `d2b usb security-key <sub>` | `d2b device security-key <sub>` |
| `d2b store verify <vm>` | `d2b volume verify <Volume>/<name>` |
| `d2b keys list/show/rotate` | `d2b activation keys list/show/rotate` |
| `d2b trust <name>` | `d2b activation trust <name>` |
| `d2b rotate-known-host <name>` | `d2b activation rotate-known-host <name>` |
| `d2b build <name>` | `d2b activation build <name>` |
| `d2b generations <name>` | `d2b activation generations <name>` |
| `d2b switch/boot/test/rollback <name>` | `d2b activation switch/boot/test/rollback <name>` |
| `d2b gc` | `d2b activation gc` |
| `d2b migrate` | `d2b activation migrate` |
| `d2b config sync/diff/approve/reject/status` | `d2b activation config sync/...` |
| `d2b console <name>` | `d2b guest console <name>` |
| `d2b vm display list/close` | `d2b provider run display-wayland display list/close` |

## Async operation lifecycle, status watch, and cancel

### `d2b op inspect` and operation IDs

Every mutating resource API call returns an operation ID in the JSON output
envelope as `operationId`. Long-running operations (Guest start/stop, Provider
lifecycle, activation switch) also emit it in human output.

```
d2b op inspect --operation-id <id> [--zone <zone>]
  [--watch]
  [--deadline <duration>]
  [--json | --human]
```

### Deadline and cancel flags

All commands that interact with the Zone runtime accept:

```
--deadline <duration>    # wall deadline (e.g. 30s, 5m); default per-command
--no-deadline            # suppress default deadline (use with care)
```

Deadline exhausted: exit code 1, `deadline-exceeded` error class, `--json`
envelope with `errorClass: "deadline-exceeded"`.

SIGINT / SIGTERM: attempt graceful cancel of the in-flight operation, emit a
cancel event envelope, restore terminal state, then exit with code 3.

SIGHUP: detach from interactive sessions (shell/exec attach) without killing
the remote session.

**Current v3 source:** Deadline partially implemented: `EXIT_API_TIMEOUT` (33)
for vm-start; exec transport deadline at `EXIT_EXEC_TRANSPORT` (69). Evidence
class: `implemented-but-unwired` for general deadline; `implemented-and-reachable`
for specific paths.

**Required delta:** Unified `deadline-exceeded` error class across all resource
API paths; SIGINT/SIGTERM cancel dispatch; `--deadline` flag on all API-backed
commands.

## Output format

### `--json` envelope

Every command's `--json` output is a single JSON object (for non-streaming
commands) or a stream of newline-separated JSON objects (for `watch`, `exec
attach`, `audit`).

Common fields in every `--json` envelope:

```json
{
  "ok": true,
  "zoneRef": "Zone/dev",
  "operationId": "<operation-id>",
  "schemaVersion": 1,
  "resource": { /* canonical ResourceSpec envelope — see §Canonical ResourceSpec JSON shape */ }
}
```

Error envelopes:

```json
{
  "ok": false,
  "zoneRef": "Zone/dev",
  "errorClass": "resource-not-found",
  "message": "bounded redacted operator detail",
  "remediation": "optional actionable hint"
}
```

The `message` and `remediation` fields are bounded (max 4096 bytes each),
UTF-8 validated, and must not contain secrets, tokens, credential bytes, argv/
environment, terminal bytes, or host paths.

**Stability:** JSON field names are stable across patch and minor versions.
Additive new fields may appear; a consumer must use `deny_unknown_fields = false`
on JSON deserialization.

**Versioning:** The `--json` envelope carries `"schemaVersion": 1` in the root.
Minor bumps add optional fields; major bumps increment the version.

### `--human` output

Human output is for interactive terminals only. Format is not machine-parseable
and may change between patch versions.

**Format defaulting rule:** `--json` and `--human` are mutually exclusive. When
neither is given: if stdout is a TTY, human output is produced; if stdout is
not a TTY, JSON is produced (script-friendly default). `--human` explicitly
overrides to human output even in non-TTY contexts; `--json` explicitly
overrides to JSON even in TTY contexts.

**Current v3 source:** `stdout_is_tty()` at
`packages/d2b/src/lib.rs:10302`. Evidence class: `implemented-and-reachable`.

## Error classes and exit codes

### Stable error classes

All error class strings are stable lower-kebab-case machine values.

| Error class | Meaning |
| --- | --- |
| `resource-not-found` | Target resource does not exist |
| `resource-already-exists` | Create precondition: resource exists |
| `resource-conflict` | Optimistic concurrency mismatch |
| `resource-schema-invalid` | Spec/status payload fails schema validation |
| `ref-invalid` | ResourceRef syntax or resolution failure |
| `authorization-denied` | RBAC or structural check denied the request |
| `zone-unavailable` | Zone runtime not reachable |
| `deadline-exceeded` | Wall deadline elapsed |
| `operation-cancelled` | SIGINT/SIGTERM graceful cancel |
| `provider-unavailable` | Target Provider is not Ready |
| `exec-transport-error` | Exec vsock/session transport unreachable or deadline |
| `exec-old-generation` | VM generation does not support guest-control exec |
| `exec-capacity` | Exec session table at capacity or rate-limited |
| `exec-protocol-error` | Malformed/out-of-contract guest response |
| `exec-auth-error` | Guest-control handshake rejected |
| `exec-internal-error` | CLI/daemon-internal failure |
| `shell-transport-error` | Shell session transport failure |
| `not-implemented` | Requested verb is not implemented in this runtime version |
| `internal-error` | Unexpected CLI/runtime internal failure |
| `bundle-integrity-failure` | Zone resource bundle SHA256 pin mismatch |
| `bundle-generation-replay` | Submitted bundleGeneration ≤ last applied generation |
| `bundle-schema-mismatch` | Bundle resourceTypeSchemaDigests do not match installed schemas |
| `resource-pending-cleanup` | Resource has outstanding deletion that must complete or be force-removed |

### Stable exit codes

| Exit code | Class | Condition |
| --- | --- | --- |
| 0 | success | Command completed successfully |
| 1 | error | Operational failure (not-found, unavailable, zone-down, etc.) |
| 2 | usage | Argument/parsing/validation error |
| 3 | cancelled | Cancelled by SIGINT/SIGTERM/deadline (interactive streams) |
| 33 | `deadline-exceeded` | API-ready wait timeout (retained from v3: `EXIT_API_TIMEOUT`) |
| 42 | `exec-internal-error` | CLI/daemon internal exec failure (`EXIT_EXEC_INTERNAL`) |
| 69 | `exec-transport-error` | Exec transport unreachable or deadline (`EXIT_EXEC_TRANSPORT`) |
| 70 | `exec-old-generation` | Old VM generation (`EXIT_EXEC_OLD_GENERATION`, `EXIT_GUEST_CONTROL_CONFIG`) |
| 75 | `exec-capacity` | Exec session table at capacity (`EXIT_EXEC_CAPACITY`) |
| 76 | `exec-protocol-error` | Malformed guest response (`EXIT_EXEC_PROTOCOL`) |
| 77 | `exec-auth-error` | Guest-control handshake rejected (`EXIT_EXEC_AUTH`) |
| 78 | `not-implemented` | Runtime returned `not-yet-implemented` |
| 128+N | signal | Terminated by signal N (non-TTY context only) |

Guest WIFEXITED 0–255 pass through `exec attach`/`exec wait` and can collide
with the reserved codes above. The `--json` envelope disambiguates via
`source`/`reason`/`guestExitCode`/`transportExitCode`.

**Host-check exit code 3:** `d2b host check` uses exit code 3 for usage errors
(retained from v3 baseline `is_host_usage` check at
`packages/d2b/src/lib.rs:2405`). This is the one documented exception to the
standard exit code 2 usage rule.

**Current v3 source:** `EXIT_API_TIMEOUT=33`, `EXIT_GUEST_CONTROL_CONFIG=70`,
`EXIT_EXEC_TRANSPORT=69`, `EXIT_EXEC_OLD_GENERATION=70`,
`EXIT_EXEC_CAPACITY=75`, `EXIT_EXEC_PROTOCOL=76`, `EXIT_EXEC_AUTH=77`,
`EXIT_EXEC_INTERNAL=42` in
`packages/d2b/src/lib.rs:92,98` and
`packages/d2b/src/exec_client.rs:32-46`.
Evidence class: `implemented-and-reachable`.

## Streams and TTY contract

### Attached exec/shell streams

When `-t` / `--tty` is given:

1. The CLI verifies stdout is a TTY (`stdout.is_terminal()`).
2. The CLI enters raw mode on the host terminal (`tcsetattr(STDIN, TCSANOW)`).
3. An RAII guard restores termios on every code path (success, error, panic,
   signal).
4. The guest owns the PTY. The host streams bytes bidirectionally without
   interpretation.
5. SIGWINCH triggers a resize notification to the guest.
6. SIGINT/SIGTERM/SIGHUP are forwarded as typed signals to the guest session.
7. SIGHUP in a shell context detaches without killing.
8. On disconnect, raw mode is restored before any error message is printed to
   stderr.

`--tty` is incompatible with `--json` (the JSON envelope cannot capture raw
terminal bytes).

**Current v3 source:** `exec_client::FdStateGuard`, `exec_client::install_signals`,
`exec_client::current_window_size`, `run_shell_fsm`, `ShellOwnerTransport` in
`packages/d2b/src/lib.rs:1957,2005,2092` and
`packages/d2b/src/exec_client.rs`. Evidence class: `implemented-and-reachable`.

### Named streams

The Zone resource API Watch and exec/shell attach operations use named streams
over ComponentSession. The CLI does not manage credit flow directly; that is
the ComponentSession layer's responsibility.

### Audit and watch streams

Non-TTY streaming commands (`d2b audit`, `d2b watch`, `d2b exec logs`)
write newline-terminated JSON objects to stdout continuously. Each object is
valid JSON on its own line. Buffering is flushed after each object. The stream
closes when the server closes, the deadline is reached, or a signal is received.

## Completion and help

### `--help` output

Every command and subcommand emits a usage block followed by a description.
The description is bounded (max 2048 chars per command). Process-markers, wave
tags, or internal identifiers are not present in help text.

Dynamic Provider-projected commands appear in the top-level `--help` listing
under a `PROVIDER COMMANDS` section with a `(provider: <name>)` annotation and
a bounded description.

### Shell completion

Shell completion scripts (bash, zsh, fish) are generated by `d2b complete
<shell>`. Completion discovery:

1. Fetches the list of Ready Providers from the Zone runtime (2s deadline).
2. For each Provider with a `cliProjection`, fetches the projection (2s
   deadline per call).
3. Generates completion for all built-in + projected commands.
4. Result is bounded at 256 KiB.

A timeout or unreachable Zone runtime produces completion for built-in commands
only, with no error.

**Current v3 source:** No completion exists in v3 baseline. Evidence class:
`ADR-only`.

## Dynamic descriptors — safety bounds

Provider CLI projections are Provider-supplied content. The CLI applies these
bounds before rendering any projection:

| Bound | Limit |
| --- | --- |
| Total bytes per projection | 64 KiB |
| Top-level subcommand name length | 32 bytes; `^[a-z][a-z0-9-]*$` |
| Sub-verb name length | 32 bytes; `^[a-z][a-z0-9-]*$` |
| Number of sub-verbs per Provider | 32 |
| Argument name length | 64 bytes |
| Number of arguments per sub-verb | 16 |
| Description length per sub-verb | 512 bytes |
| Fetch deadline per Provider | 2 s |
| Total fetch deadline (all Providers) | 10 s |

A projection that violates any bound is silently skipped with a single
`d2b: provider <name> cli-projection exceeded limit` line to stderr. No
Provider projection can cause the CLI to crash, loop, allocate unbounded memory,
or emit shell-injectable text.

Completion strings are HTML/shell-escaped before inclusion in completion
scripts. Newlines in dynamic strings are replaced with a space.

## No provider code in CLI

The CLI binary must not:

- import Provider implementation crates;
- import `d2bd`, broker, or Zone-store implementation crates;
- invoke Provider subprocesses;
- read Provider-specific configuration files (only the standard Zone socket and
  optional env overrides);
- perform sandbox compilation or minijail argument construction;
- hold or transmit credential bytes.

The CLI may import `d2b-contracts` for shared DTO types, `d2b-resource-api` for
the typed async client, and `d2b-session`/`d2b-session-unix` for
ComponentSession transport.

## Current verb/target/contract migration table

The following table maps every current v3 CLI verb to its v3 status.

| Current verb | Status | v3 successor | Current source | Evidence class |
| --- | --- | --- | --- | --- |
| `d2b list` | Deleted at 3.0 | `d2b guest list` | `cmd_list` @ lib.rs:3560 | reachable |
| `d2b status [<vm>]` | Deleted at 3.0 | `d2b guest status <name>` | `cmd_status` @ lib.rs:3587 | reachable |
| `d2b up <vm>` | Deleted at 3.0 | `d2b guest start <name>` | `cmd_vm_start` via alias | reachable |
| `d2b down <vm>` | Deleted at 3.0 | `d2b guest stop <name>` | `cmd_vm_stop` via alias | reachable |
| `d2b restart <vm>` | Deleted at 3.0 | `d2b guest restart <name>` | `cmd_vm_restart` via alias | reachable |
| `d2b vm start <vm>` | Deleted at 3.0 | `d2b guest start <name>` | `cmd_vm_start` @ lib.rs:6675 | reachable |
| `d2b vm stop <vm>` | Deleted at 3.0 | `d2b guest stop <name>` | `cmd_vm_stop` @ lib.rs:6690 | reachable |
| `d2b vm restart <vm>` | Deleted at 3.0 | `d2b guest restart <name>` | `cmd_vm_restart` @ lib.rs:6690 | reachable |
| `d2b vm list` | Deleted at 3.0 | `d2b guest list` | `cmd_vm_list` @ lib.rs | reachable |
| `d2b vm status <vm>` | Deleted at 3.0 | `d2b guest status <name>` | `cmd_vm_status` @ lib.rs:6868 | reachable |
| `d2b vm exec <vm> -- <cmd>` | Deleted at 3.0 | `d2b exec run/attach` | `cmd_vm_exec` @ lib.rs:7166 | reachable |
| `d2b vm exec <vm> list` | Deleted at 3.0 | `d2b exec list` | `cmd_vm_exec_management` @ lib.rs:7397 | reachable |
| `d2b vm exec <vm> logs <id>` | Deleted at 3.0 | `d2b exec logs` | same | reachable |
| `d2b vm exec <vm> status <id>` | Deleted at 3.0 | `d2b exec status` | same | reachable |
| `d2b vm exec <vm> kill <id>` | Deleted at 3.0 | `d2b exec kill` | same | reachable |
| `d2b vm display list/close` | Provider-projected | `audio-pipewire`/`display-wayland` projection | `cmd_vm_display` @ lib.rs | reachable |
| `d2b shell <target> [attach]` | Deleted at 3.0 | `d2b shell open <executionRef>` | `cmd_shell` @ lib.rs:1788 | reachable |
| `d2b shell <target> list` | Deleted at 3.0 | `d2b shell list` | same | reachable |
| `d2b shell <target> detach` | Deleted at 3.0 | `d2b shell detach` | same | reachable |
| `d2b shell <target> kill` | Deleted at 3.0 | `d2b shell kill` | same | reachable |
| `d2b usb attach <vm> <busid>` | Deleted at 3.0 | `d2b device usb attach` | `cmd_usb_attach` @ lib.rs | reachable |
| `d2b usb detach <vm> <busid>` | Deleted at 3.0 | `d2b device usb detach` | `cmd_usb_detach` | reachable |
| `d2b usb probe` | Deleted at 3.0 | `d2b device usb probe` | `cmd_usb_probe` | reachable |
| `d2b usb security-key status` | Deleted at 3.0 | `d2b device security-key status` | `cmd_usb_sk_status` | reachable |
| `d2b usb security-key sessions` | Deleted at 3.0 | `d2b device security-key sessions` | `cmd_usb_sk_sessions` | reachable |
| `d2b usb security-key cancel` | Deleted at 3.0 | `d2b device security-key cancel` | `cmd_usb_sk_cancel` | reachable |
| `d2b usb security-key test <vm>` | Deleted at 3.0 | `d2b device security-key test` | `cmd_usb_sk_test` | reachable |
| `d2b console <vm>` | Deleted at 3.0 | `d2b guest console <name>` | `cmd_console` | reachable |
| `d2b audio status/mic/speaker/off` | Provider-projected | `audio-pipewire` projection | `cmd_audio` @ lib.rs | reachable |
| `d2b audit` | **Retained** | `d2b audit` | `cmd_audit` @ lib.rs | reachable |
| `d2b host check` | **Retained** | `d2b host check` | `cmd_host_check` @ lib.rs:4271 | reachable |
| `d2b host prepare` | **Retained** | `d2b host prepare` | `cmd_host_prepare` @ lib.rs:4468 | reachable |
| `d2b host destroy` | **Retained** | `d2b host destroy` | `cmd_host_destroy` @ lib.rs:4549 | reachable |
| `d2b host doctor` | **Retained** | `d2b host doctor` | `cmd_host_doctor` @ lib.rs:4752 | reachable |
| `d2b host migrate-storage` | Deleted after reset | *(none; storage ADR 0034 reset)* | `cmd_host_migrate_storage` @ lib.rs:4867 | reachable |
| `d2b host install` | **Retained** | `d2b host install` | `cmd_host_install` @ lib.rs:5007 | reachable |
| `d2b host reconcile` | **Retained** | `d2b host reconcile` | `cmd_host_reconcile` @ lib.rs:5070 | reachable |
| `d2b host validate` | **Retained** | `d2b host validate` | `cmd_host_validate` @ lib.rs:4941 | reachable |
| `d2b host shutdown-hook` | **Retained** internal | `d2b host shutdown-hook` (hidden) | `cmd_host_shutdown_hook` @ lib.rs:4644 | reachable |
| `d2b realm list` | Deleted at 3.0 | `d2b zone list` | `cmd_realm_list` @ lib.rs:5942 | reachable |
| `d2b realm inspect <r>` | Deleted at 3.0 | `d2b zone get <r>` | `cmd_realm_inspect` @ lib.rs:5958 | reachable |
| `d2b realm enter <r>` | Deleted at 3.0 | `d2b shell open <gateway>` | `cmd_realm_enter` @ lib.rs:6129 | reachable |
| `d2b realm run <r> -- <cmd>` | Deleted at 3.0 | `d2b exec run <gateway> -- <cmd>` | `cmd_realm_run` @ lib.rs:6143 | reachable |
| `d2b op inspect` | **Retained** | `d2b op inspect` | `cmd_op_inspect` @ lib.rs:6098 | reachable |
| `d2b auth status` | **Retained** | `d2b auth status` | `cmd_auth_status` @ lib.rs | reachable |
| `d2b store verify <vm>` | Deleted at 3.0 | `d2b volume verify` | `cmd_store_verify` @ lib.rs | reachable |
| `d2b keys list/show/rotate` | Deleted at 3.0 | `d2b activation keys list/show/rotate` | `cmd_keys_*` @ lib.rs:986 | reachable |
| `d2b trust <name>` | Deleted at 3.0 | `d2b activation trust <name>` | `cmd_keys_trust` | reachable |
| `d2b rotate-known-host <name>` | Deleted at 3.0 | `d2b activation rotate-known-host` | `cmd_keys_rotate_known_host` | reachable |
| `d2b build <vm>` | Deleted at 3.0 | `d2b activation build` | `cmd_build` | reachable |
| `d2b generations <vm>` | Deleted at 3.0 | `d2b activation generations` | `cmd_generations` | reachable |
| `d2b switch <vm>` | Deleted at 3.0 | `d2b activation switch` | `cmd_switch` | reachable |
| `d2b boot <vm>` | Deleted at 3.0 | `d2b activation boot` | `cmd_boot` | reachable |
| `d2b test <vm>` | Deleted at 3.0 | `d2b activation test` | `cmd_test` | reachable |
| `d2b rollback <vm>` | Deleted at 3.0 | `d2b activation rollback` | `cmd_rollback` | reachable |
| `d2b gc` | Deleted at 3.0 | `d2b activation gc` | `cmd_gc` | reachable |
| `d2b migrate` | Deleted at 3.0 | `d2b activation migrate` | `cmd_migrate` | reachable |
| `d2b config sync/diff/approve/reject/status` | Deleted at 3.0 | `d2b activation config *` | `ConfigCommand` @ lib.rs:2705 | reachable |
| `d2b clipboard arm` | Provider-projected | `clipboard-wayland` projection | `cmd_clipboard_arm` @ lib.rs | reachable |
| `d2b clipboard picker` | **Deleted** (was already deprecated) | *(none)* | deprecated notice @ lib.rs:2424 | reachable |

### Removal notes

All commands listed as "Deleted at 3.0" in the full verb/target table
below are deleted at the 3.0 clean break. The removal criterion for each is:
its live v3 successor is implemented and tested, and the v2 source path
(`cmd_*` function) is deleted in the same wave.

`d2b host migrate-storage` is deleted after the v3 storage reset completes; it
has no v3 successor because the layout cutover it served is a one-time v1→v2
migration.

`d2b clipboard picker` is already removed from the dispatch table in the v3
baseline (only a deprecation notice remains at `lib.rs:2424`).

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | `packages/d2b/src/lib.rs` (18 599 lines), `exec_client.rs`, `terminal_client.rs`, `target_routing.rs`, `human_render.rs`, `status_read_model.rs` at baseline `b5ddbed6`; supplementary: `packages/d2b-contracts/src/public_wire.rs` (`ListEntry`, `VmStatus`, `VmLifecycleState`, `ShellOp`, `ShellOpResponse`, `WorkloadPublicSummary.execution_posture`), `packages/d2b-contracts/src/cli_output.rs` (`RealmListOutputV1`, `RealmInspectOutputV1`, `RealmPolicyOutputV1`), `packages/d2b-contracts/src/unsafe_local_wire.rs` (unsafe-local helper wire protocol), `packages/d2b-unsafe-local-helper/src/` (runtime helper), `packages/d2b-realm-core/src/workload.rs` (`WorkloadProviderKind`, `IsolationPosture::UnsafeLocal`, `WorkloadExecutionPosture`), `packages/d2b-core/src/processes.rs` (`ProcessRole`, `VmProcessDag`), `nixos-modules/options-realms-workloads.nix` (`kind = "unsafe-local"`), `nixos-modules/unsafe-local-workloads-json.nix`. Note: baseline uses pre-ADR 0046 terminology throughout (Realm/WorkloadId/ProcessRole/VmProcessDag etc.); see "Baseline terminology mapping" above. Unsafe-local workloads are `Host` resources NEVER `Guest`; see §Unsafe-local Host resources. |
| Evidence class | All dispatched NativeCommand variants are `implemented-and-reachable`; Zone/resource API routing, Provider CLI projection, `d2b complete`, `d2b exec run/attach/wait`, `d2b shell open/attach`, `d2b guest/process/volume/network/device/user/credential/provider/zone/activation` are `ADR-only` |
| Behavior retained | All documented exit codes; `--json`/`--human` mutual exclusion with non-TTY JSON default and `--human` explicit override; TTY raw-mode RAII; SIGWINCH/SIGINT/SIGTERM forwarding; dry-run/apply pattern; daemon-down/not-implemented envelopes; host-check exit 3; bounded output; no bash fallback; no SSH for exec/shell |
| Required delta | Zone context discovery; ResourceRef argument parsing; resource API client routing; standard `get/list/watch/create/update-spec/delete/status` verb set; `d2b exec`, `d2b shell`, `d2b guest`, `d2b process`, `d2b volume`, `d2b network`, `d2b device`, `d2b user`, `d2b credential`, `d2b provider`, `d2b zone`, `d2b quota`, `d2b emergency-policy`, `d2b activation`, `d2b complete`; Provider CLI projection loading; unified deadline/cancel; JSON schema version field; `--zone` flag |
| Reuse path | Retain current `run()` entry, clap dispatch, `CliFailure`/`report_failure`, `exec_client.rs` FSM/signals/TTY, `terminal_client.rs` traits; adapt `Context` for Zone/socket discovery; replace seqpacket with ComponentSession resource API client |
| Replacement/deletion | Raw seqpacket send/recv helpers replaced only after resource API client paths are live and tested; old `public_wire` / `broker_wire` wire envelopes retired per owning resource ResourceType wave |
| Feasibility proof | Existing exec FSM/terminal/signal test suite; clap dispatch tests; all 150+ unit tests passing at baseline |
| Future owner | Work items below |

## Implementation work items

### ADR046-cli-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-cli-001` |
| Dependency/owner | ADR046-identities-001, ADR046-api-001; CLI crate owner |
| Current source | `packages/d2b/src/lib.rs`: `Context::from_env` (reads `D2B_PUBLIC_SOCKET` env or `/run/d2b/public.sock` default; single flat socket path — no zone-qualified path, no `ZoneContext`), `NativeCli`, `NativeCommand`, `dispatch`, `CliFailure`, `report_failure`, `stdout_is_tty`; socket path model: old `Context { public_socket, broker_socket }` → target: `ZoneContext { zone_name, socket_path, session_client }` |
| Reuse source | main `a1cc0b2d` — copy/adapt these exact symbols: (1) `packages/d2b-client/src/client.rs` `Client<R,C,W>`, `ConnectedClient`, `CallOptions`, `CancellationToken`, `MetadataInput`, `RetryPolicy`, `Response` — copy unchanged as the async client foundation; (2) `packages/d2b-client/src/host_socket.rs` `HostSocketConnector::from_seqpacket_fd`, `local_daemon_endpoint_identity` — adapt: replace fixed `d2b.daemon.v2` service lookup with zone-scoped service identity; replace hardcoded `RealmPath::parse("local-root")` with `Zone/<name>`-derived target; (3) `packages/d2b-daemon-access/src/component_session.rs` `LocalUnixDaemonAccess::connect_component_session()` connect chain — adapt: replace `TargetInput::LocalRoot(realm)` with the v3 Zone target variant; adapt socket path discovery for per-Zone paths; (4) `packages/d2b-client/src/session.rs` `ComponentSessionConnector`, `ConnectedSession`, `NamedStream`, `SessionCall`, `SessionReply`, `SharedDriver` — copy unchanged; (5) `packages/d2b-client/src/target.rs` `RouteTable`, `RouteRecord`, `TargetInput`, `ResolvedTarget`, `TransportKind`, `TransportSelection` — copy unchanged, excluding `TargetInput::Realm/Workload/Provider` variants which carry ADR 0045 assumptions; (6) `packages/d2b-contracts/src/v2_component_session.rs` `LimitProfile::local_default()` constants: `MAX_REQUEST_LIFETIME_MS=900000`, `LOCAL_HANDSHAKE_DEADLINE_MS=5000`, `MAX_RECONNECT_ATTEMPTS=10`, `MAX_ACTIVE_NAMED_STREAMS=128`, `MAX_LOGICAL_MESSAGE_BYTES=1048576`, `named_stream_queue_bytes=262144`, `aggregate_named_stream_queue_bytes=4194304` — copy unchanged; these bound every CLI deadline and stream operation |
| Reuse action | adapt |
| Destination | `packages/d2b/src/lib.rs`, `packages/d2b/src/context.rs`, `packages/d2b/src/dispatch.rs` |
| Detailed design | Introduce `ZoneContext` (zone name, socket path, ComponentSession client); implement `--zone`/`D2B_ZONE`/nearest-socket discovery using adapted `LocalUnixDaemonAccess::connect_component_session()` chain; introduce `ResourceRef` argument parser; introduce unified `--json`/`--human`/`--deadline` flag infrastructure bounded by `MAX_REQUEST_LIFETIME_MS=900s`; freeze `--json` schema version 1; stabilize exit code table. Excluded ADR 0045 assumptions: `TargetInput::Realm`, `TargetInput::Workload`, `TargetInput::Provider` variants; `RealmPath::parse("local-root")` / `RealmId::derive` pattern; `RealmPath`-based service owner types. |
| Integration | All command functions receive `ZoneContext`; resource API client (`Client<RouteTable, HostSocketConnector>`) is injected for testing |
| Data migration | None; context discovery replaces env-var path lookups |
| Validation | Zone-unavailable/fallback tests; ResourceRef parse/reject vectors; exit-code round-trip tests; TTY detection tests; adapt `client.rs:typed_routes_select_exact_transport_without_fallback` (line 1053), `connector_discovers_and_authenticates_the_driver_generation` (line 1254), `daemon_transport_rejects_ancillary_data_and_oversized_packets` (line 1312) |
| Removal proof | Old `Context` struct removed only after all command functions use `ZoneContext` |

### ADR046-cli-002

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-cli-002` |
| Dependency/owner | ADR046-cli-001, ADR046-api-001; CLI crate owner |
| Current source | `packages/d2b/src/lib.rs`: `cmd_vm_start`, `cmd_vm_stop`, `cmd_vm_restart`, `cmd_vm_status`, `cmd_vm_list`, `cmd_list`, `cmd_status`; wire types: `ListResponse { vms: Vec<ListEntry> }`, `StatusResponse { entries: Vec<VmStatus> }`, `VmLifecycleState` (old: Stopped/Starting/Booted/Running/Stopping/Restarting/Failed/Unknown) from `packages/d2b-contracts/src/public_wire.rs:2152,2158,2605`; `ListEntry.vm: String` = `WorkloadId`; `VmStatus.lifecycle.state: VmLifecycleState` → target Guest `phase` (Pending\|Ready\|Succeeded\|Degraded\|Failed\|Deleted\|Unknown; Starting/Stopping/Restarting → conditions/reasons); `WorkloadPublicSummary.execution_posture: WorkloadExecutionPosture` from `public_wire.rs:267` (carries `IsolationPosture`; unsafe-local entries have `IsolationPosture::UnsafeLocal`) |
| Reuse source | main `a1cc0b2d` — copy/adapt: (1) `packages/d2b-client/src/daemon_service.rs` `DaemonClient::lifecycle()` (line 210), `DaemonClient::list_workloads()` (line 148), `DaemonClient::inspect()` (line 179); `DaemonMethod::Apply/Start/Stop/Restart/ListWorkloads` variants (lines 31-46) — adapt: replace `WorkloadLifecycleRequest`/`WorkloadName` with `Guest/<name>` ResourceRef; replace `TargetInput::Workload`-scoped calls with zone-root resource API calls; (2) `packages/d2b-contracts/src/generated_v2_services/daemon.rs` `WorkloadLifecycleProjection`, `DeploymentProjection`, `RuntimeProjection` — adapt field mapping to Guest resource spec/status; (3) `packages/d2b/src/lib.rs` `cmd_launch` (`LaunchArgs`) — adapt: the typed ComponentSession target resolution pattern applies but realm/workload-model types (`RealmPath`, `WorkloadName`) are excluded; behavior selected: idempotent apply with dry-run/apply precondition |
| Reuse action | adapt |
| Destination | `packages/d2b/src/guest.rs` (`d2b guest start/stop/restart/list/status`); unsafe-local workloads go to `packages/d2b/src/host.rs` (`d2b host list/status/get`), NOT guest.rs |
| Detailed design | Route Guest lifecycle (WorkloadProviderKind: LocalVm/QemuMedia/ProviderManaged) through `d2b.resource.v3` Get/UpdateSpec/Watch; map dry-run/apply to resource API precondition; `--no-wait-ready` exits on accepted; with-wait uses `d2b status --watch` loop. WorkloadProviderKind::UnsafeLocal entries MUST route to `d2b host` commands only; any code path that would return an unsafe-local entry from `d2b guest list` is a correctness violation. v2 commands (`d2b up/down/restart/list/status`, `d2b vm start/stop/restart/list/status`) are deleted at 3.0; `d2b migrate-check` explains replacements. |
| Integration | ZoneContext → resource API client → Guest resource; status watch uses Watch stream |
| Data migration | None |
| Validation | Dry-run/apply/wait/no-wait-ready tests; zone-unavailable degraded path; JSON output schema tests; confirm v2 command paths are absent (compilation failure if any cmd_vm_start/stop alias re-introduced) |
| Removal proof | Old `cmd_vm_start/stop/restart` seqpacket paths removed after Guest resource API paths are live with full test coverage |

### ADR046-cli-003

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-cli-003` |
| Dependency/owner | ADR046-cli-001, ADR046-api-001; CLI crate owner |
| Current source | `packages/d2b/src/exec_client.rs` (entire FSM); `packages/d2b/src/terminal_client.rs`; `packages/d2b/src/lib.rs`: `cmd_vm_exec`, `cmd_vm_exec_management`, `VmExecArgs` |
| Reuse source | main `a1cc0b2d` — copy/adapt: (1) `packages/d2b-client/src/daemon_service.rs` `DaemonClient::open_terminal(method, resource_id, operation_id, selection, options, cancellation)` returning `DaemonTerminal` (line 248) — copy-then-adapt: replaces the existing seqpacket Exec call; `DaemonMethod::Exec` (line 40) maps to `d2b exec run`; (2) `packages/d2b-contracts/src/generated_v2_services/terminal.rs` `TerminalOpenRequest`, `TerminalOpenResponse`, `TerminalStreamFrame`, `TerminalSelection`, `TerminalKind` — copy unchanged as the named-stream terminal wire protocol; (3) `packages/d2b-client/src/session.rs` `NamedStream` (`send`, `receive`, `cancel`, `close`, `is_terminal`) — copy unchanged; provides async stdio routing and cancel on disconnect; (4) `packages/d2b-client/src/daemon_service.rs` `GuestClient::inspect_exec()`, `cancel_exec()`, `open_exec_retained_log()` — adapt: rename from `WorkloadName`/`GuestClient` to `EphemeralProcess/<ref>` resource API; (5) `packages/d2b-session/src/streams.rs` `NamedStreamMux` limits (`MAX_ACTIVE_NAMED_STREAMS=128`, `named_stream_queue_bytes=262144`) — copy unchanged; bounds the exec stream pipeline; (6) `packages/d2b-session/src/cancellation.rs` `Cancellation`, `RequestRegistry` — copy unchanged; provides generation-bound per-request cancel; tests to adapt/import: `client.rs:terminal_uses_server_stream_and_validates_bidirectional_lifecycle`, `terminal_rejects_response_generation_and_non_server_stream_ids`, `invalid_terminal_selection_is_rejected_before_open_rpc`, `guest_exec_management_preserves_typed_state_and_cancel_correlation`, `guest_retained_log_open_binds_range_resource_and_selection`, `named_stream_fragments_over_queue_credit_and_has_terminal_actions`, `named_stream_grants_only_consumed_data_and_releases_blocked_sender`; excluded ADR 0045 assumptions: `GuestClient` internal `TargetInput::Workload`-scoped vsock routing (guest-control proxy uses old `WorkloadName`/`RealmPath` — these are excluded; v3 routes through resource API only) |
| Reuse action | copy-then-adapt |
| Destination | `packages/d2b/src/exec.rs` (`d2b exec run/attach/wait/status/list/logs/kill`) |
| Detailed design | Map EphemeralProcess resource lifecycle; `exec run` creates resource and returns ref; `exec attach` opens named stream via adapted `DaemonClient::open_terminal(DaemonMethod::Exec, ...)` → `DaemonTerminal`; retain full `exec_client.rs` FSM and TTY machinery from baseline; retain `--json` envelope fields `source`/`reason`/`guestExitCode`/`signal`/`transportExitCode`; retain reserved exit codes 42/69/70/75/76/77. v2 commands (`d2b vm exec *`) are deleted at 3.0; no dispatch wiring. Excluded ADR 0045: `GuestClient` vsock/guest-control proxy path; `TargetInput::Workload`; old `WorkloadName`-keyed exec management. |
| Integration | ZoneContext → EphemeralProcess Create → named stream attach via `DaemonClient::open_terminal` |
| Data migration | None |
| Validation | Full `exec_client.rs` test suite migrated; adapted tests from main `client.rs:terminal_*` and `guest_exec_*`; TTY/raw-mode/RAII/signal tests; `--json` envelope/disambiguation tests; capacity/transport/auth/protocol exit-code tests; confirm v2 `cmd_vm_exec` path is absent |
| Removal proof | Old `cmd_vm_exec` seqpacket path removed after `d2b exec` paths have equivalent coverage |

### ADR046-cli-004

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-cli-004` |
| Dependency/owner | ADR046-cli-001, `shell-terminal` Provider dossier; CLI crate owner |
| Current source | `packages/d2b/src/lib.rs`: `cmd_shell` (`ShellArgs.vm: String` = `WorkloadId` or `RealmTarget`; routes through `route_vm_target()` → `VmTargetRoute::Local\|Gateway`; gateway `Attach` fails closed via `shell_gateway_attach_failure()` with error class `gateway-shell-attach-unavailable` at lib.rs:1697,1780), `cmd_shell_attach`, `run_shell_fsm`, `ShellOwnerTransport`; wire: `ShellOp`, `ShellOpResponse`, `ShellAttachArgs { vm: String }`, `ShellListEntry`, `ShellSessionState` from `packages/d2b-contracts/src/public_wire.rs:1319,1394,1452,1409`; `exec_client.rs` signal/TTY machinery |
| Reuse source | main `a1cc0b2d` — copy/adapt: (1) `packages/d2b-client/src/daemon_service.rs` `DaemonClient::open_terminal(DaemonMethod::Shell, ...)` returning `DaemonTerminal` — copy-then-adapt; same `TerminalOpenRequest`/`TerminalOpenResponse`/`DaemonTerminal` flow as cli-003, applied to shell open/attach; (2) `packages/d2b-contracts/src/generated_v2_services/shell.rs` and `shell_ttrpc.rs` `ShellService` methods: `ShellCreate`, `ShellAttach`, `ShellDetach`, `ShellList`, `ShellInspect`, `ShellKill`, `ShellCancel` (service definition); `ShellCreateRequest`/`ShellAttachRequest`/`ShellListResponse`/`ShellInspectResponse` — copy-then-adapt: these are the target ShellSession resource CRUD wire types; adapt field names from `workload_id`/`shell_name` to `Guest/<name>` ResourceRef; (3) `packages/d2b-client/src/session.rs` `NamedStream` — copy unchanged; used for shell I/O stream; (4) `packages/d2b-session/src/cancellation.rs` `Cancellation`, `RequestRegistry` — copy unchanged; (5) `packages/d2b-session/src/deadline.rs` `DeadlineBudget` — copy unchanged; shell sessions use per-operation deadline tracking; tests to adapt/import: `client.rs:shell_management_uses_typed_selection_result_and_terminal_outcome`, `named_stream_fragments_over_queue_credit_and_has_terminal_actions`, `named_stream_grants_only_consumed_data_and_releases_blocked_sender`, `concurrent_named_streams_route_events_without_cross_consumption`; excluded ADR 0045 assumptions: `VmTargetRoute::Gateway` shell routing and `realm_router` relay path; old `ShellOp`/`ShellOpResponse` seqpacket wire; unsafe-local helper shell protocol v2 |
| Reuse action | copy-then-adapt |
| Destination | `packages/d2b/src/shell.rs` (`d2b shell open/attach/list/detach/kill/status`) |
| Detailed design | Route ShellSession resource lifecycle through resource API using adapted `ShellService` generated types; `shell open` → `ShellCreate` → `DaemonClient::open_terminal(Shell)` → `DaemonTerminal`; retain FSM/TTY/signal/RAII behavior from `run_shell_fsm`; `--name` required for kill; SIGHUP detaches without kill. v2 commands (`d2b shell <target> *`) are deleted at 3.0; no dispatch wiring. Excluded: gateway relay path (`VmTargetRoute::Gateway`); old `ShellOp`/`ShellOpResponse` seqpacket protocol; `TargetInput::Workload`-keyed realm routing. |
| Integration | ZoneContext → ShellSession Create via `DaemonClient::open_terminal` → `NamedStream` I/O |
| Data migration | None |
| Validation | Shell list/detach/kill/attach unit tests (adapted from existing); adapted `client.rs:shell_management_*` and `named_stream_*` tests; TTY RAII/signal tests; confirm v2 `cmd_shell` path is absent |
| Removal proof | Old `cmd_shell` seqpacket path removed after new shell commands have equivalent coverage |

### ADR046-cli-005

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-cli-005` |
| Dependency/owner | ADR046-cli-001, Provider model spec; CLI crate owner |
| Current source | `packages/d2b/src/lib.rs`: `cmd_audio`, `cmd_clipboard_arm`, `cmd_vm_display` |
| Reuse source | main `a1cc0b2d` — copy/adapt: (1) `packages/d2b-provider/src/rpc.rs` `RpcProviderProxy` — adapt: the CLI uses the inverse side (client calling provider via `ConnectedClient::invoke`); `RpcCall`, `RpcPayload`, `RpcResponse`, `RpcOperation` define the typed call shape for dynamic provider commands; (2) `packages/d2b-contracts/src/generated_v2_services/provider_runtime.rs`, `provider_display.rs`, `provider_audio.rs`, `provider_infrastructure.rs` — adapt: the generated service method types show what CLI projection verbs can be mapped to typed service calls; use as shape reference for the first audio/clipboard/display migration; (3) `packages/d2b-provider-toolkit/src/conformance.rs` `check_provider_conformance`, `check_descriptor_conformance` — copy-then-adapt into CLI-side projection conformance validation (bounds: 64 KiB, 32 sub-verbs, 2s deadline, shell-escape, newline strip); (4) `packages/d2b-provider-toolkit/src/server.rs` `GeneratedProviderServiceServer::generated_services()` — server-side only; use as reference for what CLI InspectSchema receives; excluded ADR 0045 assumptions: `ProviderRegistry`/`ProviderAgentAdapter` are server-side and not used in CLI; `RpcProviderProxy` internal `AuthenticatedProviderRpc` pattern is server-side; tests to adapt: `conformance.rs:every_axis_passes_identical_in_process_and_rpc_conformance`, `generated_server_dispatches_closed_methods_over_authenticated_session` |
| Reuse action | adapt |
| Destination | `packages/d2b/src/provider.rs` (`d2b provider list/get/status/inspect`; dynamic projection loading) |
| Detailed design | `d2b provider list/get/status/inspect`; InspectSchema call returns dynamic projection descriptor using `ConnectedClient::invoke` with generated provider service types; projection bounds enforcement (64 KiB, 32 sub-verbs, 2s deadline, shell-escape, newline strip); built-in name collision guard; audio/clipboard/display as first providers to migrate their projections |
| Integration | ZoneContext → Provider resource + InspectSchema via `ConnectedClient::invoke` |
| Data migration | None |
| Validation | Projection size/name/collision/timeout bounds tests; audio/clipboard/display projection conformance tests; completion script safety tests; adapted `conformance.rs` tests |
| Removal proof | Built-in `cmd_audio`/`cmd_clipboard_arm`/`cmd_vm_display` removed only after Provider projection paths pass equivalence tests |

### ADR046-cli-006

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-cli-006` |
| Dependency/owner | ADR046-cli-001; CLI crate owner |
| Current source | None (no completion exists in v3 baseline) |
| Reuse source | Optional: clap_complete crate (version to be pinned); no main-branch source |
| Reuse action | copy-unchanged (clap_complete) |
| Destination | `packages/d2b/src/complete.rs` (`d2b complete bash/zsh/fish`) |
| Detailed design | `d2b complete <shell>` emits completion script; uses clap `CommandFactory::command()` plus dynamic projection fetch (2s per-Provider, 10s total); result bounded at 256 KiB; shell-escaped; newlines stripped |
| Integration | Standalone command; no Zone API required for static completion; Zone API used for dynamic Provider projection |
| Data migration | None |
| Validation | Completion script tests (bash/zsh/fish syntax valid); projection injection safety tests; deadline/partial-Provider tests |
| Removal proof | Not applicable |

### ADR046-cli-007

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-cli-007` |
| Dependency/owner | ADR046-cli-001; CLI/activation Provider owner |
| Current source | `packages/d2b/src/lib.rs`: `cmd_build`, `cmd_generations`, `cmd_switch`, `cmd_boot`, `cmd_test`, `cmd_rollback`, `cmd_gc`, `cmd_migrate`, `cmd_keys_*`, `cmd_keys_trust`, `cmd_keys_rotate_known_host`, `ConfigCommand` variants |
| Reuse source | main `a1cc0b2d` — copy/adapt: (1) `packages/d2b-client/src/daemon_service.rs` `DaemonClient::lifecycle()` with `DaemonMethod::Apply`, `DaemonMethod::Start`, `DaemonMethod::Stop`, `DaemonMethod::Restart` — copy-then-adapt: the apply/lifecycle dispatch pattern maps cleanly to `d2b activation switch/boot/test/rollback`; retain idempotency token and dry-run/apply precondition from `DaemonMethod::Apply`; (2) `packages/d2b-contracts/src/generated_v2_services/activation.rs` activation service method types — adapt: map `ActivationBuildRequest`, `ActivationSwitchRequest`, `ActivationGenerationsRequest` to typed CLI args; excluded ADR 0045 assumptions: `DaemonMethod::ListRealms` / `DaemonMethod::ListWorkloads` are not used; old `WorkloadName`-keyed dispatch is excluded |
| Reuse action | adapt |
| Destination | `packages/d2b/src/activation.rs` (`d2b activation build/generations/switch/boot/test/rollback/gc/migrate/keys/trust/rotate-known-host/config`) |
| Detailed design | Route through `activation-nixos` Provider service via `ConnectedClient::invoke` using adapted `DaemonMethod::Apply`/lifecycle dispatch pattern; retain dry-run/apply; retain guest-control transport for config sync (no SSH). v2 top-level activation commands (`d2b build/switch/boot/test/rollback/gc/migrate/keys/trust/rotate-known-host/config`) are deleted at 3.0; no dispatch wiring. |
| Integration | ZoneContext → activation-nixos Provider service → resource API via `ConnectedClient::invoke` |
| Data migration | None |
| Validation | All existing switch/boot/test/rollback/keys tests adapted; config sync/diff/approve/reject tests; confirm v2 top-level activation paths are absent; adapted `client.rs:daemon_typed_list_preserves_projection_and_truncation` apply pattern |
| Removal proof | Old top-level activation verbs removed only after `d2b activation *` paths have equivalent coverage |

### ADR046-cli-008

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-cli-008` |
| Dependency/owner | ADR046-cli-001; CLI crate owner |
| Current source | `packages/d2b/src/lib.rs`: `cmd_host_check`, `cmd_host_prepare`, `cmd_host_destroy`, `cmd_host_doctor`, `cmd_host_install`, `cmd_host_reconcile`, `cmd_host_validate`; `host_validate.rs` |
| Reuse source | main `a1cc0b2d` — copy/adapt: (1) `packages/d2b-daemon-access/src/component_session.rs` `LocalUnixDaemonAccess::connect_component_session()` connect chain — adapt: the zone-local host commands use the same connect chain as cli-001; `d2b host prepare` and `d2b host doctor` both require a live `ZoneContext`; (2) `packages/d2b-client/src/client.rs` `ConnectedClient::invoke()` with `CallOptions` and `CancellationToken` — copy unchanged; used for Host resource Get/UpdateSpec/Status calls and broker-op dispatch; (3) `packages/d2b-contracts/src/generated_v2_services/broker.rs` broker operation request/response types — adapt: `BrokerHostPrepareRequest`, `BrokerHostDestroyRequest`, `BrokerHostDoctorRequest` (or equivalent) types define the CLI argument shape; retain broker-mediated ownership-marker semantics from baseline; excluded ADR 0045 assumptions: `TargetInput::Workload`-scoped broker routing is excluded; broker operation routing uses zone-root LocalRoot pattern only |
| Reuse action | adapt |
| Destination | `packages/d2b/src/host.rs` (all `d2b host` subcommands) |
| Detailed design | Route `host prepare/destroy` through Zone resource API Host reconcile operation via `ConnectedClient::invoke`; retain broker-mediated mutation and ownership-marker semantics; `host doctor` prefers Zone resource API status, falls back to local state files; `host check` retains exit-code 3; `host validate` retains wave/evidence-dir/scripts-dir/signature |
| Integration | ZoneContext → Host resource; broker op path retained for emergency/shutdown-hook |
| Data migration | None |
| Validation | All existing host-check/prepare/destroy/doctor/install/reconcile/validate tests; exit-code 3 regression; doctor Zone-fallback/local-state-fallback tests |
| Removal proof | Raw broker-socket paths removed only after Host resource API routes have equivalent coverage |

### ADR046-cli-009

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-cli-009` |
| Dependency/owner | ADR046-cli-001; CLI crate owner |
| Current source | `packages/d2b/src/lib.rs`: `cmd_realm_list` (reads static `realm-entrypoints.json` via `realm_policy_rows_raw()`), `cmd_realm_inspect`, `cmd_realm_enter` (→ `realm_gateway_exec_args` → `cmd_vm_exec` with `-it bash -l`), `cmd_realm_run` (→ `cmd_vm_exec` with caller argv); wire output types: `RealmListOutputV1 { realms: Vec<RealmPolicyOutputV1> }`, `RealmInspectOutputV1 { realm: RealmPolicyOutputV1 }` from `packages/d2b-contracts/src/cli_output.rs:285,292,345`; `RealmPolicyOutputV1` fields: `realm` (= `RealmId`), `mode`, `gateway_vm`, `gateway_target`, `gateway_state`, `cross_realm_policy`, `credential_boundary`; `target_routing.rs`: `Route::Local { vm }`, `Route::Gateway { gateway, target }`, `resolve_access_route()`, `VmTargetRoute`; `d2b-realm-router::RealmEntrypointTable` |
| Reuse source | main `a1cc0b2d` — reference only (no copy): `packages/d2b-realm-router/src/service_v2.rs` `RealmServiceServer`, `RealmServiceProcess`, `RealmMethod::Inspect`, `RealmMethod::ResolveRoute` — server-side multi-realm routing; this is the ADR 0045 multi-Zone topology and is **excluded** from v3 CLI as a direct reuse source; `packages/d2b-realm-router/src/remote_node.rs` `RemoteNodeRegistration`, `RemoteNodeEntry` — constellation remote routing; also excluded; note: `packages/d2b-client/src/daemon_service.rs` `DaemonClient::list_workloads()` and `DaemonMethod::ListRealms` are the closest live list-call patterns, but their zone/workload scoping uses `RealmPath`/`RealmId` types that are ADR 0045-specific; adapt `ConnectedClient::invoke()` with a v3 Zone List request type instead; no main symbols are copied unchanged for cli-009; the zone resource API type design is an ADR-only deliverable pending Zone resource spec |
| Reuse action | adapt |
| Destination | `packages/d2b/src/zone.rs` (`d2b zone get/list/status`) |
| Detailed design | `d2b zone get [<name>]` fetches Zone self resource via `ConnectedClient::invoke`; `d2b zone list` lists ZoneLink resources. v2 commands (`d2b realm list/inspect/enter/run`) are deleted at 3.0; no dispatch wiring. Excluded ADR 0045: `RealmServiceServer`/`RealmServiceProcess` multi-realm service; `RemoteNodeRegistration` constellation routing; `TargetInput::Realm`; `RealmMethod::ResolveRoute`/`AuthorizeShortcut`/`RevokeShortcut`. |
| Integration | ZoneContext → Zone resource Get/List via `ConnectedClient::invoke` |
| Data migration | None |
| Validation | Zone get/list tests; confirm v2 `cmd_realm_*` paths are absent |
| Removal proof | `cmd_realm_*` and `target_routing.rs` removed only after zone routes pass equivalence tests |

### ADR046-cli-010

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-cli-010` |
| Dependency/owner | ADR046-cli-001; CLI crate owner |
| Current source | None |
| Reuse source | main `a1cc0b2d` — copy/adapt: (1) `packages/d2b-client/src/client.rs` `ConnectedClient::invoke()`, `ConnectedClient::invoke_with_attachments()`, `ConnectedClient::open_server_stream()` — copy unchanged; these are the three primitives for resource Get/List/Watch respectively; (2) `packages/d2b-client/src/session.rs` `NamedStream` (`send`, `receive`, `cancel`, `close`, `is_terminal`) — copy unchanged; Watch stream output arrives over a named stream; (3) `packages/d2b-session/src/deadline.rs` `DeadlineBudget` — copy unchanged; `--deadline` flag maps to `DeadlineBudget::admit_metadata` wall deadline; `MAX_REQUEST_LIFETIME_MS=900000` caps all Watch/List deadlines; (4) `packages/d2b-client/src/client.rs` `CancellationToken::cancel()` — copy unchanged; `SIGINT`/SIGTERM → `CancellationToken::cancel()` → propagated to `ConnectedClient::invoke` and `NamedStream`; (5) `packages/d2b-client/src/client.rs` `MetadataInput`, `RetryPolicy`, `CallOptions` — copy unchanged; `--idempotency-token` maps to `MetadataInput`; `RetryPolicy::mutating_once()` is the default for Create/UpdateSpec/Delete; tests to adapt/import: `client.rs:metadata_retries_and_cancellation_use_canonical_driver`, `mutating_retries_require_stable_idempotency`, `concurrent_named_streams_route_events_without_cross_consumption`, `named_stream_grants_only_consumed_data_and_releases_blocked_sender`; excluded ADR 0045 assumptions: `TargetInput::Workload/Realm/Provider` routing variants; `GuestClient` cross-realm proxy routing; old `DeploymentProjection`/`RuntimeProjection` ADR 0045-specific field types |
| Reuse action | copy-then-adapt |
| Destination | `packages/d2b/src/resource.rs` (standard `d2b get/list/watch/create/update-spec/delete/status` top-level verbs) |
| Detailed design | Generic typed dispatch to resource API Get/List/Watch/Create/UpdateSpec/Delete using `ConnectedClient::invoke` (Get/List/Create/UpdateSpec/Delete) and `ConnectedClient::open_server_stream` + `NamedStream` (Watch); ResourceRef argument parsing and validation; page token pagination; `--phase`/`--label-selector` filters; `--deadline` bounded by `MAX_REQUEST_LIFETIME_MS=900s` via `DeadlineBudget`; Watch output streams resource events as JSON lines; JSON schema version field; `CancellationToken` wired to process signal handlers. Excluded: `GuestClient` vsock exec/shell routing; `TargetInput` realm/workload/provider variants. |
| Integration | ZoneContext → `ConnectedClient` → resource API |
| Data migration | None |
| Validation | Get/list/watch/create/update-spec/delete tests per ResourceType; pagination/filter/watch-deadline tests; error-class/exit-code tests; adapted `client.rs:metadata_retries_*` and `mutating_retries_*` and `concurrent_named_streams_*` tests |
| Removal proof | Not applicable (new surface) |

### ADR046-cli-011

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-cli-011` |
| Dependency/owner | ADR046-identities-002, ADR046-cli-001, ADR046-cli-002, ADR046-cli-007; Nix module owner + Zone runtime owner |
| Current source | Nix emitters: `nixos-modules/options-realms-workloads.nix` (current `d2b.envs.<e>.vms.<v>.*`), `nixos-modules/options-realms.nix` (`d2b.realms.*`), `nixos-modules/unsafe-local-workloads-json.nix` (unsafe-local source), `nixos-modules/bundle-artifacts.nix`, `nixos-modules/manifest.nix`, `nixos-modules/assertions.nix`; JSON output: `/etc/d2b/processes.json` (old bundle), `/etc/d2b/realm-entrypoints.json` (static realm index); Zone runtime apply path: `packages/d2bd/src/` (activation apply handler — pre-ADR 0046 path through `cmd_host_prepare`/broker; no live resource bundle apply); cleanup: no current resource-deletion-on-bundle-apply path at baseline |
| Reuse source | None (new implementation; no main `a1cc0b2d` reuse — this is the Nix/Zone side, not the CLI client side) |
| Reuse action | replace |
| Destination | Nix: `nixos-modules/options-zones.nix` (unified `d2b.zones.<zone>.resources` attrset; per-type `spec` sub-options generated from ResourceTypeSchema/Provider schema), `nixos-modules/bundle-emit.nix` (canonical JSON emit + SHA256 pin), `nixos-modules/assertions.nix` (updated); core controller: `packages/d2b-core-controller/src/configuration.rs`, `packages/d2b-core-controller/src/cleanup.rs`; Contracts: `packages/d2b-contracts/src/zone_bundle.rs` (new) |
| Detailed design | **Nix shape:** `d2b.zones.<zone>.resources` is `attrsOf (submodule { type; optional metadata { ownerRef; labels; annotations }; spec })`. `spec` sub-options per `type` are generated from ResourceTypeSchema and signed Provider schemas; field names remain identical. `metadata.name`/`metadata.zone`/`apiVersion` are derived; status and all core metadata are rejected in input. Vendor-qualified types are admitted only when their schema is installed. **Nix emit:** `bundle-emit.nix` emits `/etc/d2b/zones/<zone>/resource-bundle.json` plus its integrity pin with canonical resource ordering and schema digests. **Core-controller apply:** `configuration.rs` verifies bundle/catalog integrity, applies Create/Update/no-op intents with bounded async concurrency, refreshes `configurationGeneration` for unchanged configuration-managed resources without waking their controller, handles controller/API name collisions per-item without seizing them, and asynchronously deletes only persisted `managedBy=configuration` resources absent from the new configured set. `cleanup.rs` consumes `Deleted` revision watches and maintains `PendingCleanup`; it never force-removes finalizers. **Prior generation retention:** `d2b.zones.<zone>.retainedGenerations`, default 3 and range 1–16, is a compiler setting outside `Zone.spec`; no TTL. Rollback reapplies a retained bundle as a new higher generation. |
| Integration | Nix build → per-Zone `resource-bundle.json` + global private artifact catalog → `d2b activation switch` → `d2b-core-controller` configuration service → resource API Create/Update/Delete → owner controllers → finalizer cascade → cleanup watcher → Zone status update |
| Data migration | Full reset from current manifest/processes/realm-entrypoints JSON format; prior Nix-generated artifacts (`/etc/d2b/processes.json`, `/etc/d2b/realm-entrypoints.json`) deleted after Zone resource bundle activates |
| Validation | Runtime integration: all CLI-visible cleanup/status/rollback/gc/audit tests (§CLI-visible tests for activation and cleanup), including no force-finalizer path; Nix unit and build tests owned by ADR-046-nix-configuration spec |
| Removal proof | Old `nixos-modules/manifest.nix`, `nixos-modules/bundle-artifacts.nix` emitters removed only after `bundle-emit.nix` produces equivalent-or-superseding output and all downstream consumers of the old bundle format are migrated |
