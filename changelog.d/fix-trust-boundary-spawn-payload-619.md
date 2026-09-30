### Security

- The broker's `spawn-process` kernel now resolves the runner intent its
  payload names out of its own verified bundle and refuses any launch plan
  that disagrees with it. `binaryPath`, `uid`, `gid`, `supplementaryGroups`,
  `capabilities`, `namespaces`, `seccompPolicyRef`, `mountPolicy`, `umask`,
  `rootCarveOut` and the `userNamespace` mapping were previously read straight
  off the wire and applied as host credentials by the broker.
- A launch that pre-establishes a user namespace can no longer map
  in-namespace root onto host root. A `hostUidForZero` of `0` made the broker
  write `0 0 1` into the child's `uid_map`, so in-namespace root carried
  host-root DAC with the payload's own capability list raised on top, and the
  plan layer's uid-0 guard never saw the field. A host-root mapping is now
  refused unless the bundle row itself declares the ADR 0003 carve-out, which
  the payload can no longer assert for itself.
- `binaryPath` - the executable the broker `execve`s - is fenced against the
  bundle's own executable. The Device-worker argv fence
  reads the paths a launch's arguments name, so a plan could pass it with a
  trusted state path in `argv` and a different executable in `binaryPath` and
  have a worker run as that program under its own principal.
- `bindsRuntimeSocket` is derived from the launched row's declared
  Device-worker posture rather than read from the payload. A launch that
  claimed the per-Guest runtime directory on a row whose trusted posture
  grants nothing was handed `u:<uid>:rwx` on `<runtime_root>/vms/<guest>`,
  which is enough to unlink a sibling worker's socket. A disagreement is
  refused by name in both directions, and the launch that continues runs on
  the derived value.
- The protections a user-namespace launch's mount block drops are no longer
  silent. The device mask, the private `/proc` and the root secret masks are
  skipped for every broker-pre-NS spawn; a launch that would lose the root
  secret masks is now refused, and every other drop is recorded as a
  `critical` audit event naming what was lost.
- A launch whose `bundleRunnerIntentRef` the verified bundle does not resolve
  is refused, and the row it names must also be the row it claims: its VM,
  role id, wire role and cgroup subtree all have to reproduce the same
  verified row, so a plan that matches one bundle row can no longer be
  launched under another row's name. The resolver is now loaded once per
  spawn, so a bundle that is unavailable or fails verification refuses the
  launch outright.
- The `skipBinaryExistsCheck` payload field no longer switches off the
  preflight's missing-executable refusal. The broker decides that, not the
  payload.
