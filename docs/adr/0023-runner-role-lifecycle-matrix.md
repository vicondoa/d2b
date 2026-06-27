# ADR 0023: Runner-role lifecycle matrix

- Status: Accepted (v1.2)
- Date: 2026-06-09
- Related: ADR 0003 (minijail provisioning and sandbox interface),
  ADR 0011 (cgroup v2 delegation and pidfd handoff),
  ADR 0014 (W3 module / device / runner shape),
  ADR 0018 (microvm.nix removal),
  ADR 0021 (broker-pre-established user namespace for virtiofsd)

## Context

The vx-kernel retrospective noted: **"fu27 + fu34 should have been
caught at v1.1.2 review; recommends lifecycle matrices."** Both
regressions became live-deploy failures that a per-role review
checklist would have surfaced at design time:

- **fu27** (virtiofsd mount-action skip branch): the in-NS
  `unshare(CLONE_NEWNS)` path was exercised on every live virtiofsd
  spawn but had no hermetic unit test asserting the skip behaviour.
  The gap was invisible to `ssh root@<ip>` smoke gates — the VM
  booted successfully, but virtiofsd store-share semantics were
  subtly broken until the path was explicitly exercised by a probe.
- **fu34** (zombie detection in `wait_for_one_shot_exit`): the
  live-deploy hermetic-test gap means that state-Z processes were
  observable at runtime but the behavioral guard (return `Ok(())`
  when the target is already `Z`) had no hermetic proof. The
  `proc_state_tests` module tested the parser, not the behavior.

Both issues have a common root cause: the live-deploy hermetic-test
gap. The framework iterates quickly on role profiles
(`nixos-modules/minijail-profiles.nix`) but no document or tooling
demanded that every role's fork model, capability set, mount
semantics, and resource limits be enumerated and reviewed before a
profile reached production.

A **per-role lifecycle matrix** forces reviewers to reason about each
field explicitly. It creates a diff-visible record of every change
to a role's privilege posture, and it provides the per-role context
that hermetic tests and live-smoke probes need to assert correct
behaviour.

## Decision

Every new runner role **MUST** fill out the lifecycle matrix template
(see §"Matrix template") before being added to
`nixos-modules/minijail-profiles.nix`. The matrix row is part of the
role's design document, not a post-hoc annotation.

For roles that already exist, backfill matrices are provided in this
ADR (see §"Backfill — existing roles"). Any future change to a
backfilled role's profile must update its matrix row in this ADR as
part of the same commit.

## Matrix template

Each role must answer the following eleven fields. Omissions are
a review blocking finding.

| # | Field | Allowed values / format |
|---|-------|------------------------|
| 1 | **Fork model** | `clone3` (flags) \| `posix_spawn` (flags) |
| 2 | **Wait/reap owner** | `broker` \| `d2bd` \| `both` \| `pidfd-handoff (broker → d2bd)` |
| 3 | **In-NS mount-action** | `apply` \| `skip` \| `N/A` — with a brief note on what is applied or why skipped |
| 4 | **Capability bounding set** | comma-separated CAP names, or `empty` |
| 5 | **Ambient capability set** | comma-separated CAP names, or `empty` |
| 6 | **Seccomp profile reference** | key string in the seccomp policy store, or `none` |
| 7 | **FD lifetime** | `inherit` \| `SCM_RIGHTS` \| `close-on-exec` — combined where applicable |
| 8 | **umask value** | octal (e.g. `0o007`) or `inherit` (broker default) |
| 9 | **RLIMIT_NPROC value** | integer or `inherit` |
| 10 | **oom_score_adj value** | integer or `inherit` |
| 11 | **CLONE_INTO_CGROUP usage** | `yes — <subtree>` \| `no` |

## Backfill — existing roles

Backfill matrices are sourced from `nixos-modules/minijail-profiles.nix`
(HEAD `588a913`) and the broker's `packages/d2b-priv-broker/src/sys.rs`
`clone3_spawn_runner` path.

All five roles below are spawned by the broker via
`clone3_pidfd_or_fork_fallback_with_cgroup` (kernel ≥ 5.7 path),
which combines `CLONE_PIDFD` + `CLONE_INTO_CGROUP` + role-specific
namespace flags. The broker holds the pidfd briefly and then hands it
to d2bd over `SCM_RIGHTS` via the `OpenPidfd` op (ADR 0011
§"pidfd handoff"). `d2bd` is therefore the long-term reap owner;
the broker initiates the spawn and the pidfd transfer, but does not
`waitid`. **D7** (v1.2) will add `waitid(P_PIDFD)` on the broker
side for one-shot roles, changing the reap owner to `both` for that
subset; this ADR will be updated when D7 lands.

---

### cloud-hypervisor

**Role ID**: `cloud-hypervisor-runner`
**Principal**: `d2b-<vm>-runner`
**Source profile**: `mkProfile` at `nixos-modules/minijail-profiles.nix`
line ~247; companion ADR 0004.

| Field | Value |
|-------|-------|
| Fork model | `clone3` (`CLONE_PIDFD \| CLONE_INTO_CGROUP \| CLONE_NEWIPC \| CLONE_NEWNS`) — no `CLONE_NEWUSER`, no `CLONE_NEWNET`, no `CLONE_NEWPID`, no `CLONE_NEWUTS` |
| Wait/reap owner | `pidfd-handoff (broker → d2bd)` — broker transfers pidfd via `OpenPidfd`; d2bd reaps |
| In-NS mount-action | **apply** — broker bind-mounts `/dev/kvm`, `/dev/vhost-net`, `/dev/net/tun` (device nodes) and the VM state dir (RW); `/nix/store` (RO) |
| Capability bounding set | `CAP_NET_ADMIN` (setup-time union for `SCM_RIGHTS` TAP-fd recv and `TUNSETIFF`; CH drops it before entering its main loop — see note ①) |
| Ambient capability set | `empty` — broker does not raise ambient caps; minijail does not configure an ambient set for this role |
| Seccomp profile reference | `w1-cloud-hypervisor-runner` (declarative ioctl allowlist: `TUNSETIFF`, `TUNSETGROUP`; BPF compilation not yet wired — v1.2/D4 closes this gap) |
| FD lifetime | `close-on-exec` for inherited fds; TAP fd received post-exec via `SCM_RIGHTS` over CH's API socket |
| umask value | `inherit` (broker default; no socket-binding constraint) |
| RLIMIT_NPROC value | `inherit` |
| oom_score_adj value | `inherit` (0) |
| CLONE_INTO_CGROUP usage | **yes** — `d2b.slice/<vm>/cloud-hypervisor` |

**Note ①**: CH's published behaviour is to drop `CAP_NET_ADMIN`
before entering its main loop (after device-init and TAP
configuration). The minijail static allowlist cannot express
"transient"; operators MUST audit the cloud-hypervisor build to
confirm the drop happens. v1.2/D4a adds a live-smoke assertion
(`/proc/<ch-pid>/status CapEff` bit 12 = 0 after ≥10 s running).

**Relationship to version-pinning policy**: the panel-virt R1 note
recommends a CH version-pinning policy; that is documented as a
placeholder in this ADR's §"Future work". Runner-shape snapshot
tests (D15) catch argv drift in v1.2.

---

### virtiofsd

**Role ID**: `virtiofsd`
**Principal**: `d2b-<vm>-runner` (same ephemeral UID as the CH runner)
**Source profile**: `virtiofsdProfiles` at `nixos-modules/minijail-profiles.nix`
line ~184; companion ADR 0021.

| Field | Value |
|-------|-------|
| Fork model | `clone3` (`CLONE_PIDFD \| CLONE_INTO_CGROUP \| CLONE_NEWUSER \| CLONE_NEWIPC \| CLONE_NEWNS`) — broker pre-establishes user-NS; `CLONE_NEWNS` IS in the clone3 flag set here because minijail requests a mount namespace; the user-NS sync-pipe sequence gates the child from acting until `uid_map`/`gid_map` are written (ADR 0021 §"Implementation contract") |
| Wait/reap owner | `pidfd-handoff (broker → d2bd)` |
| In-NS mount-action | **apply (user-NS gated)** — child blocks on sync-pipe until parent writes `uid_map`/`gid_map`; after unblocking, child calls `unshare(CLONE_NEWNS)` then broker applies bind-mounts (state dir + runtime dir); `--sandbox=chroot` + `--inode-file-handles=never` inside the NS |
| Capability bounding set | `empty` on the host; **full** inside the single-entry user namespace (fake-root at NS-UID 0) |
| Ambient capability set | `empty` on the host; not applicable inside the user-NS (capabilities are derived from the user-NS, not the ambient set) |
| Seccomp profile reference | `w1-virtiofsd` |
| FD lifetime | `close-on-exec` for inherited fds; virtiofsd connects to the broker-created socket at the declared path (no SCM_RIGHTS fd injection) |
| umask value | `inherit` |
| RLIMIT_NPROC value | `inherit` |
| oom_score_adj value | `inherit` (0) |
| CLONE_INTO_CGROUP usage | **yes** — `d2b.slice/<vm>/virtiofsd-<share-tag>` |

**Context for in-NS mount-action**: fu27 was the live-deploy failure
where the mount-action skip branch (triggered when the role enters a
user namespace) had no hermetic test. The "skip" variant of this field
would apply to a future role that enters a user-NS but does NOT need
any host-path binds inside the NS; virtiofsd currently applies bind
mounts and therefore documents "apply (user-NS gated)".

**Relationship to version-pinning policy**: virtiofsd version-pinning
strategy is documented as a placeholder in §"Future work".

---

### swtpm

**Role ID**: `swtpm`
**Principal**: `d2b-<vm>-swtpm`
**Source profile**: `mkProfile` at `nixos-modules/minijail-profiles.nix`
line ~345. See also the `swtpm-flush` profile (same principal and
seccomp ref, short-lived pre-start flush process).

| Field | Value |
|-------|-------|
| Fork model | `clone3` (`CLONE_PIDFD \| CLONE_INTO_CGROUP \| CLONE_NEWIPC \| CLONE_NEWNS`) — no `CLONE_NEWUSER` (namespaces.user = false); no `CLONE_NEWPID` |
| Wait/reap owner | `pidfd-handoff (broker → d2bd)` |
| In-NS mount-action | **apply** — swtpm state dir (`/var/lib/d2b/vms/<vm>/swtpm`) and runtime dir (`/run/d2b/vms/<vm>/`) bound RW; state dir is a **stable RW bind** (NOT tmpfs), preserving TPM 2.0 NVRAM + EK seed across daemon restarts |
| Capability bounding set | `empty` — `capabilities = [ ]` (default; explicitly preserved per kernel-r2-4; do NOT add capability overrides without a dedicated ADR finding) |
| Ambient capability set | `empty` |
| Seccomp profile reference | `w1-swtpm` |
| FD lifetime | `close-on-exec` |
| umask value | `0o007` (v1.1.2fu36: swtpm binds control socket with mode 0660; combined with the per-VM runtime dir default ACL, lets CH connect to `snd.sock` without operator intervention) |
| RLIMIT_NPROC value | `inherit` |
| oom_score_adj value | `inherit` (0) |
| CLONE_INTO_CGROUP usage | **yes** — `d2b.slice/<vm>/swtpm` |

---

### gpu (crosvm GPU sidecar)

**Role ID**: `gpu`
**Principal**: `d2b-<vm>-gpu`
**Source profile**: `mkProfile` at `nixos-modules/minijail-profiles.nix`
line ~367.

| Field | Value |
|-------|-------|
| Fork model | `clone3` (`CLONE_PIDFD \| CLONE_INTO_CGROUP \| CLONE_NEWIPC \| CLONE_NEWNS`) — no `CLONE_NEWUSER`; no `CLONE_NEWPID` |
| Wait/reap owner | `pidfd-handoff (broker → d2bd)` |
| In-NS mount-action | **apply** — device nodes (`/dev/kvm`, `/dev/dri/renderD128`, `/dev/nvidiactl`, `/dev/nvidia0`, `/dev/nvidia-uvm`, `/dev/udmabuf`) bound into mount-NS; state dir and GPU runtime dir (`/run/d2b-gpu/<vm>/`) RW; Wayland socket (`/run/user/<waylandUid>/wayland-0`) bind-mounted inside sandbox at role-local path to prevent `../` traversal |
| Capability bounding set | `empty` — original matrix carried `CAP_SYS_NICE`; per-role smoke confirmed no NICE is required (virgl/venus/cross-domain run under `SCHED_OTHER`) |
| Ambient capability set | `empty` |
| Seccomp profile reference | `w1-gpu` |
| FD lifetime | `close-on-exec` |
| umask value | `0o007` (v1.1.2fu36: crosvm GPU sidecar binds vhost-user socket at `gpu.sock`; umask 0o007 → mode 0660; named-user ACL entry grants CH rw access) |
| RLIMIT_NPROC value | `inherit` |
| oom_score_adj value | `inherit` (0) |
| CLONE_INTO_CGROUP usage | **yes** — `d2b.slice/<vm>/gpu` |

---

### audio (vhost-device-sound sidecar)

**Role ID**: `audio`
**Principal**: `d2b-<vm>-snd`
**Source profile**: `mkProfile` at `nixos-modules/minijail-profiles.nix`
line ~435.

| Field | Value |
|-------|-------|
| Fork model | `clone3` (`CLONE_PIDFD \| CLONE_INTO_CGROUP \| CLONE_NEWIPC \| CLONE_NEWNS`) — no `CLONE_NEWUSER`; no `CLONE_NEWPID` |
| Wait/reap owner | `pidfd-handoff (broker → d2bd)` |
| In-NS mount-action | **apply** — state dir (`/var/lib/d2b/vms/<vm>/state`) and audio runtime dir (`/run/d2b/vms/<vm>/`) RW; `/run/user/<waylandUid>/` bound RW so libpipewire `connect()` to the PipeWire socket succeeds inside the mount-NS (v1.1.1fu11 Option B) |
| Capability bounding set | `CAP_NET_RAW` — vhost-device-sound's libpipewire client opens `AF_NETLINK` for the virtio-snd backend probe; `CAP_NET_RAW` gates that bind |
| Ambient capability set | `empty` |
| Seccomp profile reference | `w1-audio` |
| FD lifetime | `close-on-exec` |
| umask value | `0o007` (v1.1.2fu36: vhost-device-sound binds `snd.sock` at `/run/d2b/vms/<vm>/snd.sock`; umask 0o007 → mode 0660; per-VM default ACL makes CH's named-user entry effective) |
| RLIMIT_NPROC value | `inherit` |
| oom_score_adj value | `inherit` (0) |
| CLONE_INTO_CGROUP usage | **yes** — `d2b.slice/<vm>/audio` |

## Consequences

Positive:

- Every new role profile has a structured pre-review checklist.
  Reviewers can diff the matrix row against the `mkProfile` call
  and the broker spawn path without cross-referencing multiple
  files.
- fu27-class regressions (mount-action behavior changed without
  updating the corresponding lifecycle field) become diff-visible
  because the ADR row is part of the same commit as the profile
  change.
- fu34-class regressions (wait/reap owner changed without auditing
  zombie semantics) are caught by the "Wait/reap owner" field, which
  now demands an explicit choice rather than the implicit default.
- The 11-field template aligns with the hermetic test taxonomy: each
  field that can be mechanically verified (capability set, umask,
  seccomp ref, cgroup placement) corresponds to a check that a
  unit test or eval gate can assert.

Negative:

- Hand-maintained matrices drift if role profiles change without a
  corresponding ADR update. The mitigation is the commit rule in
  §"Decision": any profile change must update its matrix row in the
  same commit. v1.2 enforces this as a reviewer expectation; v1.3
  may automate it (see §"Future work").
- The backfill matrices above are point-in-time snapshots at HEAD
  `588a913`. If a v1.2 deliverable (D4, D5, D7) changes a role's
  profile, the responsible commit must update the corresponding row.

## Future work

- **v1.3 auto-generation via `xtask gen-role-matrix`**: v1.2 keeps
  the matrix as hand-maintained markdown. A v1.3 candidate is an
  `xtask` subcommand that derives the matrix from the Rust
  `RoleProfile` DTOs and the `minijail-profiles.nix` evaluation,
  emitting the 11 fields programmatically. This would close the
  drift risk identified in the Consequences section above.
- **CH version-pinning policy** (per panel-virt R1 note): document
  as a placeholder; runner-shape snapshot tests (D15) catch argv
  drift in v1.2. A formal pinning policy (explicit semver floor,
  CVE-response SLA, changelog review cadence) is a v1.3 candidate.
- **virtiofsd version-pinning strategy**: same disposition as CH
  above. v1.2 pinning is managed through `flake.lock` + supply-chain
  policy inherited from ADR 0009; a dedicated version-pinning ADR
  is deferred to v1.3.
- **Broker-pre-NS extension to gpu / audio / swtpm** (D5): when D5
  lands, the Capability bounding set and Fork model rows for the
  gpu, audio, and swtpm matrices will change. The D5 commit must
  update those rows in this ADR.
- **D7 `waitid(P_PIDFD)` on broker side**: when D7 lands, the
  Wait/reap owner field for one-shot roles will change from
  `pidfd-handoff (broker → d2bd)` to `both`. The D7 commit
  must update the affected rows.
