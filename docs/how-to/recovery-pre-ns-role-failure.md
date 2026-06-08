# Recovering from a broker-pre-NS role failure

This runbook helps operators recover from failures of the
broker-pre-established user-namespace (broker-pre-NS) extension
introduced in v1.2 (ADR 0021 + D5).

In v1.2, the following per-VM roles run inside a single-entry
user namespace created by the broker via `clone3(CLONE_NEWUSER)`
before `execve` of the sidecar:

- **virtiofsd** (since v1.1.2 — ADR 0021 original landing)
- **swtpm** (v1.2 / D5 swtpm portion — fully closed)
- **gpu** in render-node-only configurations (v1.2 / D5 gpu portion)
- **audio** with owned net-NS (v1.2 / D5 audio Tier 2)

If the broker child fails to construct the user-NS for one of these
roles, the spawn fails before `execve` and the per-VM DAG node
reports `SpawnRunner failed at <role>`.

## Diagnostic flow

1. **Check the broker child log**:

   ```bash
   sudo tail -100 /var/log/nixling-broker-child.log
   ```

   The child closure writes its exit code to this file (via the
   `CHILD_EXIT_*` constants defined in `packages/nixling-priv-broker/src/sys.rs`).

2. **Look for these exit codes** (the most common pre-NS failure modes):

   | Exit code | Symbolic name              | Meaning |
   |-----------|----------------------------|---------|
   | 60        | `CHILD_EXIT_PRCTL`         | `prctl(NO_NEW_PRIVS)` or similar prctl failed |
   | 61        | `CHILD_EXIT_UNSHARE`       | `unshare()` or `clone3(NEWUSER)` failed — often `EPERM` from `kernel.unprivileged_userns_clone = 0` |
   | 62        | `CHILD_EXIT_CGROUP`        | cgroup-v2 placement failed |
   | 63        | `CHILD_EXIT_MOUNT`         | mount-namespace operation failed; in user-NS context this is usually a regression of the fu27 mount-action-skip guard |
   | 70        | `CHILD_EXIT_SETGROUPS`     | `setgroups(0, NULL)` failed inside user-NS |
   | 71        | `CHILD_EXIT_SETGID`        | `setgid()` failed; uid_map / gid_map mis-written |
   | 72        | `CHILD_EXIT_SETUID`        | `setuid()` failed (same root cause as 71) |
   | 73        | `CHILD_EXIT_EXECVE`        | `execve` of the sidecar binary failed (often missing in profile, or pre-opened fd dup2 collision) |
   | 74        | `CHILD_EXIT_USER_NS_SYNC`  | The broker parent's `/proc/<pid>/{uid,gid}_map` write failed — usually a TOCTOU race or principal-UID misconfiguration |
   | 75        | `CHILD_EXIT_INVALID_UMASK` | umask value outside the valid [0o000, 0o777] range; should never fire if the manifest is consistent |
   | 76        | `CHILD_EXIT_PREOPEN_DUP2`  | dup2 of a pre-opened device fd (e.g. render-node fd handoff) failed |

## Common scenarios

### Scenario 1: `CONFIG_USER_NS=n` on the host kernel

The broker requires `kernel.unprivileged_userns_clone = 1` (or
`CONFIG_USER_NS=y` on the kernel) to call `clone3(CLONE_NEWUSER)`
as the broker's UID. NixOS enables this by default but some
hardened kernels disable it.

**Verify:**

```bash
sysctl kernel.unprivileged_userns_clone
# Expected: kernel.unprivileged_userns_clone = 1
```

**Remediate:**

Add to your NixOS configuration:

```nix
boot.kernel.sysctl."kernel.unprivileged_userns_clone" = 1;
```

Then `nixos-rebuild switch` and retry the VM start.

### Scenario 2: Temporary disable of broker-pre-NS for one role

If you need to ship a fix and the pre-NS is the blocker, you can
temporarily disable it on a per-component basis. The components
support a `brokerPreNs` boolean (or equivalent — check the
specific component's options):

```nix
# In your consumer flake's nixosConfiguration:
nixling.components.tpm.brokerPreNs = false;   # disable D5 swtpm pre-NS
nixling.components.graphics.brokerPreNs = false;  # disable D5 gpu pre-NS
nixling.components.audio.brokerPreNs = false;  # disable D5 audio Tier 2
```

This reverts the role to the v1.1.2 isolation model (minijail-only,
no user-NS), losing the zero-host-caps property of D5 but allowing
the VM to start. **File a bug report** documenting the underlying
failure so we can fix the pre-NS path properly.

### Scenario 3: UID-map / GID-map write failed (exit 74)

The broker writes `/proc/<child-pid>/uid_map` and `gid_map`
immediately after `clone3` returns. Failure of this step usually
means:

- The `stablePrincipalId("nixling-<vm>-<role>")` hash collided
  with an already-allocated system UID. Run
  `bash tests/principal-uid-collision-eval.sh` to verify.
- The broker process itself lost CAP_SETUID/CAP_SETGID between
  `clone3` and the map write — usually a regression in
  `packages/nixling-priv-broker/src/sys.rs` clone child closure.

### Scenario 4: Sidecar binary execve failed (exit 73)

Verify:

- The role's binary is reachable via the minijail-profile
  `pivotRoot` constraints.
- For gpu render-node-only: the `RENDER_NODE_INHERITED_FD = 10`
  is not colliding with another inherited fd in the sidecar's
  argv (check `processes-json.nix` for the fd-passing argv).
- For swtpm: the per-VM state directory exists with the right
  ownership (a fresh `nixling vm reset <vm>` re-provisions it).

## After remediation

Once you've identified and fixed the underlying issue, retry the
VM start:

```bash
sudo systemctl restart nixling-priv-broker
nixling vm start <vm> --apply
```

If you disabled `brokerPreNs` for any role, please re-enable it
once the broker-side fix lands and validate the smoke gate:

```bash
make pre-tag   # runs tests/live-vm-smoke.sh --full
```

The full smoke gate's `swtpm`, `gpu`, and `audio` probes will
catch regressions of the pre-NS path.

## Related docs

- ADR 0021 — `docs/adr/0021-broker-user-namespace-for-virtiofsd.md`
- ADR 0023 — `docs/adr/0023-runner-role-lifecycle-matrix.md`
- v1.2 planning notes — deliverable description
