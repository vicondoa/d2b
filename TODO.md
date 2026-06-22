# TODO

Operator-facing follow-up work captured as we hit it. New items go at
the top. Move closed items to CHANGELOG and delete from here.

## Flake provides no `devShell` → `cargo-ubuntu` CI job and `nix develop` fail

**Symptom.** The `cargo-ubuntu` job in `.github/workflows/pr-cargo-workspace.yml`
(which runs `cd packages && nix develop --command bash -c 'cargo fmt … &&
cargo clippy … && cargo test …'`) fails immediately:

```
error: flake 'git+file:///…/nixling?shallow=1' does not provide attribute
'devShells.x86_64-linux.default', 'devShell.x86_64-linux',
'packages.x86_64-linux.default' or 'defaultPackage.x86_64-linux'
```

Any local `nix develop` in the repo root fails the same way. Observed
red on PR #31 and PR #32; not specific to either change (both have
clean Rust locally — the job never gets as far as building).

**Root cause.** `flake.nix` exposes `nixosModules`, `templates`, and
`checks`, but **no `devShells` / `devShell` output**, so the CI's
`nix develop` has nothing to enter. The Rust workspace is otherwise
built/tested directly with the pinned `rust-toolchain.toml` (see the
documented manual env: `~/.rustup/toolchains/1.94.1-*/bin` + a nix
gcc-wrapper `bin` for `cc` + `CARGO_BUILD_RUSTC_WRAPPER=''`).

**Fix options.**
1. Add `devShells.${system}.default` to `flake.nix` providing the
   pinned Rust toolchain (1.94.1), a C compiler/`cc` wrapper, and the
   codegen deps (so `cargo fmt/clippy/test` + `xtask gen-*` run inside
   `nix develop`), then keep the workflow as-is.
2. Or drop `nix develop` from `pr-cargo-workspace.yml` and invoke
   cargo via the documented manual toolchain env directly.

Option 1 is preferable: it makes `nix develop` the single source of
truth for the build env both in CI and locally, matching the cargo
workflow's existing expectation.

## Resolved: `DiskInit` validates existing images before skipping

Issue #102 fixed the existence-only `ifAbsent` behavior for broker
`DiskInit`. Existing nixling-owned raw ext4 images are now opened with
`O_NOFOLLOW`, locked, checked for the declared size/owner/mode, and
validated by ext4 superblock magic before they are skipped. A present
but unformatted image is repaired only when kernel extent metadata
proves it is empty; non-empty, ambiguous, or wrongly-postured files fail
closed before the VM runner spawns.

## Speed up the `assertions-eval` gate by folding probe cases into the batch

`tests/assertions-eval.sh` now evaluates its 26-case batch via a
minimal `lib.evalModules` (nixling modules + `nixos/modules/misc/assertions.nix`
+ namespace sinks in `tests/unit/nix/eval-cases/shared.nix`) instead of a full
`nixpkgs.lib.nixosSystem` per case — the batch dropped from ~2 min to
~68 s. The remaining wall time (~9 min) is dominated by the ~9 tail
"probe" cases at the bottom of the gate, each of which spawns a
SEPARATE `nix-instantiate` whose cost is process startup
(`builtins.getFlake` + nixpkgs import), not module eval. Fold those
probes into the single batch process — extend the case schema in
`shared.nix` with an optional `probe` projection (reusing the now-safe
faithful `systemd` sink for the cases that read
`config.systemd.services` / `config.systemd.tmpfiles.rules`) so the
whole gate runs in one eval. The 3 throw-message-capture fallbacks
(`graphics-without-wayland-user`, `platform-gate-*-aarch64`) must stay
on a per-case `--show-trace` eval unless their stderr-message
assertion is relaxed (or moved to `nix-unit`'s `expectedError.msg`).
The same minimal-evalModules technique also applies to
`tests/unit/nix/eval-cases/observability.nix` and `processes-dag-order.nix`,
which still call full `nixosSystem`. This touches a critical contract
gate, so route it through panel review. Target: whole gate under
~2 min.

## `docs/reference/cli-output/status.schema.json` is stale (missing `api_ready` defs)

`cargo run -p xtask -- gen-cli-schemas` emits `ApiReadySimple` /
`ApiReadyStatusV1` definitions (from the `api_ready` field on
`StatusRequest`/status response in `packages/nixling-ipc/src/public_wire.rs`,
shipped in v1.2), but the committed `status.schema.json` predates them
and was never regenerated. No drift gate enforces this file
(`cli-json-drift.sh` only checks host-check rendering), so CI is green
despite the staleness. Regenerate and commit the schema, and consider
wiring a `cli-schemas` drift gate so it can't silently drift again.

## `privileges-doc-completeness-eval.sh` reports 10 contradictory doc rows

`tests/privileges-doc-completeness-eval.sh` fails with 10
"has a live (unmarked) doc row AND an obituary row — contradictory"
violations for retired units (`microvm-tap-interfaces@`,
`microvm-set-booted@`, `microvm-pci-devices@`, `nixling-*-store-sync`,
`nixling-known-hosts-refresh@`, `nixling-vfsd-watchdog@`,
`nixling-ch-exporter`, `nixling-otel-host-bridge`, per-env
`usbipd-proxy`/`usbipd-backend`). This is pre-existing (fails
identically on the original tree, independent of the marker scrub):
`docs/reference/privileges.md` lists these units both in a live table
and in the `## Legacy systemd surface obituary` section. Reconcile the
doc so each retired unit appears only in the obituary, or adjust the
gate's live/obituary partition logic. The gate is wired into
`tests/static.sh`.

## `nixling usb attach` never performs the guest-side `usbip attach` (vhci import unwired)

**Current state.** `nixling usb attach <vm> <busid> --apply` does only the
HOST side: `UsbipBind` (bind + per-busid lock), `UsbipBindFirewallRule`,
ensures the per-env `usbipd` backend + proxy are up, and
`UsbipProxyReconcile`. It then returns "bound … via the native daemon →
broker path". The **guest-side `usbip attach -r <usbipdHostIp> -b
<busid>`** that actually materialises the device inside the VM is never
issued.

**Evidence.** `packages/nixling-host/src/usbip_argv.rs` has a complete
`generate_guest_usbip_ssh_argv` (ssh-hardened `usbip attach/detach`
driver) — but `grep` shows it is **never called** anywhere outside its
own module/tests. `dispatch_broker_usbip_bind` in
`packages/nixlingd/src/lib.rs` ends after `UsbipProxyReconcile`. So
after `nixling usb attach`, `usbip port` in the guest is empty and no
`/dev/...` node appears until the operator manually `usbip attach`es
inside the VM. The v1.1 design's `GuestUsbipAttachOneShot` /
`GuestUsbipDetachOneShot` SpawnRunner roles (see ADR 0018) were
specified but not implemented.

**Impact.** USB passthrough is half-wired: the documented one-command
flow (`nixling usb attach`) silently leaves the device unusable in the
guest. Every consumer must hand-roll a guest-side import. Confirmed
2026-06-07 with the Openterface KVM in `work-ssd` — host bind succeeded,
`usbip probe` showed `bound`, but the guest had no device until a manual
in-VM `usbip attach`. Worked around with a temporary guest-side poller
systemd unit (`openterface-usbip-import` in `/etc/nixos/vms/work-ssd.nix`)
that attaches the declared busids from the per-env proxy.

**Right answer.** Wire the guest-side attach into the daemon so
`nixling usb attach --apply` is truly one-shot:
- After the host bind + proxy are ready, dispatch a guest-side
  `usbip attach -r <usbipdHostIp> -b <busid>` over the VM's
  nixling-managed SSH (the `generate_guest_usbip_ssh_argv` builder
  already exists), as the design's `GuestUsbipAttachOneShot`.
- The guest command needs root: either run as the VM's ssh user with a
  focused NOPASSWD sudoers entry for `usbip attach/detach`, or have the
  daemon target a dedicated guest principal. Don't require the whole VM
  to set `sudo = true`.
- Mirror it on `nixling usb detach` (`usbip detach -p <port>`, tracking
  the port the guest attach assigned) so teardown is also one-shot.
- Make both idempotent (re-attach/-detach is a no-op) and surface a
  clear error if the guest SSH or in-guest `usbip` fails, instead of
  reporting success after only the host half.
- Drop the per-consumer guest poller once this lands (remove
  `openterface-usbip-import` from `work-ssd.nix`).

## Guest Wayland-EGL apps fall back to llvmpipe through the cross-domain proxy

**Current state.** The guest-side proxy is `wl-cross-domain-proxy`.
Whether it advertises dmabuf feedback tied to the virtio-gpu render node
identity (and thus whether in-guest Wayland-EGL binds virgl rather than
llvmpipe) has not yet been confirmed on a live VM with the current proxy
chain. The host-side `nixling-wayland-filter` passes `linux_dmabuf_v1` by
default (not denied in the secure preset) so the host compositor's dmabuf
globals are available at the filter socket. The open question is whether
the guest cross-domain proxy correctly presents the virtio-gpu device
identity to the in-guest EGL platform.

## `nixling switch <vm>` fails with `broker-error` (RunActivation intent not found)

**Current state.** `nixling switch <vm> --apply` is documented as the
VM-only live-activation path (build new closure → sync to per-VM store
→ SSH in and `switch-to-configuration switch`). In practice it fails.

**Observed failures (2026-06-07):**
- As root via `sudo nixling switch sys-work-net --apply`:
  `authz-not-a-launcher: peer uid 0 is not in
  nixling.site.launcherUsers`. (sudo runs as uid 0, which is not a
  launcher — so `nixling switch` must NOT be run under sudo, but
  nothing in the CLI/help says so.)
- As the launcher user `paydro`:
  `RunActivation failed (code: broker-error, exit 78) … The daemon
  reached the broker for 'nixling switch --apply', but the broker
  refused or failed the request (target wave hint: W12). RunActivation
  references a bundle intent that the broker did not find.` This
  happened even though the declared toplevel was built **and** present
  in the per-VM store (`/var/lib/nixling/vms/<vm>/store/<toplevel>`).

**Impact.** The advertised fast path for VM-only changes is unusable.
The working alternative is the slow path: `nixos-rebuild switch`
(re-syncs every per-VM store + rebuilds the bundle) followed by
`nixling restart <vm> --apply` (clean down+up on the new closure).
`nixling restart` works reliably; `nixling switch` does not.

**Right answer.**
1. Fix the `RunActivation` bundle-intent resolution so a freshly
   built+synced per-VM closure is actually found by the broker (the
   intent id the daemon sends must match what the broker resolves from
   the current bundle — likely a stale-bundle / intent-id-derivation
   mismatch; `nixling audit --strict` was suggested as the way to dump
   the intent id, wire that into the error remediation).
2. Until fixed, make `nixling switch` fail fast with a clear message
   when run as a non-launcher (uid 0), and document that the reliable
   path for VM-only changes today is `nixos-rebuild switch` +
   `nixling restart <vm>`.

## Adding/removing a VM does not auto-update the per-env `sys-*-net` (and other sys) VMs

**Current state.** Declaring a new workload VM in an existing env
(e.g. `work-ssd` in `work`) regenerates the env's `sys-<env>-net`
closure — its dnsmasq config gains the new `dhcp-host=<mac>,<ip>,<name>`
reservation. But a `nixos-rebuild switch` reports
`nixling-store-sync: sys-work-net already at generation N; nothing to
do` and does **not** push the new closure into the **running**
net VM. The running `sys-<env>-net` keeps serving its old dnsmasq
config with no reservation for the new VM.

**Symptom.** The new VM boots (cloud-hypervisor + all sidecars reach
process-alive), its tap attaches to `br-<env>-lan`, but it never gets
its reserved DHCP lease, so it never reaches the network. nixling's
`guest-ssh-readiness` node times out at the deadline and the whole VM
start **rolls back**. From the operator's seat the new VM "just won't
start" with no obvious cause — the real fault is a stale sibling
`sys-<env>-net`. Reproduced 2026-06-07 adding `work-ssd` to the `work`
env: `sys-work-net` stayed on closure `076c5f4…` (pre-change) while the
new closure carried `dhcp-host=02:76:53:AE:57:14,10.20.0.20,work-ssd`.

**Workaround used.** Manually `nixling switch sys-<env>-net --apply`
after adding the VM, to push the new dnsmasq config live, then start
the new VM.

**Right answer.** Adding/removing a workload VM in an env should
automatically reconcile that env's auto-declared `sys-*` VMs whose
closure is a function of the env membership — at minimum
`sys-<env>-net` (dnsmasq reservations + any per-VM nft/host entries),
and any other sys VM whose config enumerates sibling VMs. Options:
1. The host activation `nixling-store-sync` + supervisor should detect
   that a running `sys-<env>-net`'s declared closure changed and either
   auto-`switch` it (preferred for net VMs — dnsmasq reload is cheap
   and non-disruptive) or at least flag it `[pending restart]` in
   `nixling vm list` / `vm status` with a clear remediation.
2. Better: make the per-VM DHCP reservation a runtime reconcile the
   net VM picks up without a full closure switch (e.g. a dnsmasq
   `dhcp-hostsfile` the host writes + `SIGHUP`), so adding a workload
   VM never requires touching the net VM's generation at all.

Until then, document in the "Adding a new VM" how-to that a
`nixling switch sys-<env>-net --apply` is required after the first
host switch for the new VM to get its lease.

## GPU/audio sidecars hardcode host `wayland-0`; breaks non-wayland-0 compositors

> **Superseded (GPU sidecar).** The GPU sidecar no longer connects to the
> host compositor socket directly. It connects to the per-VM filter socket
> at `/run/nixling-wlproxy/<vm>/wayland-0`, which is the
> `nixling-wayland-filter` proxy. The filter proxy reads the actual
> compositor socket from the broker-emitted process bundle (derived from
> `nixling.site.waylandDisplay`), so the hardcoded `wayland-0` path is no
> longer in the GPU runner's argv or environment. The `wayland-0`
> hardcoding in the minijail profiles for the GPU role is also removed.
> The audio sidecar's PipeWire socket dependency is unchanged.
>
> The `nixling.site.waylandDisplay` option was added during Wave 2 wiring
> and defaults to `"wayland-0"` for back-compat; set it to the actual
> compositor socket name (e.g. `"wayland-1"` for niri) in
> `nixling.site`.

**Original observation (2026-06-07, historic).** `nixos-modules/processes-json.nix`
hardcoded the host compositor socket name as `wayland-0` in four places — the
`gpuRunner` and `gpuRenderNodeRunner` each set
`--wayland-sock /run/user/<uid>/wayland-0` and
`WAYLAND_DISPLAY=wayland-0`. On a host whose primary compositor is NOT
on `wayland-0` (e.g. niri defaulting to `wayland-1`), the GPU sidecar
pointed at a non-existent socket. Workaround was `ln -s wayland-1
/run/user/<uid>/wayland-0` (session-runtime, lost on session end).

## Per-VM state child dirs inherit setgid → ownership-matrix drift blocks VM start

**Current state.** The per-VM state root `/var/lib/nixling/vms/<vm>/`
is created setgid (`drwxrws---`, mode `2770`). Child dirs created under
it (`host-keys`, `sshd-host-keys`, and likely `state`) inherit the
setgid bit at `mkdir` time, so they land as mode `2750` instead of the
`0o0750` the ownership matrix declares in
`packages/nixlingd/src/ownership_preflight.rs`
(`SSHD_HOST_KEYS` / `HOST_KEYS` specs) and
`nixos-modules/options-ownership-matrix.nix`.

**Symptom.** `nixling up <vm> --apply` fails closed at the
`OwnershipMatrixCheck` host-prep step with
`vm start refused: ownership-matrix drift vm=<vm>
path=/var/lib/nixling/vms/<vm>/sshd-host-keys drift_count=2` (then
`.../host-keys drift_count=1` once the first is fixed). The CLI
surfaces this only as `daemon returned unknown mutating-verb
outcome:` — the typed drift reason is buried in the nixlingd journal.
Reproduced on a freshly-declared `work-ssd` VM (2026-06-07); existing
VMs (`work-aad`) created before the regression are unaffected because
their dirs predate the setgid-inheritance path.

**Workaround used.** `chmod g-s
/var/lib/nixling/vms/<vm>/{sshd-host-keys,host-keys}` then re-run
`nixling up`. Manual, easy to miss, recurs on any new VM.

**Right answer (pick one):**
1. The activation/host-keys creation path (`host-keys.nix` +
   `nixos-modules/guest-sshd-host-keys.nix` + the host-activation
   helper) should `chmod` each per-VM child dir to its exact matrix
   mode after `mkdir`, explicitly stripping any inherited setgid bit,
   rather than relying on the umask under a setgid parent.
2. OR the ownership-matrix enforcer should treat a setgid bit
   inherited from a setgid parent as non-drift for dir entries whose
   declared mode is `07xx`-clear (i.e. compare the permission bits,
   ignore S_ISGID when the parent is setgid), and auto-reconcile via a
   broker `chmod` op instead of failing closed.

Option 1 is preferred — keep the matrix strict; make the creator honor
it.

**Also:** surface the typed `ownership-matrix drift` reason through the
CLI (`nixling up` currently prints `daemon returned unknown
mutating-verb outcome:` with an empty body) so operators see the path +
remediation without grepping the journal.

## Scrapable /metrics endpoint for nixlingd (Phase 6 observability follow-up)

**Current state.** `nixlingd` records `broker_request_total` and
other counters into an in-process Prometheus registry via
`metrics.rs::record_broker_request`.

**Gap.** No live serving path exposes the registry to
Prometheus/Alloy scrapers. The R8 attempt (commit `7dc401b`,
reverted) tried to multiplex HTTP `/metrics` through the public
`SOCK_SEQPACKET` socket, but Prometheus uses `SOCK_STREAM` and fails
with `EPROTOTYPE`.

**Right answer.** Add a dedicated `SOCK_STREAM` AF_UNIX metrics socket
from `nixlingd` (for example `/run/nixling/metrics.sock`), with
`nixling-launchers` group ACL, and wire `prometheus.scrape` config in
observability components.

**Tracking.** This graduates to the Phase-6 broker-authz follow-up
alongside the per-op privileges matrix enforcement.

## Per-op privileges-matrix enforcement at broker (Phase 6 security-hardening)

`packages/nixling-priv-broker/src/runtime.rs` documents the Phase A
runner-control trust model above the `SignalRunner` and
`DeregisterRunnerPidfd` handlers: ADR 0015 treats `nixlingd` as part
of the daemon-only TCB, `envelope.caller_role` is audit-only at the
broker, SO_PEERCRED at accept restricts callers to nixlingd, and the
pidfd registry constrains runner IDs. Phase 6 should move per-op
privileges-matrix enforcement into the broker boundary.

## Drop the `microvm.*` option namespace; nixling owns its hypervisors

**Status.** The `microvm.nix` FLAKE INPUT was dropped in v1.1 (per
[ADR 0018](docs/adr/0018-microvm-nix-removal.md); `flake.nix` line
7 carries the comment). nixling owns its per-VM evaluator
(`nixos-modules/vm-evaluator.nix` + `nixos-modules/vm-options.nix`)
and spawns every runner through the broker's `SpawnRunner` pipeline.

**What didn't get cleaned up.** The OPTION NAMESPACE `microvm.*`
survives across 29 framework `.nix` files and is the live writer
inside consumer flakes. `nixos-modules/vm-options.nix` declares
`options.microvm = { … }` (line 27) explicitly for backward-compat
with consumer flakes that still set `microvm.mem`, `microvm.shares`,
`microvm.writableStoreOverlay`, etc. That backward-compat shim is
also why every comment in the framework reads "microvm.nix's
cloud-hypervisor runner" and "microvm.nix's generator" — the names
imply an upstream dependency that no longer exists. New contributors
and operators reading the code are misled into thinking microvm.nix
is still load-bearing.

The user-facing rename: introduce a nixling-native namespace (e.g.
`nixling.vms.<vm>.runner.* / .volumes / .shares / …`), keep the
`microvm.*` aliases as a deprecation shim for one minor release,
then delete them.

### Framework files with live `microvm.*` writers (must rename)

- `nixos-modules/host.nix` lines 108, 257, 260-262, 306 — declares
  `microvm.interfaces`, `microvm.vsock.cid`, `microvm.hypervisor`,
  `microvm.cloud-hypervisor.extraArgs`, `microvm.shares`
  per-VM. This is the primary translation site.
- `nixos-modules/net.nix` line 380 — declares the net-VM's
  `microvm = { hypervisor; vcpu; mem; volumes; interfaces; }`
  block.
- `nixos-modules/components/graphics.nix` line 332 — writes
  `microvm = { hypervisor; cloud-hypervisor; … }` for graphics VMs.
- `nixos-modules/components/tpm.nix` lines 16, 25 —
  `microvm.hypervisor`, `microvm.cloud-hypervisor.extraArgs`.
- `nixos-modules/components/audio/guest.nix` lines 126, 130 —
  `microvm.hypervisor`, `microvm.extraArgsScript`.
- `nixos-modules/components/video/guest.nix` lines 15, 17 —
  `microvm.hypervisor`, `microvm.cloud-hypervisor.extraArgs`.
- `nixos-modules/components/observability/guest.nix` line 207 —
  `microvm.hypervisor`.
- `nixos-modules/vm-guest-base.nix` line 71 — `microvm.kernelParams`.
- `nixos-modules/observability-vm.nix` line 53 — `microvm.mem`.
- `nixos-modules/processes-json.nix` lines 183, 417, 454, 607 —
  reads `microvm.vsock.cid`, `microvm.graphics.socket`,
  `microvm.shares` from the evaluated per-VM config.

### Framework files with `microvm.nix` only in COMMENTS (rewrite text)

Roughly 20+ files including `vm-options.nix` (header block),
`vm-evaluator.nix`, `vm-submodule.nix`, `vm-guest-base.nix`,
`host.nix`, `processes-json.nix`, `store.nix`, `manifest.nix`,
`network.nix`, `net.nix`, `host-otel-relay-acl.nix`,
`host-activation.nix`, `host-keys.nix`, `options-vms.nix`
(line 27 `microvm.nix` reference in the description string,
line 148 / 160 / 300 in option doc-strings),
`options-site.nix`, `assertions.nix` lines 307-323 (the graphics
+ autostart assertion talks about `microvm@<vm>.service` and "the
upstream microvm.nix runner" — those units don't exist anymore).
Component modules carry stale comments about `microvm.nix's
cloud-hypervisor runner` / `microvm.nix's generator`. Rewrite each
to describe current behavior: "the broker's `SpawnRunner` op
spawns cloud-hypervisor via the Rust argv generator in
`packages/nixling-host/src/ch_argv.rs`".

### Rust files referencing `microvm` (16 files)

All in comments / doc-strings, e.g.
`packages/nixling-host/src/ch_argv.rs`,
`packages/nixling-host/src/virtiofsd_argv.rs`,
`packages/nixling-host/src/swtpm_argv.rs`,
`packages/nixling-host/src/gpu_argv.rs`,
`packages/nixling-priv-broker/src/ops/spawn_runner.rs`,
`packages/nixlingd/src/pidfs_probe.rs`,
`packages/nixlingd/src/ch_stats.rs`,
`packages/nixling-core/src/bundle_resolver.rs` (e.g. line 2265
"Per-VM systemd unit `microvm@<vm>` will be stopped..."),
`packages/nixling-host/src/host_prep_dag.rs`,
`packages/nixling-host/src/runner_argv_regenerator.rs`,
`packages/nixling/src/lib.rs`. Update to describe the current
broker/daemon path; drop the "microvm.nix's X" framing.

### Consumer side (`/etc/nixos`)

The dependency is no longer used by nixling but the consumer flake
still pulls it in. Drop:

- `/etc/nixos/flake.nix` lines 28-31 — `inputs.microvm` block.
- `/etc/nixos/flake.nix` line 45 — `microvm` in the outputs
  function signature.
- `/etc/nixos/flake.nix` line 125 — stale "checks.security-suite"
  comment that blames `inputs.microvm.nixosModules.host`.
- `/etc/nixos/modules/nixling-config.nix` line 53 — stale comment.
- `/etc/nixos/vms/nixling-test.nix` lines 29-42 — `microvm = { mem;
  vcpu; volumes; }` block; rename to the new nixling-native
  namespace.
- `/etc/nixos/vms/personal-dev.nix` lines 98-142 — same;
  particularly `microvm.writableStoreOverlay` (referenced in
  `nixos-modules/options-vms.nix` line 160).
- `/etc/nixos/vms/work-aad.nix` lines 336-354 — same.

The consumer migration is mechanical (one-time `sed`-style rename)
once the framework provides the new option names. Until then, the
deprecation shim must accept BOTH spellings.

### Wider context

- `scripts/MIGRATION-PRE-V0.1.0.md` and
  `scripts/migrate-nixling-v0.1.0.sh` mention microvm.nix as
  historical context — leave alone.
- `pkgs/spectrum-ch/` and `pkgs/crosvm-patched/` mention
  microvm.nix because they're forks of upstream binaries that
  microvm.nix also patches; the comments are documenting heritage,
  not a live dependency.
- `docs/adr/0018-microvm-nix-removal.md` is the binding decision;
  ADRs 0001, 0004, 0011, 0021, 0022, 0023 cross-reference. ADRs
  do not need rewriting (per the docs-cleanup policy from this
  session).
- `docs/adr/README.md` may need a footnote that the option
  namespace cleanup is a follow-up to ADR 0018.

### Sketch of the rename

```
microvm.hypervisor                  →  nixling.vms.<vm>.runner.hypervisor
microvm.vcpu                        →  nixling.vms.<vm>.runner.vcpu
microvm.mem                         →  nixling.vms.<vm>.runner.mem
microvm.vsock.cid                   →  nixling.vms.<vm>.runner.vsockCid
microvm.shares                      →  nixling.vms.<vm>.runner.shares
microvm.volumes                     →  nixling.vms.<vm>.runner.volumes
microvm.interfaces                  →  nixling.vms.<vm>.runner.interfaces
microvm.cloud-hypervisor.extraArgs  →  nixling.vms.<vm>.runner.cloudHypervisor.extraArgs
microvm.kernelParams                →  nixling.vms.<vm>.runner.kernelParams
microvm.writableStoreOverlay        →  nixling.vms.<vm>.runner.writableStoreOverlay
microvm.graphics.socket             →  nixling.vms.<vm>.runner.graphics.socket
microvm.extraArgsScript             →  nixling.vms.<vm>.runner.extraArgsScript
```

(Names are illustrative — pick a final shape during implementation.)

The deprecation shim in `vm-options.nix` should `lib.warn` once per
eval when a consumer flake still uses `microvm.*`, and the new
namespace becomes the documented API across `README.md`,
`templates/default/configuration.nix`, and every example under
`examples/`.

## Remove Tier-0 deployment-shape logic; fix bundle.json access

**Symptom.** `nixling host prepare --apply` and several other CLI verbs
short-circuit with `tier-0-legacy-uses-nixos-module` (exit 78) and
the misleading remediation `Add at least one VM with
nixling.vms.<vm>.supervisor = "nixlingd"` — even though the
`supervisor` option was removed in v1.1 (daemon-only is the ONLY
mode) and the deployed bundle absolutely uses the daemon path.

**Root cause.** `detect_deployment_shape` in
`packages/nixling/src/lib.rs` (~line 1901) falls back to
`DeploymentShape::Tier0AllLegacy` whenever
`context.load_bundle_context()` returns `Ok(None)` or any error. The
`.ok().flatten()` chain SILENTLY swallows the actual failure. In
practice the CLI runs as a launcher user (`paydro`) who cannot read
`/etc/nixling/bundle.json` (root:nixlingd 0640) — so every CLI
invocation from the launcher misclassifies the deployment as legacy
and the operator is told to set a long-removed option.

**Fix direction (pick one or both).**

1. **Delete the Tier-0 branches entirely.** v1.1+ is daemon-only by
   design; there is no Tier-0 / Tier-mixed code path the framework
   even supports. `DeploymentShape` should collapse to `AllDaemon`
   and `cmd_host_prepare` should not gate on shape at all. Drop:
   - `DeploymentShape::Tier0AllLegacy` / `Tier0Mixed` variants
   - The `tier-0-legacy-uses-nixos-module` /
     `single-writer-conflict` envelope branches
   - The `NIXLING_TEST_DEPLOYMENT_SHAPE` test override
   - Whatever tests assert the Tier-0 refusal contract
2. **Decide bundle.json access policy** for the CLI:
   - Either widen the file to `root:nixling-launchers 0640` (or add
     the launcher group via a setfacl seed) so the CLI can read it
     directly, OR
   - Make the CLI query bundle metadata via the daemon (already a
     trusted reader). Today the CLI does a direct file read; if we
     keep that pattern post-fix, the group needs to match.

The remediation message in `cmd_host_prepare` also needs to be
rewritten: pointing operators at a removed option is actively
misleading.

**Files to touch.**

- `packages/nixling/src/lib.rs` (`detect_deployment_shape`,
  `cmd_host_prepare`, `cmd_host_destroy`, related callers).
- `nixos-modules/bundle.nix` (file mode declaration) if widening
  the bundle perms is the path chosen.
- `docs/reference/error-codes.md` — drop the
  `#tier-0-legacy-uses-nixos-module` and
  `#single-writer-conflict` anchors.

## `pidfd-table` is not reaped when supervised processes exit

**Symptom.** Running `nixling vm start personal-dev --apply` after a
graceful CH shutdown (via `vm.shutdown` + `vmm.shutdown` on the CH
API socket) returns `vm 'personal-dev' already has a registered
supervisor pidfd (<role>)` even though every `<role>` process is
gone from `/proc`. Manually `kill`ing leftover sidecars (audio,
vsock-relay) reveals the same: `nixlingd`'s
`/var/lib/nixling/daemon-state/pidfd-table.json` still lists the
dead pids.

**Expected.** CHANGELOG v1.2 D7 ("broker pidfd-reap") promises the
broker reaps spawned children via `tokio-signalfd` +
`waitid(P_PIDFD)` and reports `ChildReaped` to `nixlingd`, which
should drop the entry from `pidfd-table`. In practice the daemon
log shows no reap events even after `pgrep` confirms the process is
gone.

**Concrete failure mode.** With stale pidfd-table entries,
`nixling vm start <vm>` refuses with the "already has supervisor
pidfd" envelope, blocking the operator from recovering without
manually killing leftover processes AND editing
`pidfd-table.json` (which is hand-modifying daemon state and risks
inconsistency). The supervisor decision logic needs to either:

1. Trust pidfd EOF directly (read each pidfd before declaring it
   "live") and drop stale entries on each start attempt, OR
2. Confirm the D7 reap chain is actually wired end-to-end and gate
   the start-refusal on a verified-live signal rather than just
   table presence.

**Files to start from.**

- `packages/nixling-priv-broker/src/sys.rs` — the reaper claims to
  use `waitid(P_PIDFD)` + signalfd; verify it runs.
- `packages/nixlingd/src/supervisor/pidfd.rs` —
  `PidfdTable::snapshot` writes the file; check where entries are
  REMOVED.
- `packages/nixlingd/src/supervisor/mod.rs` — the `ChildReaped` IPC
  consumer (if it exists).
- The daemon's `already has a registered supervisor pidfd` envelope
  is emitted from the start-DAG preflight; check what it's reading.

**Related broker-side bug.** When `nixlingd` tries to send SIGTERM
via pidfd (e.g. `nixling vm stop --apply`), the call returns EPERM
because the daemon runs as unprivileged `nixlingd` user and CH/runner
processes run under restricted uids it can't signal. The broker
(running as root) needs to own the signal dispatch path; the daemon
should ask the broker to signal, not signal directly.

## Broker should intelligently (re)spawn sidecars

**Symptom.** After the host reboots and the Wayland user logs in
later, the per-VM audio sidecar (`nixling-<vm>-snd`,
`vhost-device-sound --backend pipewire`) holds dangling fds to a
PipeWire instance that no longer exists. The guest sees a VirtIO
sound card but `aplay`/`speaker-test` returns `Write error: -4,
Interrupted system call` and Firefox playback is silent. The Plasma
audio mixer never shows the per-VM stream because the sidecar isn't
a registered client of the live `pipewire-0`.

**Root cause.** The broker's `SpawnRunner{role: Audio}` fires during
the VM start DAG, which currently runs during `nixlingd`'s autostart
on boot — before the operator has logged in and before
`/run/user/<uid>/pipewire-0` exists. The sidecar starts as the
`nixling-<vm>-snd` system user (uid in a dedicated range), opens
whatever PipeWire path is available at the time (often nothing,
sometimes a previous session that's since died), and never
reconnects when a new `pipewire-0` appears at user login. Cloud
Hypervisor's `--generic-vhost-user` connection to the sidecar is
also one-shot — even if we respawn the sidecar with the live PW env,
CH stays bound to the dead handshake.

**Pre-v1.0 (bash CLI) behaviour that worked.** The bash `nixling up
<vm>` ran from the operator's interactive Plasma terminal, which
spawned CH + the audio sidecar in lock-step with the live
`PIPEWIRE_RUNTIME_DIR=/run/user/<uid>` already exported. Both
processes saw a healthy PW socket on first connect, and the
operator's login was a hard prerequisite for invoking the CLI.

**What "intelligent" means here.**

1. **Late binding.** Per-VM sidecars that depend on a user-session
   resource (PipeWire socket, Wayland socket, dbus session bus, etc.)
   MUST NOT be spawned until the resource exists. Concretely: the
   audio sidecar should be gated on `/run/user/${waylandUid}/pipewire-0`
   being a live socket the sidecar's uid can `connect(2)` to. The
   GPU sidecar already has a similar dependency on the Wayland
   compositor socket; whatever pattern is used there should be
   generalised.

2. **Liveness watchdog.** When a sidecar exits (segfault, OOM, kill,
   user-session restart), `nixlingd`'s supervisor MUST detect it via
   the pidfd path and respawn it through the same broker
   `SpawnRunner` op. Today the pidfd-table is updated but no
   respawn fires; killing the audio sidecar leaves the VM with a
   dead virtio-snd device and CH stuck on the broken handshake.

3. **CH vhost-user reconnect.** Even with a healthy respawn, CH 52
   keeps its vhost-user connection bound to the original sidecar
   instance. Either:
   - CH must be configured with reconnect support for the
     `--generic-vhost-user` backend (check upstream availability),
     or
   - The supervisor must drive a CH API-side device remove + re-add
     (`DELETE /api/v1/vm.remove-device` + `PUT /api/v1/vm.add-device`)
     after the sidecar comes back, or
   - We accept the limitation and document that audio recovery
     requires a full `nixling vm restart <vm>`.

4. **Session-bound runner pool concept.** Several sidecars share
   the "needs a Wayland user session" property (audio, gpu,
   potentially video). It probably makes sense to introduce a
   dedicated `RunnerSessionScope::WaylandUser` (or similar) that
   the broker checks before spawning; `nixlingd` listens on
   `systemd-logind`'s `SessionNew`/`SessionRemoved` D-Bus signals
   and reconciles the pool when scopes flip.

**Files / code paths to start from.**

- `packages/nixling-priv-broker/src/live_handlers.rs` — `SpawnRunner`
  handler; the audio policy ref is `w1-audio`.
- `packages/nixlingd/src/supervisor/pidfd.rs` — pidfd lifecycle; this
  is where the respawn-on-death watchdog needs to land.
- `packages/nixlingd/src/lib.rs` — autostart + `VmStartRunner::spawn_runner`;
  this is where the session-readiness gate would live.
- `nixos-modules/components/audio/host.nix` — the existing host
  config rules (WirePlumber `client.conf.d/90-nixling.conf` etc.) are
  fine; only the spawn timing is wrong.

**Workaround until fix lands.** When audio is wedged, restart the
affected VM (`nixling vm restart <vm> --apply`). The new CH spawns
a fresh vhost-user handshake against the freshly-spawned sidecar.
This loses the guest's running session.

## Forward-chain re-apply emits duplicate ct-state rules

`/etc/nixling/host.json`'s `forward` chain accumulated 7-8 identical
`ct state established,related accept` rules after multiple
`ApplyNftables` dispatches. The script emitted by
`render_host_nft_script` is idempotent on hash but the broker
re-applies without `flush table inet nixling` first. Result is
benign but ugly. Audit `crate::ops::nft::apply_with_coexistence` and
either pre-flush the table or make the renderer track its own hash
to short-circuit no-op re-applies.

## `/run/nixling/public.sock` group-write ACL

Socket is mode `0660 nixlingd:nixling-launchers` but POSIX ACL
downgrades to `group::r-x mask::rw-` → effective `r--`. Members of
`nixling-launchers` cannot `connect(2)` because Unix sockets require
write. Either drop the POSIX ACL entirely (rely on the base mode) or
add a `mask::rwx` entry. Currently being worked by a parallel agent.

## Stale `10.0.0.0/8 → br-obs-lan` scope-link routes

The obs env declares overlapping host-LAN-style routes
(`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` all
`scope link dev br-obs-lan`) that conflict with `ApplyRoute` re-apply
for other envs and with the host's actual LAN. Likely needs a tighter
default for `nixling.envs.<env>.lanSubnet` boundaries or an explicit
opt-in for overly-broad coverage.

## `nixling vm list` and `nixling audit --strict` daemon-native handlers

Both currently return the typed `not-yet-implemented` exit-78
envelope. The CLI surface is shipped; the daemon side is not.
