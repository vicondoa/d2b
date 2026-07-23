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

### Bootstrap state-realization exception

Mandatory per-component state Volumes (see "Static Provider deployment and
component state Volumes" below) create one closed bootstrap cycle: creating a
Volume normally requires a `Provider/volume-local` controller instance to be
running, but that instance's own controller Process cannot launch until its
own state Volume already exists, and (where they exist on that target)
system-core/system-minijail need their (empty) state Volumes before any
Volume-type controller exists at all on that target.

This exception is scoped **per execution target** — each Host, Guest, or
user-domain local-storage owner that runs its own `Provider/volume-local`
controller instance has its own independent, closed, non-resource **local**
bootstrap storage mechanism, and that mechanism is the sole, narrow break in
the cycle for that target alone:

- it may provision and validate only the empty (empty-`stateSchema`) state
  Volumes for the first `Provider/volume-local` controller instance running
  on that target, and, only where system-core and/or system-minijail are
  themselves fixed bootstrap components on that same target, their (empty)
  state Volumes too — never Volumes for any other component, and never on a
  target where system-core/system-minijail are not present;
- it never crosses an execution-target boundary: a Guest's local bootstrap
  storage mechanism provisions the Guest-local `volume-local` instance's
  bootstrap Volume using only Guest-local primitives, and never receives,
  forwards, or otherwise leaks a parent-Host dirfd, path, or other Host-local
  resource handle across the Host/Guest boundary to do so. This is what lets
  a Guest bootstrap its own primitive controllers (for example, a
  Guest-local `volume-local` serving Guest-local Volumes) without any
  parent-Host state or resource access.

This is not a third Process-bootstrap Provider; each target's local bootstrap
mechanism is part of the same fixed, integrity-pinned bootstrap boundary that
already launches that target's system-core/system-minijail (where present),
and it never handles any other component's state Volume or any other
ResourceType. The Volumes it provisions are real Volume resources/identities
from their first write — ordinary resource rows with normal
generation/status, never placeholders — and as soon as that target's
`volume-local` controller Process starts, it immediately adopts and
reconciles all of them (its own, and system-core's/system-minijail's where
present on that same target) under its normal `volume-local-controller`
reconcile loop, exactly as it would adopt any pre-existing Volume row after a
restart. No other Provider or component on any target ever receives this
exception; every other component's state Volume, on every target, is created
only through the normal Core ProviderDeployment → `Provider/volume-local`
path described below.

After bootstrap:

- every non-system-core controller is Process;
- every service is Process;
- every worker is Process or EphemeralProcess;
- every other Process Provider/controller is Process;
- every process has one executionRef/domain/user placement and selected Process
  Provider.

## Static Provider deployment and component state Volumes

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

Every Provider component — controller, service, or worker, including a
stateless one — has its own private state Volume, created by Core
ProviderDeployment as part of the Provider's **ProviderStateSet**
(`ADR-046-provider-state`: the logical, query-time grouping of every Volume
resource in the Zone whose `metadata.ownerRef` resolves to `Provider/<name>`;
the set itself is never a ResourceType or a stored row — there is no separate
compartment object distinct from an ordinary Volume). A stateless component
still receives its own Volume, declared with an empty `stateSchema`; there is
no component that goes without one. Each state Volume uses the canonical full
Volume schema (see `ADR-046-resources-volume`), extended with the
`stateSchema`/`persistenceClass`/`sensitivityClass` fields defined in
`ADR-046-provider-state`, and its layout is owned by a dedicated `User/<name>`
principal drawn from a bounded, Nix-preprovisioned pool sized to the
Provider descriptor's fixed controller/service/worker/namespace counts —
never an ad hoc principal created at runtime. Core ProviderDeployment creates
every declared state Volume from the manifest's signed state declarations
before creating and launching the corresponding component Process, so the
component's `mounts` can reference an already-Ready Volume at launch. A
component mounts only its own declared view of its own state Volume (a
`mounts` entry naming that view's local dirfd); there is no cross-component or
cross-Provider sharing of another component's state Volume. Resource rows and
the core operation ledger remain the sole authority for resource references,
generation counters, backoff state, and session state — a component's state
Volume payload never duplicates any of that; it holds only the component's
private application-level working state. Creating a Volume normally requires
a `Provider/volume-local` controller instance to already be running on that
same execution target; the "Bootstrap state-realization exception" above is
the sole, narrowly scoped, per-execution-target exception to that ordering,
covering only that target's first `Provider/volume-local` controller
instance and, where they exist on that same target, `Provider/system-core`
and `Provider/system-minijail`.

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
2. resolves only trusted package/template/resource outputs;
3. asks the local broker/systemd/user/Host/Guest effect owner to launch;
4. returns provider-specific stable process identity and mandatory pidfd
   evidence;
5. observes/adopts/signals/stops only that exact identity;
6. reports bounded effects/status to the Process controller.

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
| Reuse action | extract and adapt |
| Destination | `packages/d2b-process/src/`, `packages/d2b-provider-supervisor/src/` |
| Detailed design | Common spec/status/tickets/pidfd/adoption/naming/cgroup/fast path |
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
| Reuse action | extract/adapt |
| Destination | `packages/d2b-provider-system-systemd/`, `packages/d2b-provider-system-minijail/` |
| Detailed design | Two Process/EphemeralProcess implementations, pidfd/wait ownership, system/user domains |
| Integration | Zone-installed Providers/controller instances per Host/Guest |
| Data migration | None |
| Validation | Identical schema/status conformance plus provider-specific adoption |
| Removal proof | Old helpers retained until host/user/guest parity |
