# 0018. Removal of the `microvm.nix` flake dependency (v1.1)

- Status: Implemented in v1.1
- Date: 2026-05-31
- Wave: v1.1-P8 → v1.1-P11 (landed)
- Plan slice: v1.1 §§"v1.1-P8 — Re-home processes-json.nix reads", "v1.1-P9 — Replace microvm.vms with nixling-owned submodule evaluator", "v1.1-P10 — Retire microvm@/microvm-virtiofsd@/store-sync templates", "v1.1-P11 — Drop microvm.nix flake input"
- Companion ADRs: [ADR 0001](0001-systemd-free-vm-orchestration.md), [ADR 0004](0004-cloud-hypervisor-runner-shape.md), [ADR 0015](0015-daemon-only-clean-break.md)
- Verification: `tests/microvm-nix-absent-eval.sh` + 4 sibling substrate gates; commit `edde456`.

## Context

nixling v0.x and v1.0 ship with `microvm.nix` as a flake input. The
dependency is used in three structurally distinct ways:

1. **NixOS host module import.** `nixos-modules/host.nix:184`
   imports `inputs.microvm.nixosModules.host`, pulling in upstream
   `microvm.vms` submodule definitions, the per-VM hypervisor option
   tree, the autostart `microvms.target`, and helper Nix expressions
   for virtiofsd/cloud-hypervisor/swtpm.
2. **Per-VM config translation.** `nixos-modules/host.nix:253-298`
   defines `microvm.vms = lib.mapAttrs ...` that re-keys
   `nixling.vms.<vm>` declarations into the upstream submodule
   namespace, sets per-VM `microvm.vsock.cid`,
   `microvm.hypervisor`, and `microvm.cloud-hypervisor.extraArgs`.
3. **Source-of-truth read.** `nixos-modules/processes-json.nix`
   (lines 76, 80, 87, 130, 134, 146-148, 160, 167-170, 180-213, 231,
   246-247, 268, 277, 282, 295-296, 307-326, 354-363, 472-477)
   reads from `config.microvm.vms.<vm>.config.config.microvm.*` to
   assemble the runner argv for cloud-hypervisor, virtiofsd, and
   crosvm. `nixos-modules/manifest.nix:79` reads
   `config.microvm.vms.<vm>.config.config.system.build.toplevel`
   for the per-VM toplevel hash. `nixos-modules/store.nix` and
   several other modules read `microvm.shares` and related fields.

ADR 0015 retained `microvm.nix` because retiring it required the
parallel work of re-homing every read path *and* re-implementing the
per-VM NixOS submodule evaluation (toplevel build, kernel/initrd
selection, share resolution). That work was sized at multiple phases
and explicitly deferred to v1.1.

Pressure for removal came from three directions:

- **Tagline accuracy.** The flake description still reads "Opinionated
  NixOS desktop microVM workspaces on microvm.nix". The v1.0
  end-state already owns every spawn path through `nixling-priv-broker`;
  the "on microvm.nix" framing misleads consumers about who owns
  per-VM lifecycle.
- **Substrate divergence.** Upstream `microvm.nix` evolves on its
  own schedule. nixling's per-VM contracts (broker-spawned runners,
  `OpAuditRecord` tracing, cgroup v2 delegation per
  [ADR 0011](0011-cgroup-v2-delegation-and-pidfd-handoff.md)) diverge
  from what upstream `microvm.nix` assumes (per-VM systemd templates,
  upstream `microvms.target`, upstream sidecar units). Every minor
  upstream change requires nixling-side compatibility work; pinning
  the input does not eliminate the substrate friction.
- **Audit surface.** Importing upstream NixOS modules adds whatever
  the upstream module declares to nixling's evaluated module set;
  audit reviews of "what does `host.nix` actually emit?" enumerate
  upstream modules they don't control.

## Decision

### Hard invariant

> No file under `flake.nix`, `nixos-modules/`, `packages/`, or
> `tests/` references `microvm.*` config, `inputs.microvm.*`, or
> imports anything from the `microvm.nix` flake. `flake.lock` does
> not contain a `microvm` node. The flake tagline reads
> "Opinionated NixOS desktop microVM workspaces" (no "on microvm.nix").

### Migration map (microvm.* → nixling.vms.*)

Every microvm.* field consumed by nixling production paths gets a
nixling-owned counterpart in `nixos-modules/options-vms.nix`. The
mapping is established in v1.1-P8 (processes-json re-home) and
extended through P9–P11.

| Upstream `microvm.*` field (per-VM)              | nixling-owned counterpart                              | Consumer                              |
|--------------------------------------------------|--------------------------------------------------------|---------------------------------------|
| `microvm.vsock.cid`                              | `nixling.vms.<vm>.runner.vsock.cid`                    | processes-json, broker SpawnRunner    |
| `microvm.vcpu`                                   | `nixling.vms.<vm>.runner.cpu.count`                    | processes-json, manifest              |
| `microvm.mem`                                    | `nixling.vms.<vm>.runner.memory.sizeMiB`               | processes-json                        |
| `microvm.hugepageMem`                            | `nixling.vms.<vm>.runner.memory.hugepages`             | processes-json                        |
| `microvm.balloon`                                | `nixling.vms.<vm>.runner.memory.balloon.enable`        | processes-json                        |
| `microvm.hotplugMem` / `hotpluggedMem` / `initialBalloonMem` | `nixling.vms.<vm>.runner.memory.{hotplug,initialBalloon}*` | processes-json |
| `microvm.deflateOnOOM`                           | `nixling.vms.<vm>.runner.memory.balloon.deflateOnOOM`  | processes-json                        |
| `microvm.shares`                                 | `nixling.vms.<vm>.runner.shares`                       | processes-json, store.nix, broker     |
| `microvm.volumes`                                | `nixling.vms.<vm>.runner.volumes`                      | processes-json                        |
| `microvm.devices`                                | `nixling.vms.<vm>.runner.devices`                      | processes-json                        |
| `microvm.kernel.{dev,out}` / `initrdPath` / `kernelParams` | `nixling.vms.<vm>.runner.kernel.*`           | processes-json                        |
| `microvm.storeOnDisk` / `storeDisk` / `writableStoreOverlay` | `nixling.vms.<vm>.runner.store.*`          | processes-json, store.nix             |
| `microvm.virtiofsd.*` (package, group, inodeFileHandles, extraArgs, threadPoolSize) | `nixling.vms.<vm>.runner.virtiofsd.*` | processes-json, store.nix |
| `microvm.hypervisor`                             | (constant `"cloud-hypervisor"` in nixling — drop option) | processes-json                       |
| `microvm.cloud-hypervisor.package`               | `nixling.vms.<vm>.runner.hypervisor.package`           | processes-json                        |
| `microvm.cloud-hypervisor.extraArgs`             | `nixling.vms.<vm>.runner.hypervisor.extraArgs`         | processes-json                        |
| `microvm.cloud-hypervisor.platformOEMStrings`    | `nixling.vms.<vm>.runner.hypervisor.platformOEMStrings`| processes-json                        |
| `microvm.graphics.{enable,socket,crosvmPackage}` | `nixling.vms.<vm>.runner.graphics.*`                   | processes-json, broker Gpu role       |
| `microvm.interfaces`                             | `nixling.vms.<vm>.runner.interfaces` (W3 ownership; see note below) | processes-json, host.nix, net.nix     |

Every option in the new tree carries the same Nix type as the
upstream original. The translation table is materialized in
`nixos-modules/options-vms.nix` with `description` strings that
cross-link this ADR.

**On `microvm.interfaces`.** At v1.0 HEAD `00b24c5` the
`microvm.interfaces` field is NOT yet fully nixling-owned despite
W3's IfName / TAP / macvtap ownership of the underlying
*interface naming and reconcile* surface. Three production sites
still write or consume `microvm.interfaces` directly:

- `nixos-modules/host.nix:97` writes the **net-VM** TAP/macvtap
  interface list as `microvm.interfaces = lib.mkForce [ ... ]`.
- `nixos-modules/net.nix` declares workload-VM `microvm.interfaces`
  entries for per-VM TAP attachments.
- `nixos-modules/processes-json.nix:76` reads `microvm.interfaces`
  to assemble cloud-hypervisor `--net` argv.

v1.1-P8 must re-home all three sites to `nixling.vms.<vm>.runner.interfaces`
(plus a `nixling.netVm.interfaces` analogue for the net VM
itself). The W3 IfName derivation
(per [ADR 0012](0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md))
and bridge-port + reconcile policy
(per [ADR 0013](0013-w3-firewall-coexistence-policy.md))
already own the *naming and reconcile* of these interfaces; v1.1
moves the *declaration surface* itself. The migration map row
above tracks that move.

### Per-VM NixOS evaluation

`microvm.nix` evaluates each VM as a NixOS toplevel via the
`microvm.vms.<vm>.config.config` evaluation tree, which produces
`config.system.build.toplevel` per VM. v1.1 implements the same
shape via `nixos-modules/vm-submodule.nix`:

- Each `nixling.vms.<vm>` declaration is evaluated as a NixOS
  submodule with a private `pkgs`, the nixling base modules, and
  the per-VM `runner.*` option family above.
- `nixling.vms.<vm>.config.system.build.toplevel` is the per-VM
  toplevel build, replacing
  `config.microvm.vms.<vm>.config.config.system.build.toplevel`.
- `nixling.vms.<vm>.config.system.build.declaredRunner` is computed
  by nixling-owned argv synthesis in `processes-json.nix`. Because
  this collapses production and oracle into the same code path, the
  ADR 0004 "declaredRunner as independent compatibility oracle"
  framing is retired: v1.1 keeps the same NAME for source-stability
  but the role is "production runner descriptor", not "parity
  oracle". To preserve drift detection against upstream
  cloud-hypervisor argv expectations, the existing golden fixtures
  under [`tests/golden/runner-shape/`](../../tests/golden/runner-shape)
  (already covering `examples-minimal-declaredRunner.txt`,
  `cloud-hypervisor-argv-minimal.txt`, `audio-argv-minimal.txt`,
  `gpu-argv-minimal.txt`, `otel-host-bridge-argv-minimal.txt`,
  `swtpm-argv-minimal.txt`, `usbip-argv-minimal.txt`,
  `video-argv-minimal.txt`, `virtgpu-ioctl-values.txt`,
  `virtiofsd-argv-minimal.txt`, `vsock-relay-argv-minimal.txt`,
  `parity-drift.json` — 12 fixtures total) become the parity
  oracle. **Important scope note.** At HEAD `00b24c5` the
  [`tests/runner-shape-snapshot.sh`](../../tests/runner-shape-snapshot.sh)
  driver script only exercises 2 of those fixtures
  (`examples-minimal-declaredRunner.txt` and
  `cloud-hypervisor-argv-minimal.txt`); the other 10 fixtures
  exist on disk but are consumed by other test scripts. v1.1-P9a
  extends `runner-shape-snapshot.sh` to diff every fixture in
  `tests/golden/runner-shape/` so the v1.1 parity oracle actually
  covers every supported runner shape. The frozen fixtures are
  refreshed only by panel-approved CH-version bumps.

### Sidecar/template retirement — full role matrix

v1.1 retires every per-VM systemd-template surface AND every
retired-host-singleton surface listed in
[`tests/legacy-unit-denylist-eval.sh`](../../tests/legacy-unit-denylist-eval.sh).
**Denylist coverage scoping note** (resolves R10 test-r10-2): at
v1.0 HEAD the `legacy-unit-denylist-eval.sh` gate source-scans
`nixos-modules/` for a subset of the patterns enumerated in the
matrix below; some matrix rows reference patterns that are
**scheduled to be added to the denylist gate in their owning
v1.1-P<N> phase** (per the TDD-table P10 rows in the v1.1 plan)
rather than being protected at v1.1-P0 landing. The matrix's
status column does NOT change based on gate-coverage timing —
each row's disposition (SpawnRunner / Host-prep DAG / Retired
in P6) is canonical for the v1.1 design regardless of when the
denylist gate row lands. The matrix below covers ALL 14
denylist patterns from the gate plus the v1.1-P10-expanded
patterns scheduled for that phase, each with one of three
dispositions:

- **SpawnRunner** — replaced by a broker `SpawnRunner{role: ...}`
  variant. **The role disposition matrix in this ADR (section
  "Disposition matrix" below) is the canonical, normative source
  of truth for the v1.1 SpawnRunner role inventory.** Earlier
  drafts of this ADR misnamed ADR 0004 as "the component-
  coverage matrix"; [ADR 0004](0004-cloud-hypervisor-runner-shape.md)
  is the cloud-hypervisor runner shape ADR (defining the
  Hypervisor role's argv/FD plumbing/sd_notify contract) and
  does NOT enumerate the broker SpawnRunner role variants, so
  v1.1+ implementations and reviewers MUST treat this ADR 0018
  matrix as authoritative for the `RunnerRole` enum. The
  `packages/nixling-contracts/src/runner_role.rs` enum that lands in
  v1.1-P10 derives its variants from this matrix, and the
  `tests/broker-spawn-audit-parity-eval.sh` gate enforces parity
  between the Rust enum and the matrix rows (resolves R10
  virt-r10-2 + R11 docs-r11-1).
- **Host-prep DAG** — replaced by a daemon-owned host-preparation
  op (no per-runner spawn; ordering is enforced inside `nixlingd`).
- **Retired in P6** — the unit is already gone in v1.0 source as
  part of the daemon-only clean break (per ADR 0015); the v1.1
  matrix records the disposition for completeness so future
  reviewers do not need to chase the history.

#### ADR 0011 invariant — applies to every SpawnRunner row

Every SpawnRunner role MUST preserve the
[ADR 0011](0011-cgroup-v2-delegation-and-pidfd-handoff.md)
delegated-subtree path `nixling.slice/<vm>/<role>`, keep the VM
interior cgroup nodes process-free, hand a pidfd to nixlingd over
`SCM_RIGHTS` before lifecycle ownership transfers, and use
leaf-only **broker-mediated `CgroupKill`** (v1.1-P10 op per
[ADR 0011](0011-cgroup-v2-delegation-and-pidfd-handoff.md)
Decision item 6 + [`docs/reference/cgroup-delegation.md`](../reference/cgroup-delegation.md)
"Broker ops on the cgroup tree" — broker is the sole writer of
`cgroup.kill`; daemon uses `pidfd_send_signal(SIGTERM)` first
and only requests broker-mediated `CgroupKill` escalation as
last resort) for teardown. The invariant applies
equally to broker-spawned **pre-launch hook subprocesses** (e.g.,
the store-sync rsync that runs as a Hypervisor pre-launch step):
each hook either (a) runs as its own SpawnRunner leaf with pidfd
handoff and leaf-only broker-mediated `CgroupKill`, or (b) is
in-process / forks no child (in which case there is no separate
teardown cgroup).
Hooks MUST NOT create grandchildren that escape their parent
runner's cgroup leaf.

#### Minijail / sandbox-wrapper parentage preservation (resolves R9 virt blocking)

Some SpawnRunner roles execute their payload through a
**minijail** (or equivalent sandbox wrapper) for additional
seccomp/namespace/capability confinement (the role catalog at
ADR 0004 leaves wrapper choice per role; v1.1-P10 will document
per-role wrapper settings). When a role uses such a wrapper, the
broker's lifecycle invariants — `waitid(P_PIDFD)`-based reap,
`OneShotComplete` semantics, leaf-only broker-mediated `CgroupKill`,
and the
cgroup-empty check after final reap — depend on the broker
remaining the parent of the **same kernel process object** the
pidfd refers to. The following wrapper-launch invariants are
therefore **normative** for every SpawnRunner role that wraps in
minijail/nsjail/bwrap:

1. **Exec-in-place required.** The wrapper MUST `exec` the
   payload in place (i.e., end with `execve(2)` of the payload
   binary). The same kernel `task_struct` is reused; the pidfd
   the broker holds via `clone3(CLONE_PIDFD)` / parent-side
   `pidfd_open(child_pid, 0)` is stable across the exec and
   continues to refer to the supervised payload.
2. **No double-fork.** The wrapper MUST NOT double-fork (i.e.,
   MUST NOT fork an intermediate process whose `exec` is the
   payload and whose parent then exits). Double-forking would
   re-parent the payload to PID 1 (or the nearest subreaper),
   detaching it from the broker; the broker's pidfd would then
   refer to the wrapper's exited zombie, and
   `waitid(P_PIDFD)` would reap the wrapper — NOT the payload.
   The cgroup leaf would remain populated by an
   un-reaped-by-broker payload, breaking the cgroup-empty
   check after final reap.
3. **No daemonization.** The wrapper MUST NOT call
   `daemon(3)`/`setsid(2)` + `fork(2)` to detach. Same
   re-parenting problem as double-fork.
4. **No subreaper claim inside the wrapper.** The wrapper MUST
   NOT call `prctl(PR_SET_CHILD_SUBREAPER, 1)`. ADR 0018 §
   "set-booted race-free serialization" already establishes
   NEITHER the broker nor the daemon claims subreaper for
   SpawnRunner children; this invariant extends to wrappers.
5. **pidfd-table entry tracks the supervised payload.** The
   `terminal_on_exit: true` pidfd-table entry the broker
   registers MUST refer to the payload's pidfd (which, by
   invariant 1, is the wrapper-process pidfd surviving across
   `execve`). The broker's pidfd-table MUST NOT register a
   pre-exec wrapper-helper pidfd that exits before the payload
   does.

A pre-implementation **wrapper-parentage test gate**
`tests/broker-spawn-minijail-parentage-eval.sh` (future,
v1.1-P10) shall exercise each wrapper-using role with a
synthetic payload that, inside the wrapper, calls
`pidfd_open(getpid(), 0)` to obtain a **payload-self-pidfd**,
captures `getppid()` + `getsid()`, and sends the payload-self-
pidfd to the broker via `SCM_RIGHTS` over the OneShotComplete
RPC channel along with the prctl readbacks. The broker then:

(a) `fstat`s its own pidfd-table entry for the supervised
    payload AND `fstat`s the SCM_RIGHTS-received payload-self
    pidfd; on pidfs-backed kernels (Linux ≥ 6.9, the v1.1
    kernel floor per ADR 0008's v1.1 uplift) BOTH fstats MUST
    return identical `(st_dev, st_ino)` pairs — this is the
    **process-identity check (NOT PID-namespace identity)** and
    proves the pidfd-table entry tracks the supervised payload
    (NOT a pre-exec wrapper helper that exited before the
    payload). An earlier draft of this test contract proposed
    comparing against `readlink /proc/<child>/ns/pid_for_children`
    which is PID-NAMESPACE identity and would FALSELY PASS a
    helper-vs-payload drift case (R17 test reviewer correction);
    the payload-self-pidfd via `pidfd_open(getpid(), 0)` is the
    correct primitive because pidfs inodes are stable per
    `struct pid` and a wrapper-helper-exited-before-payload
    setup would produce a DIFFERENT pidfs inode for the helper
    vs the payload, causing the fstat compare to fail.

(b) Asserts `getppid()` of the payload equals the broker's PID
    (no re-parenting per invariants #1-#3); asserts `getsid()`
    is the broker's session (no setsid/daemonize); asserts
    `prctl(PR_GET_CHILD_SUBREAPER, ...)` from inside the
    wrapper returns 0 (invariant #4 — wrapper MUST NOT claim
    subreaper); asserts `pidfd_send_signal(0, broker_pidfd)`
    succeeds (process still tracked).

(c) **Negative fixture (required)**: a synthetic
    wrapper-helper-vs-payload setup where the wrapper deliberately
    double-forks so the wrapper-helper exits before the payload
    `execve`s. The broker's pidfd-table entry then points at
    the exited wrapper-helper while the payload runs detached.
    The payload-self-pidfd via `pidfd_open(getpid(), 0)` returns
    a pidfd to the payload's `struct pid`; fstat of that pidfd
    differs from fstat of the broker's wrapper-helper pidfd
    (different pidfs inodes). The negative-fixture test asserts
    this case CAUSES the gate to FAIL — proving the
    process-identity comparison detects helper-vs-payload drift
    that PID-namespace identity would miss.

Wrappers that fail any of the five invariants above MUST NOT
be used by any SpawnRunner role; the per-role wrapper
configuration in v1.1-P10 is panel-reviewed against this
contract.

Roles that run **without** a wrapper (the v1.0 default for most
SpawnRunner rows above) are unaffected — they are direct
`execve` of the payload by the broker-forked child and the
pidfd-table entry trivially refers to the payload's kernel
process.

#### Role-independent OpAuditRecord lifecycle baseline

The `OpAuditRecord` kinds enumerated in the matrix below are the
**minimum** every row MUST emit. The role-independent baseline
applies to every SpawnRunner role:

- `SpawnRequested` — broker received the spawn request.
- `SpawnSucceeded` — child process started, pidfd opened.
- `SpawnFailed` — exec failed (ENOENT, EACCES, cgroup error).
- `ChildSignalled` — broker delivered a signal (SIGTERM/SIGKILL)
  to the child.
- `ChildExited` — child exited (pidfd became readable); record
  includes exit status + signal + WIFEXITED/WIFSIGNALED disposition.
- `Restarted` — if the role's restart policy applies, after a
  ChildExited the broker emits this when respawning.
- `PreLaunchHookStarted` / `PreLaunchHookSucceeded` /
  `PreLaunchHookFailed` — if the role has any pre-launch hook.
- `LivenessProbeStarted` / `LivenessProbeOk` /
  `LivenessProbeWedged` / `WedgeRestarted` — if the role uses
  active liveness probing (Virtiofsd does; others may opt in).

The `OpAuditRecord` parity gate
`tests/broker-spawn-audit-parity-eval.sh` (future, v1.1-P10)
asserts EVERY enabled SpawnRunner role emits the role-independent
baseline kinds. Any role that legitimately does not (e.g., a role
with restart-disabled does not emit `Restarted`) MUST list the
omitted kind in the role's audit-baseline-exception list with a
panel-approved rationale.

#### Disposition matrix (every legacy-unit-denylist entry)

| Retired systemd surface (denylist pattern)             | Disposition       | Replacement detail                                                                                            | Role-baseline `OpAuditRecord` kinds applicable |
|--------------------------------------------------------|-------------------|---------------------------------------------------------------------------------------------------------------|------------------------------------------------|
| `microvm@<vm>.service`                                 | SpawnRunner       | `Hypervisor` — cloud-hypervisor argv, signal/restart/audit, pidfd handoff                                     | full baseline (Spawn + Child + Restarted + PreLaunchHook for store-sync) |
| `microvm-virtiofsd@<vm>.service` (per-share drop-in)  | SpawnRunner       | `Virtiofsd` (one per share) — virtiofsd argv per share, FD plumbing, ACL setup                               | full baseline + LivenessProbe (active wedge detection per below) |
| `nixling-<vm>-store-sync.service`                      | Pre-launch hook   | `Hypervisor` pre-launch hook — rsync + hardlink-farm population; subprocess MUST run as its own SpawnRunner leaf per ADR 0011 binding above | PreLaunchHookStarted/Succeeded/Failed + (if forks child) full Spawn/Child baseline |
| `nixling-<vm>-swtpm.service`                           | SpawnRunner       | `SwtpmFlush` (one-shot) + `Swtpm` (long-lived) — swtpm-flush state migration; swtpm pidfd                    | full baseline (Restarted N/A for SwtpmFlush) |
| `nixling-<vm>-gpu.service`                             | SpawnRunner       | `Gpu` — crosvm gpu sidecar argv, GPU device ACL, socket FD via SCM_RIGHTS                                    | full baseline + Restarted + signal-on-VM-shutdown |
| `nixling-<vm>-video.service`                           | SpawnRunner       | `Video` — vhost-user-video argv, video device ACL                                                            | full baseline + Restarted |
| `nixling-<vm>-snd.service`                             | SpawnRunner       | `Audio` — vhost-user-sound argv, audio ACL, pipewire socket FD via SCM_RIGHTS                                | full baseline + Restarted |
| `nixling-otel-relay@<vm>.service`                      | SpawnRunner       | `OtelGuestRelay` — per-VM OTLP relay, vsock FD plumbing                                                      | full baseline + Restarted |
| `nixling-otel-host-bridge.service`                     | SpawnRunner       | `OtelHostBridge` — host-side OTLP bridge, ACL refresh (replaces `host-otel-relay-acl.nix`), unix-socket plumbing | full baseline + Restarted (v1.1-P6 lands this) |
| `nixling-vfsd-watchdog@.{service,timer}`               | Embedded in role  | Active liveness probe inside `Virtiofsd` SpawnRunner role (see "Virtiofsd wedge detection" below)             | LivenessProbe set (Started/Ok/Wedged/WedgeRestarted) |
| `nixling-sys-<env>-usbipd-{proxy,backend}.{service,socket}` | SpawnRunner   | `UsbipBackend` (per-env, long-lived `usbipd -4 --tcp-port <backendPort>`) + `UsbipProxy` (per-env, `systemd-socket-proxyd` front binding `<env.hostUplinkIp>:3240`). At v1.0 HEAD these are ALREADY broker-spawned under `nixling.slice/sys-<env>/usbipd-*` per [`docs/reference/components-usbip.md`](../reference/components-usbip.md); v1.1 only consolidates the role-matrix entry and registers the denylist pattern. **v1.1 RunnerRole catalog reconciliation note** (resolves R10 virt-r10-2 + R11 docs-r11-2 + networking-r11-2): the v1.0 [`privileges.md`](../reference/privileges.md) catalog lists a SINGULAR `Usbip` SpawnRunner role and the broker ops `UsbipBind` / `UsbipUnbind` / `UsbipProxyReconcile` / `UsbipBindFirewallRule`. The v1.1 design SUPERSEDES the singular role into the multi-variant inventory below (per ADR 0018 § "Disposition matrix" — this matrix is the canonical RunnerRole source per the section preamble above); the v1.0 broker ops (`UsbipBind` / `UsbipUnbind` / `UsbipProxyReconcile` / `UsbipBindFirewallRule`) remain unchanged and are what the v1.1 SpawnRunner leaves dispatch through. v1.1-P10 lands the corresponding privileges.md update (singular `Usbip` row → 6-variant rows) alongside the runner_role.rs enum. **Per-attach lifecycle reconciled with [`docs/reference/components-usbip.md`](../reference/components-usbip.md).** The hot-plug ceremony has **two distinct execution contexts**: (a) host-side `usbip bind`/`unbind` against the local usbipd (dispatched via the existing `UsbipBind`/`UsbipUnbind` broker ops); AND (b) guest-side `usbip attach`/`detach` which MUST run INSIDE the workload VM (per components-usbip.md "Guest-side resources created" and `vhci_hcd` requirement). Both contexts dispatch through the broker `SpawnRunner` DAG as **ephemeral one-shot SpawnRunner leaves** — but the leaf's exec payload differs: **host-side** leaves exec the host `/run/current-system/sw/bin/usbip` binary directly; **guest-side** leaves exec an `ssh` client invocation whose **remote-command** argv is the `usbip attach`/`detach` against the in-guest vhci_hcd (the same model components-usbip.md describes as "Rust CLI SSHs in and issues `usbip attach`"). The v1.1 SpawnRunner role naming reflects this split: **host-side** roles `UsbipBindOneShot{busid}` (dispatches the existing `UsbipBind` broker op via a one-shot SpawnRunner leaf — these are NOT new broker ops, they are SpawnRunner variants that exec the host `usbip bind` payload) and `UsbipUnbindOneShot{busid}` (dispatches existing `UsbipUnbind` broker op); **guest-side** roles `GuestUsbipAttachOneShot{vm, busid}` (no corresponding host-side broker op; exec: `ssh -i <vm.ssh.keyPath> <vm.ssh.user>@<vm.staticIp> -- usbip attach -r <env.usbipdHostIp> -b <busid>`) and `GuestUsbipDetachOneShot{vm, busid}` (exec: `ssh ... -- usbip detach -p <port>`). Both contexts spawn under `nixling.slice/sys-<env>/usbip-<verb>-<id>/` cgroup leaf with pidfd handoff per the ADR 0011 invariant binding above. **The `UsbipBindFirewallRule` broker op stays a broker op (NOT a SpawnRunner role).** Earlier drafts of this row described it as a SpawnRunner; per privileges.md § "Broker dispatcher fields" it is the existing v1.0 broker op that emits nftables carve-outs. It is invoked from the host-prep DAG ordering (before `UsbipBackend` SpawnRunner starts) and from the per-attach state machine (before `UsbipBindOneShot` SpawnRunner runs); there is NO `UsbipUnbindFirewallRule` op — carve-out removal is performed by re-invoking `UsbipBindFirewallRule` with a `destroy: true` payload field (the standard W3 broker-op destroy convention per [`ApplyNftables`](../adr/0013-w3-firewall-coexistence-policy.md) precedent). The guest-side `ssh` client invocation, like every host-launched SpawnRunner payload, is constrained by the minijail parentage invariants (exec-in-place, no double-fork) per the "Minijail / sandbox-wrapper parentage preservation" subsection below. **ssh(1) hardening for `Guest*OneShot` payloads** (resolves R11 kernel ssh-parentage finding): the broker's `ssh` argv MUST NOT pass `-f` (would daemonize and re-parent), MUST NOT pass `-N`/`-M` (background master-mode would do the same), MUST set `-o ControlMaster=no` and `-o ControlPath=none` (disables multiplexing master sockets which can outlive the pidfd-tracked client), MUST set `-o ControlPersist=no`, MUST set `-o BatchMode=yes` (no interactive prompts; deterministic exit on auth fail), MUST NOT read user-level `~/.ssh/config` (use `-F /dev/null`), and MUST exec the configured key + user + host explicitly. The broker enforces this via a static argv builder; the `tests/broker-spawn-minijail-parentage-eval.sh` gate (future, v1.1-P10) exercises the ssh-OneShot leaves with a synthetic guest that asserts `getppid()` and pidfd identity match expectations — proving the ssh client is exec-in-place and not double-forked. Cross-env busid exclusivity is enforced by `host.json.environments[].usbipBusidLocks[].busIds` (the per-env flock contract); the broker MUST hold the lock for the duration of the bind→attach→detach→unbind sequence and audit `UsbipLockAcquired`/`UsbipLockReleased`/`UsbipLockContended` events around it (these are daemon-side audit-event kinds, NOT broker ops or SpawnRunner roles — they land in v1.1-P10 daemon-side audit catalog). **Pre-spawn `modprobe usbip-host`** is NOT in-process — it execs the host's `/run/current-system/sw/bin/modprobe` binary. Per the R4 virt finding, this requires its own disposition: register as a daemon **host-prep DAG op** `ModprobeIfAllowed{module: "usbip-host", matrix_entry_id}` per [`docs/reference/privileges.md`](../reference/privileges.md) row `ModprobeIfAllowed` (already catalogued, scope `kernel module`, gated by `nixling.site.yubikey.enable` + at least one VM with `usbip.yubikey = true`). The host-prep DAG runs the modprobe op BEFORE the first `UsbipBackend` SpawnRunner starts for each env. **ModprobeIfAllowed failure propagation** (resolves R6 virt + R7 virt findings): if the modprobe op fails the host-prep DAG aborts the per-env USBIP bring-up sequence and returns a typed `#broker-validation-failed` envelope (exit 31 per [`error-codes.md`](../reference/error-codes.md)). The envelope `kind` is always `broker-validation-failed`; the **fine-grained denial reason** is carried in the **audit `error_kind` field** (NOT the envelope kind) using existing catalog codes from `error-codes.md`: `#modules-disabled-sysctl-locked` (kernel.modules_disabled=1 prevented load), `#host-modules-locked` (host blocks all loads), or `#modprobe-denied-not-in-matrix` (module not in the trusted-matrix allowlist). The dependent `UsbipBackend` SpawnRunner is NOT started; the daemon emits `HostPrepAborted{env, op: "ModprobeIfAllowed", error_kind, broker_op_id}` and the operator sees the typed envelope on `nixling vm start --apply` (or whichever verb triggered the per-env bring-up). The modprobe op itself is short-lived but runs under broker oversight in its own ephemeral cgroup leaf | full baseline + Restarted on the two long-lived runners (`UsbipBackend`/`UsbipProxy`); ephemeral one-shot SpawnRunner leaves (`UsbipBindOneShot`/`UsbipUnbindOneShot` host-side; `GuestUsbipAttachOneShot`/`GuestUsbipDetachOneShot` guest-via-SSH) emit the **full SpawnRunner baseline** (SpawnRequested/Succeeded/Failed + ChildExited; Restarted N/A on one-shots — listed in `tests/fixtures/broker-spawn-audit-baseline-exceptions.yaml` (future, v1.1-P10 — fixture file does NOT exist at HEAD; it will be created in v1.1-P10 alongside the role implementations) via the `applies_to: {lifecycle: one_shot}` predicate with `owner_discipline: virt`); the per-attach lock kinds (`UsbipLockAcquired`/`UsbipLockReleased`/`UsbipLockContended`) are emitted at the daemon→broker dispatch boundary, NOT at the SpawnRunner level, and are tracked under a separate daemon-side audit catalog (NOT the SpawnRunner role-baseline parity gate) |
| `microvm-tap-interfaces@.service` (and per-VM TAP)     | Host-prep DAG     | `ApplyW3TapInterfaces` (already W3-owned in v1.0 per ADRs 0012/0014)                                          | (covered by existing W3 audit kinds, not SpawnRunner) |
| `microvm-setup@.service`                               | Retired in P6     | Subsumed into daemon host-prep DAG (per ADR 0015 § Decision); no replacement needed                          | n/a |
| `microvm-pci-devices@.service`                         | Host-prep DAG     | `ApplyDeviceCgroup` (per-VM PCI passthrough device ACL via daemon-owned cgroup device controller)            | (covered by existing host-prep audit kinds) |
| `microvm-set-booted@.service`                          | Daemon-RPC + broker write-once | In-broker write-once on Hypervisor `SpawnSucceeded`; broker hands `(vm, generation, pidfd)` triple to nixlingd; nixlingd is the **single writer** of boot-counter state; accepts the update iff `(vm, generation)` matches the currently-owned Hypervisor pidfd; atomic compare-and-set after readiness validation (see "set-booted race-free serialization" below) | `DaemonStateUpdated` (nixlingd-side) + `SpawnSucceeded` (broker-side) |
| `nixling-known-hosts-refresh@.service`                 | Retired in P6     | Daemon emits known-hosts on bundle apply (per ADR 0015 § Decision); no replacement needed                    | n/a |
| `nixling-net-route-preflight.service`                  | Retired in P6     | Folded into daemon host-prep DAG `RoutePreflight` op (per ADR 0014); no separate unit                        | n/a |
| `nixling-audit-check.{service,timer}`                  | Retired in P6     | Daemon-owned audit pipeline (per ADR 0010 § audited broker read path); no separate unit                      | n/a |
| `nixling-ch-exporter.service`                          | Retired in P6     | Retired entirely (per ADR 0015 § Decision). The transitional ACL-refresh remnant in `host-otel-relay-acl.nix:256` (the `g:nixling-ch-exporter refresh_acl_set` call) is retired by v1.1-P6 as part of the ACL-script retirement | n/a |

**One-shot SpawnRunner lifecycle cleanup (Usbip*OneShot,
SwtpmFlush, store-sync hook child if forked).** Every ephemeral
one-shot SpawnRunner leaf MUST follow this race-safe cleanup
sequence after the broker emits `SpawnSucceeded`. **Critical
ownership note (resolves R7 kernel HIGH + rust major).** The
broker is the **PARENT** of every SpawnRunner child (it forked
the child); the daemon receives the pidfd via `SCM_RIGHTS` for
identity verification (BootedNotify) but is NOT the child's
parent. Per `waitid(2)`, only the calling process's own children
can be waitid'd; `waitid(P_PIDFD, pidfd_held_by_daemon, ...)`
would return `ECHILD` because the daemon is not the parent. The
v1.1 cleanup sequence therefore puts the **broker** in charge of
reaping, with the daemon as an observer:

1. After `SpawnSucceeded` the broker holds (a) the pidfd to its
   own child — preferentially obtained from `clone3(CLONE_PIDFD)`
   which atomically returns a pidfd to the new child in the parent
   (see [ADR 0011](0011-cgroup-v2-delegation-and-pidfd-handoff.md)
   § Decision step 8); the fallback path is parent-side
   `pidfd_open(child_pid, 0)` immediately after `fork(2)` returns
   the child's PID to the parent (NOT `pidfd_open(getpid())`,
   which would return a pidfd to the broker itself, not its
   child; the R9 kernel reviewer flagged the earlier wording as
   structurally impossible — a parent cannot inherit a pidfd
   opened by the child after fork, and `getpid()` evaluated in
   the parent targets the broker). (b) the cgroup leaf path, and
   (c) registers a
   pidfd-table entry on its own side with `terminal_on_exit:
   true` (a v1.1-introduced flag that opts the entry into
   one-shot cleanup semantics). The broker also forwards a
   `dup`'d pidfd to the daemon via `SCM_RIGHTS` for the
   daemon's BootedNotify identity-verification use case only;
   the daemon does NOT register the pidfd for reaping.
2. The **broker** polls the pidfd for readability via `epoll(7)`
   on the pidfd's `EPOLLIN` set. When readable (or already-exited
   at registration time — both paths are handled identically by
   the polling loop), the broker enters the cleanup sequence:
   - **First `waitid` (WNOWAIT — peek the exit status without
     reaping):** `waitid(P_PIDFD, pidfd, &si, WEXITED | WNOWAIT)`.
     The broker is the parent so this returns the exit status
     successfully. WNOWAIT explicitly does NOT reap the zombie —
     it returns the exit status while keeping the child in
     zombie state so subsequent inspection can use the pidfd
     unambiguously.
   - Emit `ChildExited` `OpAuditRecord` (broker-side audit log)
     with the exit status read above.
   - **Final `waitid` (NO WNOWAIT — reap the zombie):**
     `waitid(P_PIDFD, pidfd, &si, WEXITED)`. This call REAPS
     the zombie. The zombie was holding a cgroup reference; the
     reap releases it so the cgroup leaf becomes truly empty.
     (Per `pidfd(2)` semantics, `close(pidfd)` does NOT reap;
     only `waitid`/`waitpid` does.)
   - Read `nixling.slice/<vm>/<usbip-verb-id>/cgroup.events`
     to verify `populated 0`. **Order matters (resolves R7
     kernel MEDIUM)**: the populated check happens AFTER the
     final reap because the WNOWAIT-pinned zombie itself counts
     as a populated task in the cgroup; checking before the
     reap would always see `populated 1` due to the zombie.
     If populated remains non-zero AFTER reap (orphaned
     grand-children escaped the leaf despite the ADR 0011
     binding), idempotently issue broker-mediated `CgroupKill`
     against the leaf (broker is sole writer of `cgroup.kill`
     per cgroup-delegation.md "Broker ops on the cgroup tree";
     daemon does NOT write `cgroup.kill` directly) AND emit
     `OrphanGrandchildKilled` audit event (security-load-bearing
     — escape is a bug).
   - Close the pidfd via `close(2)`. (Safe to do AFTER the
     reaping `waitid`; the pidfd's only remaining purpose was
     to pin the kernel process object during the audit emit.)
   - Issue `rmdir(2)` on the cgroup leaf path. EBUSY is now
     impossible (zombie reaped, populated 0 verified); EBUSY
     would indicate a TOCTOU race that the broker tooling
     escalates as `LeafRemovalFailed`.
   - **Broker → daemon RPC**: broker sends `OneShotComplete`
     to the daemon. **Wire spec** (resolves R8 rust major):
     `OneShotComplete` is a new broker→daemon **unsolicited
     notification** variant added to the
     [ADR 0010](0010-wire-protocol-and-typed-errors.md) wire
     protocol in v1.1-P10. The notification is **non-blocking
     fire-and-forget** (resolves R8 virt minor): the broker
     queues the notification onto its outbound channel and
     immediately continues to the next pidfd-table cleanup;
     the daemon-side observer consumes notifications from its
     inbound channel asynchronously. A daemon-side timeout
     watchdog (configurable, default 30s) flags
     never-acked notifications via a `OneShotCompleteAckTimeout`
     audit event but does NOT block the broker. The Rust DTO
     (lands in `packages/nixling-contracts/src/broker_wire.rs` in
     v1.1-P10):

     ```rust
     // packages/nixling-contracts/src/broker_wire.rs (v1.1-P10 addition)
     //
     // Derive set matches the existing broker-wire enum convention
     // (per `packages/nixling-contracts/src/broker_wire.rs` v1.0 HEAD):
     // every wire DTO must derive Debug + Clone + PartialEq + Eq +
     // serde::Serialize + serde::Deserialize + schemars::JsonSchema.
     // The R9 rust reviewer flagged the earlier minimal Serialize/
     // Deserialize-only derives as failing the broker-wire trait
     // bounds at compile time (the `BrokerWireMessage` enum that
     // OneShotComplete is added to derives the full set above).
     #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
     #[serde(rename_all = "camelCase", deny_unknown_fields)]
     pub struct OneShotComplete {
         pub runner_role: RunnerRole,
         pub runner_id: String,
         pub exit_status: i32,
         pub orphan_killed: bool,
         pub cleanup_outcome: CleanupOutcome,
     }

     #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
     #[serde(rename_all = "kebab-case")]
     pub enum CleanupOutcome {
         Success,          // serialized as "success"
         LeafRemovalFailed, // serialized as "leaf-removal-failed"
     }
     ```

     The struct uses camelCase JSON keys (`runnerRole`,
     `runnerId`, `exitStatus`, `orphanKilled`,
     `cleanupOutcome`) to match the existing nixling-contracts wire
     style (resolves R8 rust major); the enum variants
     serialize as kebab-case via `serde(rename_all)`. The
     ADR 0010 update for the new variant lands as part of
     v1.1-P10's wire-protocol section.

     The daemon updates its observer-side pidfd-table (clears
     the `terminal_on_exit` entry, closes its own `dup`'d
     pidfd) and emits its mirror audit record. The daemon
     does NOT call waitid (it's not the parent).
3. If the one-shot exits BEFORE the broker's pidfd poll runs
   (e.g., a very-short-lived `usbip detach`), the
   `terminal_on_exit: true` flag ensures the broker's polling
   loop's first iteration still observes EPOLLIN and runs the
   cleanup path; there is no race window where the broker
   "misses" the exit.

**Subreaper / SIGCHLD disposition (resolves R7 virt + kernel
LOW + R8 virt cross-ADR conflict + R8 kernel LOW).** Neither
broker NOR daemon sets `PR_SET_CHILD_SUBREAPER` for **SpawnRunner
children**. This SUPERSEDES the v1.0 `nixlingd` PR_SET_CHILD_SUBREAPER
clause in [ADR 0011 line 84](0011-cgroup-v2-delegation-and-pidfd-handoff.md):
under v1.0, the daemon was the spawning process for many VM
subprocesses and benefited from subreaper semantics for cleanup;
under v1.1's broker-is-parent model the daemon is NOT the
spawning process for SpawnRunner children, so subreaper
semantics would create cross-process reparenting that
complicates the pidfd-table accounting. The v1.1 supersession
is normative: v1.1+ `nixlingd` does NOT set
`PR_SET_CHILD_SUBREAPER` for SpawnRunner handling. (ADR 0011's
v1.0 subreaper note remains accurate for the v1.0 source
state; v1.1-P12 docs-polish adds an explicit "Superseded by
ADR 0018 in v1.1 for SpawnRunner children" note to ADR 0011 §
that paragraph.)

The broker's SIGCHLD disposition is `SIG_DFL` for SpawnRunner
children — the default disposition preserves zombies pending
the explicit `waitid(P_PIDFD)` polling-loop reap (per signal(7)
`SIG_DFL` for SIGCHLD is "ignore" semantics that DOES NOT
auto-reap, exactly what the WNOWAIT-then-reap pattern requires).
The broker explicitly DOES NOT install a SIGCHLD handler that
would auto-reap or short-circuit the cleanup sequence; the
broker also explicitly does NOT use `SA_NOCLDWAIT` (which
WOULD auto-reap).

The daemon's SIGCHLD disposition matters ONLY for the daemon's
own direct children. The current direct-child set is:

1. The **pidfs self-probe helper child** (forked at startup,
   immediately `pause()`s, then SIGKILL'd and explicitly waitid'd
   by the probe code).

For this direct child the daemon contract is:

- The daemon's SIGCHLD disposition MUST NOT be `SIG_IGN`
  (which would auto-reap and leave waitid with ECHILD) and MUST
  NOT use `SA_NOCLDWAIT` (same effect via flag).
- If the daemon installs a general SIGCHLD handler (e.g., for
  future logging purposes), the handler MUST NOT call
  `waitid`/`waitpid` for the pidfs-probe helper PID. The
  cleanest implementation is to BLOCK SIGCHLD around the
  fork → pidfd_open → SIGKILL → waitid sequence using
  `sigprocmask(SIG_BLOCK, &chld_set, ...)` and unblock after
  the explicit waitid completes.
- The pidfs-probe code in `packages/nixlingd/src/startup.rs`
  (future, v1.1-P10) implements the SIGCHLD-block-around-probe
  pattern; the test `packages/nixlingd/tests/pidfs_probe.rs`
  asserts the helper child is NOT auto-reaped by a stray
  SIGCHLD handler.

The daemon's SIGCHLD disposition is **irrelevant for SpawnRunner
children** (they're not the daemon's direct children; the
broker is their parent and handles all reaping). A daemon
SIGCHLD handler that fires from broker activity (e.g., from
the broker's own children racing the daemon's process group)
would receive an ECHILD from any waitid attempt on those PIDs,
so the daemon's handler must tolerate ECHILD gracefully (treat
as no-op rather than panic).

The `tests/broker-spawn-audit-parity-eval.sh` gate asserts every
ephemeral one-shot SpawnRunner emits the cleanup sequence
correctly (including the `OrphanGrandchildKilled` audit path
when the test injects a process into the leaf, AND the
`OneShotComplete` broker→daemon RPC for forensic correlation).

**Virtiofsd wedge detection.** A pidfd becomes readable only when
the process exits; it does not detect wedges, uninterruptible-I/O
hangs, or no-longer-servicing-FUSE-requests states. Replacing
`nixling-vfsd-watchdog@.{service,timer}` therefore requires a
broker-owned **active liveness probe** inside the `Virtiofsd`
SpawnRunner role: a periodic FUSE ping (or timeout-bounded `stat`
on a sentinel path within the share) with a configurable threshold.
On threshold exceedance the broker emits `LivenessProbeWedged`,
leaf-kills the virtiofsd cgroup, and restarts per the
`restart-policy-eval` contract. Pidfd poll is retained as a
*secondary* signal (process-exit notification) but is not
authoritative for wedge detection. If the active probe cannot be
landed in v1.1-P7, the alternative is to keep the existing
`nixling-vfsd-watchdog@.timer` until v1.2; the v1.1 plan landing
order MUST commit to one of these two paths before P7 begins.

**Kernel modules + `modules_disabled` parity.** Upstream
`microvm.nix` derives a `kernelModules` requirement matrix for each
VM (built-in vs loadable; whether the host kernel currently has the
module loaded). [ADR 0014](0014-w3-modules-devices-runner-shape.md)
requires that nixling's host-check + runner-shape preflight consume
this matrix AND fail closed under `kernel.modules_disabled=1`. v1.1
must reproduce the matrix end-to-end. The parity gate compares the
full ADR 0014 contract, not just the module-name list:

- `nixos-modules/vm-submodule.nix` (v1.1-P9a) emits the same
  `requiredKernelModules` / `optionalKernelModules` lists for each
  example VM that the upstream evaluator produces, AND the same
  per-module **requirement class** (`required` vs `optional`),
  **load-fail-if-locked** policy (the ADR 0014 fail-closed
  behaviour under `modules_disabled=1`), and any associated
  sysctls (e.g. `net.bridge.bridge-nf-call-iptables=0` for the
  br_netfilter coexistence per ADR 0013).
- `packages/nixling-core/src/host_check.rs` continues to consume
  the lists without modification.
- New parity gate `tests/kernel-modules-parity-eval.sh`
  (future, v1.1-P9a) compares the v1.0 (microvm.nix-derived) and
  v1.1 (vm-submodule-derived) kernel-module matrices for every
  example VM:
  - **Set-equality** on `requiredKernelModules` AND
    `optionalKernelModules` (ordering ignored because nothing in
    nixling source consumes list order for module loading — verified
    by `rg -nE 'kernel_modules|kernelModules' packages/nixling-core/src/`
    showing only iteration patterns at HEAD `4b5274b` — in particular
    `for module in &host.kernel_modules` at
    `packages/nixling-core/src/host_check.rs:541`, indexed iteration
    at `host_check.rs:659,662,993,994`, struct definition at
    `packages/nixling-core/src/host.rs:115` (no order-sensitive
    consumers found).
  - **Per-module attribute equality** on requirement class,
    load-fail-if-locked policy, and sysctl-association — these
    fail the gate if any drift, regardless of ordering.
  - **modules_disabled fail-closed behaviour assertion**: synthesize
    a fixture host with `kernel.modules_disabled = 1` and a VM
    that requires a loadable module not currently loaded;
    `host_check.rs` must return the same failure typed envelope
    against both evaluators.
  If ordering is later shown to be semantically significant for
  any consumer, the gate is upgraded to require list-order
  equality and the v1.1-P9a checkpoint records the change.

### set-booted race-free serialization

The retired
`microvm-set-booted@<vm>.service` wrote `/var/lib/nixling/vms/<vm>/booted`
on systemd "boot reached" notification. The v1.1 replacement
splits the write into a **broker side** (in-process write-once on
`Hypervisor` `SpawnSucceeded` reaching the documented readiness
condition) and a **daemon side** (single writer of the boot-counter
state under `/var/lib/nixling/vms/<vm>/state.json`). The two
sides interlock as follows:

- Broker holds a per-`(vm, hypervisor-generation)` pidfd from the
  cgroup-delegation handoff (per ADR 0011). Generation increments
  monotonically on every `SpawnRequested` for the same VM.
- On `SpawnSucceeded` AND readiness validation (sd_notify READY=1
  received on the VM's vsock, or the role's documented readiness
  predicate per ADR 0004), broker sends the daemon a
  `BootedNotify { vm, generation, pidfd }` RPC. The pidfd is
  passed via `SCM_RIGHTS` so the daemon can verify ownership.
- Daemon is the **single writer** of `state.json`. It accepts
  the `BootedNotify` IFF:
  1. The generation matches the daemon's currently-owned
     Hypervisor pidfd for that VM, AND
  2. The passed pidfd refers to the **same kernel process object**
     as the daemon-held pidfd. Equality is established by
     **`fstat(2)` of both pidfds** which on pidfs-backed kernels
     returns the pidfs inode of the kernel process object; two
     pidfds for the same process produce the same
     `(st_dev, st_ino)`.

     **Kernel-floor uplift (v1.1 ONLY).** Linux pidfs (the proper
     filesystem backing pidfds with per-process inodes) landed in
     Linux 6.9. On pre-6.9 kernels, pidfds are anon_inode-backed
     and ALL pidfds share the same `(st_dev, st_ino)` — the
     fstat identity check is structurally impossible to satisfy
     correctly. v1.1 therefore **uplifts the v1.0
     [ADR 0008](0008-supported-platforms-and-rejected-targets.md)
     supported kernel floor from `>= 6.6` to `>= 6.9` for ALL
     Tier 0 + Tier 1 hosts**. The v1.0 floor of 6.6 stays in
     [ADR 0008](0008-supported-platforms-and-rejected-targets.md)
     as the historical baseline; v1.1+ requires 6.9 because
     of this dependency, and ADR 0008's "v1.1 kernel-floor
     uplift" subsection cross-links this section explicitly
     a "v1.1 kernel-floor uplift" subsection cross-linking this
     spec. Operators with kernels in `[6.6, 6.9)` MUST upgrade
     their kernel before bumping to v1.1; the v1.1 migration
     guide opens with this requirement, and a new eval-time
     assertion `tests/v1.1-kernel-floor-eval.sh` (future,
     v1.1-P10) fails on hosts that declare a kernel package with
     `version < 6.9` (best-effort detection from the configured
     `boot.kernelPackages` derivation).

     **Defence-in-depth runtime probe.** At daemon startup
     `nixlingd` runs a self-probe to confirm pidfs is supported
     on the host kernel. The probe:
     1. Forks a short-lived **helper child** (`fork() + immediate
        `pause()`); the child is reaped by the daemon as part of
        the probe's cleanup. The helper child's PID is
        DIFFERENT from `getpid()` by construction (a fresh PID
        from the kernel) — this avoids the failure mode where
        `nixlingd` is itself PID 1 in its visible namespace
        (e.g., when running inside an init-less container)
        and "PID 1" would be `getpid()` rather than an
        unrelated process. (The R6 kernel reviewer flagged
        the earlier PID-1-based probe as namespace-dependent.)
     2. Opens TWO pidfds to its own PID (`getpid()`) via
        `pidfd_open(getpid(), 0)` twice.
     3. Opens ONE pidfd to the helper child's PID.
     4. `fstat`s all three pidfds; asserts:
        - The two `getpid()` pidfds have the SAME
          `(st_dev, st_ino)` (proves pidfs returns stable
          per-process inodes).
        - The helper-child pidfd has a DIFFERENT
          `(st_dev, st_ino)` from the `getpid()` pair (proves
          pidfs returns DISTINCT inodes for distinct kernel
          process objects — the security-load-bearing claim).
     5. Sends `SIGKILL` to the helper child, waitids it (full
        reap), closes all three pidfds.

     If any of the assertions in step 4 fails, the daemon enters
     degraded mode: all `BootedNotify` RPCs are rejected with a
     typed `pidfs-unavailable` envelope (a new error-codes.md
     entry in v1.1-P10) and the daemon emits a startup
     `pidfs-self-probe-failed` audit record + a journald error
     directing the operator to the kernel-floor section of the
     migration guide. The probe is the runtime fail-closed
     defence backing the eval-time assertion; it catches any
     kernel that satisfies the version string but somehow lacks
     pidfs (e.g., a custom kernel build with pidfs stripped out).

     With pidfs confirmed, the fstat check works as documented:

     ```
     daemon_pidfd: int  // from broker's SpawnSucceeded SCM_RIGHTS handoff
     broker_pidfd: int  // from BootedNotify SCM_RIGHTS receipt

     struct stat a, b;
     if fstat(daemon_pidfd, &a) != 0 || fstat(broker_pidfd, &b) != 0 {
       reject(pidfd_identity_result: "fstat-failed");
     }
     if (a.st_dev, a.st_ino) != (b.st_dev, b.st_ino) {
       reject(pidfd_identity_result: "fstat-mismatch");  // different process
     }
     accept(pidfd_identity_result: "fstat-match");
     ```

     **Threat-model note.** `fstat` proves **same-process identity**,
     not "exact-same-fd-as-sent-over-SCM_RIGHTS". The threat we
     close is: a malicious or buggy broker sends a pidfd that
     references a process OTHER than the Hypervisor it claims. A
     forged pidfd to a different process P' yields different
     `(st_dev, st_ino)` and is rejected; that is the property we
     need. "Exact-fd" stronger claims (e.g., the broker must send
     the literal kernel fd it received from `pidfd_open`) are NOT
     achievable via fstat alone but are also unnecessary for this
     threat model — the broker is trusted to be the Hypervisor's
     spawning process and the daemon already vetted the SCM_RIGHTS
     channel.

     **Why not `pidfd_getfd(2)`.** R4 rust + kernel reviewers
     flagged that an earlier draft mis-specified `pidfd_getfd` as
     a comparison primitive. `pidfd_getfd(pidfd, targetfd, 0)`
     actually duplicates fd `targetfd` from the process referenced
     by `pidfd` into the caller's fd table — it is a
     fd-extraction primitive, not a fd-comparison primitive.
     It is also `PTRACE_MODE_ATTACH_REALCREDS`-gated against the
     target's credentials, and the daemon (non-root per ADR 0002)
     may not satisfy that on a broker-spawned (root-owned) child.
     `pidfd_getfd` is therefore explicitly removed from this spec.

     **Why not `pidfd_send_signal(0)`.** Already rejected in P0fu2
     (only proves signalable, not identity); preserved as
     explicitly-rejected for the same reasons.

     **Why not `kcmp(KCMP_FILE)`.** `kcmp(2)` with `KCMP_FILE`
     could prove "same open file description" (a stronger property
     than fstat's "same process"), but `kcmp` is `CAP_SYS_PTRACE`-gated
     against the target process (`prctl(PR_SET_DUMPABLE, 0)` further
     restricts it). The daemon does not hold `CAP_SYS_PTRACE` on
     broker-spawned children (ADR 0002 confines the daemon to its
     own credential set). Same-fd identity is not required for the
     threat model above; same-process identity via fstat is
     sufficient, and the daemon's credential surface stays minimal.

     **Procfs `Pid:` / `NSpid:` diagnostic cross-check (NOT
     security-load-bearing).** After SCM_RIGHTS receipt, the daemon
     also reads `/proc/self/fdinfo/<daemon_pidfd>` and
     `/proc/self/fdinfo/<broker_pidfd>` and compares the `Pid:`
     and `NSpid:` lines. This cross-check serves **diagnostic
     purposes only** — pidfd already pins the kernel process
     object across PID reuse (a pidfd to an exited process stays
     valid and returns `Pid: -1` per the kernel pidfd contract),
     so PID reuse is structurally impossible to fool the fstat
     check. The procfs read is recorded in audit but is not
     part of the accept/reject decision; a mismatch is logged
     as `pidfd_identity_result: "procfs-diagnostic-mismatch"`
     for forensic correlation but does NOT override an
     fstat-match accept.

  3. The current `state.json` `booted_generation` is strictly
     less than the requested generation (atomic compare-and-set
     on the on-disk state).
- On success the daemon atomically rewrites `state.json` via
  the `tempfile + rename(2)` pattern, then emits
  `DaemonStateUpdated{vm, generation, kind: "booted"}`.
- On rejection the daemon emits a full-diagnostic
  `DaemonStateRejected` record with every field needed to
  distinguish duplicate-broker-delivery from a true generation
  race in post-mortem:

  ```json
  {
    "vm": "<vm-name>",
    "requested_generation": <u64>,
    "observed_booted_generation": <u64>,
    "current_hypervisor_generation": <u64>,
    "pidfd_identity_result": "fstat-match|fstat-mismatch|fstat-failed|procfs-diagnostic-mismatch|absent",
    "cas_failure_class": "stale-generation|future-generation|forged-pidfd|concurrent-broker|n/a",
    "reason": "<human-readable-summary>"
  }
  ```

  **Classification precedence (deterministic; the daemon walks
  the checks in this order and the FIRST failure determines the
  `cas_failure_class`):**
  1. `stale-generation` — `requested_generation <
     current_hypervisor_generation` is checked FIRST; if the
     generation is strictly older than the daemon's current
     hypervisor generation, the daemon rejects WITHOUT
     performing the fstat check (the pidfd is dropped). Wins
     over `forged-pidfd` because a stale generation is
     unambiguous from BootedNotify metadata alone. **Equality
     (`requested_generation == current_hypervisor_generation`)
     proceeds to the fstat check** — this is the acceptance
     path's first stage and matches the "generation matches the
     daemon's currently-owned Hypervisor pidfd for that VM"
     condition above. The R9 kernel reviewer flagged the earlier
     `<=` predicate as self-contradictory: it would reject the
     very acceptance case.
     **Future generations**
     (`requested_generation > current_hypervisor_generation`) are
     ALSO rejected at this stage with a distinct
     `future-generation` class — the daemon cannot have a
     pidfd-table entry for a not-yet-owned generation, so the
     fstat check would have no comparand. The
     `future-generation` rejection is rare in normal operation
     (it implies the broker raced ahead of the daemon's pidfd
     handoff acknowledgement) and is treated as a hard error
     (vs `stale-generation` which can occur during VM restart
     races and is treated as a soft observability event).
  2. `forged-pidfd` — `fstat_identity_result == "fstat-mismatch"`
     OR `"fstat-failed"`. Checked SECOND because the
     pidfd-identity proof depends on a valid pidfd having been
     passed.
  3. `concurrent-broker` — `state.json.booted_generation >=
     requested_generation` (CAS failure on the on-disk write,
     after both above checks pass). Checked LAST because it
     requires acquiring the on-disk lock.

  The `pidfd_identity_result` field is recorded independently of
  the `cas_failure_class`. **Consistency rule** (resolves the R5
  kernel review): when the precedence-walk skips the fstat check
  (i.e., `stale-generation` rejection happens first), the daemon
  does NOT run the procfs cross-check either, and the
  `pidfd_identity_result` field is set to `"absent"` (NOT
  `"procfs-diagnostic-mismatch"`). The `procfs-diagnostic-mismatch`
  value is ONLY emitted when the fstat check ran AND succeeded
  AND the subsequent procfs cross-check disagreed. **This is a
  rare construction** that requires the two pidfds to have been
  opened from different mount-namespace views of `/proc` (per the
  R6 kernel reviewer's analysis: same-procfs-view reads CANNOT
  disagree for the same `struct pid`). The
  `booted_notify_race.rs` test case for `procfs-diagnostic-mismatch`
  is therefore marked `#[ignore]` with the rationale that the
  case is namespace-construction-dependent and not exercisable
  by the standard test harness; if a future test harness gains
  multi-mount-namespace support, the test can be un-ignored.
  The diagnostic emit path itself is still covered by a
  fault-injection test that constructs an artificial
  procfs-mismatch via test-only mock. The reused-PID class from
  earlier drafts is REMOVED: pidfd pins the kernel process
  object across PID reuse so a reused PID cannot pass the fstat
  check by construction.

  The broker also records the rejection in its audit trail with
  the same payload so that a forensic walk of both audit streams
  can correlate a single rejected `BootedNotify` across the two
  processes.

The race window the v1.0 systemd path implicitly closed (boot
notification arriving after the VM was already restarted) is
explicitly closed by the generation + pidfd-equality + CAS
trio. The contract is tested by
`packages/nixlingd/tests/booted_notify_race.rs` (future,
v1.1-P10) which exercises:
- Stale generation (broker delivers `BootedNotify` from
  generation N after daemon already owns generation N+1) →
  rejected with `cas_failure_class: stale-generation` and
  `pidfd_identity_result: "absent"` (the precedence rule above —
  the fstat AND procfs checks are skipped because the generation
  check runs FIRST).
- Forged-pidfd (broker forges a pidfd to an unrelated live
  process and forwards it as if it were the Hypervisor's) →
  rejected with `pidfd_identity_result: fstat-mismatch` and
  `cas_failure_class: forged-pidfd`.
- Concurrent `BootedNotify` for the same `(vm, generation)`
  from two broker instances → exactly one accepted, exactly
  one CAS failure with `cas_failure_class: concurrent-broker`.
- **Procfs-diagnostic-mismatch** (the fstat check succeeds — same
  process object — AND the procfs `NSpid:` cross-check
  disagrees, e.g., the broker passes a pidfd via SCM_RIGHTS to a
  process visible to the daemon under a different namespace
  view). The daemon ACCEPTS the BootedNotify (fstat is
  authoritative) but emits a `pidfd_identity_result:
  "procfs-diagnostic-mismatch"` record for forensic correlation.
  This case is rare but worth covering to assert the diagnostic
  emit path works.

The reused-PID race from earlier drafts is NOT tested because
it cannot occur: pidfd pins the kernel process object across
PID reuse, so a pidfd to the original Hypervisor stays valid
even if its PID is later reused, and a NEW pidfd_open(pid) on
the reused-PID would reference the new process (which fstat
would correctly distinguish from the daemon-held original).

The `systemd.targets.microvms.wants = lib.mkForce [ ]` suppression
in `nixos-modules/host.nix` is removed in v1.1-P11 because
`microvms.target` itself ceases to exist when the upstream module
is no longer imported. **Note on systemd ordering.** Removing
`microvms.target` does NOT remove any security-relevant ordering
because nixling's reconcile / runner ordering was already
DAG/broker-op ordered in v1.0 (not systemd-target ordered) per
[ADR 0013](0013-w3-firewall-coexistence-policy.md) and
[ADR 0014](0014-w3-modules-devices-runner-shape.md). Specifically:
every `SpawnRunner` call requires the host-prep DAG to have
completed `ApplyNftables`, `ApplyNmUnmanaged`, `ApplySysctl`,
`ApplyBridgePortFlags`, and `ApplyW3TapInterfaces` before the
runner starts; the DAG ordering is enforced inside `nixlingd` and
audited via the host-prep `OpAuditRecord` kinds.

The broker service already exists at v1.0 HEAD as
[`nixos-modules/host-broker.nix`](../../nixos-modules/host-broker.nix);
the unit declares `requires = [ "nixling-priv-broker.socket" ]`
and `after = [ "nixling-priv-broker.socket" "local-fs.target" ]`
plus socket activation. v1.1-P4's work is to make this module
the default-on path (currently gated behind
`nixling.daemonExperimental.enable`) and diagnose the
manual-spawn workaround the v1.0 closeout side-task used (the
runtime bring-up gap, not the unit definition). Ordering against
nftables / NM / sysctl is enforced inside the broker IPC
contract, not the systemd unit dependency graph.

### Host-OTel ACL migration table (derived from `nixos-modules/host-otel-relay-acl.nix:251-256`)

`nixos-modules/host-otel-relay-acl.nix` (retired in v1.1-P6
together with the `nixling-otel-relay@<vm>.service` +
`nixling-otel-host-bridge.service` units) refreshes ACLs via four
live `refresh_acl_set` calls plus a vestigial fifth call for
`nixling-ch-exporter` (transitional remnant). The table below is
derived directly from the source at HEAD `00b24c5`; every grant
listed in the source MUST have a v1.1 replacement before the
script can be removed.

State root path used by the ACL refresher: `/var/lib/nixling/vms/`
(per `host-otel-relay-acl.nix` `state_root` resolution). Per-VM
state dirs are `/var/lib/nixling/vms/<vm>/` at mode `0750`
nixling:nixling (set by the v1.1-P5 perms tightening that reverts
the 0755 workaround). Each sidecar group ACL grant requires
`--x` traversal on this parent dir for the group to reach the
nested target — these traversal grants are also listed below.

| Source ref                                                  | Path / socket pattern                                                  | Grantee group(s)                       | Mode  | v1.1 replacement                                                                                                            |
|-------------------------------------------------------------|------------------------------------------------------------------------|----------------------------------------|-------|-----------------------------------------------------------------------------------------------------------------------------|
| `:251` `refresh_acl_set "g:nixling-otel-relay" relay_listener_keep_dirs ... rwx` | Per-VM workload listener dir `/var/lib/nixling/vms/<workload-vm>/`     | `g:nixling-otel-relay`                 | `rwx` + default ACL | `OtelGuestRelay` SpawnRunner broker pre-spawn ops: `MkdirSetown{path: state_dir, owner: nixling-otel-relay, mode: 0700}` + create listener socket directly under daemon-owned cgroup |
| `:251` `refresh_acl_set ... "vsock.sock_${obsOtlpPort}"`    | Per-VM listener socket `/var/lib/nixling/vms/<workload-vm>/vsock.sock_<port>` | `g:nixling-otel-relay`                 | `rw`  | `OtelGuestRelay` SpawnRunner mints socket FD via `socketpair()` / vsock bind + hands to relay child via `SCM_RIGHTS`; no on-disk ACL needed |
| `:252` `refresh_acl_set "g:nixling-otel-relay" relay_stack_keep_dirs ... --x` | Obs-stack VM state dir `/var/lib/nixling/vms/<obs-vm>/`                | `g:nixling-otel-relay`                 | `--x` (traverse only) | `OtelGuestRelay` SpawnRunner broker pre-spawn `SetfaclTraverseOnly{path, group: nixling-otel-relay}` op (no default ACL — explicit single-grant) |
| `:252` `refresh_acl_set ... "vsock.sock"` (stack base, rw)  | Obs-stack base socket `/var/lib/nixling/vms/<obs-vm>/vsock.sock`        | `g:nixling-otel-relay`                 | `rw`  | `OtelGuestRelay` SpawnRunner mints connector FD (CH textual protocol) + hands to relay child via `SCM_RIGHTS`; OR (alternative) broker pre-spawn `SetfaclSocket{path, group, mode: rw}` op |
| `:253` `refresh_acl_set "g:kvm" relay_listener_keep_dirs ... --x` | Per-VM workload listener dir `/var/lib/nixling/vms/<workload-vm>/`     | `g:kvm`                                | `--x` (traverse only) | `Hypervisor` SpawnRunner broker pre-spawn `SetfaclTraverseOnly{path, group: kvm}` op (kvm group is the CH proxy user, needs `--x` to reach the listener socket the relay binds) |
| `:253` `refresh_acl_set ... "vsock.sock_${obsOtlpPort}"`    | Per-VM listener socket `/var/lib/nixling/vms/<workload-vm>/vsock.sock_<port>` | `g:kvm`                                | `--x` (connect-only) | `Hypervisor` SpawnRunner accepts the FD from `OtelGuestRelay` over `SCM_RIGHTS`; no on-disk ACL needed |
| `:254` `refresh_acl_set "g:nixling-otel-bridge" bridge_keep_dirs ... rwx` | Bridge state dir `/var/lib/nixling/vms/<obs-vm>/` (bridge mode)        | `g:nixling-otel-bridge`                | `rwx` + default ACL | `OtelHostBridge` SpawnRunner broker pre-spawn `MkdirSetown{path: bridge_state_dir, owner: nixling-otel-bridge, mode: 0700}` op |
| `:254` `refresh_acl_set ... "vsock.sock"`                   | Bridge base socket `/var/lib/nixling/vms/<obs-vm>/vsock.sock`           | `g:nixling-otel-bridge`                | `rw`  | `OtelHostBridge` SpawnRunner mints connector FD + hands to bridge child via `SCM_RIGHTS` |
| **Parent-dir traversal — new in v1.1**                      | `/var/lib/nixling/vms/<vm>/` parent (0750 after v1.1-P5)               | `g:nixling-otel-relay`, `g:nixling-otel-bridge`, `g:kvm` | `--x` (traverse only) | Daemon-owned activation script (v1.1-P5) grants `--x` ACL to each enumerated sidecar group on every per-VM parent dir; v1.1-P5 owns the activation-script change, v1.1-P6 wires the ACL grants for the OTel groups specifically |
| `:256` `refresh_acl_set "g:nixling-ch-exporter" ch_keep_dirs ... "%VM%.sock"` (RETIRED) | `nixling-ch-exporter` group ACL refresh (transitional remnant of P6-deleted `nixling-ch-exporter.service`) | `g:nixling-ch-exporter`                | (variable) | Retired entirely by v1.1-P6 (no replacement; the underlying `nixling-ch-exporter` service was already retired in P6 per ADR 0015; this ACL refresh is dead code waiting for the script's retirement) |
| (defensive) `:138-148` pre-pass revoke of `g:nixling-otel-relay` / `g:nixling-otel-bridge` on per-VM `vsock.sock` (non-obs-stack) | Per-VM workload `vsock.sock` (NOT obs-stack) | (revoke `g:nixling-otel-relay`, `g:nixling-otel-bridge`) | (n/a — revoke) | `OtelHostBridge` / `OtelGuestRelay` SpawnRunner broker startup invokes the `RevokeSocketAclIfPresent{path, groups: [nixling-otel-relay, nixling-otel-bridge]}` broker op (catalogued as a **distinct broker op** in [`docs/reference/privileges.md`](../reference/privileges.md), NOT a state-mode of `SetSocketAcl`; the earlier "extending SetSocketAcl with state: \"absent\"" framing has been retired per the R5 networking review). Audit fields include the `socket_path_hash`, `groups_revoked`, and `acl_diff` shape per the privileges.md row |

**On `/run/alloy` path.** An earlier draft of this table referenced
`/run/alloy/<vm>/` as the Alloy collector socket path. That was
incorrect — at HEAD `74c36dc` the Alloy host RuntimeDirectory is
`/run/nixling/alloy/` (per
`nixos-modules/components/observability/host.nix:15`
`alloyRuntimeDir = "/run/nixling/alloy"`; the `RuntimeDirectory =
lib.mkAfter [ "nixling/alloy" ]` is set at line 286 with
`RuntimeDirectoryMode = "0710"` at line 287) with no per-VM nested
directory. Any host-side Alloy socket access needed by the relay is
currently granted via the `alloy` user/group declared in that file.
v1.1-P6 preserves that arrangement and does not migrate it into the
broker pre-spawn path; the table above does not include an Alloy
socket row because the retired `host-otel-relay-acl.nix` does not
grant any Alloy-path ACL.

**On `g:kvm` parent-dir traversal vs listener-dir grant.** The
table contains two `g:kvm --x` rows: one on the per-VM workload
listener dir (sourced from `:253`, present in v1.0) and one on the
per-VM parent dir (new in v1.1 from the parent-traversal row).
These are **not redundant** under the v1.1-P5 0750 promotion:
- The `:253`-sourced grant is on the *listener* dir
  `/var/lib/nixling/vms/<workload-vm>/`, which is the same path
  as the parent dir for *that* VM. The two rows are duplicates
  for the workload-VM case.
- For the *obs-stack* VM, only the `:252` `--x` traverse grant
  to `g:nixling-otel-relay` exists in v1.0 source; `g:kvm` has
  no v1.0 grant on the obs-stack dir. The parent-traversal row
  in v1.1 adds `g:kvm --x` to **every** per-VM parent dir
  uniformly so that the v1.1-P5 0750-tightened dir does not
  blackhole the CH proxy (which runs as `kvm` group) from
  reaching nested sockets in any VM context.
The parent-traversal row therefore **supersedes-via-normalization**
the workload-VM listener-dir grant (same effective ACL on the
shared path) AND **adds** a new grant for the obs-stack case.
The v1.1-P5 activation script writes the parent-traversal grants
once per VM and is the only source-of-truth in v1.1; the
v1.0-sourced workload listener-dir grant is removed as dead code
when the underlying `host-otel-relay-acl.nix` script is retired
in v1.1-P6.

**Trigger for ACL refresh after script retirement.** Once
`host-otel-relay-acl.nix` is removed in v1.1-P6, the historic
on-`nixos-rebuild switch` ACL refresh cadence (the script was
invoked from `system.activationScripts.nixlingOtelAcls`) is
replaced by two coordinated trigger points:

1. **NixOS activation-script (every `nixos-rebuild switch`)** —
   v1.1-P6 lands a new `system.activationScripts.nixlingReconcileOtelAcls`
   step that runs on every switch (the same trigger cadence the
   retired script used; this preserves operator expectations).
   The activation script invokes `nixling host reconcile-otel-acls
   --apply` (a new CLI verb gated on `nixling.daemonExperimental.enable`
   being false-or-absent per v1.1-P4, AND on the daemon being
   reachable; if the daemon is unreachable during early activation,
   the script defers the reconcile to the daemon's startup path
   below). This trigger is REQUIRED (not `ExecStartPost` on
   `nixlingd.service`) because **`nixlingd.service` is
   `restartIfChanged = false`** at `nixos-modules/host-daemon.nix:154`
   per the v1.0 daemon-lifecycle invariant — restarting the
   daemon on every switch would interrupt every running VM. An
   `ExecStartPost` on a service that does not restart would not
   reliably fire on `nixos-rebuild switch`, defeating the trigger.

2. **Daemon-startup reconcile** —  `nixlingd.service`
   `ExecStartPost=` invokes a daemon RPC
   `daemon-api/host-prep ReconcileOtelAcls` on the daemon's OWN
   startup (which happens on system boot, NOT on every switch
   because of `restartIfChanged = false`). This covers the
   post-reboot case where the daemon comes up before any
   `nixos-rebuild switch` has run since the reboot.

3. **Per-spawn pre-launch** — every `OtelGuestRelay` /
   `OtelHostBridge` SpawnRunner emits the ACL grants as
   pre-launch broker ops (the table-row replacements above). This
   handles the case where an operator runs
   `nixling vm start --apply` after a state-dir reset without a
   full daemon restart, AND it is the SINGLE source-of-truth
   for per-spawn ACL state.

**Baseline coverage for stopped VMs** (resolves the R5 networking
review's underspecification finding): the activation-time and
daemon-startup reconciles (triggers 1 + 2) establish the
**baseline** ACL set covering ALL declared **AND enabled** VMs
in the bundle — i.e., every VM where
`nixling.vms.<vm>.enable = true`, **whether currently running
or not**. For an enabled-but-stopped VM the state-dir
`/var/lib/nixling/vms/<vm>/` still exists (created by the
v1.1-P5 perms-tightening activation step), and the reconcile
applies the parent-dir traversal ACLs + any per-VM static ACL
grants. **VMs with `nixling.vms.<vm>.enable = false` are
explicitly EXCLUDED from the reconcile**: their state-dirs may
or may not exist depending on whether the operator declared
the VM in a previous generation, but the ACL reconcile does
not touch them (the v1.1-P5 perms-tightening activation only
creates state-dirs for `enable = true` VMs). The per-spawn
pre-launch ops (trigger 3) are only fired when the VM actually
starts; they refresh the listener-socket and stack-base-socket
ACLs that depend on the per-spawn generation of the in-process
FDs. **In short**: enabled-stopped-VM parent-dir traversal IS
established by the activation-time reconcile; per-spawn socket
ACLs are NOT applied until the VM starts (no sockets to grant
on, and the per-spawn broker ops are the single source-of-truth
for the current generation); disabled VMs are not touched at
all. The v1.1-P6 test gate `tests/otel-acl-migration-eval.sh`
covers all three cases (running-VM with per-spawn ops applied;
enabled-stopped-VM with parent-dir only; disabled VM with no
touch).

**Op-ownership clarification.** The
`daemon-api/host-prep ReconcileOtelAcls` op is a **daemon
host-prep action** (not a single broker op). It dispatches to
the broker via existing `SetSocketAcl` / `RevokeSocketAclIfPresent`
broker ops per the migration table above. The audit events:

- `ReconcileOtelAclsStarted` / `Succeeded` / `Failed` are
  recorded in the **daemon-side audit log** (`OpAuditRecord`
  kind extended in v1.1-P6 to include these variants;
  documented in `docs/reference/error-codes.md` host-prep
  catalog section).
- Per-row `SetSocketAcl` and `RevokeSocketAclIfPresent` ops are
  recorded in the **broker-side audit log** with the existing
  `socket_path_hash` + `mode` + `acl_diff` fields per
  `docs/reference/privileges.md:47`.
- The daemon-side `ReconcileOtelAcls` record carries a
  `broker_op_ids` array correlating to the broker-side
  per-row audit entries for forensic walk.

The three trigger points are idempotent (the broker ops are
already declared `partial (replace stale only)` in
`docs/reference/privileges.md:47`), and the activation-time
reconcile establishes the baseline so that per-spawn
incremental work is minimal. No nftables ordering changes are
required because the ACLs are filesystem-scope and have no
firewall interaction.

**Activation-script wiring contract (v1.1-P6 implementation
spec).** The R6 networking reviewer correctly flagged that the
current v1.0 source still wires
`system.activationScripts.nixlingOtelSocketAcls` to the legacy
`nixling-otel-acl-refresh` bash helper. v1.1-P6 lands a
**replacement** activation step
`system.activationScripts.nixlingReconcileOtelAcls` with the
following concrete behaviour. The probe path uses a **minimal
daemon-socket connect check** (NOT the full `host doctor`,
which has many sub-checks for broker / metrics / runner /
module / autostart — a metrics-endpoint failure should NOT
soft-defer the ACL reconcile per the R7 networking review).
The probe invokes `nixling host reconcile-otel-acls --apply
--json` directly and inspects the typed envelope. **Envelope
shape** (per the v1.0 source-of-truth at
[`tests/golden/cli-output/host-check-daemon-down.json`](../../tests/golden/cli-output/host-check-daemon-down.json)
and [`docs/reference/daemon-api.md`](../reference/daemon-api.md)):
the typed envelope is a **top-level JSON object** with
`{code, docs_anchor, exit_code, kind, observed_state,
remediation, what_was_checked}` fields — `code` carries the
machine-readable error kind (e.g., `"daemon-down"`), `kind` is
the broader envelope class (e.g., `"host-check-error"`,
`"host-prep-error"`). The activation script parses `.code`
(NOT the prior-draft `.error.kind` which does not match the
v1.0 envelope shape):

```nix
# nixos-modules/host-otel-reconcile-acls.nix (future, v1.1-P6)
system.activationScripts.nixlingReconcileOtelAcls = {
  text = ''
    set +e  # don't abort activation on daemon-down

    SCRATCH=$(mktemp /run/nixling/activation-reconcile-XXXXXX.json)
    trap 'rm -f "$SCRATCH"' EXIT

    ${nixlingCli}/bin/nixling host reconcile-otel-acls --apply --json \
      > "$SCRATCH" 2>&1
    rc=$?

    # Parse the JSON envelope's top-level `code` field via the
    # daemon's own jq subset.
    err_code=$(${nixlingCli}/bin/nixling-activation-jq \
      -r '.code // empty' < "$SCRATCH" 2>/dev/null)

    case "$rc:$err_code" in
      0:)
        # Success
        ;;
      1:daemon-down)
        # SOFT defer — daemon-startup ExecStartPost will pick it up
        printf 'nixling: daemon not reachable during activation; ' >&2
        printf 'OTel ACL reconcile deferred to nixlingd startup\n' >&2
        exit 0
        ;;
      *)
        # HARD failure — any other non-zero exit OR unrecognized
        # envelope code. Operator sees the typed envelope in stderr
        # and can re-run after fixing the underlying issue.
        printf 'nixling: OTel ACL reconcile FAILED at activation: ' >&2
        cat "$SCRATCH" >&2
        exit 1
        ;;
    esac
  '';
  deps = [ "users" "specialfs" "etc" ];
};
```

**`nixling-activation-jq` provisioning** (resolves R8 networking
major). `nixling-activation-jq` is NOT a separate binary; it
is a thin wrapper shipped INSIDE the `nixlingCli` Nix derivation
(same package, sibling bin) that invokes a vendored Go-based
`gojq` (or a similar pure-Go jq replacement). v1.1-P6 lands the
wrapper as a flake-output package addition:

```nix
# packages/nixling/Cargo.toml or sibling nixlingCli derivation
# v1.1-P6 adds:
nixling-activation-jq = pkgs.writeShellScriptBin "nixling-activation-jq" ''
  exec ${pkgs.gojq}/bin/gojq "$@"
'';
# And the nixlingCli package exports both bins:
postInstall = ''
  ln -s ${nixling-activation-jq}/bin/nixling-activation-jq $out/bin/
'';
```

The wrapper is named `nixling-activation-jq` (not `jq` or `gojq`)
so it does not shadow operator-installed jq on the host PATH.
The eval gate `tests/host-otel-acl-activation-eval.sh` asserts
the binary exists at `${nixlingCli}/bin/nixling-activation-jq`
AND that its output for a fixture envelope matches `.code`
extraction. The vendored gojq is provisioned from the
flake-pinned nixpkgs input (per the no-bash gate's flake-pinned
toolchain pattern).

The activation script distinguishes two failure modes:
- **Daemon-down (soft defer)**: invocation returns exit 1 +
  top-level envelope `code: daemon-down` (with `kind:
  host-check-error` per the envelope-shape spec above; the
  soft-defer predicate is the **exit 1 + top-level `.code` field
  match**, NEVER `.kind`). Activation exits 0 with a
  stderr message; the daemon-startup `ExecStartPost=` trigger
  picks up the reconcile when `nixlingd` starts.
  `nixos-rebuild switch` succeeds.
- **Daemon-reachable but reconcile errored (hard failure)**:
  any other non-zero exit (including `#broker-validation-failed`
  exit 31, `#internal-io` exit 50). Activation exits 1;
  `nixos-rebuild switch` reports a partial activation. The
  operator sees the typed envelope from the reconcile invocation
  in stderr.

**Scratch-file cleanup**: the `trap 'rm -f "$SCRATCH"' EXIT`
removes the temp file on all exit paths (success, soft-defer,
hard-failure). The previous draft used static
`/run/nixling/.activation-{doctor-probe,reconcile}.json` paths
that leaked on success; the v1.1-P6 spec uses `mktemp` +
trap-on-EXIT cleanup so no stale state remains in `/run/nixling`.

**The `deps = [ "users" "specialfs" "etc" ]` ordering** uses
canonical NixOS activation-script names (NOT systemd unit
names — the R6 networking reviewer correctly flagged the prior
`"nixling-priv-broker-socket"` value as the wrong shape).
`users` + `specialfs` + `etc` are the standard pre-requisites
that ensure user/group creation, `/proc`/`/sys` mounting, and
`/etc/nixos` materialization are complete before the
`nixling-priv-broker.socket` activation step (which lives
under systemd, not under activation-scripts). The activation
script does NOT directly depend on the broker socket; it
treats `daemon-down` as a soft-defer, so socket-not-yet-ready
manifests as top-level envelope `code: daemon-down` (with
`kind: host-check-error` carrying the broader envelope class)
and the script exits 0.

New test gate `tests/host-otel-acl-activation-eval.sh` (future,
v1.1-P6) asserts the activation script's NixOS eval produces
the exact shape above (deps ordering uses activation-script
names not systemd unit names, soft-defer triggered by
**exit 1 + top-level `.code == "daemon-down"`** — NEVER on
`.kind`, since `.kind` carries the broader envelope class
`host-check-error` here, hard-failure on other errors,
scratch-file cleanup via trap),
AND that the v1.0 `system.activationScripts.nixlingOtelSocketAcls`
is no longer declared anywhere in the post-v1.1-P6 NixOS
module set.

**Daemon-side audit-event catalog entries** (resolves R6
networking minor). The `ReconcileOtelAclsStarted` /
`ReconcileOtelAclsSucceeded` / `ReconcileOtelAclsFailed`
audit events are NEW daemon-side `OpAuditRecord` variants
introduced in v1.1-P6. They land in `docs/reference/error-codes.md`
in a new "Host-prep daemon-side audit events" subsection as part
of the v1.1-P6 doc-deliverable. v1.0's `error-codes.md` does not
yet list them (they are future entries). The full record shape:
```
ReconcileOtelAcls{Started|Succeeded|Failed} {
  trigger: "activation-script | daemon-startup | per-spawn-prelaunch",
  reconcile_id: <uuid>,
  vm_set: [<vm-name>...],
  broker_op_ids: [<op-id>...],  // empty until ops dispatched
  duration_ms: <u64>,           // Succeeded/Failed only
  error_kind: <enum>            // Failed only; see enum below
}
```

**`error_kind` enumeration for `ReconcileOtelAclsFailed`**
(resolves R7 networking minor). The `error_kind` field is
**not** a free-form string; it MUST be one of the following
enumerated values, mirroring the `error-codes.md` catalog:

- `daemon-down` — daemon RPC layer unreachable (typically only
  emitted by the per-spawn-prelaunch trigger; the
  activation-script trigger never reaches this code path
  because it soft-defers on `daemon-down` per the contract
  above)
- `broker-validation-failed` — one or more `SetSocketAcl` /
  `RevokeSocketAclIfPresent` broker ops were denied by the
  broker's validation layer. The `broker_op_ids` array carries
  the per-row broker audit IDs; each broker-side audit record
  has its own `error_kind` from the broker decision catalog
  (e.g., `socket-acl-target-not-owned`, etc.) for fine-grained
  forensic walk.
- `internal-io` — daemon-side I/O failure during the reconcile
  (e.g., reading the bundle file failed, or the daemon→broker
  IPC channel errored). Indicates a daemon-internal bug or a
  filesystem fault.

Any future error kinds added by v1.1-Pn or later panel-approved
changes MUST extend this enum AND the corresponding
`error-codes.md` catalog entry; the audit-record schema in
`packages/nixlingd/src/audit.rs` (future) declares the enum
with `#[serde(deny_unknown_fields)]` and `serde::Deserialize`
that rejects unknown variants.

New test `tests/otel-acl-migration-eval.sh` (future, v1.1-P6)
asserts that for every row above, the v1.0 `refresh_acl_set` call
produced by `host-otel-relay-acl.nix` has a corresponding broker
pre-spawn op (or SCM_RIGHTS handoff) in the v1.1 source. The test
runs both code paths against a fixture VM (workload + obs-stack)
and diffs the resulting effective ACL via `getfacl --absolute-names
-R /var/lib/nixling/vms/`. The diff must show only the
intentional differences (revoked grants that v1.1 replaces with
fd-passing); any unexplained drift fails the gate.

### Flake-input removal

v1.1-P11 removes `inputs.microvm` from `flake.nix` and drops the
`inputs.microvm.nixosModules.host` import from `host.nix:184`.
`flake.lock` is regenerated; the `microvm` node disappears. The
flake tagline is updated.

### Rust-source follow-through

`packages/nixling/src/lib.rs` currently renders systemd-unit
status output keyed on **six** per-VM systemd units, not just
`microvm@*` and `microvm-virtiofsd@*`. The full inventory at HEAD
`4b5274b`, derived from
`grep -nE 'StatusServicesOutputV2|systemctl_state' packages/nixling/src/lib.rs`:

- `StatusServicesOutputV2` struct definition with six fields:
  `nixling`, `microvm`, `virtiofsd`, `gpu` (Option), `snd`
  (Option), `swtpm` (Option) at `lib.rs:99,112`.
- `vm_service_states` (`lib.rs:3232-3243`) probes systemctl
  state for each of the six units per VM:
  - `nixling@<vm>.service` (line 3235)
  - `microvm@<vm>.service` (line 3236)
  - `microvm-virtiofsd@<vm>.service` (line 3237)
  - `nixling-<vm>-gpu.service` (line 3238, gated on `vm.graphics`)
  - `vm.audio_service` (line 3240-3241, gated on `vm.audio`)
  - `nixling-<vm>-swtpm.service` (line 3242, gated on `vm.tpm`)
- `vm_counts_as_running` (`lib.rs:3275`) consumes the struct
  (must update to read broker-spawn role state).
- `lib.rs:3389` formats `microvm@<vm> (backend): {state}` in the
  human-form status line; additional render sites for the other
  five fields follow the same pattern at adjacent lines.
- Schema goldens under `tests/golden/cli-output/` reference the
  six field names via the generated JSON schema.

v1.1-P10 (template retirement) requires a coordinated Rust-source
update that addresses **every** systemctl probe site, not just the
two `microvm@*`/`microvm-virtiofsd@*` ones:

- Replace ALL six `StatusServicesOutputV2` fields with
  broker-spawn role names that match the SpawnRunner role matrix
  above. Concrete rename map AND explicit Rust DTO type/cardinality
  spec (resolves the R4 rust review's "ambiguous field cardinality"
  finding):
  - `nixling` → (DELETE — the per-VM `nixling@<vm>.service`
    template was retired in P6 per ADR 0015; v1.0 source still
    probes it as a vestigial check, which v1.1-P10 removes).
  - `microvm` → `hypervisor: HypervisorState` (single per-VM
    instance; non-optional because every VM has a Hypervisor
    spawn role).
  - `virtiofsd` → `virtiofsd: BTreeMap<ShareTag, VirtiofsdState>`
    (one entry per share, where `ShareTag` is a `String`).
    **Stable key contract**: the key is the share's `tag` field
    from `nixling.vms.<vm>.runner.shares[].tag`, which v1.1-P8
    makes a REQUIRED + non-empty + unique field in the per-VM
    options schema (the v1.0 source allows array ordering as an
    implicit identifier; v1.1 makes `tag` the explicit stable
    id). Duplicate `tag` values across a VM's shares are
    rejected at eval time by a new assertion in
    `nixos-modules/assertions.nix` that uses a pure
    `lib.groupBy` over the shares list (collecting all
    duplicate tags into a single human-readable error rather
    than throw-on-first; this lets operators fix multiple
    duplicates in one round). The DTO uses `BTreeMap` (not
    `Vec`) so that JSON-output ordering is deterministic on
    the tag string. **Rust deserialization contract**: the
    `nixling-core` crate provides a custom
    `BTreeMap<ShareTag, VirtiofsdState>` deserializer that
    rejects duplicate JSON object keys with an explicit error
    (the stock `serde_json` deserializer silently allows
    duplicates and keeps the last value; v1.1 requires the
    fail-closed deserializer). The test crate also asserts that
    a JSON array shape (`[{"tag":"a","..."},...]`) is REJECTED
    as the wrong shape, with a typed error pointing at the
    expected object shape. Tests in
    `packages/nixling-core/tests/virtiofsd_state_dedup.rs`
    (future, v1.1-P10) cover both the duplicate-key JSON case
    and the wrong-shape-array case.
  - `gpu` → `gpu: Option<GpuState>` (per-VM, present iff
    `vm.graphics == true`; broker-spawn `Gpu` role).
  - `snd` → `audio: Option<AudioState>` (per-VM, present iff
    `vm.audio == true`; rename to match the `Audio` SpawnRunner
    role naming).
  - `swtpm` → `swtpm: Option<SwtpmState>` (per-VM, present iff
    `vm.tpm == true`; field name preserved; the `SwtpmFlush`
    one-shot lifecycle appears as `SwtpmState::flush_history:
    Vec<FlushAttempt>` sub-field, NOT a separate top-level
    field).
  - **New** `otel_relay: Option<OtelGuestRelayState>` — per-VM,
    present iff observability is enabled for the VM (per
    `nixling.vms.<vm>.observability.enable` at HEAD).
  - **New** `otel_host_bridge: Option<OtelHostBridgeState>` —
    **host singleton** (NOT per-VM). The `StatusServicesOutputV2`
    type is currently per-VM-rendered; v1.1-P10 splits the
    output into a per-VM section AND a host section. The
    **schematic JSON-output shape** (resolves the R5 rust review's
    "host-section schema compatibility claim is unsafe" finding;
    rendered as a fenced `text` block instead of `json` to make
    explicit that the `{ ... } | null` placeholders are schema
    annotations, NOT literal JSON):

    ```text
    {
      "schema_version": 3,
      "vms": {
        "<vm-name-1>": {
          "hypervisor": <HypervisorState>,
          "virtiofsd": { "<share-tag>": <VirtiofsdState> },
          "gpu":         <GpuState | null>,
          "audio":       <AudioState | null>,
          "swtpm":       <SwtpmState | null>,
          "otel_relay":  <OtelGuestRelayState | null>
        },
        "<vm-name-2>": <same VMSection shape>
      },
      "host": {
        "otel_host_bridge": <OtelHostBridgeState | null>,
        "usbip_backend":    { "<env-name>": <UsbipBackendState> },
        "usbip_proxy":      { "<env-name>": <UsbipProxyState> }
      }
    }
    ```

    The angle-bracketed placeholders (`<HypervisorState>`,
    `<GpuState | null>`, etc.) are **custom schematic notation**
    used in this ADR to describe the shape — they are NOT
    JSON-Schema `$ref` / `anyOf` syntax. The canonical
    JSON-Schema artifact at
    `docs/reference/cli-output/status.schema.json` uses standard
    `$ref` for type references and `oneOf: [{type: object}, {type: null}]`
    for nullable types per JSON Schema draft-07 conventions. The
    text-block format here is a human-readable summary; the
    schema file is authoritative.

    **Schema artifact path** (resolves the R6 + R7 rust reviews):
    v1.1-P10 introduces `StatusOutputV3` as the new Rust DTO
    name (NOT a reuse of `StatusServicesOutputV2`); the
    canonical JSON schema regenerates to
    `docs/reference/cli-output/status.schema.json` (same path,
    new content) AND the schema's `$id` URI bumps to
    `urn:nixling:status:v3` so consumer parsers can detect the
    schema version by URI. The **schema drift gate** is the
    existing `cargo xtask gen-schemas` no-diff CI check (a
    dedicated `tests/schema-drift-eval.sh` does NOT exist at
    v1.0 HEAD — the R7 rust reviewer correctly flagged the
    prior draft's reference to a non-existent file); the drift
    check is invoked as part of the standard `cargo xtask test`
    flow. The full `StatusOutputV3` Rust source path:
    `packages/nixling/src/status_v3.rs` (new file in v1.1-P10,
    following the `packages/nixling/src/` flat module convention
    — see `ls packages/nixling/src/`). At v1.0 HEAD the
    `StatusServicesOutputV2` struct lives inline in `lib.rs`
    around lines 99-114; v1.1-P10 extracts it to a new
    `packages/nixling/src/status_v2_compat.rs` file. To
    preserve the existing `lib.rs::StatusServicesOutputV2`
    import path (which is referenced from
    `packages/nixling/src/lib.rs` test modules + other call
    sites), `lib.rs` adds `pub use status_v2_compat::StatusServicesOutputV2;`
    so the type is still reachable under both
    `nixling::StatusServicesOutputV2` (compat alias) AND
    `nixling::status_v2_compat::StatusServicesOutputV2`
    (canonical new path). No call-site rewrites needed (R8
    rust medium).

    **Dual-output CLI surface** (resolves R6 + R7 + R8 rust
    reviews). v1.1-P10 introduces a `--status-schema-version=2|3`
    CLI flag on `nixling status`. **Clap placement spec**:
    the flag is declared on the existing `StatusArgs` struct
    in `packages/nixling/src/lib.rs` (the `Args` body for the
    `status` subcommand), NOT as a top-level CLI flag:

    ```rust
    // packages/nixling/src/lib.rs (v1.1-P10 addition to StatusArgs)
    #[derive(clap::Args, Debug, Clone)]
    pub struct StatusArgs {
        // ... existing v1.0 fields ...
        /// Schema version for `--json` output. Default 3 (v1.1+
        /// shape). 2 selects the v1.0-compatible
        /// `StatusServicesOutputV2` rendering; only valid for
        /// one release-cycle (v1.2 removes the flag and
        /// `status_v2_compat.rs`).
        #[arg(long, value_parser = clap::value_parser!(u8).range(2..=3), default_value = "3")]
        pub status_schema_version: u8,
    }
    ```

    The flag is StatusArgs-scoped (not top-level) so it does
    not pollute other subcommand surfaces. The selected version
    is carried into `nixlingd` via the existing daemon-api
    `StatusRequest` envelope as a new `schema_version: u8`
    field (v1.1-P10 daemon-api update):

    ```rust
    // packages/nixling-contracts/src/public_wire.rs (v1.1-P10 addition)
    //
    // Derive set matches existing public-wire DTO convention:
    // Debug + Clone + PartialEq + Eq + serde::Serialize +
    // serde::Deserialize + schemars::JsonSchema (the public-wire
    // trait bounds documented in v1.0 baseline).
    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct StatusRequest {
        // ... existing fields ...
        /// Schema version requested by the client. Defaults to 3
        /// for v1.1; only 2 or 3 are accepted (other values are
        /// rejected by the public-wire handler with a typed
        /// `status-schema-version-out-of-range` envelope before
        /// the request reaches the renderer).
        #[serde(default = "default_status_schema_version")]
        pub schema_version: u8,
    }

    fn default_status_schema_version() -> u8 { 3 }
    ```

    The `#[serde(default = ...)]` attribute is required for
    backward compatibility: v1.0 clients (and any other client
    that omits the field) deserialize successfully and the
    server-side handler sees `schema_version == 3`. The R10
    rust reviewer flagged the prior draft (no default) as
    breaking existing request compatibility — without the
    serde default, deserialization of a v1.0-shaped
    `StatusRequest` (with no `schema_version` field) would
    fail with a `missing field` error.

    Range validation lives in the public-wire handler (NOT in
    the deserializer itself; serde-side validation is awkward
    for value-range checks and rejecting via a typed
    `RequestValidationFailed` envelope is operator-friendlier
    than a serde error). The handler asserts
    `schema_version >= 2 && schema_version <= 3` immediately
    after deserialization and emits a typed
    `status-schema-version-out-of-range` envelope (exit 31
    `#broker-validation-failed` per
    [`error-codes.md`](../reference/error-codes.md)) otherwise.

    `--status-schema-version=2` selects the v1.0-compatible
    `StatusServicesOutputV2` rendering for one release-cycle of
    compatibility; v1.2 removes both the flag AND the
    `status_v2_compat.rs` file AND the daemon-api
    `schema_version` field. The flag is documented in
    `cli-contract.md` under the `status` verb section as part
    of v1.1-P10. **One-time deprecation warning** (resolves R8
    product minor): when `--status-schema-version` is omitted
    (default 3), v1.1.0 emits a one-time stderr-only deprecation
    warning the FIRST time `nixling status` runs in a shell
    session, pointing operators at the v2 compatibility flag.
    The warning state is tracked via a marker file at
    `~/.cache/nixling/.status-schema-v1.1-warned`; the warning
    fires once per user per host until v1.2 removes the flag.
    Schema goldens cover BOTH outputs:
    `tests/golden/cli-output/status-v2-{human,json}.golden`
    (preserved from v1.0) AND
    `tests/golden/cli-output/status-v3-{human,json}.golden`
    (new in v1.1-P10).

    **Schema-bump impact** (resolves the R5 rust review): the
    v1.1 shape is a **breaking schema change** from v1.0's
    `StatusServicesOutputV2` (which had no `vms` / `host` outer
    keys; the output was per-VM rendered without an enclosing
    object). The schema version bumps from `2` to `3` in
    v1.1-P10; consumer JSON parsers that strict-deny unknown
    fields will need to upgrade their consumer-side schema in
    lockstep. The `schema_version` field is added at the JSON
    root so strict consumers can fail-fast on a mismatch. The
    v1.1 migration guide documents the consumer-flake
    schema-version pinning recipe.
  - **New** `usbip_backend: BTreeMap<EnvName, UsbipBackendState>`
    + `usbip_proxy: BTreeMap<EnvName, UsbipProxyState>` (per-env,
    host section). `EnvName` is a `String` keyed by the env name
    declared in `nixling.environments.<env>`. Empty map if no
    env has USBIP enabled.
  - **Per-attach USBIP state** (`UsbipBindOneShot`, etc.) is
    transient and is NOT in `StatusServicesOutputV2`. The
    `nixling usb status` CLI surface (NOT `nixling status`)
    enumerates active per-attach sessions via a separate daemon
    RPC; the schema is owned by the USBIP component reference
    doc, not by the status-output schema.
- Replace every `systemctl_state(...)` call in `vm_service_states`
  with a single daemon RPC `daemon-api/status QueryVmSpawnerSessions{vm}`
  that returns the active SpawnRunner-session state for every role.
- Update the human-form `nixling status` rendering to read
  e.g. "hypervisor: running (broker-spawn)" instead of
  "microvm@<vm> (backend): {state}"; emit one line per active
  SpawnRunner role.
- Regenerate the schema (`cargo xtask gen-schemas`) and update the
  status-output golden fixtures under `tests/golden/cli-output/`.
- The schema bump is field-additive + field-rename at the
  JSON-schema level and the human-form change is operator-visible;
  both are documented in the v1.1 migration guide.

### Audit-baseline-exception structured allowlist

The role-baseline OpAuditRecord matrix defines the **minimum**
audit events every SpawnRunner role MUST emit. Roles that
legitimately do not emit a baseline kind (e.g., `SwtpmFlush` is
one-shot and never emits `Restarted`) MUST register the exception
in a structured allowlist file rather than relying on prose
rationale alone. The allowlist contract supports BOTH per-role
exceptions AND **predicate-based** exceptions (for categories of
roles that share a structural omission, e.g., all one-shot
roles never emit `Restarted` — a predicate avoids a per-role
entry explosion):

- Location: `tests/fixtures/broker-spawn-audit-baseline-exceptions.yaml`
  (NOTE: this fixture file does NOT exist at v1.0 HEAD `eed0894`;
  it is created by v1.1-P10 alongside the role implementations.
  ADR 0018 + components-usbip.md cite the path as `future, v1.1-P10`.)
- **Implementation contract** (resolves R7 security minor): the
  validator MUST use `serde_yaml` deserialization into typed
  Rust structs annotated with `#[serde(deny_unknown_fields)]` on
  every struct in the hierarchy, AND a custom `Deserializer`
  helper that enforces the anchored regex patterns inline (not
  post-parse). The implementation lives in
  `tests/tools/baseline-exception-validator/` (a small dev-only
  Rust binary, panel-reviewed at commit time per the
  no-bash-ast-walker pattern). YAML parsers that accept
  trailing-junk strings without `deny_unknown_fields` are
  forbidden. The validator runs as part of
  `tests/broker-spawn-audit-parity-eval.sh` setup, before any
  audit-event assertion.
- **Schema** (YAML; v1.1-P10 implementation MUST validate against
  this JSON-Schema-equivalent contract before the gate runs). The
  validator is strict — `additionalProperties: false` on every
  object (enforced via `#[serde(deny_unknown_fields)]`), anchored
  regexes (enforced via the custom Deserializer helper above),
  no string-with-trailing-junk acceptance:

  ```yaml
  # tests/fixtures/broker-spawn-audit-baseline-exceptions.yaml
  #
  # JSON-Schema-equivalent contract (no unknown fields anywhere,
  # all regexes anchored ^...$):
  #
  #   exceptions: array of entry. Each entry has EXACTLY ONE
  #   of the following two scoping shapes:
  #
  #   (A) per-role entry:
  #     role: string enum [Hypervisor, Virtiofsd, SwtpmFlush, Swtpm,
  #           Gpu, Video, Audio, OtelGuestRelay, OtelHostBridge,
  #           UsbipBackend, UsbipProxy, UsbipBindOneShot,
  #           UsbipUnbindOneShot, GuestUsbipAttachOneShot,
  #           GuestUsbipDetachOneShot]
  #
  #   (B) predicate entry:
  #     applies_to:
  #       lifecycle: string enum [long_running, one_shot, pre_launch_hook]
  #       OR
  #       role_glob: string matching ^[A-Z][A-Za-z]*(\*[A-Za-z]*)?$
  #              (anchored single-glob form; pattern MUST start
  #              with `[A-Z]` and MAY contain at most ONE `*`
  #              glob wildcard that matches zero-or-more
  #              `[A-Za-z]` chars. Examples that MATCH:
  #              "Usbip*OneShot" matches `UsbipBindOneShot` /
  #              `UsbipUnbindOneShot` (host-side, the prefix
  #              `Usbip` + glob `*` + suffix `OneShot`);
  #              "GuestUsbip*OneShot" matches
  #              `GuestUsbipAttachOneShot` /
  #              `GuestUsbipDetachOneShot` (the prefix
  #              `GuestUsbip` + glob `*` + suffix `OneShot`).
  #              EXACT-name entries are also valid (no `*`
  #              wildcard): e.g. `role_glob: "SwtpmFlush"`
  #              matches only `SwtpmFlush`. EXAMPLES THAT DO
  #              NOT MATCH the regex (the validator REJECTS
  #              with `role_glob-regex-mismatch` diagnostic
  #              per the "Validator diagnostic shape"
  #              subsection below): "*UsbipAttachOneShot"
  #              (starts with `*`, not `[A-Z]`); "Usbip*Bind*OneShot"
  #              (more than one `*`); "usbipBindOneShot"
  #              (lowercase first char). For roles requiring
  #              wildcard coverage that the anchored single-glob
  #              cannot express (e.g. matching ALL of
  #              `UsbipBindOneShot` + `UsbipUnbindOneShot` +
  #              `GuestUsbipAttachOneShot` + `GuestUsbipDetachOneShot`
  #              in one entry), use multiple per-role
  #              entries — predicates with `lifecycle: one_shot`
  #              also work for the broader category.)
  #
  #   **Predicate-omitted-kinds restriction (resolves R6 virt finding):**
  #   When `applies_to.lifecycle == one_shot`, the `omitted_kinds`
  #   field MAY contain ONLY the following structurally-inapplicable
  #   kinds:
  #     [Restarted, LivenessProbeStarted, LivenessProbeOk,
  #      LivenessProbeWedged, WedgeRestarted]
  #   The validator REJECTS predicate entries that omit any of the
  #   security-load-bearing kinds:
  #     [SpawnRequested, SpawnSucceeded, SpawnFailed, ChildExited,
  #      ChildSignalled, PreLaunchHookStarted, PreLaunchHookSucceeded,
  #      PreLaunchHookFailed]
  #   Per-role entries (shape (A)) MAY omit ANY baseline kind with
  #   panel-approved adr_ref + owner_discipline; predicate-scoped
  #   omissions are restricted because they apply to a category of
  #   roles and a category-level omission of a security-load-bearing
  #   kind is too broad.
  #
  #   Both shapes require:
  #     omitted_kinds: non-empty array of string enum from the role-baseline list
  #     rationale: string, minLength 50
  #     adr_ref: string matching ^(ADR\s+\d{4}|panel-r\d+-(rust|virt|kernel|security|networking|software|test|product|docs)-r\d+)$
  #     owner_discipline: string enum [rust, virt, kernel, security, networking, software, test, product, docs]
  #     expires_at: string matching ^(never|\d{4}-\d{2}-\d{2})$
  #
  #   **Validator diagnostic shape (resolves R6 security minor):**
  #   On validation failure the validator emits to stderr:
  #     exceptions[<index>]: <error-message>
  #   where <error-message> is one of:
  #     - "exactly one of `role` or `applies_to` must be set"
  #       (both set, or neither set)
  #     - "predicate-scope omitted_kind `<kind>` is not allowed
  #        for lifecycle=one_shot; only [Restarted, LivenessProbe*,
  #        WedgeRestarted] may be omitted"
  #     - "role_glob-regex-mismatch: role_glob `<value>` does not
  #        match ^[A-Z][A-Za-z]*(\\*[A-Za-z]*)?$ (anchored
  #        single-glob form; see schema-comment above for
  #        matching/non-matching examples)"
  #     - "role-name-unknown: role `<value>` is not in the
  #        v1.1 RunnerRole inventory; see the "Disposition matrix"
  #        section in this ADR"
  #     - "adr_ref `<value>` does not match ^(ADR\\s+\\d{4}|...)$"
  #     - "owner_discipline `<value>` is not one of the 9 panel
  #        disciplines"
  #     - "expires_at `<value>` is not `never` or YYYY-MM-DD"
  #     - "unknown field `<name>` in exception entry"
  #     - "duplicate entry: role `<role>` already declared at
  #        exceptions[<other-index>]"
  #     - "empty predicate: applies_to has neither lifecycle nor
  #        role_glob set"
  #   Each diagnostic includes the YAML entry index AND a remediation
  #   pointer to this ADR section.

  exceptions:
    # Per-role example:
    - role: SwtpmFlush
      omitted_kinds:
        - Restarted
        - LivenessProbeStarted
        - LivenessProbeOk
        - LivenessProbeWedged
        - WedgeRestarted
      rationale: |
        SwtpmFlush is a one-shot state-migration runner that
        does not survive past its initial success. Restart and
        liveness-probe semantics are not applicable.
      adr_ref: ADR 0018
      owner_discipline: virt
      expires_at: never

    # Predicate example (covers UsbipBindOneShot,
    # UsbipUnbindOneShot, GuestUsbipAttachOneShot, GuestUsbipDetachOneShot
    # without enumerating each):
    - applies_to:
        lifecycle: one_shot
      omitted_kinds:
        - Restarted
      rationale: |
        One-shot SpawnRunner leaves exit after a single successful
        run; broker does NOT restart them by definition. The
        Restarted audit kind is structurally inapplicable.
      adr_ref: ADR 0018
      owner_discipline: virt
      expires_at: never
  ```

  The `owner_discipline` field maps to one of the 9 panel
  disciplines and assigns review accountability: when an
  exception is added or modified, the named discipline's
  reviewer must explicitly sign off as part of the round in
  which the change lands. The validator rejects entries that
  set BOTH `role` AND `applies_to`, AND entries that set
  NEITHER, AND entries whose `adr_ref` does not match the
  anchored regex above, AND entries whose `owner_discipline`
  is not one of the 9 enumerated values.
- `tests/broker-spawn-audit-parity-eval.sh` reads ONLY this file
  as the exception source; ad-hoc inline exceptions in test code
  are forbidden. A new exception lands by editing the file in
  the same commit that adds the role; the file is itself a panel
  review surface (every diff to it triggers a full 9-discipline
  re-review of the affected role).

## Alternatives considered

### A1. Keep `microvm.nix` as a "vendored" pinned input

Pin `microvm.nix` to a known-good rev and treat it as an internal
dependency we choose not to update.

**Rejected** because:
- Pinning does not eliminate the audit surface; every evaluated
  module remains in nixling's NixOS module set.
- Pinning does not eliminate the substrate friction; nixling still
  has to keep its read paths compatible with the pinned upstream
  shape.
- Pinning carries a long-tail maintenance cost (CVE backports,
  nixpkgs version skew) for no nixling-side benefit.

### A2. Replace `microvm.nix` with a different upstream microVM
NixOS module (e.g. fork of microvm.nix)

**Rejected** because:
- Substrate friction transfers to the new upstream; the structural
  problem (per-VM systemd templates, upstream `microvms.target`)
  is shared across the ecosystem.
- A fork is even more maintenance burden than the upstream pin.

### A3. Re-export an internal `microvm.nix`-shaped option tree
for consumer compat

Keep the `microvm.*` namespace as a deprecated alias that maps to
the new `nixling.vms.<vm>.runner.*` tree.

**Rejected** because:
- No consumer flake under nixling's control reads `microvm.*` —
  the option tree is internal to nixling-the-framework.
- Aliasing adds duplicate-option-source confusion and complicates
  the eval gate that asserts no `microvm.*` remains.

### A4. Defer to v1.2

Ship v1.1 without the microvm.nix removal.

**Rejected** because:
- The user explicitly required no `microvm.nix` dependency for
  v1.1.
- The deferred surfaces in CHANGELOG v1.0 ("microvm@<vm>.service",
  "microvm-virtiofsd@<vm>.service", "store-sync") all hang off
  the microvm.nix import; partial retirement leaves the
  surface area unstable.

## Consequences

### Positive

- **Single source of truth.** Per-VM contract lives entirely under
  `nixling.vms.<vm>.*`. Reviewers do not chase config across two
  option namespaces.
- **Tagline accuracy.** nixling stops claiming to be "on microvm.nix"
  when its v1.0 architecture already owns every spawn path.
- **Audit surface shrinkage.** Removing `inputs.microvm.nixosModules.host`
  removes ~30 upstream submodules from nixling's evaluated module
  set.
- **Looser coupling to nixpkgs cadence.** No `microvm.nix` flake
  input means `nix flake update` no longer pulls upstream microvm
  rev bumps that change runner argv shape.
- **Smaller flake.lock.** One fewer top-level node, plus its
  transitive deps that nixling does not consume.

### Negative

- **Internal Nix code volume grows.** `vm-submodule.nix` is
  non-trivial; it re-implements the relevant upstream per-VM module
  evaluation. The migration tests guard against drift but the code
  itself is now nixling's to maintain.
- **Lifecycle audit-event surface must be reproduced in broker
  records.** Retired systemd units provided journal-level evidence
  for unit start/stop/exit/restart. Replacing them requires that
  the broker emit the equivalent `OpAuditRecord` kinds enumerated
  in the SpawnRunner role-matrix table (`SpawnRequested`,
  `SpawnSucceeded`, `SpawnFailed`, `ChildSignalled`, `ChildExited`,
  `Restarted`, `PreLaunchHookStarted/Succeeded/Failed`,
  `LivenessProbeStarted/Ok/Wedged`, `WedgeRestarted`). Test
  `tests/broker-spawn-audit-parity-eval.sh` (future, v1.1-P10)
  asserts that for every retired unit, the broker emits an audit
  record covering each of the listed lifecycle events under a
  fault-injection probe.
- **One-time consumer-flake disruption.** Any consumer (none known
  in-tree) that overrode `microvm.vms.<vm>.config.*` directly must
  migrate to `nixling.vms.<vm>.runner.*`. The v1.1 migration guide
  (`docs/how-to/migrate-nixling-v1-0-to-v1-1.md`, future, v1.1-P12)
  reproduces the full migration table above and lists every
  retired upstream option with its nixling-owned counterpart.
- **Lock-in to nixling-owned per-VM model.** Diverging from
  ecosystem conventions costs interop with other microvm-tooling
  (none currently consumed); the v1.0 daemon-only architecture
  already requires that divergence.

### Neutral

- **ADR 0001 / ADR 0015 status unchanged.** This ADR extends their
  decisions to cover the substrate ownership; it does not
  contradict either of them. **ADR 0004 status updated.** The
  "declaredRunner as independent compatibility oracle" framing in
  ADR 0004 is superseded by the snapshot-vs-frozen-fixture parity
  approach documented in § Per-VM NixOS evaluation above; the
  `declaredRunner` name is preserved for source-stability but its
  role is now "production runner descriptor", not "oracle". ADR
  0004's verification text should be marked **superseded by ADR
  0018 in v1.1** in a v1.1-P12 docs-polish edit.

## Verification

Verification depends on a suite of new eval gates landed across
v1.1-P8 → P11 (file names are *future* until each phase ships):

- `tests/processes-json-eval.sh` (future, v1.1-P8) compares the
  `bundle.json` produced by the v1.0 `microvm.*` reads against the
  v1.1 `nixling.vms.*` reads on the same example bundle. The
  comparison strategy is **canonical-JSON normalization**, not
  byte-for-byte raw diff:
  - Load both outputs; sort object keys recursively.
  - Apply path-level normalization to every value matching a Nix
    store path. The **normative regex** is
    `^/nix/store/(?P<hash>[0-9a-df-np-sv-z]{32})-(?P<name>[^/]+)(?P<rest>/.*)?$`
    — `<hash>` is exactly 32 characters from the Nix base32
    alphabet (the v1.1 test reviewer noted that Nix base32
    EXCLUDES `e`, `o`, `t`, `u` to avoid case-confusion glyphs,
    so the alphabet is `0-9a-df-np-sv-z`; the over-permissive
    `[0-9a-z]` from earlier drafts is replaced with the
    strict alphabet here), immediately followed by `-`, then
    `<name>` extending to the next `/` or end of string, then
    optional `<rest>` starting with `/`. Replacement: `<hash>` is
    replaced with the literal string `<HASH>`, `<name>` and
    `<rest>` are preserved. Two paths that differ only in `<hash>`
    compare equal; a path whose basename `<name>` changes (e.g.,
    a different derivation name) FAILS the diff. Strings that do
    not match the regex (i.e., are not Nix store paths) are
    passed through unchanged. Fixture pair under
    `tests/fixtures/processes-json-eval/` exercises both cases
    (hash-only drift → pass; name drift → fail) AND a third
    fixture for a string that contains `[0-9a-z]{32}` but with
    `e`/`o`/`t`/`u` in the 32-char window (should NOT match the
    regex; should pass through unchanged — guards against
    over-normalization of non-Nix-store paths that happen to
    look hash-like).
  - Apply field-level redaction for fully-volatile fields
    enumerated in `tests/fixtures/bundle-json-volatile-fields.json`
    (e.g. randomly-allocated socket suffixes, per-build
    timestamps). Redacted fields are separately asserted as
    "present and well-formed but value not compared".
  - **Path-level and field-level normalization are orthogonal.**
    Nix store-path normalization (regex above) applies to ANY
    string value matching the store-path shape; field-level
    redaction (volatile-fields.json) applies to specific JSON
    pointers regardless of value shape. Runtime paths with
    volatile suffixes that are NOT Nix store paths (e.g.,
    `/var/lib/nixling/runtime/<vm>/socket-12345.sock`) MUST be
    listed in `bundle-json-volatile-fields.json` by JSON pointer
    — the store-path regex MUST NOT be relaxed to match them.
    The fixture set includes a negative test (a non-store path
    with a volatile suffix) that the store-path regex correctly
    leaves unchanged.
  - Assert structural equality on the normalized output.

  The gate is removed after v1.1-P11 (no v1.0 path left); the
  toplevel-hash stability gate below replaces it.

- `tests/vm-submodule-eval.sh` (future, v1.1-P9a) asserts every
  example VM produces the **same** `system.build.toplevel`
  derivation hash via the new `vm-submodule.nix` path as the v1.0
  `microvm.vms` path on the same input. Hash equality is the
  strongest gate the substrate can offer; if the new evaluator
  introduces any semantically-neutral derivation drift
  (attribute-ordering, `lib.mkOverride` priority shifts,
  module-provenance metadata), the gate FAILS by default. The
  v1.1-P9a work must either eliminate the drift or land an
  explicit **per-attribute structural-equivalence fallback** that
  compares the following NixOS config slice (the canonical set
  the v1.0 → v1.1 substrate replacement is allowed to drift on
  IFF the drift is semantically neutral):
  - `config.system.build.toplevel.outPath` (basename component
    only — derivation hash drift is the expected source of
    needing the fallback in the first place).
  - `config.system.build.extraDependencies`.
  - `config.systemd.services.<name>.serviceConfig` for every
    enabled service.
  - `config.systemd.units` (full unit text after the existing
    NixOS unit renderer).
  - `config.boot.kernelPackages.kernel.outPath` (basename) +
    `config.boot.kernelParams` + `config.boot.initrd.kernelModules`
    + `config.boot.initrd.extraFiles`.
  - `config.system.activationScripts.<name>.text` for every
    activation step.
  - `config.systemd.tmpfiles.rules` (sorted).
  - `config.users.users` + `config.users.groups` (filtered to
    nixling-relevant entries; system-default users are not
    compared).
  - `config.fileSystems`.
  - `config.environment.etc.<path>.source` + `.text` + `.mode`
    for every nixling-emitted entry.
  - Every `nixling.vms.<vm>.runner.*` option's final
    `config.<runner-path>` value (the runner-adjacent options
    feeding `processes-json.nix`).
  Path-level normalization (see `processes-json-eval.sh` above)
  applies to every value in the slice that holds a Nix store
  path. If any slice attribute drifts beyond the normalization,
  the phase is blocked until the drift is eliminated or
  panel-justified.

- `tests/kernel-modules-parity-eval.sh` (future, v1.1-P9a, per
  [ADR 0014](0014-w3-modules-devices-runner-shape.md)) — see the
  full contract in the "Kernel modules + `modules_disabled`
  parity" subsection above. Compares set-equality on module
  lists AND per-module attribute equality on requirement class,
  load-fail-if-locked policy, and sysctl associations.

- `tests/otel-acl-migration-eval.sh` (future, v1.1-P6) asserts the
  retired `host-otel-relay-acl.nix` ACL grants are reproduced by
  broker pre-spawn operations (or SCM_RIGHTS handoff) per the
  ACL migration table above, including the parent-dir traversal
  rows.

- `tests/broker-spawn-audit-parity-eval.sh` (future, v1.1-P10)
  asserts every retired-unit lifecycle event is covered by an
  equivalent `OpAuditRecord` per the role-baseline + role-matrix
  tables.

- `packages/nixlingd/tests/booted_notify_race.rs` (future, v1.1-P10)
  asserts the set-booted race-free serialization contract per
  the spec above.

- `tests/microvm-nix-absent-eval.sh` (future, v1.1-P11) asserts
  `inputs ? microvm` is `false` in `flake.nix` after the input is
  dropped.

- `tests/runner-shape-snapshot.sh` (existing; **extended** in
  v1.1-P9a) — at HEAD `00b24c5` the script diffs only 2 of the
  12 fixtures under `tests/golden/runner-shape/`. v1.1-P9a
  extends the script to diff every fixture (`audio`, `gpu`,
  `otel-host-bridge`, `swtpm`, `usbip`, `video`,
  `virtgpu-ioctl-values`, `virtiofsd`, `vsock-relay` in
  addition to the existing 2). This replaces the ADR 0004
  declaredRunner-as-oracle role.

- `nix flake check` succeeds on the v1.1 HEAD.
- `nix flake metadata --json | jq '.locks.nodes | keys'` does not
  contain `microvm`.
- All smoke evals (`smoke-eval`, `smoke-eval-graphics`,
  `smoke-eval-tpm`, `smoke-eval-aarch64`,
  `smoke-eval-home-manager`, `smoke-eval-extraspecialargs`)
  return the same 54 attrs as v1.0 on the same example bundle.
- Live re-validation: `nixos-rebuild switch` on the user's host
  produces no toplevel-hash drift for `personal-dev` or `work-aad`.
