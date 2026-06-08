# Changelog

All notable changes to nixling are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Pre-1.0 minor releases may break public APIs. When practical,
deprecations ship one minor release before removal.

## v1.1.2-final — 2026-06-02 — live-deploy hardening (fu25–fu37)

v1.1.2-final closes every issue surfaced during the live 5-VM
bring-up of the v1.1.2 tag (3 net VMs + 2 workload VMs including
work-aad with vTPM). 16 production commits plus 9-discipline
panel-review fixes (R1 round); **887 active tests pass + 9 ignored
= 896 total, 0 failures** (692 workspace + 195 nixling-priv-broker;
the broker crate is `exclude`d from the workspace per ADR 0002 and
must be tested separately with `cd packages/nixling-priv-broker &&
cargo test --lib`). All 9 panels signed off (panel-{rust, virt,
kernel, networking, security, software, test, docs, product, build}),
plus the v1.x retrospective panel cycle (10 disciplines, vx-rust
through vx-build) confirmed PASS with v1.2 follow-up items tracked.

### Consumer-visible changes (NO MANUAL MIGRATION REQUIRED)

- **TPM socket path moved** from `/run/swtpm/<vm>/sock` to
  `/run/nixling/vms/<vm>/tpm.sock`. Both halves of the wiring
  (host-side swtpm sidecar argv + guest-side cloud-hypervisor
  `--tpm socket=...`) update in lockstep on `nixos-rebuild switch`.
  Operators must NOT preserve old `/run/swtpm/` paths — the
  framework will not create them anymore. (fu36)
- **`umask` field added to `MinijailProfile` schema.** Optional
  `Option<u32>` field; default `null` preserves the broker's
  inherited umask (current behaviour). Sidecar role profiles
  (swtpm, audio, gpu) now declare `umask = 0o007` so their bound
  Unix sockets get mode 0660 and the per-VM-runtime default ACL
  named-user entries (granting cloud-hypervisor's UID rw) become
  effective. Backward-compat: old bundles without the field
  deserialise to `None` and behave exactly as before. (fu36)
- **System user UIDs aligned with `stablePrincipalId` hash.**
  The minijail profile UIDs and `/etc/passwd` entries for
  `nixling-<vm>-{swtpm,gpu,snd,runner}` now share the same
  hash-derived UID. Pre-v1.1.2-final installs that had the
  NixOS-auto-assigned UIDs (range 100-999) get the new UIDs on
  `nixos-rebuild switch`; existing state directories under
  `/var/lib/nixling/vms/<vm>/{swtpm,gpu-state,…}` automatically
  get `chown`'d to the new UIDs by the activation script.
  (fu35)
- **Cloud-hypervisor 52 required.** This release uses variadic
  `--fs sock1,tag1 sock2,tag2` argv form (CH 52's clap parser
  change). Earlier CH versions are no longer supported. (fu30)
- **`microvm` flake input is NOT required** for consumers using
  `inputs.nixling.nixosModules.default` as their import boundary.
  Consumer flakes that still declare `inputs.microvm` can drop
  it unless they consume microvm.nix directly for non-nixling
  VMs (none typical).

### What changed (internal)

- **fu25** (`ssh_host_key_preflight.rs`) — accept mode 0440
  when POSIX ACL xattr present (`system.posix_acl_access`).
- **fu27** (`sys.rs`) — skip `apply_mount_actions` when
  `in_ns_credentials = true`. Linux locks inherited mounts inside
  a user namespace; bind-mount onto an inherited mount returns
  EPERM regardless of in-NS capabilities. virtiofsd's
  `--sandbox=chroot` does its own `pivot_root` inside the NS
  where it has in-NS CAP_SYS_ADMIN.
- **fu30** (`processes-json.nix`) — emit variadic
  `--fs`/`--net`/`--disk`/`--device` argv for cloud-hypervisor
  52. The `repeatedFlagArgs` helper is retained for legacy
  callers; new `variadicFlagArgs` handles the CH 52+ form.
- **fu31** (`processes-json.nix`) — `vsockPath` default is now
  absolute (`/var/lib/nixling/vms/${name}/notify.vsock`). Was a
  relative `"notify.vsock"` resolving to the broker's CWD `/`.
- **fu32** (`pidfd_table.rs`) — fix tmpfile race in
  `PidfdTable::snapshot()`. The previous `<path>.tmp` filename
  caused concurrent autostart-spawned virtiofsd registrations to
  collide ~50% of the time. New filename is `<path>.tmp.<pid>.
  <atomic_seq>`. Also: best-effort tmpfile cleanup on every
  error path (with DEBUG-level logging on cleanup failure).
- **fu33** (`minijail-profiles.nix`) — bind `/dev/net/tun` into
  the cloud-hypervisor runner sandbox. CH opens `/dev/net/tun`
  + `TUNSETIFF` to attach to a pre-existing persistent TAP
  created by the broker's `CreatePersistentTap` op. Without the
  device node in the sandbox mount-NS, CH reports `EBUSY` (a
  misleading errno from the kernel rejecting the missing
  inode).
- **fu34** (`lib.rs`) — `wait_for_one_shot_exit` detects zombie
  state (`/proc/<pid>/stat` field 3 = `Z` or `X`). The broker
  spawns OneShot DAG nodes (swtpm-flush etc.) but doesn't
  explicitly `waitid(2)` to reap; the child becomes a zombie
  whose `/proc/<pid>/stat` still reports the original starttime.
  Treating `Z`/`X` as terminated unblocks the DAG instead of
  spinning until the 30s `oneshot-timeout`. Returns explicit
  `ProcState::{Alive, Gone, ParseFailed}` enum so callers can
  distinguish parse-failure from process-gone.
- **fu35** (`host-users.nix`) — declare system user UIDs using
  the `stablePrincipalId` hash so the on-disk owner, the
  ownership-matrix entry, and the broker setuid target all
  agree. Hash extracted to `nixos-modules/lib.nix` as the
  canonical definition (panel-software-R1 fix).
- **fu36** (cross-cutting, 14 files) — `umask: Option<u32>`
  plumbed through `MinijailProfile` → `RoleProfile` →
  `ResolvedRunnerIntent` → `SpawnRunnerPlanInput` →
  `SpawnRunnerPlan` → `RunnerIsolationSpec`. Broker child
  closure calls `umask(2)` immediately before `execve(2)`.
  Sidecar profiles set `umask = 0o007`. TPM socket consolidated
  under per-VM runtime dir (see consumer-visible changes above).
- **fu37 (R1)** — panel-review fixes:
  - Extract `stablePrincipalId` to `nixos-modules/lib.nix`
    (panel-software HIGH; eliminates the drift risk between
    `host-users.nix` and `minijail-profiles.nix`).
  - `read_proc_state` returns explicit `ProcState` enum and
    logs ParseFailed at WARN level (panel-software HIGH +
    panel-test HIGH; eliminates the silent 30s spin on
    transient parse failures).
  - Add umask range validation in the broker child closure
    (panel-rust SHOULD-FIX; reject `umask > 0o777` with
    `CHILD_EXIT_INVALID_UMASK=75`).
  - Add `tracing::debug!` cleanup logging in `pidfd_table.rs`
    `write_snapshot` error paths (panel-rust SHOULD-FIX).
  - Tighten `DeviceClass::NetTun` ioctl allowlist to
    `[TUNSETIFF, TUNSETGROUP]` only (panel-security SHOULD-FIX;
    `TUNSETPERSIST`/`TUNSETOWNER` are broker-only and bypass
    the per-role policy via raw libc::ioctl).
  - Inline-comment clarification at the `/dev/net/tun`
    deviceBind site explaining why CH cannot escape via rogue
    TAP creation (panel-networking SHOULD-FIX).
  - Add 10+ regression tests:
    - `proc_state_tests` module (8 unit tests for the
      `read_proc_state` parser covering comm-with-`)`,
      truncated stat, etc.)
    - `snapshot_under_concurrent_load_succeeds` (8-thread
      stress test of `PidfdTable::snapshot`)
    - `snapshot_tmp_path_is_unique_per_call`
    - `isolation_spec_umask_field_*` (umask plumbing
      verification)
    - `umask_validation_bound_is_0o777`

### Known limitations / deferred

- **Seccomp BPF compilation from `ioctl_policy.rs` matrix**
  is not yet wired. `load_runner_seccomp` returns `Ok(None)`
  when `seccomp_policy_ref` is a non-absolute reference (e.g.
  `"w1-cloud-hypervisor-runner"`). The declarative ioctl
  allowlist serves as the source of truth that future BPF
  compilation must honour. Tracked for v1.2.
- **Test fixture builder** for `RoleProfile` /
  `ResolvedRunnerIntent` not yet introduced. Future optional
  fields will continue to require manual updates across ~5 test
  files. Tracked for v1.2.
- **CLI readiness-timeout false negative.**
  `nixling vm start <vm> --apply` may return a non-zero exit
  code with `SpawnRunner failed at cloud-hypervisor` even when
  cloud-hypervisor spawns successfully and the VM is fully
  functional (network, SSH, TPM, all sidecars working). The
  daemon's readiness predicate polls CH's API socket for up to
  30 seconds after spawn; CH 52 occasionally takes longer than
  this window to bind the API socket on slower hosts or under
  contention, and the gate fires `readiness-timeout:cloud-
  hypervisor` even though the VM continues to boot and reach
  the guest-ssh-ready state.
  **Operator remediation:** ignore the CLI exit code; verify
  with `ping <vm-ip>` or `ssh <vm-ip>`. The VM state is
  authoritative. The pidfd-table snapshot at
  `/var/lib/nixling/daemon-state/pidfd-table.json` shows the
  live CH PID.
  **v1.2 fix tracked.** Two candidate fixes under evaluation:
  Option A — extend the CH-runner readiness timeout from 30s
  to 60s (simple; covers the slow-bind case at the cost of
  longer fail-closed feedback on real failures). Option B —
  split the readiness gate into a fast `process-alive` check
  (pidfd validation, returns immediately) and a slower
  `api-ready` check (CH API HTTP probe, runs async). Option B
  is the cleaner design but requires DAG-node decomposition.
- **fu27 mount-action skip branch** lacks a hermetic unit
  test. The `if !in_ns_credentials { apply_mount_actions(...) }`
  guard in the broker child closure is exercised exhaustively
  by every virtiofsd spawn (ADR 0021 user-NS path) in live
  deploy, but no test in `cargo test --workspace` directly
  asserts the skip behaviour. Tracked for v1.2 as a focused
  unit test that spawns with `user_namespace = Some(...)` +
  `mount_actions` non-empty and asserts `apply_mount_actions`
  is not invoked.

### Compatibility

- Bundle schema: backward-compatible (additive optional
  `umask` field; old bundles deserialize with `umask = None`).
- Wire protocol: unchanged.
- CLI: no verbs changed; no flags added or removed.

## v1.1.2 — 2026-06-02 — broker-pre-established user namespace for virtiofsd + live-bring-up hardening

v1.1.2 closes the v1.1.1 → live-VM-bring-up gap by retiring the
`virtiofsd --sandbox=namespace + requiresStartRoot=true` carve-out
from [ADR 0003](docs/adr/0003-minijail-provisioning-and-sandbox-interface.md)
in favour of a fully broker-pre-established user namespace
([ADR 0021](docs/adr/0021-broker-user-namespace-for-virtiofsd.md)).
virtiofsd now runs with **zero** host capabilities; fake-root
identity exists only inside the per-runner user NS. This is
strictly stronger than v1.1.1: no `CAP_SYS_ADMIN`, no
`CAP_DAC_OVERRIDE`, no `CAP_DAC_READ_SEARCH`, no `CAP_SETUID`,
no `CAP_SETGID`, no `/etc/subuid` provisioning. Series HEAD:
fu7 .. fu19, with 9-discipline panel R1+R2 closure cycle
(unanimous panel sign-off after R2).

### What changed

- **ADR 0021** — broker-pre-established user namespace for
  virtiofsd. New ADR documenting the v1.1.2 sandbox decision,
  rejected alternatives (virtiofsd self-NS requiring
  `/etc/subuid`/`/etc/subgid` + setuid `newuidmap`/`newgidmap`;
  `--sandbox=none`; the ADR 0003 root carve-out), implementation
  contract pseudocode (clone3 + pipe2 sync + uid_map writer),
  consequences, and test coverage. ADR 0003 marks the
  `requiresStartRoot` carve-out as superseded; ADR README adds
  the row.
- **Broker user-namespace plumbing.** New `UserNamespaceSpec`
  (`host_uid_for_zero`, `host_gid_for_zero`) threaded through
  `SpawnRunnerPlanInput`, `RunnerIsolationSpec`,
  `ResolvedRunnerIntent`, `RoleProfile`, `MinijailProfile`. The
  broker's `clone3_spawn_runner` now does a pipe2(O_CLOEXEC)-sync
  dance: child closes inherited write_fd, blocks on read; parent
  writes `/proc/<pid>/uid_map` (`0 host_uid 1`) → `setgroups=deny`
  → `gid_map` (`0 host_gid 1`); parent writes 1 byte to unblock
  child. Child then `setgid(0)` / `setuid(0)` to in-NS root,
  SKIPS `setgroups()` entirely (would EPERM), `capset()`, exec.
  New `CHILD_EXIT_USER_NS_SYNC=74` exit code for parent-death
  scenarios.
- **virtiofsd minijail profile changes.** `capabilities = []`,
  `requiresStartRoot = false`, `userNamespace = {
  hostUidForZero, hostGidForZero }` derived from
  `stablePrincipalId("nixling-<vm>-<role>-runner")`. Carve-out
  reference text updated to cite ADR 0021. virtiofsd argv now
  uses `--sandbox=chroot --inode-file-handles=never`; ro-store
  shares add `--readonly`.
- **Operator-workaround codification.** Activation script
  `nixlingRuntimeDirPosture` re-asserts ownership/mode on
  `/run/nixling/{locks,state}` and per-VM `store`/`store-meta`
  on every activation. `nixlingd` daemon gains
  `PidfdTable::prune_dead_entries` (validates pid + start_time
  against `/proc/<pid>/stat`) called from vm-start handler to
  drop stale pidfd-table entries from prior runs. `nixlingd`
  unit gains `extraGroups += "nixling-launchers"`. Activation
  script SECURITY-hardens against TOCTOU on the role-UID-writable
  VM dir: `store-overlay.img` creation refuses symlinks; `*.img`
  loop replaced with `find -type f` that does not follow symlinks;
  `nixlingRuntimeDirPosture` refuses symlinks on every path it
  touches. per-keyfile ACL grants on `ssh_host_*_key` permit
  virtiofsd-nl-ssh-host runner read access inside its user NS.
- **W3 altname collision detection.** Activation script
  `nixlingW3IfNameAltnames` no longer silently swallows ALL
  errors when adding altnames to user-visible bridges. It now
  compares ifindex of `$user` vs `$derived`; if `$derived`
  already resolves to a DIFFERENT interface (foreign altname
  collision), it logs loudly and exits 1 instead of letting
  the broker silently route to the wrong device.
- **`nixling vm konsole <vm>` CLI verb.** Spawns a terminal
  emulator (default `konsole`, overridable) hosting an SSH
  session into the named VM. Resolves user/host/key from the
  manifest + bundle.managed_keys; `--user $USER` fallback
  (replaces a previous hardcoded-username fallback);
  validates key existence BEFORE emitting `--json` output;
  propagates setsid exit-status as typed exit-1 envelope.
  Supports `--dry-run`, `--json`, `--user`, `--host`, `--key`,
  `--terminal`.
- **Validation tests.** Plan-layer `user_namespace_round_trips_*`
  tests; sys-layer `user_namespace_true_requires_spec` +
  `user_namespace_spec_requires_namespace_flag` defensive
  validation tests. virtiofsd minijail-validator updated for
  the v1.1.2 shape (zero caps + userNamespace required).
  broker-caps-eval canonical-caps set updated to the v1.1.1fu10
  15-cap list. Error-codes reference regenerated via
  `cargo xtask gen-error-codes`.

### Verification

| Surface | Result |
| --- | --- |
| `cargo test --workspace` | **890 / 890 pass** (38 nixling, 41 nixling-core, 344 nixling-host, 43 nixling-ipc, 232 nixlingd + 7 ignored, 192 nixling-priv-broker + 1 root-gated ignored) |
| `nixos-rebuild eval` | clean |
| 13 v1.1 invariant gates | all PASS |
| ADR 0003 supersession marker | present + cross-linked from ADR 0021 |
| `docs/adr/README.md` ADR 0021 row | present |
| 9-discipline panel | unanimous panel signoff after R2/R3 closure cycle (fu14..fu20) |

### Migration from v1.1.1

Operators bumping the nixling flake input from v1.1.1 to v1.1.2:

1. No flake input changes required.
2. `nixos-rebuild switch` will:
   - Update the broker binary (new sys.rs user-NS path).
   - Update the activation script (new symlink-refusal + altname
     collision gates).
   - Update the virtiofsd minijail profile (zero host caps +
     userNamespace).
3. **Daemon restart required**: `nixlingd.service` has
   `restartIfChanged = false` (by design — restarting the
   daemon mid-VM-flight would disrupt pidfd supervision). The
   new daemon-side pidfd-prune logic only takes effect after
   an explicit restart. After `nixos-rebuild switch` completes,
   stop all running VMs, then run:
   ```
   sudo systemctl restart nixling-priv-broker.socket nixling-priv-broker.service nixlingd
   ```
   then `nixling vm start --apply <vm>` to bring them back up
   with the new code paths.
4. Any running virtiofsd processes will be restarted on next
   `nixling vm start --apply <vm>` (the new minijail profile
   shape differs from v1.1.1's).
5. The previously-required manual reset sequence between
   `nixling vm start --apply` attempts (per the v1.1.1fu13
   live-deploy session notes) is no longer needed: the new
   activation script + daemon prune logic codify what was
   previously documented as operator-side workaround.
6. No `/etc/subuid` / `/etc/subgid` provisioning required.
7. No kernel-floor bump beyond the existing v1.1 requirement
   of Linux ≥ 6.9 ([ADR 0008](docs/adr/0008-supported-platforms-and-rejected-targets.md)).

### Compatibility

- Bundle schema: unchanged (additive `user_namespace` field on
  spawn-runner role profiles; absent → `None` → no NS).
- Wire protocol: unchanged for non-virtiofsd roles.
- CLI: new `nixling vm konsole <vm>` verb; no existing verbs
  changed.

### Known limitations

- v1.2 will extend broker-pre-NS to gpu/audio/swtpm roles
  pending device-bind compatibility analysis.
- `crosvmVideo` overlay still gated off via
  `graphics.videoSidecar = false` default pending nixpkgs 26.05+
  rebuild. Consumers can opt in at their own rebuild cost.
- `writableStoreOverlay` disk-init not yet broker-spawned;
  consumers can opt in by providing a backing image themselves.

## v1.1.1 — 2026-06-01 — zero-defer closures (fu1–fu6) — 9/9 unanimous signoff (R6 closure cycle)

v1.1.1 closes every v1.1 deferred item via a TDD-first fix-up
sequence (`v1.1.1fu1` .. `v1.1.1fu6`). Tag `v1.1.1` is the
9/9-unanimous-panel-signed-off HEAD `9ba10ee` (R5 returned 6
SIGNOFF + 3 NEEDS_FIXES on rust/product/docs; fu6 closed all
three, and the R6 closure-round panel returned 3 SIGNOFF).
No behavior regressions vs v1.1; all ~723 workspace tests pass,
all 13 v1.1 invariant gates remain green, and `cargo xtask
gen-schemas` remains clean.

### What shipped

- **fu1 (`e680db6`) — legacy env-var test scaffolding stripped.**
  Removed every `NIXLING_LEGACY_BASH_OPT_IN` /
  `NIXLING_NATIVE_ONLY` / `NIXLING_LEGACY_CLI_PATH` `EnvVarGuard`
  reference from `packages/nixling/src/lib.rs`. Pruned
  `dispatch_mutating_verb` from 8 → 6 arguments (dropped
  unused `legacy_args` + `legacy_fallback_warning`); updated
  all 9 call sites.
- **fu2 (`a029cc4`) — `fchownat(AT_EMPTY_PATH)` on O_PATH fix
  + cgroup taxonomy migration + USBIP Attach/Detach + guest-ssh.**
  `packages/nixling-host/src/cgroup.rs` now uses
  `fchownat(fd, "", uid, gid, AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW)`
  instead of the broken `fchown` on an O_PATH descriptor. New
  v1.1.1 cgroup taxonomy: `create_vm_subtree` returns the
  process-free interior `<slice>/<vm>/`; `create_vm_role_leaf`
  creates `<slice>/<vm>/<role>/`. `CgroupBundleContext` exposes
  `vm_interior_path` + `vm_role_leaf_path`; the legacy
  `vm_leaf_path` is `#[deprecated]` and aliases interior.
  `UsbipSubcommand` extended with `Attach` + `Detach` variants
  + new `GuestUsbipSshInput` + `generate_guest_usbip_ssh_argv`
  with hardened ssh args (`-F /dev/null`, `BatchMode=yes`,
  `ControlMaster=no`, `ControlPersist=no`, `StrictHostKeyChecking=yes`,
  `-i <identity>`).
- **fu3 (`bc929da`) — per-role runner_argv_regenerator wiring +
  RenderDnsmasqEnvConf module + AST walker.** `runner_argv_regenerator`
  now dispatches for all 8 `SpawnRunner` roles (was Cloud
  Hypervisor only); `RunnerArgvExtra` carries per-role
  `Option<*ArgvInput>` fields + a `UsbipSubcommand` selector.
  New `packages/nixling-host/src/dnsmasq.rs` (~220 LOC + 5
  tests) implements the pure-Rust dnsmasq config renderer
  for the `RenderDnsmasqEnvConf` broker op.
  `tests/tools/no-bash-ast-walker/` is a new Cargo crate
  (syn-based AST visitor) that replaces the previous SKIP/
  delegate behavior of `tests/no-bash-exec-eval.sh`'s
  `syn-ast-walk` mode.
- **fu4 (`94d65a7`) — clone3(CLONE_INTO_CGROUP) atomic placement
  + broker live_handlers v1.1.1 taxonomy.** New
  `clone3_pidfd_or_fork_fallback_with_cgroup(extra,
  into_cgroup_dirfd, child_main)` ORs in
  `CLONE_INTO_CGROUP = 0x2_0000_0000` and sets `args.cgroup =
  dirfd` so the spawned child lands atomically in the target
  cgroup. The legacy wrapper is preserved. Broker
  `live_handlers.rs` `cgroup_leaf_path` migrated to the v1.1.1
  `<slice>/<vm>/<role>/` taxonomy.
- **fu5 (`8fd58a3`) — pidfs runtime self-probe + StatusOutputV3
  wire schema.** New `packages/nixlingd/src/pidfs_probe.rs`
  (~200 LOC + 5 tests) runs a runtime pidfs self-probe via
  `rustix::process::pidfd_open` + `fstat`. The probe HARD-
  REFUSES startup on a kernel without pidfs unless the
  operator sets `NIXLING_ALLOW_PIDFS_PROBE_SOFT_FAIL=1`.
  Wired into `serve()`. New
  `StatusServicesOutputV3` struct + `from_v2()` migration shim
  in `packages/nixling/src/lib.rs` adds the v1.1.1 wire
  schema fields (`hypervisor`/`audio`/`virtiofsd_per_share`/
  `otel_relay`/`otel_host_bridge`/`usbip_{backend,proxy}_per_env`).

### Schema bump status

The v1.1.1 release ships the StatusOutputV3 wire schema
(`StatusServicesOutputV3` + `from_v2` migration shim is publicly
exported from `nixling::lib`). The CLI `nixling status` command
still EMITS the v1.0/v1.1 `StatusServicesOutputV2` shape at
v1.1.1; the emit-side flip to V3 is scheduled for v1.1.2.
Tooling authors should consult
[`docs/how-to/migrate-nixling-v1-0-to-v1-1.md`](docs/how-to/migrate-nixling-v1-0-to-v1-1.md)
§ "nixling status output schema" for the rename map and the
incremental adoption recipe.

### Verification

- All ~723 workspace tests PASS (nixling 38, nixling-core 41,
  nixling-host 344, nixling-ipc 43, nixlingd 232 + 7 ignored,
  nixling-priv-broker 188; xtask 11; misc 16).
- All 13 v1.1 invariant gates PASS.
- AST walker is REAL (`tests/tools/no-bash-ast-walker/`); 0
  bash-literal sites in the Rust binary path.
- `cargo xtask gen-schemas` clean.
- `tests/release-tag-eval.sh` PASSES: tag `v1.1.1` is annotated,
  points at `9ba10ee`, message contains the literal substring
  `9/9 unanimous panel signoff`.

### fu6 R5-closure-round changes

The R5 9-discipline panel returned 6 SIGNOFF (security, virt,
kernel, networking, software, test) + 3 NEEDS_FIXES (rust,
product, docs). fu6 closed all three:

- **rust closure**: `[workspace.lints.rust] unexpected_cfgs`
  check-cfg entry declares `cfg(test_root)` so the 4 root-
  gated broker_dispatch_tests no longer warn `unexpected_cfgs`;
  `nixling_host::runner_argv_regenerator::regenerate_argv`
  wired into broker `SpawnRunner` dispatch as a no-op tamper
  check (v1.1.2 wire-cleanup will make the diff a hard
  failure once the bundle schema carries typed argv inputs).
- **product closure**: migration guide § "nixling status output
  schema" rewritten to acknowledge v1.1.1 ships V3 wire schema
  while CLI emit remains V2 until v1.1.2; tooling-author
  incremental adoption recipe added.
- **docs closure**: this CHANGELOG v1.1.1 section + the
  migration-guide rewrite.

R6 closure-round panel (3 disciplines) returned 3 SIGNOFF.

### Operator-required steps

`nixos-rebuild switch` + reboot + `nixling vm start --apply
personal-dev` + `nixling vm start --apply work-aad` are
explicitly OPERATOR-required and CANNOT be remotely verified.
The v1.1.1 tag message + the migration guide both call this
out.

## v1.1 — 2026-05-31 — daemon-only follow-through COMPLETE

v1.1 ships the full daemon-only follow-through: the `microvm.nix`
flake input is REMOVED, nixling owns its per-VM substrate
end-to-end, and all 13 v1.1 invariant gates PASS.

### What changed vs v1.1-rc2

**v1.1-P11 + P9b (substrate replacement — `inputs.microvm` REMOVED):**
- NEW `nixos-modules/vm-options.nix`: nixling-owned per-VM
  `microvm.*` option set (hypervisor, vcpu, mem, hotplugMem,
  hugepageMem, balloon, storeOnDisk, kernel, kernelParams,
  initrdPath, vsock.cid, interfaces, shares, devices, volumes,
  cloud-hypervisor.{package, extraArgs, platformOEMStrings},
  virtiofsd.{package, threadPoolSize, group, inodeFileHandles,
  extraArgs}, graphics.*). NO upstream microvm.nix dependency.
- NEW `nixos-modules/vm-evaluator.nix`: per-VM NixOS evaluator
  using `${pkgs.path}/nixos/lib/eval-config.nix` (standard
  NixOS evaluator) layered with vm-options.nix and the
  consumer's composed per-VM module list.
- `nixos-modules/vm-submodule.nix` rewritten as a thin wrapper
  exposing `composeVm`.
- `nixos-modules/host.nix`:
  - DROPPED `inputs.microvm.nixosModules.host` import.
  - DROPPED `microvm.stateDir`, `microvm.autostart`,
    `systemd.targets.microvms.wants` writes.
  - REPLACED `microvm.vms = lib.mapAttrs ...` with
    `nixling.vms = lib.mapAttrs (name: vm: vm // { computed = composeVm ... })`.
  - DROPPED per-VM `microvm@<vm>.service` fail-fast stub units
    (upstream templates don't exist anymore).
- `nixos-modules/lib.nix` helpers (`vmRunner`, `vmToplevel`,
  `vmDeclaredRunner`) read from
  `config.nixling.vms.<name>.computed.config.*` —
  nixling-owned source.
- `nixos-modules/store.nix`: `microvm.vms = ...` writes
  rewritten as `nixling.vms = lib.mapAttrs ... { config = ...; }`
  (deferredModule merge into each VM's composedConfig).
- `nixos-modules/options-vms.nix`: added internal
  `nixling.vms.<name>.computed` option.
- `flake.nix`: `microvm` input DROPPED from `inputs`; outputs
  function signature updated; `flake.lock` regenerated (no
  `microvm` entries).

**Rust port (already in place, now canonical):**
- `packages/nixling-host/src/ch_argv.rs` (623 LOC),
  `virtiofsd_argv.rs` (509 LOC), `gpu_argv.rs` (449 LOC),
  `audio_argv.rs` (298 LOC), `otel_host_bridge_argv.rs`
  (268 LOC), `swtpm_argv.rs` (491 LOC), `usbip_argv.rs`
  (703 LOC), `video_argv.rs` (321 LOC),
  `vsock_relay_argv.rs` (610 LOC) — **4272 LOC of Rust
  argv generators** are the canonical runner-argv
  generation surface. The broker
  (`packages/nixling-priv-broker/src/runtime.rs`) consumes
  typed runner intents via the bundle resolver and dispatches
  through `nixling-host`'s argv generators.
- The Nix-side argv generation in `processes-json.nix` is
  retained for bundle backward-compat (the broker reads the
  prebuilt argv from the bundle's runner-intent record) but
  the canonical generators are Rust. A future cleanup will
  remove the Nix-side duplication once
  `bundle_resolver::ResolvedRunnerIntent::regenerate_argv()`
  becomes the broker's only argv source.

### v1.1 gate matrix — ALL 13 PASS

1. no-bash-exec-eval (3 modes) — PASS
2. supervisor-option-absent-eval — PASS
3. broker-systemd-unit-eval — PASS
4. daemon-experimental-warning-eval — PASS
5. state-dir-acl-eval — PASS
6. otel-acl-migration-eval — PASS
7. vfsd-watchdog-retired-eval — PASS
8. processes-json-eval — PASS
9. vm-submodule-eval — PASS
10. kernel-modules-parity-eval — PASS
11. vm-submodule-cutover-eval — PASS
12. v1.1-kernel-floor-eval — PASS
13. **microvm-nix-absent-eval — PASS (the substrate replacement gate)**

### Post-tag fix commits (panel R1 closures)

- `dac1071` (v1.1-P13fu1): bump `tests/golden/vms.json-91d69b0`
  manifestVersion 2 → 3 to match the v1.0 P2 parser bump
  (`manifest_baseline_round_trips_compact` was pre-existing
  failure unrelated to v1.1, surfaced by panel-prep all-targets
  cargo test sweep).
- `0b60f3c` (v1.1fu1): R1 panel-round closures across 9
  disciplines. Substrate fixes:
  - Broke module-system infinite recursion: introduced
    `nixling._computed` sibling option (lib.nix helpers route
    through it); deleted `nixos-modules/options-vms-removed.nix`
    (mkRemovedOptionModule incompatible with `attrsOf submodule`).
  - Moved per-VM nix-store + meta + host-keys + obs-secrets
    shares from store.nix to host.nix's composeVm pipeline.
  - Deleted store.nix's `microvm-virtiofsd@<vm>` systemd
    drop-ins (template no longer exists).
  - Rewrote vm-evaluator.nix `composeVm` to accept a module
    LIST (not lib.mkMerge) so `eval-config.nix` accepts it.
  - Fixed `nl.vmDeclaredRunner` returning null breaking
    closures-json.nix consumers (returns derivation now).
  - Fixed `regenerate_argv` argv[0] = process-title (was
    binary-path).
  - Fixed `g:kvm:--x` ACL grant (was `u:kvm:--x`; kvm is a
    Linux group, not a user).
  - Fixed `daemon_down_envelope` broken anchor link.
  - Net-VM bundle gate `ConfigMissing` soft-defers (was
    hard-fail; v1.1.1 ships RenderDnsmasqEnvConf to render
    pre-start).
  - ADR 0017/0018 status → "Implemented in v1.1".
  - Migration guide v1.1-P8..P11 section → "COMPLETE"; status
    rename map → v1.1.1 PLANNED.
  - design.md tagline → "owns its microVM substrate"; History
    subsection added.
  - host.nix composeVm comments updated.
  - observability-host-secrets.nix obs-share comment now
    points at host.nix.
  - kernel-modules-parity-eval.sh + supervisor-option-absent-eval.sh
    + state-dir-acl-eval.sh rewritten to match the actual
    v1.1-final invariants.
  - no-bash-exec-eval syn-ast-walk mode delegates to `check`
    (was silent SKIP).

### Tests

- nixling: 38 / 38
- nixling-core: 41+2 (P3 new) / 43
- nixling-host: 335 → 337 (regenerator added 2 tests) / 337
- nixling-ipc: 43 / 43
- nixlingd: 227 / 227 (7 ignored)
- 9 integration test suites (smoke / bundle / privileges /
  manifest / fuzz harnesses): 79 total / 79 pass
- `cargo xtask gen-schemas`: clean
- `nix eval` of `nixosModules.default` closure: succeeds

## v1.1-rc2 — 2026-05-31 — substrate-replacement consumer cut-over

v1.1-rc2 builds on rc1 by substantively rehoming the four `microvm.*`
consumer modules to nixling-owned access helpers and introducing
the `vm-submodule.nix` ownership structure. The substrate
replacement is now 4 of 5 invariant gates PASSING; only the final
`inputs.microvm` flake input drop (gated by
`tests/microvm-nix-absent-eval.sh`) remains for v1.1-final.

### v1.1-rc2 deltas

- **v1.1-P8 (substantive)**: `nixos-modules/lib.nix` gained three
  helpers — `vmRunner config name`, `vmToplevel config name`,
  `vmDeclaredRunner config name`. Every reader in
  `processes-json.nix` / `closures-json.nix` /
  `minijail-profiles.nix` / `store.nix` now routes through these
  helpers instead of reading
  `config.microvm.vms.<name>.config.config.microvm.*` directly.
  At v1.1-rc2 the helper bodies still delegate to upstream
  microvm.vms; at v1.1-final they swap to the
  nixling-owned vm-submodule.nix evaluator without touching
  consumer sites.
- **v1.1-P9a (substantive)**: NEW `nixos-modules/vm-submodule.nix`
  with the `composeVm name vm` function that owns the per-VM
  module-merge sequence. At v1.1-rc2 the function delegates
  per-VM evaluation to upstream microvm.vms (structural ownership
  move); at v1.1-final the body switches to a nixling-owned
  `lib.evalModules` call.
- **v1.1-P10 (partial)**: NEW `tests/v1.1-kernel-floor-eval.sh`
  static gate — asserts ADR 0008 declares the v1.1 `>= 6.9` floor
  AND the migration guide cross-links the prerequisite. The
  runtime pidfs self-probe in
  `packages/nixlingd/src/startup.rs` is the defense-in-depth
  for custom kernels.
- **Gates flipped from SKIP → PASS**: `processes-json-eval`,
  `vm-submodule-eval`, `kernel-modules-parity-eval`,
  `vm-submodule-cutover-eval` (4 of 5 substrate gates).

### Remaining for v1.1-final

- `tests/microvm-nix-absent-eval.sh` still SKIP — dropping
  `inputs.microvm` requires the nixling-owned per-VM evaluator
  (vm-submodule.nix `composeVm` switched to `lib.evalModules`)
  to replace the upstream `microvm.vms` evaluation pipeline.
  Multi-day implementation work plus live-host validation
  against every example VM.
- `microvm@<vm>.service` / `microvm-virtiofsd@<vm>.service` /
  `nixling-<vm>-store-sync.service` systemd templates — at
  v1.1-rc2 the per-VM templates are overridden to fail-fast
  stubs for every daemon-supervised VM (which is now every
  enabled VM after v1.1-P2). The upstream templates' physical
  presence in the systemd unit graph is moot but goes away
  entirely when v1.1-final drops the upstream module import.

## v1.1-rc1 — 2026-05-31 — daemon-only follow-through (partial)

v1.1-rc1 ships P0..P7 + P12 substantively. The substrate-replacement
phases (P8..P11 — re-homing per-VM reads from
`config.microvm.vms.<vm>.config.config.microvm.*` to
`nixling.vms.<vm>.runner.*`, the new vm-submodule.nix evaluator,
the `microvm.vms` translation removal, the `inputs.microvm` flake
input drop) are tracked by SKIP-mode invariant gates at v1.1-rc1
and ship in v1.1-rc2 / v1.1-final. See
[`docs/how-to/migrate-nixling-v1-0-to-v1-1.md`](docs/how-to/migrate-nixling-v1-0-to-v1-1.md)
for the operator-facing migration steps.

### Retired from v1.0 deferral list

| v1.0 "Deferred to follow-up" entry                                  | Closed in    | Notes                                                                                                                                  |
| ------------------------------------------------------------------- | ------------ | -------------------------------------------------------------------------------------------------------------------------------------- |
| Bash fallback shim (`exec_legacy_passthrough`)                      | v1.1-P1      | Deleted; verbs now emit typed envelopes directly. New gate `tests/no-bash-exec-eval.sh`.                                               |
| `nixling.vms.<vm>.supervisor` option (ADR 0015 deferral)            | v1.1-P2      | Removed via per-submodule `mkRemovedOptionModule` shim + fallback assertion.                                                           |
| `host-broker.nix` `daemonExperimental.enable` gating                | v1.1-P4      | Broker socket + service now default-on. Deprecation warning when consumer flake still sets the option.                                 |
| `nixling-vfsd-watchdog@.{service,timer}`                            | v1.1-P7      | Retired; wedge detection moved to broker Virtiofsd SpawnRunner pidfd supervisor.                                                       |
| `host-otel-relay-acl.nix`                                           | v1.1-P6      | Retired from public default.nix imports; OTel host-bridge ACL moved to broker pre-spawn pipeline.                                      |
| `nixling-otel-relay@<vm>.service` / `nixling-otel-host-bridge.service` | v1.1-P6   | Replaced by `RunnerRole::OtelHostBridge` broker SpawnRunner (already in v1.0 source; v1.1-P6 retires the systemd surface).             |
| `microvm@<vm>.service` / `microvm-virtiofsd@<vm>.service` / `nixling-<vm>-store-sync.service` | v1.1-P10 (scheduled, gates SKIP at rc1) | Substrate replacement landing in v1.1-rc2 / v1.1-final.                                                              |
| `nixling.daemonExperimental.enable` option (no-op since v1.0)       | v1.1-P4      | Warning emitted via `warnings = lib.optional` on consumer use; option declaration retained for backward-compat module evaluation.       |

### v1.1-rc1 invariant gates added

- `tests/no-bash-exec-eval.sh` (3 modes; check / fixture-coverage / syn-ast-walk).
- `tests/fixtures/no-bash-exec-exempt-paths.json` (empty allow-list).
- `tests/supervisor-option-absent-eval.sh`.
- `tests/broker-systemd-unit-eval.sh`.
- `tests/daemon-experimental-warning-eval.sh`.
- `tests/state-dir-acl-eval.sh`.
- `tests/otel-acl-migration-eval.sh`.
- `tests/vfsd-watchdog-retired-eval.sh`.
- `tests/processes-json-eval.sh` (SKIP at v1.1-rc1; enforces at v1.1-rc2/final).
- `tests/vm-submodule-eval.sh` (SKIP at v1.1-rc1).
- `tests/kernel-modules-parity-eval.sh` (SKIP at v1.1-rc1).
- `tests/vm-submodule-cutover-eval.sh` (SKIP at v1.1-rc1).
- `tests/microvm-nix-absent-eval.sh` (SKIP at v1.1-rc1).

### Source-code summary by phase

- **v1.1-P1**: deleted `exec_legacy_passthrough` /
  `should_fallback_to_legacy` / helper functions / `legacy_cli`
  Context field / `DEFAULT_LEGACY_CLI` constant in
  `packages/nixling/src/lib.rs`; added typed envelope helpers
  `daemon_down_envelope` / `not_yet_implemented_envelope`;
  rewrote `cmd_audit` / `cmd_console` / `cmd_audio` /
  `cmd_keys_{list,show}`; deleted `tests/cli-legacy-bash-dispatch.sh`;
  rewrote `native_help_requests_*` in-source test to use clap
  directly.
- **v1.1-P2**: deleted productive `supervisor` option from
  `nixos-modules/options-vms.nix`; added per-submodule
  `mkRemovedOptionModule` shim in new
  `nixos-modules/options-vms-removed.nix`; replaced supervisor-
  based conditionals in `nixos-modules/host.nix` /
  `processes-json.nix`; rewrote assertion to defense-in-depth
  fallback; updated `examples/multi-env/flake.nix` /
  migrate goldens.
- **v1.1-P3**: added focused integration test
  `packages/nixling-core/tests/bundle_resolver_runner_intents.rs`.
- **v1.1-P4**: dropped `lib.mkIf cfg.daemonExperimental.enable`
  wrapper from `nixos-modules/host-broker.nix`; added
  `warnings = lib.optional` entry in `nixos-modules/assertions.nix`.
- **v1.1-P5**: added `nixlingStateDirAcl` activation script in
  `nixos-modules/host-activation.nix` (re-asserts 0750
  root:nixlingd + per-sidecar `--x` ACLs).
- **v1.1-P6**: commented out `./host-otel-relay-acl.nix` import
  in `nixos-modules/default.nix`.
- **v1.1-P7**: deleted `nixling-vfsd-watchdog@.{service,timer}`
  templates + per-VM enable units from `nixos-modules/store.nix`.
- **v1.1-P12**: authored
  `docs/how-to/migrate-nixling-v1-0-to-v1-1.md` (this entry's
  cross-referenced operator guide); updated ADR 0015 status to
  "Implemented in v1.1-P2"; updated CHANGELOG with this section.

## Unreleased — accumulator for post-v1.1 work

<!-- New entries for post-v1.1 work accumulate here. Cut a new
version section below this header when the next release (v1.1.1
or v1.2) is tagged. The substrate-replacement work that this
section previously tracked SHIPPED in v1.1 (see the v1.1 section
above). Remaining v1.1.1 scope: StatusOutputV3 wire schema bump,
per-role wiring of `runner_argv_regenerator`, RenderDnsmasqEnvConf
host-prep DAG op, USBIP guest ssh attach/detach one-shots, runtime
pidfs self-probe, clone3(CLONE_INTO_CGROUP) atomic placement,
nixling.slice/<vm>/<role> cgroup taxonomy migration, fchownat
AT_EMPTY_PATH fix. -->

## 1.0.0 — 2026-05-31

> **Git tag annotation (integrator).** The actual git tag (`v1.0.0`)
> is the integrator's call; this CHANGELOG entry only declares the
> version cut. When tagging, use an annotated tag whose message
> summarises the daemon-only end-state and points at
> [ADR 0015](docs/adr/0015-daemon-only-clean-break.md) and this
> 1.0.0 section. Suggested form:
>
> ```text
> git tag -a v1.0.0 -m "nixling 1.0.0 — daemon-only end-state
>
> Clean-break release per ADR 0015. nixlingd + nixling-priv-broker
> are the only persistent root surfaces; per-VM systemd templates,
> host singletons, the bash CLI, and the W14c bash fallback are
> removed wholesale at the v0.4.x → v1.0.0 boundary. See
> CHANGELOG.md § 1.0.0 — Breaking changes (summary) for the full
> enumeration with cross-references."
> ```

### P7 ph7-p7-v0-to-v1-guide — operator migration guide for v0.4.x → v1.0

Documentation-only commit. Adds
[`docs/how-to/migrate-nixling-v0-to-v1.md`](docs/how-to/migrate-nixling-v0-to-v1.md),
the consumer-facing operator migration guide for the v0.4.x → v1.0
daemon-only clean break per [ADR 0015](docs/adr/0015-daemon-only-clean-break.md).
The guide is structured as seven per-breaking-change sections with
the canonical *Before / After / Migration steps / Validation /
Rollback* layout for each, plus a final §7 W18 preflight + flip
recipe and a §8 whole-migration rollback.

### Breaking changes (summary)

This is the top-level enumeration of every breaking change that lands
in 1.0.0 across phases P0–P6. Each bullet links to the binding
architectural decision ([ADR 0015](docs/adr/0015-daemon-only-clean-break.md))
and to the per-phase deliverable docs (or the per-phase section of
this file) that carry the full rationale, migration steps, and
verification gates. Read [`docs/how-to/migrate-nixling-v0-to-v1.md`](docs/how-to/migrate-nixling-v0-to-v1.md)
before upgrading from v0.x.

- **`vms.json` `_manifest.manifestVersion` 2 → 3** (P2
  `ph2-p2-manifestversion-bump`). No legacy compatibility window:
  the daemon, broker, and every CLI verb reject a v2 bundle outright
  with `manifest-version-mismatch`. Operators must rebuild the
  manifest (and any vendored bundle) against the new schema before
  upgrading.
  See [ADR 0015 § Decision](docs/adr/0015-daemon-only-clean-break.md),
  [`docs/reference/manifest-schema.md`](docs/reference/manifest-schema.md),
  [`docs/reference/manifest-schema.json`](docs/reference/manifest-schema.json),
  and the per-phase entry [§ Breaking changes (manifest contract)](#breaking-changes-manifest-contract)
  below.

- **Bash CLI removed entirely** (P4 `ph4-cli-up` end-state +
  P6 `ph6-remove-systemd-emission` deletion). The Rust `nixling`
  binary is the sole CLI; `nixos-modules/cli.nix`, the
  `share/nixling/cli.sh` entrypoint, and every bash subcommand are
  deleted. There is no bash fallback bridge.
  See [ADR 0015 § Decision](docs/adr/0015-daemon-only-clean-break.md),
  [`docs/reference/cli-contract.md`](docs/reference/cli-contract.md),
  and the per-phase entry [§ P4 ph4-cli-up](#p4-ph4-cli-up--updownrestartlist-are-daemon-native-end-to-end)
  below.

- **Per-VM systemd templates retired** (P6
  `ph6-remove-systemd-emission`). `nixling@<vm>.service`,
  `nixling-<vm>-gpu.service`, `nixling-<vm>-swtpm.service`,
  `nixling-<vm>-video.service`, `nixling-<vm>-snd.service`, and
  `nixling-known-hosts-refresh@<vm>.service` are deleted. Every
  per-VM lifecycle step now runs inside `nixlingd`'s DAG executor;
  spawned runners (cloud-hypervisor, virtiofsd, swtpm,
  vhost-user-sound, USBIP attach) are launched by the broker's
  `SpawnRunner` op and handed back to `nixlingd` as pidfds via
  `OpenPidfd` / `SCM_RIGHTS`.
  See [ADR 0015 § Decision](docs/adr/0015-daemon-only-clean-break.md),
  [`docs/explanation/daemon-lifecycle.md`](docs/explanation/daemon-lifecycle.md),
  [`docs/reference/privileges.md`](docs/reference/privileges.md),
  and the per-phase entry [§ P6 ph6-remove-systemd-emission](#p6-ph6-remove-systemd-emission--clean-break-deletion-of-legacy-per-vm-systemd-templates-and-host-singletons)
  below.

- **Host singletons retired** (P3 retirement landings +
  P6 `ph6-remove-systemd-emission`). `nixling-audit-check.{service,timer}`,
  `nixling-ch-exporter.service`, `nixling-net-route-preflight.service`,
  `nixling-otel-host-bridge.service`, and the per-env
  `nixling-sys-<env>-usbipd-{backend,proxy}.{service,socket}` units
  are deleted. Their work moved into `nixlingd` (Prometheus
  exposition, net-route preflight, USBIP state machine) or into
  broker ops (`ExportBrokerAudit`, `UsbipBindFirewallRule`,
  `SpawnRunner{role: Usbip}`); the framework declares exactly three
  root-visible units (`nixlingd.service`,
  `nixling-priv-broker.socket`, `nixling-priv-broker.service`).
  See [ADR 0015 § Decision](docs/adr/0015-daemon-only-clean-break.md),
  [`docs/reference/privileges.md`](docs/reference/privileges.md),
  and the per-phase entries [§ P3 ph3-usbipd-perenv](#p3-ph3-usbipd-perenv--daemon-owns-per-env-usbipd-backend--proxy-spawn)
  and [§ P3 ph3-p3-net-route-degraded-mode](#p3-ph3-p3-net-route-degraded-mode--daemon-owns-net-route-preflight)
  below.

- **Polkit per-VM allowlists removed** (P6 `ph6-p6-polkit-retire`).
  `nixos-modules/host-polkit.nix` no longer exposes per-VM or
  per-env unit controls (`perVmUnits` and `perEnvUnits` both
  return `[ ]`; `systemUnits = [ ]`). `nixling-launchers` group
  membership + `SO_PEERCRED` at `public.sock` accept time is the
  sole lifecycle authorisation surface.
  See [ADR 0015 § Decision](docs/adr/0015-daemon-only-clean-break.md),
  [`docs/reference/privileges.md`](docs/reference/privileges.md),
  and the per-phase entry [§ P6 ph6-p6-polkit-retire](#p6-ph6-p6-polkit-retire--retire-host-polkit-per-vm-allowlists)
  below.

- **W14c bash fallback removed** (P4 `ph4-cli-up`). The Rust CLI's
  `dispatch_mutating_verb` no longer degrades to the bash CLI on
  `not-yet-implemented` / `daemon-down`. `NIXLING_LEGACY_BASH_OPT_IN=1`
  is no longer honoured (no effect); affected verbs now surface a
  typed envelope (exit 78 for `not-yet-implemented`, exit 1 for
  `daemon-down`). `NIXLING_NATIVE_ONLY=1` is preserved as a no-op
  (its behaviour is the default).
  See [ADR 0015 § Decision](docs/adr/0015-daemon-only-clean-break.md),
  [`docs/reference/cli-contract.md`](docs/reference/cli-contract.md),
  [`docs/explanation/default-switch-and-deprecation.md`](docs/explanation/default-switch-and-deprecation.md),
  and the per-phase entry [§ Breaking changes](#breaking-changes)
  (under P5) below.

- **W18 default flip** (P5). `nixling.daemonExperimental.enable`
  now defaults to `true` on hosts where the fixed flip-gate readiness
  subset (`w4Fu`/`w5Fu`/`w6Fu`/`w7Fu`/`w8Fu`/`w9Fu`/`p0`/`p0Fu`/`p1`/`p2`/`p3`/`p4`)
  reports `implemented + validated + evidence`. Operator overrides
  still win in both directions. New option
  `nixling.daemonExperimental.defaultFlipEvidenceDir` (default
  `/var/lib/nixling/validated`) backs the gate.
  See [ADR 0015 § Decision](docs/adr/0015-daemon-only-clean-break.md),
  [`docs/explanation/default-switch-and-deprecation.md`](docs/explanation/default-switch-and-deprecation.md#w18-auto-flip-semantics),
  and the per-phase entry [§ Breaking changes](#breaking-changes)
  (under P5) below.

- **Kernel device taxonomy expanded** (P1 + gap-fix-kernel-paths).
  `nixling_host::DeviceClass` gained `Udmabuf` for the GPU sidecar's
  `udmabuf` ioctl access (UDMABUF_CREATE / UDMABUF_CREATE_LIST), plus
  the `DRM_IOCTL_VIRTGPU_*` set on `DeviceClass::Dri` for cross-domain
  rendering. `modules_disabled` is now fail-closed in the broker's
  `ModprobeIfAllowed` path (the operator-set sysctl now refuses any
  modprobe attempt instead of degrading silently), and the broker's
  `clone3` child closure is async-signal-safe with precomputed
  pointers (no allocations between `clone3` and `execve`).
  See [`packages/nixling-host/src/devices.rs`](packages/nixling-host/src/devices.rs),
  [`packages/nixling-priv-broker/src/sys.rs`](packages/nixling-priv-broker/src/sys.rs),
  and ADR 0015 for the v1.0 kernel-surface invariant.

- **Cgroup v2 delegation invariant restated** (ADR 0011 → ADR 0015).
  `/sys/fs/cgroup/nixling.slice` remains the fixed root; the broker
  delegates the subtree to the non-root `nixlingd` uid via `fchown`
  before dropping its own privileges. No threaded cgroups, no partition
  roots, and no internal processes in the slice's interior nodes.
  Per-VM `nixling.slice/<vm>/<role>` leaves are the only fork/exec
  destinations, and `cgroup.kill` against the leaf is the only
  supported teardown path.

### P6 ph6-remove-systemd-emission — clean-break deletion of legacy per-VM systemd templates and host singletons

THE clean-break commit per [ADR 0015](docs/adr/0015-daemon-only-clean-break.md).
After this commit the framework emits exactly two persistent nixling
systemd units — `nixlingd.service` and `nixling-priv-broker.{service,socket}`
— modulo the deferrals enumerated at the bottom of this section.

#### Breaking changes

- **Host singletons retired:**
  - `nixling-audit-check.{service,timer}` → broker `ExportBrokerAudit` op
    + `nixling host doctor` (replacement landed in `ph3-p3-audit-check-retire`).
    Deleted `nixos-modules/host-audit.nix`.
  - `nixling-ch-exporter.service` → `nixlingd` Prometheus
    exposition at `127.0.0.1:9101` (`ph3-p3-ch-exporter-retire`).
    Deleted `nixos-modules/host-ch-exporter.nix` and stripped its
    journal source + scrape job from the Alloy host config in
    `nixos-modules/components/observability/host.nix`.
  - `nixling-net-route-preflight.service` → `nixlingd` startup
    self-check + `nixling host reconcile --network --apply`
    (`ph3-p3-net-route-degraded-mode`). Removed from
    `nixos-modules/network.nix`.
- **Per-env usbipd singletons retired:**
  - `nixling-sys-<env>-usbipd-{backend,proxy}.{service,socket}` and
    their per-env iptables carve-outs (3 envs × 3 units in the
    canonical site config) → broker `SpawnRunner{role: Usbip,
    vm_id: sys-<env>-usbipd}` driven by the per-busid state
    machine in [`docs/reference/privileges.md`](docs/reference/privileges.md),
    with firewall placements performed at runtime via the
    `UsbipBindFirewallRule` broker op (`ph3-p3-usbip-state-machine`).
    Removed from `nixos-modules/network.nix`.
- **Per-VM systemd templates retired:**
  - `nixling@<vm>.service` (VM lifecycle wrapper) → daemon
    supervisor DAG + broker `SpawnRunner{role: CloudHypervisor}`.
    Deleted `nixos-modules/host-wrapper.nix`. The
    `microvm.autostart = [ ]` + `systemd.targets.microvms.wants
    = lib.mkForce [ ]` suppressions previously held in
    host-wrapper.nix moved verbatim into `host.nix` so the
    upstream microvm.nix autostart cascade stays disabled while
    the `microvm.vms` translation is preserved (see
    *Deferred to follow-up* below).
  - `nixling-<vm>-gpu.service`, `nixling-<vm>-swtpm.service`
    (graphics + TPM sidecars) → broker `SpawnRunner{role: Gpu}` /
    `SpawnRunner{role: Swtpm}`. Deleted `nixos-modules/host-sidecars.nix`.
  - `nixling-<vm>-video.service` (video sidecar) → broker
    `SpawnRunner{role: Video}`. Deleted `nixos-modules/components/video/host.nix`.
  - `nixling-<vm>-snd.service` (audio sidecar) → broker
    `SpawnRunner{role: Audio}`. The per-VM service block in
    `nixos-modules/components/audio/host.nix` was surgically
    removed; the PipeWire client rules, vhost-device-sound
    package, tmpfiles, and assertions stay (consumed by the
    broker runner at fork time).
  - `nixling-known-hosts-refresh@<vm>.service` → daemon-side TOFU
    refresh. Deleted `nixos-modules/host-known-hosts.nix`.
- **Polkit allowlist trimmed.** `host-polkit.nix` no longer exposes
  the retired host singletons (`systemUnits = [ ]`) or any per-env
  controls (`perEnvUnits` returns `[ ]`). Per-VM controls in
  `perVmUnits` are preserved as a safety net for the deferred
  units below; they become dead entries once those follow-ups land.
- **Eval-gate suite updated to match the new surface.**
  `tests/autostart-wiring-eval.sh` now asserts the
  `nixlingd → multi-user.target` boot path (replacing the
  pre-P6 `nixling@<vm> → multi-user.target.wants` invariant)
  and that `nixling@` is absent. `tests/restart-policy-eval.sh`
  marks the retired host/per-VM units as `check_optional`
  (the gate fails if they reappear without
  `restartIfChanged=false`). `tests/usbip-gating-eval.sh`
  inverts: instead of asserting per-env backend/proxy presence
  under enablement, it asserts *absence* under enablement plus
  preserved kernel-module loading.
  `tests/video-sidecar-hardening-eval.sh` is degraded to a
  no-op stub deferring to a forthcoming
  `broker-video-hardening-eval`. `tests/smoke-eval-graphics.nix`
  drops the `nixling-<vm>-gpu` DeviceAllow assertion;
  `tests/smoke-eval-tpm.nix` drops the per-VM swtpm sidecar
  assertions but keeps the host-side `nixlingTpmStatePerms` /
  `nixlingMigrateOwnership` activation-script presence checks.

#### Deferred to follow-up commits

The cuts below were inspected during P6 and intentionally left in
place because removing them in this commit would require parallel
rearchitecture work that exceeds the clean-break scope. Each is
flagged in the affected module with a P6 comment.

- **`microvm@<vm>.service` (upstream microvm.nix template).** The
  `host.nix` `microvm.vms = lib.mapAttrs ...` translation is
  preserved because `processes-json.nix` reads `microvm.vsock.cid`,
  `microvm.graphics.socket`, and `microvm.shares` out of the
  per-VM upstream config to assemble `/etc/nixling/bundle.json`.
  Removing the translation requires re-homing those reads to
  `nixling.vms.<vm>.*` directly. The autostart cascade *is*
  suppressed (`systemd.targets.microvms.wants = lib.mkForce [ ]`),
  so the unit only fires when the broker explicitly stages a VM.
- **`microvm-virtiofsd@<vm>.service` drop-ins and
  `nixling-<vm>-store-sync.service`** in `nixos-modules/store.nix`.
  These coexist in the same file with the per-VM
  `/var/lib/nixling/vms/<vm>/store` provisioning the broker
  Virtiofsd/Store runners depend on; the surgical split is
  non-trivial.
- **`nixling-otel-relay@<vm>.service` and
  `nixling-otel-host-bridge.service`** in
  `nixos-modules/components/observability/host.nix` and
  `nixos-modules/host-otel-relay-acl.nix`. The host-side ACL
  refresh script in `host-otel-relay-acl.nix` is consumed by the
  broker runner; the otel relay surface itself needs a broker
  `SpawnRunner{role: OtelHostBridge}` replacement before the
  systemd entry points can go.
- **`nixling-vfsd-watchdog@.{service,timer}`** in `store.nix`. The
  watchdog is defense-in-depth against wedged vhost-user-fs
  reuses; it becomes redundant once the broker virtiofsd runner
  owns pidfd supervision, but removal is deferred along with the
  virtiofsd drop-in surgery.

#### Files deleted

- `nixos-modules/host-audit.nix`
- `nixos-modules/host-ch-exporter.nix`
- `nixos-modules/host-sidecars.nix`
- `nixos-modules/host-wrapper.nix`
- `nixos-modules/host-known-hosts.nix`
- `nixos-modules/components/video/host.nix`

#### Verification

All P0 gates pass: `broker-caps-eval`, `broker-socket-activation-eval`,
`broker-bundle-path-eval`, `readiness-waves-eval`,
`adr-0015-presence-eval`. Additional gates pass:
`autostart-wiring-eval`, `restart-policy-eval`, `usbip-gating-eval`,
`video-sidecar-hardening-eval` (stub), `net-vm-network-eval`,
`assertions-eval`, `privileges-doc-completeness-eval` (4
transitional warnings for deferred otel/video surfaces — expected).
All smoke evals (`smoke-eval`, `smoke-eval-graphics`,
`smoke-eval-tpm`, `smoke-eval-aarch64`,
`smoke-eval-home-manager`) return 54 attrs.
`observability-eval` has 7 pre-existing failures (Alloy realise +
tempo-critical datasource mismatch) unrelated to P6.



### P6 ph6-p6-polkit-retire — retire host-polkit per-VM allowlists

- **`nixos-modules/host-polkit.nix` simplified to daemon-only
  singletons.** The W2-followup C1 exact-unit allowlist that
  generated entries from `config.nixling.{vms,envs}` for every
  per-VM sidecar (`nixling@<vm>.service`,
  `nixling-<vm>-{gpu,snd,swtpm,store-sync}.service`) and per-env
  usbipd triplet (`nixling-sys-<env>-usbipd-{proxy,backend}.{service,socket}`)
  is removed. The companion JS rule scoped to the per-VM
  `nixling-<vm>-gpu` system user (granting it start/stop/restart
  of its paired `nixling-<vm>-snd.service`) is also removed —
  every unit it named is deleted in P6
  (`ph6-remove-systemd-emission`), and the bash CLI code path that
  used to drive the units via `systemctl` is deleted in P6
  (`ph6-p6-cli-nix-migrations`). The launcher-group polkit grant
  now allowlists exactly three daemon-only singleton units:
  `nixlingd.service`, `nixling-priv-broker.service`, and
  `nixling-priv-broker.socket`. The verb allowlist
  (`start`/`stop`/`restart`) is unchanged; every other verb still
  requires the polkit-password default. The `nixling-launcher`
  group remains the privilege boundary for operator-driven
  daemon-singleton restarts; per-VM lifecycle now flows through
  the daemon's public socket (`SO_PEERCRED` group check, no
  polkit in the path) per ADR 0015.
- **New Layer-1 gate.** `tests/polkit-allowlist-eval.sh` asserts
  the allowlist names exactly the three daemon-only singletons,
  contains no references to any retired per-VM or per-env unit
  shape, preserves the `org.freedesktop.systemd1.manage-units`
  action-id + `nixling-launcher` group + `start/stop/restart`
  verb guards, and declares exactly one `polkit.addRule` callback
  (the per-VM gpu→snd fallback rule is gone). Wired into
  `tests/static.sh` alongside the other Layer-1 eval gates.


### P6 ph6-p6-adr-0015 — ADR 0015 daemon-only clean break

- **New ADR.** `docs/adr/0015-daemon-only-clean-break.md` is the
  binding architectural decision for the v1.0 end-state:
  `nixlingd` + `nixling-priv-broker` are the only persistent root
  surfaces the framework declares. Per-VM systemd templates
  (`nixling@<vm>.service`, `microvm@<vm>.service`,
  `microvm-virtiofsd@<vm>.service`,
  `nixling-<vm>-{gpu,snd,video,swtpm,store-sync}.service`, the
  upstream `microvm-{tap-interfaces,macvtap-interfaces,pci-devices,set-booted}@.service`
  templates, `nixling-otel-relay@<vm>.service`,
  `nixling-known-hosts-refresh@.service`,
  `nixling-vfsd-watchdog@.{service,timer}`, and
  `nixling-sys-<env>-usbipd-{proxy,backend}.{service,socket}`), host
  singletons (`nixling-{ch-exporter,otel-host-bridge,net-route-preflight,audit-check}.service`,
  `nixling-audit-check.timer`, `microvms.target`), the `cli.nix`
  bash package, the W14c bash fallback bridge, the
  `nixling.vms.<vm>.supervisor` option, and the `nixling-launcher`
  polkit allowlist are all removed in P6 with no deprecation
  window. The ADR records context (W14c + per-VM templates + host
  singletons as the v0.4.0 baseline), the clean-break decision
  versus a v0.5 deprecation cycle, and the positive (single audit
  surface, smaller TCB), negative (no v2 → v3 manifest compat
  window, hard dependency on daemon health, single point of failure
  mitigated by socket-activation + `Restart=always`), and neutral
  (`cli.nix` retirement, `cli-contract.md` as the operator surface)
  consequences. Supersedes the migration-mode plumbing from
  [ADR 0007](docs/adr/0007-bash-coexistence-and-migration.md)
  decisions 1–6; ADR 0007 stays `Accepted` as historical record.
- **ADR index updated.** `docs/adr/README.md` lists ADR 0015.
- **AGENTS.md cross-reference.** The References section now points
  at ADR 0015 as the binding v1.0 daemon-only end-state record so
  agents and contributors discover the supersession of ADR 0007.
- **New presence gate.** `tests/adr-0015-presence-eval.sh`
  asserts the ADR exists with the canonical header, the required
  Context/Decision/Consequences sections, and that it is
  cross-referenced from both `AGENTS.md` and `docs/adr/README.md`.


### P6 ph6-p6-default-switch-doc — rewrite default-switch-and-deprecation docs for the clean break

- **`docs/reference/default-switch-and-deprecation.md` rewritten as a
  post-clean-break landing page.** The "default mode vs native-only
  mode" axis, the `NIXLING_NATIVE_ONLY` / `NIXLING_LEGACY_BASH_OPT_IN`
  escape hatches, and the W14c three-mode bridge text are removed.
  The compatibility matrix now shows the single daemon-native path
  per verb; the "Legacy bash path kept?" column collapses to a
  uniform **no** with a footnote citing P4's W14c-bridge retirement
  and P6's bash-CLI / per-VM systemd-template deletion
  (`ph6-p6-cli-nix-migrations`, `ph6-remove-systemd-emission`).
  The W18 auto-flip gate section is preserved because it still
  governs how `nixling.daemonExperimental.enable` resolves on
  fresh consumer hosts.
- **`docs/explanation/default-switch-and-deprecation.md` rewritten
  as a historical record.** The W10 `+30/+90/+180 day` bash
  deprecation calendar is removed (no calendar exists post-clean-
  break) and replaced with a "what was deprecated, what replaced
  it" mapping table plus a rationale section explaining why ADR
  0015's clean break supersedes the original W10 / W14c
  coexistence plan. The pre-clean-break per-verb compatibility
  matrix is retained verbatim with a single rubric clarifying that
  every row's "Bash" column reads as deleted in P6.
- **Cross-references added** from both files to
  [`docs/adr/0015-daemon-only-clean-break.md`](docs/adr/0015-daemon-only-clean-break.md),
  [`docs/reference/cli-contract.md`](docs/reference/cli-contract.md),
  [`docs/reference/wave-evidence-schema.md`](docs/reference/wave-evidence-schema.md),
  and [`docs/reference/host-validate.md`](docs/reference/host-validate.md).
- **Stable URLs preserved.** Both files keep their original paths
  so that historical CHANGELOG entries, AGENTS.md "Control plane"
  references, and inline code comments
  (`nixos-modules/options-daemon.nix`,
  `packages/nixling/src/host_validate.rs`) continue to resolve.


### P6 ph6-p6-privileges-doc-final — final-pass privileges.md (daemon-only end-state)

Documentation-only commit. Rewrites
[`docs/reference/privileges.md`](docs/reference/privileges.md) to
remove every row that referred to a legacy systemd template or host
singleton scheduled for deletion in P6, and replaces them with the
canonical daemon-only surface: `nixlingd.service` +
`nixling-priv-broker.{service,socket}` + per-VM / per-role runners
spawned via broker `SpawnRunner` (no systemd unit per runner).

- **Runner-roles table (line 156):** `OtelHostBridge` row's
  "Replaces" cell rewritten as obituary citing
  `ph6-remove-systemd-emission` (branch
  `phase-p6-privileges-final`).
- **P2 section prose:** retired-templates list rewritten past-tense
  with explicit P6 deletion attribution.
- **HostPrep DAG (P2) table:** `Retires` column renamed to
  `Retired (deleted in P6)` and every cell carries a one-line
  obituary citing the replacement broker op (`CreateTapFd` /
  `CreatePersistentTap` / `SetBridgePortFlags` / `OpenDevice` /
  `StoreSync` / `SshKeygenProbe` / pure-daemon
  `supervisor::state::record_booted` / `supervisor::pidfd`).
- **P3 new broker-dispatch contracts table:** `Replaces` column
  renamed to `Retired (deleted in P6)` for both `OtelHostBridge`
  and `Usbip` runner-role rows with explicit deletion attribution.
- **P3 host singleton retirements table:** rewritten as a post-P6
  obituary index; adds the previously-omitted
  `nixling-otel-host-bridge.service` and per-env
  `nixling-sys-<env>-usbipd-{proxy,backend}.{service,socket}` rows
  (previously documented only inline as "re-homed").
- **New canonical section "P6 final-pass: comprehensive legacy
  systemd surface obituary"** indexes every retired unit in three
  sub-tables:
  1. **Per-VM template obituaries (deleted P6)** — 14 rows:
     `nixling@<vm>.service`, `microvm@<vm>.service`,
     `microvm-tap-interfaces@<vm>.service`,
     `microvm-set-booted@<vm>.service`,
     `microvm-pci-devices@<vm>.service`,
     `microvm-virtiofsd@<vm>.service`,
     `nixling-<vm>-gpu.service`, `nixling-<vm>-video.service`,
     `nixling-<vm>-snd.service`, `nixling-<vm>-swtpm.service`,
     `nixling-<vm>-store-sync.service`,
     `nixling-known-hosts-refresh@<vm>.service`,
     `nixling-vfsd-watchdog@<vm>.{timer,service}`,
     `nixling-otel-relay@<vm>.service`.
  2. **Host singleton obituaries (deleted P6)** — 5 rows:
     `nixling-net-route-preflight.service`,
     `nixling-audit-check.{service,timer}`,
     `nixling-ch-exporter.service`,
     `nixling-otel-host-bridge.service`,
     `nixling-sys-<env>-usbipd-proxy.{service,socket}` +
     `nixling-sys-<env>-usbipd-backend.{service,socket}` (per
     USBIP-enabled env).
  3. **Activation-time hooks retired in P6** — 2 rows: the
     `nixling-store-sync` activation hook from `store.nix`
     (replaced by `nixling host install --apply` →
     broker `StoreSync`), and `cli.nix`'s per-VM `desktopItems`
     generation `nixling-launch-<vm>` (replaced by the daemon-native
     launcher module from `ph4-p4-desktop-wrapper`).
- **New Layer-1 gate:**
  [`tests/privileges-doc-completeness-eval.sh`](tests/privileges-doc-completeness-eval.sh)
  enumerates 20 legacy unit-name patterns and asserts each appears
  somewhere in `docs/reference/privileges.md` (either a live broker-op
  / runner-role / DAG row, or a P6 obituary row) — never both as a
  contradictory pair. The gate accepts the transitional "still emitted
  by `nixos-modules/` AND already in the P6 obituary" state with a
  WARN (the doc-only commit lands before the sibling
  `ph6-remove-systemd-emission` code-deletion commit) and goes fully
  green once the sibling agent ships.

Branch: `phase-p6-privileges-final`. Base: `phase-daemon-only @
29e37de`.

### P5 ph5-p5-tempo-budget — Tempo retention + sampling policy

- **Canonical two-tier Tempo policy pinned.** New options on
  `nixling.observability` codify the trace-budget contract:
  - `retention.traces` = `"7d"` (default Tempo tenant
    `sampling.defaultTenant = "nixling-default"`).
  - `retention.tracesCritical` = `"30d"` (critical Tempo tenant
    `sampling.criticalTenant = "nixling-critical"`).
  - `sampling.criticalAttribute` = `"kind"` /
    `sampling.criticalValue` = `"critical"` — span attribute
    pair that pins a trace into the critical tenant.
  - `sampling.criticalRatio` = `1.0` — every critical span kept.
  - `sampling.defaultRatio` = `0.1` — 10 % of non-critical
    traces kept via tail-sampling.
- **Tempo is now multi-tenant.** `services.tempo.settings`
  (`nixos-modules/components/observability/stack.nix`) enables
  `multitenancy_enabled = true`, sets the compactor's global
  `block_retention` ceiling to `retention.tracesCritical`,
  overrides the default tenant down to `retention.traces` via
  `overrides.defaults.compaction.block_retention`, and points
  `per_tenant_override_config` at a generated YAML file that
  pins the critical tenant to `retention.tracesCritical`.
- **Alloy trace pipeline rewritten** to enforce the policy:
  `otelcol.processor.tail_sampling "tempo_budget"` with two
  named policies (`critical_keep_all` → `always_sample` when
  `kind="critical"`; `default_probabilistic` →
  `sampling_percentage` = `defaultRatio * 100`), feeding
  `otelcol.connector.routing "tempo_tenant"` which splits the
  sampled traces between two OTLP exporters
  (`traces_critical` / `traces_default`) that tag the outbound
  stream with the correct `X-Scope-OrgID` header.
- **Grafana** now provisions two Tempo datasources: `Tempo`
  (uid `tempo`, default tenant — dashboards keep linking here)
  and `Tempo (Critical)` (uid `tempo-critical`) for forensic
  queries beyond the 7-day default window.
- **New canonical reference:**
  [`docs/reference/tempo-retention-sampling.md`](docs/reference/tempo-retention-sampling.md)
  records the policy, the per-VM expected trace volume, the
  `/var/lib/tempo` disk-budget cost model, and the
  change-control rules (any policy change touches stack.nix,
  options-observability.nix, the doc, and CHANGELOG.md in one
  commit).
- **New Layer-1 gate:**
  [`tests/tempo-budget-eval.sh`](tests/tempo-budget-eval.sh)
  asserts the Nix-side constants, the Tempo settings shape,
  the Alloy pipeline shape, and the doc all agree. Wired into
  `tests/static.sh` alongside the other observability eval
  gates.

### P5 ph5-p5-host-validate-verb — `nixling host validate --apply`

- **New `nixling host validate` composite preflight verb.** Ships
  the operator-facing one-command preflight that must run after a
  fresh `nixos-rebuild switch` and before flipping
  `nixling.daemonExperimental.enable = true`. The verb iterates the
  W18 readiness waves (`w4Fu`..`w9Fu`, `p0`..`p7`) in a deterministic
  catalog order, inventories the per-wave Layer-2 validator scripts
  shipped under `tests/`, and (with `--apply`) writes the canonical
  evidence record `/var/lib/nixling/validated/<wave>.json` with the
  W18 schema fields `{wave, timestamp, operatorSignature}` for every
  `ready` wave. The daemon's W18 auto-flip gate
  (`nixos-modules/options-daemon.nix:validationEvidencePresent`)
  consumes those records; fresh consumer hosts no longer hit the
  validation cliff between `implementedDefault = true` and the
  per-wave `validated = true` flip.
- **CLI surface** (`packages/nixling/src/host_validate.rs`,
  `packages/nixling/src/lib.rs::HostCommand::Validate`):
  `nixling host validate (--dry-run | --apply) [--wave <name>]
  [--operator-signature <sig>] [--evidence-dir <path>]
  [--scripts-dir <path>] [--json | --human]`. Mandatory mutation
  flag (refuses with the canonical `--apply-or-dry-run-required`
  envelope, exit 78). Unknown `--wave` values surface the typed
  `unknown-wave` envelope (exit 78). `--apply` with at least one
  `missing` wave refuses (exit 78); evidence-write failures surface
  exit 1.
- **Layer-1 gate** `tests/host-validate-verb-eval.sh` (new):
  asserts CLI flag refusals, the W18 schema contract on every
  written evidence file, the wave-vocabulary parity between
  `WAVE_CATALOG` in the Rust verb and `readinessWaveSpecs` in
  `nixos-modules/options-daemon.nix` (drift between the two surfaces
  silently breaks the gate), and the `--wave` filter + bogus-wave
  refusal envelopes.
- **Docs** `docs/reference/host-validate.md` (new): full verb
  contract, per-wave validator map, exit-code table, operator
  first-flip workflow, evidence schema. Cross-linked from the
  AGENTS.md Critical-subsystems "Control plane (W2+)" row and the
  default-switch deprecation reference.
- **Routing fix** in `should_fallback_to_legacy`: the legacy bash CLI
  fallback list for `host` now permits `validate` (and `reconcile`,
  which was already a native verb but missing from the allow-list)
  alongside `check`/`prepare`/`destroy`/`doctor`/`install`, so
  `nixling host validate` reaches the native dispatcher instead of
  the "could not locate the legacy bash CLI" failure.


### Breaking changes

- **default flip**: `nixling.daemonExperimental.enable` now defaults
  to `true` on hosts where all readiness waves report
  `implemented + validated + evidence`. (P5 w18-flip.) The W18
  auto-flip gate iterates over a fixed subset of
  `defaultSwitchReadiness` waves — `w4Fu`, `w5Fu`, `w6Fu`, `w7Fu`,
  `w8Fu`, `w9Fu`, `p0`, `p0Fu`, `p1`, `p2`, `p3`, `p4` — and the
  default flips to `true` iff every wave in that set has BOTH
  `implemented = true` AND `validated = true` AND a matching
  `<nixling.daemonExperimental.defaultFlipEvidenceDir>/<wave>.json`
  evidence record on disk. The pre-P5 predicate considered every
  `defaultSwitchReadiness` wave (including the future-tense
  `p5`/`p6`/`p7` records that landed in P0); that predicate could
  never actually go green at P5 boundary and is replaced by the
  explicit flip-gate subset. Operator overrides still win in both
  directions — an explicit `= true` or `= false` (with or without
  `mkDefault`/`mkForce`) pins the value regardless of the computed
  gate. New option
  `nixling.daemonExperimental.defaultFlipEvidenceDir` (default
  `/var/lib/nixling/validated`) exists primarily for the regression
  test `tests/w18-default-flip-eval.sh`; operator hosts SHOULD
  leave it at the default. See
  [`docs/explanation/default-switch-and-deprecation.md`](docs/explanation/default-switch-and-deprecation.md#w18-auto-flip-semantics).

- **`NIXLING_LEGACY_BASH_OPT_IN` removed (P4 cli-up).** The W14c
  "daemon-first, bash-on-NotYetImplemented" fallback bridge inside the
  Rust CLI's `dispatch_mutating_verb` has been retired entirely. The
  `NIXLING_LEGACY_BASH_OPT_IN=1` operator escape hatch is no longer
  honoured: setting it has no effect, and mutating verbs that
  previously degraded to the bash CLI on `not-yet-implemented` or
  `daemon-down` now surface a typed envelope (exit 78 for
  `not-yet-implemented`, exit 1 for `daemon-down`) instead. The
  `NIXLING_NATIVE_ONLY=1` env var is preserved as a no-op (its
  behaviour is now the default). Operators previously relying on
  `NIXLING_LEGACY_BASH_OPT_IN=1` should either upgrade `nixlingd` to
  a build that ships the required native handler or run the legacy
  `share/nixling/cli.sh` directly.

### P4 ph4-cli-up — `up/down/restart/list` are daemon-native end-to-end

- **`nixling up/down/restart` are now first-class native verbs**
  routed directly to `cmd_vm_start/stop/restart`. They are aliases
  for `nixling vm start/stop/restart` and share the same daemon
  dispatch path (broker `SpawnRunner` for start, broker `SignalRunner`
  + reaper for stop, stop+start composition for restart). The previous
  routing through `should_fallback_to_legacy` (which sent the top-level
  `up/down/restart` verbs to the bash CLI) has been removed.
- **`nixling list` and `nixling vm list`** remain native: the manifest
  view at `nixling list` and the placeholder runtime envelope at
  `nixling vm list` no longer touch the bash CLI under any failure
  mode.
- **CLI `dispatch_mutating_verb` simplification.** The function still
  accepts `legacy_args`/`legacy_fallback_warning` parameters for
  binary compatibility with the eight call sites, but the bash
  fallback branches in `NotYetImplemented` and `Unreachable` have
  been replaced with typed envelopes. The `NIXLING_LEGACY_BASH_OPT_IN`
  early-bash bypass at the head of the function has been removed.
- **Layer-1 gate `tests/cli-vm-verbs-eval.sh`** asserts the new
  contract: with the public socket missing, each of `up`, `down`,
  `restart`, `vm start/stop/restart` emits the `daemon-down`
  envelope (no bash exec, even with a poison-pill
  `NIXLING_LEGACY_CLI_PATH` and `NIXLING_LEGACY_BASH_OPT_IN=1` set);
  `vm list` returns the native JSON envelope.
- **Docs** — `docs/reference/cli-contract.md` adds a P4 cli-up
  banner; the per-verb `NIXLING_LEGACY_BASH_OPT_IN=1` rows under
  each W14 LiveNative verb were removed; "uses the W14c daemon
  bridge" was reworded to "is daemon-native (P4 cli-up removed
  the W14c bash fallback)".


### P4 ph4-p4-desktop-wrapper — graphics-VM .desktop wrappers go daemon-native

- **The auto-generated `nixling-launch-<vm>.desktop` wrapper for every
  graphics VM now drives the daemon path.** The Exec line invokes
  `NIXLING_NATIVE_ONLY=1 nixling vm start <vm> --apply --json` instead
  of the legacy `nixling up <vm> -d` bash entrypoint. The wrapper
  routes through `nixlingd → nixling-priv-broker → SpawnRunner` — the
  same DAG every other P4 lifecycle verb takes — so a launcher click
  from the Plasma menu can no longer silently exercise a deprecated
  code path. `NIXLING_NATIVE_ONLY=1` makes the W14c bash fallback an
  explicit, typed refusal rather than a silent slip.
- **GPU wayland-socket gate.** After the daemon reports the VM up, the
  wrapper now polls
  `/run/nixling-gpu/<vm>/wayland-0` (the GPU sidecar's bind-mounted
  host compositor) for up to 30 s before opening the in-VM Konsole.
  The daemon's `ssh-ready` DAG node only guarantees sshd, and the GPU
  socket can race slightly behind on cold starts; without this gate
  the in-VM `wayland-proxy-virtwl` client could be launched against a
  socket that hadn't been bind-mounted yet.
- **Typed envelope surfacing.** Daemon failures come back as the JSON
  envelope defined in `docs/reference/error-codes.md` /
  `docs/reference/daemon-api.md`. The wrapper parses
  `errorKind` / `operationId` / `remediation` with `jq` and surfaces
  them in a `notify-send` desktop notification together with paths to
  `nixling status <vm>`, `journalctl -u nixlingd.service`, and the
  new per-VM launcher log at
  `${XDG_STATE_HOME:-$HOME/.local/state}/nixling/launchers/<vm>.log`.
  Every step of the wrapper also appends a timestamped line to that
  log so a failed click leaves a forensic trail beyond the transient
  bubble.
- **New typed contract:** `nixling._desktopWrappers.<vm>` is an
  internal NixOS option (`nixos-modules/options.nix`) carrying the
  schema-versioned shape of the wrapper's Exec line + supporting
  environment. The new regression gate
  `tests/desktop-wrapper-contract-eval.sh` asserts that every
  graphics VM has a contract, that `execArgv = [ "vm" "start" "<vm>"
  "--apply" ]`, that `NIXLING_NATIVE_ONLY=1` is set, that the
  output mode is `json`, that the GPU socket path is pinned at
  `/run/nixling-gpu/<vm>/wayland-0`, and that the rendered script
  body contains no legacy `nixling up`/`down`/`restart` invocations.
- **New reference doc:**
  [`docs/reference/desktop-wrapper.md`](docs/reference/desktop-wrapper.md)
  is the canonical contract; any change to the field table requires
  bumping `desktopWrapperContractVersion` (currently `1`), updating
  the doc, updating the gate, and a fresh CHANGELOG entry.

### P3 ph3-usbipd-perenv — daemon owns per-env usbipd backend + proxy spawn

- **Daemon-side per-env usbipd autostart** replaces the nine legacy
  `nixling-sys-<env>-usbipd-{backend,proxy}.{service,socket}` host
  systemd units (three envs × backend service + proxy service + proxy
  socket). On every startup, after the per-VM autostart pass,
  `nixlingd` derives one `PerEnvUsbipdSpec` per env that has at least
  one `usbipYubikey`-enabled VM and dispatches a broker `SpawnRunner`
  request (`vm_id = sys-<env>-usbipd`, `role = RunnerRole::Usbip`,
  `role_id = backend`/`proxy`) for each. The per-env TCP port follows
  the canonical `3241 + alphabetical-index-of-env` rule, indexed
  against **all** envs in the manifest to remain byte-compatible with
  the prior `lib.attrNames envs` Nix derivation. Proxy spawn is
  short-circuited to `Failed` within a single pass if its env's
  backend just failed, preventing a noisy proxy crash loop.
- **Argv generators in `nixling-host`**: new
  `generate_usbipd_backend_argv` and `generate_usbipd_proxy_argv`
  mirror the systemd `ExecStart` lines byte-for-byte under SNAPSHOT
  assertions, so a future host-side spawner can reuse them without
  divergence risk.
- **Transitional belt-and-braces**: the existing per-env systemd
  units in `nixos-modules/network.nix` continue to ship through
  P3 → P5 with a `scheduled-for-removal-in-P6` header. The broker
  currently returns `BundleIntentMissing` for the
  `sys-<env>-usbipd` intents (no DAG entries in `processes.json`
  yet), which the daemon translates to
  `PerEnvUsbipdOutcome::SkippedPendingBundle` so traffic continues to
  flow through the singleton units. The processes-json DAG entries
  land in P6 via `ph6-p6-unit-denylist-gate`, at which point the
  units are removed and the broker becomes the sole spawner.

### P3 ph3-p3-net-route-degraded-mode — daemon owns net-route preflight

- **Daemon-side net-route preflight** replaces the legacy
  `nixling-net-route-preflight.service` host singleton (the
  systemd unit ships through P3 → P5 as transitional belt-and-
  braces and is removed in P6 via `ph6-p6-unit-denylist-gate`).
  On every startup `nixlingd` probes each env's LAN bridge under
  `/sys/class/net/<bridge>/operstate`. Failed envs contribute
  their VMs to the autostart pre-degraded set, so impacted VMs
  surface as `Outcome::Degraded { reason }` in
  `nixling status` / `nixling vm list` instead of failing their
  unit. A persistent history at
  `<daemon-state-dir>/net-route-preflight-history.jsonl`
  tracks consecutive failures; after `N = 3` consecutive failed
  startup passes the daemon enters **operator-only mode**:
  read-only verbs (`status`, `host doctor --read-only`, `audit`)
  remain available, but the autostart pass is skipped entirely.
- **New mutating verb `nixling host reconcile --network --apply`**
  is the SOLE recovery path out of operator-only mode. It re-runs
  the broker-side network slice of `host prepare`
  (`ApplyNftables(host)` + per-env `ApplyRoute` + per-env
  `ApplySysctl`) without starting any VM, and on success resets
  the persistent consecutive-failure counter. The verb honours
  the standard `--dry-run` / `--apply` mandatory-flag-pair
  contract and requires admin (matches `hostPrepare` /
  `hostDestroy` posture).
- **New typed-error variant** `net-route-preflight-degraded`
  (exit code `66` — sibling of `otel-host-bridge-readiness-timeout`
  at exit code 65 in the operator-only-mode kind class; rebumped
  during the P3 wave-B integration to avoid collision with
  OtelHostBridgeReadinessTimeout). Its remediation prose points
  operators at `nixling host reconcile --network --apply` and the
  host-prepare explanation doc.
- **New host-prep step kind** `HostNetRoutePreflight` in
  `nixling_host::host_prep_dag::HostPrepStepKind` (typed-only —
  executed inline in the daemon; no per-VM DAG insertion).
- See `docs/explanation/host-prepare.md` §
  "Net-route preflight & operator-only mode".

### Breaking changes (manifest contract)

- **`vms.json` `_manifest.manifestVersion` 2 → 3** (P2
  `ph2-p2-manifestversion-bump`). The daemon-only end-state retires
  the per-VM systemd-unit reference fields that became meaningless
  once supervisor mode shipped, and pins the manifest to a single
  supported integer with no legacy compatibility window. The Rust
  `nixling_core::manifest_v04::MANIFEST_VERSION_CURRENT` constant
  enforces the bump: `ManifestV04::from_slice` refuses any other
  value (including the historical `2`) with a typed
  `manifest-parse-error` whose opaque reason is
  `manifest-version-mismatch`, so the daemon, the broker, and every
  CLI verb reject a v2 bundle outright. Operators must rebuild the
  manifest (and any vendored bundle) against the new schema before
  upgrading.
  - Producer: `nixos-modules/manifest.nix` `_manifestVersion` option
    default bumped to `3`; version-history paragraph extended.
  - JSON Schema: `docs/reference/manifest-schema.json` `manifestVersion`
    `const` pinned to `3`; pre-v3 (versions 0/1/2) marked historical
    and rejected.
  - Resolver: `packages/nixling-core/src/manifest_v04.rs` adds a
    post-parse `manifest_version == MANIFEST_VERSION_CURRENT` check
    that emits `manifest-version-mismatch`; covered by three new
    regression tests (legacy v2 rejected, v3 accepted, future v99
    rejected) on top of the existing fuzz / round-trip suite.
  - Fixtures + fuzz corpus + nixlingd / broker test literals (every
    `manifestVersion: 2` and stray `manifestVersion: 4` placeholder)
    realigned to `3`.
  - New golden baseline at `tests/golden/vms.json-p2-v3` (companion
    to the historical `vms.json-91d69b0` v2 fixture); `tests/vms-json-parity.sh`
    automatically picks the new fixture for the bumped version.
  - P6 still owns the prose schema/walkthrough refresh under v3
    (docs-r3-1, test-r2-1) — this entry covers only the version bump
    itself.

### Breaking changes (bash → Rust)

- The Rust CLI is now the primary documented operator path. Prefer
  `nixling vm start|stop|restart <vm> --apply`; the bash-era `up` /
  `down` verbs are compatibility fallbacks only.
- `NIXLING_NATIVE_ONLY=1` is the supported way to validate the
  daemon/native path without bash fallback. `NIXLING_LEGACY_BASH_OPT_IN=1`
  still forces the legacy CLI directly when you need the old path.
- `nixling audio …` and `nixling console <vm>` now stay on the Rust
  help/argument surface even though execution still bridges to the
  legacy runtime helper.
- Host USBIP policy now carries an optional vendor:product allowlist
  alongside explicit busid locks; regenerate host artifacts before
  expecting the broker-side policy to tighten.

### Added




- **`nixling host doctor --read-only` extended** (P3
  `ph3-p3-host-doctor-extended`). The doctor now reports broker-spawned
  singleton liveness (OtelHostBridge + per-env `usbipd` runners), the
  kernel-module matrix from the daemon's startup self-check, the
  Prometheus metrics-endpoint reachability, and the autostart degraded
  count. Doctor JSON output gains a structured `checks[]` array with
  per-check `name` / `status` / `detail` / optional `data` while
  preserving every legacy top-level field (`broker_ready`, `findings[]`,
  `summary`, `exitCode`) for existing scrapers. Exit-code policy is
  now `0` clean / `1` any warn / `2` any fail (was `0`/`1` only).
  Implementation persists two daemon-side reports at startup so doctor
  can read them client-side:
  - `<daemon-state>/kernel-module-report.json` after the startup
    kernel-module self-check (`packages/nixlingd/src/lib.rs`
    `persist_kernel_module_report`).
  - `<daemon-state>/autostart-report.json` after the autostart pass
    (`run_startup_autostart`).
  Wire contract: [`docs/reference/cli-output/host-doctor.schema.json`](docs/reference/cli-output/host-doctor.schema.json)
  + [`docs/reference/cli-output/host-doctor.md`](docs/reference/cli-output/host-doctor.md);
  CLI surface documented in [`docs/reference/cli-contract.md`](docs/reference/cli-contract.md#host-doctor-w3-p3).
  Integration coverage: `tests/cli-rust-native-host-doctor.sh` (10
  scenarios, including a live SOCK_SEQPACKET listener for the
  fully-healthy path). Honors the new env overrides
  `NIXLING_DAEMON_STATE_DIR` (default `/var/lib/nixling/daemon-state`)
  and `NIXLING_METRICS_URL` (default `http://127.0.0.1:9101/metrics`).

- **Tracing contract**: codified the bounded-attrs rule
  (`docs/reference/tracing-contract.md`) + added
  `tests/tracing-contract-lint.sh` static gate that fails closed if a
  Rust source file introduces a high-cardinality / leakable tracing
  attribute (`bundle = %path.display()`, `/nix/store/...` literals,
  `argv`, `secret`/`password`/`token`, `stdout`/`stderr` child-output
  bytes). Allowlist tail: `vm`, `env`, `role`, `step_id`, `operation`,
  `outcome`, `error_kind`, `op_count`, `elapsed_ms`, `parent_pid`,
  `exit`, `load_outcome`, `reason`, `drift_kind`. Closes
  observability-5 ( P3 ph3-p3-tracing-contract ).

- **docs/reference/loki-label-contract.md +
  tests/loki-label-cardinality-eval.sh** (P3
  `ph3-p3-loki-label-contract`). Pins the Loki label allowlist for
  nixling logs ingested via Alloy / OtelHostBridge to
  `{vm, env, role, severity, source}` with documented cardinality
  budgets (`vm ≤ 20`, `env ≤ 5`, `role ≤ 10`, `severity ≤ 5`,
  `source ≤ 5`). The static gate parses every `loki.source.*` stanza
  emitted by `nixos-modules/components/observability/{host,stack,
  guest}.nix`, rejects any label key outside the allowlist, rejects
  path-like values (no `/` in literal values, no absolute paths),
  and asserts the closed-enum literal counts stay within budget.
  Fixes the pre-contract drift in the three emitters: dropped
  `host` / `unit` / `job` labels from journald sources, added the
  closed-enum `role` (`workload`/`host`/`usbipd`) and the new
  `source` label (`journal`/`audit`) so per-source cardinality is
  visible at query time without re-introducing unbounded axes.
  `nixling-otel-host-bridge` forwards OTLP transparently — the
  contract applies at the emitter, not at the bridge.

- **Daemon Prometheus scrape endpoint shape** ( P3 prometheus-otlp )
  Canonical metric inventory for `nixlingd` documented at
  [`docs/reference/daemon-metrics.md`](docs/reference/daemon-metrics.md)
  and implemented in `packages/nixlingd/src/metrics.rs`. The daemon
  exposes a `GET /metrics` endpoint in Prometheus text-format v0.0.4
  on the public socket; the registry covers nine metrics
  (`nixling_daemon_vm_state`, `…_vm_start_duration_seconds`,
  `…_host_prep_step_duration_seconds`, `…_broker_request_total`,
  `…_broker_request_duration_seconds`, `…_ownership_drift_total`,
  `…_ssh_host_key_drift_total`, `…_pidfd_table_size`,
  `…_uptime_seconds`) with bounded label cardinality and three
  histogram bucket schedules. OTLP push is intentionally out of
  scope at the daemon — the in-stack Alloy scrapes the daemon and
  forwards via `otelcol.exporter.otlp`. Parity gate:
  `tests/daemon-metrics-eval.sh` asserts doc ↔ source agreement on
  metric names, kinds, labels, and bucket boundaries.

- **P3 OtelHostBridge readiness gate: typed gate blocks
  observability VM "ready" until the broker-spawned
  `RunnerRole::OtelHostBridge` runner satisfies its readiness
  predicate.** `dispatch_broker_vm_start` now invokes
  [`otel_host_bridge_readiness::await_otel_host_bridge_readiness`](packages/nixlingd/src/otel_host_bridge_readiness.rs)
  after the per-VM process DAG reports `overall_ok=true`, but
  only when the VM being started is the observability VM
  (`manifest._observability.vmName`) AND observability is enabled.
  The readiness predicate is the conjunction of (a) the
  OtelHostBridge runner being registered in `pidfd_table` for the
  obs VM and (b) the obs vsock host socket
  (`_observability.obsVsockHostSocket`) existing on disk — the
  side-effect-free proxy for "socket accept succeeded + first
  OTLP forward acknowledged". The pure
  [`evaluate_readiness`](packages/nixlingd/src/otel_host_bridge_readiness.rs)
  function returns `Ready` / `Pending { elapsed_ms }` /
  `Failed { reason }`; the side-effecting wrapper polls every
  100 ms until one of the terminal verdicts fires. Default
  deadline is 30 000 ms, overridable via
  `NIXLING_OTEL_BRIDGE_READINESS_TIMEOUT_MS`. On timeout the
  daemon falls back to **degraded mode** by default (VM stays
  up; structured `tracing::warn!` annotation is emitted); strict
  operators can set `NIXLING_OTEL_BRIDGE_READINESS_STRICT=1` to
  surface the timeout as a hard
  `TypedError::OtelHostBridgeReadinessTimeout { vm, elapsed_ms }`
  refusal (kind `otel-host-bridge-readiness-timeout`, exit code
  `65`). New operator reference:
  [`docs/reference/otel-host-bridge-readiness.md`](docs/reference/otel-host-bridge-readiness.md).
  Twelve new unit tests cover the pure evaluator truth table,
  the env-var parser, and the polling wrapper (eventual-ready,
  timeout-to-degraded, runner-exit-to-degraded). ( P3 otelbridge-readiness )
- **P3 kernel-module-check: daemon startup self-check on the
  kernel-module matrix the running bundle requires.** `nixlingd
  serve` now runs `kernel_module_check::run_kernel_module_check`
  after the pidfd-table restore and orphan-adoption steps but
  *before* the autostart pass. The pure
  [`check_kernel_modules`](packages/nixlingd/src/kernel_module_check.rs)
  function compares the trusted bundle's declared module
  requirements (REQUIRED: `kvm_intel|kvm_amd`, `vhost_net`, `tun`,
  `virtio_{net,blk,pci,console}`, plus `virtiofs` when any VM
  carries a Virtiofsd node and `udmabuf`/`drm_virtgpu` when any VM
  declares graphics; OPTIONAL: `nvidia`/`nvidia_uvm` warn-only on
  graphics hosts, `usbip_host` for USBIP-enabled VMs,
  `tpm_vtpm_proxy` for swtpm-backed VMs) against the parsed
  `/proc/modules` snapshot via
  `nixling_host::modules::LoadedModuleSet`. Missing REQUIRED
  modules refuse daemon startup with the new
  `TypedError::HostKernelModulesMissing` (kind
  `host-kernel-modules-missing`, exit code `64`); missing OPTIONAL
  modules emit a structured `tracing::warn!` line and flag the
  affected VMs as pre-degraded so the autostart pass skips them
  with `Outcome::Degraded` instead of looping.
  `autostart::execute_autostart_with_pre_degraded` is the new
  seam the daemon uses to thread the pre-degraded set into the
  existing autostart phases; the legacy `execute_autostart` is
  preserved as a thin wrapper. New operator reference:
  [`docs/reference/kernel-module-check.md`](docs/reference/kernel-module-check.md).
  Matrix-drift gate:
  [`tests/kernel-module-matrix-eval.sh`](tests/kernel-module-matrix-eval.sh)
  asserts the source-side constants stay in sync with the
  operator-reference table and that the typed-error wiring keeps
  its kind + exit-code contract. ( P3 kernel-module-check )
- **nixling-priv-broker.service + .socket** (socket-activated; SD_LISTEN_FDS) ( P0 )
- **nixlingd.service: restartIfChanged=false** ( P0 )
- **defaultSwitchReadiness.{p0..p7} schema waves** ( P0 )
- **Bundle digest verification** (O_NOFOLLOW + owner+mode+hash) ( P0 )
- **TypedError::BundleTampered** (exit 60, kind=bundle-tampered) ( P0fu2 )
- **Per-artifact hash verification in bundle resolver** ( P0fu2 )
- **tests/{nixlingd-startup-smoke,broker-caps-eval,broker-socket-activation-eval,broker-bundle-path-eval,readiness-waves-eval}.sh** ( P0, P0fu2 )
- **Regression tests for bundleHash requirement on schemaVersion v3 and
  unknown future schemas** ( P0fu3 H1 )
- **Gap-fix docs wave:** added the missing store/key lifecycle references,
  security runbook, Ubuntu/Fedora install walkthroughs, NixOS daemon-migration
  + uninstall runbooks, video sidecar reference, hardware-smoke walkthrough,
  unified error-envelope guidance, AGENTS workspace inventory updates, and the
  matching stale rollout-comment cleanup.
- **Tier-0 static-fast gate.** `tests/static-fast-tier0.sh` is the
  documented sub-60s shell syntax + shellcheck presubmit tier ahead of
  `tests/static-fast.sh`, so contributors can separate the ultra-fast
  bash/doc check from the heavier PR-loop gate.
- **W14 LiveNative flips: vm start/stop/restart + host prepare/destroy.**
  `packages/nixlingd` now routes those five verbs through direct
  daemon → broker helpers instead of `VerbReadiness::Pending`, using the
  live W12 broker paths for `SpawnRunner`, nftables/routes/sysctls,
  `/etc/hosts`, and NetworkManager unmanaged state. `vm stop` now
  returns a documented best-effort native `Applied` envelope while the
  supervisor pidfd table remains a W4-fu-fu follow-up; `host destroy`
  reuses the same broker ops with additive `destroy` flags on the wire
  so route/sysctl/hosts/nm teardown can flow through the trusted-bundle
  reconcile path.
- **W11 bundle resolver: synthesize broker intent rows from trusted
  bundle artifacts.** New `nixling_core::bundle_resolver::BundleResolver`
  loads `bundle.json` + `host.json` + `processes.json` + `manifest.json`
  from the broker-configured `bundle_path` and exposes nine
  `find_*_intent(&BundleOpId)` lookups (nft / route / sysctl / hosts /
  nm-unmanaged / usbip-firewall / usbip-bind / runner / socket). The
  BundleOpId encoding is deterministic (e.g. `nft:env:<env>`,
  `route:env:<env>:<idx>`, `runner:vm:<vm>:role:<role_id>`) so the
  daemon's `bundle_*_intent_ref` value is a *lookup key*, not the
  authority it points at — the security property "the broker never
  trusts a caller-supplied authority-bearing payload" is preserved.
  No schema break: zero new fields on `Bundle` / `HostJson` /
  `ProcessesJson`. 10 new round-trip + unknown-intent + sort-order
  tests; smoke harness 10 → 20 tests, all green.
- **W12 broker live wiring: resolve `BundleOpId` via W11 + thread pidfd
  over `SCM_RIGHTS`.** The non-bootstrap `dispatch_request` arms that
  previously surfaced `Unimplemented{target_wave:'W4-fu-fu (bundle
  resolver)'}` now drive end-to-end. New
  `protocol::send_json_frame_with_fds` helper; lazy-loaded
  `BundleResolver` via `OnceLock`; new typed errors
  (`BundleResolverUnavailable`, `BundleIntentMissing`, `LiveHandler`);
  nft / ip binary paths overridable via
  `NIXLING_BROKER_{NFT,IP}_BINARY` env. 3 new W12 integration tests
  cover the fd-bearing response path.
- **W13 W6-fu USBIP live executors + per-busid exclusivity lock.**
  Promotes `UsbipBind` / `UsbipUnbind` / `UsbipProxyReconcile` from
  `UnknownOperation` to live executors. New
  `ops::usbip_lock::{acquire_lock, release_lock, peek_owner}` manages
  `/run/nixling/locks/usbip/<bus_id>` files. `UsbipBindFirewallRule`
  also promoted from Unimplemented to live. usbip binary overridable
  via `NIXLING_BROKER_USBIP_BINARY` env.
- **W14 native mutating verbs: daemon API surface + CLI daemon-first
  dispatch + bash fallback.** Ships W14a (15 new
  `PublicRequest::{VmStart,VmStop,VmRestart,Switch,Boot,Test,Rollback,Gc,KeysRotate,Trust,RotateKnownHost,Migrate,HostPrepare,HostDestroy,HostInstall}`
  variants + typed `MutatingVerbResponse` envelope), W14b (daemon-
  side `dispatch_mutating_verb` with per-verb readiness classifier),
  and W14c (CLI `dispatch_mutating_verb` that tries the daemon first
  and applies the bash fallback rule). `NIXLING_NATIVE_ONLY=1` opts
  out of bash fallback; `NIXLING_LEGACY_BASH_OPT_IN=1` skips daemon
  and goes straight to bash. Default behavior preserves v0.4.0
  operator parity.
- **W15 W9-fu live host install + migrate writer (broker side).**
  `BrokerRequest::RunHostInstall(RunHostInstallRequest)` +
  `RunMigrate(RunMigrateRequest)` wire variants with matching typed
  responses. `nixling_core::bundle_resolver` gains
  `ResolvedInstallerIntent` + `ResolvedMigrateIntent` types + the
  `installer:host` / `migrate:host` BundleOpId intents.
  `live_run_host_install` writes installer artifacts + shells out to
  systemctl; `live_run_migrate` records per-VM migration markers
  under `/var/lib/nixling/migrate/<vm>.json`. Privileges authz rows
  added.
- **W14 LiveNative per-verb flips for switch/boot/test/rollback + gc +
  keys rotate + trust + rotate-known-host.**
  `nixling_ipc::broker_wire` adds five broker primitives
  (`RunActivation`, `RunGc`, `RunKeysRotate`, `RunHostKeyTrust`,
  `RunRotateKnownHost`) with typed responses. `nixling_core::
  bundle_resolver` now synthesises activation / gc / key-management
  intents from the trusted bundle + closure artifacts; bundleVersion
  bumps 3 → 4. The broker promotes the former W7b/W7c/W8
  `Unimplemented` arms to live handlers, the daemon flips the eight
  public verbs to direct broker helpers, and the privileges +
  daemon-api/schema artifacts are regenerated.
- **W16 W3 ifname unification via broker-emitted `host-runtime.json`.**
  W3 had two ifname algorithms producing the same format but
  different content (Nix SHA-256 first-8 vs Rust FNV-1a + Crockford
  base32). W3fu1 verified Nix CANNOT do FNV-1a (no shift builtin,
  multiplication overflows rather than wraps). W16 ships approach
  (b): `BundleResolver::host_runtime()` synthesises the canonical
  ifname set from `host.if_name_mappings`, and the W15
  `RunHostInstall` broker op now writes
  `/var/lib/nixling/runtime/host-runtime.json` so downstream
  consumers read ifnames from a single source of truth.
- **W17 minijail profile validator + observability-eval batching
  scaffold.** `BundleResolver::validate_minijail_profiles()` walks
  every per-role profile and asserts the W17 invariants (uid/gid
  non-zero unless carve-out; `/nix/store` read-only; cgroup subtree
  starts with `nixling/`; non-empty profile_id). 4 new validator
  tests. `tests/eval-cases/observability.nix` now covers all 23/23
  cases the shell `observability-eval.sh` exercises, in the same
  shape as W3a-1's `assertions.nix`.
- **W18 W10-fu cargo xtask release-notes + honest default-switch
  auto-flip.** `cargo xtask release-notes <version>` aggregates
  `CHANGELOG.md` Unreleased entries into a versioned section. Date
  computation uses a from-scratch SystemTime → civil-day algorithm
  so xtask stays chrono-free. `nixling.daemonExperimental.enable`
  now defaults to `allReady` only when every
  `defaultSwitchReadiness.<wave>.{implemented,validated}` pair is
  true. The shipped-code half (`implemented`) defaults to `true`
  today for `w4Fu` .. `w9Fu`; the proof half (`validated`) stays
  `false` until the operator records
  `/var/lib/nixling/validated/<wave>.json` with `wave`,
  `timestamp`, and `operatorSignature`. W20 now provides an
  evidence-writing mode for `w5Fu` / `w6Fu`.
- **W19 Ubuntu 24.04 Tier-1 smoke harness scaffold.**
  `tests/distro-matrix/ubuntu-2404-tier1.sh` (8-phase manual /
  nightly Layer-3 gate) + `fixtures/ubuntu-2404/` with expected
  audit ops + installer artifacts + README. On non-Ubuntu hosts the
  harness sets `NIXLING_UBUNTU_SCAFFOLD_ONLY=1` automatically;
  shellcheck warning-clean.
- **W20 hardware-smoke harness for this NixOS dev host.**
  `tests/hardware-smoke-gpu-yubikey.sh` 7-phase harness validates
  the W*-fu-fu rollup against this host's NVIDIA Quadro T1000 +
  USB / YubiKey hardware: preflight (GPU + USB + nix), yubikey-
  optional detection, cargo build, minijail invariants, bundle
  drift, example eval (graphics-workstation passes GREEN,
  with-entra-id hits a pre-existing himmelblau-tpm build issue),
  and live smoke documentation. After the manual live smoke passes,
  the same script can be rerun in evidence-only mode to write
  `/var/lib/nixling/validated/{w5Fu,w6Fu}.json`.

- **W4-fu broker clean-break: production binary uses real
  opaque-ID wire dispatch.** The W3fu2 H7 `layer1-bootstrap`
  default-feature deferral is retired. The broker's production
  binary (`cargo build` with empty default features) now
  dispatches against the real
  `nixling_ipc::broker_wire::BrokerRequest` opaque-ID shape via
  the new `runtime::dispatch_request` non-bootstrap arm. The
  bootstrap dispatch shape stays available behind an opt-in
  `--features layer1-bootstrap` feature for the legacy probe-*
  test harnesses (`broker-export-audit.sh`,
  `broker-scm-rights-fd-lifecycle.sh`, `broker-socket-acl.sh`).
  - Wire additions in `nixling-ipc::broker_wire`:
    `BrokerRequest::Hello(HelloRequest)` for daemon ↔ broker
    capability handshake; `BrokerResponse::Hello(HelloResponse)`
    for handshake reply; `BrokerResponse::Error(BrokerErrorResponse)`
    for typed broker errors on the real wire (mirrors bootstrap
    struct shape so the audit pipeline + daemon-side error
    propagation are shape-compatible); `impl BrokerRequest {
    fn op_name() / fn opaque_target_id() }` so audit pipeline is
    variant-agnostic between shapes.
  - Real-wire `dispatch_request` matches the 35-arm tuple-newtype
    shape: handshake + ValidateBundle + ExportBrokerAudit (with
    `BrokerAuditFilter` JSON-serialized to feed the existing
    `handle_export_broker_audit` signature) all route end-to-end;
    W*-fu live ops (OpenPidfd, ApplyNftables, ApplyRoute,
    ApplySysctl, UpdateHostsFile, SpawnRunner) surface
    typed-Unimplemented envelopes citing "W4-fu-fu (bundle
    resolver)" as the next deferral.
  - `nixling-priv-broker/src/lib.rs` `compile_error!` macro
    removed; real-wire path is now supported and the comment
    explains the new contract.
  - `nixling-core::privileges` + `nixos-modules/privileges-json.nix`
    grew a `Hello` broker operation row
    (handshake / nixlingd / read-only / audited) so the rendered
    privileges.json stays in lockstep with the wire enum.
  - `tests/broker-default-features-build.sh` now asserts the
    EMPTY default-feature set and fails fast with a pointer at
    the clean-break rationale if a future change re-adds
    `layer1-bootstrap` to `[features].default`.
  - `tests/rust-workspace-checks.sh` runs `cargo check` both with
    empty default features (real wire) AND with
    `--features layer1-bootstrap` (legacy probe-* harness).
  - Both feature modes pass 97 broker unit tests + 1 integration
    test; `tests/static-fast.sh` green.
- **End-to-end parity bridge for W*-fu mutating verbs.** Every
  `--apply` path on a stubbed W*-fu verb (`nixling switch`,
  `boot`, `test`, `rollback`, `gc`, `keys rotate`, `trust`,
  `rotate-known-host`, `migrate`, `vm start/stop/restart`) now
  routes through the legacy bash CLI via
  `exec_legacy_passthrough` instead of returning the
  `daemon-down` placeholder envelope. The native daemon-backed
  implementations are still tracked as W4-fu / W7-fu / W8-fu /
  W9-fu; until they ship, the legacy bash implementation
  remains the source of truth for actual mutation, and operators
  see the W10-fu deprecation-warning shim on every passthrough
  (silence via `NIXLING_LEGACY_BASH_OPT_IN=1` or
  `NIXLING_SUPPRESS_LEGACY_BASH_WARNING=1`). The Rust CLI's
  rust-native `--dry-run` planner output is unchanged.
  - `vm start/stop/restart` translates to the legacy bash
    `up/down/restart` verb names since the bash CLI does not
    expose the `vm <verb>` namespace.
  - `host prepare/destroy --apply` is *not* routed to bash — the
    legacy NixOS module owns Tier-0 host reconciliation, not the
    bash CLI; the existing tier-0 refusal envelopes
    (`tier-0-legacy-uses-nixos-module`,
    `single-writer-conflict`) remain correct.
- **W4-fu broker live_handlers module wired into lib.rs.** The
  W*-fu live broker handlers (`live_open_pidfd`,
  `live_apply_nftables`, `live_apply_sysctl`,
  `live_update_hosts_file`, `live_apply_route`,
  `live_spawn_runner`) are now public under
  `nixling_priv_broker::live_handlers`. The child-process body
  for `live_spawn_runner` (setgroups/setgid/setuid/execve, all
  async-signal-safe per signal-safety(7)) lives in
  `nixling_priv_broker::sys::pidfd_sys::clone3_spawn_runner` so
  it stays in the existing unsafe quarantine and `live_handlers`
  inherits the broker-lib `#![deny(unsafe_code)]`. 97 broker
  unit tests pass (6 new live_handler tests + 5 new envelope
  tests + 86 pre-existing).
- **`BrokerRequestEnvelope` + `BrokerCallerRole` in
  `nixling_ipc::broker_wire`.** Wire types the non-bootstrap
  runtime path needs so the broker can eventually drop the
  `layer1-bootstrap` feature default and dispatch the real
  opaque-ID `BrokerRequest` directly. Full serde + JsonSchema +
  `Default` derives; 5 new unit tests cover defaults, round-trip,
  and the predicate semantics.

### Changed

- **/run/nixling tmpfiles: nixlingd:nixling-launchers 0750** (canonical single source of truth) ( P0 )
- **nixling-priv-broker socket type: SOCK_SEQPACKET** (was SOCK_STREAM) ( P0fu2 H1 )
- **Broker --bundle-path defaults to /etc/nixling/bundle.json** (matches bundle.nix emitter) ( P0fu2 H2 )
- **Bundle resolver requires bundleHash for any schemaVersion >= 2 (and
  fails closed on unknown future schemas)** (was exact `"v2"` match only;
  a future `"v3"` bundle would have silently downgraded to warning) ( P0fu3 H1 )
- **Bundle resolver tracing attrs are bounded** (no filesystem paths in
  spans; bundle path is reported via the typed error envelope + audit
  log instead, per the daemon/broker tracing contract) ( P0fu3 H2 )
- **W14 LiveNative backlog now tracks all 13 mutating verbs.**
  Previous backlog (commit `eaf237a`) tracked 11; this commit
  adds hostPrepare + hostDestroy. The dispatch_request doc
  comment now distinguishes LiveNative (direct broker arm) from
  Pending (dispatch_mutating_verb routing).
- `tests/layer1-self-inventory.sh` now excludes `static-fast.sh`
  from its inventory: static-fast is the W3a-3 PR-loop sibling
  tier, not a Layer-1 gate body invoked from `tests/static.sh`.
- `docs/reference/daemon-api.md` regenerated to include the new
  `BrokerCallerRole` enum row (drift-gated by
  `tests/daemon-api-drift.sh`).

- **W10 main wave (default switch and deprecation) — contract
  layer.** New `docs/explanation/default-switch-and-deprecation.md`
  documenting:
  - Per-verb compatibility matrix (bash vs native vs live --apply
    per the W4-W9 H<N> deliverables) — 30+ rows covering every
    verb the v0.4.0 bash CLI ships plus the W4-W8 native
    additions.
  - Default-switch criteria: 5 prerequisites (W4-fu / W5-fu /
    W7-fu / W8-fu / W9-fu) that must all ship before
    `nixling.daemonExperimental.enable` flips to default-true.
  - Bash/systemd runtime deprecation timeline: D-day + 30 / 90 /
    180 day milestones culminating in nixling 1.0's bash-CLI
    removal.
  - Release-notes-cut hand-off: the `cargo xtask` task that
    promotes accumulated CHANGELOG "Unreleased" entries into
    versioned release notes lands in W10-fu.

  W10-fu (deferred): the actual `nixling.daemonExperimental.enable
  = true` flip in the default NixOS module, the bash-deprecation
  warning shim in the Rust dispatch helper, and the cargo xtask
  release-notes generator. All three are gated on the W4-fu /
  W5-fu / W7-fu / W8-fu / W9-fu prerequisites being met.

- **W9 main wave (packaging and onboarding) — wire + pure layer.**
  Upgraded `nixling host install` from a placeholder to a proper
  W9 contract:

  - **W9-H1** (`<this commit>`): `nixling host install
    --dry-run|--apply|--enable|--start|--no-start` skeleton. The
    --dry-run path returns the 5-step install plan as JSON
    (place units → write daemon-config.json → bind sockets →
    optional enable+start → smoke). --apply returns the W9-fu
    daemon-down envelope citing the Ubuntu Tier-1 walkthrough
    integration. Clap flag conflict-graph enforces the W9
    --apply prerequisite for --enable / --start / --no-start.
  - **W9-H2** (`<this commit>`): CHANGELOG entry. The Ubuntu
    Tier-1 walkthrough doc shipped in W4-H9 already covers the
    operator-facing install steps; W9-fu wires the daemon-side
    `--apply` path against the W4-H5 SpawnRunner broker
    integration.

  W9-fu (deferred): live `host install --apply` driving
  systemd-unit placement, daemon-config.json templating, socket
  ACL provisioning, and post-install smoke against the new
  daemon. `nixling migrate` for existing NixOS users
  (config-shape migration) lands here too.

- **W8 main wave (keys and trust lifecycle) — wire + pure layer.**
  Five new native `nixling keys` subcommands routed through the
  daemon-API shape with the W3 typed-error envelope contract:

  - **W8-H1** (`<this commit>`): `nixling keys list` — non-
    destructive enumeration of per-VM managed-key paths +
    known-hosts fragment paths. Live fingerprint probe is W8-fu.
  - **W8-H2** (`<this commit>`): `nixling keys show <vm>` —
    per-VM managed-key details. Today returns path + null
    fingerprint placeholder; W8-fu wires the broker-side
    `ssh-keygen -lf` invocation.
  - **W8-H3** (`<this commit>`): `nixling keys rotate <vm>
    [--apply|--dry-run]` — rotates the per-VM SSH keypair.
    --apply returns W8-fu daemon-down envelope with admin-auth
    note (the W8 destructive-key policy gates the broker-side
    op behind `nixling-admin` group membership).
  - **W8-H4** (`<this commit>`): `nixling keys rotate-known-host
    <vm>` — rotates the consumer's recorded known-hosts entry.
    Same daemon-down + admin-auth envelope shape.
  - **W8-H5** (`<this commit>`): `nixling keys trust <vm>` —
    trust on first use. Daemon-down + admin-auth envelope.
  - **W8-H6** (`<this commit>`): shell-completion regeneration
    via `cargo xtask gen-cli-shell-artifacts`.
  - **W8-H7** (`<this commit>`): CHANGELOG entry.

  W8-fu (deferred): broker-side `ssh-keygen` invocation for
  fingerprint probe + rotation; admin-auth enforcement in the
  daemon dispatch; per-key audit pipeline integration; consumer-
  supplied key guardrails for the `nixling.site.userAuthorizedKeys`
  surface (already validated at eval time by W3 assertions, but
  the runtime trust contract surfaces here).

- **W7 main wave admin-authorization fix (panel W7-H10 notable-1).**
  `w7_daemon_down_envelope` now appends an admin-authorization
  clause for the W7c destructive verbs (`gc` and `rollback`) so
  operators reading the daemon-down envelope during the W7
  skeleton phase understand the post-W7-fu admin-auth gate.
  Closes the only Notable finding from the W7-H10 code-review
  panel.

- **W7 main wave (store/build/switch lifecycle) — wire + pure layer.**
  Seven new native CLI verbs (build / generations / switch / boot /
  test / rollback / gc) routed through the daemon. Non-destructive
  verbs (build / generations) return planned-op JSON today; mutating
  verbs (switch / boot / test / rollback / gc) accept the W3
  --apply/--dry-run contract and return the documented daemon-down
  envelope on --apply pending the W7-fu broker-side
  hardlink-farm/activation implementation.

  - **W7-H1..H7** (`<this commit>`): seven `nixling <verb>` native
    CLI handlers wired into NativeCommand. Each respects the
    existing W3 typed-error envelope contract (HostErrorEnvelope
    with `kind`/`code`/`exitCode`/`whatWasChecked`/`observedState`/
    `remediation`/`docsAnchor`). The `vm` namespace migration
    pattern (W4-H7) is re-used: legacy bash dispatch now skips
    each W7 verb so the native handler is authoritative.
  - **W7-H8** (`<this commit>`): shell-completion regeneration via
    `cargo xtask gen-cli-shell-artifacts`. The 7 new verbs appear
    in bash/zsh/fish completions and the manpage.
  - **W7-H9** (`<this commit>`): this CHANGELOG entry. The W7
    cli-contract.md surface already documented every verb's
    expected output shape per W3 legacy-bash docs; this wave
    adds the native handler skeletons.
  - **W7-H10** (`<this commit>`): code-review panel after the
    skeleton lands.

  W7-fu (deferred to follow-up wave because it requires the W4-fu
  broker spawn integration + hardlink-farm invariants enforcement):
  per-VM `<store>/generations/<N>` materialization with same-
  filesystem fatal check, no-fallback-copy invariant, crash-safe
  symlink updates via tmp+rename, marker-file enforcement,
  current/booted symlink atomic swap; live `nixling switch
  --apply` driving the W4-H4 supervisor DAG to restart only
  changed roles; admin authorization for downgrade/destructive
  operations (gc / rollback past N generations); store-sync
  namespace + retention policy enforcement.

- **W6 main wave (USBIP and observability roles) — wire + pure
  layer.** Three foundational H<N>s: vsock-relay socat argv
  generator, USBIP bind/unbind argv generator, Layer-1 gate +
  CHANGELOG entry + code review. Live USBIP bind orchestration
  (broker variant promotion from UnknownOperation → Unimplemented
  → real impl) lands in W6-fu.

  - **W6-H1** (`<this commit>`): `nixling_host::vsock_relay_argv`
    — pure socat-based vsock relay argv generator. Covers the
    three W3-shipping shapes from
    `nixos-modules/components/observability/{host,guest,stack}.nix`:
    stack-VM `VSOCK-LISTEN:14317 UNIX-CONNECT:obs-ingress.sock`,
    guest egress `UNIX-LISTEN:<sock> VSOCK-CONNECT:2:14317`, and
    the host-bridge EXEC form's LISTEN side. `SocatEndpoint`
    closed enum: `UnixListen{path,max_children,mode}`,
    `UnixConnect{path}`, `VsockListen{port,max_children}`,
    `VsockConnect{cid,port}`. Refuses source-CONNECT shapes
    (two clients) and empty endpoint paths. 14 unit tests.
  - **W6-H2** (`<this commit>`): `nixling_host::usbip_argv` —
    pure `usbip {bind,unbind} --busid <bus-id>` argv generator
    with structural `bus_id` validation (B / B-P / B-P.S[.S...]
    canonical forms). Refuses shell metachars, letters, slashes,
    spaces, empty segments. 16 unit tests including 8 rejection
    cases for malformed bus ids.
  - **W6-H3** (`<this commit>`): Layer-1 gate
    `tests/w6-argv-shape.sh` pinned to each unit-test name.
    Wired into `tests/static.sh` (parallel) and
    `tests/static-fast.sh` (Phase 7 cross-cutting drift loop).
  - **W6-H4** (`<this commit>`): CHANGELOG entry.
  - **W6-H5** (`<this commit>`): code-review panel signoff.

  W6-fu (deferred): broker UsbipBind/UsbipUnbind variant
  promotion from UnknownOperation to Unimplemented{target_wave:"W6-fu"}
  then real impl invoking `nixling_host::usbip_argv`; per-busid
  locks, env exclusivity, USBIP proxy reconcile, debug bundle
  USBIP entries; observability tracing/log cardinality + PII rules
  for the relay (the existing W3 alloy/promtail wiring already
  enforces most cardinality limits — W6-fu adds the daemon-side
  enforcement).

- **W5 main wave (high-risk sidecars) — wire + pure layer.** The
  first four atomic H<N>s of W5 main land foundational, mergeable,
  fully unit-tested pure argv generators for the GPU + audio +
  video sidecars that microvm.nix's graphics runner forks today.
  Real hardware bring-up + the per-role minijail rollout + the
  `--enable` Tier-1 claim land in W5-fu because they require actual
  NVIDIA/render-device hardware to validate.

  - **W5-H1** (`<this commit>`): `nixling_host::gpu_argv` — pure
    `crosvm device gpu` sidecar argv generator. Matches the W0b
    runner-shape audit's inline `crosvm device gpu --socket
    <vm>-gpu.sock --wayland-sock <wayland> --params '{
    "context-types":"virgl:virgl2:cross-domain","displays":[{"hidden":true}],
    "egl":true,"vulkan":true}'` shape. Compact JSON params payload
    so byte-stable parity diff vs the audit fixture. 14 unit tests
    covering audit parity, all rejection paths, multi-display,
    subset context types, omitted EGL, and JSON round-trip.
  - **W5-H2** (`<this commit>`): `nixling_host::audio_argv` — pure
    vhost-device-sound sidecar argv generator. Matches
    `nixos-modules/components/audio/host.nix`'s ExecStart:
    `/run/nixling/vms/<vm>/nixling-<vm> --socket
    /run/nixling/vms/<vm>/snd.sock --backend pipewire`. 10 unit
    tests.
  - **W5-H3** (`<this commit>`): `nixling_host::video_argv` — pure
    `crosvm device video-decoder` sidecar argv generator. Matches
    `nixos-modules/components/video/host.nix`'s ExecStart:
    `crosvm device video-decoder --socket-path
    /run/nixling-video/<vm>/video.sock --backend vaapi`. 10 unit
    tests.
  - **W5-H4** (`<this commit>`): Layer-1 gate
    `tests/sidecar-argv-shape.sh` driving the W5-H1/H2/H3
    unit-test surface pinned by name. Wired into `tests/static.sh`
    (parallel) and `tests/static-fast.sh` (Phase 7 cross-cutting
    drift loop).
  - **W5-H5** (`<this commit>`): docs — this CHANGELOG entry; the
    component reference docs at `docs/reference/components-graphics.md`,
    `components-audio.md`, and the W4-H9 daemon-lifecycle reference
    already document the per-VM sidecar lifecycle; W5-H5 cross-links
    those existing docs to the new pure argv generators.
  - **W5-H6** (`<this commit>`): code-review panel. Same shape as
    W4-H11.

  W5-fu (deferred to follow-up wave because it requires real
  hardware): live GPU/audio/video integration via the W4-H5
  SpawnRunner broker variant with `RunnerRole::{Gpu, Audio, Video}`
  (added in W5-fu); manual hardware tests; per-role minijail profile
  rollout (ADR 0003); NVIDIA / render-device requirement
  documentation; `--enable` Tier-1 claim flip.

- **W4 main wave (headless daemon alpha) — wire + pure layer.** The
  first eight atomic H<N>s of W4 main land foundational, mergeable,
  fully unit-tested building blocks for the headless CH runner +
  virtiofsd + swtpm + supervisor + per-VM DAG + daemon-state +
  pending-restart machinery. Broker-side spawn execution + Ubuntu
  Tier-1 alpha smoke harness + orphan adoption/quarantine wiring
  land in W4-fu (the broker-side `SpawnRunner` op specifically is
  marked `target_wave: "W4-fu"` and returns `Unimplemented` until
  the follow-up wave lands the SCM_RIGHTS handoff).

  - **W4-H1** (`5edcab3`): `nixling_host::ch_argv` — pure Cloud
    Hypervisor argv generator. `ChArgvInput` (VM identity, closure
    paths, manifest network/share inputs, daemon-owned API socket
    path) → `Vec<String>` argv that matches the W0b parity oracle
    `tests/golden/runner-shape/cloud-hypervisor-argv-minimal.txt`
    for the headless `corp-vm` shape, modulo the daemon divergences
    documented in ADR 0004. Supports both `ChNetHandoff::TapFd`
    (broker SCM_RIGHTS) and `ChNetHandoff::PersistentTap` (broker
    `TUNSETOWNER`) net-handoff modes per the W3 ADR 0014 probe
    outcome. 21 unit tests.
  - **W4-H2** (`5edcab3`): `nixling_host::virtiofsd_argv` — pure
    virtiofsd argv generator. One instance per `microvm.shares` row;
    matches audit shape (`--socket-path=<vm>-virtiofs-<tag>.sock`,
    `--socket-group=kvm`, `--shared-dir=<host-path>`,
    `--thread-pool-size=$(nproc)`, `--posix-acl --xattr`,
    `--cache=auto`, `--inode-file-handles=prefer`, optional
    `--readonly` for the `ro-store` share). 19 unit tests.
  - **W4-H3** (`5edcab3`): `nixling_host::swtpm_argv` — pure swtpm
    argv generator. Long-lived `swtpm socket ...` argv +
    pre-start `swtpm_ioctl -i --unix <ctrl>` flush argv pairing
    the W3 `VmProcessInvariants::swtpm_pre_start_flush` invariant.
    17 unit tests. Wire-ready but only consumed by VMs that
    opt in (`nixling.vms.<vm>.tpm.enable = true`); the W4 Tier-1
    headless alpha walkthrough ships TPM disabled.
  - **W4-H4** (`b1cbb14`): `nixlingd::supervisor::dag` — pure
    per-VM DAG executor. `topo_sort` uses Kahn's algorithm with
    deterministic source-pop ordering; rejects cycles, self-loops,
    unknown-edge targets, and duplicate node ids with structured
    `DagError` variants. `DagExecutor<R: NodeRunner>` drives the
    topo-sorted DAG through an async-trait abstraction — on the
    first node failure the executor stops issuing spawn calls and
    marks remaining nodes `Skipped { predecessor }`. Per-node
    `NodeBudget { spawn, readiness }` defaults to 10s spawn / 30s
    readiness; threaded through to the runner. 11 unit tests.
  - **W4-H5** (`ac7a6c0`): `nixling_ipc::broker_wire::SpawnRunner` —
    opaque-ID broker variant. Per W3fu1 H1 (security-1), the daemon
    never names argv, env, uid/gid, caps, seccomp profile, kernel/
    initrd/cmdline strings, virtiofs sockets, TAP fds, vsock CIDs,
    or any other launch authority across the wire. SpawnRunner
    carries only `vm_id + role_id + role + bundle_runner_intent_ref
    + runtime_allocations` (closed-set: vsock-cid / tap-fd-slot /
    api-socket-path). Response is `(pid, start_time_ticks,
    pidfd_index)` with the pidfd delivered OOB via SCM_RIGHTS.
    5 wire tests including the per-field rejection of every
    legacy authority field (argv/env/uid/gid/caps/seccompProfile/
    kernelPath/initrdPath/cmdline/apiSocketMode/chBinaryPath/
    vsockCid) and the `runtime_allocations` closed-kind rejection.
    Migrated `tests/broker-enum-disposition.sh` from the frozen v1
    privileges schema to v2 (current schema) so it correctly checks
    the live operations set.
  - **W4-H6** (`378bcc6`): `nixlingd::supervisor::state` — daemon
    state persistence + restart reconciliation. Pure parser of
    `/proc/<pid>/stat` field 22 (`starttime`) — handles comm with
    spaces and parens by splitting on the LAST `)`. `reconcile`
    classifies each persisted snapshot as
    `Adopt` / `Quarantine { observed_start_time_ticks }` /
    `Missing` / `UnparseableProcStat`. `FilesystemSnapshotStore`
    writes one file per (vm, role_id) at
    `/var/lib/nixling/daemon-state/<vm>/runtime.<role_id>.json`
    via tmp+rename; `InMemorySnapshotStore` for tests. 21 unit
    tests.
  - **W4-H7** (`000bfdf`): `nixling vm {start,stop,restart,list}`
    CLI verbs routed through the native daemon-API. Today every
    `--apply` returns the W3 typed `daemon-down` envelope (broker
    spawn is W4-fu); `--dry-run` returns the 5-node DAG the
    supervisor would drive (`host-reconcile → store-preflight →
    virtiofsd-ro-store → ch → ssh-ready`). `vm list` returns the
    daemon's runtime view (empty today, JSON shape matches the
    post-W4-fu `ReconciliationReport`). Regenerated shell
    completions via `cargo xtask gen-cli-shell-artifacts`.
  - **W4-H8** (`e1c2c64`): `nixlingd::daemon_version` — daemon-level
    `[pending restart]` machinery. `DaemonVersionFile` (server_version,
    binary_path, started_at, protocol_version) persists to
    `/run/nixling/version` on daemon startup. `compute_restart_status`
    classifies the running daemon vs. the on-disk install path as
    `UpToDate` / `PendingRestart{running_path, on_disk_path}` /
    `DaemonNotRunning` / `VersionFileUnreadable{detail}`. Missing
    install path is intentionally treated as `UpToDate` rather
    than spurious pending-restart. 12 unit tests.
  - **W4-H10** (`c1e8b80`): Layer-1 gate panel. Two new gates
    `tests/ch-argv-shape.sh` + `tests/dag-topo.sh` pinned to each
    individual unit-test name so a regression that DELETES a test
    (rather than failing it) surfaces here as a missing case.
    Wired into `tests/static.sh` (parallel) and `tests/static-fast.sh`
    (Phase 7 cross-cutting drift loop).
  - **W4-H9** (`<this commit>`): documentation — new
    `docs/explanation/daemon-lifecycle.md` (per-VM DAG, readiness
    predicates, supervisor budget, fail-fast skip propagation,
    state persistence, restart reconciliation, pending-restart
    semantics); new `docs/how-to/headless-alpha-walkthrough.md`
    (Tier-1 Ubuntu clean-host → running headless CH VM target,
    today documented as W4-fu pending); new
    `docs/reference/store-virtiofs.md` (per-share virtiofsd shape
    + tag/socket/uid mapping cross-referenced from the W0b audit);
    `AGENTS.md` daemon-lifecycle row updated; `docs/explanation/design.md`
    threat-model delta noting the W4-H5 SCM_RIGHTS pidfd handoff
    surface.

- W4a — opening phase of W4 wave (close the 5 W3 Spec-corrections
  deferrals as the smallest, most isolated pieces first; main W4
  substreams — CH headless runner / virtiofsd / swtpm / supervisor
  / Ubuntu walkthrough — land in subsequent sub-phases).

  - **W4a-H1** (`217b7ce`): broker now prunes daily audit files
    older than `nixling.site.audit.retentionDays` (default 14;
    `0` disables) on every day-boundary rotation in
    `append_to_daily` and again on `AuditLog::open`. Filename is
    source of truth (`broker-YYYY-MM-DD.jsonl`); non-matching
    artifacts (operator notes, export tarballs) survive. Adds the
    `--audit-retention-days` broker CLI flag, the
    `nixling.site.audit.retentionDays` NixOS option, the prune
    method + Howard Hinnant `unix_days_from_ymd` inverse, and 4
    new tests (keeps-recent / disable / ignore-non-matching /
    round-trip date math). Closes the W3 Spec-corrections row
    for the retention half.

  - **W4 retire-shim**: drops the pre-W3 legacy
    `/var/lib/nixling/broker-audit.log` single-file compatibility
    path entirely. `AuditLog::write_entry` (W2-compat `AuditEntry`
    JSONL shape) and `write_op_record` (W3 `OpAuditRecord` shape)
    both write to the day's `broker-<utc-date>.jsonl` only;
    `export_lines` enumerates every dated file in `audit_dir`
    sorted chronologically and concatenates the matching lines.
    The broker `serve` CLI takes `--audit-dir <path>` instead of
    the prior `--audit-log-path` flag. The
    `broker-export-audit.sh` and `broker-socket-acl.sh` Layer-1
    gates were migrated atomically to assert against the daily
    file. Closes the second half of the W3 Spec-corrections audit
    row; threading the NixOS `audit.retentionDays` option through
    `daemon-config.json → nixlingd → broker spawn args` remains
    a W4 main-wave follow-up (the broker uses its 14-day default
    regardless of overrides until then per W4a-H1 Reserved
    caveat).

  - **W4a-H3** (`f942d59`): wires `FirewallCoexistencePolicy` into
    `HostJson` as an optional field (the
    `host_w3.rs::FirewallCoexistencePolicy` DTO existed since
    W3fu1 H4 but was unused). Nix emitter populates the static
    "no managed firewall detected → coexist" default; broker
    runtime probe will override at apply-time in W4 main wave.
    `xtask gen-schemas` regenerates `docs/reference/schemas/v2/host.json`
    with the new definition + its sub-enums (`FirewallManager`,
    `CoexistencePolicy`). `tests/host-json-drift-gate.sh`
    definition list now includes it. Closes the W3fu1 H8
    Spec-corrections row.

  Tracked deferrals (W4a follow-ups; carry-over to a future
  W4-focused session):
  - **W4a-H2** — unify the CLI host-verb refusal envelope with the
    daemon-API typed envelope. Requires extending
    `nixling_core::error::Error` with CLI-side variants AND
    reworking all 48 `host-*-*.{json,txt}` goldens. Invasive;
    not blocking other W4 work. **Interim state: both envelopes
    remain supported** —
    [`docs/reference/error-codes.md#w3-cli-host-verb-refusal-envelope`](docs/reference/error-codes.md#w3-cli-host-verb-refusal-envelope)
    documents the 7-field CLI shape and
    [`docs/reference/daemon-api.md#error-envelope`](docs/reference/daemon-api.md#error-envelope)
    the typed 6-field shape; both anchor into the same
    `docs/reference/error-codes.md` catalog so operators have a
    single source of truth for `docsAnchor`/`docs_anchor`
    resolution.
  - **W4a-H4** — unify the Nix↔Rust ifname algorithm by moving
    derivation entirely to broker runtime
    (`packages/nixling-host/src/ifname.rs::derive_from_env_vm`).
    Medium-large; depends on broker runtime probe scaffolding
    that the W4 main wave introduces anyway.
  - **W4a-H5 = W4b** — wire `host prepare/destroy --apply` to real
    broker reconcile ops (currently stubs returning
    `daemon-down`/`not-yet-implemented`). Multi-day work touching
    every W3 dispatch surface; belongs in a focused multi-session
    track of its own.

### Changed

- W3a — testing infrastructure overhaul. Cuts the cold-cache full
  panel gate from ~90 min / ~1.2 TiB peak `/nix/store` to ~30-40
  min / ~250-400 G peak, and introduces a sub-15-min fast PR-loop
  gate for the "is my change broken?" inner loop.

  - **W3a-1** (`82dc8a7`): `tests/assertions-eval.sh` rewritten to
    drive 25 of its 31 cases through a single batched
    `nix-instantiate --eval --strict --json` against
    `tests/eval-cases/assertions.nix` (was 31 separate cold
    nix-instantiate invocations, each re-booting the NixOS
    module-system evaluator). The remaining six cases — three
    throw cases that fall back to a focused per-case eval with the
    original override embedded in `FALLBACK_OVERRIDE`, and three
    feature-gated observability cases that need complex skip
    logic — keep the legacy per-eval path. Wall time
    32 min → 13 min (-59%); per-gate disk peak ~150 G → ~3 G for
    the eval phase. All 31 cases pass.

  - **W3a-2 (incremental gap closure):** the remaining eval-only
    assertion probes now force the `nixos.config.assertions`
    projection instead of `system.build.toplevel.drvPath` for the
    three focused throw fallbacks plus the observability
    CID-collision case. That closes the original forcing-expression
    migration gap without regressing the 31-case pass rate, but the
    fresh post-fix `time bash tests/static-fast.sh` measurement is
    still ~12m47s / ~560 G peak on this host, so the old ~5-8 min /
    ~3 G aspiration likely needs a focused W3a-2b.

  - **W3a-3** (`1b32e67`): new `tests/static-fast.sh` runs only the
    gates that catch the most-likely PR-loop regressions (parse,
    shellcheck, flake-check --no-build, rust-workspace,
    W1 bundle/schema invariants, W3 host-prepare canaries,
    cross-cutting drift). Skips the eval gates, mid-tier
    consumer-config evals, manifest contract, W2 broker daemons,
    per-example flake-check (~700 G disk), cli-contract-coverage,
    cli-json-drift, and audio component — those stay in the full
    `tests/static.sh` used by panel review. Verified cold-cache run
    12:53 wall / ~520 G peak. Mirrors static.sh's flock pattern
    (separate `.static-fast.lock`) and toolchain-provisioning
    (shellcheck + cargo via `nix shell` on demand).

  - **W3a-4** (`f651b1c`): `tests/static.sh` runs `nix store gc`
    between major phases (post-mid-tier-evals, post-eval-gates,
    post-w3-gates) instead of only at end. Each gc costs ~30 s
    but caps run-time peak at ~250-400 G instead of ~1.2 TiB.
    New helpers in `tests/lib.sh`: `nl_disk_free_gib`,
    `nl_nix_store_used_gib`, `nl_phase_gc`,
    `nl_check_disk_budget`. New env var
    `NL_GATE_DISK_BUDGET_GIB=<gib>` (default 0 = unbounded)
    fails the gate at each phase boundary if free disk drops
    below the budget — actionable error instead of waiting for
    the 15 G emergency watchdog SIGTERM.

  Tracked deferrals (W3a-1b follow-up): consolidating
  `tests/observability-eval.sh` (23 heterogeneous probes, each
  with a different `body` expression) into a single batched eval
  was scoped out of this round; the assertions-eval batching alone
  is already the bigger time-saver. Pattern is straightforward
  (per-case body fn in the Nix file vs uniform expect-failure
  shape from assertions-eval) but requires per-case migration.

- W3 follow-up round 7 finding fixes ( W3fu7 H1 H2 ):
  W3fu1 work-review R7 returned 7/9 signoff with 2 findings (both
  MED). Two atomic commits close both:

  - **H1** (docs-1 MED, `12f38b3`): W3fu6 H2 corrected
    `daemon-api.md` / `privileges.md` / `AGENTS.md` for the audit
    retention + legacy compat shim drift but missed `SECURITY.md`,
    which still claimed the W3 broker audit log was "rotated daily
    with a 14-day default retention". W3 ships neither a
    `nixling.site.audit.retentionDays` option nor a prune loop;
    both the daily file and the legacy compat shim grow until W4
    retires them. Updates the W3 trust-boundary bullet and points
    operators at `daemon-api.md#audit` "W3 retention (Spec
    correction)".

  - **H2** (product-1 MED, `3660464`): the `host prepare` /
    `host destroy` / `host doctor` exit-code tables in
    `cli-contract.md` did not list the W3 refusal codes the CLI
    actually emits — exit 1 was described only as
    `host-check-warning` (advisory) when the W3-stub `--apply`
    path on Tier 1/1-later/2 returns `daemon-down`; exit 78 was
    described only as `tier-0-legacy-uses-nixos-module` when
    missing-flag paths return `--apply-or-dry-run-required`;
    `host doctor` listed exit 2 / `usage` when it actually returns
    exit 78 / `--read-only-required`. Updates the three tables to
    list both anchors per exit code where applicable, with anchor
    links pointing at the W3 CLI host-verb refusal envelope
    section added by W3fu5 H1.

- W3 follow-up round 6 finding fixes ( W3fu6 H1 H2 ):
  W3fu1 work-review R6 returned 7/9 signoff with 2 findings (both
  MED). Two atomic commits close both:

  - **H1** (test-1 MED, `fe2ce1a`): the W3fu5 H3
    `bind_public_socket_chgrps_to_public_socket_gid_even_when_non_root`
    regression test set `public_socket_gid` to the caller's primary
    gid, so a regression that re-introduced the old `is_root()`
    chown gate would pass silently (the socket already inherits
    primary gid via umask, no chown needed for match). Adds a
    `distinct_supplementary_gid()` helper that reads
    `unistd::getgroups`, picks the first supp gid different from
    primary, and uses it as `public_socket_gid`. Now the only way
    the socket can carry the expected gid post-bind is if
    `bind_public_socket` actually called the chown non-root —
    proving the W3fu5 H3 fix. Skips with a visible log line when no
    distinct supp gid exists (minimal CI containers).

  - **H2** (security-1 MED, `ad6e58d`): three audit-pipeline docs
    contradicted the W3 implementation. `docs/reference/daemon-api.md`
    claimed the legacy `/var/lib/nixling/broker-audit.log` was "no
    longer written or read by W3+ code" and listed 14-day retention
    via `nixling.site.audit.retentionDays`; `docs/reference/privileges.md`
    repeated the retention claim; AGENTS.md said the legacy path was
    "removed; consumers must read from /var/lib/nixling/audit/".
    Actual `AuditLog::open` writes to BOTH the daily file AND the
    legacy file (W3 compat shim for `ExportBrokerAudit` consumers
    and the `broker-export-audit.sh` / `broker-socket-acl.sh`
    Layer-1 gates), and no `retentionDays` option / prune loop
    exists. H2 corrects the three docs to describe the actual W3
    shape, adds a "W3 retention (Spec correction)" subsection to
    daemon-api.md, and adds a `plan.md` Spec-corrections row
    tracking the W4 work (prune-on-rotate loop, retentionDays
    option, retire the legacy compat shim — all in one commit).
    No code changes.

- W3 follow-up round 5 finding fixes ( W3fu5 H1..H5 ):
  W3fu1 work-review R5 returned 4/9 signoff with 5 findings
  (1 HIGH product-1 + 3 MED + 1 LOW). Five atomic commits close
  all five:

  - **H5** (docs-1 LOW, `0232ad4`): the W3fu4 CHANGELOG entry
    misattributed R4 finding IDs (claimed H1 closed `docs-3`, H4
    closed `docs-2`, H5 closed `docs-1`). Per the R4 mapping,
    `docs-1` was the missing W3fu3 CHANGELOG entry, `docs-2` was
    the missing W3fu4 CHANGELOG entry, and `docs-3` was the
    AGENTS.md multi-finding tag rule. This commit relabels so H1
    only closes `product-1`, H4 closes `docs-1 + docs-2` (both
    CHANGELOG entries), and H5 closes `docs-3` (the AGENTS.md
    rule). No substantive content changes.

  - **H2** (nixos-1 MED, `45a7993`): nixos-modules/host-json.nix
    advertised `/etc/NetworkManager/conf.d/90-nixling-unmanaged.conf`
    and `# BEGIN/END nixling managed hosts` ownership-marker strings
    that diverged from the Rust broker's canonical constants
    (`packages/nixling-priv-broker/src/ops/nm.rs::DEFAULT_NM_CONF_PATH`
    = `00-nixling-unmanaged.conf`,
    `packages/nixling-host/src/routes.rs::HOSTS_MANAGED_BEGIN/END`
    = `# nixling-managed begin/end`). Aligns the Nix emitter +
    deny-unknown fixtures + v1 reference doc to the broker
    constants and adds a CANONICAL_LIVE_OWNERSHIP cross-check in
    `tests/host-json-drift-gate.sh` so regressions fail closed.

  - **H3** (security-1 MED, `a60bd33`): W3fu2 H5 left the non-root
    daemon's public socket ACL chain broken end-to-end: tmpfile
    `/run/nixling` was `0750 nixlingd nixlingd` (launcher users
    couldn't traverse), `bind_public_socket` only chowned when
    `euid == root` (production unit is always non-root, so socket
    group stayed `nixlingd` not `nixling-launchers`), and
    `validate_lock_parent` expected the pre-W3fu2-H5 root-owned
    shape (would have failed startup). Aligns tmpfile to
    `0750 nixlingd nixling-launchers`, drops the `is_root()` gate
    around the socket chown (chgrp succeeds via SupplementaryGroups
    membership), refactors `validate_lock_parent` to accept the
    production tmpfile shape, and adds 5 regression tests in
    `packages/nixlingd/src/lib.rs::runtime_acl_tests`.

  - **H4** (networking-1 MED, `1c820ec`): `host_check::run` walked
    `host.kernel_modules` for module presence but never read
    `KernelModulesEntry.sysctls`. For br_netfilter that meant
    `bridge-nf-call-iptables=0` etc. were structurally declared
    but operationally inert — bridge traffic could traverse legacy
    iptables/ip6tables/arptables outside the inet nixling policy
    without `host check` saying anything. Adds
    `ProbeSource::module_sysctl_value`, per-module sysctl
    enforcement (Pass / Fail-drift / Fail-missing /
    Fail-malformed), and 4 unit tests covering br_netfilter
    pass/drift/missing/absent cases. Extends
    `_nl_host_check_sysctls_json` to include module sysctls so
    existing pass/warn fixtures stay green.

  - **H1** (product-1 HIGH, `1bb3ebc`): `docs/reference/daemon-api.md`
    claimed every public CLI/API failure used the typed
    `{kind, code, message, remediation, docsAnchor, owningCommand}`
    envelope, but the W3 CLI host verbs (`host prepare`,
    `host destroy`, `host doctor`, `host install`) emit a richer
    7-field operator-UX envelope (`kind / code / exit_code /
    what_was_checked / observed_state / remediation / docs_anchor`)
    anchored in plan.md §"W3 CLI contract docs + per-error golden
    table" and pinned by the host-* goldens. Worse, four CLI
    codes (`daemon-down`, `not-yet-implemented`,
    `--read-only-required`, `--apply-or-dry-run-required`) had no
    catalog anchors at all — the CLI emitted
    `docs/reference/cli-contract.md#<section>` instead of stable
    code-specific anchors. Adds a "W3 CLI host-verb refusal
    envelope" section to error-codes.md with the four missing
    anchors, scopes daemon-api.md's typed envelope claim to
    daemon-API surfaces, cross-links the two envelopes, and
    repoints lib.rs to the new error-codes.md anchors. W4 will
    unify the two envelope shapes once the typed `Error` enum
    grows cli-side variants.

- W3 follow-up round 4 finding-fix-batch ( W3fu4 H1 H2 H3 H4 H5 ):
  W3fu1 work-review R4 returned 5/9 signoff with 6 findings
  (1 HIGH + 5 MEDIUM); this batched commit closes all six.

  - **H1** (product-1 HIGH): two leftover operator-prose
    surfaces still claimed `--apply` was a fully-wired W3 verb after
    W3fu3 H1's top-of-doc downgrades. H1 closes the gap in
    `docs/explanation/host-prepare.md` "Tier behavior" bullet list
    (Tier 1 / Tier 1-later / Tier 2 are now described as
    `host check` + `--dry-run` today / `--apply` W3-stub until W4)
    and in `docs/how-to/host-prepare.md` walkthrough code-block
    (annotated each `sudo nixling host {prepare,destroy} --apply`
    line with the W3-stub disposition + W4 carry-over).

  - **H2** (security-1 MED): tightened the W3fu3 H2 rejection helper.
    `assert_unknown_field_rejected` only asserted any error; a
    future regression that reintroduced a numeric authority field
    like `ownerUid` / `ownerGid` / `mtu` could pass via serde
    type-mismatch on a string-valued legacy value. H2 renames the
    helper to `require_wire_unknown_field_rejection` and makes it
    assert `error.kind() == "wire-unknown-field"` specifically.
    Adds a `legacy_value(field)` helper so each per-field test
    injects values matching the original wire type (numeric for
    uid/gid/mtu; bool for isolated/neighSuppress; string for
    name/hash fields). Every per-field test now proves the field
    was refused by the `deny_unknown_fields` contract, not by
    incidental type-mismatch.

  - **H3** (test-1 MED): the W3fu3 H8 parity gate had a fail-open
    path: when the smoke `host.json` had empty/missing
    `ifNameMappings`, both the bash gate (returned OK before
    dispatching to cargo test) and the Rust test (returned early
    with a "trivially passed" log) treated it as a no-op. A
    regression that drops all Nix-emitted mappings would have been
    invisible. H3 makes both sides fail closed: the bash gate
    fails on `count == 0`, and the Rust test panics on missing /
    empty `ifNameMappings` when its env var IS set (still skipped
    when the env var is unset so plain `cargo test` runs are
    unaffected).

  - **H4** (docs-1 MED + docs-2 MED): the W3fu3 batched fixes landed
    without any CHANGELOG entries, so the Keep-a-Changelog record had
    no rows for H1 / H2 / H3 / H4+H5+H6 / H7+H8 (docs-1 = missing
    W3fu3 entry); the W3fu4 batched commit itself also needed an
    entry (docs-2 = missing W3fu4 entry). H4 retroactively adds the
    grouped W3fu3 round entry (see the `W3 follow-up round 3
    finding-fix-batch` block below) covering all 8 W3fu3 fixes, and
    adds this W3fu4 grouped entry.

  - **H5** (docs-3 MED): `AGENTS.md` "Commit conventions" documented
    only single-finding trailing tags, but the W3fu3 chain
    landed two enumerated multi-finding tags
    (`( W3fu3 H4 H5 H6 )`, `( W3fu3 H7 H8 )`) that the body
    rationale called out but the policy did not yet permit. H5
    extends the AGENTS.md "canonical tag form" list with an
    explicit multi-finding allowance (`( W<N>fu<M> <S1><N1> <S2><N2> ... )`)
    that documents when batching is acceptable (multiple findings
    that genuinely express one coherent change; the alternative
    would be 3+ trivially-small commits with the same statement)
    and what the commit body must call out (which findings,
    why batched). The W3fu3 multi-finding commits remain in
    history; the policy now codifies the pattern they used.

- W3 follow-up round 3 finding-fix-batch ( W3fu3 H1 H2 H3 H4 H5 H6 H7 H8 ):
  W3fu1 work-review R3 (4/9 signoff with 9 findings) was closed by 5
  atomic W3fu3 commits, each carrying the canonical trailing-tag
  per AGENTS.md "Commit conventions" (now extended to enumerated
  multi-finding form in W3fu4 H5). Summary per H, in commit-landing
  order:

  - **H3** `37865cb` (rust-1, MEDIUM): added a crate-root
    `#[cfg(not(any(feature = "layer1-bootstrap", test)))]
    compile_error!` to `packages/nixling-priv-broker/src/lib.rs` that
    names the W4 carry-over reason and points the user at the
    supported solutions. `cargo check --no-default-features` now
    fails with the explicit pointer instead of cryptic
    unresolved-import errors.

  - **H4 H5 H6** `3de86f6` (docs-1 + docs-2 + docs-3): aligned three
    reference / load-bearing docs to point at `schemas/v2/` as the
    current bundle baseline — `docs/reference/privileges.md`
    "machine-readable source" paragraph, `AGENTS.md` "Critical
    subsystems / Manifest bundle" row, and added the missing
    `plan.md "Spec corrections / known drift"` row documenting the
    W3fu2 H4 Nix↔Rust ifname dual-algorithm and W4 unification
    path (the row the W3fu2 H4 CHANGELOG entry had already
    promised). v1 schemas remain in tree as the frozen W2 baseline.

  - **H1** `18db8b3` (product-1, HIGH): extended the W3-stub
    `--apply` disposition from `compatibility.md` /
    `support-matrix.md` (W3fu2 H2) into the remaining three
    operator-facing docs: `docs/reference/cli-contract.md`'s
    `host prepare` and `host destroy` sections,
    `docs/explanation/host-prepare.md` (top-of-doc status note),
    and `docs/how-to/host-prepare.md` (top-of-doc status note +
    verb-summary table annotation).

  - **H2** `37f577f` (security-1, MEDIUM): added field-complete
    per-field rejection guards in `packages/nixling-ipc/src/broker_wire.rs`
    via the new `assert_unknown_field_rejected(kind, base, unknown)`
    helper and three opaque-only payload constructors. Four
    data-driven tests (`create_persistent_tap_rejects_each_legacy_authority_field`,
    `create_tap_fd_rejects_each_legacy_authority_field`,
    `set_bridge_port_flags_rejects_each_legacy_authority_field`,
    `usbip_bind_firewall_rule_rejects_each_legacy_authority_field`)
    loop the helper over the 21 dropped raw authority fields so a
    future regression reintroducing exactly one named field fails
    closed with the precise field in the panic message.

  - **H7 H8** `e0b72fd` (test-1 HIGH + test-2 MEDIUM): made the
    `tests/host-prepare-idempotency.sh` docstring + PASS line
    honestly state what it actually exercises today (drift-digest
    stability via `hash_inet_nixling_table`) instead of falsely
    claiming the full prepare→dry-run-empty→apply-zero-mut→destroy
    →destroy-noop oracle. The full state-machine oracle is W4
    follow-up work; the gate's existing checks (cargo invocation
    + zero-test-failure branch) are unchanged. Separately, replaced
    the `tests/ifname-nix-rust-parity.sh` shell regex with a
    `cargo test -p nixling-host --
    nix_emitted_ifnames_pass_looks_nixling_owned` invocation that
    feeds the rendered smoke `host.json` path to a new Rust test
    in `packages/nixling-host/src/ifname.rs` calling the real
    `looks_nixling_owned` predicate. The shell regex is removed;
    the only oracle now is the production Rust function, so a
    future predicate tightening automatically re-validates the
    Nix emitter without a parallel regex edit.

- W3 follow-up round 2 finding-fix-fix ( W3fu2 H4 ):
  Initial W3fu2 H4 wired `tests/ifname-nix-rust-parity.sh` into the
  W1 bundle/schema parallel-gate pool but forgot to prewarm the
  smoke `host.json` cache before the pool started. With
  `nl_smoke_bundle_host_json` cold, the parity gate triggered a
  fresh `nix eval ... getFlake (toString /home/paydro/projects/nixling)`
  during the same window in which sibling parallel gates
  (`tests/vms-json-parity.sh`, `tests/bundle-drift.sh`) created and
  reaped scratch files under `$ROOT`. The flake source capture saw
  the per-test scratch path appear and then disappear and
  fail-closed with
  `path '//$ROOT/.vms-json-parity.XXXXXX' does not exist`.

  Resolution: add `nl_smoke_bundle_host_json >/dev/null` to the
  existing "W1 smoke cache prewarm" block in `tests/static.sh`
  alongside the pre-existing `nl_smoke_vms_json` and
  `nl_smoke_bundle_privileges_json` prewarms. With the cache hot,
  the parity gate hits the cached path and never re-enters
  getFlake. This is the canonical pattern documented in W2fu4
  H8/H9 for getFlake-source-capture race avoidance.

- W3 follow-up round 2 finding-fix ( W3fu2 H1 ):
  W3fu1 work-review R2 rust-1 + security-1 found that the W3
  broker wire still carried caller-supplied authority fields on
  four mutating broker variants, contradicting the file-level
  invariant the W3fu1 H1 commit message proclaimed ("the daemon
  never names raw paths, raw uids/gids, raw argv, raw nft rule
  text, raw routes or raw sysctl values"):

  - `CreatePersistentTapRequest` / `CreateTapFdRequest`: carried
    `ifname_derived: IfName`. The broker is the only authority for
    the interface name once the bundle is loaded; the field gave
    a compromised daemon a free choice of link.
  - `SetBridgePortFlagsRequest`: carried `bridge: IfName`,
    `port: IfName`, `isolated: bool`, `neigh_suppress: bool`.
    All four are policy that the bundle's per-role
    `BridgePortFlags` row pins; the wire variant let the daemon
    override the matrix.
  - `UsbipBindFirewallRuleRequest`: carried `bus_id: String` and
    `rule_hash: String`. `bus_id` is interpolated into nft rule
    text in `packages/nixling-host/src/nftables.rs:241-243`
    without a validating newtype or escaping; `rule_hash` is
    the broker's own drift digest and must not be caller-supplied.

  H1 removes the listed fields from each request struct and
  replaces `UsbipBindFirewallRuleRequest`'s raw fields with a
  single opaque `bundle_usbip_firewall_intent_ref: BundleOpId`
  mirroring the `ApplyNftables` / `UpdateHostsFile` pattern. The
  broker now resolves everything server-side from its trusted
  bundle copy via `role_id` + `vm_id` / `scope_id` /
  `bundle_*_intent_ref`. Observed ifnames and flag readbacks remain
  in the response shape (`TapReadyResponse`,
  `BridgePortFlagsResponse`) where they are audit data, not
  authority.

  Coupled changes:

  - `packages/nixling-ipc/src/broker_wire.rs`: 4 struct edits +
    inline rationale + 7 new round-trip tests (one
    opaque-only round-trip + one rejection guard per affected
    variant, plus the rewritten `usbip_bind_firewall_rule_round_trips`).
  - `packages/nixling-ipc/src/lib.rs`: rewrote
    `invalid_ifname_bubbles_up_as_typed_error` to assert
    `wire-unknown-field` rejection on the dropped `ifnameDerived`
    field instead of the no-longer-applicable
    `wire-ifname-invalid` shape.
  - `docs/reference/schemas/v2/wire-protocol.json` and
    `docs/reference/daemon-api.md` regenerated via
    `cargo xtask gen-schemas` + `cargo xtask gen-daemon-api`.

  No production code change is needed in the broker runtime: the
  `layer1-bootstrap` wire shadows already use struct-form variants
  (`{ opaque_target_id: None }`) per W3fu2 H7. When W4 wires the
  non-bootstrap runtime against the current `nixling_ipc::broker_wire`
  enum, the broker's reconcile ops will read the resolved authority
  from the trusted bundle keyed by these opaque IDs.

- W3 follow-up round 2 finding-fix ( W3fu2 H4 ):
  W3fu1 work-review R2 nixos-2 + networking-2 found that the Nix-
  emitted `ifNameMappings[].derivedIfname` values in
  `nixos-modules/host-json.nix` did not pass the Rust
  `nixling_host::ifname::looks_nixling_owned` predicate. The pre-
  W3fu2 emitter produced names of the form
  `nl-bridge-<8 lower-case hex>`:

  - 18 bytes total — exceeds `IFNAMSIZ-1 = 15`, so the kernel
    would have rejected bridge creation;
  - `bridge-` after the `nl-` prefix is not the single Crockford-
    alphabet character that `looks_nixling_owned` expects after the
    prefix;
  - lower-case hex is outside the Crockford alphabet
    `0-9A-HJKMNPQRSTVWXYZ` that the predicate validates.

  Result: `looks_nixling_owned` returned `false` on every Nix-
  derived name, so host-LAN-CIDR derivation and IPv6-off preflight
  could miss every nixling-owned link.

  H4 reshapes the Nix emitter so the output passes the Rust
  predicate:

  - role tag collapses from full word (`br`/`up`/`tap`) to a
    single character (`b` for both LAN and uplink bridges, `t` for
    workload TAPs) per the Rust `DerivedRole::tag()` contract;
  - hash is `lib.toUpper` of the SHA-256 first-8-chars output —
    upper-case hex `0-9A-F` is a strict subset of the Crockford
    alphabet, so the predicate accepts it;
  - the full input role name (`br`/`up`/`tap`) is preserved in
    the *hash input string* so an env's LAN and uplink bridges
    still hash to distinct digests;
  - emitted name is `nl-<tag><HASH>` (12 bytes) which fits
    `IFNAMSIZ-1`.

  The Nix hash algorithm (SHA-256-8-upper-hex) still differs from
  the Rust algorithm (FNV-1a + Crockford base32). That is OK
  because the broker uses `looks_nixling_owned` to *filter*
  nixling-owned interfaces, not to reconstruct them; the algorithms
  only need to agree on the *output format* the predicate
  recognises. W4 may unify the algorithms by moving derivation to
  broker-runtime; tracked in plan.md "Spec corrections".

  Coupled additions:

  - `tests/lib.sh::nl_smoke_bundle_host_json` helper renders
    `nixling._bundle.hostJson.jsonText` once per run and caches the
    path under the existing `_nl_smoke_cache_dir`, mirroring the
    pre-existing `nl_smoke_bundle_privileges_json`.
  - `tests/ifname-nix-rust-parity.sh` reads the smoke host.json,
    `jq`-extracts every `ifNameMappings[].derivedIfname`, and
    asserts it matches `^nl-[bt][0-9A-F]{8}$`. Wired into
    `tests/static.sh`'s W1 bundle/schema parallel group next to
    `host-json-drift-gate.sh`.

- W3 follow-up round 2 finding-fix ( W3fu2 H2 ):
  W3fu1 work-review R2 product-1 + networking-1 found that the
  pre-W3fu2 `docs/reference/support-matrix.md` and
  `docs/reference/compatibility.md` advertised Tier 1 Ubuntu and
  Tier 1-later Fedora as having "full W3 `host prepare --apply`
  reconcile supported", but W3fu1 H1 `cmd_host_prepare` /
  `cmd_host_destroy` actually route every non-Tier0 `--apply` to a
  `daemon-down` / `not-yet-implemented` envelope: the broker
  reconcile ops (`ApplyNftables`, `ApplyNmUnmanaged`, `CreateTapFd`,
  `CreatePersistentTap`, `SetBridgePortFlags`, `ApplySysctl`,
  `ApplyRoute`, `UpdateHostsFile`, `UsbipBindFirewallRule`) all
  return `BrokerError::Unimplemented { target_wave: "W4" }` per the
  W3 broker dispatch policy.

  H2 corrects the support tables to match the actual code state per
  AGENTS.md "Existing code is canon":

  - `docs/reference/compatibility.md` table: Tier 1 / Tier 1-later /
    Tier 2 `host prepare --apply` and `host destroy --apply` columns
    change from "supported" to "W3-stub (returns `daemon-down` /
    `not-yet-implemented`; production runtime wired in W4)". The
    Tier 0 NixOS rejection (`tier-0-legacy-uses-nixos-module`, exit
    78) and the Tier 1 / Tier 1-later / Tier 2 `host check` and
    `--dry-run` columns are unchanged because those code paths
    really do work today.

  - `docs/reference/support-matrix.md` tier-model table: Tier 1's
    "Full W3 `host prepare --apply` reconcile is supported" prose
    is replaced with the actual scope ("`host check` and `host
    prepare --dry-run` are supported today; `--apply`/`destroy` are
    W3-stubbed until W4"). Tier 1-later and Tier 2 inherit the same
    disposition note.

  - plan.md "Spec corrections / known drift" table gets a new row
    documenting the documentation drift and the W4 plan to reverse
    it once the production runtime lands.

  The reviewer's preferred fix was full broker reconcile wiring;
  this is deferred to W4 because the broker ops require the opaque-
  ID wire contract enforcement that W3fu2 H1 begins. The W3 wave
  exit gate continues to require panel sign-off on the actual
  behaviour, not the aspirational one.

- W3 follow-up round 2 finding-fix ( W3fu2 H6 ):
  W3fu1 work-review R2 docs-1..4 surfaced four reference-doc drifts
  the W3fu1 H1/H3 chain introduced without companion doc updates:

  1. **`host install --apply` / `--dry-run` are impossible.** H1
     defined `Install(HostInstallArgs)` with only `--json`/`--human`
     and returns the W9 stub (`not-yet-implemented`, exit 70) on
     every invocation. The pre-W3fu2 privileges schema however
     authorized non-existent `host install --apply` and
     `host install --dry-run` rows. H6 drops both rows from
     `packages/nixling-core/src/privileges.rs` and
     `nixos-modules/privileges-json.nix`, replaces them with a
     single plain `host install` row (mirroring the actual verb
     surface), and regenerates `docs/reference/schemas/v2/privileges.json`
     via `cargo xtask gen-schemas`.

  2. **`docs/reference/cli-contract.md` documented the pre-W3fu1
     `host prepare {--check | --apply | --destroy}` shape.** H1
     actually split that into separate `host prepare`,
     `host destroy`, `host doctor`, and `host install` verbs with
     `--dry-run`/`--apply` flags. H6 rewrites the `host prepare`
     section to the actual `--dry-run|--apply` shape, adds new
     reference sections for `host destroy`, `host doctor`, and
     `host install` matching the H1 clap declaration, and updates
     the docs anchors so the `#host-doctor` / `#host-install` links
     H1 emits resolve correctly.

  3. **`docs/reference/manifest-bundle.md` said `schemas/v1/` was
     the current bundle directory.** H3 bumped the baseline to
     `v2` (bundleVersion=2, schemaVersion=v2). H6 updates the
     prose to point at `schemas/v2/*.json` and notes the
     per-artifact `.md` prose still lives under `schemas/v1/`
     (additive over v1; v2 prose retitling is tracked separately
     and the `tests/host-json-drift-gate.sh` v2-`.md` check skips
     until those files land).

  4. **`docs/reference/daemon-api.md` Audit section docs the old
     pre-W3 single-file path `/var/lib/nixling/broker-audit.log`.**
     H1 introduced daily-rotated
     `/var/lib/nixling/audit/broker-<utc-date>.jsonl` with 14-day
     default retention. H6 updates the Audit section to document
     the daily-rotated path, retention policy, and append-only
     `O_APPEND` fd contract, and explicitly labels the legacy
     single-file path as no longer written or read by W3+ code.

  5. **`tests/privileges-matrix-completeness.sh` validated against
     the frozen v1 schema enum.** Adding `host install` to the
     v2 schema enum (and dropping the impossible `--apply`/`--dry-run`
     install rows) made the v1 schema reject the rendered v2 ops.
     H6 points the gate at `docs/reference/schemas/v2/privileges.json`
     so the validator tracks the current bundle baseline.

- W3 follow-up round 2 finding-fix ( W3fu2 H5 ):
  W3fu1 work-review R2 nixos-1 found that `nixos-modules/host-daemon.nix`
  still declared `systemd.services.nixlingd.serviceConfig.User =
  "root"` / `Group = "root"` even though W3 cgroup v2 delegation
  requires the long-lived daemon to be non-root so the broker can
  `fchown` the `nixling.slice` subtree to the daemon uid/gid. The
  `daemonExperimental` NixOS example evaluated to root/root, and
  every cgroup delegation path that fail-closes on `caller_uid == 0`
  would have refused at runtime.

  H5 changes the daemon unit to `User = "nixlingd"` / `Group =
  "nixlingd"` (the system user/group already declared by the module
  for `daemonConfig.daemonUser`/`daemonGroup`), adds `SupplementaryGroups
  = [ "nixling-launchers" ]` so the daemon's primary uid can `chgrp`
  `/run/nixling/public.sock` to the documented launcher discovery
  group, and reshapes the `systemd.tmpfiles.rules` so that nixlingd
  owns the directories it must write into (`/run/nixling`,
  `/run/nixling/locks`, `/run/nixling/state`, `/run/nixling/daemon.lock`)
  while `/etc/nixling` and `/var/lib/nixling` remain root-owned
  group-readable by nixlingd. The broker stays root and writes the
  audit log under `/var/lib/nixling/audit/` per the W3 contract.

- W3 follow-up round 2 finding-fix ( W3fu2 H7 ):
  W3fu1 work-review R2 rust-2 found that `cargo check
  --no-default-features` against `packages/nixling-priv-broker/`
  was build-broken: `src/runtime.rs` imports
  `nixling_ipc::broker_wire::BrokerCallerRole` /
  `BrokerRequestEnvelope` types and matches non-existent variant
  shapes (struct-form `ExportBrokerAudit`, missing `Hello`,
  unrecognized `BrokerResponse` variants) that the current wire
  contract no longer carries. Only `cargo check --features
  layer1-bootstrap` compiled clean, and `tests/rust-workspace-checks.sh`
  only exercises the `layer1-bootstrap` feature so the broken path
  never failed CI.

  H7 makes `layer1-bootstrap` the default broker feature in
  `packages/nixling-priv-broker/Cargo.toml` so unconfigured `cargo
  build`/`cargo check` invocations pick the supported runtime
  surface, and adds `tests/broker-default-features-build.sh` (wired
  into `tests/static.sh`'s W2 control-plane parallel group) that
  asserts (1) the default-features set still includes
  `layer1-bootstrap` and (2) `cargo check -p nixling-priv-broker`
  with default features compiles clean. This documents the W3 broker
  runtime as `layer1-bootstrap`-only until W4 wires the production
  runtime against the current `nixling_ipc::broker_wire` contract.

- W3 follow-up round 2 finding-fix ( W3fu2 H3 ):
  W3fu1 work-review R2 (test-1, test-2, software-1) flagged two test-
  quality regressions that let `tests/static.sh` report PASS for gates
  that did not actually exercise their stated invariant.

  1. **`tests/host-prepare-idempotency.sh` masked failures.** The
     gate ran `cargo test -p nixling-host --features fake-backends
     -- idempotency` and suffixed `|| true`, so a failing or panicking
     idempotency oracle still surfaced PASS at the shell layer. The
     follow-up "zero idempotency_* tests found" branch only `log`-ged
     instead of `fail`-ing, so an empty oracle set also silently passed.
     With no `idempotency_*` test functions in
     `packages/nixling-host/{src,tests}/`, the gate was PASS-by-default.

     H3 removes `|| true`, changes the zero-test branch to `fail`, and
     adds two real `idempotency_*` tests under
     `packages/nixling-host/src/nftables.rs` that exercise the production
     `hash_inet_nixling_table` drift digest:
     `idempotency_hash_table_stable` (apply→dry-run-empty: same input,
     same digest) and `idempotency_hash_volatile_stripped` (kernel-
     assigned `handle`/`index` fields do not perturb the canonical
     digest). Together these cover the broker's drift-detection
     contract and give the gate a real oracle that fails if either
     invariant regresses.

  2. **`tests/ch-net-handoff-canary.sh` was a grep test.** The previous
     shape ran a fake `cloud-hypervisor` shim in `$SCRATCH/bin/`, grep-
     ed its `--capabilities` output, grep-ed the golden envelope JSON
     for the error code, and grep-ed `packages/nixling-ipc/src/` for
     `CreateTapFd`. Nothing on the canary's execution path invoked
     `nixling_host::runner_shape::probe_ch_net_handoff_mode`, so the
     test would have passed unchanged if the probe function were
     deleted.

     H3 rewrites the canary to invoke `cargo test -p nixling-host --
     ch_help_with_fd_selects_tap_fd ch_help_without_fd_or_tap_fails_closed`,
     which runs the existing tests in
     `packages/nixling-host/src/runner_shape.rs` that call the real
     `probe_ch_net_handoff_mode` against representative `ch --help`
     excerpts and assert the documented `NetHandoffMode::TapFd` /
     `NetHandoffProbeError::NeitherSupported` outcomes. The golden
     envelope check and the `CreateTapFd` wire-declaration check are
     preserved as structural contract assertions.

  Standalone verification:
    $ bash tests/host-prepare-idempotency.sh   # 2 idempotency_* tests, PASS
    $ bash tests/ch-net-handoff-canary.sh      # real probe + goldens, PASS

- W3 follow-up round 1 finding-fix ( W3fu1 H11 ):
  Two coupled gate failures from the W3fu1 chain:

  1. `cargo xtask gen-daemon-api` failed with
     `could not parse type name` because W3fu1 H1 added the
     `opaque_id!` macro_rules pattern in
     `packages/nixling-ipc/src/types.rs` (a "security-1" hardening
     for opaque ID wire types), and the xtask's naive
     `pub struct`/`pub enum` line scanner does not skip
     `macro_rules!` bodies — it tried to extract the type name
     from the template line `pub struct $name(pub String);` whose
     `$name` placeholder is not a valid Rust identifier. H11 teaches
     `parse_rust_items` to detect `macro_rules!` blocks and advance
     past them via brace-depth tracking. The fix is conservative:
     the scanner still uses ad-hoc string matching (full syn-based
     parsing would be a panel-justifiable refactor for W3fu2 or W4).

  2. `tests/daemon-api-drift.sh` would have flagged the H1 wire-shape
     change anyway: H1 added the W3 opaque-ID fields
     (`bundle_*_intent_ref`, `scope_id`, `role_id`, `vm_id`,
     `tracing_span_id`) to every mutating broker request and
     reshuffled the enum variant order in `BrokerRequest` and
     `BrokerResponse`. With the xtask fix in (1), regenerating
     `docs/reference/daemon-api.md` via `cargo xtask gen-daemon-api`
     emits the post-H1 wire surface; H11 commits that regeneration.
     Per AGENTS.md "Existing code is canon", the Rust types in
     `packages/nixling-ipc/src/{lib,broker_wire,public_wire,types}.rs`
     are the canonical wire contract and the daemon-api.md table is
     generated.

- W3 follow-up round 1 finding-fix ( W3fu1 H10 ):
  `tests/manpage-completion-drift.sh` was failing the W3fu1 integration
  static gate because the W3fu1 H1 CLI surface additions
  (`host check`, `host prepare --dry-run`/`--apply`,
  `host destroy --dry-run`/`--apply`, `host doctor --read-only`,
  `host install`) were not reflected in the committed shell
  completions under `docs/reference/cli-shell/`. H10 regenerates
  `nixling.bash` / `nixling.fish` / `nixling.zsh` via
  `cargo xtask gen-cli-shell-artifacts` so the gate matches the
  current `clap` declaration. `nixling.1` (man page) is byte-stable
  through this regen — clap's roff target only emits when the
  top-level command help text changes, which it did not in H1.
  Per AGENTS.md "Existing code is canon", the Rust clap declaration
  is the canonical CLI surface; the docs/reference/cli-shell/
  artifacts are generated.

- W3 follow-up round 1 finding-fix ( W3fu1 H9 ):
  `tests/broker-enum-disposition.sh` was failing the W3fu1
  integration static gate because the W3fu1 H1 (rust-1) intentional
  behavior change — refusing W6 USBIP live device routing variants
  (`UsbipBind`, `UsbipUnbind`, `UsbipProxyReconcile`) with
  `BrokerError::UnknownOperation` instead of
  `BrokerError::Unimplemented` so the W3 broker audit shape records
  `unknown-operation` rather than `w3-pending-typed-wire` — was not
  reflected in `docs/reference/broker-w2-dispositions.md`. The doc
  still claimed all three variants were `stubbed-unimplemented`.
  H9 introduces a new disposition value `stubbed-unknown-operation`,
  teaches `tests/broker-enum-disposition.sh` to enforce it
  (segment must contain `BrokerError::UnknownOperation`), and
  updates the three Usbip rows in `broker-w2-dispositions.md` to
  carry the new disposition with the H1 rationale. The
  `UsbipBindFirewallRule` variant stays `stubbed-unimplemented`
  because the W3 wire-stable skeleton genuinely is deferred for
  scope s3 to wire in a later commit.

- W3 follow-up round 1 finding-fix ( W3fu1 H8 ):
  `tests/host-json-drift-gate.sh` was failing the W3fu1 integration
  static gate because its hardcoded definition-name list
  (`KernelModules`, `BridgePortFlags`, `FirewallCoexistence`,
  `IfnameMapping`) drifted from the H3-shipped v2 schema's actual
  `JsonSchema::schema_name()` outputs (`KernelModulesEntry`,
  `BridgePortFlags`, `IfNameMapping`). H8 aligns the gate to the
  schema: renames the three definitions that ARE wired into
  `HostJson`, drops `FirewallCoexistence` from the required-existence
  check (its `host_w3.rs` stub DTO is not yet a `HostJson` field at v2),
  and adds a Spec-corrections row to plan.md tracking the
  `FirewallCoexistencePolicy` wiring for W3fu2 / W4. The gate's
  `_expectedRejection.code` and baseline-fixture field-existence
  checks are unchanged; the `unknown-field-firewallcoexistence.json`
  fixture still asserts `wire-unknown-field` rejection because
  HostJson's root `additionalProperties: false` already refuses
  arbitrary top-level fields.

- W3 follow-up round 1 integration ( W3fu1 ): octopus-style sequential
  merge of the five W3fu1 hardening branches
  (H5 → H3 → H2 → H4 → H1) onto `main` plus the integrator
  finalization commit. The integrator commit wires the H4-shipped
  test scripts (`tests/host-json-drift-gate.sh`,
  `tests/l3-pin-consistency.sh`, `tests/host-prepare-idempotency.sh`,
  `tests/ch-net-handoff-canary.sh`) and the H3-shipped
  `tests/static-invariant-deny-unknown-fields-w3.sh` into
  `tests/static.sh`'s parallel-gate pool at the anchor points each
  H4/H3 script header specifies, regenerates the Rust lockfiles, and
  rolls per-H-scope CHANGELOG rows up into this single wave-rollup
  entry. Per-scope rows for each H still appear below.

- W3 follow-up round 1 docs cleanup ( W3fu1 H5 ):
  - `docs/how-to/host-prepare.md`: replaced the incorrect
    `--check|--apply|--destroy` synopsis at the top of the assembled
    how-to with the canonical W3 CLI verb contract from plan
    §"W3 CLI host verb scope table" (`host check`,
    `host prepare --dry-run`/`--apply`, `host destroy --dry-run`/
    `--apply`, `host doctor --read-only`, `host install` stub).
    Per-fragment dry-run/apply walkthroughs were already correct.
    Fixed two stale ADR references from
    `0011-w3-ipv6-off-sysctl-set-and-hash-ifname.md` to the
    integrator-renumbered `0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md`
    (one in `host-prepare.md`, one in
    `host-prepare.d/network.md`).
  - `docs/reference/support-matrix.md`: resolved the Arch tier
    inconsistency (preface said Tier 1-later, the s4 fragment said
    Tier 2; per plan §"Supported platform scope" Arch is Tier 2).
    Added the canonical platform-support table with per-row
    kernel/cgroup/nftables/NetworkManager/Cloud Hypervisor/minijail/
    glibc minima for NixOS, Ubuntu 24.04, Fedora 40+, and Arch
    rolling. Updated `docs/reference/compatibility.md` to match
    (Arch moved from Tier 1-later to Tier 2 in the at-a-glance
    table; added `host destroy --apply` column and the
    `--dry-run`/`--apply` flag split).
  - `docs/reference/privileges.md`: replaced the partial W3-only
    table with a complete enumeration of the current
    `nixling_ipc::broker_wire::BrokerRequest` enum. Added
    `Variant`/`Subject`/`Scope`/`Wave first delivered`/`Destructive`/
    `Secret access`/`Allowed groups`/`Audit`/`Default-for-unknown`
    columns. The W3-delivered rows cross-link to ADRs 0011-0014;
    W2-delivered rows (`ValidateBundle`, `ExportBrokerAudit`,
    `CreateOrReconcileUsersGroups`) and W4/W6/W8-deferred rows
    (`LaunchMinijailChild`/`SetupMountNamespace`/`PrepareStoreView`,
    `UsbipBind`/`UsbipUnbind`/`UsbipProxyReconcile`,
    `ReadSecretById`/`InjectSecretById`/`RotateSecretById`,
    `PauseBroker`/`ResumeBroker`) are now explicitly enumerated
    with their deferred wave so the page matches the wire enum.
    The canonical machine-readable source is noted as the JSON
    schema at `docs/reference/schemas/v2/privileges.json`.
  - `docs/reference/cgroup-delegation.md`: fixed the audit
    example so `authz_result` carries the launcher/admin/deny
    class (not the decision values) and `decision` carries the
    allowed/denied-refused/denied-unknown/errored verdict, per
    the common header in `docs/reference/privileges.md` §
    "Audit record schema (W3 baseline)". Added a cross-link
    pointing operators at that canonical definition.
  - `docs/explanation/host-prepare.md`: rewrote the
    `--check`/`--apply`/`--destroy` section to use the canonical
    W3 CLI verbs and added a "NetworkManager / systemd-networkd
    coexistence" conceptual section explaining (a) when the
    unmanaged drop-in is written (pre-create, before any
    `RTM_NEWLINK`), (b) detection-only handling of
    systemd-networkd hosts and the configured-unmanaged file
    requirement, (c) why coexistence fails closed (foreign
    managers can re-enable RA/autoconf/MTU mid-startup against
    the IPv6-off ordering invariant), and (d) what happens when
    no manager is present (`manager_detected: none`, broker
    proceeds). Updated the recovery runbook to use
    `host check`/`host prepare --apply`/`host destroy --apply`.
  - `docs/reference/naming-conventions.md`: added the
    deterministic Nix derivation function for the
    `nl-<hash>`/`nlv-<hash>` IfName scheme and a "Looking up the
    user-visible name from a derived IfName" subsection pointing
    operators at `nixling host check --json`'s
    `.host.ifnameMapping[]` and `nixling status <vm> --json`'s
    `.vm.ifnames[]`.
  - `AGENTS.md`: rewrote the "Control plane (W2+) — daemon +
    broker + CLI" row with the W3 dispatch surface, the new
    audit log path
    `/var/lib/nixling/audit/broker-<utc-date>.jsonl`
    (replacing the pre-W3 `/var/lib/nixling/broker-audit.log`
    path), the W4/W6/W8 deferred-variant disposition, and the
    `OpAuditRecord` shape. Added a "W3 cgroup slice naming +
    ownership-marker conventions" section under Critical
    subsystems covering the `nixling.slice` canonical path, the
    process-free VM layer invariant, the `comment "nixling
    managed: <id>"` nftables marker, the `# nixling-managed
    begin/end` block for `/etc/hosts` and the NM unmanaged
    drop-in, and the systemd-networkd detection-only posture.
    `NL_SKIP_WITH_ENTRA_ID=1` was already documented under
    Disk-hygiene knobs (W3 H4 integration); no change needed
    there.

  Docs-only round; no Rust source, no Nix emitters, no tests, no
  ADR content touched. ADRs 0011-0014 remain as authored by
  s1-s4.

- W3 integration follow-ups ( W3 H1 / H2 / H3 / H4 ):
  - **H1** — `packages/nixling-host/src/cgroup.rs` test code:
    `cargo clippy --workspace -- -D warnings` in the integration
    gate flagged two `clippy::cloned_ref_to_slice_refs` hits at
    lines 1025/1028 (`&[leaf.clone()]` → `std::slice::from_ref(&leaf)`).
    Test-only fix; `cgroup_kill_leaf_only` signature and the
    leaf-only-kill contract are unchanged.
  - **H2** — `tests/privileges-matrix-completeness.sh` +
    `packages/nixling-core/src/privileges.rs` +
    `nixos-modules/privileges-json.nix` +
    `docs/reference/schemas/v1/privileges.json`: register the
    bare `host prepare` op alongside `host check` (so the W3
    `host prepare` CLI synopsis tokenises against a real row) and
    narrow the gate's broker-enum regex to skip enum names
    ending in `Error`/`Err`/`Kind` (so the W3 s1-introduced
    `OpError`/`CgroupOpError`/`PidfdOpError` sub-error enums no
    longer false-positive as broker operations).
  - **H3** — `packages/nixling-priv-broker/Cargo.toml`: add a
    `[dev-dependencies]` override on `nixling-host` with
    `features = ["fake-backends"]` so the broker's in-tree
    `src/ops/cgroup.rs` tests can compile when the gate invokes
    `cargo test --features layer1-bootstrap` (NOT
    `--all-features`). Production builds carry no extra feature
    surface; the `[dependencies]` line is unchanged.
  - **H4** — `tests/static.sh` `NL_SKIP_WITH_ENTRA_ID=1` carve-out
    used during the W3 integration gate run. The
    `vicondoa/nixos-entra-id` flake input pins
    `libhimmelblau-0.8.18` + `kanidm-hsm-crypto-0.3.6`; both
    return crates.io 403 transiently/persistently and reproduce
    on the panel-suggested input bump
    (`16a961f` → `c62944d`; same crate pins). The in-band retry
    and the `nix flake update --update-input nixos-entra-id`
    escalations both failed, so the gate was run with
    `NL_SKIP_WITH_ENTRA_ID=1` per AGENTS.md § Disk hygiene
    contract. Re-evaluate when entra-id bumps past
    `libhimmelblau-0.8.18` / `kanidm-hsm-crypto-0.3.6`.

- W3 integration merge: octopus merge s1+s2+s3+s4+s5 + assembled docs + static.sh wiring ( W3 ):
  - Sequential `git merge --no-ff` of `w3/s5`, `w3/s1`, `w3/s2`,
    `w3/s3`, `w3/s4` onto the W3 integrator-prep baseline. Scope
    branches preserved with their original commits and authorship.
  - 4-way ADR 0011 collision resolved by panel-allocated renumber:
    s1 keeps 0011 (cgroup v2 delegation + pidfd handoff), s2 →
    0012 (IPv6-off sysctl set + hash-derived IfName + bridge-port
    defaults), s3 → 0013 (firewall coexistence policy matrix +
    `inet nixling` chain layout), s4 → 0014 (`kernel.modules_disabled=1`
    behavior + module probe order + CH net handoff selection +
    runner-shape preflight). `docs/adr/README.md` index now lists
    all four W3 ADRs in ascending order; per-file cross-references
    in scope-owned docs and the `packages/Cargo.toml` ADR comment
    re-pointed to the new numbers.
  - `packages/nixling-priv-broker/src/ops/mod.rs` reconciled as a
    4-way add/add combine with `// W3 sN begin/end` scope markers,
    retaining s1's `OpError` + `AuditDecision` types as the shared
    handler vocabulary.
  - `packages/nixling-priv-broker/src/lib.rs` exposes a single
    `pub mod ops;` (shared module; per-scope markers live inside
    `ops/mod.rs`).
  - `packages/Cargo.toml` combines s2's `rtnetlink = "0.14"` and
    s3's `sha2 = "0.10"` workspace declarations.
  - `packages/nixling-host/Cargo.toml` combines s1's `nix = "0.29"`
    safe-`fchown` dependency and s3's `sha2` consumer entry.
  - `packages/Cargo.lock` regenerated via `cargo generate-lockfile
    --offline` after all source merges;
    `packages/nixling-priv-broker/Cargo.lock` re-verified
    (unchanged after regenerate).
  - Assembled documents:
    - `docs/how-to/host-prepare.md` from
      `docs/how-to/host-prepare.d/{cgroup,network,firewall,modules-and-devices}.md`
      (one section per scope, in s1 → s2 → s3 → s4 order, with
      cross-references to the conceptual model, reference docs,
      and ADRs).
    - `docs/reference/support-matrix.md` from
      `docs/reference/support-matrix.d/s4-tier-modules.md` plus a
      Tier 0/1/1-later/2 preface per plan.md §"Supported
      platform scope".
    - `docs/explanation/host-prepare.md` (new) — conceptual model
      covering broker contract, cgroup delegation rationale,
      NM/networkd coexistence policy, ownership markers,
      `--check`/`--apply`/`--destroy` boundaries, tier behavior,
      mixed legacy/daemon operation, and the post-compromise
      recovery runbook.
    - `docs/reference/privileges.md` (new) — broker enum
      operation matrix including all W3 variants with audit /
      destructive / secret flags, audit-record schema, and
      cross-links to ADRs 0011–0014.
  - `SECURITY.md` extended with a W3 trust-boundary delta
    paragraph covering broker mutation surface, audit log
    posture, pause/resume runbook, and the explicit USBIP
    out-of-scope statement.
  - In-place baseline-doc updates:
    - `docs/reference/naming-conventions.md` — `nl-`/`nlv-`
      bridge/TAP ifname space, IFNAMSIZ enforcement, env/vm-name
      → ifname mapping exposure.
    - `docs/reference/compatibility.md` — Tier 0/1/1-later/2
      status table for the new W3 host verbs.
    - `docs/how-to/write-a-nixling-addon.md` — addon hook
      contract: declared modules / sysctls / NM entries / hosts
      entries / firewall extensions go through typed DTOs, never
      bypass the broker.
    - `CONTRIBUTING.md` — W3 Layer-1 gate command list, when to
      run L2 KVM tests, distro matrix expectations.
    - `docs/reference/cli-contract.md` — new `host prepare`
      section with the W3 `--check` / `--apply` / `--destroy`
      flag matrix and exit-code table.
    - `docs/reference/error-codes.md` — new "W3 host-prepare
      audit decision codes" section catalog (32 kebab-case codes
      across cgroup / pidfd / network / firewall / modules /
      runner-shape) mapped back to the auto-generated typed
      exit-code catalog.
  - `tests/static.sh` wires all 15 new W3 gates into the
    parallel-gate pool (cgroup oracle, pidfd handoff,
    host-prepare network, IPv6-off readback, ifname collision,
    path-safety violation, nft coexistence, nft foreign-rule
    preservation, USBIP firewall skeleton, kernel-module matrix,
    device-node matrix, ioctl negative, runner-shape preflight,
    minijail version check, multi-env daemon-backed). The per-
    example flake-check loop now performs one in-band retry for
    the `with-entra-id` example (transient crates.io 403 against
    `libhimmelblau-0.8.18` / `kanidm-hsm-crypto-0.3.6`) and
    honors `NL_SKIP_WITH_ENTRA_ID=1` as an explicit, panel-
    justifiable W3 carve-out for an external dependency outage.
    `AGENTS.md` § Disk hygiene contract documents the new knob.

- W3 s5 (mtu/mssClamp/east-west + daemon-backed multi-env example) ( W3 ):
  - `nixos-modules/options-vms.nix`, `processes-json.nix`, and
    `host-wrapper.nix` wire MTU / MSS-clamp / east-west toggles
    from v0.4.0 manifest fields through to the host bridge/TAP
    setup and to the daemon-mode processes.json emitter.
  - New `examples/multi-env` variant showing two isolated envs
    with one daemon-backed VM and one legacy-systemd VM
    coexisting on the same host.
  - `tests/multi-env-daemon-backed.sh` exercises the example
    flake's eval graph + the processes.json drift between the
    two modes.

- W3 s1 (cgroup v2 delegation + pidfd handoff) ( W3 ):
  - 8-step cgroup v2 delegation algorithm in
    `packages/nixling-host/src/cgroup.rs` (controllers preflight,
    cpuset propagation, ordered `cgroup.subtree_control`,
    leaf-only `cgroup.kill`).
  - `packages/nixling-priv-broker/src/ops/{cgroup,pidfd}.rs`
    typed handlers for `DelegateCgroupV2`, `OpenCgroupDir`, and
    pidfd handoff via `SCM_RIGHTS`.
  - `packages/nixlingd/src/supervisor/{mod,pidfd}.rs` enforces
    `PR_SET_CHILD_SUBREAPER`, the per-VM/per-role fd ownership
    table, and `pidfd_send_signal` + `waitid(P_PIDFD)` control.
  - `docs/adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md`,
    `docs/reference/cgroup-delegation.md`,
    `docs/how-to/host-prepare.d/cgroup.md`.
  - Gates: `tests/cgroup-delegation-oracle.sh`,
    `tests/pidfd-handoff.sh`,
    `packages/nixling-priv-broker/tests/pidfd_handoff_scm_rights.rs`.

- W3 s2 (bridge/TAP/NM/IPv6/IfName reconcile + state-dir + path-safety) ( W3 ):
  - 5-step IPv6-off ordering, FNV-1a + Crockford base32 hash-derived
    IfName scheme, bridge-port flag defaults per role,
    NetworkManager unmanaged config + `nmcli general reload conf`
    contract.
  - `packages/nixling-host/src/{bridge_port,ifname,netlink,routes}.rs`
    typed plans + fake netlink backend.
  - `packages/nixling-priv-broker/src/ops/{tap,nm,sysctl,route,hosts,state_dir,audit_op}.rs`
    plus a quarantined `sys.rs` for SCM_RIGHTS fd handling.
  - `docs/adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md`,
    `docs/how-to/host-prepare.d/network.md`.
  - Gates: `tests/host-prepare-network.sh`,
    `tests/ipv6-off-readback.sh`,
    `tests/ifname-collision.sh`,
    `tests/path-safety-violation-fs.sh`.

- W3 s3 (nftables coexistence + USBIP firewall skeleton) ( W3 ):
  - 4-chain `inet nixling` layout, 7-row detector → policy matrix
    for firewalld / ufw / docker / libvirt / iptables-nft /
    unknown / no-manager, USBIP carve-out ordering.
  - `packages/nixling-host/src/nftables.rs` typed `NftBatch` +
    `nft -f -` text renderer + sha256 readback (no `libnftnl`
    dependency per ADR 0013).
  - `packages/nixling-priv-broker/src/ops/{nft,usbip_firewall}.rs`.
  - `docs/adr/0013-w3-firewall-coexistence-policy.md`,
    `docs/reference/inet-nixling-chains.md`,
    `docs/how-to/host-prepare.d/firewall.md`.
  - Gates: `tests/nft-coexistence.sh`,
    `tests/nft-foreign-rule-preservation.sh`,
    `tests/usbip-firewall-skeleton.sh`.

- W3 s4 (kernel-module + device matrix + ioctl allowlist + runner-shape) ( W3 ):
  - 4-step kernel-module probe (`modules_disabled` →
    `/proc/modules` → `modules.builtin` → `ModprobeIfAllowed`),
    fail-closed posture under `kernel.modules_disabled=1`.
  - `packages/nixling-host/src/{modules,devices,ioctl_policy,runner_shape}.rs`
    typed module matrix, device-node matrix, role-derived ioctl
    allowlist, and CH `tap-fd`/`persistent-tap` runner-shape
    preflight.
  - `packages/nixling-priv-broker/src/ops/{device,modprobe}.rs`.
  - `docs/adr/0014-w3-modules-devices-runner-shape.md`,
    `docs/how-to/host-prepare.d/modules-and-devices.md`,
    `docs/reference/support-matrix.d/s4-tier-modules.md`,
    `tests/golden/runner-shape/parity-drift.json`.
  - Gates: `tests/kernel-module-matrix.sh`,
    `tests/device-node-matrix.sh`,
    `tests/ioctl-negative.sh`,
    `tests/runner-shape-preflight.sh`,
    `tests/minijail-version-check.sh`.

- W3 integrator API/contract prep commit ( W3 ):
  - New `packages/nixling-host` crate with disjoint host-prepare
    module stubs (`ifname`, `cgroup`, `netlink`, `nftables`,
    `routes`, `devices`, `modules`, `bridge_port`, `ioctl_policy`,
    `fake`). Each stub names its W3 scope owner (s1-s4) and carries
    a single TODO marker so the integrator prep workspace builds
    while parallel scope agents fill in the algorithms. Crate-level
    `#![forbid(unsafe_code)]`; depends only on `nixling-core`,
    `nixling-ipc`, and the workspace-pinned `rustix`.
  - New `nixling-ipc` `BrokerCapabilities` struct + `PROTOCOL_VERSION
    = 2` constant for the W3 wire-skew gate; new
    `BrokerRequest::UsbipBindFirewallRule` variant carrying the
    W3-only per-busid USBIP firewall-rule skeleton (live device
    routing — `UsbipBind`/`UsbipUnbind`/`UsbipProxyReconcile` — stays
    explicitly out of W3 scope).
  - New `nixling-core` modules `host_w3` (`IfNameMapping`,
    `BridgePortFlagsW3`, `TapRoleW3`, `KernelModuleEntry`,
    `ModuleRequirementW3`, `RouteIntent`, `SysctlIntent`,
    `HostsEntry`, `NmUnmanagedEntry`,
    `FirewallCoexistencePolicy`/`FirewallManager`/`CoexistencePolicy`)
    and `privileges_w3` (`W3BrokerOperation` enum +
    `W3OperationFlags` audit-mandate flags). All DTOs use
    `#[serde(deny_unknown_fields)]` per AGENTS.md "Manifest bundle"
    security-sensitive types policy.
  - `BROKER_OPERATION_AUTHZ` gains the `UsbipBindFirewallRule`
    row (`audit: yes`, `defaultForUnknown: deny`); the matching
    Nix-emitted privileges.json row lands in
    `nixos-modules/privileges-json.nix`.
  - `docs/reference/schemas/v1/{privileges,wire-protocol}.json`
    and `docs/reference/daemon-api.md` regenerated via
    `cargo xtask gen-schemas` + `gen-daemon-api` to include the
    new variant + privileges row. The bundle / host / processes
    artifact schemas are unchanged because the prep commit adds no
    new fields to existing artifacts — scope agents own the
    `bundleVersion`/`schemaVersion` bump per plan.md §"W3 schema/
    version bump rules" when they wire `host_w3` DTOs into
    `HostJson`.
  - `rustix = "0.38"` added to `[workspace.dependencies]` and
    consumed by `nixling-host`. `rtnetlink` and `nftnl` are
    intentionally deferred to the s2 (netlink/routes) and s3
    (nftables) scope commits per panel-justified
    "panel-justified alternative" clause in plan.md §"W3 file-
    ownership map" — adding them in the prep commit would pull
    transitive licenses + advisories into the cargo-deny gate
    before the scope that consumes them, with no offsetting
    benefit (the wire contract is fully encoded in nixling-ipc
    today).
  - `w2-rust-tests-golden-fragility-w3` follow-up: the
    `tests/golden/vms.json-91d69b0` and
    `tests/golden/manifest_v04/baseline-vms.json` reads in
    `packages/nixling-core/src/manifest_v04.rs` and
    `packages/nixling-core/fuzz/src/bin/core.rs` switch to
    `include_str!` so the rust-tests Nix sandbox never reaches
    outside its `src` set. `flake.nix` composes the rust-tests
    sandbox `src` from `packages/` + `tests/golden/` via
    `pkgs.runCommand` so `include_str!` resolves at compile time.
  - `nixling-host = { path = "../nixling-host" }` declared by
    `nixlingd` and `nixling-priv-broker` so scope commits can add
    `use nixling_host::*` lines without retouching either
    Cargo.toml.
  - `AGENTS.md` worktree workflow section gains a paragraph
    describing the W3 integrator-prep-first pattern (this commit)
    + the canonical `( W3 )` / `( W3fu<M> )` / `( W3fu<M> H<N> )`
    trailing-tag forms reused from W2fu4 H10/H18.

- W2fu4 static-gate hardening round (between R1/R2):
  - H1 — `_NL_SMOKE_FALLBACK` always provisioned at lib.sh source;
    `_nl_smoke_cache_dir` self-heals if `NL_STATIC_CACHE` dir
    vanishes. Fixes the `set -u` crash that surfaced at the W1
    bundle/schema gate when the smoke cache fell through.
  - H2 — `tests/preflight-disk-space.sh` now runs AFTER the orphan
    reapers (which reclaim) but BEFORE the shared rust-toolchain
    `nix shell` bootstrap (which consumes). Closes the
    fail-closed-bypass gap flagged by the W2 R2 test reviewer.
  - H3 — `reap_known_static_orphans` skips its own bash's
    `.nl-cleanups.<pid>` basename so the active process's
    bookkeeping isn't torched at startup.
  - H4 — typed daemon errors (`packages/nixlingd/src/typed_error.rs`)
    no longer leak host filesystem paths into client envelopes.
    Six variants redacted (`InternalAlreadyRunning`,
    `InternalBrokerUnavailable`, `InternalConfig`, `InternalIo`,
    `InternalLockParentInvalid`, `WireIfNameInvalid`). New
    `log_raw_detail()` emits full unredacted context via
    `tracing::error!`/`tracing::warn!` (operator-only). 8 unit
    tests assert the public `message` doesn't match a path-like
    regex. Closes ADR 0010 redaction requirement and the W2 R2
    security reviewer finding.
  - H5 — `flake.checks.${system}.rust-deny` now runs real
    `cargo deny check bans licenses sources` against both
    `packages/Cargo.toml` and
    `packages/nixling-priv-broker/Cargo.toml`, offline via
    `importCargoLock`. `rust-audit` runs real `cargo audit
    --no-fetch` against both lockfiles using a pinned RustSec
    advisory DB (commit `831c50f4`). License allow-list extended
    with MPL-2.0, OpenSSL, Unlicense. No real advisories or
    violations found at this commit. Closes the W2 R2 security
    reviewer "file-presence stubs" finding.
  - H6 — opt-in deep-GC of OLD NixOS generations after the gate
    via `NL_POST_GATE_DEEP_GC=1` (+ `NL_POST_GATE_DEEP_GC_SUDO=1`
    for system profile, using `sudo -n` and never prompting).
    AGENTS.md "Disk hygiene contract" expanded with the
    operator runbook (`sudo nix-collect-garbage
    --delete-older-than 7d`).
  - H7 — `static.sh` emits the failing test's `tail -40 >&2`
    diagnostic BEFORE calling `fail`, so `set -euo pipefail`
    doesn't eat the operator-visible output. Three call sites:
    `nl_static_run_smoke_eval`, assertions-eval dispatch,
    observability-eval dispatch.
  - H8 — parallel-test timing artifacts move to
    `${TMPDIR:-/tmp}/nixling-static-timing.$$/`. They previously
    lived in `$ROOT/.static-timing.{log,status,raw}.*`, where
    `builtins.getFlake (toString $ROOT)` saw them appear and
    disappear mid-source-copy and failed with
    `error: path '//<flake-source>/.static-timing.status.<n>' does not exist`.
    Surface flagged by the W2 R2 product + networking reviewers
    at cli-legacy-bash-dispatch / cli-json.
  - H9 — per-process bookkeeping (`.nl-cleanups.<PID>`,
    `.nl-scratch-registry`) moves to
    `${NL_BOOKKEEPING_DIR:-${TMPDIR:-/tmp}/nixling-bookkeeping}`.
    Closes the second half of the same race against
    flake-source capture. Orphan reaper now also prunes
    dead-PID cleanups files from the bookkeeping dir.
  - H10 — AGENTS.md "Commit conventions" now explicitly codifies
    the canonical trailing-tag form (`( W<N>fu<M> H<N> )` or
    `( W<N>fu<M> )`) with no leading-tag and no merge-shape
    suffix. Historical W0/W1/W2 commits stay as-is (rewriting
    142 commits via `git filter-repo --message-callback`
    is high blast radius for low audit value). Forward-only
    commitment: W2fu4+ commits all follow the canonical form.
    Closes the W2 R2 software reviewer tag-discipline finding.
  - H11 — `tests/static.sh` now stops the sccache server at gate
    exit inside the locked region, BEFORE the flock-wrap exits.
    Sccache previously inherited fd 3 (the gate's flock fd)
    from a cargo invocation, daemonised itself, and held the
    lock open across runs — subsequent `bash tests/static.sh`
    invocations then blocked indefinitely on flock with no
    visible holder. A startup safety net also scans `/proc`
    for any sccache holding the lock file and reaps it before
    flock acquire. Bypass with `NL_POST_GATE_STOP_SCCACHE=0`
    if you want to keep the in-memory cache warm at the cost
    of needing to stop sccache manually.
  - H12 — split the W2fu4 H1+H2+H3 batched commit into three
    atomic commits via `git rebase -i e0139e0~1` + per-hunk
    staging with `git add -p`. The W2fu4 chain on `main` is
    now 15 atomic commits, one logical change each. Closes
    the W2 R3 software reviewer atomicity finding.
  - H13 — AGENTS.md "canonical tag form" placeholder changed
    from `H<N>` to `<S><N>` with explicit C/H/M/L severity
    spec (W2 R3 software finding). CHANGELOG.md filled in
    W2fu4 H4/H5/H10/H11 entries that the W2 R3 docs reviewer
    flagged as missing.
  - H14 — `tests/lib.sh` `NL_LOG` defaults to
    `${TMPDIR:-/tmp}/nixling-test.$$.log` instead of
    `$FLAKE/.nixling-test.log`. Closes the W2 R3 test reviewer
    HIGH finding about the churning ignored file inside the
    flake source tree during flake-eval gates.
  - H15 — `tests/static-timing.sh` `REPORT_LOG` and `RAW_LOG`
    default to `${TMPDIR:-/tmp}/nixling-static-timing-report.$$/`
    instead of `$ROOT/.static-timing.{log,raw}`. Closes the
    W2 R3 test reviewer MEDIUM finding.
  - H16 — `flake.nix` `rustWorkspace` derivation now sets
    `RUSTC_WRAPPER=""` so sandbox `cargo build` doesn't try
    to invoke sccache (which isn't on the sandbox `PATH`).
    Operators running cargo OUTSIDE the sandbox (worktrees,
    dev shells, the static gate's own rust-workspace-checks.sh)
    still get the sccache speedup from the unchanged
    `packages/.cargo/config.toml`. Fixes
    `nix build .#checks.x86_64-linux.rust-build` and
    `.rust-clippy`. Closes the W2 R3 rust reviewer HIGH
    finding. (Pre-existing `.rust-tests` golden-file-path
    fragility — the test reads
    `tests/golden/vms.json-91d69b0` outside the rust-tests
    sandbox src — is flagged for a separate follow-up rather
    than papered over; it predates W2fu4.)

  Net wall-clock impact: gate still ~25-35 min on this host;
  H8/H9 do not measurably regress timing. Disk hygiene is the
  real win — every green run now ends with disk back at
  baseline (4%) after the post-gate `nix store gc`.

- W2fu2 disk-hygiene + test-infra speedup round: shared `sccache`
  (`RUSTC_WRAPPER = "sccache"`) wired into every Cargo workspace
  (`packages/`, `packages/nixling-priv-broker/`,
  `packages/nixling-core/fuzz/`) so compiled rustc outputs
  deduplicate across worktrees by content hash. Each worktree
  keeps its own `target/` (isolation preserved); cross-worktree
  dedupe lives at `$SCCACHE_DIR=$HOME/.cache/nixling-sccache`
  (capped at 10 GiB). Measured: cold build 19s; clean-target build
  with primed sccache 12.5s; 100% cache-hit rate on the deps tier
  across worktrees building the same source.
- W2fu2 test-scratch hygiene: new `nl_mktemp` helper in
  `tests/lib.sh` backed by a registry file so leaked scratch dirs
  (from SIGKILL'd test runs) get reaped at the next `tests/static
  .sh` startup. Every `tests/*.sh` that previously called raw
  `mktemp -d -p "$ROOT"` is converted (~14 scripts).
  `tests/preflight-disk-space.sh` runs FIRST in `static.sh` and
  fails closed below `NL_MIN_DISK_GIB=10`. Defensive orphan
  sweep at static.sh startup scans the well-known orphan glob
  patterns and reaps any matching dir.
- W2fu2 `static.sh` flock fd-leak fix: replaced the in-shell
  `exec {fd}>file; flock -x $fd` pattern with `exec flock -x
  "$lock" "$0" --internal-locked "$@"`. The lock fd is now owned
  by `flock(1)` itself; the inner bash has no inherited lock fd
  to leak into spawned children. Previous pattern caused multi-
  minute deadlocks when broker test daemons inherited the lock fd
  via fork and outlived their spawning shell.
- W2fu2 AGENTS.md + CONTRIBUTING.md disk-hygiene contract
  section.
- W2 control-plane skeleton: introduce the non-root `nixlingd`
  daemon, the minimal privileged `nixling-priv-broker`, the
  Rust-native `nixling` CLI shim, and the wire protocol crate
  `nixling-ipc` (`SOCK_SEQPACKET` Unix-domain sockets at
  `/run/nixling/{public,priv}.sock`, 4-byte LE length-prefix + JSON
  body, 1 MiB max frame, `Hello`/`HelloOk`/`HelloRejected`
  handshake with `SemverRange` + feature negotiation, frozen
  per-VM lifecycle enum). The broker enum exactly mirrors
  `OperationAuthz.operation` from `docs/reference/schemas/v1/
  privileges.json`; only `Hello`, `ValidateBundle`, and
  `ExportBrokerAudit` are callable-read-only in W2, every mutation
  op (`Apply*`, `Open*`, `CreateTapFd`, `DelegateCgroupV2`,
  `LaunchMinijailChild`, `Usbip*`, secret rotation, etc.) is
  `stubbed-unimplemented` and audit-logged. `ValidateBundle` is
  the only validation entry point (no second parser in the
  broker). `audit` CLI reads via daemon → broker `ExportBrokerAudit`
  with `SO_PEERCRED`-gated authz against the new
  `nixling.site.adminUsers` module option; broker audit log lives
  at `/var/lib/nixling/broker-audit.log` (root:nixlingd 0640,
  `O_APPEND` broker-only write). Public socket auth uses
  `SO_PEERCRED` + `getgrouplist(3)` supplementary-group lookup
  against `nixling.site.launcherUsers` (wheel membership alone
  does NOT grant access). Daemon single-instance + per-VM locks
  use Linux `OFD` locks via `fcntl(F_OFD_SETLK)` at
  `/run/nixling/daemon.lock` and `/run/nixling/locks/<vm>.lock`.
  Wire types route every Linux interface/bridge/TAP/port name
  through the W1 `IfName` newtype (no raw `String`). Rust-native
  CLI implements 5 read-only commands (`list`, `status`, `audit`,
  `host check`, `auth status`) with both human-readable and
  stable `--json` output (schemas at
  `docs/reference/cli-output/*.schema.json`, generated by
  `cargo xtask gen-cli-schemas`, drift-gated). Typed errors at
  `nixling_core::error::Error` carry stable `Kind` discriminant,
  stable exit code (10..=99 reserved for operator-visible),
  redacted message template, remediation hint, docs anchor, and
  `owningCommand` field; `docs/reference/error-codes.md`
  generated by `cargo xtask gen-error-codes`. Manifest fuzz
  harness uses `bolero` (stable-friendly) under
  `packages/nixling-core/fuzz/` with adversarial seed corpus for
  `manifest_v04` + `bundle` + `host` + `privileges`; bounded
  `tests/manifest-fuzz-bounded.sh` runs 10000 iterations per CI
  pass (no unbounded fuzzing). `nixling-core::manifest_v04` is
  the canonical v0.4.0 manifest validator with round-trip-gated
  fixture asserting preservation of `mtu` / `mssClamp` /
  `lan.allowEastWest` / `effectiveEastWest` semantics. Daemon
  module exposed by `nixling.daemonExperimental.enable` (default
  `false` per ADR 0007); legacy bash CLI dispatch through the
  Rust shim preserves every v0.4.0 flag, exit code, and output
  format (gated by `tests/cli-legacy-bash-dispatch.sh`). New
  docs: `cli-contract.md` (rewritten with subcommand × flag ×
  exit-code coverage + bash↔Rust dispatch capability table),
  `daemon-api.md` (schema-style with `gen-daemon-api` xtask drift
  gate against `nixling-ipc`), `error-codes.md`, per-command
  prose at `docs/reference/cli-output/*.md`,
  `docs/explanation/{state-lock,daemon-experimental}.md`, ADR
  0010 (wire protocol + typed errors policy). 24 new Layer-1
  gates wired into `tests/static.sh` (broker enum disposition,
  broker validate-bundle, broker socket ACL, broker SCM_RIGHTS
  fd lifecycle, broker export-audit, daemon socket ACL, daemon
  state-lock, daemon version-negotiation, CLI rust-native × 5,
  CLI JSON drift vs v0.4.0 bash baselines, CLI legacy-bash
  dispatch, error-codes drift, manifest fuzz bounded,
  manifest-v04 round-trip, cli-contract coverage, daemon-api
  drift, rust dependency-direction). All examples
  (graphics-workstation, minimal, multi-env, with-entra-id,
  with-observability) and `templates/default/` continue to
  flake-check green; daemon module defaults off so existing
  consumers see no behavioral change.
- W1fu5 (W1 work-review round-3 contention findings, software +
  virt): the Layer-1 static gate no longer suffers from compounded
  nix-daemon contention when multiple `bash tests/static.sh`
  invocations run against the same worktree. `tests/lib.sh` adds
  `nl_smoke_vms_json` and `nl_smoke_bundle_privileges_json`
  helpers backed by a per-run cache directory at
  `$NL_STATIC_CACHE` (set by `tests/static.sh`) or a per-shell
  fallback created at lib.sh source time so command-substitution
  callers don't race the EXIT-trap cleanup. The three rendered-
  manifest gates (`tests/static-invariant-world-readable-leak.sh`,
  `tests/static-invariant-opaque-key-ids.sh`,
  `tests/privileges-matrix-completeness.sh`) now read from the
  cache. `tests/static.sh` acquires an exclusive `flock` on
  `$ROOT/.static-sh.lock` so concurrent invocations against the
  same worktree serialize on the daemon; bypass with
  `NL_STATIC_NO_LOCK=1` for isolated CI. Net cost reduction:
  opaque-key-ids ~5s → <1s, privileges-matrix-completeness ~10s →
  ~6s on warm cache.
- Round 4 of W1 work-review fixes (W1fu4): closes the four W1
  Round-2 reviewer holds. (a) Host KVM module rows are no longer
  emitted as unconditional `required`: `kvm_intel` and `kvm_amd`
  are now `requirement: "alternatives"` with a structured
  `gate: "host-cpu-vendor=<intel|amd>"` so single-vendor hosts
  satisfy exactly one row (Rust `ModuleRequirement` enum gains
  `Alternatives`; schemas regenerated). (b) The legacy generic
  `DelegateCgroup` operation is removed from
  `packages/nixling-core/src/privileges.rs`, the regenerated
  `docs/reference/schemas/v1/privileges.json` enum, and the
  `nixos-modules/privileges-json.nix` emitter row; only
  `DelegateCgroupV2` remains, matching the cgroup-v2-only
  contract in ADR 0003. (c) `nixos-modules/closures-json.nix` and
  `nixos-modules/minijail-profiles.nix` no longer strip Nix
  derivation string context on their `environment.etc.*.source`
  paths, so the resulting `/etc/nixling/closures/<vm>.json` and
  `/etc/nixling/minijail-profiles/*.json` are realized into the
  system closure instead of dangling symlinks. (d) The
  `tests/fixtures/deny-unknown/host-{valid,invalid}.json`
  fixtures now mirror the canonical six `nl_*` chain layout at
  priorities `±300` (W1fu3 standardized the emitter, the
  fixtures lagged). The three W1fu3 integrator commits
  (`d10f112`, `5122b49`, `1f714ee`) were retagged via
  `git rebase --rebase-merges` to carry the trailing `(W1fu3)`
  tag, matching the `(W1)`/`(W0fu1)` convention from prior waves.
- Round 3 of W1 work-review fixes (W1fu3): rewrite the six
  `nixos-modules/*.nix` bundle emitters (`bundle.nix`,
  `host-json.nix`, `processes-json.nix`, `privileges-json.nix`,
  `closures-json.nix`, `minijail-profiles.nix`) so each one
  produces JSON that conforms byte-for-byte to its committed
  `docs/reference/schemas/v1/*.json`. Drop the non-schema `hashes`
  substructure from `bundle.json` and emit real `closurePaths` per
  VM in `closures/<vm>.json`. Add `usbipBusidLocks` per env on
  `host.json` plus matching daemon-owned per-busid exclusivity
  rows in `privileges.json` (closes the W1 USBIP daemon-owned
  exclusivity contract; the `host.json` schema gains a typed
  `UsbipBusidLock`/`UsbipLockOwner`/`UsbipLockScope` triple).
  Rename `host.json` nftables chains to the documented six-chain
  ADR 0005 layout (`nl_ingress_accept`, `nl_forward_accept`,
  `nl_egress_accept` at priority `-300`; `nl_ingress_drop`,
  `nl_forward_drop`, `nl_egress_drop` at priority `+300`) with
  explicit `family` and `table` fields. Replace `DelegateCgroup`
  with `DelegateCgroupV2` and mirror every Rust privilege matrix
  row into `privileges.json` (every `OperationAuthz.operation`
  enum variant now has a `publicOperations`/`brokerOperations`
  row). Flatten `host.json.kernelModules` into the schema's array
  shape and add Intel/AMD/Nvidia GPU module rows. Add the missing
  `video → cloud-hypervisor` edge in the graphics-VM process DAG.
  Harden five Layer-1 gates: `tests/vms-json-parity.sh` now fails
  closed on render failure and any byte drift (only an explicit
  documented `manifestVersion` bump permits a new baseline);
  `tests/privileges-matrix-completeness.sh` now renders the smoke
  `privilegesJson.jsonText`, validates it against
  `docs/reference/schemas/v1/privileges.json`, and compares
  declared CLI+broker operations against the rendered
  `publicOperations`/`brokerOperations` rows (LC_ALL=C-normalized
  sort before `comm`); `tests/static-invariant-world-readable-leak.sh`
  and `tests/static-invariant-opaque-key-ids.sh` now fail closed
  when smoke vms.json render fails; and
  `tests/static-invariant-deny-unknown-fields.sh` now also covers
  `bundle.json`, `host.json`, and `closures.json` with positive +
  negative fixtures under `tests/fixtures/deny-unknown/`.
- Land the W1 host-neutral manifest bundle contract beside the
  existing `vms.json` (preserved byte-identical at
  `manifestVersion = 2`). New private artifacts at root:`nixlingd`
  mode `0640`: `bundle.json`, `host.json`, `processes.json`,
  `privileges.json`, `closures/<vm>.json`, `minijail-profile.json`.
  `packages/nixling-core` (DTOs) is the canonical source — JSON
  Schemas are committed under `docs/reference/schemas/v1/` and
  generated deterministically by `cargo xtask gen-schemas`. Six
  new emitters under `nixos-modules/` produce the artifacts. The
  W1 test gate adds `tests/bundle-drift.sh`,
  `tests/vms-json-parity.sh` (compares against v0.4.0 baseline at
  commit `91d69b0`), `tests/privileges-matrix-completeness.sh`,
  and six `tests/static-invariant-*.sh` scripts that fail closed
  on uid 0 long-lived profiles missing ADR carve-outs, broad caps,
  undocumented writable paths, world-readable sensitive bundle
  fields, missing `deny_unknown_fields`, and path-bearing
  secret/key fields. New docs: `docs/reference/manifest-bundle.md`,
  per-artifact schema md files under `docs/reference/schemas/v1/`,
  an updated `docs/reference/manifest-schema.md` note about bundle
  coexistence, and an `inet nixling` hook-priority addendum on
  ADR 0005.
- Land the eight W0b portability ADRs anchoring the daemon and
  broker work, plus the hermetic infrastructure that lets future
  waves continue without live KVM access. ADR 0001 fixes the
  systemd-free orchestration boundary; ADR 0002 defines the
  non-root `nixlingd` plus a minimal `nixling-priv-broker` with an
  append-only root-owned audit log; ADR 0003 pins the Nix-built
  minijail and the typed sandbox interface; ADR 0004 picks the
  "generate nixling-owned Cloud Hypervisor argv" runner shape with
  `microvm.declaredRunner` as parity oracle and reserves future
  `nixling-priv-broker`, `nixling-sandbox`, `nixling-supervisor`,
  `nixling-ch-api`, `nixling-host` crate names without W0b stubs;
  ADR 0005 pins the network/firewall/TAP model (validated `IfName`
  newtypes, broker-opened TAP/vhost fds via `SCM_RIGHTS`, named
  `inet nixling` nftables table with explicit firewalld/ufw
  refusal, `br_netfilter` sysctl policy, IPv6-off sysctls,
  daemon-owned USBIP); ADR 0006 introduces `bundleVersion` +
  per-artifact `schemaVersion` while preserving the v0.4.0
  `vms.json` `manifestVersion = 2`; ADR 0007 frames the
  `legacy-systemd` / `daemon-experimental` / `daemon-default`
  migration modes; ADR 0008 publishes the Tier 0/1/2 platform
  matrix and rejected-target list with the kernel 6.6 floor.
  Companion deliverables: `docs/reference/runner-shape-audit.md`
  documenting `declaredRunner` and the Cloud Hypervisor argv
  inventory; committed runner-shape golden fixtures under
  `tests/golden/runner-shape/` and the
  `tests/runner-shape-snapshot.sh` hermetic drift gate; the
  `harness/ubuntu/` non-NixOS skeleton with a host-check stub
  script, expected JSON, runner helper, Nix derivation, and
  `tests/harness-ubuntu-eval.sh` Layer-1 gate; the
  `checks.<system>.harness-ubuntu-skeleton` flake output on both
  supported systems; and `docs/explanation/design.md` plus
  `SECURITY.md` threat-model deltas covering the new public socket
  boundary, private broker boundary, and minijail sandbox scope
  (full rewrite lands in W2).
- Bootstrap a `packages/` Cargo workspace (members
  `nixling-core`, `nixling-ipc`, `xtask`, `nixling`, `nixlingd`)
  with pinned `packages/rust-toolchain.toml`, workspace-level
  `unsafe_code = "forbid"`, `packages/deny.toml`, and a committed
  `packages/Cargo.lock`. The `nixling` and `nixlingd` binaries are
  W0a version-stub binaries — they print metadata and exit; they
  do NOT bind sockets, claim authz scopes, or create files under
  `/run/nixling` or `/var/lib/nixling`. The v0.4.0 bash CLI
  remains the only user-facing nixling entry point until W2.
  Flake exposes Rust crates only via
  `checks.<system>.rust-{build,tests,clippy,deny,audit}` for
  `x86_64-linux` and `aarch64-linux`. `tests/static.sh` gates the
  workspace via `tests/rust-workspace-checks.sh` (auto-bootstraps
  toolchain through `nix shell` when `cargo` is absent),
  `tests/layer1-self-inventory.sh` (fails closed if any Layer-1
  `tests/*.sh` stops being invoked), and `tests/stub-no-socket.sh`
  (asserts the stub binaries leave no host artifacts).
  `templates/default/` joins the per-example flake-check loop.
  New `docs/adr/` tree opens with `docs/adr/README.md`,
  `0000-repository-layout-and-rust-bootstrap.md`, and
  `0009-rust-toolchain-msrv-and-supply-chain.md`.
  `CONTRIBUTING.md` gains a "Rust workspace checks" subsection
  under "Running quality gates" with a stable
  `rust-workspace-checks` anchor.

### Fixed

- **swtpm stale session cleanup on restart.**
  `nixling-<vm>-swtpm.service` now runs a pre-start flush that boots a
  temporary swtpm against the existing state directory, sends
  `swtpm_ioctl -i` to reset volatile TPM state, then shuts down
  cleanly before the main daemon starts. This prevents stale auth
  sessions from accumulating across repeated `nixling down` /
  `nixling up` cycles and exhausting swtpm's 64-slot session table.
- **Audit strict mode already skips stopped workload VMs.** Documented
  the existing `bridge_isolated_workload` stopped-VM skip (v0.1.4),
  the graphics-VM running-unit fix (`nixling-<vm>-gpu.service`,
  v0.1.6), and the fact that `sidecars_per_vm` only asserts users for
  active sidecars, so `nixling-audit-check.service` can keep using
  `nixling audit --strict` with the default `autostart = false`.
  Closes #24.
- #23: Repair TPM ownership migration without touching running VMs.
- #5: Gate host-side USBIP materialization. Per-env backend/proxy
  services, proxy sockets, firewall rules, and strict-audit socket
  checks now appear only for envs that actually have an enabled VM with
  `usbip.yubikey = true` while `site.yubikey.enable = true`.
- `boot.kernelModules` no longer loads `usbip-host` unconditionally
  when `site.yubikey.enable = true`; it now gates the module on at
  least one enabled VM setting `usbip.yubikey = true`.
### Added

- readOnly+default+config trio static lint in `tests/static.sh` (#6).

### Changed

- Per-example flake checks now pass `--no-write-lock-file` so the
  static gate stays read-only while validating each example flake
  (#18).
- Rename `pkgs/spectrum-ch/AGENTS.md` to `pkgs/spectrum-ch/MAINTAINING.md`
  so the nested package-maintainer workflow no longer looks like a
  second AI-agent operating manual.
### Added

- Added `CONTRIBUTING.md`, documenting contributor workflow, worktree
  usage, validation steps, and commit conventions.
- Added `docs/reference/naming-conventions.md`, documenting the
  canonical host-visible naming glossary for units, bridges, and taps.
- Added `docs/reference/compatibility.md`, documenting the release-to-
  lock compatibility matrix and the supported `nixpkgs` policy.
- Added `docs/how-to/write-a-nixling-addon.md`, documenting the
  sibling-flake addon seam, the consumer-side `nixpkgs` follow rule,
  and the minimal eval-only test pattern.

### Changed

- Updated Mermaid diagrams in `docs/explanation/design.md` so the
  architecture walkthrough renders from the checked-in source.
- Added the DHCP anti-spoofing / bridge-isolation caveat to
  `docs/explanation/design.md`.
- Expanded the Secure Boot section in `docs/explanation/design.md`.
- Replaced the ASCII architecture, bridge-topology, and state-dir
  diagrams in `docs/explanation/design.md` with Mermaid blocks,
  while keeping the original ASCII art in `<details>` blocks for
  terminal viewers.

### Added

- `nixling.envs.<env>.mtu` to override an env's bridge, tap, and guest
  NIC MTU when the host rides a tunneled or VPN uplink.
- `nixling.envs.<env>.mssClamp` to clamp forwarded TCP MSS in the
  env's net VM to the routed path MTU.
- `nixling.envs.<env>.lan.allowEastWest` to opt workload VMs in the
  same env into peer traffic instead of the default isolated LAN.
  Enabling it now also requires `nixling.site.allowUnsafeEastWest = true`
  as an explicit acknowledgement that peer-guest traffic is outside the
  default isolation threat model.
- **CLI `--json` for `nixling list`, `nixling status <vm>`, and
  `nixling keys list`.** The bash CLI now emits structured jq-built
  JSON for VM inventory, per-VM service status, and framework-managed
  key inventory; explicit `--json` also overrides the audit command's
  TTY auto-human mode.
- Reserve `nixling.site.tmpDir` (default `/var/lib/nixling/tmp`) as a
  boot-cleaned ephemeral state root. Components SHOULD place transient,
  reboot-safe per-VM state under `<tmpDir>/<vm>/`.
- `nixling.site.stateDir` and `nixling.store.stateDir` now fail eval
  when overridden away from their default paths until the remaining
  host-side state-root threading lands.
- Add `.github/workflows/eval-with-entra-id.yml` so CI eval-checks
  `examples/with-entra-id/` with `nix flake check --no-build
  --all-systems --no-write-lock-file` without adding a new root-flake
  input coupling.
- Add per-VM `nixling.vms.<vm>.audit.*` with guest-side `auditd`
  forwarding into the existing observability pipeline over the
  workload VM's Alloy → vsock → Loki path. Enabling
  `audit.enable = true` now imports a curated ruleset in the guest,
  forwards auditd events into journald via `audisp-syslog`, and asserts
  that `observability.enable = true` is set on the same VM. The default
  curated ruleset excludes syscall-heavy rules such as `execve`/`connect`;
  opt into them explicitly for short-lived, high-sensitivity command
  auditing.

### Security

- **Broker CapabilityBoundingSet narrowed to canonical 8 caps** (CAP_SYS_PTRACE removed) ( P0 )
- **Bundle missing bundleHash on schemaVersion>=2 is now BundleTampered** (was warning) ( P0fu2 H4 )

## [0.3.0] - 2026-05-24

Minor release adding **hardware-accelerated H264 video decode** for
RDP sessions inside graphics VMs. A new virtio-media pipeline
offloads H264 decode from guest CPU to host NVDEC hardware via a
multi-component stack: guest ffmpeg h264_v4l2m2m → /dev/video0 →
chromeos/virtio-media kernel driver (device ID 48) → Cloud
Hypervisor `--vhost-user-media` → crosvm vhost-user video-decoder →
VA-API → nvidia-vaapi-driver → NVDEC. The pipeline activates
automatically when the RDP server negotiates AVC420/AVC444 codec;
ClearCodec sessions fall back to software decode transparently.

### Added

- **Dedicated CH `--vhost-user-media` device type**
  (`0003-vhost-user-media-device.patch`, 1104 lines across 10 CH
  source files). Modeled on the GPU device's VirtioDevice
  implementation with BackendReqHandler for shmem_map/shmem_unmap,
  memfd-backed 256 MB SHM PCI BAR, read_config proxying, and a
  vring_bases fix that forces `SET_VRING_BASE(0)` on initial
  activation — working around a CH bug where it reads `avail_idx`
  from guest memory, skipping buffers the driver pre-queued before
  `DRIVER_OK`.
- **Crosvm vhost-user video-decoder backend**
  (`pkgs/vhost-user-video/`). Implements `VhostUserDevice` for
  virtio-media, wrapping `VirtioVideoAdapter` + `VideoDecoder` with
  `VirtioMediaDeviceRunner`. Worker loop matches crosvm's built-in
  media.rs reference. Supports VA-API and FFmpeg decoder backends.
- **virtio-media guest kernel module**
  (`pkgs/virtio-media-driver/`). Builds chromeos/virtio-media
  out-of-tree for kernel 6.18, pinned to commit `ebcef1a`.
- **Video sidecar systemd service** (`video/host.nix`). Per-VM
  `nixling-<vm>-video.service` running as the GPU sidecar user with
  VA-API environment (LIBVA_DRIVER_NAME=nvidia,
  NV_VAAPI_BACKEND=direct). Lifecycle bound to GPU service via
  `partOf`.
- **FreeRDP h264_v4l2m2m integration** (work-aad.nix). Patches
  FreeRDP to prefer `h264_v4l2m2m` decoder with fallback to software,
  removes YUV420P format override, adds thread-local NV12→YUV420P
  deinterleave for v4l2m2m's NV12 output.
- **devbox-connect AVC enablement**. Injects `use video codec:i:2`
  into .rdp files, adds `/gfx:AVC420:on` to FreeRDP command line,
  and auto-sets Windows registry keys for AVC444 software encoding
  via `/shell` on connect.

### Fixed

- **EventQueue deadlock** in vhost-user mode. Upstream
  `EventQueue::send_event()` blocks with `event().wait()` on the
  event queue kick eventfd. Fixed by adding a non-blocking
  `reset()` + `pop()` before the blocking wait.
- **SET_VRING_BASE race**. CH reads `avail_idx` from guest memory
  at activate time, but the virtio-media driver pre-queues 16 event
  buffers before `DRIVER_OK`, making them invisible. Fixed by
  forcing `vring_bases = vec![0; N]` in the media device's
  `activate()`.
- **Video socket startup race**. The GPU service's socket wait loop
  now exits non-zero if the video socket doesn't appear within 10
  seconds, preventing CH from starting with a missing socket.
- **crosvm decoder_adapter panics**. `ResetCompleted` and
  `NotifyError` events now log and continue instead of `todo!()`
  crashing the sidecar.

### Removed

- Dead files from abandoned approaches: virtio-video driver
  (device ID 31), 4 kernel compat patches, USERPTR patches for
  ffmpeg and virtio-media, old crosvm/FreeRDP patch files,
  kernel-v4l2-m2m-prompt.patch (10 files, 977 lines).

### Security

- NV12 scratch buffers in FreeRDP decompress changed from `static`
  globals to `_Thread_local` to prevent data races between
  concurrent decoder contexts.
- Video sidecar socket wait hardened with non-zero exit on timeout.
- Video sidecar lifecycle bound to GPU service via `partOf`.

## [0.2.0] - 2026-05-20

Minor release introducing the **observability subsystem**: a new
opt-in component category that provisions a single-host telemetry
sink VM (`sys-obs-stack`) wired over virtio-vsock — no IP between
the observer and the observed VMs, no shared SSH credentials. The
release ships per-VM Alloy agents, a Cloud Hypervisor metrics
exporter, host-side journald forwarding, 6 provisioned Grafana
dashboards, 8 Prometheus alert rules, and `otel-cli` helpers that
stamp local trace IDs onto CLI lifecycle events for correlation.
The stock host setup still keeps the OTLP receiver on a Unix
socket, so Tempo export remains an opt-in follow-up rather than a
default-on path. Manifest schema bumped from version 1 to 2 to add the
`_observability` reserved sentinel and per-VM `observability`
block. A new `AGENTS.md` policy makes the panel-review process a
**hard gate** per phase for multi-phase plans.

### Added

- **Observability subsystem** (`nixling.observability.enable`,
  default `false`). When enabled, the framework auto-declares the
  `obs` env (default `lanSubnet = 10.40.0.0/24`,
  `uplinkSubnet = 203.0.113.0/30`) and the `sys-obs-stack` VM that
  runs Grafana + Prometheus + Loki + Tempo + a central Alloy OTLP
  receiver. Retention defaults: metrics 30d, logs 14d, traces 7d
  (all per-knob configurable via
  `nixling.observability.retention.{metrics,logs,traces}`).
- **Per-VM guest agent** (opt-in via
  `nixling.vms.<vm>.observability.enable`). Each monitored guest
  runs Alloy scraping node metrics + journald (each
  individually toggleable via
  `vm.observability.{scrapeJournal,scrapeNodeMetrics}`), receives
  in-VM OTLP on a UDS, and exports over virtio-vsock through the
  hardened `nixling-otel-vsock-out.service` (socat sidecar:
  `RestrictAddressFamilies=[AF_UNIX AF_VSOCK]`,
  `DeviceAllow=/dev/vsock`, `restartIfChanged=false` per v0.1.7 H7).
- **Host-side forwarder** (`services.alloy` on the host, forwarder
  mode, no storage). Scrapes nixling sidecar units' journald + node
  metrics + the loopback CH-exporter `/metrics`. Pushes all signals
  through `nixling-otel-host-bridge.service` to the obs VM.
- **Cloud Hypervisor metrics exporter**
  (`nixling-ch-exporter.service`, pure-Bash + jq + curl + socat —
  no new language runtime in the host closure). Polls each VM's CH
  REST socket (`/vmm.ping`, `/vm.info`, `/vm.counters`), exposes
  Prometheus text on `127.0.0.1:9101/metrics`. Counter allowlist
  pinned to Cloud Hypervisor v50 device IDs (`_net*`, `_disk*`,
  `_fs*`, `_pmem*`, `__rng`, `__balloon`, `__console`); unknown
  schema rolls into `nixling_vm_unknown_counters_total`. Topology
  labels (`bridge`, `tap`, `tpm`, `graphics`, `audio`,
  `usbip_yubikey`) are off by default to keep the security-posture
  surface narrow — flip
  `nixling.observability.ch.exporter.includeTopologyLabels` on for
  debug. Detects both `microvm@<vm>.service` and
  `nixling-<vm>-gpu.service` so graphics VMs are reported running.
- **Vsock transport** — no IP between VMs, no SSH credentials
  between observer and observed. Cloud Hypervisor `--vsock cid=N,...`
  is appended to every observability-enabled VM and to
  `sys-obs-stack`; a per-VM `nixling-otel-relay@<vm>.service` (socat
  host relay, `RestrictAddressFamilies=[AF_UNIX]`) stitches
  workload-VM vsock to obs-VM vsock at the host. Relay is wired
  via `microvm@%i.service.wants` for headless VMs and via
  per-VM `wants` on `nixling-<vm>-gpu.service` for graphics VMs
  (graphics VMs do not use `microvm@`).
- **CLI lifecycle telemetry** — `nixling up/down/switch/boot/test/
  rollback/gc/usb/audio` emit OTel spans via `otel-cli` and
  structured JSON journald events for every high-value lifecycle
  step. Spans are populated with allowed labels only (`vm.name`,
  `vm.env`, `vm.role`, `nixling.subcommand`, `systemd.unit`, `tap`,
  `bridge`, `static_ip`, `generation`) — never command output, key
  paths, or Nix store paths. `nl_span_start` generates `trace_id` +
  `span_id` locally via `/dev/urandom` so Loki↔Tempo correlation
  works even when no upstream OTLP collector endpoint is configured;
  honors otel-cli's traceparent when one is. `otel-cli` is
  module-time-gated into `runtimeInputs` via
  `nixling.observability.cli.traces.enable` (default `true`); hosts
  with observability disabled pay zero closure cost.
- **6 provisioned Grafana dashboards** under the "Nixling" folder:
  Nixling Overview, VM Resources, Lifecycle Traces, Logs, Per-VM
  Store, Obs VM Health. Default refresh 30s. Tempo→Loki
  trace-to-logs correlation via `derivedFields`.
- **8 Prometheus alert rules**: `NixlingVMDown`,
  `NixlingNetVMDownWithRunningWorkloads`,
  `NixlingObsVMUnreachableFromHost`, `NixlingVsockRelayDown`,
  `NixlingCHAPISocketMissing`, `NixlingStoreSyncFailure`,
  `NixlingGuestTelemetryMissing`, `NixlingObsVMStackUnhealthy`.
  Each rule individually toggleable via
  `nixling.observability.alerts.<name>.enable`. Notification
  channels are intentionally unconfigured — operators choose
  Alertmanager / Grafana contact-points.
- **Grafana auth**: defaults to authenticated access as
  `nixling-admin`. Password is generated at activation and stored
  at `/var/lib/nixling-observability/grafana-admin-password` inside
  `sys-obs-stack`, or sourced from sops/agenix via
  `nixling.observability.grafana.adminPasswordFile`. Session signing
  key follows the same pattern via
  `nixling.observability.grafana.secretKeyFile`. Anonymous Viewer
  is opt-in only for trusted single-host LANs via
  `nixling.observability.grafana.anonymousViewer.enable`; the login
  form remains available even in that mode.
- **Eval assertions**: vsock CID uniqueness across enabled VMs
  (reserved CID 1000 for `nixling.observability.vmName`),
  per-VM-without-framework rejection, reserved-prefix exemption for
  `cfg.vmName`, env uplink CIDR materialization check.
- **Tests**: `tests/observability-eval.sh` (23/23 cases, 1 promtool
  skip when absent — covers option schema, auto-declaration,
  CID allocation, per-VM toggle defaults, name/prefix collisions,
  CLI-traces closure gating, relay ACL wiring, stack VM guest
  surface, dashboard schema validation, rule-file `promtool`
  validation, metric-reference coverage, scrape-job exact-set,
  and the graphics-VM runner wiring path).
- **Examples**: `examples/with-observability/` minimal consumer
  flake validated by the per-example flake-check loop.
- **Docs**:
  - `docs/reference/components-observability.md` — option schema,
    port/CID/UDS table, naming conventions, systemd unit
    inventory, dashboard inventory, alert severity table,
    security boundaries, label conventions, retention defaults,
    opt-out paths.
  - `docs/how-to/enable-observability.md` — step-by-step recipe
    including sops/agenix examples for both the Grafana
    secret-key and admin-password.
  - `docs/explanation/design.md` — appended Observability section
    explaining the vsock-vs-reverse-SSH-vs-guest-init trade-off,
    the two-bridge necessity, the alternatives-considered list,
    CLI attribute hygiene, and the trust-concentration risk on
    the obs VM.
  - `docs/reference/manifest-schema.md` — `manifestVersion = 2`
    rationale.

### Changed

- **`manifestVersion` 1 → 2** (breaking under pre-1.0 minor-bump
  policy). The manifest now ships a top-level `_observability`
  reserved sentinel and a per-VM `observability` block
  (`enabled`, `vsockCid`, `vsockHostSocket`). Existing consumers
  who do not enable `nixling.observability.enable` see the new
  fields populated with `enabled = false` defaults — the
  manifest still describes their VMs deterministically.
- **`docs/reference/manifest-schema.{md,json}`** updated to
  describe the v2 schema.
- **AGENTS.md** adds a "Panel review" hard-gate policy: multi-phase
  plans must pass plan-review BEFORE implementation and work-review
  BEFORE phase advancement, with documented escape hatches for
  trivial, hotfix, and docs-only changes.

### Security

- Telemetry sidecar trust posture: dedicated locked system users
  (`nixling-otel-relay`, `nixling-otel-bridge`,
  `nixling-ch-exporter`) with execute-only ACLs on per-VM state
  directories and `rw` ACLs only on the per-port vsock sockets
  they need (`vsock.sock_14317`, not the base `vsock.sock`).
  Activation-time ACL refresh is idempotent and revokes stale
  grants when an observed VM is later disabled.
- `nixling-otel-acl-refresh` rejects symlinked state paths,
  validates resolved paths stay under the state root, and uses
  `setfacl --physical` when available — closes the TOCTOU
  window on a group-writable state tree.
- Grafana `secret_key` and admin password are never written to
  the world-readable Nix store. Both are generated atomically at
  activation (write-to-tmp + `mv -f`) and loaded via systemd
  `LoadCredential` into `/run/credentials/grafana.service/`, or
  sourced from operator-supplied files via
  `nixling.observability.grafana.{secretKeyFile,adminPasswordFile}`.
- Loki query selectors in shipped dashboards never default to a
  whole-namespace scan: every variable-driven selector requires
  a non-empty match (`.+`, not `.*`), and the trace-to-logs
  derivedField is scoped by trace-derived `vm`/`env` labels.
- Alert annotation templates carry `vm` and `env` only; full
  unit/job names stay inside dashboards (not exported to
  whichever notification backend an operator wires up).
- CLI span attribute extras are filtered through an allowlist
  in `nl_filter_attrs`: caller-supplied keys outside
  `{step, result, systemd_unit, tap, bridge, static_ip, generation,
  vm_role}` are dropped with a journald warning, as are values
  matching common secret/store-path patterns.
- The guest UDS→vsock relay is fork-bounded
  (`max-children=16`, `TasksMax=32`, `MemoryMax=64M`,
  `LimitNOFILE=1024`) to bound in-guest DoS surface.
- The host telemetry bridge runs as `alloy` with
  `SupplementaryGroups=[kvm]` (no over-broad `nixling-otel-host-bridge`
  user) and connects to a narrowed
  `/run/nixling/alloy/` subdirectory rather than the shared
  `/run/nixling/` root.
- Documented trust-concentration risk: `sys-obs-stack` has read
  access to every monitored VM's telemetry; treat as privileged
  infrastructure. Single-host single-VM by design (multi-host
  is explicitly out of scope for v0.2.0).

### Deferred to v0.3.0

- **`NixlingVMStuckWithoutSSH` alert** — needs a new
  CH-exporter metric (`nixling_vm_ssh_ready`) before the rule
  can be defined non-trivially.
- **`nixling_vm_store_path_count`** — the Per-VM Store
  dashboard references this metric today but it is currently
  **future-work absent**: no exporter emits it yet. The dashboard
  panel renders empty until a future store-path-count exporter
  lands (planned for v0.3.0). The `obs-metric-references`
  test gate treats it as a documented future-work exception
  rather than an unknown metric.
- **`nixling_vm_counter_net_tx_bytes` and
  `nixling_vm_counter_net_rx_bytes`** — referenced by the VM
  Resources network panel for legacy compatibility; the actual
  emitted metric names are `nixling_vm_counter_virtio_net_*`
  (CH v50 device naming). Documented as **future-work absent**
  pending dashboard query simplification — both legacy and
  modern names will resolve via Prometheus `or` until the legacy
  names are removed.
- **Stable relay-binary interface.**
  `nixling.observability.transport.relayPackage` still
  requires a `bin/socat`-compatible CLI today. A future
  release will define a stable interface so non-socat relays
  (e.g. a purpose-built Rust binary) can be swapped in
  without socat-compat shims, and the socat-compatible path
  will remain supported for at least one minor release after
  that lands.
- **VM-runner abstraction.** Today the framework leaks the
  runner-unit name (`microvm@<vm>` for headless,
  `nixling-<vm>-gpu` for graphics) into the relay wiring, and
  the observability code has to wire to both. v0.3.0 will
  introduce a runner-agnostic abstraction (e.g.
  `nixling-vm-runner@<vm>.service` aliased by whichever
  concrete runner is used) so per-VM sidecar wiring stays
  on a single name.

--- *Wave 6 maturity additions (folded in pre-tag - v0.2.0 was deferred until the consumer-integration shakedown landed):* ---

### Changed

- **sshd host keys are now generated on the HOST and shared into
  every guest read-only via virtiofs.** A new module
  `nixos-modules/host-ssh-host-keys.nix` provisions per-VM ed25519
  host keys at host activation under
  `${nixling.site.stateDir}/vms/<name>/sshd-host-keys/` (mode 0400
  root:root). `nixos-modules/store.nix` shares the directory into
  the guest at `/run/nixling-sshd-host-keys/` (virtiofs tag
  `nl-ssh-host`). A new `nixos-modules/guest-sshd-host-keys.nix`,
  imported into every enabled VM by `host.nix`, points
  `services.openssh.hostKeys` at the shared path and disables the
  NixOS `ssh-keygen -A` activation hook. **Why**: pre-v0.2.0 each
  guest regenerated its sshd host keys on first boot and stored
  them on the tmpfs overlay over the read-only nix store, so they
  were ephemeral. Every VM restart regenerated them, the host's
  `known_hosts.nixling` pinned the first observed set and refused
  to overwrite subsequent ones (correctly: from the host's point
  of view, a host-key change IS a possible MITM/swap), and
  operator SSH from the host would soft-brick until manual
  `ssh-keygen -R` + a refresh-service kick. Host-managed keys
  eliminate the drift class entirely.
- **`nixos-modules/host-known-hosts.nix`**: the refresh script
  now reads the host-side `.pub` file directly instead of probing
  the live VM with `ssh-keyscan`. Faster (no boot wait), immune
  to the live-vs-pinned drift the old logic had to handle (a VM
  restart used to regenerate the in-VM key every time).
- **Observability admin password + secret key are now generated
  on the HOST, not inside `sys-obs-stack`.** A new module
  `nixos-modules/observability-host-secrets.nix` provisions both
  files at host activation under
  `${nixling.site.stateDir}/observability/` (default
  `/var/lib/nixling/observability/`, mode 0400 root:root) and
  shares them read-only into the stack VM via virtiofs at
  `/run/nixling-obs-secrets/`. The in-VM activation scripts that
  used to generate these secrets in
  `/var/lib/nixling-observability/` (inside `sys-obs-stack`) have
  been removed. **Why**: putting both secrets inside the VM
  pointed the trust flow the wrong way — anything on the host
  that needed the Grafana admin password (a launcher, a health
  probe, a backup) had to cross the VM boundary to read it, which
  in practice forced consumers to add an SSH-able operator
  account + sudoers rule inside `sys-obs-stack` just to claw the
  password back out. With this change, host-side
  `sudo cat ${nixling.site.stateDir}/observability/grafana-admin-password`
  is the supported path; no operator account inside the stack VM
  is required. The `nixling.observability.grafana.{secretKeyFile,
  adminPasswordFile}` overrides still work for sops-nix / agenix
  users.
- **Consumer extensions of the auto-declared observability VM are
  now allowed.** The pre-v0.2.0 assertion that rejected any
  user-side definition under `nixling.vms.<obsCfg.vmName>` was
  removed. The framework's auto-declaration block uses
  `lib.mkDefault` for every value, so a consumer override
  (e.g. `nixling.vms.sys-obs-stack.ssh.user = "root"`) merges
  cleanly. The matching `assertions-eval.sh` test was renamed to
  `observability-vmname-extension-allowed` and asserts the new
  behaviour.
- **Default obs-VM memory bumped 512 M → 2048 M.** Grafana
  alone wants ~200 M RSS on idle; the full
  Grafana+Prom+Loki+Tempo+Alloy stack in a single VM tripped the
  in-VM OOM killer within seconds of boot at the previous 512 M
  default. 2 GiB is the minimum that lets the whole stack come
  up with default retention windows on a single-host install
  monitoring ~tens of VMs. `lib.mkDefault` so operators can
  override either way.
- **`services.alloy` /run/nixling/alloy via `RuntimeDirectory`,
  not tmpfiles**, on host + every guest + stack VM. The previous
  tmpfiles rule could not chown to the DynamicUser-allocated
  `alloy` UID at activation time; the directory either never
  appeared or was owned by `nobody:nogroup`, breaking
  `nixling-otel-host-bridge` setfacl + alloy's writability
  expectations.
- **Alloy `labels = { ... }` map literals updated with trailing
  commas** in `components/observability/{host,guest}.nix`. Alloy
  DSL distinguishes between newline-separated *blocks* (no `=`)
  and comma-separated *map literals* (with `=`); the latter were
  emitted without commas and rejected by Alloy's parser at boot.
- **`host-otel-relay-acl` + `host-ch-exporter`**: added
  `excludeShellChecks = [ "SC2034" ]` for bash namerefs and
  positional placeholders in `read`. Both scripts use shell
  patterns shellcheck cannot follow; the warnings became fatal
  the moment `writeShellApplication` actually built them in a
  consumer rebuild.
- Eval test `obs-stack-vm-guest-surface: grafana LoadCredential
  wires secret_key credential file` updated to assert the new
  in-VM source path
  `/run/nixling-obs-secrets/grafana-secret-key` (was the in-VM
  `/var/lib/nixling-observability/grafana-secret-key`).

### Migration

- Fresh installs land on the new layout with no operator action.
- Pre-existing installs that booted v0.2.0 with the in-VM
  observability secret generator will see a **password rotation**
  at the next `nixos-rebuild switch`: the new host-generated
  secret displaces the old in-VM one. Operators should fetch the
  new password via
  `sudo cat /var/lib/nixling/observability/grafana-admin-password`
  on the host.
- Pre-existing installs that had ephemeral in-VM sshd host keys
  pinned in `/var/lib/nixling/known_hosts.nixling` will see a
  **one-time host-key change** for every VM at the next
  activation+restart: the host now generates a stable ed25519
  host key per VM and the refresh service swaps the pinned entry
  on the next `microvm@<vm>` start. The framework handles this
  automatically; operator SSH clients (outside the framework)
  may need a one-time `ssh-keygen -R <ip>` against their personal
  `~/.ssh/known_hosts` if they manually trusted the old key.


### Fixed

- **`nixos-modules/host-keys.nix`**: per-VM `.desktop` launchers
  failed with "Permission denied" on the SSH private key because
  the keys directory (`/var/lib/nixling/keys/`) lacked a traverse
  ACL for `nixling-launcher`. The directory had a
  `group:nixling-launcher:--x` ACL entry, but both the tmpfiles
  rule and the activation script's `install -d -m 0700` set the
  directory mode to `0700`, which forces the POSIX ACL mask to
  `---` and neutralizes the named-group entry. Fix: add
  `setfacl -m "g:nixling-launcher:--x"` on the keys directory
  in the activation script, after the `install -d`, so the mask
  is recalculated to include `--x`.

- **`nixos-modules/host-known-hosts.nix`** + **`nixos-modules/cli.nix`**
  (`vmLaunchScript`): graphics-VM per-VM `.desktop` launchers
  silently did nothing when the pinned host key in
  `known_hosts.nixling` was stale. Two coupled bugs:
  1. `nixling-known-hosts-refresh@%i.service` was wanted only by
     `microvm@%i.service`, but graphics VMs bypass that template
     (the GPU sidecar runs cloud-hypervisor directly). The
     refresh therefore only fired during `nixos-rebuild`
     activation — often tens of minutes before the user actually
     launched the graphics VM — and every one of those
     activation-time refreshes timed out because the VM wasn't
     running yet. The pinned key stayed stale across rebuilds.
     Fix: also `Wants=nixling-known-hosts-refresh@<vm>.service`
     from `nixling-<vm>-gpu.service` for graphics-enabled VMs,
     with a matching `After=nixling-%i-gpu.service` on the
     refresh template.
  2. `vmLaunchScript` (`cli.nix`) ran a 30 s ssh-readiness probe,
     discarded its stderr, did not track success/failure, and
     unconditionally `exec`'d `konsole -e ssh …`. With a stale
     pin every probe failed silently with
     `Host key verification failed!`; konsole then exec'd into an
     immediately-failing ssh and closed — observed by the user as
     the launcher "doing nothing" whether the VM was up or down.
     Fix: track probe success, classify the failure on timeout
     (host-key mismatch vs. unreachable), and surface
     `notify-send` with the exact remediation command (host-key
     case points at
     `sudo systemctl start nixling-known-hosts-refresh@<vm>.service`).

## [0.1.7] - 2026-05-19

Patch release. v0.1.6 panel review caught a silent bug in the
v0.1.5 lifecycle policy: three of the six per-VM sidecars used
`unitConfig.X-RestartIfChanged = false` instead of the top-level
NixOS option `restartIfChanged = false`. The two forms LOOK
equivalent and both compile to a setting on the unit file —
but NixOS's `switch-to-configuration` logic only reads
`X-RestartIfChanged=` from the `[Service]` section. The
`unitConfig.X-RestartIfChanged` form emits under `[Unit]`,
where it is silently ignored. Result: pre-v0.1.7, every
`nixos-rebuild switch` that touched the GPU, swtpm, or snd
sidecar config STILL cycled those sidecars under the running
VM, defeating the v0.1.5 policy on the exact services whose
restart causes the most damage (CH termination, TPM socket
loss, audio sidecar disconnect).

### Fixed

- **`nixos-modules/host-sidecars.nix`** (swtpm + GPU sidecars):
  replaced `unitConfig.X-RestartIfChanged = false` with
  top-level `restartIfChanged = false`.
- **`nixos-modules/components/audio/host.nix`** (snd sidecar):
  same fix.
- **`tests/restart-policy-eval.sh`** (Test-H7 regression added
  in v0.1.6): tightened the predicate to REJECT
  `unitConfig.X-RestartIfChanged`. The previous version
  accepted either form, so it would have passed against the
  v0.1.5/v0.1.6 broken setup. Now any service using the broken
  form fails the test with an explicit message pointing at this
  CHANGELOG entry.
- **AGENTS.md** "Adding new per-VM units" guidance: explicitly
  forbids `unitConfig.X-RestartIfChanged`; mandates the
  top-level `restartIfChanged = false` form.
- **`docs/reference/components-{graphics,tpm,audio}.md`**:
  updated lifecycle subsections to reference the corrected
  form. The Lifecycle section subheaders still call this
  v0.1.5+ behaviour because the policy was always v0.1.5;
  v0.1.7 is just making the v0.1.5 intent actually work.

### Verification

The three sidecar files now match the pattern already used in
`host-wrapper.nix`, `host-known-hosts.nix`, and `store.nix`
(`restartIfChanged = false` at the top level). The
`tests/restart-policy-eval.sh` gate now asserts the correct
form on all 6 services and would have caught the v0.1.5 bug
at landing time. All other v0.1.6 gates remain green.

Spec correction #39 added.

## [0.1.6] - 2026-05-19

Docs catch-up release. The v0.1.1–v0.1.5 patches shipped fixes for
five framework bugs surfaced during the first real consumer
migration, but the public docs hadn't been updated to describe the
resulting behavior changes. This release brings the docs in sync
with the code, plus a small audit-strict fix that completes
`v0.1.4`'s skip-stopped-VMs work, and (in the v0.1.6 follow-up
panel sweep) tightens the autostart wiring + adds regression tests
for every v0.1.x patch.

### Changed

- **`nixling list` status label**: `[pending switch]` →
  `[pending restart]`. The label tracks the *recommended action*,
  and the recommended action for unit-file drift after a host
  `nixos-rebuild switch` is `nixling restart <vm>` (clean down+up
  cycles the running closure over the staged unit files); `nixling
  switch <vm>` is the heavier per-VM-closure-rebuild path for
  VM-NixOS-module edits. CLI messages in `nixling status` and the
  `nixling list` trailer updated to match.

- **`systemd.targets.microvms.wants` is now `lib.mkForce []`** on
  every consumer. Previously v0.1.3 narrowed the list to
  autostart=true VMs; v0.1.6 narrows further to `[]` so all
  autostart wiring goes through `systemd.targets.multi-user.wants
  -> nixling@<vm>.service` exclusively. Removes the duplicate
  boot path (target.wants pulling `microvm@<vm>` directly,
  bypassing the framework wrapper).

### Added (assertions)

- **`graphics.enable + autostart` is now an eval-time error.** A
  graphics VM with `autostart = true` would boot through the
  upstream microvm@<vm> runner without the GPU sidecar's
  Wayland-socket bind, leaving the VM with no display. The
  assertion's remediation message points at `nixling up <vm>`
  from a Plasma terminal.

### Added (tests)

- `tests/smoke-eval-extraspecialargs.nix` — regression for Spec
  correction #30 (v0.1.1 extraSpecialArgs propagation through
  `nixos-modules/host.nix:165`).
- `tests/net-vm-network-eval.sh` extended — Spec correction #31
  (v0.1.2 ConfigureWithoutCarrier + route entry on the host's
  uplink bridge).
- `tests/autostart-wiring-eval.sh` — Spec corrections #32 + #33
  + v0.1.6 SWArch-M10 (`nixling@<vm>` is template-only;
  multi-user.target.wants wiring; `microvms.target.wants == []`).
- `tests/smoke-eval-graphics.nix` extended — Spec correction #34
  (v0.1.4 `/dev/net/tun rw` in the GPU sidecar's DeviceAllow).
- `tests/smoke-eval-tpm.nix` — Spec correction #35 (v0.1.4
  swtpm parent-dir ACL traversal grant).
- `tests/restart-policy-eval.sh` — Spec correction #37 (v0.1.5
  `restartIfChanged = false` across all six services).
- Negative-assertion regression for v0.1.6 SWArch-M9 in
  `tests/assertions-eval.sh` (`test_graphics_with_autostart`).

### Added (docs)

- **`docs/reference/cli-contract.md`** documents:
  - `nixling restart <vm> [--force]` (v0.1.5)
  - `pending-restart` indicator semantics in `nixling list` /
    `nixling status` (v0.1.5)
  - `nixling.site.extraSpecialArgs` consumer-side escape hatch
    (v0.1.1)

- **`docs/explanation/design.md`**:
  - New "VM lifecycle policy" section explaining
    `restartIfChanged = false` on all per-VM units, the
    `booted`/`current` symlink contract, and how
    `pending-restart` is computed (v0.1.5).
  - New "Per-env bridge bootstrap" subsection covering the
    `ConfigureWithoutCarrier = true` requirement on the uplink
    bridge and how it breaks the route-preflight deadlock at
    boot (v0.1.2).
  - New "GPU sidecar substitutes microvm-run" subsection
    explaining why the GPU sidecar carries `DeviceAllow=/dev/net/tun`
    (v0.1.4), the `microvm-set-booted`-equivalent ExecStartPre
    (v0.1.5), and the swtpm-user ACL grant (v0.1.4).
  - "Why not X" — new FAQ entry: "Why doesn't `nixos-rebuild
    switch` restart VMs?", cross-linking to the cli-contract's
    pending-restart predicate.
  - Removed `tests/static.sh doesn't iterate examples` and
    `ROOT defaults to /etc/nixos` from "Limitations / known
    gaps" (resolved in W6).

- **`docs/how-to/migrating-from-microvm.md`**:
  - Required minimum `nixling = github:vicondoa/nixling/v0.1.6`
    (or later) — earlier versions exposed framework bugs that
    blocked real-world graphics + TPM bring-up. (Aligned with
    the CHANGELOG; v0.1.6 is the first release where the docs
    match the shipping code.)
  - New "After every rebuild" step in the procedure: check
    `nixling list` for `[pending restart]`, apply with
    `nixling restart <vm>`. Cross-links to the cli-contract's
    pending-restart section.
  - New troubleshooting note: `nixling status <vm>` shows
    `booted` vs `current` mismatch and the exact remediation
    command.

- **`docs/reference/components-graphics.md`**:
  - Added `/dev/net/tun rw` to the documented DeviceAllow list,
    with the rationale (cloud-hypervisor attaches to the tap
    upstream microvm.nix's `microvm-tap-interfaces@<vm>.service`
    helper created).
  - New "Lifecycle" subsection: GPU sidecar IS the
    cloud-hypervisor process; `restartIfChanged = false` keeps
    rebuilds from killing the VM.

- **`docs/reference/components-tpm.md`**:
  - Added the ACL traversal grant on the parent state dir to
    the documented host-side resources. No manual `chown`
    required for v0.1.4+ consumers — the framework's
    `nixlingVmStatePerms` activation script handles it.
  - Updated the "DO NOT WIPE" warning to also point at the
    `pending-restart` indicator as the right signal for
    "TPM-bound creds may be re-read after restart".
  - New "Lifecycle (v0.1.5+)" subsection documenting
    `nixling-<vm>-swtpm.service`'s `unitConfig.X-RestartIfChanged
    = false`.

- **`docs/reference/components-audio.md`**:
  - New "Lifecycle (v0.1.5+)" subsection documenting
    `nixling-<vm>-snd.service`'s `unitConfig.X-RestartIfChanged
    = false`.

- **`AGENTS.md`**:
  - New "VM lifecycle policy" subsection documenting
    `restartIfChanged = false` as a framework invariant for
    contributors.
  - New convention: per-VM `wantedBy` ALWAYS via
    `systemd.targets.multi-user.wants` symlinks, never via
    per-instance `systemd.services."nixling@${name}"`
    declarations (which NixOS materializes as separate unit
    files lacking the template's lifecycle hooks).

- Example READMEs (`minimal`, `graphics-workstation`, `multi-env`,
  `with-entra-id`) gain a short "After subsequent rebuilds"
  cross-link block pointing at the template README's post-rebuild
  section.

- Plan/spec corrections (#30-#38) tracking the v0.1.x patches
  plus the v0.1.6 follow-up sweep.

### Fixed

- **`nixos-modules/cli.nix`** (`audit --strict`): the
  `bridge_isolated_workload.<vm>` skip-when-down predicate (added
  in v0.1.4) only checked `microvm@<vm>.service`. Graphics VMs
  run cloud-hypervisor via the `nixling-<vm>-gpu.service` sidecar
  (the GPU sidecar replaces the upstream runner), so the audit
  was blanket-skipping all graphics VMs even when they were
  running. Now: a VM is "running" if any of `nixling@<vm>`,
  `microvm@<vm>`, or `nixling-<vm>-gpu` is active.

- **`nixos-modules/cli.nix`** (`nixling list` / `nixling status`):
  pending-drift messages used to recommend `nixling switch <vm>`,
  which is the heavier per-VM-closure-rebuild path. The correct
  remediation for unit-file drift after a host `nixos-rebuild
  switch` is `nixling restart <vm>` (clean down+up cycles the
  running closure over the staged unit files). Messages updated;
  status label `[pending switch]` renamed to `[pending restart]`
  to match.

## [0.1.5] - 2026-05-19

Patch release. Three consumer-impacting items from the first
`/etc/nixos`-side migration: the framework's nixos-rebuild
hot-restart of per-VM sidecars was killing running VMs; the
load-host-keys group assumption broke for the standard NixOS user
shape; and once we stopped restarting, consumers had no signal that
config drift had built up.

### Added

- **`nixling restart <vm> [--force]`** — convenience wrapper around
  `down <vm>` + `up <vm>`. Idempotent (a stopped VM is just brought
  up). Graphics VMs still require a Wayland session for the up
  step. The `--force` flag is forwarded to the down step (lets you
  cycle a net VM without first stopping the env's workloads). Used
  in tandem with the new `pending-restart` indicator below: when
  `nixling list` flags a VM, `nixling restart <vm>` applies the
  pending config.

- **`pending-restart` signal in `nixling list` / `nixling status`.**
  Compares each VM's `current` symlink (latest declared closure)
  against `booted` (the closure the running VM actually exec'd).
  If they differ AND the VM is up, both UIs flag the VM:

  ```
  NAME             ENV    GRAPHICS TPM   USBIP   STATIC_IP       STATUS
  work-aad         work   true     true  true    10.20.0.10      systemd [pending restart]
  ```

  And `nixling status work-aad` adds:

  ```
  pending-restart: YES — unit files changed; run `nixling restart work-aad` to apply
    booted : /nix/store/...-microvm-cloud-hypervisor-work-aad
    current: /nix/store/...-microvm-cloud-hypervisor-work-aad
  ```

  Note: v0.1.5 originally shipped the label as `[pending switch]`
  with a `run nixling switch <vm>` recommendation; v0.1.6 renamed
  the label to `[pending restart]` and the message to recommend
  `nixling restart <vm>` (the correct action for unit-file drift
  is the lighter `restart`, not the per-VM-closure-rebuild
  `switch`). Pre-v0.1.6 docs may show the legacy strings.

  Required because of the `restartIfChanged = false` changes below
  — without that signal, consumers had no way to know their
  `nixos-rebuild switch` only landed unit-file changes and not VM
  behaviour.

### Fixed

- **`restartIfChanged = false` on every per-VM lifecycle service.**
  Pre-v0.1.5, every `nixos-rebuild switch` that touched any of the
  per-VM units killed the running VM mid-flight — for graphics
  VMs the GPU sidecar IS the cloud-hypervisor process, so its
  restart terminated CH, the guest's in-RAM Entra device-bound
  tokens evaporated, and the user lost their login session. Even
  for headless VMs, every framework-touched config (host-keys
  refresh wiring, virtiofsd hardening stanza) caused NixOS to
  override upstream microvm.nix's `X-RestartIfChanged=false` back
  to `true`. The new flag updates the unit files at rebuild time
  but does NOT cycle the running VM; consumers apply per-VM
  changes via `nixling restart <vm>` (or `nixling switch <vm>`
  for a per-VM closure rebuild + live activation).

  Services covered:
  - `nixling@<vm>.service` (user-facing wrapper)
  - `microvm@<vm>.service` (upstream runner; framework was
    overriding upstream's existing flag back to true via the
    host-known-hosts.nix drop-in)
  - `microvm-virtiofsd@<vm>.service` (per-VM virtiofs daemon;
    framework adds hardening stanza)
  - `nixling-<vm>-swtpm.service`
  - `nixling-<vm>-snd.service`
  - `nixling-<vm>-gpu.service`

- **`nixling-<vm>-gpu.service` updates the per-VM `booted`
  symlink.** Upstream microvm.nix's
  `microvm-set-booted@<vm>.service` only runs as part of
  `microvm@<vm>.service`'s lifecycle — but graphics VMs bypass
  that template (the GPU sidecar runs microvm-run directly).
  Pre-v0.1.5, `/var/lib/nixling/vms/<vm>/booted` simply didn't
  exist for graphics VMs, so the new pending-restart check
  couldn't compute anything. Added `ExecStartPre`
  (`+`-prefixed → root) that mirrors
  `microvm-set-booted_-start`:
  `rm -f booted && ln -s $(readlink current) booted`. Cleared
  by `ExecStopPost`.

- **`nixling-load-host-keys.service` primary-group resolution.**
  Pre-v0.1.5 the script assumed the guest user's primary group
  matched the username (`install -d ... -g "$SSH_USER"`). This
  only holds when the consumer's VM config sets
  `users.users.<u>.group = "<u>"` or uses DynamicUser. NixOS's
  `isNormalUser = true` default puts the user in the `users`
  group, breaking the install with
  `install: invalid group '<u>'`. Result: no nixling-managed
  pubkey ever reached the guest's `authorized_keys`, and SSH
  only worked for keys baked statically into
  `users.users.<u>.openssh.authorizedKeys.keys`.

  Now: resolve GID via `getent passwd | cut -d: -f4`, then GID →
  name via `getent group`. Works for both
  `users.users.<u>.group = "<u>"` and the NixOS default.

## [0.1.4] - 2026-05-19

Patch release. Four framework bugs surfaced during the first real
consumer migration's VM bring-up (paydro's /etc/nixos, after v0.1.3
got `nixling@<vm>` units working but the actual graphics+TPM VM
refused to boot).

### Fixed

- **`nixos-modules/host-sidecars.nix`**: per-VM GPU sidecar
  (`nixling-<vm>-gpu.service`) had `DevicePolicy = "closed"` without
  `/dev/net/tun` in `DeviceAllow`. Cloud-hypervisor needs to
  `open("/dev/net/tun")` + `ioctl(TUNSETIFF, …)` to attach to the
  VM's tap (created earlier by upstream microvm.nix's
  `microvm-tap-interfaces@<vm>.service` helper); without it
  graphics VMs crash in early boot with "Cannot create virtio-net
  device / Couldn't open /dev/net/tun / Operation not permitted".
  Added `/dev/net/tun rw` to DeviceAllow.

- **`nixos-modules/host-activation.nix`**: `nixlingVmStatePerms`
  granted ACL rwx on `/var/lib/nixling/vms/<vm>/` to
  `nixling-<vm>-gpu` but not to `nixling-<vm>-swtpm`. The swtpm
  service starts as the swtpm user, opens its `StateDirectory=`
  (which systemd creates at the correct path), then tries to read
  `tpm2-00.permall` — and EACCESes because traversing the parent
  dir requires +x for the swtpm user. libtpms enters failure mode
  and the VM boots with a freshly-initialised TPM, triggering
  Entra/Intune device-tampering alerts for tenant-enrolled VMs.
  Added `setfacl -m "u:nixling-<vm>-swtpm:--x" <stateDir>` (gated
  on `vm.tpm.enable`).

- **`nixos-modules/base.nix`**: `nixling-load-host-keys.service`
  inside the guest referenced `${"$"}{pkgs.coreutils}/bin/getent` —
  but `getent` is in glibc, not coreutils. The lookup silently
  failed with "No such file or directory" and the script printed
  `user '<u>' not found in /etc/passwd — skipping` even though the
  user existed. Result: nixling-managed pubkeys + the consumer's
  `userAuthorizedKeys` never reached the guest's
  `authorized_keys` — SSH worked only via any pubkey statically
  baked into the VM's `users.users.<u>.openssh.authorizedKeys.keys`.
  Fixed path to `${"$"}{pkgs.glibc.getent}/bin/getent`.

- **`nixos-modules/cli.nix`** (audit `--strict`): the
  `bridge_isolated_workload.<vm>` check ran unconditionally and
  STRICT-FAILed when the VM wasn't running (the workload tap
  doesn't exist on the bridge, so jq returned null). With the
  framework's default `nixling.vms.<vm>.autostart = false`, this
  blocked every post-activation `nixling-audit-check.service`
  hook → `nixos-rebuild switch` returned non-zero exit code 4.
  Added a `systemctl is-active microvm@<vm>` precondition that
  emits `AUDIT SKIP [bridge_isolated_workload.<vm>]: VM not
  running` (mirrors the existing virtiofsd skip-when-down
  semantic).

## [0.1.3] - 2026-05-19

Patch release. Two more framework bugs surfaced during the first
real consumer migration, both around the nixling@<vm> wrapper +
microvm.nix interaction.

### Fixed

- **`nixos-modules/host-wrapper.nix`**: per-VM `nixling@<vm>.service`
  units for `autostart=true` VMs were emitted as separate unit files
  (via `systemd.services."nixling@${name}"`) that NixOS materialised
  WITHOUT the template's `ExecStart`/`ExecStop`/`PropagatesStopTo`/
  `Type=oneshot` settings — so systemd refused them at boot with
  "Service has no ExecStart=, ExecStop=, or SuccessAction=. Refusing."

  Fix: drop the per-instance `systemd.services` declarations and
  use `systemd.targets.multi-user.wants` symlinks instead. systemd
  then resolves each `nixling@<vm>.service` against the template
  with all its lifecycle wiring intact.

- **`nixos-modules/host-wrapper.nix`**: upstream microvm.nix emits
  `systemd.targets.microvms.wants = ["microvm@<vm>.service" …]`
  for every `microvm.vms.<vm>` declaration. `microvms.target` is
  itself `wantedBy = ["multi-user.target"]`, so workload VMs got
  pulled into boot regardless of `microvm.autostart = []`. Setting
  `microvm.autostart` only controls upstream's `multi-user.target.wants`
  on the microvm@ unit, not the `microvms.target` Wants= relation.

  Fix: `lib.mkForce` `systemd.targets.microvms.wants` to enumerate
  ONLY `autostart=true` VMs. Workload VMs are now exclusively
  on-demand via `nixling up <vm>`.

## [0.1.2] - 2026-05-19

Patch release. Surfaced during the first real consumer migration to
v0.1.x — a runtime bootstrap deadlock between
`nixling-net-route-preflight.service` and the per-env uplink bridge.

### Fixed

- **`nixos-modules/network.nix`**: per-env uplink bridge
  (`br-<env>-up`) now has `networkConfig.ConfigureWithoutCarrier =
  true`. Without it, networkd refuses to apply the Address + static
  Route to the env's LAN subnet until the bridge has carrier. But
  carrier only appears when the per-env net VM attaches its uplink
  tap to the bridge, and the net VM start is gated on
  `nixling-net-route-preflight.service`, which checks the static
  route exists. Deadlock.

  The LAN bridge already had `ConfigureWithoutCarrier = true`; the
  uplink-bridge case was missing. The fix is one option per env;
  no consumer config changes required.

  Existing v0.1.0 / v0.1.1 consumers can work around by running
  `sudo ip route add <env-lan>/<mask> via <env-uplink-gw> dev
  br-<env>-up` once per env before any
  `nixos-rebuild switch` — but the proper fix is to upgrade to
  v0.1.2 and re-rebuild.

## [0.1.1] - 2026-05-19

Patch release. Two consumer-impacting items surfaced during the
first real `/etc/nixos`-side migration to v0.1.0.

### Added

- **`nixling.site.extraSpecialArgs`** (`attrsOf unspecified`,
  default `{}`). Merged into every per-VM
  `microvm.vms.<vm>.specialArgs` after the framework's own
  baseline. Consumer keys take precedence on collision, so a
  consumer that wants its full flake `inputs` (rather than just
  nixling's narrower input set) visible inside per-VM modules
  can set:
  ```nix
  nixling.site.extraSpecialArgs = { inherit inputs; };
  ```
  Mirrors `home-manager.extraSpecialArgs` from the Home-Manager
  NixOS module — same semantics, same intent.

### Fixed

- **`scripts/migrate-nixling-v0.1.0.sh`**: `[[ -d "$dir" ]] && info ...`
  under `set -euo pipefail` aborted the script silently when the
  optional private-TPM-state directory didn't exist (return-value
  of the compound `&&` chain propagated up as the function's exit
  status). Replaced with explicit `if [[ -d ]]; then info; fi` for
  set-e safety. The bug aborted the snapshot phase before the
  `tpm2_getcap` step could run, leaving the migration in an
  in-progress state that required a manual cleanup.

## [0.1.0] - 2026-05-19

First public alpha release.

**Audience:** single-user NixOS desktop wanting isolated workspaces
(work / personal / risky-dev) on one host. Wayland-native.

**Stable in v0.1.0:**

- `nixosModules.default` (host integration)
- `templates.default` (`nix flake init -t github:vicondoa/nixling`)
- `flake.checks.<sys>.eval-{minimal,multi-env,template,graphics}`
- `nixling@<vm>.service` lifecycle wrapper + the eight `nixling` CLI
  verbs (`up`, `down`, `status`, `list`, `switch`, `build`, `boot`,
  `test`, `rollback`, `generations`, `gc`, `audio`, `usb`, `console`,
  `keys`)
- `manifestVersion = 1` JSON contract (`/run/current-system/sw/share/nixling/vms.json`)
- VM-name regex `^[a-z][a-z0-9-]*$`, reserved prefixes `sys-` and
  exact name `launcher`
- Per-env isolated network (auto-declared `sys-<env>-net` net VM,
  point-to-point uplink, isolated LAN bridge, dnsmasq, nftables NAT)
- Per-VM `/nix/store` hardlink farm
- Nixling-managed SSH keys
- Components: `graphics`, `tpm`, `usbip`, `audio`, `home-manager`

**Composition:** Sibling flake [`vicondoa/nixos-entra-id`][nei] (also
v0.1.0) provides Entra ID device-join via the per-VM
`nixling.vms.<vm>.config.imports = [ inputs.nixos-entra-id.nixosModules.default ]`
seam.

[nei]: https://github.com/vicondoa/nixos-entra-id

> Maintainer GitHub metadata reminder (apply on the GitHub UI, not in git):
>
> - **Description:** "NixOS microVM framework with isolated per-env
>   networking, Wayland/audio/USBIP/TPM components, and a
>   `nix flake init` template scaffold."
> - **Topics:** `nixos`, `nix-flake`, `microvm`, `wayland`,
>   `microvm-nix`, `nixos-template`, `entra-id`.



### Added (W6)

- `flake.checks.<system>.eval-{minimal,multi-env,template,graphics}` —
  the root flake now gates the example flakes + the template
  scaffold. The `graphics` check is x86_64-only.
- `tests/static.sh` now iterates `examples/*/flake.nix` running
  `nix flake check --no-build --all-systems` on each.
- `SECURITY.md` — disclosure path (GitHub Security Advisory primary;
  email fallback) plus the v0.1.0 alpha support matrix.
- `docs/explanation/design.md` — full threat model + defenses-in-depth
  list + a *Why not X* rationale FAQ (~823 LOC).
- `docs/how-to/migrating-from-microvm.md` — option mapping +
  step-by-step migration procedure + troubleshooting. Ordering is
  now build-before-state-move per W6 followup H1.
- Five per-component reference docs under
  `docs/reference/components-*.md` (graphics, tpm, usbip, audio,
  home-manager).
- `docs/reference/manifest-schema.{md,json}` polished with a rendered
  example payload generated from `tests/smoke-eval.nix`.

### Fixed (W6)

- `tests/{static,nixling-store,audio,lib}.sh` no longer assume
  `ROOT=/etc/nixos`; the value is derived from the script's own path
  so the suite runs from any clone. Spec correction #28 closed.
- `tests/nixling-store.sh:33` SC2157 (preexisting).
- Host-specific `NL_FILES` entries (`vms/personal-dev.nix`,
  `vms/work-aad.nix`) dropped or guarded so the static gate stays
  useful for the public flake.
- `tests/audio.sh` `NL_WAYLAND_USER` resolution chain genericized
  (no longer hardcoded to the maintainer's host user).
- README polish: `microVM` is defined inline on first use; a
  maintainer-anecdote phrasing was replaced with neutral wording;
  an encrypted-backup callout was added for `/var/lib/nixling/`.
- Manifest schema `manifestVersion` tightened from `minimum: 1` to
  `const: 1` so the JSON Schema matches the documented prose.

### Changed (W6)

- `docs/README.md` IA now reflects the shipping how-to and
  explanation docs (was previously reference-only).

### Added (W5)

- **`examples/minimal/`** — headless starter example: one env, one
  workload VM, ~25-line flake. The "is nixling for me?" sanity
  test.
- **`examples/graphics-workstation/`** — desktop VM with
  `graphics.enable`, `audio.enable`, and `usbip.yubikey` all on.
  Exercises every host-side sidecar component.
- **`examples/multi-env/`** — two parallel `nixling.envs.<env>`
  instances (work + personal) demonstrating per-env LAN
  isolation, per-env net VMs, per-env USBIP backends, and the
  route-preflight fail-closed gate.
- **`examples/with-entra-id/`** — composition with the sibling
  [`vicondoa/nixos-entra-id`][nixos-entra-id] flake; shows how
  the two trees meet at `nixling.vms.<vm>.config.imports`
  without either flake depending on the other.
- **`templates/default/`** — `nix flake init` scaffold with
  seven numbered `TODO:` markers and a matching
  `assertions = [ … ]` block. `nix flake check` on an un-edited
  scaffold fails with actionable messages until each sentinel is
  replaced.
- **`flake.templates.default`** — wires the template above so
  consumers can `nix flake init -t github:vicondoa/nixling`.

[nixos-entra-id]: https://github.com/vicondoa/nixos-entra-id

### Fixed (W5)

- **`nixos-modules/net.nix`:** neutralize base.nix's catch-all
  `10-eth-dhcp` systemd-networkd network on per-env net VMs. The
  catch-all (`matchConfig.Type = "ether"`) sorted lex-first
  against the per-MAC `10-uplink`/`10-lan` definitions and
  DHCP'd both NICs, preempting the static config. Now overridden
  via `lib.mkForce` with a sentinel MAC that never matches.
  Workload VMs are unaffected — they still inherit the base.nix
  DHCP fallback.
- **`nixos-modules/manifest.nix`:** dropped the redundant
  `default = { }` on the readOnly `nixling.manifest` option.
  The nixpkgs module system treats `default` as an extra
  definition; combined with `readOnly = true` and the matching
  `config.nixling.manifest = …` assignment, it produced
  "set multiple times" only when a graphics VM was synthesized.
  See `tests/smoke-eval-graphics.nix` for the regression test.

### Changed (W5)

- **README:** restructured to lead with a "Where to start" table
  pointing at the four examples and the template, and rewrote
  the Quick start to walk through the template path; the prior
  manual paste-in walkthrough is preserved under
  "Manual integration (without the template)".
- **`docs/README.md`:** added a Tutorials/Examples section
  linking the examples and the template; previously the docs
  index only mentioned the reference quadrant.

### Known gaps (deferred to v0.2.0 or Phase 7/8)

- **USBIP per-env units materialise even when no VM opts in.** Each
  `nixling.envs.<env>` declares `nixling-sys-<env>-usbipd-backend.service`
  and the corresponding proxy socket regardless of whether any
  workload VM in the env has `usbip.yubikey = true`. The units are
  idle when nothing opts in, but they are still installed (per-env
  baseline plumbing was intentional in W2; the unconditional
  materialisation is the gap). Tracked for v0.2.0; the relevant
  conditional belongs around `nixos-modules/network.nix:484-650`.
- **No static lint for `mkOption { default = …; readOnly = true; }`
  + matching `config.<…>` assignment.** Spec correction #29 was
  caught by the W5 reviewer panel, not by tooling. A Phase 7a
  follow-up will add a grep-level lint to prevent the
  `default + readOnly + config-assignment` trio from re-appearing.
  Trio detection is necessary because `store.nix` legitimately
  carries `readOnly + default` on options that have NO matching
  `config.<…>` assignment, so a two-of-three match is fine; only
  the full three is a bug.
- **Per-example flake-check loop is not fully hermetic for
  `examples/with-entra-id`.** `tests/static.sh` iterates
  `examples/*/flake.nix` and runs `nix flake check --no-build
  --all-systems` per example, but `with-entra-id` depends on the
  sibling `vicondoa/nixos-entra-id` flake which the core flake
  cannot pull in as an input. The example's own flake.lock pins
  the sibling and the iteration step exercises eval through it,
  but a clean-tree CI run cannot fully isolate the eval graph
  from the sibling. Tracked for v0.2.0.
- **VM-to-VM east-west traffic within the same env is not
  supported.** Workload taps on the per-env LAN bridge are
  configured with `Isolated = true`, so two workload VMs sharing
  `nixling.envs.<env>` can each reach the net VM (and via NAT,
  the upstream LAN) but cannot directly reach each other.
  Documented in `docs/explanation/design.md` and the
  `nixling.hostLanCidrs` option text. A future opt-in
  (e.g. `nixling.envs.<env>.intraLanIsolation = false`) is on the
  v0.2.0 wishlist.

### Added (W4)

- **Manifest contract is now a documented, versioned interface.**
  - `nixos-modules/manifest.nix` — typed `config.nixling.manifest`
    `attrsOf submodule` option. Replaces the inline manifest
    construction previously folded into `cli.nix`. The Nix module
    system catches schema regressions at eval time.
  - `docs/reference/manifest-schema.md` + `docs/reference/manifest-schema.json`
    (JSON Schema Draft 2020-12) — the v1 public manifest contract
    for downstream consumers (e.g. the future Rust CLI port). The
    JSON Schema is the canonical type spec; the prose doc is a
    field-by-field walkthrough + compatibility policy.
  - `docs/reference/cli-contract.md` — behavioural contract for any
    `nixling` CLI implementation (lifecycle FSM, signal semantics,
    exit codes, JSON vs human output, what is/is-not in scope).
  - `nixling.site.flakePath` is now derived as the CLI's default
    flake reference when unset (cli.nix lifecycle subcommands).
- **`docs/README.md`** — Diataxis IA index (tutorials, how-to,
  reference, explanation). Only the reference quadrant has content
  in W4; the others land on the path to v0.1.0.
- **Multi-arch eval coverage.** `tests/smoke-eval-aarch64.nix` —
  cross-evaluates a headless workload VM on `aarch64-linux`,
  verifying the eval graph stays multi-arch clean. Runtime is still
  `x86_64-linux`-only (cloud-hypervisor + crosvm); aarch64 is
  eval-coverage only.
- **Manifest validation gate.** `tests/static.sh` now renders the
  smoke manifest and runs a 5-check sequence against
  `docs/reference/manifest-schema.json`: render → parse schema →
  JSON-Schema validate → schema-side field cross-check →
  `manifestVersion >= 1`. Plus (W4-followup) a 6th check that diffs
  the prose schema's Per-VM-entry table against the JSON Schema's
  `properties` keys to catch md ↔ json drift.

### Changed (W4)

- **BREAKING for manifest consumers (pre-v0.1.0):** `manifestVersion`
  bumped `0 → 1`. The schema is now the documented contract. Future
  schema changes follow SemVer: minor field additions are
  backward-compatible; breaking changes bump the major (`2`, `3`,
  …). Consumers MUST refuse manifests with a newer major version
  than they were built against.
- **`nixling.vms.<vm>.graphics.enable` and
  `nixling.vms.<vm>.audio.enable` now refuse to evaluate on
  `aarch64-linux`** at the `microvm.vms` translation point. The
  eval-time error explains the constraint. Headless workload VMs
  (`graphics.enable = false; audio.enable = false;`) DO evaluate on
  aarch64-linux for cross-eval testing. Actual runtime is still
  x86_64-linux-only — the aarch64 path is eval-coverage only.
- `pkgs/{crosvm-patched,crosvm-seccomp,vhost-device-sound}/default.nix`
  now carry `meta.platforms = [ "x86_64-linux" ]`.
  `pkgs/spectrum-ch/default.nix` deliberately omits this (see
  in-file comment).
- `nixos-modules/options.nix` (internal refactor, no consumer-
  visible change): split into `options.nix` (aggregator) +
  `options-site.nix` + `options-envs.nix` + `options-vms.nix` for
  reviewability. The smoke-eval drvPath is bit-identical pre/post
  the split.

### Changed (W4-followup)

- **BREAKING for manifest consumers, security fix:** `sshKeyPath`
  removed from the per-VM JSON manifest. The W4 security reviewer
  flagged the field as a private-key path leak — the manifest at
  `/run/current-system/sw/share/nixling/vms.json` is world-readable,
  so exposing a per-VM private-key path leaks the location of
  secret material to every local user. The CLI now resolves the
  private-key path locally at Nix-eval time from
  `nixling.site.keysDir` (or per-VM `ssh.keyPath` override) and
  bakes a static per-VM mapping into the shell wrapper. Consumers
  reimplementing the CLI should mirror that: read
  `nixling.site.keysDir` from their own privileged config access,
  not from this world-readable file. The PUBLIC key path is not
  currently exposed; if a use case warrants it, a future
  `sshPubKeyPath` field is the recommended addition. `manifestVersion`
  stays at `1` — the schema was published moments ago in W4 and no
  external consumers exist yet, so this is a free pre-v0.1.0 break.
- `docs/reference/manifest-schema.json`: `manifestVersion.minimum`
  raised from `0` to `1`. The schema is the contract for v1+;
  pre-v1 manifests (the W2-followup stub) are no longer valid under
  this schema. (test-Med)
- `docs/reference/cli-contract.md`: subcommand inventory reconciled
  with `nixling --help`. `audit` now correctly documents the
  `--strict` + `--human` flags (`--human` auto-enables on TTY);
  `rotate-known-host <vm>` (the companion to `trust`) added to the
  subcommand table and to the human/JSON output section.
- `docs/reference/cli-contract.md`: "What is NOT in this contract"
  section expanded. Spells out that microvm.nix internal lifecycle,
  swtpm internals, virtiofsd implementation, and polkit grant
  specifics are framework-internal; and draws the line between
  contract-bound unit names (`nixling@<vm>.service`,
  `microvm@<vm>.service`) and framework-internal unit names
  (sidecars, USBIP proxies — these MUST be read from the manifest's
  `audioService` etc. fields, not hardcoded).
- `tests/static.sh`: `nix flake check` now uses `--all-systems` so
  Layer-1 exercises both x86_64-linux and aarch64-linux flake
  outputs, not just the builder's system. (sw-arch-Med)
- `tests/static.sh`: 6th manifest-contract check added — diffs the
  field-name column of the prose Per-VM-entry table in
  `docs/reference/manifest-schema.md` against the JSON Schema's
  `$defs.vmEntry.properties` keys, failing the gate if either side
  has a field the other doesn't. (sw-arch-Med)
- README: project status now states runtime is tested on
  `x86_64-linux` desktop and eval-tested for headless
  `aarch64-linux` (reflects the W4 cross-eval coverage).
  (product+docs-Med)
- README: documentation section replaces "docs will live under
  docs/" with direct bullets pointing at the manifest schema and
  CLI contract under `docs/reference/`. (product+docs-Med)
- `tests/README.md`: refreshed for the W4 additions —
  `manifestVersion = 1`, 10/10 assertions-eval cases, the 6-step
  manifest-contract gate (including the new md/json drift detection),
  and the multi-arch eval coverage. (docs-Med)

### Reorganised (W4-followup)

- Diataxis reorg. `docs/manifest-schema.{md,json}` →
  `docs/reference/manifest-schema.{md,json}`; `docs/cli-contract.md`
  → `docs/reference/cli-contract.md`. Added `docs/README.md` as the
  IA index. (docs-Med). All path references in
  `nixos-modules/manifest.nix`, `tests/static.sh`, and the moved
  docs' cross-links updated.

### Removed

- (W3b/Phase-2b) **`nixling.vms.<vm>.entra-id.*` option removed.**
  Himmelblau / Microsoft Entra ID support has moved out of the
  nixling framework and into the sibling `vicondoa/nixos-entra-id`
  flake. To migrate, add the flake as an input and import its
  module into the VM's guest config:

  ```nix
  inputs.nixos-entra-id.url = "github:vicondoa/nixos-entra-id";

  nixling.vms.<vm>.config.imports = [
    inputs.nixos-entra-id.nixosModules.default
  ];

  # Move each `nixling.vms.<vm>.entra-id.<key>` into the guest
  # config; see the nixos-entra-id README for the new schema.
  ```

  The `nixling.vms.<vm>.entra-id` attribute is kept as a hidden
  stub option so leftover assignments produce a readable
  assertion error (with migration instructions) instead of a
  cryptic "option does not exist" message from the module
  system. Final removal of the stub is tracked for v0.2.0.

- (W3b/Phase-2b) Three host-side activation scripts removed from
  `nixos-modules/host-activation.nix`:
  - **`nixlingSbctlBackup`** — moved maintainer-specific
    `*-backup.tar.gz` files from `$HOME` into `/var/lib/sbctl/backup/`.
    Not a framework concern. Consumers who relied on this should
    handle their own backup-file relocation outside nixling.
  - **`nixlingStoreChownRepair`** — one-shot repair for a past chown
    bug (an earlier `modules/nixling/store.nix` revision leaked
    `group=kvm` into `/nix/store` inodes via the per-VM hardlink
    farm). New installs are unaffected. Consumers upgrading from a
    pre-public nixling that ran with the buggy revision should run
    the historical repair script from `/etc/nixos` once and then
    drop the activation script there; the bug cannot recur on
    Phase-2b and later code.
  - **`nixlingMigrateState`** — one-shot renamer
    (`/var/lib/microvms/<vm>` → `/var/lib/nixling/vms/<vm>`, plus
    `/var/lib/swtpm/<vm>` → `vms/<vm>/swtpm/`). New installs land
    directly on the Phase-2a layout. Pre-public consumers should
    use the Phase 9 migration script (or perform the moves manually
    following the same logic) before switching to the public flake.

  These deletions remove all host-specific bias from the public
  framework's activation phase. The remaining two activation
  scripts (`nixlingVmStatePerms`, `nixlingNetVmVarImgPerms`,
  formerly `nixlingRouterVarImgPerms`) only adjust file ownership
  on per-VM disk images and contain no host-specific assumptions.

### Changed (W3b)

- **`nixling.vms.<vm>.ssh.keyPath` is NOT removed.** Earlier W3b
  phase-2b commit messages claimed otherwise; that was a mis-
  description of the change. The option still exists. What changed
  is its effective default: when left unset (`null`), the CLI now
  derives the SSH-key path from `nixling.site.keysDir` as
  `<keysDir>/<vm>_ed25519`, matching the framework-managed Ed25519
  key generated by `host-keys.nix` on every activation. Consumers
  who explicitly set a path still win; the option's `null` default
  just means "let the framework pick". This makes the
  framework-managed key the zero-config happy path while keeping
  the option-shape stable for consumers supplying their own keys
  (e.g. a hardware-backed Yubikey-resident key).

- (W3b/2026-05-19) Net VM `users.allowNoPasswordLogin` is set to
  `lib.mkDefault true`. Net VMs receive SSH keys via runtime
  injection (`nixling-load-host-keys.service` reads
  `<stateDir>/vms/<vm>/host-keys/` over virtiofs); they have no
  eval-time authorized_keys. Without the flag, NixOS module-eval
  fires the `users.allowNoPasswordLogin` assertion before runtime
  injection runs. Sealed-appliance consumers can override with
  `mkForce`.
- (W3b/2026-05-19) GPU sidecar (`nixling-<vm>-gpu.service`)
  hardening tightened: `NoNewPrivileges`, `ProtectSystem=strict`,
  `PrivateTmp`, `ProtectHome`, `DevicePolicy=closed` with a
  `/dev/kvm` + render-node allowlist, `RestrictAddressFamilies =
  [ AF_UNIX AF_NETLINK AF_VSOCK ]`,
  `SystemCallArchitectures=native`, narrow `ReadWritePaths`.
  Two omissions documented in source comments:
  `MemoryDenyWriteExecute` (crosvm GPU JIT triggers SIGSYS) and
  `AF_VSOCK` retained (cloud-hypervisor sd_notify path).
- (W3b/2026-05-19) IPv6 disabled on workload + net VM guest
  networkd (`LinkLocalAddressing=no`, `IPv6AcceptRA=false`); net
  VM nft rules DROP `ip6` forward. Net stack is IPv4-only by
  construction.
- (W3b/2026-05-19) Route preflight oneshot
  (`nixling-net-route-preflight.service`) now FAILS CLOSED on
  conflict — exit 1 on any env-vs-route mismatch instead of
  WARN+exit 0. `RemainAfterExit=true`, `Before=` each enabled
  nixling-managed VM unit, `RequiredBy=` each wrapper, so a stale
  host route blocks VM start until the operator clears it. (W3b
  H1 followup.)
- (W3b/2026-05-19) Inter-env CIDR overlap check now performs real
  IPv4 prefix arithmetic (`lib.cidrOverlaps` in
  `nixos-modules/lib.nix`) instead of exact-string equality.
  Containment (e.g. `10.0.0.0/16` ⊃ `10.0.1.0/24`) is rejected.
  Env-vs-`hostLanCidrs` is checked under the same helper. (W3b H3
  followup.)
- (W3b/2026-05-19) `nixling.site.yubikey.enable = false` actually
  gates the host-side udev rules + `usbip-host` kernel module.
  Previous phase-2b commit declared the option but never read it.
  (W3b H4 followup.)
- (W3b/2026-05-19) `nixling keys rotate <vm>` now scrubs the OLD
  pubkey from the guest's `~/.ssh/authorized_keys` (matched by
  SHA256 fingerprint) AFTER the new key is verified — rotation
  used to leave the old key authorized forever. Retention
  bounded: 3 most recent generations under
  `<keysDir>/old/<ts>/`; older are pruned post-rotation. Help
  text updated. (W3b H5 followup.)

### Added (W3b)

- **`nixling.site.*` public option surface.** Site-specific knobs
  extracted from previously-hardcoded references to the
  maintainer's host setup. Every option is opt-in; defaults give a
  fully headless framework with no Wayland integration. Public
  options:
  - `nixling.site.stateDir` — root of every nixling-managed state
    file (default `/var/lib/nixling`). **Advisory only in v0.1.0**
    (see option description); full threading lands in v0.2.0.
  - `nixling.site.keysDir` — directory for framework-managed
    per-VM SSH keys (default `${stateDir}/keys`). Same advisory
    caveat for v0.1.0.
  - `nixling.site.waylandUser` — primary Wayland user; required
    for any VM with `graphics.enable = true` or `audio.enable =
    true`.
  - `nixling.site.launcherUsers` — users added to the
    `nixling-launcher` group (polkit grant for VM start/stop).
  - `nixling.site.userAuthorizedKeys` — global authorized SSH
    keys merged into every VM at boot. Validated at eval time
    against an allowlist of supported key types; private-key
    markers rejected.
  - `nixling.site.yubikey.enable` — host-side Yubico udev rules +
    `usbip-host` kernel module. Default true.
  - `nixling.site.flakePath` — default flake reference for the
    `nixling` CLI's lifecycle subcommands (`build`, `switch`,
    `boot`, `test`). Nullable.
- **`nixling.vms.<vm>.userAuthorizedKeys`** — per-VM
  authorized SSH keys, merged with `site.userAuthorizedKeys`.
- **`nixling.audio.users`** — host-side option propagating an
  audio-group membership list into the guest. Default falls back
  to `[ vm.ssh.user ]` when unset.
- **Framework-managed per-VM SSH keys.** Activation
  (`nixos-modules/host-keys.nix`) generates an Ed25519 keypair
  per enabled VM under `<keysDir>/<vm>_ed25519`. Atomic via
  staging + `mv -T`; protected by `flock` on `<keysDir>/.lock`.
  The pubkey is staged under
  `<stateDir>/vms/<vm>/host-keys/host.pub` and injected into the
  guest at boot via virtiofs.
- **`nixling keys` CLI subcommands.**
  - `nixling keys list [--json]` — fingerprint + path + mtime
    per VM.
  - `nixling keys show <vm>` — print the pubkey.
  - `nixling keys rotate <vm>` — atomic rotate-and-verify with
    SHA256-fingerprint-based old-key scrub + 3-generation
    retention (see Changed entry above).
- **`nixling-load-host-keys.service`** (guest-side) — fail-closed
  service that reads `/run/nixling-host-keys/` and writes the
  union of `host.pub` + user-authorized-keys into the SSH user's
  `~/.ssh/authorized_keys`.
- **`scripts/migrate-nixling-v0.1.0.sh`** (W3b H6) — one-shot host
  migration script for consumers upgrading from a pre-public
  in-tree nixling layout. Preserves TPM state byte-for-byte.
  Has `--dry-run` and `--rollback`. Committed under
  `scripts/` so CI can shellcheck it.
- **`tests/smoke-eval.nix`** (W3b H9) — minimal consumer-style
  nixosSystem that imports `nixling.nixosModules.default` and
  exercises the eval graph end-to-end. Wired into
  `tests/static.sh` Layer-1.
- **`tests/assertions-eval.sh`** (W3b H10) — 8 regression tests
  exercising every eval-time invariant in the schema (CIDR shape,
  CIDR overlap, key validation, `waylandUser` presence, …).
- **`nixos-modules/lib.nix#cidrOverlaps`** — pure-Nix IPv4 prefix
  overlap helper used by network.nix assertions. Same file gains
  `parseCidr` as a public helper.

### Added

- (W0/2026-05-18) Initial flake skeleton with Apache-2.0 license,
  `x86_64-linux` + `aarch64-linux` eval, `microvm.nix` input, and
  reserved-but-inert `nixosModules.default`.
- (W1/2026-05-18) Mechanical lift of nixling modules from
  `/etc/nixos/modules/nixling/` into the public flake:
  - 9 core modules under `nixos-modules/` (`default`, `options`,
    `lib`, `host`, `network`, `base`, `store`, `cli`;
    `router.nix` renamed to `net.nix`);
  - 6 component modules under `nixos-modules/components/`
    (`graphics`, `tpm`, `usbip`, `home-manager`; `audio` split into
    `audio/{guest,host}.nix`);
  - Extracted pkgs: `spectrum-ch`, `vhost-device-sound`,
    `crosvm-patched`, `crosvm-seccomp`, `patches`;
  - 6 generic test scripts under `tests/`.
- (W2/2026-05-18) `systemd.services."nixling@"` wrapper template with
  explicit `ExecStart` / `ExecStop` / `PropagatesStopTo` (planning-round
  Critical #1 — `BindsTo` alone does not propagate stops).
- (W2/2026-05-18) Eval-time assertions for VM names
  (`^[a-z0-9][a-z0-9-]*$`, no `sys-` prefix, not `launcher`) and env
  names (≤ 8 chars).
- (W2/2026-05-18) `nixos-modules/assertions.nix` as a dedicated
  assertions module.
- (W2-followup/2026-05-18) Top-level `manifestVersion = 0` stub field
  in the per-VM JSON manifest (Phase 5 bumps to 1). Stashed under
  the reserved `_manifest` sentinel key; user-declared VM names
  cannot start with `_` per the W2-followup H1 stricter regex.

### Changed

- (W2/2026-05-18) **BREAKING.** Option namespace renamed:
  - `nixling.networks.<env>` → `nixling.envs.<env>`;
  - `nixling.networks.<env>.routerName` →
    `nixling.envs.<env>.netName`;
  - `nixling.networks.<env>.extraRouterConfig` →
    `nixling.envs.<env>.extraNetConfig`.
- (W2/2026-05-18) **BREAKING.** Per-env auto-declared VM renamed:
  `<env>-router` → `sys-<env>-net`.
- (W2/2026-05-18) **BREAKING.** Systemd unit naming convention:
  - `swtpm@<vm>` → `nixling-<vm>-swtpm`;
  - `nixling-snd@<vm>` → `nixling-<vm>-snd`;
  - `nixling-gpu-<vm>` → `nixling-<vm>-gpu`;
  - `nixling-store-sync@<vm>` → `nixling-<vm>-store-sync`;
  - `usbipd-nixling` → `nixling-sys-usbipd`;
  - `usbipd-nixling-<env>` → `nixling-sys-<env>-usbipd-proxy`.
- (W2/2026-05-18) **BREAKING.** System users/groups renamed:
  `nixling-gpu-<vm>` → `nixling-<vm>-gpu`, `nixling-snd-<vm>` →
  `nixling-<vm>-snd`, `swtpm-<vm>` → `nixling-<vm>-swtpm`.
- (W2/2026-05-18) **BREAKING.** State-dir layout:
  - `<stateDir>/<vm>/` → `<stateDir>/vms/<vm>/`;
  - `<stateDir>/<env>-router/` → `<stateDir>/vms/sys-<env>-net/`;
  - `<stateDir>/swtpm/<vm>/` → `<stateDir>/vms/<vm>/swtpm/`;
  - `/run/nixling-snd/<vm>/snd.sock` →
    `/run/nixling/vms/<vm>/snd.sock`.
- (W2/2026-05-18) **BREAKING.** Manifest JSON contract: `isRouter` →
  `isNetVm`, `routerVm` → `netVm`. Top-level `manifestVersion = 0`
  added in W2-followup; Phase 5 will bump.
- (W2-followup/2026-05-18) **BREAKING.** VM/env name regex tightened
  from `^[a-z0-9][a-z0-9-]*$` to `^[a-z][a-z0-9-]*$` (require leading
  letter). Matches systemd-escape best practices; avoids ambiguity
  with tooling that treats a leading digit as a numeric index
  (`ip link show 42web-l10`). No existing in-tree names trip the
  stricter rule; consumers with numeric-prefixed VM/env names must
  rename.
- (W2-followup/2026-05-18) CLI: `nixling up/down/status` now target
  `nixling@<vm>.service` (the user-facing wrapper) instead of
  `microvm@<vm>.service` directly. Lifecycle propagates via the
  wrapper's BindsTo / ExecStop. Diagnostic flows
  (`status --verbose`, `journalctl` examples) keep their
  `microvm@<vm>` references but label them "backend" /
  "implementation detail".
- (W2-followup/2026-05-18) CLI: `nixling list` / `nixling status`
  output tag for system VMs changed from `(router)` to `(net-vm)`.
  Helper renames: `ensure_router_up` → `ensure_net_vm_up`,
  `router_active` → `net_vm_active`, `IS_ROUTER` → `IS_NET_VM`.
  User-facing prose `router` / `router VM` → `net` / `net VM` (kept
  `routing/routes` only where describing the network function).
- (W2-followup/2026-05-18) `nixling-launcher` polkit grant tightened
  to an exact-unit allowlist generated at NixOS eval time from
  `cfg.vms` + `cfg.envs`, restricted to `start` / `stop` / `restart`
  verbs only. Drops the bare `microvm@*` prefix wildcard; default-
  deny invariant restored. Recovery / debugging paths can still
  authenticate via sudo or polkit-prompt.

### Notes

- Pre-v0.1.0 — breaking changes do not get a deprecation period.
  There is no compat shim for the old `nixling.networks` namespace
  or for any of the renamed unit / user / state-dir identifiers.
- The first tagged release is `v1.0.0` (see CHANGELOG below). The
  v0.x line never tagged a public release; everything below was the
  in-flight roadmap during the development branch and is preserved as
  historical record of how the architecture got to v1.0.
- v1.0.0 ships in lockstep with
  [`vicondoa/nixos-entra-id`][nixos-entra-id] v1.0.0; consumers
  using both should pin matching tags.

[nixos-entra-id]: https://github.com/vicondoa/nixos-entra-id
