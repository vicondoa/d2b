# 0015. Daemon-only clean break (v1.0)

- Status: Accepted
- Date: 2026-05-31
- Wave: P6
- Plan slice: §"Phase 6: clean break — daemon-only end-state", docs-4
- Companion ADRs: [ADR 0001](0001-systemd-free-vm-orchestration.md), [ADR 0002](0002-non-root-daemon-and-privileged-broker.md), [ADR 0007](0007-bash-coexistence-and-migration.md), [ADR 0010](0010-wire-protocol-and-typed-errors.md)

## Context

The v0.4.0 baseline shipped three persistent root surfaces in addition
to `nixlingd`:

1. **Per-VM systemd templates** — `nixling@<vm>.service`,
   `microvm@<vm>.service`, `microvm-virtiofsd@<vm>.service`,
   `nixling-<vm>-{gpu,snd,video,swtpm,store-sync}.service`,
   `microvm-{tap-interfaces,macvtap-interfaces,pci-devices,set-booted}@.service`,
   `nixling-otel-relay@<vm>.service`, `nixling-known-hosts-refresh@.service`,
   `nixling-vfsd-watchdog@.{service,timer}`, and the
   per-env `nixling-sys-<env>-usbipd-{proxy,backend}.{service,socket}`
   units. Each template carried its own `restartIfChanged = false`,
   its own user/group, its own ACL surface, and its own startup
   ordering.
2. **Host singleton framework services** —
   `nixling-{ch-exporter,otel-host-bridge,net-route-preflight,audit-check}.service`,
   `nixling-audit-check.timer`, and the `microvms.target` aggregator.
3. **The W14c bash CLI fallback bridge** — the Rust `nixling` binary
   shelled out to `/run/current-system/sw/bin/nixling-legacy` for any
   verb the daemon could not yet serve, gated by
   `NIXLING_LEGACY_BASH_OPT_IN` and `NIXLING_LEGACY_CLI`. The
   `cli.nix` Nix module packaged the bash entrypoints and every
   `nixling-<vm>-*` desktop wrapper read from it.

P0–P5 closed the readiness gap. By the time P5 lands, the daemon
covers every verb on the v0.4.0 user-facing surface, the broker has
live handlers for the full host-prepare dispatch
(see [ADR 0011](0011-cgroup-v2-delegation-and-pidfd-handoff.md),
[ADR 0012](0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md),
[ADR 0013](0013-w3-firewall-coexistence-policy.md),
[ADR 0014](0014-w3-modules-devices-runner-shape.md)), the
W14c fallback bridge has been retired (P4 cli-up), and the daemon
auto-flip predicates from
[`docs/reference/default-switch-and-deprecation.md`](../reference/default-switch-and-deprecation.md)
are all true with first-run `/var/lib/nixling/validated/<wave>.json`
evidence records (P5).

Pressure for a clean break came from three directions:

- **Audit-surface sprawl.** Every legacy template was a separate
  root-touching code path. The threat model in
  [`docs/explanation/design.md`](../explanation/design.md) treats each
  one as a distinct privilege boundary; keeping them alive alongside
  `nixlingd` meant auditing N+1 surfaces forever.
- **Two-writer hazard.** ADR 0007's `legacy-systemd` /
  `daemon-experimental` / `daemon-default` split bought a migration
  window at the cost of a per-VM `supervisor = "systemd" |
  "nixlingd"` field and a `/run/nixling/locks/<vm>` filesystem lock
  to keep two writers off the same VM. Both became dead weight once
  every VM was daemon-owned.
- **Operator confusion.** With two CLIs (`nixling` Rust binary and
  `nixling-legacy` bash) and three modes, every support thread
  started with "which mode are you in?". Operators consistently asked
  for one entrypoint.

The decision under review was: **deprecation cycle vs. clean break**.

A deprecation cycle would have kept the legacy surfaces alive under
`nixling.compat.legacySystemd = true` for one minor release (v0.5),
warned at activation time, then deleted them in v0.6. This carries a
real cost: every legacy unit remains in the audit scope of v0.5, the
manifest must keep accepting both `supervisor` values, the
`tests/cli-json.sh` gate must keep both code paths green, the
documentation tree must explain both modes, and the v0.5 → v0.6
upgrade still has the same blast radius — just deferred.

A clean break collapses every legacy surface to "removed in v1.0" in
a single coherent cut at the v0.4.x → v1.0.0 boundary. The cost is
no in-place upgrade path for consumers who skip the migration guide;
the benefit is one audit surface, one CLI, one manifest schema, and
one set of docs from v1.0 onwards.

We chose the clean break.

## Decision

From v1.0.0, `nixlingd` and `nixling-priv-broker` are the **only**
persistent root surfaces the nixling framework declares.

### Cgroup v2 delegation invariant (restated from ADR 0011)

v1.0 daemon-only depends on the cgroup v2 delegation contract
established in [ADR 0011](0011-cgroup-v2-delegation-and-pidfd-handoff.md);
this ADR restates it as a v1.0 hard invariant so reviewers can audit
it from one place:

- The fixed root is `/sys/fs/cgroup/nixling.slice`. The systemd unit
  for `nixling-priv-broker.service` declares
  `Delegate=cpu cpuset io memory pids` against this slice.
- The broker `fchown`s the slice subtree to the non-root `nixlingd`
  uid/gid before dropping its own privileges (the broker keeps
  `CAP_SYS_ADMIN` only for the bounded set of fork/exec sequences
  enumerated in `nixling-host::DeviceClass`; otherwise it runs with
  the empty capability bounding set).
- No threaded cgroups, no partition roots, and no internal processes
  in the slice's interior nodes. Per-VM
  `nixling.slice/<vm>/<role>` leaves are the only fork/exec
  destinations.
- Teardown uses **broker-mediated `CgroupKill`** (v1.1-P10 op
  per [ADR 0011](0011-cgroup-v2-delegation-and-pidfd-handoff.md)
  Decision item 6 + [`docs/reference/cgroup-delegation.md`](../reference/cgroup-delegation.md)
  "Broker ops on the cgroup tree" — **the broker is the sole
  writer of `cgroup.kill`; the daemon NEVER writes
  `cgroup.kill` directly**) against the leaf only; daemon
  uses `pidfd_send_signal(SIGTERM)` first and ONLY escalates to
  broker-mediated `CgroupKill` as a last resort. No parent-side
  signalling. The pidfd registered in the supervisor pidfd table
  (see `packages/nixlingd/src/supervisor/pidfd_table.rs`) is the
  lifecycle-of-record for each runner.

### What stays

- **`nixlingd.service`** — non-root daemon, supervisor of every
  per-VM DAG, owner of state under `/var/lib/nixling/`,
  socket-activated `public.sock` (mode 0660, group
  `nixling`), `restartIfChanged = false`.
- **`nixling-priv-broker.socket` + `nixling-priv-broker.service`** —
  socket-activated privileged broker (see
  [ADR 0002](0002-non-root-daemon-and-privileged-broker.md)),
  dispatcher for every audited host mutation
  (see [`docs/reference/privileges.md`](../reference/privileges.md)),
  append-only audit log at `/var/lib/nixling/audit/broker-<utc-date>.jsonl`.
- **The `nixling` group** — the lifecycle permission
  boundary. Membership in this group plus `SO_PEERCRED` at
  `public.sock` accept time is the only authorisation surface for
  `nixling vm {start,stop,restart,switch}`.

The complete set of root-visible units on a v1.0 host is:

```text
nixlingd.service
nixling-priv-broker.socket
nixling-priv-broker.service
```

### What is deleted

The following are removed wholesale in P6 (no `nixling.compat.*`
escape hatch, no deprecation warnings, no eval-time off-by-default
fallback):

- Every per-VM systemd template enumerated in the legacy-unit
  denylist gate
  ([`tests/legacy-unit-denylist-eval.sh`](../../tests/legacy-unit-denylist-eval.sh))
  AND every additional pattern listed below. **Denylist coverage
  scoping note** (resolves R11 test-r11-2): at v1.0 HEAD the
  `legacy-unit-denylist-eval.sh` gate source-scans
  `nixos-modules/` for a subset of these patterns; the full list
  below is the v1.1 design surface and the residual patterns
  are **scheduled to be added to the denylist gate in their
  owning v1.1-P<N> phase** (see the v1.1 plan TDD-table P10 rows
  — `microvm@`, `microvm-virtiofsd@`, `microvm-pci-devices@`,
  `microvm-set-booted@`, `nixling-vfsd-watchdog@`, and
  `microvms.target` land their denylist registrations in
  v1.1-P10). Until each row lands, this enumeration MUST be
  read as "intent + design scope", not as "currently-gated":
  `nixling@<vm>.service`, `microvm@<vm>.service`,
  `microvm-virtiofsd@<vm>.service`,
  `microvm-{tap-interfaces,macvtap-interfaces,pci-devices,set-booted}@.service`,
  `nixling-<vm>-{gpu,snd,video,swtpm,store-sync}.service`,
  `nixling-otel-relay@<vm>.service`,
  `nixling-known-hosts-refresh@.service`,
  `nixling-vfsd-watchdog@.{service,timer}`,
  `nixling-sys-<env>-usbipd-{proxy,backend}.{service,socket}`.
- Every host singleton framework service:
  `nixling-{ch-exporter,otel-host-bridge,net-route-preflight,audit-check}.service`,
  `nixling-audit-check.timer`, `microvms.target`.
- The `cli.nix` Nix module and every bash entrypoint under
  `scripts/` it packaged. The `/run/current-system/sw/bin/nixling`
  binary is the Rust CLI, end of story.
- The W14c bash fallback bridge and its env-knob escape hatches:
  `NIXLING_LEGACY_BASH_OPT_IN`, `NIXLING_LEGACY_CLI`,
  `NIXLING_NATIVE_ONLY` (the latter retained as a no-op only because
  P4 cli-up already documented it; future minor releases may delete
  the no-op too).
- The `nixling.vms.<vm>.supervisor` option. **Removed in v1.1-P2**
  via a per-submodule `mkRemovedOptionModule` shim in
  `nixos-modules/options-vms-removed.nix` (imported into the per-VM
  submodule's `imports` list per the `attrsOf submodule` /
  `mkRemovedOptionModule` interaction documented in
  `nixpkgs/lib/modules.nix`). Setting the option in a consumer flake
  produces this typed friendly error:

  ```
  nixling.vms.<vm>.supervisor was removed in v1.1 per ADR 0015
  (daemon-only clean break). The v1.0 daemon-only end-state makes
  "nixlingd" the only valid supervisor; v1.1 completes the migration
  by deleting the option entirely.

  Migration: remove every "supervisor = ..." line from your consumer
  flake's nixling.vms.<vm>.* declarations. The daemon-only path is
  the default and only path.
  ```

  **Implementation status (v1.1):** the option DEFINITION is deleted
  from `nixos-modules/options-vms.nix`. The per-submodule
  `mkRemovedOptionModule` shim in `options-vms-removed.nix` is the
  primary error path; a defense-in-depth assertion in
  `nixos-modules/assertions.nix` fires as a backup only when the
  shim is bypassed (e.g. consumer flake subverts the module set).
  Tier 0 detection in `packages/nixling/src/lib.rs`
  `detect_deployment_shape` continues to function — every enabled VM
  is now daemon-supervised, and the systemd-template path is
  retired alongside the supervisor option (the `microvm@<vm>`
  template definitions themselves go in v1.1-P10 per ADR 0018).

  **Companion v1.1 ADRs:**
  [ADR 0017 — No bash fallbacks invariant](0017-no-bash-fallbacks-invariant.md)
  retires `exec_legacy_passthrough` and the residual bash-fallback
  call sites in v1.1-P1. [ADR 0018 — Removal of the microvm.nix
  flake dependency](0018-microvm-nix-removal.md) retires the per-VM
  systemd templates listed below alongside the `microvm.nix`
  substrate in v1.1-P8 → P11.

- The `nixling-launcher` polkit allowlist and its per-VM rules. The
  group itself stays declared for permission-boundary continuity but
  no polkit rule consumes it.
- The per-VM filesystem lock at `/run/nixling/locks/<vm>` (the
  two-writer hazard it guarded no longer exists).
- The `daemon-experimental` / `legacy-systemd` mode plumbing from
  [ADR 0007](0007-bash-coexistence-and-migration.md). ADR 0007
  remains in the repository as historical context; this ADR
  supersedes its decisions 1–6 in their entirety.

### Why no v0 → v1 manifest compatibility window

The manifest bumps from `manifestVersion = 2` to `manifestVersion = 3`
in P2 (`ph2-p2-manifestversion-bump`); fields that previously
encoded systemd-coupled semantics (notably `audioService` and
`apiSocket`) are redocumented as broker-spawn descriptors in P6
under the already-bumped v3 schema. A v0.4 consumer manifest is
**not** machine-translatable to v3 — the per-VM `supervisor` field
is gone, the per-env `nixling-sys-<env>-usbipd-*` socket emitter is
gone, and the host-singleton observability units are gone. Operators
must follow [`docs/how-to/migrate-nixling-v0-to-v1.md`](../how-to/migrate-nixling-v0-to-v1.md)
(landed in P7) to regenerate their `configuration.nix`. There is no
v2 → v3 auto-rewriter.

## Consequences

### Positive

- **Single audit surface.** The framework declares three units. Every
  root-touching code path lives behind `nixling-priv-broker` and is
  recorded in `broker-<utc-date>.jsonl` with the full
  `OpAuditRecord` header
  (see [ADR 0010](0010-wire-protocol-and-typed-errors.md)). Security
  reviews enumerate two binaries and one socket, not N+12.
- **Smaller TCB.** Per-VM systemd templates ran as bespoke users
  (`microvm`, `nixling-<vm>-gpu`, …) each with their own ACL surface
  on `/dev/{kvm,vhost-net,vhost-vsock,tpm0}`, their own
  capability/syscall-filter sets, and their own seccomp profiles.
  Collapsing to a single broker dispatcher means one capability
  matrix, one set of minijail profiles
  (see [ADR 0003](0003-minijail-provisioning-and-sandbox-interface.md)),
  and one audit trail.
- **Daemon health is the only health.** `nixling host doctor` and
  `nixling host validate` report against `nixlingd` + broker only,
  with no "is your `microvm-virtiofsd@<vm>` template wedged?"
  diagnostic axis. The operator surface is
  [`docs/reference/cli-contract.md`](../reference/cli-contract.md).
- **Manifest schema simplification.** The per-VM lock contract is
  gone, and `audioService` / `apiSocket` shrink to opaque
  broker-spawn descriptors. **The `supervisor` field was removed in
  v1.1-P2** via the top-level fallback assertion in
  `nixos-modules/assertions.nix` (the per-submodule
  `mkRemovedOptionModule` shim approach was incompatible with
  `attrsOf submodule` semantics; the v1.1-final assertion fires the
  same friendly ADR-0015 message). The `tests/cli-json.sh` and
  manifest contract gates have one branch per verb instead of two.
- **Operator UX.** One CLI, one config option family
  (`nixling.daemonExperimental.*` retired alongside the bash
  fallback), one set of docs. Support threads no longer start with
  "which mode are you in?".

### Negative

- **No v2 → v3 compatibility window.** Consumer hosts on v0.4 cannot
  upgrade in place without editing `configuration.nix`. The migration
  guide ([`docs/how-to/migrate-nixling-v0-to-v1.md`](../how-to/migrate-nixling-v0-to-v1.md))
  is required reading.
- **Hard dependency on daemon health.** With the per-VM systemd
  templates gone, there is no "systemctl can still bring a VM up if
  `nixlingd` is wedged" escape hatch. Mitigations:
  - `nixling-priv-broker.socket` is socket-activated, so the broker
    cold-starts on first verb dispatch and does not require the
    daemon to be up.
  - `nixlingd.service` carries `Restart=always` with a bounded
    `RestartSec`, so a crash recovers without operator intervention.
  - `nixling host doctor` (a daemon-independent CLI surface) reports
    daemon liveness and the broker `health/v1` probe.
  - The supervisor's persisted DAG state under
    `/var/lib/nixling/supervisor/state.json` survives daemon
    restarts; on restart, `nixlingd` reconciles against the recorded
    state and resumes supervision without losing pidfd handoffs
    (see [ADR 0011](0011-cgroup-v2-delegation-and-pidfd-handoff.md)).
- **Single point of failure for lifecycle ops.** `nixlingd` is the
  only writer for every per-VM DAG. Mitigations: socket-activation
  + `Restart=always` policy as above, plus the daemon's
  reconnect-on-`ENOENT` contract for `public.sock`, plus the broker
  `RestartSec` + `StartLimitIntervalSec` settings keeping a crash
  loop from spiralling. Operators wanting hot-spare semantics are
  out of scope — nixling is a single-host framework.

### Neutral

- **`cli.nix` package retired.** The Nix module that packaged the
  bash entrypoints is deleted. Per-VM `.desktop` wrappers
  (`nixling-launch-<vm>`) are regenerated by the new daemon-native
  launcher module emitting `nixling vm start --apply` calls (P6
  `ph6-p6-cli-nix-migrations`).
- **`cli-contract.md` becomes the operator-facing surface.** With
  one CLI and no bash man page, [`docs/reference/cli-contract.md`](../reference/cli-contract.md)
  is the canonical description of every verb, exit code, and JSON
  envelope. The bash `man nixling` is retired in P7 (`ph7-p7-rust-cli-manpage`).
- **AGENTS.md "Naming conventions" table shrinks.** Most rows
  (`nixling@<vm>.service`, `microvm@<vm>.service`,
  `nixling-<vm>-<role>.service`, host-singleton `nixling-<role>.service`)
  are removed in the P6 AGENTS.md rewrite (docs-5/6/7). Only
  `nixlingd.service`, `nixling-priv-broker.service`, and
  `nixling-priv-broker.socket` remain.
- **ADR 0007 status unchanged.** ADR 0007 stays `Accepted` as the
  historical record of the W2–W14c coexistence path; this ADR
  documents the end-state and is the binding reference from v1.0
  onwards. The ADR 0007 → ADR 0015 supersession is recorded in the
  ADR index ([`docs/adr/README.md`](README.md)).

## Verification

- [`packages/nixling-contract-tests/tests/policy_lints.rs`](../../packages/nixling-contract-tests/tests/policy_lints.rs)
  (`adr_0015_present_with_header_and_cross_references`) asserts this file
  exists and is cross-referenced from `AGENTS.md`.
- [`tests/legacy-unit-denylist-eval.sh`](../../tests/legacy-unit-denylist-eval.sh)
  (P6 `ph6-p6-unit-denylist-gate`) asserts none of the deleted unit
  names appear in `nixos-rebuild dry-build` output on any example.
- [`tests/assertions-eval.sh`](../../tests/assertions-eval.sh)
  (v1.1-P2 closure of the planned-in-P6 `ph6-p6-supervisor-removed-assertion`)
  asserts that setting `nixling.vms.<vm>.supervisor` fails eval with
  the operator-friendly message above. The primary error path is the
  per-submodule `mkRemovedOptionModule` shim in
  `nixos-modules/options-vms-removed.nix`; a defense-in-depth
  assertion in `assertions.nix` is the fallback path when the shim
  is bypassed.
- [`tests/supervisor-option-absent-eval.sh`](../../tests/supervisor-option-absent-eval.sh)
  (v1.1-P2 invariant gate) asserts the productive option declaration
  is absent from `nixos-modules/options-vms.nix` AND the
  `mkRemovedOptionModule` shim is present + wired into the
  submodule's `imports` list.
- P6 exit criterion:
  `systemctl list-units --no-pager --all | grep -E '^(nixling|microvm)' | wc -l`
  returns `3` on the test host.
