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

After bootstrap:

- every non-system-core controller is Process;
- every service is Process;
- every worker is Process or EphemeralProcess;
- every other Process Provider/controller is Process;
- every process has one executionRef/domain/user placement and selected Process
  Provider.

## ProviderSupervisor

ProviderSupervisor is a fixed local effect adapter, not a Provider/resource
controller. It receives a Process-controller-authenticated LaunchTicket bound
to:

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

## Process common spec

Common fields:

- providerRef;
- executionRef;
- domain and conditional userRef;
- processClass controller/service/worker;
- owning Provider/component/template;
- sealed config and Credential refs;
- mounts by Volume/view/path/access;
- sandbox;
- budget;
- Network/Device refs and usage;
- endpoints/ports;
- bus/telemetry;
- readiness/deadlines;
- desired lifecycle.

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
- umask/rlimits/oom policy.

The selected Process Provider compiles these into its implementation:

- system-minijail → broker-validated minijail/clone3 plan;
- system-systemd → transient unit/scope hardening and any approved sandbox
  wrapper;
- future Provider → same semantic conformance.

Raw policy fragments are package/core-reviewed artifacts, never Provider config
strings.

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
