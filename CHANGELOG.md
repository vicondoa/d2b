# Changelog

All notable changes to nixling are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Pre-1.0 minor releases may break public APIs. When practical,
deprecations ship one minor release before removal.

## [Unreleased]

### Added

- Bundle: the private manifest bundle now emits `storage.json` and
  `sync.json` contracts for managed paths, process restart/adoption
  policies, degraded-state taxonomy, and lock/lease synchronization
  policy.
- Broker: added bundle-resolved `ReconcileStorageScope` and
  `ValidateLockSpec` operations so storage and synchronization contracts
  can be inspected and, for static directory specs, reconciled without
  daemon-supplied raw paths.
- Daemon: startup now performs a read-only storage/restart/sync contract
  check and persists `storage-lifecycle-report.json` for degraded-state
  adoption work and future doctor/status UX.
- CLI: `nixling host doctor --read-only` now surfaces
  `storage-lifecycle-report.json` with bounded issue kinds and inline
  remediation for storage/restart/sync contract drift.
- CLI: `nixling host doctor --read-only` now treats the private broker
  socket, optional metrics endpoint absence, and current swtpm namespace
  posture as healthy when those surfaces match the deployed policy.
- CLI: added `nixling host migrate-storage --dry-run`, which emits a
  checkpoint ID, exact rollback command, preserved-data inventory,
  cutover-only cleanup candidates, and fail-closed hazards for the
  planned storage layout migration.
- Tests: added storage lifecycle report schema and serialization
  regression coverage so doctor/status consumers see the same camelCase
  contract that the daemon writes.
- Documentation: ADR 0034 and the storage lifecycle explanation now
  define the planned generated contracts for managed paths, process
  restart/adoption, synchronization, lock ownership, degraded-state
  reporting, and the one-time storage cutover.
- ADR 0032 gateway lifecycle: gateway-mode `nixling vm
  start/stop/restart <aca target>` now routes through lifecycle
  operations backed by the ACA preview REST data plane. Gateway config
  can declare non-secret ACA subscription/resource-group/sandbox-group/
  region/image coordinates, and the provider creates/reuses disk images
  and sandboxes by nixling workload labels instead of shelling out to the
  preview `aca` CLI.
- ADR 0032 ACA display: gateway config can now carry the non-secret ACA
  managed-identity client id used by local validation probes, while the
  live display sender receives a gateway-minted short-lived Relay Send
  bearer instead of the long-lived Relay rule key.
- NixOS: added `nixling.site.usePrebuiltHostTools` so development hosts
  can validate source-built `nixling`, `nixlingd`, and activation helper
  binaries before matching release prebuilts exist.
- CI: merging `main` after cutting a new dated changelog section now
  auto-tags the release and publishes pre-built `x86_64-linux` host
  binary tarballs for `nixlingd`, `nixling`, `nixling-priv-broker`,
  `nixling-wayland-filter`, and `nixling-activation-helper`, alongside
  `SHA256SUMS`, on the matching GitHub Release.
- CI: after publishing a GitHub Release, the release workflow now
  computes Nix SRI hashes for each tarball, writes `nix/prebuilt.json`,
  and auto-commits the manifest back to `main` so consuming flakes can
  fetch the published host binaries by hash without manual updates.
- `nixling.vms.<vm>.qemuMedia.window.niriBorderColor` lets
  qemu-media host QEMU windows use a VM-specific niri border color.
  qemu-media windows now route through the nixling Wayland filter proxy
  so the generated niri include can match the VM-prefixed app-id
  `nixling.<vm>.*`.
- `qemuMedia` image-file sources can now be declared directly with an
  absolute `path` and `format = "raw"`; physical USB sources continue to
  use opaque refs plus `nixling usb enroll`.

### Changed

- **Breaking:** VMs with `nixling.vms.<vm>.usbip.yubikey = true` must
  now also enable `nixling.vms.<vm>.guest.control.enable = true`. USBIP
  guest attach/detach is owned by guestd over authenticated
  guest-control; there is no SSH fallback.
- Broker: `nixling-priv-broker.service` now defaults `RUST_LOG` to
  `info` instead of `debug`, keeping high-volume broker diagnostics out
  of normal journal/OTel log exports unless an operator opts into debug
  logging.
- CI: the PR aarch64 flake leg now runs only the lightweight
  `smoke-eval-aarch64.nix` check instead of the full native aarch64
  flake sweep.
- NixOS module: `nixling.site.usePrebuiltHostTools = false` now also
  forces `nixling-priv-broker` to build from the local source checkout,
  keeping the broker wire/bundle parser aligned with `nixlingd`.
- `bundleVersion` 5 → 6: adds the private storage lifecycle and
  synchronization artifacts to the trusted bundle.
- CI: pull requests now fail closed when Rust/Nix/Cargo changes do not
  update `CHANGELOG.md`, or when the changelog is missing
  `## [Unreleased]`, uses duplicate/out-of-order version headers, or
  carries non-semver versions / non-ISO release dates.
- `manifestVersion` 5 → 6: per-VM entries now carry runtime/provider
  metadata and provider capability summaries. Provider-specific socket
  and vsock fields are nullable so `qemu-media` entries do not fabricate
  Cloud Hypervisor or guest-control artifacts.
- ADR 0032 constellation core contracts now publish and document hardened
  schema roots: target and identifier parsing is bounded and
  fail-closed, capabilities are positive assertions, mutating operations
  require idempotency keys, and audit/error/trace payloads carry only
  redacted, bounded metadata. This is a contributor-facing contract; it
  does not change CLI, daemon, or host behavior.
- `qemu-media` VMs now emit a typed QMP-only QEMU runner process node
  instead of being absent from `processes.json`; they still do not emit
  Cloud Hypervisor, store/virtiofs, or guest-control runner data.

### Fixed

- ADR 0032 ACA display: the daemon-owned verified Relay listener now
  survives the synchronous `gatewayDisplay` request runtime, so Waypipe
  sessions remain connected after `nixling vm exec <aca target>` returns
  and the forwarded Wayland app can stay visible on the host compositor.
- USBIP attach/detach now routes guest-side import cleanup through guestd
  over authenticated guest-control, removing the CLI's SSH fallback and
  preventing stale guest imports from blocking reattach after daemon restarts.
- `qemu-media` VM start no longer runs NixOS-only state ownership and
  SSH host-key preflights, allowing externally booted media VMs to use
  the qemu-media runner-owned state directory prepared by host
  reconciliation. Physical USB boot media now attaches through QMP's
  `host_device` block driver instead of the regular-file-only `file`
  driver. qemu-media runners now pass explicit QEMU memory and vCPU
  sizing, defaulting to 4 GiB and 2 vCPUs instead of QEMU's tiny
  built-in RAM default. qemu-media guest RAM now uses a non-dumpable,
  non-KSM memory backend by default, and `qemuMedia.security.lockMemory`
  can fail closed with QEMU `mem-lock` when the host cannot keep guest RAM
  out of swap.
- Documentation: sanitized ADR 0032's ACA/Relay live-proof record so the
  architectural validation summary remains without publishing live sandbox,
  disk-image, command-output, or compositor-window identifiers.

## [1.3.1] - 2026-06-18

### Fixed

- Nix packaging now keeps legitimate source files whose names contain
  `target` (for example `nixling-constellation-core/src/target.rs`) while
  still filtering Cargo `target/` build directories out of package sources.
- USBIP lock acquire is now idempotent for the same VM: when a VM is
  restarted (`nixling down` + `nixling up`), the broker no longer
  refuses to re-bind a busid that the same VM already owns. Previously,
  every VM restart required a manual `nixling usb detach` + `nixling usb
  attach` cycle because the lock file persisted across the stop/start.
## [1.3.0] - 2026-06-18

### Fixed

- `tpm.enable` first-run: enabling TPM on a VM with no pre-existing
  `/var/lib/nixling/vms/<vm>/swtpm` state directory no longer wedges
  the VM's start. The privileged broker now provisions the per-VM
  swtpm state directory (owner `nixling-<vm>-swtpm`, mode `0700`) on
  first start, so swtpm no longer dies with a fatal NVRAM `ENOENT`.
  The documented manual `install -d … swtpm` workaround is no longer
  needed.
- A required per-VM runner that exits during VM start (e.g. swtpm)
  now fails the start fast with a typed, actionable error instead of
  blocking the daemon for the full readiness budget (~300 s). The
  swtpm control-socket readiness now waits for an active listener
  rather than the bare socket inode.
- The daemon handles client connections concurrently (bounded), so a
  slow or failing VM start no longer stalls unrelated clients
  (e.g. host status feeds). Mutating lifecycle operations are
  serialized per-VM/globally; read-only requests run in parallel.
- Per-VM NixOS evaluations now inherit the host's `nixpkgs.overlays` in
  addition to `nixpkgs.config`, so consumer security overlays patch VM
  closures as well as host closures.

### Security

- The per-VM state root `/var/lib/nixling/vms/<vm>/` is now `3770`
  (setgid **+ sticky**) so a non-owner per-VM role UID cannot rename
  or replace the principal-owned `swtpm` NVRAM directory. The swtpm
  state directory's inherited ACLs are cleared to owner-only `0700`
  on provisioning.
- TPM state-loss is fail-closed: a previously-provisioned swtpm state
  directory that goes missing or is replaced fails the VM start with
  `previously-provisioned-swtpm-state-missing` (bound to the
  directory identity via a root-owned marker outside the
  role-writable tree) rather than silently re-creating an empty TPM.

### Changed

- `bundleVersion` 4 → 5: adds the audited `PrepareSwtpmDir` broker
  operation for per-VM swtpm state-directory provisioning.

- CI: the `pr-l1-static-fast` x86_64 flake check is now sharded one job per
  flake check via a dynamic matrix (`make test-flake-list` enumerates the
  names; each shard runs `make test-flake` with `NL_FLAKE_CHECK=<name>` to
  instantiate a single check in its own evaluator process). This replaces the
  monolithic `nix flake check`, which evaluated every nixosSystem toplevel in
  one process and OOM-killed the 16 GB runner (kept alive only by a 14 GB
  swapfile, ~41 min). A companion `flake-eval-x86-outputs` job evaluates the
  non-`checks` x86 outputs (`packages.*`, via `NL_FLAKE_OUTPUTS=1`) that the
  per-check shards don't cover and the aarch64 leg (which only evaluates aarch64
  outputs) would miss. A stable `test-flake-x86` aggregator job gates on all of
  them to preserve the required status context, and a fail-closed drift gate
  (`tests/unit/gates/flake-check-matrix-sync.sh`, run by `make test-drift`,
  regenerate the pin with `make flake-matrix-pin`) keeps the CI shard matrix in
  sync with the flake's check set. The aarch64 leg still runs the full
  monolithic check.
- CI: the `test-rust` gate now restores/saves an sccache **local-disk** cache
  via `actions/cache` (opt-in through the new `NL_CI_SCCACHE=1`, honored by
  `tests/test-rust.sh`; the pinned `sccache` is put on `PATH` since hosted
  runners ship rustup and skip the nix-shell that would otherwise supply it).
  We deliberately avoid sccache's native GitHub Actions backend: it exports
  `ACTIONS_RUNTIME_TOKEN` into the job shell environment, where the untrusted
  crate code the gate compiles and runs could read and exfiltrate it;
  `actions/cache` keeps that token inside its own action process. The broker's
  per-feature-pass target dirs are now deterministic siblings (not `mktemp`) so
  `CARGO_TARGET_DIR`, which sccache hashes, doesn't churn the cache key.

### Removed

- CI: deleted the redundant `pr-cargo-workspace` workflow, which re-ran
  `make test-rust` + `make test-proofs` already covered by `pr-l1-static-fast`'s
  `test-rust`/`test-proofs` jobs. Its `ci-uses-make` allowlist entry is removed
  too, and `cargo-ubuntu` is dropped from `main`'s required status checks.

### Added

- Internal v2 constellation provider-abstraction crates
  ([ADR 0032](docs/adr/0032-nixling-v2-constellation-control-plane.md)),
  with **no user-facing behavior change**: `nixling-constellation-core`
  (the pure, codec-neutral model — strongly-typed identifiers with
  fail-closed deserialization, the capability model, the semantic
  `ConstellationFrame` with a trusted per-operation required-capability
  mapping, the redacted audit envelope, and a bounded trace context) and
  `nixling-constellation-provider` (the async provider trait surface —
  runtime/workload/display/transport/stream-mux/codec/credential/
  daemon-access providers — with typed capability descriptors, structured
  capability-denial errors, byte-carrying transport sessions, and
  fail-closed mock/conformance fixtures). The same change adds the
  remaining foundation crates: `nixling-constellation-codec-protobuf`
  (a `prost` codec behind the `ProtocolCodec` trait, with frame-cap and
  fail-closed decode validation), `nixling-constellation-transport`
  (an in-memory loopback transport for conformance),
  `nixling-constellation-router` (the codec-neutral operation router +
  single-owner idempotency/dedup store keyed by the full operation
  namespace), `nixling-daemon-access` (the transport-neutral CLI↔daemon
  semantic API with its current local-Unix binding), `nixling-host-providers`
  (byte-identical local adapters over the existing Cloud Hypervisor and
  cross-domain Wayland argv generators), plus compile-only constellation
  peer-module skeletons inside `nixlingd`. These crates are the foundation
  for later ADR 0032 work; they do not change any CLI, daemon, or
  on-host behavior.

- Documentation for the v2 constellation control plane
  ([ADR 0032](docs/adr/0032-nixling-v2-constellation-control-plane.md)):
  the threat model in `docs/explanation/design.md` now describes the
  realm-gateway trust boundary — the host daemon and broker hold no
  realm relay/provider credentials, remote node registries, or realm
  audit (those live inside a per-realm gateway guest VM); a realm relay
  is an untrusted, ciphertext-only rendezvous transport; relay identity
  is never local authorization (`SO_PEERCRED` + the `nixling` group
  remain the only local lifecycle authz surface); and work and personal
  realms never share a gateway guest or an L2 bridge. `SECURITY.md`,
  `docs/reference/privileges.md`, `docs/reference/daemon-api.md`, and
  `docs/reference/daemon-audit-check.md` are updated to state the same
  relay-is-not-local-auth and no-host-held-realm-credentials boundary.

- Host OTel collector parity (ADR 0033). New
  `nixling.observability.host.*` options bring the host edge collector to
  parity with the per-VM guest collector: `host.scrapeJournal` adds a host
  `journald` receiver (severity-mapped, restart-resuming `file_storage`
  cursor) and `host.otlpIngest.enable` adds a host-local OTLP ingest
  endpoint (a Unix socket in a dedicated `/run/nixling/otel/ingest/`
  subdirectory, isolated from `host-egress.sock`) plus a `traces` pipeline
  and `otlp` on the `metrics`/`logs` pipelines. Both default off and ship
  over the existing host → `sys-obs` vsock bridge (never a LAN).
  `host.otlpIngest.clientGroup` optionally widens the ingest socket from
  `0600` to a `0660` group. See
  [ADR 0033](docs/adr/0033-host-collector-parity.md).

### Changed

- All Rust workspaces (main + `nixling-priv-broker`) moved to **Rust
  edition 2024**; the pinned toolchain remains 1.94.1 and `unsafe_code`
  stays `forbid` (no `unsafe` was introduced for the migration).

- `deployment.environment` is now machine-and-env aware: the central
  collector stamps it `<hostName>` for host telemetry and
  `<hostName>-<env>` for workload VMs (e.g. `ddbus`, `ddbus-work`,
  `ddbus-personal`), instead of the bare host name for everything.
  `host.name` remains the per-source name (the host's name for host
  telemetry, the VM's name for workloads). See
  [ADR 0033](docs/adr/0033-host-collector-parity.md).

- Host-origin telemetry now carries the **hostname** as `vm.name` /
  `host.name` (via `nixling.observability.host.identityName`, default
  `networking.hostName`), assigned at the trusted ingress boundary, rather
  than the literal `"host"`. `vm.role` stays `"host"`. This is a default
  label change for observability-enabled hosts even with the new receivers
  off; set `nixling.observability.host.identityName = "host"` to keep the
  old labels. See [ADR 0033](docs/adr/0033-host-collector-parity.md).

- `ReadGuestFile` guest-control RPC: a single-shot, bounded, enum-keyed
  (initially `GuestConfig`-only) RPC for the host to read a small,
  trusted in-guest file over the authenticated vsock channel.
  `nixling-guestd` resolves the path with a safe `openat` from a trusted
  directory fd (`O_RDONLY | O_CLOEXEC | O_NOFOLLOW`, rejecting symlinks /
  `..` / non-regular files) and enforces a size cap before allocating;
  the response is bounded below both the ttRPC and `public.sock` frame
  budgets. The capability is negotiated as
  `GuestCapability::ReadGuestFile`, and an authenticated guest that does
  not advertise it fails closed. File-specific typed errors
  (`FileNotFound` / `FileTooLarge` / `PathUnsafe` / `ReadDenied`) carry
  operator-actionable remediations rather than a blind retry. The
  guest-control protocol version is bumped accordingly. See
  [ADR 0029](docs/adr/0029-framework-ssh-to-typed-guest-rpc.md).

- Production guest-control transport bridge: the host daemon now drives
  the authenticated vsock channel to guest-control VMs end-to-end. A
  broker-backed signer forwards each guest-control sign request verbatim
  to the privileged broker over a timeout-bounded dispatch, and a probe
  orchestrator resolves the per-VM vsock socket and peer credentials from
  the trusted bundle, connects to the host CID, and runs the
  authenticated Hello / Authenticate / Health handshake on a dedicated
  runtime with per-attempt timeouts. Spawning a guest-control VM's
  cloud-hypervisor runner now grants the unprivileged daemon a minimal,
  single-uid ACL (`--x` traversal on the per-VM state dir, `rw` on
  `vsock.sock`) scoped to the current socket inode. Because there is no
  CH-stop teardown hook carrying the socket path, the ACL is refreshed as
  a revoke-then-grant on each cloud-hypervisor (re)spawn — any stale grant
  left on a replaced or disabled socket inode is revoked before the live
  grant, so a prior generation cannot retain access (stop-time teardown is
  future work). Both the revoke and grant are audited with hash-only
  fields (no raw paths).

- New admin-only `public.sock` verb `ReadGuestConfig { vm }`: returns the
  editable guest config working copy of a guest-control VM as a bounded
  base64 string over the authenticated bridge. The daemon enforces the
  admin role before any probe / sign / read, recomputes size and sha256
  from the received bytes (never trusting guest-reported values), and
  bounds the encoded payload below both the ttRPC and `public.sock`
  frames.

  `tty=true && detach=false` now routes to a PTY-backed,
  connection-owned, non-durable attached exec. PTY setup keeps
  `unsafe_code = "forbid"` via a helper-exec pattern — a new `--tty-exec`
  mode of the static `nixling-exec-runner` performs the
  `setsid` + `TIOCSCTTY` + `tcsetwinsize` + `dup2` + `execve` handshake in
  safe `rustix`, so `nixling-guestd` never acquires a controlling
  terminal. stdout/stderr are merged onto the stdout stream
  (`ReadOutput(stderr)` returns a typed stderr-unavailable error);
  `CloseStdin` injects VEOF (`0x04`) and keeps the master open;
  `TtyWinResize` and `ExecSignal` are serialized through the per-exec
  control sequence, with signals restricted to the foreground process
  group (resolved via `tcgetpgrp` at delivery) and the
  `INT/TERM/HUP/QUIT/WINCH/USR1/USR2/KILL/TSTP/CONT` allowlist. An absent
  `initial_terminal_size` defaults to 24×80. Interactive sessions run
  indefinitely by default; teardown drops the master (SIGHUP), waits a
  bounded grace, then SIGKILLs the whole TTY session (in-session
  no-orphan; a `setsid`/double-fork escapee is a documented trusted-root
  limitation). Interactive detached exec remains unsupported; use
  non-TTY `nixling vm exec -d` for detached commands. See
  [`docs/reference/guest-control-exec-interactive-tty.md`](docs/reference/guest-control-exec-interactive-tty.md)
  and the interactive-exec section of
  [ADR 0028](docs/adr/0028-guest-control-plane-over-vsock.md). The
  guest-control wire contract is unchanged (the TTY surface was already
  present).

- New per-VM option `nixling.vms.<vm>.guest.exec.interactiveMaxRuntimeSec`
  (default `0` = unlimited) caps interactive TTY exec runtime
  independently of the non-interactive attached ceiling. It is mirrored
  read-only into the guest config and forced from the host module, and
  emitted to `nixling-guestd` as `--interactive-max-runtime-sec`
  alongside the detached exec surface.

- Guest exec now accepts bare command names and relative program paths in
  both attached and detached modes. `guestd` passes `argv[0]` through the
  workload user's login shell (`exec "$@"`), so the command is resolved
  by that user's login `PATH`; invalid program names get the distinct
  `INVALID_PROGRAM` / `guest-control-invalid-program` error. The
  console replacement is `nixling vm exec -it <vm> -- bash`.

- Detached workload-user exec is enabled with
  `nixling vm exec -d <vm> -- <cmd>` and VM-first management verbs:
  `nixling vm exec <vm> list`, `logs <exec_id>`, `status <exec_id>`,
  and `kill <exec_id>`. Detached jobs are non-TTY, run as the workload
  user (never root), stay inside guestd rather than adding a broker op,
  and survive host client disconnect. Retained stdout/stderr use bounded
  ring buffers with dropped/truncated accounting and per-stream offsets;
  `kill` maps to idempotent two-phase `ExecCancel` (graceful terminate,
  bounded grace, force kill). Guestd reconciles detached runner/workload
  units at startup, cleans orphaned workloads, and reaps terminal records.

### Fixed

- The OTel host-bridge runner (`socat UNIX-LISTEN:host-egress.sock,...`)
  now self-heals across obs-VM restarts. `socat` does not unlink a
  pre-existing socket path before binding, so a stale `host-egress.sock`
  left by a previously-drained bridge made the freshly-spawned bridge
  exit immediately ("address in use"); the readiness probe only checks
  the socket *file* exists, so the failure was masked and host telemetry
  silently stopped reaching `sys-obs`. The broker now drops a
  provably-stale (non-listening) `host-egress.sock` before each
  `OtelHostBridge` spawn — mirroring the existing cloud-hypervisor / video
  socket preflight — so restarting the obs VM no longer wedges the host
  telemetry path. A live listener is never removed, and only sockets under
  `/run/nixling/otel/` are eligible.

- The privileged broker now compiles under the `layer1-bootstrap`
  feature (and thus `--all-features`): the guest-control `GuestControlSign`
  audit-redaction arm in `request_fields_value` is gated to the real-wire
  build, since under `layer1-bootstrap` `BrokerRequest` aliases to the
  bootstrap `BootstrapCall`, which has no such variant. The `Read` and
  `FileTypeExt` imports it uses are gated the same way so the bootstrap
  build stays warning-clean.

- The broker's non-socket-activated (test-mode / legacy) self-bind path
  now constrains the creation umask so the socket is materialized at
  `0o660` directly. `fchmod()` on an `AF_UNIX` socket fd does not change
  the bound path's mode on some kernels/filesystems, so the prior
  `fchmod`-only approach could leave the socket world-traversable
  (`0o755`). Production is unaffected (it uses socket activation, where
  systemd owns the socket mode).

- Guest-control chunked stdio docs now account for protobuf `bytes`
  allocation before handler entry by specifying ttRPC receive caps,
  bounded post-decode byte semaphores, and per-exec stdin permits for
  malicious concurrent `WriteStdin` fan-in.

- TPM-enabled guests now flush stale loaded/saved TPM sessions during
  early boot before SRK provisioning. This prevents swtpm session-handle
  exhaustion from breaking TPM-bound credentials while preserving NVRAM
  and persistent handles.

- Detached exec (`nixling vm exec -d`) now works end-to-end. Three faults in
  its initial implementation are fixed: the per-VM exec runner verified the
  workload's cgroup placement against a top-level `nixling-exec.slice` path
  even though systemd nests it under `nixling.slice`, so every detached
  command was killed at spawn; the daemon panicked (taking down `nixlingd`)
  when a detached management verb (`list`/`logs`/`status`/`kill`) was
  dispatched, because it built a nested async runtime on the request thread;
  and the guest reconciler matched a running workload's command against
  `systemctl show` output using exact, quote-aware argv tokens, but systemd
  renders `ExecStart` argv as a literal, unescaped, space-joined string — so
  live jobs (and any command containing a space, quote, backslash, or
  semicolon) were misclassified as foreign and reaped as `lost-guestd`
  shortly after starting. Workload identity is now matched against systemd's
  raw rendering, and a failed runner-side spawn verification logs an
  actionable guest-journal diagnostic. (Detached command arguments may not
  contain a newline or carriage return, which `systemctl show` cannot render
  on one line; such commands are now rejected at create as an invalid argument
  rather than starting and then being reaped.)

### Added

- `nixling vm exec <vm> -- <cmd…>` (and `-it` for an interactive TTY):
  an admin-only operator command that runs a command inside a running
  guest over the authenticated guest-control transport — CLI → daemon
  `public.sock` → authenticated guest-control vsock → `guestd` exec
  RPCs. There is no SSH and no host PTY (the guest owns the PTY); the
  host only flips termios via an RAII raw-mode guard restored on every
  exit, error, disconnect, or panic. Non-interactive mode streams
  stdout/stderr separately; `-it` allocates a guest PTY, merges stderr
  into stdout, and forwards `SIGWINCH`/`SIGINT`/`SIGQUIT`/`SIGHUP`/
  `SIGTERM`/`SIGTSTP` to the guest foreground process group (signal
  handlers enqueue only). The daemon holds an in-process exec session
  table whose per-session workers own one persistent authenticated
  guest-control client with fresh per-op deadlines; session-table caps
  (global / per-UID / per-VM) and `Start` rate limiting are enforced
  before connect/auth, and an old or non-guest-control generation fails
  closed with exit `70` (no proxy, no SSH fallback). Guest exit status
  passes through unchanged (`128+N` for signal death); transport, auth,
  capacity, protocol, old-generation, and internal failures map to
  reserved CLI exit codes that `--json` disambiguates from a guest exit
  code via `source`/`reason`/`guestExitCode`/`transportExitCode`. `-it`
  is human-only and is rejected together with `--json`; non-interactive
  detached commands use `nixling vm exec -d <vm> -- <cmd>`. Attached exec
  establishes one redacted kind=critical audit event (vm / peer uid / tty
  only), and detached create/kill adds redacted daemon audit carrying only
  vm / peer uid / result / exec id. Opaque session handles, argv, and
  stdio/env/cwd/paths never reach any log, span, audit record, or metric
  label.

- Detached guest exec: `ExecCreate(detach=true)` runs a non-interactive
  command that outlives the originating connection, supervised by the root
  guest daemon through slot-based `systemd-run` transient units
  (`nixling-exec-<NN>.service`, scoped to a guest-internal `nixling-exec`
  slice). Unit names and argv carry only the slot index — never the exec id,
  argv, environment, or cwd. stdout/stderr are retained in slot-keyed files
  under a root-owned, 0700, boot-scoped `/run/nixling-exec` parent with
  drop-oldest truncation accounting: 4 MiB per stream, an exact 256 MiB
  VM-global quota (32 retained slots × 2 streams × 4 MiB), and 8 active
  execs per VM. Detached execs run indefinitely by default
  (`guest.exec.detachedMaxRuntimeSec = 0`), with an optional per-VM runtime
  ceiling. Cancellation is a two-phase, control-file mechanism with no
  in-process signal handler. Terminal records are retained for 30 minutes
  then garbage-collected; a running detached job is never reaped. guestd
  re-adopts live detached execs across a guestd restart within one boot,
  reconciles valid runner/workload units before advertising detached
  capability, and cleans orphaned workloads. The operator CLI exposes the
  substrate as `nixling vm exec -d <vm> -- <cmd>` plus
  `nixling vm exec <vm> list|logs|status|kill` management verbs.

- `ExecList` RPC (guest-control protocol version 2): a minimal, read-only
  discovery call that enumerates the caller's detached execs for the same
  VM token + boot (bounded ≤32). Each entry carries the exec id, slot,
  state, create time, an argv SHA-256 hash (never raw argv), and per-stream
  truncation/dropped-byte counters. The CLI and public daemon DTOs do not
  expose the argv hash.

- `ExecExpired` guest-control error kind, distinguishing a retention-evicted
  detached record from `StaleSession` (boot mismatch) and `ExecNotFound`
  (unknown id).

- Host VM option `nixling.vms.<vm>.guest.exec.detachedMaxRuntimeSec`
  (unsigned, default 0 = indefinite) plumbed through to the guest exec
  runtime as a per-exec `RuntimeMaxSec` backstop when non-zero.


  `packages/nixling-ipc/proto/guest_control.proto` — generated schema plus
  protobuf source for the ADR 0028 ttRPC contract, covering health, Hello,
  capabilities, exec lifecycle, chunked stdio RPC shapes, bounded health
  labels, bounded string identifiers/payload metadata, oneof-style terminal
  status, structured stdio error results, and descriptor-shape drift checks.

- Initial guest-side Rust crates for the guest control plane:
  `nixling-guestd`, `nixling-userd`, and `nixling-exec-runner`, with
  fail-closed binaries, fakeable daemon/user/session traits, and bounded
  runner input validation.

- Bootstrap/fail-closed guest-static package outputs `nixling-guestd-static`,
  `nixling-userd-static`, and `nixling-exec-runner-static`, plus an ELF check
  proving the guest binaries have no interpreter or dynamic dependencies.
  Guest VM evals now consume these static outputs through the guest-control
  module, with a static-fast eval gate proving the package references.

- Opt-in guest-control auth token delivery wiring: per-VM runtime token path
  option, framework-owned materialized token file, read-only guest credential
  share, and guestd `LoadCredential` wiring with eval coverage.

- Host-owned Cloud Hypervisor vsock allocation now uses the manifest's
  base socket path for every VM, reserves distinct CIDs for env net VMs and
  workload VMs, and rejects consumer `--vsock` overrides so observability and
  guest-control port reservations share one authoritative per-VM vsock device.
  This bumps the public manifest to `manifestVersion = 5` because the existing
  `observability.vsockCid` / `observability.vsockHostSocket` fields now define
  the base Cloud Hypervisor vsock device. (`5` unifies this base-vsock change
  with the SigNoz observability metadata that landed as `4` on a sibling
  branch; the shipped parser/daemon/broker accept only `5`.)

- `nixlingd` now has an internal Cloud Hypervisor CONNECT helper for the
  guest-control transport port. This is transport groundwork only: it does not
  change VM readiness, status output, CLI help, or exec behavior.

- `packages/nixling-ipc/src/generated/guest_control.rs` now contains committed
  protobuf message bindings generated from
  `packages/nixling-ipc/proto/guest_control.proto` via
  `cargo run --locked --manifest-path packages/Cargo.toml -p xtask -- gen-guest-proto`.
  The new
  `tests/guest-proto-bindings.sh` gate verifies the generated bindings are
  deterministic, unsafe-free, and message-only (no ttRPC runtime stubs).

- Guest-control protobuf now has an authenticated `Authenticate` handshake:
  `Hello` is challenge-only, authenticated health/capabilities are returned
  only after proof-of-possession, and `nixling-guestd` has a pure auth core
  with fixed-size HMAC transcript tests. No listener, readiness, or exec CLI
  behavior is enabled yet.

- `nixling-guestd` now owns generated ttRPC service bindings and a dormant
  `--serve --vm-id <vm>` service mode for Hello challenge, Authenticate, and
  authenticated Health/Capabilities. The guest service remains opt-in manual-start only
  (`wantedBy = []`) and does not enable host readiness or exec behavior.

- The privileged broker now exposes a structured guest-control HMAC signer, and
  `nixlingd` has a host-side authenticated Health probe helper. The helper
  produces daemon-local health evidence only; it does not replace SSH readiness
  or enable exec.

- Guest exec policy option `nixling.vms.<vm>.guest.exec.enable` gates guest
  exec (off by default). This is dormant policy wiring only; no exec
  runtime/CLI behavior is enabled by this option yet.

- Guest-control retained-log security requirements and canary-based
  redaction test coverage for stdout/stderr logs, telemetry, health, and
  CLI JSON.

- `proofs/chunked-stdio-conformance` — executable safe-Rust proof for
  the selected Kata-style chunked stdio exec I/O protocol, covering
  byte-exact offset reads, idempotent stdin writes, slow-consumer bounds,
  concurrent attached fairness, stale sessions, EOF, resize, and signal
  exit mapping.

- Strengthened PTY/job-control proof coverage for guest-control exec,
  including session leadership, controlling-terminal foreground process
  groups, PTY close/drain behavior, SIGWINCH resize semantics, and
  protocol-side TTY `CloseStdin`.

- `docs/reference/guest-control-exec-io-credit-window.md` — bounded ttRPC
  duplex-stream exec I/O design using nixling `TerminalFrame` messages,
  explicit byte credit, close/EOF, resize/signal/exit/error frames, CLI
  behavior, conformance matrix, risks, and required tests.

- Guest systemd-journal log collection. The per-VM OpenTelemetry
  collector now follows the guest journal through the contrib `journald`
  receiver and forwards it to SigNoz as logs tagged with the VM's
  `vm.name` / `vm.env` resource attributes, with the journal `PRIORITY`
  mapped to a readable OTel severity (`INFO`/`WARN`/`ERROR`/…) and a
  `file_storage` cursor so a collector restart resumes without dropping
  entries. `nixling.vms.<vm>.observability.scrapeJournal` now defaults
  to `true` (previously a reserved no-op) and the guest collector user
  is granted `systemd-journal` read access plus `journalctl` on its
  unit PATH. Ingested telemetry's `deployment.environment` resource
  attribute is the physical host machine name (from the host's
  `networking.hostName`, settable via `nixling.observability.hostName`)
  so SigNoz groups VMs by the host they run on; the per-VM env stays on
  `vm.env` / `service.namespace`.

- Native, container-free SigNoz observability backend packages and ADR.
  The bundled observability path now targets SigNoz, the SigNoz OTel
  Collector, schema migrator, ClickHouse, and ClickHouse Keeper as native
  NixOS services.

- `nixling.site.niriVmBorders.{enable,outputPath}` — opt-in niri KDL
  window-rule include generator. When enabled, installs a KDL file at
  the configured path (default `/etc/nixling/niri-vm-borders.kdl`)
  containing a crosvm scanout-window hide rule and one
  `window-rule` per enabled graphics VM. Rules match the
  `nixling.<vm>.` app-id prefix that the host Wayland filter proxy
  writes onto guest windows. Include the file from niri config with
  `include "/etc/nixling/niri-vm-borders.kdl"`. Requires niri ≥ 0.1.9.
- `nixling.vms.<vm>.graphics.niriBorderColor` — per-VM active border
  color override for the generated niri rules, as a six-digit CSS hex
  color (`#rrggbb`). Defaults to `null`, which uses a deterministic
  palette color derived from the VM name.
- `nixling.vms.<vm>.graphics.waylandFilter.{enable,denyGlobals,allowGlobals,maxVersions}`
  — host-side Wayland filter controls for graphics VMs that opt into
  cross-domain forwarding. The filter is enabled by default when
  `graphics.crossDomainTrusted = true`, denies unknown/high-risk globals
  by default, and exposes explicit allow/deny/version-cap overrides.
- `nixling.vms.<vm>.graphics.waylandFilter.{byteLogging,dmabufAllow,dmabufDeny}`
  — default-off diagnostics and dmabuf format/modifier controls for the
  host-side Wayland filter. The filter preserves compositor dmabuf
  feedback by default and lets operators hide known-bad format/modifier
  pairs while keeping buffer creation requests fail-closed against the
  same policy.
- `docs/how-to/niri-vm-borders.md` — how-to for enabling the niri
  include, customizing colors, verifying the setup, and understanding
  the `crossDomainTrusted` requirement for app-id matching.
- `docs/how-to/migrate-to-wayland-proxy.md` — migration guide covering
  app-id renaming, Xwayland fail-closed behavior, `crossDomainTrusted`
  requirement, niri rule updates, and rollback procedure.
- `docs/reference/wayland-filter-warnings.md` — reference warning
  catalog for `graphics.waylandFilter` listing every warning condition,
  the triggering option or global, why the warning exists, and how to
  override intentionally.

- StoreSync-only observability JSONL export. The privileged broker now
  writes a positive-allow-list projection of each terminal StoreSync
  attempt to `<stateDir>/observability/store-sync/store-sync-<utc-date>.jsonl`
  (`0640`, daily-rotated, best-effort). The export carries exactly the
  allow-listed fields (`schema_version`, `target_vm`, `vm_id`,
  `target_env`, `generation_id`, `generation_token`, `sync_status`,
  `error_stage`, `cleanup_status`, `cleanup_reason`, `authz_outcome`,
  closure/linked/skipped/swept counts, `fast_path`, and the flattened
  `*_ms` timings) via a dedicated `StoreSyncObservabilityRecord` struct
  so no serializer ever receives the full host audit record; host-only
  fields (`caller_principal`, `retained_generations`, host/store paths,
  `db.dump`, marker payloads) are redacted by construction. Host Alloy
  follows only this export glob (`local.file_match` + `loki.source.file`,
  following rotation) and the `alloy` identity receives focused
  read/traverse ACLs to the export directory only — never the unified
  broker audit log, the privileged daemon socket, or nixlingd state. The
  Loki stream stays a host singleton (`vm="host"`, `env="host"`,
  `role="host"`, `source="store-sync-audit"`); `target_vm`/`target_env`
  remain JSON content. `target_env` is resolved from the trusted manifest
  when present (and remains a JSON field, not a stream label). New gate
  `tests/store-sync-export-eval.sh`;
  `tests/loki-label-cardinality-eval.sh` now also parses
  `local.file_match` `path_targets` label maps. See
  [ADR 0027](docs/adr/0027-store-view-hardlink-live-pool.md) and
  `docs/reference/store-sync.md` § "Observability export".

- `nixling store verify <vm> [--repair] [--json]` — explicit
  broker-backed live-pool integrity verification for the ADR 0027 split
  store-view. The CLI is thin and never reads `store-view` directly;
  `nixlingd` sends a typed `BrokerRequest::StoreVerify` to the privileged
  broker, which verifies `state/current`, `meta/current`, the host marker,
  zero-length live marker, and every manifest top-level basename in
  `live/`. It writes host-only integrity state under
  `store-view/state/generations/<generation-id>/integrity.json` (or
  `state/integrity-unknown.json` when generation identity is unavailable)
  and returns the signed JSON envelope documented in
  `docs/reference/cli-output/store-verify.md`. `--repair` now delegates to
  StoreSync as a forced non-fast-path republish, then verifies again before
  returning `repaired`; incomplete repairs remain exit-4 `drift`/`unknown`
  instead of a success-shaped result.
- `nixling store verify` now performs deep recursive live-pool verification
  against trusted source closure paths (file type, executable bit, symlink
  target, and hardlink identity or byte equality for copied fallback files).
  Existing top-level packages with internal drift are repaired by staging clean
  replacements and swapping them into `live/` with same-filesystem
  `RENAME_EXCHANGE`, so the served basename is never absent.
- StoreSync success audit/export records now populate available phase timings
  (`lock_wait_ms`, `lock_hold_ms`, `probe_ms`, `verify_ms`, `stage_ms`,
  `metadata_ms`, `cleanup_ms`) in addition to `total_ms`.
- StoreSync now performs conservative cleanup/retention when no virtiofsd
  process appears to be serving the VM's `store-view/live` path. Offline-safe
  cleanup removes unretained live basenames, stale meta/state generation dirs,
  and stale gcroots; online or uncertain serving state defers cleanup.
- Cross-mount store-view materialisation no longer shells through
  `unshare ... /bin/sh -ceu ...`. The broker now execs
  `nixling-activation-helper private-store <verb>` directly; the helper
  unshares its own mount namespace, makes propagation private, lazily detaches
  `/nix/store`, then runs the selected build/replace verb from stdin JSON.

- `nixling config` verb group — the host-side review/approve workflow
  for a VM's guest-editable `guestConfigFile`: `config sync` pulls the
  in-guest edited file over the existing per-VM SSH key into a
  user-local staging copy; `config diff` shows a unified diff against a
  live file; `config approve` atomically writes the staged copy onto an
  operator-chosen target; `config reject` discards it; `config status`
  reports pending stagings. The CLI only writes its own staging area and
  the operator-named `--to` target — it never auto-touches the config
  tree. `approve`/`reject` are host-operator-only and are the
  authoritative containment boundary (the host only ever evaluates an
  operator-approved guest file); an eval-time namespace lint on
  `nixling switch` additionally rejects guest-set host-owned options as
  defense-in-depth. No new privileged surface (no virtiofs, no new
  socket); the untrusted pull is bounded (size cap + timeout). `nixling
  up` / `start` and `nixling status` also print a human-output note when
  a VM has a pending un-approved staged config.
- `nixling.vms.<vm>.guestConfigFile` — a dedicated, **guest-editable**
  per-VM NixOS module for the in-guest OS layer (packages, services,
  in-guest users, files). It is merged into the guest like `config`,
  but is **contained**: a best-effort eval-time namespace lint rejects
  it if it sets any host-owned `microvm.*` (runner substrate) or
  `nixling.*` (framework) option, naming the offending option(s)
  (detected by definition-existence over the real NixOS module set, so
  `imports`/`builtins.toFile`/`_file`-spoofing are caught). The lint is
  defense-in-depth, not a sound sandbox — operator review/approve is the
  authoritative boundary; see
  [ADR 0024](docs/adr/0024-in-vm-guest-config-sync.md) for the trust
  model and the deferred sound-evaluator work. This is the foundation
  for the in-VM config-sync workflow — an operator can edit this file
  from inside the VM and sync it back for review. Host-owned settings
  stay in `config`, which the guest cannot edit. When set, the current
  approved guest config is also seeded into the VM (read-only at
  `/etc/nixling/guest-config.nix`, plus a writable working copy at
  `/var/lib/nixling-guest/guest-config.nix`) so it can be edited from
  inside the VM. See
  [`docs/how-to/edit-vm-config-from-inside.md`](docs/how-to/edit-vm-config-from-inside.md).

### Removed

- `nixling vm konsole` is removed. The subcommand was a thin wrapper that
  re-exec'd `nixling vm exec -it <vm> -- <login-shell> -l` inside a host
  terminal emulator; operators now invoke `nixling vm exec -it` directly.
  All references (CLI surface, shell completions, manpage, and reference
  docs) are dropped accordingly.

### Changed

- `nixling vm exec` now runs the requested command as the VM's
  configured workload user (`ssh.user`) — **never root** — inside a real
  PAM login session (`systemd-run --property=PAMName=login
  --uid=<user>`). The command sees the same environment an interactive
  SSH login would (`XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`, the login-shell
  profile), so graphical and login-shell workflows (e.g. launching a
  browser) work unchanged; operators elevate with `sudo` inside the
  session. `guestd` host-fixes the exec identity and ignores the wire
  `user` field. The per-VM `guest.exec.allowRoot` and `guest.exec.users`
  options are removed — enabling `guest.exec.enable = true` on a VM with
  a workload user is sufficient, and a VM whose `ssh.user` is unset,
  `root`, or otherwise invalid disables exec at eval time with a typed
  message. See
  [ADR 0030](docs/adr/0030-guest-exec-as-workload-user.md).
- Framework readiness for a guest-control-capable VM is now the
  authenticated guest-control Health probe rather than a raw TCP-22 SSH
  connect. The per-VM DAG node `guest-ssh-readiness` is replaced by
  `guest-control-health` (`ProcessRole::GuestControlHealth` +
  `ReadinessPredicate::GuestControlHealth`), which fails closed: a VM is
  ready only once the daemon completes the full authenticated handshake
  and the guest reports `Healthy` or `Degraded`. Old-generation /
  unreachable / auth-failed / timed-out guests are never marked ready.
  Per-VM guest sshd and host keys remain for the SSH compatibility
  window but no longer drive framework readiness. See
  [ADR 0029](docs/adr/0029-framework-ssh-to-typed-guest-rpc.md).
- `nixling config sync` on a guest-control VM now pulls the editable
  guest config over the authenticated guest-control bridge (the new
  `ReadGuestConfig` daemon verb) instead of an SSH transfer. The host
  computes size and sha256 from the received bytes and keeps the existing
  atomic temp+fsync+rename staging. `--dry-run` reports
  `transport: "guest-control"` and the planned target without reading any
  guest bytes or printing an SSH command, and SSH-only flags
  (`--host` / `--user` / `--key` / `--known-hosts` / `--guest-path`) are
  rejected on the guest-control path with a remediation pointing at the
  operator SSH compatibility transport. Old-generation VMs that predate
  guest-control fail closed with `guest-control-unavailable-old-generation`.
- The framework readiness label is now the canonical `guest-control-health`
  (no per-VM suffix) across `status`, `vm list`, and the start preview;
  the start-preview DAG no longer hard-codes an `ssh-ready` node.
- The default observability VM name is now `sys-obs`. The old
  `sys-obs-stack` state is not deleted automatically; keep it for
  rollback until the new stack is validated.
- Observability metadata in `vms.json` moves to manifest version 5 for
  the SigNoz backend shape (unified with the base-vsock change; the
  intermediate `4` was never shipped on its own). Historical v3 fixtures
  remain frozen.
- Host and guest telemetry collection is moving from Alloy pipelines to
  OpenTelemetry Collector services that export OTLP over nixling's
  broker-supervised Unix/vsock transport.
- Retired Grafana credential-file options are now documented as
  compatibility shims; native SigNoz credentials can be sourced from
  `nixling.observability.signoz.{jwtSecretFile,rootPasswordFile,clickhousePasswordFile}`.
- `retention.*` and `sampling.*` remain compatibility shims for the
  retired Tempo/Loki backend and warn when changed; native
  SigNoz/ClickHouse retention is operator-managed.
- Per-VM store isolation is moving to the Rust-owned `store-view/live`
  hardlink pool
  ([ADR 0027](docs/adr/0027-store-view-hardlink-live-pool.md)). The
  broker `StoreSync` path is the canonical writer for store-view
  metadata and live pool updates; host activation no longer
  builds/sweeps store-view closures. The guest readiness marker
  `store-view/live/.nixling-marker-<vm>` is a zero-length file, and each
  generation publishes a guest-safe `meta.json` authored by an
  independent allow-list serializer (`schema_version`, `generation_id`,
  `generation_token`, `sync_status`, `closure_count`) that never
  receives the full host audit record. The broker `StoreSync` wire
  response now carries the collision-free `generation_id` alongside the
  u32 `generation_token` (request + response renamed `generation` →
  `generation_token`); the token is display/wire only and is never used
  as the on-disk layout key. Each StoreSync attempt that reaches the
  broker handler emits exactly one terminal structured broker audit
  record under the signed `StoreSyncAuditFields` schema
  (`schema_version = 1`) with invariant-enforcing constructors and
  `validate()`: success records use `ok_fast_path` / `ok_non_fast_path`,
  and a failure emits a `failed` record carrying the classified
  `error_stage` (the failure surfaces as `BrokerError::StoreSyncFailed`
  and is never double-audited). Authorization-deny emission
  (`error_stage = authz`) is modelled by the `denied` constructor but is
  not yet reachable from dispatch, pending a per-VM StoreSync
  authorization policy.
- Graphics VMs that opt into cross-domain forwarding use
  `wl-cross-domain-proxy` in the guest and a host-side
  `nixling-wayland-filter` proxy instead of the former
  `wayland-proxy-virtwl` guest relay.
- `nixling.vms.<vm>.graphics.xwayland.enable = true` now fails eval
  during the Wayland-only migration. X11 application support will return
  through a separately validated helper path.

### Security

- Graphics VMs that opt into cross-domain forwarding now route guest
  Wayland traffic through a host-jailed `nixling-wayland-filter` process
  before reaching the real host compositor. The GPU sidecar connects to
  the per-VM filter socket; the dedicated `nixling-<vm>-wlproxy`
  principal is the VM-specific role with compositor socket access.
- Per-VM store isolation: the daemon-native virtiofsd `ro-store` runner
  served the host's entire `/nix/store` to every guest, so a guest's
  `/nix/store` exposed all host store paths instead of only the VM's own
  closure. virtiofsd now serves the per-VM closure-only hardlink farm
  (`/var/lib/nixling/vms/<vm>/store`), restoring the isolation the legacy
  `BindReadOnlyPaths /nix/store -> per-VM farm` provided; a guest's
  `/nix/store` now contains only its own closure.
- StoreSync observability export confinement: Grafana Alloy is granted
  focused POSIX ACLs (`u:alloy:--x` traverse on `<stateDir>` and
  `<stateDir>/observability`, `u:alloy:r-x` + a `default:u:alloy:r--`
  ACL on the export dir) to read the StoreSync export and nothing else
  under the broker state dir. Alloy is never added to the `nixlingd`
  group and gets no read access to the unified broker audit log
  (`<stateDir>/audit/broker-*.jsonl`) or the privileged daemon socket.
  The export itself is a redacted projection, so a host-Alloy compromise
  exposes only the allow-listed StoreSync fields already destined for
  Loki, not the host-confidential audit stream.

### Fixed

- The host OTel bridge is now represented as a daemon/broker process role
  (`otel-host-bridge`) so readiness can track the broker-spawned runner.
- Observability relay ACL setup now excludes the host bridge principal
  from broad obs-VM state directory grants and uses the nixling-owned OTel
  runtime path for the bridge egress socket.
- TPM-enabled guests now flush stale loaded/saved TPM sessions during
  early boot before SRK provisioning. This prevents swtpm session-handle
  exhaustion from breaking TPM-bound credentials while preserving NVRAM
  and persistent handles.
- VM start (`nixling up` / `switch`) no longer aborts with
  `SpawnRunner failed ... broker-error` ("Invalid cross-device link")
  while building the per-VM store-view hardlink farm on hosts where
  `/nix/store` is bind-mounted read-only on top of itself (the stock
  NixOS layout). `link(2)` is rejected across that vfsmount boundary
  even when both paths share the same underlying filesystem, so the
  broker's in-process farm build failed with `EXDEV`. The broker now
  builds the farm inside a private mount namespace where `/nix/store`
  is lazily detached (mirroring the existing activation-time
  `nixling-store-sync` workaround), via the `nixling-activation-helper
  build-store-view-farm` subprocess, and only falls back to that
  namespace path when an in-process build actually hits the cross-mount
  case (so same-filesystem hosts and tests stay in-process). A raw
  `EXDEV` at the `link(2)` site is now classified as a recoverable
  same-filesystem cross-mount (retried in the namespace) versus a fatal
  genuinely-different-filesystem error (propagated).
- VM start no longer fails while building the per-VM store-view farm on
  a `nix-store --optimise`d store. Deduplicated empty/tiny store files
  share a single inode that reaches the filesystem hardlink ceiling
  (ext4 `EXT4_LINK_MAX` = 65000); the farm builder now falls back to a
  byte copy for those already-saturated (read-only) inodes instead of
  failing with `EMLINK`.
- VM start no longer leaves the per-VM state/runtime root
  (`/var/lib/nixling/vms/<vm>`, `/run/nixling/vms/<vm>`) owned by a
  transient runner principal with a clipped POSIX ACL mask. The
  vm-start directory prepares now preserve the ownership + mode that
  host activation establishes (`nixlingd:users 2770` plus per-runner
  ACLs) on an existing directory, so runners (virtiofsd, gpu, video)
  keep write access to their per-VM runtime dir and the ownership-matrix
  preflight no longer trips.
- `nixling switch` / `boot` / `test` no longer fail with `broker-error`
  ("no store-view intent in the trusted bundle"). The per-VM closure
  artifact now emits a populated `hostGeneration` (a deterministic,
  content-derived store-view generation), so the broker builds a
  store-view intent for every VM instead of skipping it. Previously
  live activation was impossible and the only way to apply a new
  generation was `nixling down <vm> --apply` followed by
  `nixling up <vm> --apply`. The per-VM `/nix/store` hardlink farm now
  also fails closed on a store-view generation collision (two distinct
  closures of one VM mapping to the same generation number) instead of
  unioning them, by pinning the closure identity in the generation
  marker.
- VM start no longer aborts with `SpawnRunner failed ... broker-error`
  on the first runtime-directory step. The broker's path-safe directory
  opener resolved every path from `/` with `RESOLVE_NO_XDEV`, which
  fails with `EXDEV` ("Invalid cross-device link") the moment it must
  cross a mount boundary — and the per-VM runtime dir lives under the
  `/run` tmpfs, the tap device under `/dev`, cgroups under `/sys`, etc.
  Resolution now walks component by component and follows a *real*,
  pre-existing mount crossing (still refusing symlink / magic-link
  components and `..` escapes at every step), so legitimate
  cross-filesystem paths resolve while the load-bearing symlink
  protection is preserved.
- Broker spawn/host-prep failures are no longer opaque. The broker now
  logs the live-handler root cause (errno / path / stderr) to its
  journal, the daemon includes the broker's `message` in its
  `vm start node spawn failed` log, and failure remediations point at
  the working `journalctl -u nixling-priv-broker` instead of the
  `nixling audit --strict` command (which returns `not-yet-implemented`).
- GitHub Actions PR hardening keeps fork PR code off self-hosted
  runners, makes the privileged oracle workflow manual-dispatch only,
  and repairs the affected CI validation gates so the hardening can
  merge through the normal PR checks.

## [1.2.0] - 2026-06-03

Primarily a stabilization release per
[ADR 0022](docs/adr/0022-stabilization-mode-releases.md): deferrals
from the v1.x cycle close out and a live-VM smoke gate is now
required before tagging. It also lands two default-off, opt-in
graphics video-decode paths and unifies the lifecycle Unix group
into a single `nixling` group — a breaking change for consumer
configs that referenced the legacy group names (see
**Changed (breaking)** below).

### Added

- `nixling vm start --apply` readiness split into `process-alive` +
  `api-ready` DAG nodes. `--no-wait-api` opts into exit-0 once the
  process is alive; the strict-API default is preserved.
- `nixling vm status --json` surfaces the new `api_ready` field
  (`yes` / `pending` / `timeout` / error).
- `nixling host doctor` ships four new probes
  (`check_seccomp_bpf_loaded`, `check_pre_ns_posture`,
  `check_broker_reap_health`, `check_bridge_ipv6_sysctl`); see
  [`docs/reference/doctor.md`](docs/reference/doctor.md).
- `writableStoreOverlay` re-enabled. The broker provisions the per-VM
  overlay disk via the new `SpawnRunnerPlanOp::DiskInit` op
  (`mkfs.ext4` on first spawn). Size override via
  `nixling.vms.<vm>.writableStoreOverlaySize` (default 1 GiB).
- `tests/integration/live/live-vm-smoke.sh` (`--lite` / `--full`) is the maintainer
  pre-tag gate (`make pre-tag` / `make smoke-lite`); results land in
  `${TMPDIR:-/tmp}/nixling-smoke-run-log.txt`.
- New ADRs:
  [ADR 0022](docs/adr/0022-stabilization-mode-releases.md)
  (stabilization-mode releases) and
  [ADR 0023](docs/adr/0023-runner-role-lifecycle-matrix.md)
  (runner-role lifecycle matrix).
- New runbooks:
  [`docs/how-to/recovery-pre-ns-role-failure.md`](docs/how-to/recovery-pre-ns-role-failure.md),
  [`docs/how-to/route-conflicts.md`](docs/how-to/route-conflicts.md).
- Graphics VMs can opt into the daemon-spawned virtio-media H264 decode
  path with `nixling.vms.<vm>.graphics.videoSidecar = true`. The path uses
  the vendored patched Cloud Hypervisor `--vhost-user-media` support and a
  patched crosvm `device video-decoder --backend vaapi` runner; no per-VM
  systemd unit or stock-binary fallback is introduced.
- Graphics VMs can opt into experimental guest VA-API video forwarding with
  `nixling.vms.<vm>.graphics.virglVideo = true`. The switch is default-off
  and surfaces a status readiness marker because it enables
  `VIRGL_RENDERER_USE_VIDEO` in the crosvm/virglrenderer GPU path.

### Changed

- **Seccomp BPF programs are now compiled from `ioctl_policy.rs`**
  and loaded by the broker before `execve`; the per-role allowlists
  are the source of truth.
- **Broker pre-NS user namespace** extended to the `swtpm` role
  (full), the `gpu` role (render-node only via `SCM_RIGHTS` fd
  passing), and the `audio` role (owned net-NS). Long-lived sidecars
  now run with zero host capabilities inside the broker-established
  user namespace. See
  [ADR 0021](docs/adr/0021-broker-user-namespace-for-virtiofsd.md).
- **Broker now reaps spawned children** via tokio signalfd +
  `waitid(P_PIDFD)` and reports `ChildReaped` to `nixlingd`.
- Bridge IPv6 sysctls (`disable_ipv6 = 1` on `br-*-up`) are now
  applied at boot via `boot.kernel.sysctl`.
- `nixling-priv-broker` may drop `CAP_NET_ADMIN` from its minijail
  bounding set when pre-created TAP fds are passed through.
- `umask` is plumbed end-to-end through `MinijailProfile` →
  `RoleProfile` → `SpawnRunnerPlan`; sidecar profiles default to
  `0o007`.

### Changed (breaking)

- Unified the legacy `nixling-launcher` and `nixling-launchers` Unix
  groups into a single `nixling` group. The activation script re-chgrps
  state files automatically on the next `nixos-rebuild switch` using a
  fd-safe numeric-gid migration helper. Consumer NixOS configs that
  reference the legacy group names in `users.<name>.extraGroups` must
  update to `"nixling"`. Required post-switch step:
  `sudo systemctl restart nixlingd.service`. See
  [docs/how-to/migrate-nixling-v1-1-to-v1-2.md](docs/how-to/migrate-nixling-v1-1-to-v1-2.md).
  The broker caller-role audit label remains `"nixling-launcher"` for
  audit-format stability; see
  [docs/reference/naming-conventions.md](docs/reference/naming-conventions.md#broker-caller-role-audit-labels).
  `OperationFields::DeregisterRunnerPidfd { vm_id, role_id }` now
  appears in broker audit logs on successful `vm stop` cleanup for
  per-VM-UID runners; scripts that previously matched the old broker
  error exit see the corrected successful behavior instead.

  Note: the legacy `nixling-launcher` and `nixling-launchers` Unix
  groups remain on the system as empty v1.2 migration tombstones (zero
  membership, gid preserved in `/etc/group`). `getent group
  nixling-launcher` will still return a record with an empty member
  list. They are slated for removal in a v1.3 follow-up.

### Fixed

- Disk-init dispatch: `nixlingd` now invokes `BrokerRequest::DiskInit`
  before `SpawnRunner` when the plan node carries plan-ops.
- Overlay disk gets the same CH disk-arg defaults (`direct`,
  `image_type`, `num_queues`) as regular volumes.
- Guest fstab: ro-store virtiofs share mounts at `/nix/.ro-store`
  and the overlay backing disk mounts at `/nix/.rw-store` (both
  `neededForBoot = true`) so initramfs assembles the overlayfs
  correctly.
- `net_route_preflight` now tolerates `NO-CARRIER` state.
- `tests/principal-uid-collision-eval.sh` verifies the
  `stablePrincipalId` hash produces unique UIDs.
- Declared `microvm.volumes` now get stable virtio-blk serials and matching
  guest `fileSystems` mounts. This fixes guests whose persistent `/var`
  volume was attached but not mounted, causing identity-bearing state such as
  `/etc/machine-id`, systemd credentials, and Himmelblau cache data to live on
  tmpfs and change after each VM restart.
  Existing Entra-joined VMs affected by the old behavior may need one final
  enrollment after upgrading if their previous `/var` identity state only ever
  lived on tmpfs; after the persistent `/var` volume is populated, restarts
  should not trigger re-enrollment.
- `nixling vm stop` no longer fails with `pidfd_table SIGTERM failed`
  when the runner runs as a per-VM dedicated UID: the daemon falls back
  to a broker-mediated signal on EPERM and deregisters the broker-side
  pidfd registry after successful termination.
- `nixling vm konsole` no longer reports `ssh key not found` when the
  parent directory is unreadable: the CLI distinguishes ENOENT from
  EACCES and emits an actionable error pointing at `nixling` group
  membership.
- `/var/lib/nixling/` now grants execute-only ACL traversal to the
  lifecycle group so the CLI can resolve keys and bundles without
  widening read access.
- Video sidecars now run as a dedicated `nixling-<vm>-video` principal, and
  activation/broker ACL refreshes deny that principal access to host
  Wayland, PipeWire, and Pulse sockets while preserving GPU cross-domain
  access for `nixling-<vm>-gpu`.

### Documentation

- ADRs 0003, 0011, 0021 received "Updated v1.2" subsections
  describing the broker-pre-NS extensions and reap responsibility.

### Deferred

- Drop the empty `nixling-launcher` and `nixling-launchers` Unix group
  declarations introduced as v1.2 migration tombstones, after one
  release of confirmed clean migration.

## [1.1.2] - 2026-06-02

v1.1.2 closes the v1.1.1 → live-VM bring-up gap by retiring the
`virtiofsd --sandbox=namespace + requiresStartRoot = true` carve-out
from [ADR 0003](docs/adr/0003-minijail-provisioning-and-sandbox-interface.md)
in favour of a broker-pre-established single-entry user namespace
([ADR 0021](docs/adr/0021-broker-user-namespace-for-virtiofsd.md)).

### Changed

- **virtiofsd runs with zero host capabilities** inside a broker
  pre-established user namespace. The broker uses
  `clone3(CLONE_NEWUSER)` and writes `/proc/<pid>/uid_map` before
  execing virtiofsd. `--sandbox=chroot` replaces `--sandbox=namespace`.
- TPM socket moved from `/run/swtpm/<vm>/sock` to
  `/run/nixling/vms/<vm>/tpm.sock`; both halves of the wiring update
  in lockstep on rebuild.
- Sidecar UIDs are now derived from the `stablePrincipalId` hash so
  on-disk owner, ownership-matrix entry, and broker setuid target
  all agree.
- Cloud-Hypervisor 52 is now the required version (variadic
  `--fs sock1,tag1 sock2,tag2` argv form).
- `MinijailProfile` gains an optional `umask: Option<u32>` field;
  sidecar profiles (`swtpm`, `audio`, `gpu`) use `0o007` so bound
  Unix sockets land mode `0660`.

### Fixed

- `ssh_host_key_preflight` accepts mode `0440` when a POSIX ACL
  xattr is present.
- Variadic CH argv emission, absolute `vsockPath`, dev/net/tun bind
  inside the CH sandbox, and several broker child-process robustness
  fixes (tmpfile race in `PidfdTable::snapshot`, zombie detection in
  `wait_for_one_shot_exit`).

### Notes

- `microvm` is no longer required as a consumer flake input.

## [1.1.1] - 2026-06-01

Closes every v1.1 deferral.

### Added

- **`StatusServicesOutputV3`** wire schema with broker-spawn-aware
  fields (`hypervisor`, `virtiofsd_per_share`, `audio`,
  `otel_relay`, `otel_host_bridge`, `usbip_backend_per_env`,
  `usbip_proxy_per_env`). A `from_v2()` conversion shim is exported
  for incremental adoption; the CLI emit-side flip lands in v1.1.2.
- **`nixling vm konsole <vm>`** — opens an SSH session to a VM in a
  host terminal. Resolves the key from the bundle's
  `managed_keys.effective_key_path` and detaches via `setsid`.
- **Atomic cgroup placement** via `clone3(CLONE_INTO_CGROUP)`. New
  per-VM `<slice>/<vm>/<role>/` taxonomy (the per-VM interior node
  stays process-free).
- **USBIP guest attach/detach** routed through hardened SSH argv.
- **Pidfs runtime self-probe**: `nixlingd` hard-refuses to start on
  kernels without pidfs unless
  `NIXLING_ALLOW_PIDFS_PROBE_SOFT_FAIL=1` is set.
- **`RenderDnsmasqEnvConf`** pure-Rust dnsmasq config renderer as a
  broker host-prep op.
- A real syn-based AST walker
  (`tests/tools/no-bash-ast-walker/`) backs
  `tests/no-bash-exec-eval.sh`.

### Fixed

- `fchownat(AT_EMPTY_PATH)` replaces broken `fchown` on `O_PATH`
  descriptors in the cgroup module.

## [1.1.0] - 2026-05-31

Daemon-only follow-through. Nixling now owns its per-VM microVM
substrate end-to-end; the `microvm.nix` flake input is gone.

### Added

- **`nixos-modules/vm-options.nix`** declares the per-VM option set
  (hypervisor, vcpu, mem, kernel, shares, devices, volumes, …).
- **`nixos-modules/vm-evaluator.nix`** evaluates per-VM modules with
  the upstream NixOS evaluator (`eval-config.nix`). The
  `nixling.vms.<vm>.computed` option exposes the result.
- Rust runner-argv generators in `packages/nixling-host/`
  (cloud-hypervisor, virtiofsd, swtpm, gpu, audio, usbip,
  vsock-relay, otel-host-bridge) are now the canonical argv source.
- Typed CLI envelopes for `daemon-down` (exit 1) and
  `not-yet-implemented` (exit 78). The Rust CLI never invokes bash.

### Removed

- `microvm.nix` flake input dropped from `flake.nix`. Consumers who
  only inherited the input via `nixling.inputs.microvm.follows = …`
  need no flake change; consumers who declared `microvm.url`
  themselves can drop the input if they don't use microvm directly.
- `nixling.vms.<vm>.supervisor` option removed. Setting it now
  fails eval with a typed friendly message.
- `nixling-vfsd-watchdog@.{service,timer}` retired (wedge detection
  moved into the broker's virtiofsd `SpawnRunner` pidfd supervisor).
- `host-otel-relay-acl.nix` retired; OTel host-bridge ACL moved
  into the broker pre-spawn pipeline.

### Changed

- Kernel floor uplifted to **Linux ≥ 6.9** (`pidfs`-backed pidfd
  identity is required). See
  [ADR 0008](docs/adr/0008-supported-platforms-and-rejected-targets.md).
- `nixling.daemonExperimental.enable` is now obsolete and a no-op;
  the broker socket/service are enabled by default. The option name
  remains evaluable, with a warning when set.
- New invariant gates: `no-bash-exec-eval`,
  `supervisor-option-absent-eval`, `broker-systemd-unit-eval`,
  `daemon-experimental-warning-eval`, `state-dir-acl-eval`,
  `otel-acl-migration-eval`, `vfsd-watchdog-retired-eval`,
  `processes-json-eval`, `vm-submodule-eval`,
  `kernel-modules-parity-eval`, `vm-submodule-cutover-eval`,
  `v1.1-kernel-floor-eval`, `microvm-nix-absent-eval`.

## [1.0.0] - 2026-05-31

Daemon-only end-state per
[ADR 0015](docs/adr/0015-daemon-only-clean-break.md). Clean break
from the v0.x bash CLI + per-VM systemd templates: `nixlingd` and
`nixling-priv-broker` are the only persistent root surfaces.

### Removed (breaking)

- **Bash CLI deleted.** `nixos-modules/cli.nix`, the
  `share/nixling/cli.sh` entrypoint, and every bash subcommand are
  gone. The Rust `nixling` binary is the sole CLI; there is no
  fallback bridge. `NIXLING_LEGACY_BASH_OPT_IN` and
  `NIXLING_NATIVE_ONLY` are no-ops.
- **Per-VM systemd templates retired.** `nixling@<vm>.service`,
  `nixling-<vm>-{gpu,swtpm,video,snd}.service`, and
  `nixling-known-hosts-refresh@<vm>.service` are deleted. Every
  per-VM lifecycle step runs inside `nixlingd`'s DAG executor;
  spawned runners (cloud-hypervisor, virtiofsd, swtpm,
  vhost-user-sound, USBIP attach) are launched by the broker's
  `SpawnRunner` op and handed back as pidfds via `OpenPidfd` /
  `SCM_RIGHTS`.
- **Host singletons retired.**
  `nixling-audit-check.{service,timer}`,
  `nixling-ch-exporter.service`,
  `nixling-net-route-preflight.service`,
  `nixling-otel-host-bridge.service`, and per-env
  `nixling-sys-<env>-usbipd-*` units are deleted. Their work moved
  into `nixlingd` (Prometheus exposition, net-route preflight,
  USBIP state machine) or into broker ops (`ExportBrokerAudit`,
  `UsbipBindFirewallRule`, `SpawnRunner{role: Usbip}`).
- **Polkit per-VM allowlists removed.** `nixling-launchers` group
  membership + `SO_PEERCRED` on `public.sock` is the only lifecycle
  authorisation surface.

### Changed (breaking)

- **Manifest `manifestVersion`: 2 → 3.** No compatibility window;
  the daemon and CLI reject v2 bundles with
  `manifest-version-mismatch`. Operators must rebuild the manifest.
- **Cgroup v2 slice** consolidated to a single `nixling.slice`
  delegated to the `nixlingd` uid by the broker; see
  [ADR 0011](docs/adr/0011-cgroup-delegation-and-ownership.md).
- `nixling_host::DeviceClass` gained `Udmabuf` for GPU sidecar
  ioctls; `modules_disabled` is fail-closed in the broker's
  `ModprobeIfAllowed` path.

### Added

- **`nixling host validate` / `host reconcile`** — host-side
  preflight + degraded-mode recovery for the daemon's net-route
  monitor.
- **Broker audit** (`OpAuditRecord`) at
  `/var/lib/nixling/audit/broker-<utc-date>.jsonl`
  (`0640 root:nixlingd`, append-only, daily rotation, 14-day
  retention by default; override with
  `nixling.site.audit.retentionDays`).
- **`docs/how-to/migrate-nixling-v0-to-v1.md`** is the
  operator-facing migration guide.

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
  `DeviceAllow=/dev/vsock`, `restartIfChanged=false`).
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
  requires a `bin/socat`-compatible CLI today. Non-socat
  relays need a dedicated compatibility interface before the
  socat-compatible path can be removed.
- **VM-runner abstraction.** Today the framework leaks the
  runner-unit name (`microvm@<vm>` for headless,
  `nixling-<vm>-gpu` for graphics) into the relay wiring, and
  the observability code has to wire to both. A runner-agnostic
  abstraction is required before per-VM sidecar wiring can stay
  on a single name.


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

Patch release. Review of v0.1.6 caught a silent bug in the
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
- **`tests/restart-policy-eval.sh`** (regression added in v0.1.6):
  tightened the predicate to REJECT `unitConfig.X-RestartIfChanged`.
  The previous version accepted either form, so it would have passed
  against the v0.1.5/v0.1.6 broken setup. Now any service using the
  broken form fails the test with an explicit message pointing at
  this CHANGELOG entry.
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


## [0.1.6] - 2026-05-19

Docs catch-up release. The v0.1.1–v0.1.5 patches shipped fixes for
five framework bugs surfaced during the first real consumer
migration, but the public docs hadn't been updated to describe the
resulting behavior changes. This release brings the docs in sync
with the code, plus a small audit-strict fix that completes
`v0.1.4`'s skip-stopped-VMs work, tightens the autostart wiring,
and adds regression tests for every v0.1.x patch.

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

- `tests/unit/smoke/smoke-eval-extraspecialargs.nix` — regression for v0.1.1
  `extraSpecialArgs` propagation through `nixos-modules/host.nix:165`.
- `tests/net-vm-network-eval.sh` extended — regression for v0.1.2
  `ConfigureWithoutCarrier` + route entry on the host's uplink bridge.
- `tests/autostart-wiring-eval.sh` — covers `nixling@<vm>` as
  template-only, multi-user.target.wants wiring, and
  `microvms.target.wants == []`.
- `tests/unit/smoke/smoke-eval-graphics.nix` extended — regression for v0.1.4
  `/dev/net/tun rw` in the GPU sidecar's DeviceAllow.
- `tests/unit/smoke/smoke-eval-tpm.nix` — regression for v0.1.4 swtpm parent-dir
  ACL traversal grant.
- `tests/restart-policy-eval.sh` — regression for v0.1.5
  `restartIfChanged = false` across all six services.
- Negative-assertion regression in `tests/assertions-eval.sh`
  (`test_graphics_with_autostart`).

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
    gaps" (resolved).

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

**Composition:** Sibling flake [`vicondoa/entrablau.nix`][entrablau] (also
v0.1.0) provides Entra ID device-join via the per-VM
`nixling.vms.<vm>.config.imports = [ inputs.entrablau.nixosModules.default ]`
seam.

[entrablau]: https://github.com/vicondoa/entrablau.nix

> Maintainer GitHub metadata reminder (apply on the GitHub UI, not in git):
>
> - **Description:** "NixOS microVM framework with isolated per-env
>   networking, Wayland/audio/USBIP/TPM components, and a
>   `nix flake init` template scaffold."
> - **Topics:** `nixos`, `nix-flake`, `microvm`, `wayland`,
>   `microvm-nix`, `nixos-template`, `entra-id`.



### Added

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
  now build-before-state-move.
- Five per-component reference docs under
  `docs/reference/components-*.md` (graphics, tpm, usbip, audio,
  home-manager).
- `docs/reference/manifest-schema.{md,json}` polished with a rendered
  example payload generated from `tests/unit/smoke/smoke-eval.nix`.
- **`examples/minimal/`** — headless starter example: one env, one
  workload VM, ~25-line flake. Provides a quick sanity test.
- **`examples/graphics-workstation/`** — desktop VM with
  `graphics.enable`, `audio.enable`, and `usbip.yubikey` all on.
  Exercises every host-side sidecar component.
- **`examples/multi-env/`** — two parallel `nixling.envs.<env>`
  instances (work + personal) demonstrating per-env LAN
  isolation, per-env net VMs, per-env USBIP backends, and the
  route-preflight fail-closed gate.
- **`examples/with-entra-id/`** — composition with the sibling
  [`vicondoa/entrablau.nix`][entrablau] flake; shows how
  the two trees meet at `nixling.vms.<vm>.config.imports`
  without either flake depending on the other.
- **`templates/default/`** — `nix flake init` scaffold with
  seven numbered placeholder markers and a matching
  `assertions = [ … ]` block. `nix flake check` on an un-edited
  scaffold fails with actionable messages until each sentinel is
  replaced.
- **`flake.templates.default`** — wires the template above so
  consumers can `nix flake init -t github:vicondoa/nixling`.
- **Manifest contract is now a documented, versioned interface.**
  - `nixos-modules/manifest.nix` — typed `config.nixling.manifest`
    `attrsOf submodule` option. Replaces the inline manifest
    construction previously folded into `cli.nix`. The Nix module
    system catches schema regressions at eval time.
  - `docs/reference/manifest-schema.md` + `docs/reference/manifest-schema.json`
    (JSON Schema Draft 2020-12) — the v1 public manifest contract
    for downstream consumers such as the Rust CLI. The
    JSON Schema is the canonical type spec; the prose doc is a
    field-by-field walkthrough + compatibility policy.
  - `docs/reference/cli-contract.md` — behavioural contract for any
    `nixling` CLI implementation (lifecycle FSM, signal semantics,
    exit codes, JSON vs human output, what is/is-not in scope).
  - `nixling.site.flakePath` is now derived as the CLI's default
    flake reference when unset (cli.nix lifecycle subcommands).
- **`docs/README.md`** — Diataxis IA index (tutorials, how-to,
  reference, explanation). The reference quadrant landed first;
  the others landed before v0.1.0.
- **Multi-arch eval coverage.** `tests/unit/smoke/smoke-eval-aarch64.nix` —
  cross-evaluates a headless workload VM on `aarch64-linux`,
  verifying the eval graph stays multi-arch clean. Runtime is still
  `x86_64-linux`-only (cloud-hypervisor + crosvm); aarch64 is
  eval-coverage only.
- **Manifest validation gate.** `tests/static.sh` now renders the
  smoke manifest and runs a 6-check sequence against
  `docs/reference/manifest-schema.json`: render → parse schema →
  JSON-Schema validate → schema-side field cross-check →
  `manifestVersion >= 1` → prose-schema table diff against the JSON
  Schema's `properties` keys to catch md ↔ json drift.
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
- **`scripts/migrate-nixling-v0.1.0.sh`** — one-shot host migration
  script for consumers upgrading from a pre-public in-tree nixling
  layout. Preserves TPM state byte-for-byte. Has `--dry-run` and
  `--rollback`. Committed under `scripts/` so CI can shellcheck it.
- **`tests/unit/smoke/smoke-eval.nix`** — minimal consumer-style nixosSystem
  that imports `nixling.nixosModules.default` and exercises the
  eval graph end-to-end. Wired into `tests/static.sh` Layer-1.
- **`tests/assertions-eval.sh`** — 8 regression tests exercising every
  eval-time invariant in the schema (CIDR shape, CIDR overlap, key
  validation, `waylandUser` presence, …).
- **`nixos-modules/lib.nix#cidrOverlaps`** — pure-Nix IPv4 prefix
  overlap helper used by network.nix assertions. Same file gains
  `parseCidr` as a public helper.
- Initial flake skeleton with Apache-2.0 license, `x86_64-linux` +
  `aarch64-linux` eval, `microvm.nix` input, and reserved-but-inert
  `nixosModules.default`.
- Mechanical lift of nixling modules from `/etc/nixos/modules/nixling/`
  into the public flake:
  - 9 core modules under `nixos-modules/` (`default`, `options`,
    `lib`, `host`, `network`, `base`, `store`, `cli`;
    `router.nix` renamed to `net.nix`);
  - 6 component modules under `nixos-modules/components/`
    (`graphics`, `tpm`, `usbip`, `home-manager`; `audio` split into
    `audio/{guest,host}.nix`);
  - Extracted pkgs: `spectrum-ch`, `vhost-device-sound`,
    `crosvm-patched`, `crosvm-seccomp`, `patches`;
  - 6 generic test scripts under `tests/`.
- `systemd.services."nixling@"` wrapper template with explicit
  `ExecStart` / `ExecStop` / `PropagatesStopTo`; `BindsTo` alone
  does not propagate stops.
- Eval-time assertions for VM names (`^[a-z0-9][a-z0-9-]*$`, no
  `sys-` prefix, not `launcher`) and env names (≤ 8 chars).
- `nixos-modules/assertions.nix` as a dedicated assertions module.
- Top-level `manifestVersion = 0` stub field in the per-VM JSON
  manifest. It was added as a stub; a later release bumps it. Stashed
  under the reserved `_manifest` sentinel key; user-declared VM names
  cannot start with `_` under the stricter regex.

### Changed

- `docs/README.md` IA now reflects the shipping how-to and
  explanation docs (was previously reference-only).
- **README:** restructured to lead with a Where-to-start table
  pointing at the four examples and the template, and rewrote
  the Quick start to walk through the template path; the prior
  manual paste-in walkthrough is preserved under Manual integration
  without the template.
- **`docs/README.md`:** added a Tutorials/Examples section linking the
  examples and the template; previously the docs index only mentioned
  the reference quadrant.
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
- **BREAKING for manifest consumers, security fix:** `sshKeyPath`
  removed from the per-VM JSON manifest. Security review flagged
  the field as a private-key path leak — the manifest at
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
  stays at `1` — the schema was published moments before release and
  no external consumers exist yet, so this is a free pre-v0.1.0 break.
- `docs/reference/manifest-schema.json`: `manifestVersion.minimum`
  raised from `0` to `1`. The schema is the contract for v1+;
  pre-v1 manifest stubs are no longer valid under this schema.
- `docs/reference/cli-contract.md`: subcommand inventory reconciled
  with `nixling --help`. `audit` now correctly documents the
  `--strict` + `--human` flags (`--human` auto-enables on TTY);
  `rotate-known-host <vm>` (the companion to `trust`) added to the
  subcommand table and to the human/JSON output section.
- `docs/reference/cli-contract.md`: the What-is-not-in-this-contract
  section expanded. Spells out that microvm.nix internal lifecycle,
  swtpm internals, virtiofsd implementation, and polkit grant
  specifics are framework-internal; and draws the line between
  contract-bound unit names (`nixling@<vm>.service`,
  `microvm@<vm>.service`) and framework-internal unit names
  (sidecars, USBIP proxies — these MUST be read from the manifest's
  `audioService` etc. fields, not hardcoded).
- `tests/static.sh`: `nix flake check` now uses `--all-systems` so
  Layer-1 exercises both x86_64-linux and aarch64-linux flake
  outputs, not just the builder's system.
- `tests/static.sh`: 6th manifest-contract check added — diffs the
  field-name column of the prose Per-VM-entry table in
  `docs/reference/manifest-schema.md` against the JSON Schema's
  `$defs.vmEntry.properties` keys, failing the gate if either side
  has a field the other doesn't.
- README: project status now states runtime is tested on
  `x86_64-linux` desktop and eval-tested for headless
  `aarch64-linux`, reflecting cross-eval coverage.
- README: documentation section replaces a placeholder docs directory
  note with direct bullets pointing at the manifest schema and CLI
  contract under `docs/reference/`.
- `tests/README.md`: refreshed for `manifestVersion = 1`, 10/10
  assertions-eval cases, the 6-step manifest-contract gate (including
  the new md/json drift detection), and the multi-arch eval coverage.
- Diataxis reorg. `docs/manifest-schema.{md,json}` →
  `docs/reference/manifest-schema.{md,json}`; `docs/cli-contract.md`
  → `docs/reference/cli-contract.md`. Added `docs/README.md` as the
  IA index. All path references in `nixos-modules/manifest.nix`,
  `tests/static.sh`, and the moved docs' cross-links updated.
- **`nixling.vms.<vm>.ssh.keyPath` is NOT removed.** Earlier commit
  messages claimed otherwise; that was a mis-description of the
  change. The option still exists. What changed is its effective
  default: when left unset (`null`), the CLI now derives the SSH-key
  path from `nixling.site.keysDir` as `<keysDir>/<vm>_ed25519`,
  matching the framework-managed Ed25519 key generated by
  `host-keys.nix` on every activation. Consumers who explicitly set
  a path still win; the option's `null` default lets the framework
  pick. This makes the framework-managed key the zero-config happy
  path while keeping the option-shape stable for consumers supplying
  their own keys (e.g. a hardware-backed Yubikey-resident key).
- Net VM `users.allowNoPasswordLogin` is set to `lib.mkDefault true`.
  Net VMs receive SSH keys via runtime injection
  (`nixling-load-host-keys.service` reads
  `<stateDir>/vms/<vm>/host-keys/` over virtiofs); they have no
  eval-time authorized_keys. Without the flag, NixOS module-eval
  fires the `users.allowNoPasswordLogin` assertion before runtime
  injection runs. Sealed-appliance consumers can override with
  `mkForce`.
- GPU sidecar (`nixling-<vm>-gpu.service`) hardening tightened:
  `NoNewPrivileges`, `ProtectSystem=strict`, `PrivateTmp`,
  `ProtectHome`, `DevicePolicy=closed` with a `/dev/kvm` +
  render-node allowlist, `RestrictAddressFamilies =
  [ AF_UNIX AF_NETLINK AF_VSOCK ]`, `SystemCallArchitectures=native`,
  narrow `ReadWritePaths`. Two omissions documented in source
  comments: `MemoryDenyWriteExecute` (crosvm GPU JIT triggers SIGSYS)
  and `AF_VSOCK` retained (cloud-hypervisor sd_notify path).
- IPv6 disabled on workload + net VM guest networkd
  (`LinkLocalAddressing=no`, `IPv6AcceptRA=false`); net VM nft rules
  DROP `ip6` forward. Net stack is IPv4-only by construction.
- Route preflight oneshot (`nixling-net-route-preflight.service`) now
  FAILS CLOSED on conflict — exit 1 on any env-vs-route mismatch
  instead of WARN+exit 0. `RemainAfterExit=true`, `Before=` each
  enabled nixling-managed VM unit, `RequiredBy=` each wrapper, so a
  stale host route blocks VM start until the operator clears it.
- **BREAKING.** Option namespace renamed:
  - `nixling.networks.<env>` → `nixling.envs.<env>`;
  - `nixling.networks.<env>.routerName` →
    `nixling.envs.<env>.netName`;
  - `nixling.networks.<env>.extraRouterConfig` →
    `nixling.envs.<env>.extraNetConfig`.
- **BREAKING.** Per-env auto-declared VM renamed:
  `<env>-router` → `sys-<env>-net`.
- **BREAKING.** Systemd unit naming convention:
  - `swtpm@<vm>` → `nixling-<vm>-swtpm`;
  - `nixling-snd@<vm>` → `nixling-<vm>-snd`;
  - `nixling-gpu-<vm>` → `nixling-<vm>-gpu`;
  - `nixling-store-sync@<vm>` → `nixling-<vm>-store-sync`;
  - `usbipd-nixling` → `nixling-sys-usbipd`;
  - `usbipd-nixling-<env>` → `nixling-sys-<env>-usbipd-proxy`.
- **BREAKING.** System users/groups renamed: `nixling-gpu-<vm>` →
  `nixling-<vm>-gpu`, `nixling-snd-<vm>` → `nixling-<vm>-snd`,
  `swtpm-<vm>` → `nixling-<vm>-swtpm`.
- **BREAKING.** State-dir layout:
  - `<stateDir>/<vm>/` → `<stateDir>/vms/<vm>/`;
  - `<stateDir>/<env>-router/` → `<stateDir>/vms/sys-<env>-net/`;
  - `<stateDir>/swtpm/<vm>/` → `<stateDir>/vms/<vm>/swtpm/`;
  - `/run/nixling-snd/<vm>/snd.sock` →
    `/run/nixling/vms/<vm>/snd.sock`.
- **BREAKING.** Manifest JSON contract: `isRouter` → `isNetVm`,
  `routerVm` → `netVm`. Top-level `manifestVersion = 0` was added as
  a stub; a later release bumps it.
- **BREAKING.** VM/env name regex tightened from
  `^[a-z0-9][a-z0-9-]*$` to `^[a-z][a-z0-9-]*$` (require leading
  letter). Matches systemd-escape best practices; avoids ambiguity
  with tooling that treats a leading digit as a numeric index
  (`ip link show 42web-l10`). No existing in-tree names trip the
  stricter rule; consumers with numeric-prefixed VM/env names must
  rename.
- CLI: `nixling up/down/status` now target `nixling@<vm>.service`
  (the user-facing wrapper) instead of `microvm@<vm>.service`
  directly. Lifecycle propagates via the wrapper's BindsTo /
  ExecStop. Diagnostic flows (`status --verbose`, `journalctl`
  examples) keep their `microvm@<vm>` references but label them
  `backend` / `implementation detail`.
- CLI: `nixling list` / `nixling status` output tag for system VMs
  changed from `(router)` to `(net-vm)`. Helper renames:
  `ensure_router_up` → `ensure_net_vm_up`, `router_active` →
  `net_vm_active`, `IS_ROUTER` → `IS_NET_VM`. User-facing prose
  `router` / `router VM` → `net` / `net VM` (kept `routing/routes`
  only where describing the network function).
- `nixling-launcher` polkit grant tightened to an exact-unit allowlist
  generated at NixOS eval time from `cfg.vms` + `cfg.envs`, restricted
  to `start` / `stop` / `restart` verbs only. Drops the bare
  `microvm@*` prefix wildcard; default-deny invariant restored.
  Recovery / debugging paths can still authenticate via sudo or
  polkit-prompt.
- Pre-v0.1.0 breaking changes do not get a deprecation period. There
  is no compat shim for the old `nixling.networks` namespace or for
  any of the renamed unit / user / state-dir identifiers.
- The first tagged release is `v1.0.0`; the v0.x line never tagged a
  public release. These v0.x entries were the in-flight roadmap during
  the development branch and are preserved as historical record of how
  the architecture got to v1.0.
- v1.0.0 ships in lockstep with
  [`vicondoa/entrablau.nix`][entrablau] v1.0.0; consumers
  using both should pin matching tags.

### Fixed

- `tests/{static,nixling-store,audio,lib}.sh` no longer assume
  `ROOT=/etc/nixos`; the value is derived from the script's own path
  so the suite runs from any clone.
- `tests/integration/live/nixling-store.sh:33` SC2157 (preexisting).
- Host-specific `NL_FILES` entries (`vms/personal-dev.nix`,
  `vms/work-aad.nix`) dropped or guarded so the static gate stays
  useful for the public flake.
- `tests/integration/live/audio.sh` `NL_WAYLAND_USER` resolution chain genericized
  (no longer hardcoded to the maintainer's host user).
- README polish: `microVM` is defined inline on first use; a
  maintainer-anecdote phrasing was replaced with neutral wording;
  an encrypted-backup callout was added for `/var/lib/nixling/`.
- Manifest schema `manifestVersion` tightened from `minimum: 1` to
  `const: 1` so the JSON Schema matches the documented prose.
- **`nixos-modules/net.nix`:** neutralize base.nix's catch-all
  `10-eth-dhcp` systemd-networkd network on per-env net VMs. The
  catch-all (`matchConfig.Type = "ether"`) sorted lex-first against
  the per-MAC `10-uplink`/`10-lan` definitions and DHCP'd both NICs,
  preempting the static config. Now overridden via `lib.mkForce` with
  a sentinel MAC that never matches. Workload VMs are unaffected —
  they still inherit the base.nix DHCP fallback.
- **`nixos-modules/manifest.nix`:** dropped the redundant
  `default = { }` on the readOnly `nixling.manifest` option. The
  nixpkgs module system treats `default` as an extra definition;
  combined with `readOnly = true` and the matching
  `config.nixling.manifest = …` assignment, it produced
  `set multiple times` only when a graphics VM was synthesized. See
  `tests/unit/smoke/smoke-eval-graphics.nix` for the regression test.
- Inter-env CIDR overlap check now performs real IPv4 prefix
  arithmetic (`lib.cidrOverlaps` in `nixos-modules/lib.nix`) instead
  of exact-string equality. Containment (e.g. `10.0.0.0/16` ⊃
  `10.0.1.0/24`) is rejected. Env-vs-`hostLanCidrs` is checked under
  the same helper.
- `nixling.site.yubikey.enable = false` actually gates the host-side
  udev rules + `usbip-host` kernel module. Previous code declared the
  option but never read it.
- `nixling keys rotate <vm>` now scrubs the OLD pubkey from the
  guest's `~/.ssh/authorized_keys` (matched by SHA256 fingerprint)
  AFTER the new key is verified — rotation used to leave the old key
  authorized forever. Retention bounded: 3 most recent generations
  under `<keysDir>/old/<ts>/`; older are pruned post-rotation. Help
  text updated.

### Removed

- **`nixling.vms.<vm>.entra-id.*` option removed.** Himmelblau /
  Microsoft Entra ID support has moved out of the nixling framework
  and into the sibling `vicondoa/entrablau.nix` flake. To migrate,
  add the flake as an input and import its module into the VM's guest
  config:

  ```nix
  inputs.entrablau.url = "github:vicondoa/entrablau.nix";

  nixling.vms.<vm>.config.imports = [
    inputs.entrablau.nixosModules.default
  ];

  # Move each `nixling.vms.<vm>.entra-id.<key>` into the guest
  # config; see the entrablau README for the new schema.
  ```

  The `nixling.vms.<vm>.entra-id` attribute is kept as a hidden
  stub option so leftover assignments produce a readable assertion
  error (with migration instructions) instead of a cryptic
  `option does not exist` message from the module system. Final
  removal of the stub is tracked for v0.2.0.

- Three host-side activation scripts removed from
  `nixos-modules/host-activation.nix`:
  - **`nixlingSbctlBackup`** — moved maintainer-specific
    `*-backup.tar.gz` files from `$HOME` into `/var/lib/sbctl/backup/`.
    Not a framework concern. Consumers who relied on this should
    handle their own backup-file relocation outside nixling.
  - **`nixlingStoreChownRepair`** — one-shot repair for a past chown
    bug (an earlier `modules/nixling/store.nix` revision leaked
    `group=kvm` into `/nix/store` inodes via the per-VM hardlink
    farm). New installs are unaffected. Consumers upgrading from a
    pre-public nixling that ran with the buggy revision should run the
    historical repair script from `/etc/nixos` once and then drop the
    activation script there; the bug cannot recur in public code.
  - **`nixlingMigrateState`** — one-shot renamer
    (`/var/lib/microvms/<vm>` → `/var/lib/nixling/vms/<vm>`, plus
    `/var/lib/swtpm/<vm>` → `vms/<vm>/swtpm/`). New installs land
    directly on the current layout. Pre-public consumers should use
    the migration script (or perform the moves manually following the
    same logic) before switching to the public flake.

  These deletions remove all host-specific bias from the public
  framework's activation logic. The remaining two activation scripts
  (`nixlingVmStatePerms`, `nixlingNetVmVarImgPerms`, formerly
  `nixlingRouterVarImgPerms`) only adjust file ownership on per-VM
  disk images and contain no host-specific assumptions.

### Known gaps

- **USBIP per-env units materialise even when no VM opts in.** Each
  `nixling.envs.<env>` declares `nixling-sys-<env>-usbipd-backend.service`
  and the corresponding proxy socket regardless of whether any
  workload VM in the env has `usbip.yubikey = true`. The units are
  idle when nothing opts in, but they are still installed. The
  unconditional materialisation is the gap. Tracked for v0.2.0; the
  relevant conditional belongs around `nixos-modules/network.nix:484-650`.
- **No static lint for `mkOption { default = …; readOnly = true; }`
  + matching `config.<…>` assignment.** The issue was caught by
  review, not by tooling. A follow-up will add a grep-level lint to
  prevent the `default + readOnly + config-assignment` trio from
  re-appearing. Trio detection is necessary because `store.nix`
  legitimately carries `readOnly + default` on options that have NO
  matching `config.<…>` assignment, so a two-of-three match is fine;
  only the full three is a bug.
- **Per-example flake-check loop is not fully hermetic for
  `examples/with-entra-id`.** `tests/static.sh` iterates
  `examples/*/flake.nix` and runs `nix flake check --no-build
  --all-systems` per example, but `with-entra-id` depends on the
  sibling `vicondoa/entrablau.nix` flake which the core flake
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

[entrablau]: https://github.com/vicondoa/entrablau.nix
