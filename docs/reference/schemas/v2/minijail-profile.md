# `minijail-profile.json` schema (`v2`)

Schema: [`minijail-profile.json`](./minijail-profile.json)

Each minijail profile row describes the sandbox contract for one long-lived
process role referenced from `processes.json`.

## Top-level fields

- `profileId` — stable identifier referenced from process roles.
- `role` — human-readable role name.
- `uid` / `gid` — post-drop runtime identity.
- `requiresStartRoot` — whether the role may begin as uid 0.
- `capabilities` — retained Linux capability set.
- `namespaces` — namespace isolation contract.
- `mountPolicy` — writable-path and mount-shape policy.
- `cgroupPlacement` — delegated cgroup target for the role.
- `seccompPolicyRef` — seccomp policy reference.
- `exceptionRef` / `adr_carve_out` — ADR or plan justification for any
  privileged carve-out.
- `userNamespace` — when non-null, broker pre-establishes a single-entry
  user namespace for the role (ADR 0021). Shape: `{ hostUidForZero,
  hostGidForZero }`. virtiofsd roles use this for least-privilege FS
  serving without `CAP_DAC_*` on the host.
- `umask` — **v1.1.2-final**: optional file-creation mask (`Option<u32>`)
  installed in the spawned child via `umask(2)` immediately before
  `execve(2)`. `null` (the default) inherits the broker's umask
  (current behaviour). Sidecar roles that bind shared Unix sockets
  (vhost-user-sound at `snd.sock`, crosvm-gpu at `gpu.sock`, swtpm at
  `tpm.sock`) declare `umask = 7` (octal `0o007`) so the resulting
  sockets are mode 0660 — combined with the per-VM-runtime default ACL
  granting cloud-hypervisor's UID rwx, the named-user ACL becomes
  effective (mask:rw, not mask:---). The broker rejects values
  greater than `0o777` at exec time with `CHILD_EXIT_INVALID_UMASK=75`.

## Contract notes

- This artifact is metadata, not the minijail argv itself; the broker maps it
  to live sandbox execution.
- `requiresStartRoot = false` plus empty capabilities is the expected default.
- `umask` is honoured by the broker child closure unconditionally; profiles
  that do NOT need a specific umask should omit the field (deserialises to
  `None`) rather than declaring `umask = 0`.
