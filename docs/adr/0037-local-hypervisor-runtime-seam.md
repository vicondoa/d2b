# ADR 0037: Local hypervisor runtime seam

- Status: Accepted
- Date: 2026-06-20
- Related: ADR 0006 (manifest bundle versioning), ADR 0015 (daemon-only clean
  break), ADR 0025 (host-jailed Wayland filter proxy role), ADR 0032 (d2b
  v2 constellation control plane), ADR 0034 (storage lifecycle, restart
  adoption, and synchronization), ADR 0035 (efficiency and simplification
  roadmap), ADR 0036 (qemu-media runtime)

## Context

D2b now has more than one local hypervisor shape. The NixOS workload path
uses Cloud Hypervisor/crosvm plus supporting services such as virtiofsd,
swtpm, USBIP attach runners, audio, video, and the Wayland proxy. The
qemu-media path uses QEMU, QMP, a host graphics window, and broker-opened media
fds. Both are VM runtimes supervised by `d2bd`, yet their public status,
capability reporting, denial paths, stopped-state handling, and service
bookkeeping have grown independently.

That duplication makes provider work harder. ADR 0032 needs a clean provider
model, ADR 0034 needs consistent lifecycle/storage semantics, and ADR 0035
requires fewer parallel contract shapes. The local hypervisor seam is the
place to converge these behaviors without leaking backend-specific details into
public CLI or daemon contracts.

## Decision

QEMU qemu-media, Cloud Hypervisor/crosvm workloads, and their supporting
sidecars are implementations of one runtime/service abstraction.

A runtime is the primary VM backend for a declared VM. A service is a
supporting process or host resource that the runtime may depend on or expose:
Wayland proxy, virtiofsd, swtpm, USBIP attach, audio, video, QMP media
transactions, and similar per-VM dependencies. The daemon supervises runtimes
and services through shared contracts; backend-specific code lives behind
adapters.

### Shared DTOs and generated contracts

The generated bundle and Rust DTOs should model the common concepts once:

- runtime identity: kind, provider id, VM id, environment, autostart policy,
  manual-only reasons, and provider version/schema information;
- runtime capabilities: positive assertions such as lifecycle verbs, display,
  guest-control, store sync, media boot, runtime media hotplug, USBIP, TPM,
  audio, video, and readiness semantics;
- runtime operations: start, stop, restart, status, adopt, reap, and
  provider-specific operation families surfaced only through typed capability
  gates;
- service specs: role id, required/optional dependency class, start ordering,
  readiness predicate, restart class, storage references, redaction class, and
  ownership authority;
- observed state: desired state, live process state, readiness, stopped-state
  reason, degraded-state reason, unsupported capabilities, and safe
  remediation text.

Provider adapters may keep private internal structs for QMP, Cloud Hypervisor
API, crosvm command lines, USBIP, and media fd transactions. Those structs are
not public daemon protocol and are not duplicated as CLI view models.

### Public daemon status and capability protocol

The public daemon protocol reports a VM's runtime and services through a
provider-neutral status shape. At minimum, status includes:

- `runtimeKind` and `provider`;
- desired lifecycle state and observed process state;
- readiness state and readiness source;
- capability summaries as positive booleans or closed enum values;
- unsupported operation reasons for common CLI verbs;
- service health for required and optional services;
- redacted provider details that are safe for CLI, JSON consumers, and audit
  correlation.

The CLI consumes this protocol rather than re-deriving runtime behavior from
VM names, process node names, or provider-specific fields. Human output may be
runtime-specific, but it must be a presentation adapter over the shared status
record.

### Operation-denial semantics

Unsupported operations fail before side effects. The daemon checks runtime
capabilities before allocating sessions, opening broker requests, touching
storage, or spawning provider workers. Denials use typed usage/policy errors,
not provider-specific internal errors.

Examples:

- guest exec, config sync, and guest-control health are denied for qemu-media
  because the runtime does not expose guest-control;
- USBIP attach is denied for qemu-media because qemu-media runtime hotplug is
  QMP media hotplug, not guest USBIP;
- QMP media attach is denied for Cloud Hypervisor/crosvm workloads unless a
  future provider explicitly advertises that capability;
- autostart is denied or skipped with an explicit manual-only reason when the
  runtime requires operator-present media or a host graphics window.

Dry-run and JSON output report the same denial class and remediation text that
an apply path would use. No command should partially execute and then discover
that the runtime lacks the requested capability.

### Lifecycle, reap, and stopped-state semantics

All local hypervisor runtimes follow the same lifecycle vocabulary:

- `starting`: required services are being prepared or the primary runtime is
  being spawned;
- `running`: the primary runtime is alive and the runtime-specific readiness
  predicate has passed;
- `stopping`: the daemon is intentionally stopping the primary runtime and its
  owned services;
- `stopped`: no live primary runtime remains and all owned service cleanup has
  either completed or is reported as degraded with remediation;
- `degraded`: the daemon cannot prove safe adoption, cleanup, or readiness and
  operator action is required.

The primary runtime exit determines the VM's stopped/running boundary. Required
service failure during start fails the start. Required service death while
running transitions the VM to degraded or triggers the runtime-specific stop
policy. Optional service failure is reported as degraded service health without
inventing a false primary-runtime state.

Reaping is owner-specific. The daemon owns pidfd observation and state
transitions; the broker owns privileged stop/kill operations it is explicitly
asked to perform. Normal daemon restart is a continuation event: adapters first
attempt adoption using declared identity and storage contracts, then quarantine
or degrade ambiguous state before cleanup.

Stopped state must be explicit and durable enough for status to distinguish:

- never started;
- stopped by operator;
- stopped after primary runtime exit;
- stopped after failed start cleanup;
- degraded because cleanup or adoption is unsafe.

### Adapter boundaries

Backend-specific mechanisms remain behind adapters:

- Cloud Hypervisor API calls, crosvm command construction, virtiofsd setup,
  swtpm setup, USBIP runners, and guest-control wiring stay in the
  Cloud Hypervisor/crosvm provider adapter family.
- QEMU command construction, QMP readiness, QMP fd passing, media block/device
  node names, and qemu-media registry compatibility stay in the qemu-media
  adapter family.
- The broker operation catalog remains typed. Shared daemon code asks for a
  runtime/service operation; the adapter chooses the typed broker op and
  redaction plan.

No public CLI, daemon JSON, or generated manifest field should expose raw QMP
object ids, Cloud Hypervisor socket paths, crosvm-only argv details, USB bus
ids, block paths, image paths, or registry paths unless a specific redacted
public contract allows it.

### qemu-media USB cleanup

The public `d2b usb enroll` surface will be removed. Enrollment is a
current qemu-media compatibility mechanism, not the long-term runtime seam.

For qemu-media boot media, physical USB resolution moves to VM start. The
start path resolves the declared boot-drive ref for that start attempt, opens
it in the broker after the same safety preflights, attaches it through QMP
before the paused VM is continued, and reports only redacted state. The boot
resolution is start-time and boot-drive-only; it does not become a durable
public enrollment registry command.

Runtime qemu-media attach and detach remain supported while the VM is running.
Those operations use QMP fd/block/device transactions through the qemu-media
adapter and broker. They are runtime media operations, not USBIP operations and
not a public enrollment workflow.

### Autostart policy

qemu-media autostart remains denied when the VM has a host graphics window, a
physical USB boot disk, or both. These starts require operator-present host
context. The denial is reported through the shared autostart/manual-only
capability protocol so list/status/doctor surfaces can explain why the VM was
not started automatically.

Cloud Hypervisor/crosvm workloads may keep their existing autostart semantics
when their runtime capabilities and services are safe for unattended start.
The shared abstraction does not make every runtime equally autostartable; it
makes the reason explicit.

### Wayland control consumes capabilities

Wayland control (`wl-control`) and window-policy code consume
daemon-reported runtime and service capabilities. It must not infer behavior
from provider names or from whether a VM happens to have a qemu-media or Cloud
Hypervisor process node.

A VM that advertises host-window display capability can receive host window
policy, app-id rewriting, and niri border metadata. A VM that does not
advertise that capability is denied those operations with a typed unsupported
capability result.

## Consequences

The cleanup removes duplicated public contracts and keeps provider-specific
runtime mechanisms in one adapter layer per backend. It also creates a planned
breaking change for qemu-media users: the public enrollment command disappears
in favor of start-time boot media resolution and runtime QMP hotplug.

Existing qemu-media registry behavior remains documented in ADR 0036 as the
current implementation baseline. Migration work must preserve redaction,
broker-owned media fd opening, paused-before-boot QMP sequencing, explicit
manual-only/autostart denials, and unsupported-capability errors for guest
features that qemu-media does not provide.
