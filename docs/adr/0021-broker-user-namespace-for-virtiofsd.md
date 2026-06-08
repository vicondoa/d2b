# ADR 0021: Broker-pre-established user namespace for virtiofsd

- Status: Accepted (v1.1.1)
- Date: 2026-06-02
- Supersedes: ADR 0003 §"virtiofsd --sandbox=namespace setup
  exception" (which carved out `requiresStartRoot=true` and a
  CAP_SYS_ADMIN / CAP_DAC_OVERRIDE union for virtiofsd)
- Related: ADR 0001 (systemd-free orchestration: broker-owned
  privilege quarantine), ADR 0011 (cgroup v2 delegation + pidfd
  handoff), ADR 0014 (W3 module / device / runner shape), ADR 0018
  (microvm.nix removal)

## Context

`virtiofsd` (rust-vmm reimplementation) serves a host directory
to a guest over the `vhost-user-fs` protocol. The serving model
requires the daemon to be able to:

1. `setresuid(0, 0, 0)` so the file-system operations execute as
   real-root in some namespace
2. Optionally call `name_to_handle_at(2)` / `open_by_handle_at(2)`
   (the `--inode-file-handles=always` path); these require
   `CAP_DAC_READ_SEARCH` effective on the host
3. Optionally pivot_root into a chroot (`--sandbox=chroot`),
   which requires `CAP_SYS_ADMIN` effective
4. Preserve file mode/UID bits accurately so the guest sees
   files with the same exec/setuid semantics they have on the
   host (otherwise `chroot $sysroot $closure/prepare-root` in
   the guest initrd exits with 126 — "command found but cannot
   execute" — which is the symptom we hit pre-fu14)

Through v1.1.1fu13, our broker spawned virtiofsd as a non-root
ephemeral UID (`stablePrincipalId "nixling-<vm>-runner"`) with
the cap union above in the *bounding* set but NOT effective.
virtiofsd's `setresuid(0)` failed with EPERM, file-handle
support self-disabled, and the guest initrd `chroot` failed
because files served over `vhost-user-fs` from a non-root,
non-fake-root virtiofsd carry UID overflow (`65534`) which
some `execve` paths in the guest kernel treat as inaccessible
for the chroot context.

We needed a model where:

- virtiofsd has the privileges it needs to serve files correctly
- The HOST attack surface is minimal (no `CAP_DAC_*`, no
  `CAP_SYS_ADMIN`, no `CAP_SETUID` on the broker-spawned process
  visible to the host)
- The model is uniformly applied to all four virtiofsd shares
  (ro-store + nl-meta + nl-hkeys + nl-ssh-host) without ad-hoc
  per-share carve-outs
- The implementation does not require `/etc/subuid` /
  `/etc/subgid` allocations (which are operator-visible state
  and a per-host migration burden)

## Decision

The broker pre-establishes a per-runner user namespace before
exec'ing virtiofsd. Concretely:

1. The role profile (`minijail-profiles.nix`) declares
   `userNamespace = { hostUidForZero = <uid>; hostGidForZero = <gid>; }`
   for the virtiofsd shares. The host UID is the
   `stablePrincipalId` for `nixling-<vm>-runner` (the same
   ephemeral UID virtiofsd already runs as).
2. `processes-json.nix` propagates this into the role's
   `RoleProfile.userNamespace` field (camelCase JSON shape).
3. The broker's bundle resolver (`bundle_resolver.rs`) carries
   the spec through to `ResolvedRunnerIntent.user_namespace`.
4. The broker's spawn path (`SpawnRunnerPlanInput.user_namespace`
   → `RunnerIsolationSpec.user_namespace`) drives the kernel
   syscalls:
   - `clone3` with `CLONE_NEWUSER` (and `CLONE_NEWPID` only when
     the role profile requests it via `namespaces.pid = true`).
     **`CLONE_NEWNS` is intentionally NOT in the clone3 flag set**;
     the mount namespace, when needed, is created later via
     `unshare(CLONE_NEWNS)` from inside the child AFTER the
     sync-pipe read returns. Doing the unshare before the sync
     read would fail because the child hasn't yet acquired in-NS
     root via the parent-written `uid_map`.
   - Child blocks on a `pipe2(O_CLOEXEC)` sync pipe
   - Parent writes `/proc/<pid>/uid_map = "0 <host_uid_for_zero> 1\n"`,
     `/proc/<pid>/setgroups = "deny"`, then
     `/proc/<pid>/gid_map = "0 <host_gid_for_zero> 1\n"`
   - Parent signals the child via the pipe
   - Child unblocks; from inside the user NS its UID 0 maps
     to the host's ephemeral UID, so it can call
     `setgid(0)` / `setuid(0)` successfully (NOT
     `setuid(host_uid_for_zero)` — host UIDs are unmapped
     inside the new user NS and would return `EINVAL`) and
     gets a full capability set *inside the user NS*. The
     child explicitly SKIPS `setgroups()` because the parent's
     `setgroups deny` write would make any `setgroups(2)` call
     return `EPERM`; the role profile MUST declare empty
     supplementary groups (preflight enforces).
5. The role's *host* capability set is `[]`. There is no
   `CAP_DAC_OVERRIDE`, `CAP_SYS_ADMIN`, `CAP_SETUID`, or
   `CAP_DAC_READ_SEARCH` granted on the host.
6. virtiofsd is invoked with `--sandbox=chroot
   --inode-file-handles=never` (and `--readonly` for the
   `/nix/store` share). Inside the user NS, `--sandbox=chroot`
   works because the namespace gives virtiofsd
   `CAP_SYS_ADMIN` for `pivot_root(2)`.

### Trade-off: guest-visible UIDs are mapped via single-entry NS

Because the user namespace is **single-entry** (only NS-UID 0
maps to the host's ephemeral runner UID; every other host UID
is unmapped), files served by virtiofsd through this NS appear
in the guest with their virtiofsd-visible UID, which is the
mapped UID inside the NS. Files owned by **host root** on the
shared directory will appear inside virtiofsd's NS as the
**overflow UID** (kernel default `65534` / `nobody`), and
virtiofsd will then forward that overflow UID to the guest.

This is **acceptable for ALL current virtiofsd shares**:

- `/nix/store` is content-addressed and world-readable; the
  guest doesn't need true root-UID ownership semantics on it.
- `nl-ssh-host` per-keyfile (`ssh_host_ed25519_key` 0400
  root:root): a per-keyfile ACL grant (`u:<runtime_uid>:r`)
  installed by `host-activation.nix` makes the file readable
  to virtiofsd inside the NS; the file appears to the guest
  with the overflow UID, but the guest's sshd doesn't care
  about UID ownership on its host key — only its read mode.
- `nl-hkeys` / `nl-meta`: same story — content semantics, not
  ownership semantics, are what matters.

If a future share needs true UID-preserving semantics (e.g.,
`/home/<user>` mounted into the guest with the host user's
real UID), this single-entry mapping is insufficient. Such a
share would need either:

- A multi-entry mapping (`/etc/subuid` + `newuidmap` — rejected
  for the v1.1.2 closure per the "Alternatives considered"
  section), OR
- An out-of-band ID translation policy in virtiofsd (`--uid-map`
  + `--gid-map` arguments — also requires subuid provisioning).

This trade-off is explicit and documented; the v1.1.2 model
covers every virtiofsd use case nixling currently ships.

## Consequences

Positive:

- The HOST never has a virtiofsd process with `CAP_DAC_OVERRIDE`
  effective. The principle-of-least-privilege gains are
  measurable: per `man capabilities`, the v1.1.0 model granted
  virtiofsd 10 caps in the bounding set, of which 1
  (`CAP_DAC_OVERRIDE`) is exploitable on a vulnerable
  virtiofsd into a full host filesystem-read escape. The v1.1.1
  model exposes ZERO host caps on the virtiofsd process.
- `requiresStartRoot=false` for virtiofsd. The "must start
  root then drop" carve-out (ADR 0003) is retired.
- File-mode preservation: in the user NS, virtiofsd is
  fake-root, so files served over `vhost-user-fs` carry their
  real host UIDs (mapped to 0 for the runner principal) and
  the guest initrd `chroot $sysroot $closure/prepare-root`
  succeeds. The pre-fu14 exit-126 symptom is fixed.
- No `/etc/subuid` provisioning required. The single-entry
  mapping does not need a subuid range — it maps in-NS UID 0
  to the principal's already-allocated ephemeral UID.
- The sync-pipe wait is deterministic; there is no race
  between child-side `setresuid(0)` and parent-side
  `uid_map` writes.

Negative:

- The broker's spawn path is more complex. `sys.rs`
  `clone3_spawn_runner` now has a parent-side post-fork phase
  (uid_map writes, signal) in addition to the child-side phase.
  Audit/triage of a failed spawn must distinguish
  `CHILD_EXIT_USER_NS_SYNC` from `CHILD_EXIT_SETUID`.
- If the parent process dies between the `clone3` syscall and
  the `uid_map` write, the child is wedged in `read()` on a
  pipe whose write end has been closed. The `EOF` causes a
  short read (`n != 1`), so the child exits with
  `CHILD_EXIT_USER_NS_SYNC`. This is observable in
  `journalctl -u nixling-priv-broker` as the spawned process
  exiting with status 74.
- The `CLONE_NEWUSER` flag was previously guarded against in
  the broker (an explicit `Unsupported` error). The guard now
  becomes a positive check: `namespaces.user=true` requires
  `user_namespace=Some(spec)` and vice versa; the broker
  refuses orphan settings.

## Alternatives considered

1. **virtiofsd's own `--uid-map=:0:<uid>:1:` flag** (the
   "Scenario B" from the research report). This delegates
   user-NS creation to virtiofsd itself via `newuidmap` /
   `newgidmap` setuid helpers. **Rejected** because it would
   require `/etc/subuid` provisioning per principal — the
   `newuidmap` tool refuses single-entry maps that overlap
   with the operator's primary UID without a subuid
   declaration. Operator-visible state and a per-host
   migration burden.

2. **`--sandbox=none --readonly`** (the "Scenario A" from the
   research report). For the `/nix/store` share specifically,
   this works because the store is content-addressed and
   already world-readable. **Rejected for v1.1.1** because the
   mutable per-VM shares (`nl-meta`, `nl-hkeys`, `nl-ssh-host`)
   carry actual confidentiality (the SSH host keys especially)
   and dropping the sandbox would let any guest with a
   symlink-traversal vulnerability in virtiofsd escape to
   arbitrary host paths.

3. **Leave virtiofsd as v1.1.0 (root carve-out)**.
   **Rejected** because per the live-deploy debug it does not
   work end-to-end on broker-spawned non-root runners — the
   exit-126 chroot failure in guest initrd is a hard blocker.
   The "carve-out to root" path also defeats the broker's
   privilege-quarantine premise (ADR 0001).

## Implementation contract

The kernel-level contract this ADR establishes (corrected
v1.1.1fu15 + ordering-corrected v1.1.2fu17 to match actual broker
code):

```
broker:
  sync_pipe = pipe2(O_CLOEXEC)
  # CLONE_NEWPID is added to the flag set ONLY when the role
  # profile explicitly sets namespaces.pid = true. virtiofsd
  # profiles default namespaces.pid = false, so the live
  # virtiofsd spawn uses only CLONE_NEWUSER | CLONE_PIDFD here.
  outcome = clone3({
      flags: CLONE_NEWUSER | CLONE_PIDFD
             | (if namespaces.pid then CLONE_NEWPID else 0),
      ...
  })
  if outcome.is_child:
    # clone3 + CLOEXEC: the read_fd is CLOEXEC, but execve hasn't
    # happened yet, so we close the write_fd we inherited (a copy
    # of the parent's write end exists in the child until execve)
    # BEFORE blocking on read. Otherwise broker-death between
    # clone3 and uid_map write leaves us wedged forever — our own
    # write_fd copy keeps the pipe open.
    close(sync_pipe.write_fd)
    read(sync_pipe.read_fd, 1 byte)   # blocks until parent maps written
    # Now we are guaranteed to be in-NS root, so we can do:
    prctl(PR_SET_NO_NEW_PRIVS, 1)
    if isolation.mount_required:
      unshare(CLONE_NEWNS)             # mount NS lazy after user NS
    # Optional: mount/cgroup setup happens here, with CAP_SYS_ADMIN
    # available inside the user NS.
    setgid(0)                          # in-NS GID 0 (mapped to host_gid_for_zero)
    setuid(0)                          # in-NS UID 0 (mapped to host_uid_for_zero)
    # setgroups() is SKIPPED entirely when in user-NS — parent
    # wrote `setgroups deny` so any setgroups call returns EPERM.
    # supplementary_groups MUST be empty (preflight enforces).
    capset(child_caps)                 # in-NS caps (full inside NS)
    execve(virtiofsd_binary, argv, env)
  else:
    write("/proc/<child_pid>/uid_map", "0 <host_uid_for_zero> 1\n")
    write("/proc/<child_pid>/setgroups", "deny")
    write("/proc/<child_pid>/gid_map", "0 <host_gid_for_zero> 1\n")
    close(sync_pipe.read_fd)           # parent has no further use
    write(sync_pipe.write_fd, 1 byte)  # unblock child
    return outcome
```

The parent's writes are sequenced strictly:
`uid_map` → `setgroups=deny` → `gid_map`. Per `man 7
user_namespaces`, writing `gid_map` requires either
`CAP_SETGID` in the parent's user NS OR `setgroups=deny`
to have been written first. Our broker may not have
`CAP_SETGID` (it runs as root in the host NS but may not
in subsequent v1.2 broker-pre-NS chaining), so we use the
`setgroups=deny` path defensively.

Note: `CLONE_NEWNS` is intentionally NOT in the clone3 flag
set — the mount namespace, when needed, is created later via
`unshare(CLONE_NEWNS)` AFTER the sync-pipe read returns
(i.e. AFTER the parent has populated the uid_map). This
matches `man 7 user_namespaces` recommendation: a process in a
user NS may freely unshare mount/PID NS without `CAP_SYS_ADMIN`
in the parent. Doing the unshare BEFORE the sync read would
fail because the child hasn't yet acquired in-NS root.

Note: child calls `setuid(0)` / `setgid(0)` to drop to in-NS
root, NOT to the host's `host_uid_for_zero`. Inside the new
user NS, host UIDs are UNMAPPED; the only valid identities
are NS-{u,g}id 0 (mapped to host {u,g}id) and 65534/nobody
(the unmapped overflow id). A `setuid(host_uid_for_zero)`
call would return `EINVAL`.

## Test coverage

- `packages/nixling-priv-broker/src/ops/spawn_runner.rs`:
  - `user_namespace_round_trips_none` — bundle without
    user_namespace flows through preflight unchanged
  - `user_namespace_round_trips_some` — bundle with spec
    flows through preflight unchanged
  - `user_namespace_with_zero_uid_is_allowed_in_plan_layer` —
    pins that the preflight does NOT reject UID 0 maps; the
    refusal is enforced in `runtime.rs` against
    `adr_carve_out`.
- `packages/nixling-priv-broker/src/sys.rs`:
  - `user_namespace_true_requires_spec` — broker rejects
    `namespaces.user=true` without `user_namespace=Some(_)`
  - `user_namespace_spec_requires_namespace_flag` — broker
    rejects orphan `user_namespace=Some(_)` without
    `namespaces.user=true`
- Live-deploy validation:
  - `personal-dev` boots end-to-end with the new sandbox
    model (initrd reaches multi-user.target; SSH connect
    succeeds from host)
  - `work-aad` boots end-to-end concurrently
  - `journalctl -u nixling-priv-broker | grep -i virtiofsd`
    shows NO `Couldn't set the process uid as root: -1`
    warnings under the new model

## Future work (v1.2 candidates, out of v1.1.1 scope)

- Multi-principal mapping. The current spec is single-entry
  (in-NS UID 0 → one host UID). Future write-heavy shares may
  need multi-range maps to preserve per-guest-user ownership.
- `newuidmap`/`newgidmap` helper integration as an opt-in
  alternative for operators who want subuid-range mappings.
- Apply the broker-pre-NS model to other roles (swtpm, gpu, audio)
  for uniform least-privilege; the chief blocker today for gpu and
  audio is that those roles need access to host devices
  (`/dev/dri/renderD128`, `/dev/snd/*`) which the user NS
  does not natively grant — would need bind-mount + setfacl
  coordination.

  **v1.2 D5/P2.3 partial closure**: swtpm is **fully closed** in
  v1.2. swtpm has zero device binds + zero host caps + Unix socket
  only — a direct translation of the virtiofsd model. The long-lived
  swtpm sidecar profile now declares `userNamespace = { hostUidForZero
  = stablePrincipalId "nixling-<vm>-swtpm"; hostGidForZero = ... }`
  and runs with zero host capabilities inside a single-entry user NS.

  **v1.2 D5/P2.3 gpu render-node closure (v1.2fu25)**: gpu is
  **partially closed** for the render-node-only case. The
  `gpu-render-node` profile (selected when
  `graphics.renderNodeOnly = true`) uses SCM_RIGHTS-style fd
  inheritance: the broker parent pre-opens `/dev/dri/renderD128`
  before `clone3(CLONE_NEWUSER)`, `dup2`s it to
  `RENDER_NODE_INHERITED_FD = 10` in the user-NS child, and the
  crosvm argv references `--gpu-device-node /proc/self/fd/10`.
  Render nodes bypass DRM master authentication entirely
  (`DRM_IOCTL_SET_MASTER` / `DRM_IOCTL_AUTH_MAGIC` not required),
  so the pre-opened fd is fully usable inside the user NS.
  NVIDIA / non-render-node device passthrough remains out of scope
  (those devices have host-owned char-device permission checks that
  a single-entry user NS cannot bridge without device-specific
  kernel support). The legacy `gpu` profile is unchanged.

  audio remains scope-restricted pending AF_NETLINK dependency
  elimination. Deferred to a subsequent fuN per plan §P2.3.

  **v1.2fu27 D5/P2.3 audio closure (Tier 2)**: audio is **fully
  closed** via **user-NS + owned-net-NS**. Root cause: vhost-device-sound's
  libpipewire client opens `AF_NETLINK(NETLINK_KOBJECT_UEVENT)` during
  `pw_context_new()` (spa-alsa-monitor); in a user-NS-only spawn,
  `ns_capable(net->user_ns, CAP_NET_RAW)` checks the initial user NS
  (owner of the pre-existing host net NS) — bind fails with `EPERM`.
  Tier 1 (PipeWire config elimination via `PIPEWIRE_LATENCY` /
  `PIPEWIRE_NODE` / `PIPEWIRE_REMOTE`) was investigated and rejected:
  the AF_NETLINK open is structural in libpipewire's context-init path
  and precedes user-facing env-var consumption. Tier 2 resolution: add
  `namespaces.net = true` to the audio minijail profile. The child calls
  `unshare(CLONE_NEWNET)` inside the user NS (after uid_map is written);
  the new net NS is owned by the new user NS; `CAP_NET_RAW` is effective
  there. No changes to `RunnerIsolationSpec` or `sys.rs` were needed —
  `NamespaceSet.net` + `unshare_namespace_flags` already handled
  `CLONE_NEWNET`. Audio `capabilities` reduced from `["CAP_NET_RAW"]` to
  `[]`. Bullet 3 is **fully closed** for v1.2.

- Single-entry user-NS limitation for write-heavy shares (e.g.
  `/home/<user>` mounts needing true UID-preserving semantics)
  — remains out of v1.2 scope; tied to bullet 1's multi-principal
  mapping work.
