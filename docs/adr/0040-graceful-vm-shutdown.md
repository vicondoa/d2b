# ADR 0040: Graceful local VM shutdown before VMM termination

- Status: Accepted
- Date: 2026-06-22
- Related: ADR 0011 (cgroup v2 delegation and pidfd handoff), ADR 0015
  (daemon-only clean break), ADR 0023 (runner-role lifecycle matrix),
  ADR 0034 (storage lifecycle, restart adoption, and synchronization),
  ADR 0036 (qemu-media runtime), ADR 0037 (local hypervisor runtime seam)

## Context

`d2bd` owns VM lifecycle through the daemon-supervised process DAG. Today
`d2b vm stop` drains registered runner pidfds in reverse DAG order by
sending `SIGTERM`, waiting for a bounded timeout, and escalating to `SIGKILL`
when the runner does not exit. That gives the daemon one uniform stop path, but
it treats a VM's primary VMM process the same as a replaceable sidecar.

For local hypervisor runtimes, terminating the VMM from the host side is not the
same as asking the guest OS to shut down. A host-side signal can look like power
loss to stateful guest services and filesystems. The failure mode is especially
visible for NixOS guests backed by Cloud Hypervisor, but the same lifecycle
principle applies to qemu-media: a QEMU guest should receive an ACPI/QMP
shutdown request before d2b terminates the QEMU process.

The existing provider surfaces already expose the control channels needed for a
clean first phase:

- Cloud Hypervisor VMs declare a per-VM API socket in the public manifest
  (`apiSocket`) and Cloud Hypervisor exposes `PUT /api/v1/vm.shutdown` plus
  `GET /api/v1/vm.info` state.
- qemu-media VMs start QEMU with a QMP socket under
  `/run/d2b/vms/<vm>/qmp.sock`. The broker already uses QMP for
  qemu-media boot and hotplug transactions.

The stop path still needs a force override. Operators sometimes need to bypass a
hung guest shutdown path and recover the host. Force-stop must be explicit,
visible, and auditable; it must not become the default desktop control.

## Decision

D2b will add a provider-aware graceful guest shutdown phase before
terminating a local VM's primary VMM runner.

### Default stop

For `d2b vm stop <vm> --apply`, the daemon will first resolve the VM's
runtime provider and ask the guest to shut down through the provider channel:

| Runtime provider | Primary runner | Graceful request | State reconciliation | Empty-VMM cleanup |
| --- | --- | --- | --- | --- |
| Cloud Hypervisor NixOS | `ch-runner` | `PUT /api/v1/vm.shutdown` | `GET /api/v1/vm.info`; `Created` / `Shutdown` mean the guest is no longer running | clean VMM exit when supported, otherwise existing pidfd cleanup |
| QEMU qemu-media | `qemu-media` | broker-mediated QMP `system_powerdown` | broker-mediated QMP `query-status`; `shutdown` means the guest is no longer running | broker-mediated QMP `quit` before pidfd cleanup |

The daemon will poll provider state and pidfd liveness until the configured
guest-shutdown timeout expires. If the VMM exits, d2b deregisters the
pidfd, removes the runner snapshot, and continues normal sidecar cleanup. If
provider state becomes guest-stopped while the VMM pid remains alive, d2b
treats guest shutdown as successful and then performs clean empty-VMM cleanup.
Clean empty-VMM cleanup must wait for the VMM pidfd to report process exit and
release inherited resources such as TAP fds before the stop operation is
considered complete or a restart is allowed. When a provider clean-exit command
is sent, d2b waits on the pidfd for a short bounded cleanup grace before
falling through to the existing SIGTERM/SIGKILL fallback.

Sidecars keep the existing pidfd signal path. The provider graceful phase is for
the primary local VMM runner, not for every process in the per-VM DAG.
The graceful wait blocks reverse-DAG teardown: network resources, virtiofsd,
swtpm, QEMU/CH sidecars, and other owned dependencies are not drained until the
primary VMM runner has either exited cleanly or entered the explicit forced
fallback path.

Failed-start rollback is different from normal `vm stop`. Rollback is cleaning
up a partially constructed process graph, so it skips the guest graceful-wait
phase and uses the force cleanup path for any already-spawned primary VMM
runner. This avoids waiting the full guest shutdown timeout for a boot that did
not become a healthy running guest.

### Configurable timeout

The default graceful shutdown timeout is a Nix/daemon configuration value
rendered into `/etc/d2b/daemon-config.json` and read by `DaemonConfig`. The
option lives under `d2b.daemon.lifecycle.gracefulShutdown.timeoutSeconds`
so future daemon lifecycle settings have a stable namespace. It defaults to a
more generous guest-shutdown timeout than the forced-cleanup signal window
(initially 90 seconds unless implementation evidence selects another bounded
value). Invalid values fail at eval/config parse time rather than silently
clamping.

`d2b.daemon.lifecycle.gracefulShutdown.enable` is the site-wide default
toggle and defaults to `true`. Operators can disable graceful shutdown globally
during migration or for sites where guests intentionally do not respond to ACPI
or provider shutdown. Per-VM enablement still overrides the site default.
This is an explicit safety-fix exception to the usual default-off preference:
issue 100 requires normal `vm stop` / `restart` / `down` to give supported
guests a chance to flush state without requiring a new opt-in.

Each VM may override the default through a generated lifecycle contract:

- `d2b.vms.<vm>.lifecycle.gracefulShutdown.enable = false` declares that
  the VM intentionally bypasses provider graceful shutdown and uses force
  cleanup as its normal stop behavior without producing a spurious degraded
  marker.
- `d2b.vms.<vm>.lifecycle.gracefulShutdown.timeoutSeconds = null | <positive int>`
  overrides the daemon default for that VM when set.

Provider runtimes that support graceful shutdown default this override to
enabled. In Nix option terms, the default is derived from the VM runtime kind
and the global toggle (`config.d2b.daemon.lifecycle.gracefulShutdown.enable`
for Cloud Hypervisor NixOS and qemu-media; `false` for unsupported future
providers unless they opt in). Unsupported future providers must declare
unsupported/disabled graceful shutdown explicitly rather than silently waiting
and timing out.
Timeout options have a bounded maximum (initially 600 seconds unless
implementation evidence selects a smaller limit) so configuration typos cannot
make host shutdown hang for hours.

The existing SIGTERM/SIGKILL timeouts remain the forced-cleanup policy after a
graceful request fails, times out, or is explicitly bypassed.
If a required sidecar needed for guest shutdown (for example virtiofsd) crashes
during the graceful wait, the daemon interrupts the graceful wait and escalates
to forced cleanup; a guest that lost its storage path cannot be expected to
finish a clean shutdown.

Provider connect/read/write operations have their own short bounded timeouts.
The overall shutdown timeout does not permit a single stuck Unix socket
operation to block the daemon executor.
Provider HTTP/QMP reads also have strict maximum payload sizes before parsing or
logging, so a compromised guest/provider cannot exhaust daemon or broker memory
with an oversized response.
Metrics scraping keeps its own independent strict timeout after the CH HTTP
helper is shared with lifecycle code, so a hung provider socket cannot stall the
Prometheus endpoint.

Provider-specific I/O is isolated behind an explicit async daemon trait seam so
the core lifecycle loop consumes typed shutdown outcomes rather than embedding
Cloud Hypervisor HTTP or QEMU QMP details directly. The trait methods perform
Unix-socket HTTP or broker IPC, so they must be async/non-blocking in the daemon
runtime.

QEMU QMP lifecycle commands are always broker-mediated through a typed broker
operation. The unprivileged daemon does not open or speak to QMP sockets
directly. This preserves the broker audit boundary and avoids duplicating the
QMP parser already used for qemu-media boot/hotplug.

Provider-state polling uses an explicit bounded interval (for example one or
two seconds) between attempts. Mutating QMP lifecycle commands such as
`system_powerdown` and `quit` produce bounded broker audit records. Read-only
polling commands such as repeated `query-status` do not emit one
`OpAuditRecord` per poll attempt; their results are represented through the
daemon lifecycle audit/summary and bounded telemetry to avoid audit-log floods.
Expected connection loss while the VM is terminating (`ECONNRESET`, EOF,
ENOENT, and similar) is classified as expected termination context rather than a
broker ERROR audit event for the read-only query op.

### Force-stop override

The public lifecycle request carries an explicit serde-defaulted force flag
surfaced by the CLI as `d2b vm stop <vm> --force --apply`. The serde default
is load-bearing: existing JSON clients that omit the field keep the graceful
default.

Force-stop means:

1. skip provider graceful request and provider-state wait;
2. use the existing pidfd SIGTERM wait and SIGKILL escalation policy;
3. record a durable audit event and warning-style summary that this stop
   intentionally bypassed graceful guest shutdown.

Force-stop is not a synonym for immediate SIGKILL. It is an emergency escape
hatch from guest/provider shutdown wait. If SIGTERM succeeds, no SIGKILL is
sent.

`d2b vm restart <vm> --force --apply` is supported and applies the force
flag only to the stop phase before the subsequent start. The start phase is
unchanged.

Every stop-like public surface carries the same force semantics:
`d2b vm stop`, its top-level `down` alias, `d2b vm restart`, and any
environment/all-VM down or restart surface. In particular, top-level `down` must
support `--force` so operators can recover a hung environment without manually
force-stopping every VM. `-f` is the short alias for `--force` on stop-like
commands. Unsupported future combinations are rejected explicitly; no stop-like
command silently ignores `--force`.

Every graceful-stop and force-stop request writes a durable daemon lifecycle
audit event to the existing managed daemon audit stream before provider
requests or signals are sent. This covers Cloud Hypervisor `vm.shutdown` even
though the unprivileged daemon sends that HTTP request directly, and it covers
force-stop even if the final SIGTERM/SIGKILL is delivered directly via pidfd
rather than through a broker `SignalRunner` fallback. The audit record captures
bounded, non-secret fields such as VM, peer uid/authz class, provider,
`force_requested`, and the applied timeout in seconds.
The final shutdown outcome is also recorded durably with a bounded outcome enum
so audit trails show whether the VM shut down cleanly, timed out, or required
forced cleanup.

Environment-wide stop/down/restart operations must preserve dependency order:
workload VMs complete their graceful or forced stop before d2b stops the
auto-declared net VM for that environment. Otherwise guests can lose bridge/TAP
connectivity while still trying to flush network-backed services.

Host shutdown/reboot uses the same policy. The framework still declares no
per-VM systemd units; instead, the singleton `d2bd.service` participates in
system teardown by invoking the daemon's all-VM graceful shutdown path before
the host reaches final process killing. That path preserves workload-before-net
VM ordering and waits for primary VMM pidfd exit before reporting completion.
The systemd integration must distinguish host shutdown/reboot from a manual
`systemctl restart d2bd.service`: daemon restarts remain continuation events
and must not stop VMs. The NixOS unit uses `ExecStop=` to call a CLI hook such
as `d2b host shutdown-hook`; systemd runs that hook before sending SIGTERM
to the daemon main process. The hook uses a robust systemd state check before
invoking all-VM shutdown, preferably querying
`org.freedesktop.systemd1.Manager` directly and falling back only to checking
that `systemctl is-system-running` returns the exact state `stopping`. It exits
immediately for normal daemon restarts. It does not parse job listings with
grep, and the daemon does not trap SIGTERM to stop VMs. Unit commands use
absolute store paths for the d2b CLI and systemd helpers, not PATH lookup.
If the hook communicates with the daemon over `public.sock`, daemon authz
uses the same lifecycle authorization surface as every other public operation:
the `d2bd` system user is a member of the `d2b` lifecycle group, and
`SO_PEERCRED` observes that group-authorized peer. No special hardcoded daemon
uid bypass is added.

No fourth root-visible shutdown unit is introduced. ADR 0015's three-unit
surface remains intact: `d2bd.service`, `d2b-priv-broker.socket`, and
`d2b-priv-broker.service`. To avoid systemd refusing socket activation after
the shutdown transaction starts, d2bd ensures the existing broker service is
already active whenever it supervises or adopts live VMM runners that may need
broker-mediated shutdown, for example by holding a broker keepalive connection
while live VMMs exist. If no live VMM runners exist, no graceful VMM shutdown
needs broker activation during host teardown.
`d2bd.service` is ordered `After=d2b-priv-broker.service` so, during
shutdown, systemd stops d2bd before terminating the active broker service;
broker-mediated QMP shutdown remains available for the full graceful sequence.

Within each dependency phase, shutdown requests run in parallel: all workload
VMs in an environment receive graceful shutdown before the net VM phase begins.
The unit's `TimeoutStopSec` is derived as at least
`maxWorkloadTimeoutSeconds + maxNetVmTimeoutSeconds +
2 * forceFallbackTimeoutSeconds + sidecarCleanupGraceSeconds`:
one maximum timeout for the workload phase, one maximum timeout for the net-VM
phase, one forced-fallback window for each phase, plus sidecar cleanup grace.
The maxima include the daemon default and all per-VM overrides; disabled
graceful-shutdown VMs contribute zero, and empty phases contribute zero.
`forceFallbackTimeoutSeconds` covers the existing SIGTERM/SIGKILL waits, and
`sidecarCleanupGraceSeconds` is a concrete Nix budget that accounts for the
declared reverse-DAG sidecar cleanup chain, including forced-fallback windows for
sidecar roles that still stop sequentially. The rendered systemd value includes
the `s` suffix. This prevents systemd from reaching final shutdown killing
before d2b's graceful, forced-fallback, and sidecar cleanup phases can
complete.
The NixOS unit orders `d2bd.service` after the broker service, broker
socket, dbus service/socket, and any systemd-recognized `d2b.slice` unit if
present, so those dependencies remain available while `ExecStop=` runs. The
calculated `TimeoutStopSec` is assigned with an override priority that still
permits local operator overrides.

### Status, restart adoption, and degraded reporting

Status/list must not report a VM as cleanly running when the provider reports a
guest-stopped state but the VMM process still exists. This can happen after a
Cloud Hypervisor shutdown request leaves the VMM in `Created`, or after QEMU
reports `shutdown` while the QEMU process remains alive.

Daemon restart adoption may still adopt a live VMM pidfd when identity matches,
but the public lifecycle state must combine pidfd state with provider state:

- primary VMM pid alive + provider running => `Running`;
- primary VMM pid alive + provider guest-stopped => stopped/degraded cleanup
  required, not clean `Running`;
- no primary VMM pid alive => `Stopped` after owned sidecar cleanup or degraded
  if cleanup cannot be proven.

If startup adoption discovers a live primary VMM pidfd whose provider state is
already guest-stopped, the daemon resumes empty-VMM cleanup automatically rather
than leaking the terminal VMM process until a manual `vm stop`.

Pidfd exit detection uses async pidfd readability (`tokio::io::unix::AsyncFd`
or the daemon runtime's equivalent over `poll(2)` / `epoll(7)`), not
`waitid(P_PIDFD, ...)`, because `d2bd` is not necessarily the VMM's parent
process. `waitid` can report `ECHILD` for non-child pidfds while the process is
still alive; pidfd readability is the correct liveness signal for supervised
broker-spawned runners.

Primary VMM pidfd readability alone does not prove all runner resources are
released: a leaked child process could still hold TAP fds or the vsock socket.
Restart is allowed only after both the primary VMM pidfd is readable and the
primary runner leaf cgroup and resource-holding sidecar leaf cgroups report
`cgroup.events populated == 0`. If the primary VMM has exited but a relevant
leaf remains populated, those are live leaked processes, not zombie-only
bookkeeping; the daemon escalates through the existing broker-mediated leaf
`CgroupKill` operation documented in `docs/reference/cgroup-delegation.md`
rather than passively polling forever. The daemon never writes `cgroup.kill`
directly. The post-`CgroupKill` wait for `populated == 0` is strictly bounded;
uninterruptible leaked processes that keep a leaf populated after the bound are
reported as degraded rather than hanging the stop operation.

Provider socket errors that race with process exit (`EPIPE`, `ECONNRESET`, EOF,
or equivalent HTTP/QMP disconnects) are classified by re-checking pidfd liveness
before reporting an API failure. If the pidfd exited, the stop is a clean VMM
exit, not a forced provider failure.

Forced fallback and graceful-timeout outcomes must be visible in the mutating
command summary. If the daemon has a durable degraded-state surface for the
case, status/doctor should expose it with a remediation command.
Degraded markers created for forced or timed-out shutdown are cleared on the
next successful clean `vm stop` or successful `vm start`, so recovered VMs do
not stay permanently degraded.

An explicit operator `--force` request is not itself a degraded condition. It is
a successful execution of operator intent and is recorded as an audit event plus
a warning-style command summary. Degraded markers are reserved for unexpected
graceful timeout, provider API/QMP failure, or cleanup failure.
When graceful shutdown is administratively disabled by configuration, command
summaries distinguish it from an explicit operator force request, for example
`graceful shutdown disabled by config; used standard forced cleanup`.

When a degraded marker is present after an unexpected graceful timeout or
cleanup failure, `status`/`doctor` remediation distinguishes guest-level timeout
from host-level empty-VMM cleanup failure. Guest timeout text points operators to
resolve the guest issue or run `d2b vm stop <vm> --force --apply` when they
explicitly choose to bypass guest shutdown waiting. Empty-VMM cleanup failure
points to host-side runner cleanup/remediation rather than blaming the guest.

Telemetry uses bounded enum attributes such as `clean_guest_shutdown`,
`clean_vmm_cleanup`, `api_unavailable`, `timeout_exceeded`, and
`force_requested`; it also includes the applied graceful-shutdown timeout as a
numeric metric value or span attribute, not as a Prometheus string label. It
must not use full human CLI summary strings as metric or span attribute values.
Local diagnostic
logs may include raw provider I/O or QMP/HTTP error details in the daemon
journal to support operator debugging, subject to existing redaction rules.
QMP broker audit records include only bounded parameters such as VM name and QMP
command name; raw QMP responses and guest-controlled output are excluded.
Degraded shutdown states are exported as bounded metrics as well as status/doctor
markers so operators can alert on hung VMs from dashboards.
Dashboards must surface `timeout_exceeded` prominently so operators can identify
VMs that need per-VM or global graceful shutdown tuning/disablement.
Shutdown telemetry also records the actual elapsed graceful-shutdown duration
(for example as a histogram named with a `_seconds` base-unit suffix or bounded
span numeric attribute) so operators can tune global and per-VM `timeoutSeconds`
from observed guest behavior. Audit
events emitted while resuming empty-VMM cleanup during daemon restart adoption
carry a distinct trigger/action field so they are not confused with
user-initiated stop requests. Raw provider errors logged locally are truncated
and redacted before writing to the journal.

### Desktop control surfaces

Downstream operator controls must preserve the safe default. In
`d2b-wlcontrol`, the primary visible Stop button remains graceful stop.
Force shutdown is available only from the expanded controls revealed by the
ellipsis affordance, uses destructive styling, and requires explicit
confirmation. It must not be offered as a primary/default button.

## Consequences

- Normal stop/restart/down gives stateful guests a chance to flush services and
  filesystems before the host terminates the VMM process.
- Stop behavior becomes provider-aware, so lifecycle code needs a test seam for
  Cloud Hypervisor HTTP and QEMU QMP state.
- The public stop contract grows an explicit force flag. JSON consumers must
  default it to false for backward compatibility, and clients omit the field
  when false so newer CLIs can still talk to older daemons that deny unknown
  request fields.
- Public wire/schema artifacts must be regenerated with the repository's xtask
  generators after adding the force flag and any broker QMP shutdown op.
- The broker privilege catalogue and rendered privileges schema must include
  the new QMP lifecycle operation(s), their authorization boundary, audited
  fields, and redaction posture.
- QEMU shutdown requires a new typed broker operation so QMP lifecycle commands
  remain broker-mediated and audited like existing qemu-media boot/hotplug
  operations.
- The status path must account for provider state, not just pidfd liveness, for
  local hypervisor runtimes.
- Daemon config rendering must be audited while adding the new timeout. Any
  existing declared daemon option that is missing from `daemon-config.json`
  rendering, such as autostart parallelism, must be fixed in the same change so
  Nix overrides reach the daemon.
- Adding per-VM graceful shutdown metadata changes the public manifest contract;
  `manifestVersion` must be bumped and `docs/reference/manifest-schema.{md,json}`
  must be updated.
- During live host upgrades, daemons must handle the older manifest version
  until the host switches to a configuration that renders the new lifecycle
  metadata; missing lifecycle fields default to the pre-upgrade behavior.
- Migration documentation must tell operators that graceful stop increases the
  maximum stop and host reboot/shutdown duration and show how to disable it
  globally or per VM for guests that do not respond to provider shutdown,
  including qemu-media live ISOs or other non-ACPI-aware ephemeral media.
- The load-bearing VM lifecycle section in `AGENTS.md` must be updated in the
  same change so contributors preserve the new provider-aware shutdown contract.
- Force-stop remains available for hung guests but is intentionally less
  prominent in human UI surfaces.

## Alternatives considered

### Keep pidfd-only stop

Rejected. It is simple and already implemented, but it makes host-side VMM
termination the normal path for stateful guests and leaves avoidable data-loss
failure modes open.

### Always send SIGKILL for force-stop

Rejected. The force override bypasses guest shutdown waiting, not all cleanup
discipline. Preserving SIGTERM before SIGKILL keeps forced stop less destructive
when the VMM can exit promptly.

### Make force-stop the primary UI action

Rejected. It optimizes for rare emergency recovery at the expense of the safe
default. The force action belongs behind an advanced/ellipsis affordance with
clear confirmation text.

### Cloud Hypervisor only

Rejected. qemu-media is a local hypervisor runtime with a provider shutdown
channel, and the runtime seam should not encode a one-provider lifecycle policy.

## Validation

Required implementation validation:

- unit tests prove Cloud Hypervisor `vm.shutdown` is requested before signaling
  `ch-runner`;
- unit tests prove QEMU `system_powerdown` is requested before signaling the
  `qemu-media` runner;
- broker tests prove QMP shutdown/status/quit lifecycle commands route through
  the typed broker op and never require daemon-direct QMP access;
- tests cover environment-wide stop/down/restart ordering so workload VMs finish
  graceful shutdown before the env's net VM is stopped;
- tests cover failed-start rollback skipping provider graceful wait and using
  the force cleanup path for any spawned primary VMM;
- tests cover rapid restart waiting for primary VMM pidfd exit/TAP fd release
  and vsock CID release before the subsequent start, including resource-holding
  sidecar leaf cgroups;
- tests simulate a leaked child process in the primary VMM leaf and verify
  broker `CgroupKill` clears it before restart proceeds;
- NixOS/unit tests cover host shutdown service behavior, including
  `TimeoutStopSec` being at least maximum workload timeout plus maximum net-VM
  timeout plus two forced-fallback windows plus sidecar cleanup grace,
  accounting for per-VM overrides, and all-VM graceful shutdown running in
  parallel per dependency phase;
- NixOS/unit tests cover manual `systemctl restart d2bd.service` remaining a
  continuation event, not triggering all-VM shutdown;
- NixOS/unit tests cover shutdown ordering keeping `d2b-priv-broker.service`
  active until d2bd completes broker-mediated QMP shutdown;
- tests cover pidfd exit detection through `poll`/`epoll` semantics rather than
  `waitid(P_PIDFD)` for non-child VMMs;
- tests cover async `AsyncFd`/runtime integration for pidfd readability so the
  daemon executor is not blocked by raw polling calls;
- tests cover provider guest-stopped state with a live VMM pid so list/status do
  not report clean `Running`;
- tests cover daemon restart adoption resuming empty-VMM cleanup when provider
  state is already guest-stopped;
- tests cover API/QMP unavailable and graceful-timeout forced fallback with
  explicit summaries;
- tests cover the force flag bypassing provider graceful wait while preserving
  SIGTERM/SIGKILL policy;
- CLI tests cover progress text for long waits and clarify that `--force` skips
  graceful shutdown but still begins standard SIGTERM teardown, not immediate
  SIGKILL;
- tests cover a concurrent `vm stop --force` interrupting an in-progress
  graceful stop for the same VM instead of waiting for the original timeout;
- tests cover legacy `VmLifecycleRequest` JSON without `force` deserializing to
  `force = false`, and normal clients omitting `force=false` from serialized
  payloads for old-daemon compatibility;
- CLI parser tests cover `vm restart --force` propagation to the stop phase;
- audit tests cover durable recording of explicit `force_requested` intent even
  when the final signal is delivered by daemon pidfd rather than broker
  fallback;
- audit tests cover durable recording of the final shutdown outcome, not only
  pre-action intent;
- Nix eval tests cover timeout option rendering and invalid values;
- Nix eval tests cover global enable propagation to supported per-VM defaults,
  per-VM overrides, and eval-time `1..600` bounds for global/per-VM timeouts;
- Nix eval/manifest tests cover per-VM graceful shutdown disablement and
  per-VM timeout override rendering;
- manifest schema/reference tests cover the required `manifestVersion` bump and
  `docs/reference/manifest-schema.{md,json}` updates;
- bundle/schemaVersion impact is evaluated and documented for private bundle
  artifacts; bump if the private bundle contract changes;
- Nix eval/rendering tests cover the new timeout and any previously-declared
  daemon options fixed in the same change;
- Nix eval tests cover `TimeoutStopSec` aggregation and `s` suffix rendering
  with mixed per-VM timeout overrides and disabled graceful shutdown;
- drift validation covers regenerated public/broker schemas and docs after wire
  fields or broker ops change;
- privileges tests/docs cover the new QMP lifecycle broker op catalogue entry;
- privileges/audit tests cover read-only QMP `query-status` suppressing success
  audit records to prevent polling floods;
- daemon API docs/drift cover the new force flag and daemon lifecycle audit
  fields;
- AGENTS.md lifecycle guidance is updated alongside the implementation;
- metrics tests cover degraded shutdown marker export for hung/unkillable VMs;
- Cloud Hypervisor metrics tests prove the `ch_stats.rs` helper refactor
  preserves exporter output and treats ENOENT, ECONNREFUSED, EOF, and
  ECONNRESET during startup/termination races as normal scrape unavailability
  rather than noisy journal spam;
- `d2b-wlcontrol` tests cover normal Stop as graceful default and Force
  shutdown as ellipsis-only advanced action.
