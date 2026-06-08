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

## Contract notes

- This artifact is metadata, not the minijail argv itself; the broker maps it
  to live sandbox execution.
- `requiresStartRoot = false` plus empty capabilities is the expected default.
