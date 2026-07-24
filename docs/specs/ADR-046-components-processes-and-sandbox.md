# ADR 0046 components, processes, and sandbox

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-components-processes-and-sandbox` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | Process Providers, ProviderSupervisor, broker/Host/Guest supervisors |
| Depends on | `ADR-046-provider-model-and-packaging`, `ADR-046-primitive-resource-composition`, `ADR-046-componentsession-and-bus` |
| Supersedes | Current ProcessRole/VmProcessDag as the public process model |

## Bootstrap boundary

Non-resource bootstrap is closed:

- Zone runtime with embedded store/resource API/bus endpoint;
- Zone privileged broker where required;
- one fixed core-controller/Provider-system-core process;
- minimum Host/Guest supervisor/effect adapter;
- fixed user supervisor;
- fixed Provider/system-minijail controller process;
- transport/listener resources necessary to reach the owning Zone.

Bootstrap mechanisms are not Primitive ResourceSpecs. They cannot be selected by
third-party Providers or widened by config.

Provider/system-core and Provider/system-minijail are the only Provider
controllers without Process resources. system-minijail is fixed because no
Process controller exists to launch the first Process controller. It is
integrity-pinned, uses the compiled bootstrap authorization, and launches every
later Process controller, including Provider/system-systemd.

### No bootstrap state Volume

Provider state Volumes are optional and declared only under the storage-need
test (see "Static Provider deployment and optional component state Volumes"
below and `ADR-046-provider-state`). The fixed bootstrap components — the first
`Provider/volume-local` controller instance on each execution target, and
(where present) `Provider/system-core` and `Provider/system-minijail` — keep
their bounded non-secret operational state in the owning resource's `status`
subresource and the core Operation ledger, and declare **no** state Volume.
Because no component requires a state Volume before a `volume-local` instance
is Ready, there is no bootstrap state-Volume cycle, no per-execution-target
local bootstrap storage mechanism, and no bootstrap-storage exception (D086,
superseded by D087). There is no hidden bootstrap store.

A fixed bootstrap component reaches Ready by adopting running processes and
re-deriving its observed state from `status`, the core Operation ledger, and
independent external observation (cgroup-leaf scanning, fresh pidfds, marker
reverification against external reality). If a future bootstrap component ever
needs secret or large private recovery state that cannot enter status, it must
be introduced through a new reviewed design that declares an ordinary optional
state Volume; it does not reintroduce a bootstrap-storage exception.

After bootstrap:

- every non-system-core controller is Process;
- every service is Process;
- every worker is Process or EphemeralProcess;
- every other Process Provider/controller is Process;
- every process has one executionRef/domain/user placement and selected Process
  Provider.

## Static Provider deployment and optional component state Volumes

Core ProviderDeployment reads the signed manifest/catalog entry a Provider's
`artifactId` resolves to and creates the Provider's entire static
controller/service Process graph from it. A Provider controller never
bootstraps its own Process (the two fixed exceptions are system-core, which
has no Process, and system-minijail, which launches every later Process
controller including system-systemd's). The static graph is created once, at
Provider install/reconcile time, from the manifest's declared component
descriptors — never invented, widened, or self-launched by the Provider's own
code at runtime.

Controllers may create authorized *dynamic* children beyond that static
graph: additional Process/EphemeralProcess resources, and other primitive or
vendor resources their descriptor authorizes (for example, volume-virtiofs
creating a virtiofsd Process per attachment). A service that needs worker
help never spawns a worker itself; it sends a typed internal request to its
owning controller, which creates the worker Process through the normal
ProviderDeployment/EffectPort path.

Provider state Volumes are **optional**. Bounded non-secret operational state
belongs in the owning resource's `status` subresource and the core Operation
ledger by default (D087). A Provider component declares a state Volume only
when a specific payload passes the storage-need test: it is a secret or
sensitive private datum, is large/binary/file content, is private data unsafe
for status readers, or is bounded but revision-unsuitable with a demonstrated
recovery need (`ADR-046-provider-state`). A stateless component declares no
state Volume, receives none, and contributes none to the Provider's optional
**ProviderStateSet** (`ADR-046-provider-state`: the logical, query-time
grouping of the *declared* Volume resources in the Zone whose
`metadata.ownerRef` resolves to `Provider/<name>`; the set is never a
ResourceType or a stored row, and it is empty for a Provider that declares no
state Volume). There is no empty identity-only Volume and no separate
compartment object distinct from an ordinary Volume.

Each declared state Volume uses the canonical full Volume schema (see
`ADR-046-resources-volume`), extended with the
`stateSchema`/`persistenceClass`/`sensitivityClass` fields defined in
`ADR-046-provider-state`, and its layout is owned by a dedicated `User/<name>`
principal drawn from a bounded, Nix-preprovisioned pool sized to the
Provider descriptor's fixed controller/service/worker/namespace counts —
never an ad hoc principal created at runtime. Core ProviderDeployment creates
every *declared* state Volume from the manifest's signed state declarations
before creating and launching the corresponding component Process, so the
component's `mounts` can reference an already-Ready Volume at launch. A
component mounts only its own declared view of its own state Volume (a
`mounts` entry naming that view's local dirfd); there is no cross-component or
cross-Provider sharing of another component's state Volume. Resource rows,
resource `status`, and the core Operation ledger remain the sole authority for
resource references, generation counters, backoff/idempotency state, and
session state — a component's state Volume payload never duplicates any of
that; it holds only the component's private secret/large/revision-unsuitable
working payload. Creating a declared state Volume normally requires a
`Provider/volume-local` controller instance to already be running on that same
execution target; because the fixed bootstrap components declare no state
Volume, no component needs a Volume before a `volume-local` instance is Ready,
so there is no bootstrap ordering exception (see "No bootstrap state Volume"
above).

A worker Process has no ResourceClient, no d2b-bus/dependency-portal access,
no Credential access, no CLI, no broker access, and no authority to spawn
further children. Every resource reference, FD, and configuration value a
worker needs is inherited through its LaunchTicket at launch time. The one
narrow exception: a worker may be declared to own its exact workload child
process when that child is the worker's manifest-fixed data-plane purpose
(for example, a persistent-shell supervisor worker owning its shell PTY
child) — and only then, under an explicit descriptor policy naming that exact
child relationship. This exception never grants a worker broker, bus, or
arbitrary child-spawn authority beyond that one fixed relationship.

## ProviderSupervisor and EffectPort

No Provider process — including a primitive Process, Volume, Network, or
Device Provider — imports the broker crate, receives a broker socket or DTO,
directly opens a host path/device/systemd socket, or performs privileged
mutation itself. A Provider controller/service validates and decides semantics
only, then calls an injected async typed **EffectPort** trait using opaque
resource/intent/template/policy IDs. Core owns a small, fixed set of effect
adapters — one per effect domain — that privately map each EffectPort call to
the actual broker/allocator/systemd/user/guest/kernel operation. The broker
remains the sole privileged executor and independent audit owner of every
mutation; no effect adapter and no Provider bypasses it.

**ProviderSupervisor** is the fixed effect adapter for the Process domain (the
`ProcessLaunchEffectPort`). It is a fixed local effect adapter, not a
Provider/resource controller. It receives a Process-controller-authenticated
LaunchTicket bound to:

- Process/EphemeralProcess ref/UID/revision/generation;
- owner Provider/component/template;
- executionRef/domain/userRef;
- selected Process Provider;
- compiled sandbox/budget/mount/device/network/endpoint configuration digests;
- exact inherited FD table;
- operation/deadline/cancellation;
- expected process identity/readiness.

It:

1. verifies ticket/current resource/controller lease;
2. for every Endpoint, rejects any visibility outside
   `owner|provider|zone`, applies that coarse scope, authenticates the exact
   subject and signed Provider component, evaluates Role/RoleBinding, and
   intersects the canonical `consumerPolicy.allowedSubjects`,
   `allowedProviderComponents`, and `allowedOperations` allowlists; request
   fields cannot select those identities, no visibility alias is accepted, and
   any mismatch is `endpoint-resolve-denied`;
3. resolves only trusted package/template/resource outputs;
4. asks the local broker/systemd/user/Host/Guest effect owner to launch;
5. returns provider-specific stable process identity and mandatory pidfd
   evidence;
6. observes/adopts/signals/stops only that exact identity;
7. reports bounded effects/status to the Process controller.

It never interprets Provider root settings, chooses dependencies, reads sibling
state, or registers services/commands.

The same pattern generalizes to the other primitive domains: volume-local and
volume-virtiofs call a `VolumeLayoutEffectPort`/`VolumeSourceEffectPort` with
the Volume resource UID, layout-entry index, and resolved semantic
owner/mode/ACL settings — never a raw host path or broker DTO — and the
volume-domain effect adapter performs the actual broker layout operation
(`ADR-046-resources-volume`). Network and Device Providers call the equivalent
`NetworkEffectPort`/`DeviceEffectPort` with opaque resource/intent IDs. Every
such effect adapter is, like ProviderSupervisor, a fixed core-owned component,
never a Provider or resource controller, and never widened by third-party
config.

## Process common spec

Common fields (exact frozen names; see `ADR-046-resources-host-guest-process-user`
for the full ExecutionSpec table):

- `providerRef`;
- `executionRef`;
- `domain` and conditional `userRef`;
- `processClass` controller/service/worker;
- owning Provider/component, plain `template`;
- `configRef` and `credentialRefs`;
- `mounts` by Volume/view/path/access;
- `sandbox`;
- canonical nested `budget`;
- `networkUsage`;
- `deviceUsage`;
- `endpoints`;
- `telemetry`;
- Process/EphemeralProcess lifecycle fields (desired lifecycle, readiness/health,
  restart/backoff, adoption/drain, one-shot deadlines/terminal TTLs).

These are the only common field names. They are never renamed to `network`,
`devices`, a command/binary/argv field, an endpoint kind/path/service field, or
a custom budget/readiness/restart shape.

No caller-controlled executable path, UID/GID, host path, cgroup path,
capability, raw seccomp, minijail argv, systemd property, broker operation,
socket address, credential byte, or environment escapes the signed Provider/
Process/Volume schema.

## Sandbox compilation

Inline Process sandbox declares semantic requirements:

- namespace isolation classes;
- capability classes;
- seccomp classes;
- mount/Volume views;
- Device/Network access;
- read-only root/store posture;
- environment classes;
- LSM labels/profile references where supported;
- no-new-privileges/start-root requirements;
- fd classes;
- umask/rlimits/oom policy;
- a semantic user-namespace `mappingClass` (never a numeric host UID/GID; see
  `ADR-046-resources-host-guest-process-user`).

The Process Provider controller never compiles or applies this plan itself. It
validates the declared SandboxSpec and calls the `ProcessLaunchEffectPort`
(ProviderSupervisor) with the resource UID, the selected Process Provider, and
the sandbox digest. ProviderSupervisor and the broker compile the semantic
request into its implementation:

- system-minijail → ProviderSupervisor requests a broker-validated
  minijail/clone3 plan; the broker compiles and applies it;
- system-systemd → ProviderSupervisor requests transient unit/scope hardening
  and any approved sandbox wrapper from the fixed systemd effect owner;
- future Provider → same semantic conformance through its own fixed effect
  owner.

The Provider process itself never opens a systemd/D-Bus socket, calls the
broker, or issues `clone3`/`pidfd_open` directly. Raw policy fragments are
package/core-reviewed artifacts, never Provider config strings.

## Pidfd and wait/reap

Every Process Provider has a local verified pidfd.

### system-minijail

- broker `clone3(CLONE_PIDFD)` preferred;
- reviewed fallback only where contract permits;
- d2b owns wait/reap;
- process born directly in final cgroup;
- adoption verifies pid/start-time/cgroup/executable/template/generation before
  `pidfd_open`;
- ambiguity → Unknown/quarantine, never broad kill/reuse.

### system-systemd

- non-forking transient unit/scope;
- bind InvocationID, cgroup, MainPID, start-time, Provider/template/generation;
- open verified pidfd;
- systemd owns wait/reap;
- no daemonizing/forking type;
- unit-name alone is not identity;
- adoption revalidates all stable identity before pidfd open.

Pidfd:

- is not persisted;
- is not public status;
- never crosses d2b-bus/Zone/Host/Guest transport;
- is closed/reopened across controller restart after identity verification.

## User domain

User-domain Process uses the same ResourceType:

- executionRef;
- domain=user;
- exact userRef or inherited Host/Guest defaultUserRef;
- the referenced ExecutionPolicy allows user;
- system-systemd uses verified transient user scope via fixed user supervisor;
- system-minijail may support user domain only if descriptor/conformance says so;
- UserPortal needs are inline typed refs/settings;
- same-UID non-isolation is explicit status/security posture.

User supervisor is a fixed local effect adapter, not another Provider.

## Host/Guest locality

A Process controller instance declares compatible Host/Guest Provider
capabilities. The ResourceType does not change for:

- physical Host;
- VM Guest;
- ACA sandbox Guest;
- Azure/full-host Guest;
- remote/nested Guest.

The Host/Guest local supervisor/effect adapter performs launch and reports to
the owning Zone over d2b-bus/ComponentSession. A Guest does not need a Zone
store unless it separately hosts a child Zone.

## Process naming and cgroups

Diagnostic process names:

```text
z-<zone-id>@<bootstrap-process>
s-<execution-id>@<process-name>
u-<execution-id>-<user-id>@<process-name>
```

Opaque short IDs are derived from immutable UIDs, not human labels. Executable/
template/generation remain separate verified identity.

System cgroup shape:

```text
z-<zone-id>/
  controller/
  broker/
  executions/
    e-<execution-id>/
      system/
        providers/
          p-<provider-id>/
            components/
              c-<component-id>/
                process/
                workers/
                  w-<worker-id>/
```

Intermediate nodes are process-free. User processes live in exact user scopes
and are not misrepresented as system cgroup children.

## Process status

Common status plus:

- process implementation/provider;
- process identity digest;
- wait/reap owner;
- executionRef/domain/userRef;
- config/sandbox/resource revision digests;
- readiness/health/restart/backoff;
- last start/exit class;
- optional exitCode in outcome;
- adoption/quarantine condition.

No PID, pidfd, unit name, path, argv, environment, terminal bytes, or raw
Provider diagnostic is public status/audit.

## Fast launch

After durable ready Process commit:

- direct post-commit hint;
- p95 handler start <=5 ms;
- p95 launch-attempt start <=20 ms;
- Process controller launches in independent async task;
- watch loop dispatches the next independent resource concurrently;
- readiness wait does not hold controller-wide queue;
- status transitions are async expected-revision writes.

## Current-role disposition rule

Every v3 ProcessRole/helper/unit is classified as:

- fixed bootstrap;
- Process/EphemeralProcess under exact Provider;
- non-process controller/probe;
- deleted after successor.

Each disposition names:

- current source/emitter/runtime caller;
- ResourceType owner/Provider;
- Process common/Provider-specific fields;
- executionRef/domain/userRef;
- Volume/Network/Device dependencies;
- readiness/restart/adoption/finalizer;
- current tests copied/adapted;
- exact old path removal.

No generic Process conversion may lose role-specific watchdog behavior,
pre-start effects, fine-grained ACL/device policy, or restart semantics.

## Representative baseline mapping

| Current role | Future owner |
| --- | --- |
| CloudHypervisorRunner | Runtime Cloud Hypervisor Provider owns a Process under Host |
| QemuMediaRunner | Runtime QEMU Provider owns Process |
| Virtiofsd | Volume Provider owns Process per attachment |
| Swtpm | TPM Device Provider owns Process plus Volume state |
| GPU/GpuRenderNode/Video | Device GPU Provider owns Processes; Display Provider consumes Device/endpoints |
| Audio | Audio Provider owns Process |
| WaylandProxy | Display/Wayland Provider owns Process |
| VsockRelay | Transport Provider owns Process |
| OtelHostBridge | Observability Provider owns Process |
| Usbip | USB Device Provider owns Process/EphemeralProcess |
| GuestControlHealth/readiness probes | controller observation, not Process |
| HostReconcile/store preflight | controller logic, not Process |

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | `d2b-core/src/processes.rs`, minijail/profile/runtime; d2bd DAG/pidfd/adoption; broker SpawnRunner; unsafe-local helper; guestd/exec runner; Nix system/user/guest units |
| Evidence class | Current launch paths are reachable; generic Process Providers/supervisor are ADR-only |
| Behavior retained | Typed argv/profile, broker privilege, pidfd identity/adoption, direct cgroup placement, user scope, guest locality, readiness/watchdogs, redaction |
| Required delta | Common Process/EphemeralProcess, selected Provider, Host/Guest executionRef/domain placement, generic supervisor/async controller/status |
| Reuse path | Exact current/main source mappings in role/Provider dossiers |
| Replacement/deletion | No ProcessRole/unit/helper removal until successor behavior/test parity |
| Feasibility proof | systemd/minijail shared conformance; Host system/user and Guest execution; fast parallel launch |
| Future owner | Work items below |

## Implementation work items

### ADR046-process-001

| Field | Value |
| --- | --- |
| Dependency/owner | W0/W2; Process contracts/supervisor |
| Current source | `d2b-core/src/{processes,process_builder,minijail_profile}.rs`; `d2bd/src/supervisor/*`; broker `spawn_runner.rs` |
| Reuse source | Useful main ProviderSupervisor/session/process code named by sub-items |
| Reuse action | adapt |
| Destination | `packages/d2b-process/src/`, `packages/d2b-provider-supervisor/src/` |
| Detailed design | Common spec/status/tickets/pidfd/adoption/naming/cgroup/fast path Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Process Provider controllers → supervisor/broker/systemd → async status |
| Data migration | Full reset; no role snapshot import |
| Validation | Shared conformance/fault/latency tests |
| Removal proof | Role/DAG path removed only per role disposition |

### ADR046-process-002

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-process-001; systemd/minijail Provider owners |
| Current source | unsafe-local helper runtime/systemd; guest exec systemd-run; broker SpawnRunner/minijail |
| Reuse source | Main process/session helpers if selected by exact sub-items |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-system-systemd/`, `packages/d2b-provider-system-minijail/` |
| Detailed design | Two Process/EphemeralProcess implementations, pidfd/wait ownership, system/user domains Primary reuse disposition: `adapt`. Preserved source-plan detail: extract/adapt. |
| Integration | Zone-installed Providers/controller instances per Host/Guest |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | Identical schema/status conformance plus provider-specific adoption |
| Removal proof | Old helpers retained until host/user/guest parity |
