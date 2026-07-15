# Changelog

All notable changes to d2b are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Pre-1.0 minor releases may break public APIs. When practical,
deprecations ship one minor release before removal.

## [Unreleased]

### Added

- Added default-empty, non-publishable first-party provider implementation
  crate reservations, an in-process Azure VM fake SDK boundary, and a
  fail-closed workspace policy inventory for their naming and dependencies.
- Added canonical d2b 2.0 implementations for Unix sessions, authenticated
  sessions, providers, provider tooling, state persistence, and clients.
- Added Tokio-async client/session/provider APIs and explicit `spawn_blocking`
  state adapters so synchronous filesystem and lock calls do not block runtime
  workers.
- Added independent Rust and pure-Nix canonical v2 identity derivation with
  shared vectors, plus generated schemas and references for identity,
  ComponentSession, providers, services, and state/storage/sync/audit.
- Added the strict d2b 2.0 provider contract rail with eleven closed authority
  axes, bounded descriptors and registries, shared in-process/agent traits,
  explicit lifecycle and adoption bindings, and co-located opaque credential
  leases.
- Added reusable provider-toolkit value builders and exact-descriptor
  conformance fixtures that preserve operation, owner, generation,
  fingerprint, correlation, and time bindings without exposing identifiers in
  diagnostics.
- Added the complete internal ComponentSession v2 protobuf/ttrpc service
  inventory, strict bounded wire validation, schema fingerprints, reference
  artifacts, and generated asynchronous client/server bindings.
- Added a hermetic async ttrpc API-fit contract that pins the selected runtime
  and generator versions, commits drift-checked test bindings, and proves a
  yielding Unix-socket round trip without freezing a production service schema.
- Added the checked-in W1 delivery authority manifest so the wave is validated
  and sealed by the same immutable workflow introduced for later waves.
- Added Rust delivery tooling for validated stack graphs, immutable
  candidate-addressed snapshots, authority-captured validation evidence,
  exact-role panel attestations, non-circular wave seals, history-only reuse
  proofs, and fail-closed merge eligibility.
- Added reproducible delivery packages and a default development shell with
  Git Town, GitHub CLI, pinned-nightly `cargo-udeps`, and
  `cargo-semver-checks`; stacked-base PR workflows now emit tree-bound check
  summaries.

### Changed

- Changed observability provider queries to return a canonical bounded result
  through the provider trait, proxy, protobuf/ttrpc service, and toolkit
  adapter without dropping records or introducing free-form labels.
- Added an exact user-agent placement binding for userd-owned Secret Service
  leases while preserving provider-agent co-location for cloud credentials.
- Bound transport connection requests to an exact opaque transport binding ID.
- Strengthened provider dependency policy to inspect transitive Cargo package
  identities and keep denied Azure VM scaffolds out of production graphs.
- Split `d2b-contracts` into default-empty, least-privilege feature families,
  migrated every workspace consumer to explicit domains, and reserved
  independent empty d2b 2.0 identity, component-session, service, provider,
  and state contract rails.
- Unified all maintained host and guest Rust crates at version 2.0.0 under one
  workspace, toolchain policy, dependency graph, and canonical lockfile while
  retaining focused broker feature and static guest validation.
- Standardized common-API implementation crates on sortable
  `<base>-<implementation>` names, including `d2b-provider-host` and
  `d2b-session-unix`, and made the workspace member inventory
  alphanumerically ordered.
- Kept sccache enabled throughout generated CI and persisted its local cache
  without exposing cache-service credentials to compiled code.
- Replaced the unavailable GitHub stack preview dependency with locked Git
  Town 23.0.1, ordinary GitHub PR authority, recursive parent-graph
  verification, and a typed fail-closed capability check.
- Bound delivery snapshots and seals to checked-in manifests, Git Town parent
  graphs, exact GitHub PR/check publishers and OIDs, verified
  validation and panel provenance, portable multi-repository content IDs,
  private XDG state, history-only proofs, and fail-closed merge authority.
- Hardened delivery sealing with real nested GitHub check-suite metadata,
  retained offline CI attestation bundles, provider-rooted signed panel
  receipts, fresh history CI run identities, base-relative diff bindings,
  squash/rebase/merge stack progression, immutable validation checkouts, and
  exact base-and-head merge compare-and-swap requirements.
- Moved Layer-1 manifest validation, bounded local orchestration, flake-check
  discovery, and generated workflow ownership from Python and Bash into Rust
  `xtask`, and enabled the generated PR workflow for stacked non-main bases.
- Replaced serial phase gating and the three-unit process assumption with
  speculative Git Town PR stacks, exact-tree concurrent CI/validator/panel lanes,
  external `xtask` evidence and seals, full ten-role end-of-wave review, and the
  accepted local-root plus parent-spawned per-realm controller/broker model.
- Made post-wave cleanup delete merged remote branches, finished worktrees and
  their real Cargo targets, and local feature branches before running Nix GC.
- Expanded and accepted ADR 0045 as the d2b 2.0 clean-break contract: destructive
  no-backup reset, universal authenticated ComponentSession IPC, eleven typed
  provider authorities, literal per-realm controller/broker boundaries,
  short-ID state ownership, scoped secret/key lifecycles, one Cargo workspace,
  coordinated toolkit cutover, and immutable-tree delivery seals.
- Replaced generic provider request state, binding, and stream fields with an
  exact closed operation-input union, method-specific compatibility checks,
  strict protobuf conversion, and one shared provider dispatchability policy.

### Fixed

- Closed d2b 2.0 foundation integration gaps with a driven ComponentSession,
  authenticated two-phase descriptor binding, phase-aware peer credentials,
  canonical provider-agent serving, lock-bound state generations, and bounded
  application-gated logical streams, cancellation-safe async kernel adapters,
  framed Unix streams, transport-scoped routing, adapter-owned out-of-order
  ttrpc correlation, terminal stream cleanup, concurrent named-stream
  demultiplexing, actionable closed error context, transfer-safe OFD locks, and
  race-free provider drain.
- Isolated standalone proof targets by proof and pinned Rust toolchain so
  concurrent immutable Layer-1 validation cannot reuse incompatible rustc
  metadata from the main workspace.
- Pinned the exact `rustc`, `rustdoc`, and `clippy-driver` executables used by
  workspace and proof Cargo commands, and separated proof clippy/test targets,
  preventing intra-gate compiler selection drift.
- Shipped the complete pinned stable Rust distribution in delivery shells and
  asserted Clippy's compiler identity, preventing fallback to a newer host
  `clippy-driver`.
- Closed W2 service-contract ambiguity by making provider capability claims
  method-exact, binding response attachments and streams, rejecting mixed
  identity scopes and contradictory error outcomes, returning exact typed
  validation errors, making role scopes and audit actors workload-bound,
  keeping schema and serde enum-field casing identical, and distinguishing
  `SCM_CREDENTIALS` records from pidfds.
- Made pure-Nix identity checks and rejection vectors portable to
  implementations that cannot materialize NUL-containing JSON strings, and
  corrected generated tuple-struct API documentation.
- Moved the async ttrpc API-fit socket fixture to the validation-owned socket
  root so canonical read-only source checks can execute it.
- Made W2 delivery authority fingerprint each generated service binding and
  source protobuf as a tracked blob instead of using unsupported directories.
- Kept unified toolchain-scoped Cargo gate targets out of CI caches while
  retaining `.sccache`, and installed pinned sccache for release builds.
- Hardened final delivery authority with secret-free validation environments,
  authenticated merged-prefix topology, tolerant external GraphQL decoding,
  shared signal forwarding, and kernel-backed child-output polling.
- Isolated Cargo state for validation commands and normalized newly created
  private directories before restrictive umasks can prevent fd anchoring.
- Preserved controlled HOME/XDG configuration only for trusted Git Town
  authority probes after clearing ambient validation environments.
- Made delivery state use true open-file-description locks and exact
  post-creation private modes independent of the caller's umask.
- Preserved Unix signal termination in Layer-1 results and required canonical,
  duplicate-free fingerprint declarations.
- Kept signal and umask regression coverage Rust-native so the no-shell AST gate
  remains closed.
- Routed Layer-1 logs and shell-test scratch state to the private validation
  output root when the reviewed checkout is read-only.
- Routed pinned Cargo gate targets and proof sockets through writable,
  validation-owned paths while keeping Unix socket names short.
- Kept tracked validation sources read-only while allowing only private,
  ignored Cargo target directories to remain writable.
- Added ShellCheck to the delivery environment and routed the remaining short
  CLI socket fixture through the validation socket root.
- Routed guestd credential, activation, guest-file, and output-ring fixtures
  through validation-owned scratch when the checkout is read-only.
- Made deliverable regression-gate discovery deterministic under `pipefail`
  instead of relying on a SIGPIPE-prone `grep | head` pipeline.
- Made local flake-shard discovery retain command failure, retry one transient
  evaluator/daemon disconnect, and fail explicitly if discovery still fails.
- Updated flake-matrix drift enforcement to require cache-preserving discovery
  instead of the retired Rust wrapper-clearing command.
- Routed unsafe-local runtime, readiness, and supervisor-socket fixtures through
  the short validation socket root when the source checkout is read-only.
- Hardened delivery evidence and summaries by reporting output truncation,
  removing private job logs unless explicitly retained, preserving complete
  Markdown detail blocks, rejecting extra panel JSON, and recovering from stale
  immutable-write temporary names.
- Corrected the panel attestation example to include its required receipt
  locator.
- Kept anchored snapshot state alive through eligibility and atomic merge,
  closing a snapshot replacement race, and recognized fresh status contexts,
  third-party check runs, and same-workflow GitHub reruns without weakening
  timestamp or run-identity checks.
- Made terminal signals operation-scoped so interrupts between supervised
  subprocesses unwind normally and remove read-only validation checkouts.
- Described panel model enforcement as a candidate-bound contract rather than
  embedding a provider-specific review attribution in process prose.
- Corrected the strict panel receipt example to use the implemented artifact
  kind and provider identifiers.
- Synchronized the independent documentation policy test with the 13-field
  panel receipt contract.
- Added pinned OpenSSL to the delivery app runtime so panel receipt signature
  verification works outside the development shell.
- Corrected the unsafe-local graphical readiness test to exercise its declared
  runtime-directory socket instead of a checkout-relative shadow path.
- Routed d2bd config, broker-dispatch, and security-key test fixtures through
  validation-owned output and short socket roots.
- Routed xtask signal-marker and dangling-symlink fixtures through validation
  output and kept signal-test diagnostics independently bounded.
- Routed broker media and guest-control signing tempdirs through validation
  output instead of the read-only crate root.
- Deferred package-local target-island removal to validation checkout cleanup
  when the target's source parent is intentionally read-only.
- Made schema reproducibility generate twice into validation-owned scratch
  instead of rewriting committed schema paths.
- Made broker nextest inventory use the committed lock fail-closed instead of
  backing it up and restoring it through the source tree.
- Made canonical drift generation run in a writable detached clone while
  preserving the in-place developer path and sccache wrapper.
- Installed pinned sccache in the non-generated Rust eval workflow and enforced
  its cache environment in policy tests.
- Quoted generated workflow display names, percent-encoded local flake URIs,
  and made fresh-CI ordering robust to later updates of an older workflow run.
- Made terminal-signal cleanup coverage use explicit same-group Rust children,
  eliminating shell job-control races.
- Pinned Git Town and GitHub CLI source builds and separated stacked-wave
  procedures from the delivery contract reference.
- Reconstructed merged stack prefixes from historical GitHub merge authority
  and authoritative manifest order after Git Town removes merged lineage,
  while requiring exact retargeted local lineage for every remaining active
  branch.
- Expanded W1 delivery fingerprints into explicit tracked source blobs so the
  immutable snapshot covers the complete delivery and Layer-1 implementations.
- Removed the redundant guest-only deny check after guest crates joined the
  canonical workspace, and made binary banners report the 2.0.0 package version.
- Hardened delivery subprocess supervision against PID reuse, orphaned process
  groups, descendant-held pipes, dangling symlinks, ambiguous Git remotes, and
  lost bounded diagnostics.
- Staged external attestations and panel receipts into private immutable state
  before verification, retained bounded validation output, and made delivery
  state access fd-relative with nonblocking OFD locks.
- Made generated Layer-1 CI evaluate exact pull-request heads, redact workspace
  paths from failure tails, and publish bounded failure summaries to GitHub.
- Aligned delivery command documentation, Nix tool pinning, and the flake-check
  matrix with the implemented fail-closed workflow.
- Hardened Layer-1 scheduling and generated CI rollups against slot underfill,
  skippable-only dependency coverage, invalid workflow job IDs, option-like Make
  targets, and clean environments without `sccache`.
- Aligned delivery documentation and fail-closed policy checks with the
  hardened candidate/content identifiers, external panel attestations,
  manifest-driven Layer-1 commands, and PR-before-final-gates order.
- Delayed host-terminal first-client readiness until the terminal child remains
  alive through a bounded stabilization window, preventing desktop launchers
  from reporting success when WezTerm immediately rejects its configured
  domain, and made child-exit observation event-driven through its pidfd.
- Derived `persistent-shell` and `pty` workload capabilities from
  `shell.enable`, keeping provider-neutral desktop discovery consistent with
  the generated shell launcher item and guest runtime.
- Removed stale per-VM observability relay sockets before broker spawn so a
  normal VM down/up cycle cannot fail with the relay address still in use.
- Kept persistent-shell integration scratch paths within Linux UNIX-socket
  limits when tests run from long worktree paths.
- Rejected local-VM realm workloads that advertise persistent shells while the
  referenced guest has shell support disabled, preventing desktop clients from
  exposing a terminal action that can only fail at runtime.
- Made exec-runner drain tests tolerate saturated CI scheduling while
  preserving clean-EOF and leaked-pipe coverage.
- Made unsafe-local helper FD cleanup tests track the original kernel object so
  concurrent numeric descriptor reuse cannot report a false leak.
- Removed a scheduler-sensitive wall-clock assertion from zombie process
  detection coverage while retaining the behavioral timeout check.
- Isolated the pinned Rust gate's Cargo outputs by toolchain so developer builds
  from another compiler cannot poison doctest metadata, and kept those compiler
  artifacts out of CI cache restoration.
- Reused an already-installed pinned Rust toolchain before attempting a network
  bootstrap.
- Made broker audit tests reserve exclusive process-local scratch directories
  before opening their logs.
- Hardened the delivery environment with native build tools and `sccache`,
  built the delivery app with pinned Rust 1.94.1 on both supported systems,
  kept delivery evidence outside Git metadata, and bound non-generated PR
  workflow summaries to the exact checked-out candidate commit.

### Removed

- Removed the superseded standalone Wayland-proxy feasibility crate, redundant
  nested Cargo workspaces and lockfiles, and the stale duplicate
  activation-helper source.

## [1.4.1] - 2026-07-12

### Added

- Added ADR 0045, defining parent-owned workload-hosted realm
  controllers; explicit runtime, infrastructure, transport, substrate,
  credential, and display provider responsibilities; type-first sortable
  provider crates; generic Unix/vsock/direct/Azure-Relay byte transports;
  Noise-authenticated component sessions with ttrpc/protobuf control services;
  Entra and YubiKey credential placement; and policy-authorized peer shortcuts
  over inherited shared transport fabrics.

### Fixed

- Fixed unsafe-local persistent shells inheriting `TERM=dumb` from the systemd
  user manager by supplying a fixed true-color terminal baseline while
  preserving the rest of the manager environment and login-shell startup.
- Made the mkfs diagnostic bound test exercise the formatter directly instead
  of depending on unrelated existing-image repair stages.
- Made output-ring wake coverage observe data and EOF as separate valid
  notifications instead of racing both producer events into one read.
- Made disk-init test directories use exclusive process-local IDs so parallel
  tests cannot silently share and remove each other's scratch state.
- Made failed-fd-send coverage track the original pipe identity so concurrent
  numeric fd reuse cannot produce a false leak report.
- Stabilized shell-supervisor teardown coverage by allowing its asynchronous
  socket cleanup the same bounded reconciliation horizon used by the runtime,
  waking the supervisor accept loop so its owned listener unlinks before forced
  scope teardown, and ensuring a missing/replaced control socket cannot block
  verified scope collection or ledger cleanup.

- Fixed the provider-neutral `launch` command missing from the public
  authorization matrix and generated privileges schema. Configured launches
  remain scoped per workload/realm to launcher or admin callers, audited, and
  broker-free.

## [1.4.0] - 2026-07-12

### Added

- Added the realm-native control plane under `d2b.realms.<realm>`, including
  canonical `<workload>.<realm-path>.d2b` targets, provider-neutral workload
  identity, realm network and UI metadata, generated realm artifacts, bounded
  realm/operation inspection commands, and metadata-only topology, access, and
  resource-allocation layers.
- Added the explicit, default-denied `unsafe-local` provider for host-user
  workloads. Generic typed `exec` and `shell` launcher items now work across
  local VMs, qemu-media, and unsafe-local targets through `d2b launch`, with
  persistent host shells, same-UID helper supervision, Wayland identity rails,
  and visible no-isolation posture.
- Added daemon-owned serial-console and audio operations, including
  provider-capability dispatch and host/guest mute and volume controls.
- Added an opt-in FIDO2/WebAuthn security-key proxy that presents a host device
  to opted-in guests as virtual HID over vsock without transferring USB
  ownership, plus status, session, cancellation, test, and notification
  commands.
- Added explicit USB attachment for any physically present device to an
  eligible VM, with preflight validation, audited ownership, and rollback.
- Added the opt-in `d2b-clipd` clipboard authority, picker-driven cross-realm
  paste, virtualized guest clipboard transport, and Niri focused-window
  attribution.
- Added a compositor-agnostic UI color contract rendered as
  `/etc/d2b/ui-colors.json` and `/etc/d2b/ui-colors.css`, with a Niri VM-border
  backend and per-realm accent metadata.
- Added macvtap-backed external network attachment for env net VMs, including
  independent egress NAT, port forwards, and mDNS/`.local` reflection.
- Added generated storage and synchronization contracts, read-only startup
  validation, `d2b host doctor --read-only`, and
  `d2b host migrate-storage --dry-run`.
- Added provider-aware graceful VM shutdown with configurable global and per-VM
  timeouts, plus explicit `--force` lifecycle overrides.
- Added experimental remote full-host and provider-managed Azure Container Apps
  adapters with capability matrices, bounded backoff/circuit behavior, and
  redacted diagnostics. Production remote transport remains out of scope.
- Added release automation that creates a version tag and GitHub release with
  host binaries, checksums, and a Nix hash manifest when a dated changelog
  section reaches `main`.

### Changed

- **Breaking:** Renamed the project to **d2b: Double Dutch Bus**. Commands,
  packages, services, sockets, Nix options, paths, schemas, and telemetry now
  use only `d2b` naming; no legacy aliases are provided.
- **Breaking:** Removed the legacy `d2b.gateways` and nested gateway/ACA sandbox
  configuration surfaces. Configurations must migrate to `d2b.realms`.
- **Breaking:** Explicit `d2b://` targets must include the reserved `.d2b`
  suffix; omitted suffixes no longer fall back to local VM routing.
- **Breaking:** Unsupported constellation streams and operations now return
  typed unsupported errors instead of falling back to generic byte streams.
- **Breaking:** VMs using `usbip.yubikey = true` must enable guest control;
  USBIP attach and detach no longer have an SSH fallback.
- Advanced the public manifest schema to version 7 and the private bundle
  contract to version 11. The release adds runtime/provider capabilities,
  graceful-shutdown metadata, realm artifacts, configured launcher items,
  unsafe-local helper policy, and storage/synchronization contracts.
- Renamed the Wayland proxy package and binary to `d2b-wayland-proxy` and the
  configuration surface to `graphics.waylandProxy.*`. The former option path is
  retained as a compatibility alias for this release.
- Moved audio process identity entirely into the daemon-managed audio runner and
  retired the former audio service path.
- Changed live VM activation to a broker-prepare, guest-control activation, and
  broker-commit flow. Offline activation now fails closed except for explicit
  boot staging.
- Changed daemon list/status handling to use request-scoped artifact snapshots
  and parallel per-VM status probes, improving consistency and desktop-client
  latency.
- Changed runtime/state creation to rely on tmpfiles-owned parents and
  narrowly-scoped ACLs instead of activation-time permission repair.
- Changed `d2b-priv-broker.service` default logging from `debug` to `info`.

### Fixed

- Fixed unsafe-local graphical launches by supervising the proxy and configured
  app in one verified user scope with private runtime paths, typed readiness,
  first-client gating, bounded socket names, exact child reaping, canonical
  realm colors, and no direct-compositor fallback.
- Fixed picker/clipboard protocol compatibility, focus restoration, proxied
  virtual-keyboard replay, endpoint payload handling, cancellation, and
  backpressure while preserving picker-only transfer authority.
- Fixed USBIP claim, bind/unbind, firewall, ACL rollback, restart reconciliation,
  and revocation races; hardened security-key UHID framing, socket lifetime, and
  udev behavior.
- Fixed console and audio session ownership, QMP chardev handling, PipeWire
  targeting, and provider dispatch.
- Fixed daemon restart and host-switch continuity: `d2bd.service` reports ready
  only after socket bind and runner adoption, while running VMs remain alive.
- Fixed guest exec and GUI launch establishment timeouts under heavy virtiofs
  load.
- Fixed runtime, state, guest-control, observability, and per-role ACL ordering
  so daemon and runner access survives reboot and host switches without local
  overrides.
- Fixed net-VM cold-boot host preparation, qemu-media synchronization contract
  rendering, broker child reaping, and existing disk-image validation.
- Fixed realm controller and workload identity JSON field names and nesting to
  match their Rust DTOs.
- Fixed realm workload CLI routing, bare-VM migration hints, persistent-shell
  owner framing, and guest journal sizing.

### Removed

- Removed `d2b usb enroll`; qemu-media USB boot selection now uses
  `qemuMedia.source.usbSelector.byIdName` and `d2b usb probe`.

### Security

- Kept realm relay/provider credentials, remote registries, and realm audit out
  of the host daemon, broker, and bundle; relay identity is never mapped to
  local lifecycle authorization.
- Enforced same-UID unsafe-local helper registration, private proxy/readiness
  sockets, immutable proxy binaries, operation fingerprint parity, fail-closed
  group eligibility, and explicit logout/login after new group assignment.
- Enforced picker/clipd-only cross-realm clipboard transfer, strict bounded and
  redacted protocol metadata, destination-focus verification, and proxy-safe
  synthetic paste ordering.
- Tightened broker, runtime, qemu-media, observability, and per-role path
  ownership so diagnostics remain redacted and mutable host state stays within
  its declared authority.

## [1.3.1] - 2026-06-18

### Fixed

- Nix packaging now keeps legitimate source files whose names contain
  `target` (for example `d2b-constellation-core/src/target.rs`) while
  still filtering Cargo `target/` build directories out of package sources.
- USBIP lock acquire is now idempotent for the same VM: when a VM is
  restarted (`d2b down` + `d2b up`), the broker no longer
  refuses to re-bind a busid that the same VM already owns. Previously,
  every VM restart required a manual `d2b usb detach` + `d2b usb
  attach` cycle because the lock file persisted across the stop/start.
## [1.3.0] - 2026-06-18

### Fixed

- `tpm.enable` first-run: enabling TPM on a VM with no pre-existing
  `/var/lib/d2b/vms/<vm>/swtpm` state directory no longer wedges
  the VM's start. The privileged broker now provisions the per-VM
  swtpm state directory (owner `d2b-<vm>-swtpm`, mode `0700`) on
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

- The per-VM state root `/var/lib/d2b/vms/<vm>/` is now `3770`
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
  names; each shard runs `make test-flake` with `D2B_FLAKE_CHECK=<name>` to
  instantiate a single check in its own evaluator process). This replaces the
  monolithic `nix flake check`, which evaluated every nixosSystem toplevel in
  one process and OOM-killed the 16 GB runner (kept alive only by a 14 GB
  swapfile, ~41 min). A companion `flake-eval-x86-outputs` job evaluates the
  non-`checks` x86 outputs (`packages.*`, via `D2B_FLAKE_OUTPUTS=1`) that the
  per-check shards don't cover and the aarch64 leg (which only evaluates aarch64
  outputs) would miss. A stable `test-flake-x86` aggregator job gates on all of
  them to preserve the required status context, and a fail-closed drift gate
  (`tests/unit/gates/flake-check-matrix-sync.sh`, run by `make test-drift`,
  regenerate the pin with `make flake-matrix-pin`) keeps the CI shard matrix in
  sync with the flake's check set. The aarch64 leg still runs the full
  monolithic check.
- CI: the `test-rust` gate now restores/saves an sccache **local-disk** cache
  via `actions/cache` (opt-in through the new `D2B_CI_SCCACHE=1`, honored by
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
  ([ADR 0032](docs/adr/0032-d2b-v2-constellation-control-plane.md)),
  with **no user-facing behavior change**: `d2b-constellation-core`
  (the pure, codec-neutral model — strongly-typed identifiers with
  fail-closed deserialization, the capability model, the semantic
  `ConstellationFrame` with a trusted per-operation required-capability
  mapping, the redacted audit envelope, and a bounded trace context) and
  `d2b-constellation-provider` (the async provider trait surface —
  runtime/workload/display/transport/stream-mux/codec/credential/
  daemon-access providers — with typed capability descriptors, structured
  capability-denial errors, byte-carrying transport sessions, and
  fail-closed mock/conformance fixtures). The same change adds the
  remaining foundation crates: `d2b-constellation-codec-protobuf`
  (a `prost` codec behind the `ProtocolCodec` trait, with frame-cap and
  fail-closed decode validation), `d2b-constellation-transport`
  (an in-memory loopback transport for conformance),
  `d2b-constellation-router` (the codec-neutral operation router +
  single-owner idempotency/dedup store keyed by the full operation
  namespace), `d2b-daemon-access` (the transport-neutral CLI↔daemon
  semantic API with its current local-Unix binding), `d2b-provider-host`
  (byte-identical local adapters over the existing Cloud Hypervisor and
  cross-domain Wayland argv generators), plus compile-only constellation
  peer-module skeletons inside `d2bd`. These crates are the foundation
  for later ADR 0032 work; they do not change any CLI, daemon, or
  on-host behavior.

- Documentation for the v2 constellation control plane
  ([ADR 0032](docs/adr/0032-d2b-v2-constellation-control-plane.md)):
  the threat model in `docs/explanation/design.md` now describes the
  realm-gateway trust boundary — the host daemon and broker hold no
  realm relay/provider credentials, remote node registries, or realm
  audit (those live inside a per-realm gateway guest VM); a realm relay
  is an untrusted, ciphertext-only rendezvous transport; relay identity
  is never local authorization (`SO_PEERCRED` + the `d2b` group
  remain the only local lifecycle authz surface); and work and personal
  realms never share a gateway guest or an L2 bridge. `SECURITY.md`,
  `docs/reference/privileges.md`, `docs/reference/daemon-api.md`, and
  `docs/reference/daemon-audit-check.md` are updated to state the same
  relay-is-not-local-auth and no-host-held-realm-credentials boundary.

- Host OTel collector parity (ADR 0033). New
  `d2b.observability.host.*` options bring the host edge collector to
  parity with the per-VM guest collector: `host.scrapeJournal` adds a host
  `journald` receiver (severity-mapped, restart-resuming `file_storage`
  cursor) and `host.otlpIngest.enable` adds a host-local OTLP ingest
  endpoint (a Unix socket in a dedicated `/run/d2b/otel/ingest/`
  subdirectory, isolated from `host-egress.sock`) plus a `traces` pipeline
  and `otlp` on the `metrics`/`logs` pipelines. Both default off and ship
  over the existing host → `sys-obs` vsock bridge (never a LAN).
  `host.otlpIngest.clientGroup` optionally widens the ingest socket from
  `0600` to a `0660` group. See
  [ADR 0033](docs/adr/0033-host-collector-parity.md).

### Changed

- All Rust workspaces (main + `d2b-priv-broker`) moved to **Rust
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
  `host.name` (via `d2b.observability.host.identityName`, default
  `networking.hostName`), assigned at the trusted ingress boundary, rather
  than the literal `"host"`. `vm.role` stays `"host"`. This is a default
  label change for observability-enabled hosts even with the new receivers
  off; set `d2b.observability.host.identityName = "host"` to keep the
  old labels. See [ADR 0033](docs/adr/0033-host-collector-parity.md).

- `ReadGuestFile` guest-control RPC: a single-shot, bounded, enum-keyed
  (initially `GuestConfig`-only) RPC for the host to read a small,
  trusted in-guest file over the authenticated vsock channel.
  `d2b-guestd` resolves the path with a safe `openat` from a trusted
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
  mode of the static `d2b-exec-runner` performs the
  `setsid` + `TIOCSCTTY` + `tcsetwinsize` + `dup2` + `execve` handshake in
  safe `rustix`, so `d2b-guestd` never acquires a controlling
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
  non-TTY `d2b vm exec -d` for detached commands. See
  [`docs/reference/guest-control-exec-interactive-tty.md`](docs/reference/guest-control-exec-interactive-tty.md)
  and the interactive-exec section of
  [ADR 0028](docs/adr/0028-guest-control-plane-over-vsock.md). The
  guest-control wire contract is unchanged (the TTY surface was already
  present).

- New per-VM option `d2b.vms.<vm>.guest.exec.interactiveMaxRuntimeSec`
  (default `0` = unlimited) caps interactive TTY exec runtime
  independently of the non-interactive attached ceiling. It is mirrored
  read-only into the guest config and forced from the host module, and
  emitted to `d2b-guestd` as `--interactive-max-runtime-sec`
  alongside the detached exec surface.

- Guest exec now accepts bare command names and relative program paths in
  both attached and detached modes. `guestd` passes `argv[0]` through the
  workload user's login shell (`exec "$@"`), so the command is resolved
  by that user's login `PATH`; invalid program names get the distinct
  `INVALID_PROGRAM` / `guest-control-invalid-program` error. The
  console replacement is `d2b vm exec -it <vm> -- bash`.

- Detached workload-user exec is enabled with
  `d2b vm exec -d <vm> -- <cmd>` and VM-first management verbs:
  `d2b vm exec <vm> list`, `logs <exec_id>`, `status <exec_id>`,
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
  `/run/d2b/otel/` are eligible.

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

- Detached exec (`d2b vm exec -d`) now works end-to-end. Three faults in
  its initial implementation are fixed: the per-VM exec runner verified the
  workload's cgroup placement against a top-level `d2b-exec.slice` path
  even though systemd nests it under `d2b.slice`, so every detached
  command was killed at spawn; the daemon panicked (taking down `d2bd`)
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

- `d2b vm exec <vm> -- <cmd…>` (and `-it` for an interactive TTY):
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
  detached commands use `d2b vm exec -d <vm> -- <cmd>`. Attached exec
  establishes one redacted kind=critical audit event (vm / peer uid / tty
  only), and detached create/kill adds redacted daemon audit carrying only
  vm / peer uid / result / exec id. Opaque session handles, argv, and
  stdio/env/cwd/paths never reach any log, span, audit record, or metric
  label.

- Detached guest exec: `ExecCreate(detach=true)` runs a non-interactive
  command that outlives the originating connection, supervised by the root
  guest daemon through slot-based `systemd-run` transient units
  (`d2b-exec-<NN>.service`, scoped to a guest-internal `d2b-exec`
  slice). Unit names and argv carry only the slot index — never the exec id,
  argv, environment, or cwd. stdout/stderr are retained in slot-keyed files
  under a root-owned, 0700, boot-scoped `/run/d2b-exec` parent with
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
  substrate as `d2b vm exec -d <vm> -- <cmd>` plus
  `d2b vm exec <vm> list|logs|status|kill` management verbs.

- `ExecList` RPC (guest-control protocol version 2): a minimal, read-only
  discovery call that enumerates the caller's detached execs for the same
  VM token + boot (bounded ≤32). Each entry carries the exec id, slot,
  state, create time, an argv SHA-256 hash (never raw argv), and per-stream
  truncation/dropped-byte counters. The CLI and public daemon DTOs do not
  expose the argv hash.

- `ExecExpired` guest-control error kind, distinguishing a retention-evicted
  detached record from `StaleSession` (boot mismatch) and `ExecNotFound`
  (unknown id).

- Host VM option `d2b.vms.<vm>.guest.exec.detachedMaxRuntimeSec`
  (unsigned, default 0 = indefinite) plumbed through to the guest exec
  runtime as a per-exec `RuntimeMaxSec` backstop when non-zero.


  `packages/d2b-contracts/proto/guest_control.proto` — generated schema plus
  protobuf source for the ADR 0028 ttRPC contract, covering health, Hello,
  capabilities, exec lifecycle, chunked stdio RPC shapes, bounded health
  labels, bounded string identifiers/payload metadata, oneof-style terminal
  status, structured stdio error results, and descriptor-shape drift checks.

- Initial guest-side Rust crates for the guest control plane:
  `d2b-guestd`, `d2b-userd`, and `d2b-exec-runner`, with
  fail-closed binaries, fakeable daemon/user/session traits, and bounded
  runner input validation.

- Bootstrap/fail-closed guest-static package outputs `d2b-guestd-static`,
  `d2b-userd-static`, and `d2b-exec-runner-static`, plus an ELF check
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

- `d2bd` now has an internal Cloud Hypervisor CONNECT helper for the
  guest-control transport port. This is transport groundwork only: it does not
  change VM readiness, status output, CLI help, or exec behavior.

- `packages/d2b-contracts/src/generated/guest_control.rs` now contains committed
  protobuf message bindings generated from
  `packages/d2b-contracts/proto/guest_control.proto` via
  `cargo run --locked --manifest-path packages/Cargo.toml -p xtask -- gen-guest-proto`.
  The new
  `tests/guest-proto-bindings.sh` gate verifies the generated bindings are
  deterministic, unsafe-free, and message-only (no ttRPC runtime stubs).

- Guest-control protobuf now has an authenticated `Authenticate` handshake:
  `Hello` is challenge-only, authenticated health/capabilities are returned
  only after proof-of-possession, and `d2b-guestd` has a pure auth core
  with fixed-size HMAC transcript tests. No listener, readiness, or exec CLI
  behavior is enabled yet.

- `d2b-guestd` now owns generated ttRPC service bindings and a dormant
  `--serve --vm-id <vm>` service mode for Hello challenge, Authenticate, and
  authenticated Health/Capabilities. The guest service remains opt-in manual-start only
  (`wantedBy = []`) and does not enable host readiness or exec behavior.

- The privileged broker now exposes a structured guest-control HMAC signer, and
  `d2bd` has a host-side authenticated Health probe helper. The helper
  produces daemon-local health evidence only; it does not replace SSH readiness
  or enable exec.

- Guest exec policy option `d2b.vms.<vm>.guest.exec.enable` gates guest
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
  duplex-stream exec I/O design using d2b `TerminalFrame` messages,
  explicit byte credit, close/EOF, resize/signal/exit/error frames, CLI
  behavior, conformance matrix, risks, and required tests.

- Guest systemd-journal log collection. The per-VM OpenTelemetry
  collector now follows the guest journal through the contrib `journald`
  receiver and forwards it to SigNoz as logs tagged with the VM's
  `vm.name` / `vm.env` resource attributes, with the journal `PRIORITY`
  mapped to a readable OTel severity (`INFO`/`WARN`/`ERROR`/…) and a
  `file_storage` cursor so a collector restart resumes without dropping
  entries. `d2b.vms.<vm>.observability.scrapeJournal` now defaults
  to `true` (previously a reserved no-op) and the guest collector user
  is granted `systemd-journal` read access plus `journalctl` on its
  unit PATH. Ingested telemetry's `deployment.environment` resource
  attribute is the physical host machine name (from the host's
  `networking.hostName`, settable via `d2b.observability.hostName`)
  so SigNoz groups VMs by the host they run on; the per-VM env stays on
  `vm.env` / `service.namespace`.

- Native, container-free SigNoz observability backend packages and ADR.
  The bundled observability path now targets SigNoz, the SigNoz OTel
  Collector, schema migrator, ClickHouse, and ClickHouse Keeper as native
  NixOS services.

- `d2b.site.niriVmBorders.{enable,outputPath}` — opt-in niri KDL
  window-rule include generator. When enabled, installs a KDL file at
  the configured path (default `/etc/d2b/niri-vm-borders.kdl`)
  containing a crosvm scanout-window hide rule and one
  `window-rule` per enabled graphics VM. Rules match the
  `d2b.<vm>.` app-id prefix that the host Wayland filter proxy
  writes onto guest windows. Include the file from niri config with
  `include "/etc/d2b/niri-vm-borders.kdl"`. Requires niri ≥ 0.1.9.
- `d2b.vms.<vm>.graphics.niriBorderColor` — per-VM active border
  color override for the generated niri rules, as a six-digit CSS hex
  color (`#rrggbb`). Defaults to `null`, which uses a deterministic
  palette color derived from the VM name.
- `d2b.vms.<vm>.graphics.waylandFilter.{enable,denyGlobals,allowGlobals,maxVersions}`
  — host-side Wayland filter controls for graphics VMs that opt into
  cross-domain forwarding. The filter is enabled by default when
  `graphics.crossDomainTrusted = true`, denies unknown/high-risk globals
  by default, and exposes explicit allow/deny/version-cap overrides.
- `d2b.vms.<vm>.graphics.waylandFilter.{byteLogging,dmabufAllow,dmabufDeny}`
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
  broker audit log, the privileged daemon socket, or d2bd state. The
  Loki stream stays a host singleton (`vm="host"`, `env="host"`,
  `role="host"`, `source="store-sync-audit"`); `target_vm`/`target_env`
  remain JSON content. `target_env` is resolved from the trusted manifest
  when present (and remains a JSON field, not a stream label). New gate
  `tests/store-sync-export-eval.sh`;
  `tests/loki-label-cardinality-eval.sh` now also parses
  `local.file_match` `path_targets` label maps. See
  [ADR 0027](docs/adr/0027-store-view-hardlink-live-pool.md) and
  `docs/reference/store-sync.md` § "Observability export".

- `d2b store verify <vm> [--repair] [--json]` — explicit
  broker-backed live-pool integrity verification for the ADR 0027 split
  store-view. The CLI is thin and never reads `store-view` directly;
  `d2bd` sends a typed `BrokerRequest::StoreVerify` to the privileged
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
- `d2b store verify` now performs deep recursive live-pool verification
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
  `d2b-activation-helper private-store <verb>` directly; the helper
  unshares its own mount namespace, makes propagation private, lazily detaches
  `/nix/store`, then runs the selected build/replace verb from stdin JSON.

- `d2b config` verb group — the host-side review/approve workflow
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
  `d2b switch` additionally rejects guest-set host-owned options as
  defense-in-depth. No new privileged surface (no virtiofs, no new
  socket); the untrusted pull is bounded (size cap + timeout). `d2b
  up` / `start` and `d2b status` also print a human-output note when
  a VM has a pending un-approved staged config.
- `d2b.vms.<vm>.guestConfigFile` — a dedicated, **guest-editable**
  per-VM NixOS module for the in-guest OS layer (packages, services,
  in-guest users, files). It is merged into the guest like `config`,
  but is **contained**: a best-effort eval-time namespace lint rejects
  it if it sets any host-owned `microvm.*` (runner substrate) or
  `d2b.*` (framework) option, naming the offending option(s)
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
  `/etc/d2b/guest-config.nix`, plus a writable working copy at
  `/var/lib/d2b-guest/guest-config.nix`) so it can be edited from
  inside the VM. See
  [`docs/how-to/edit-vm-config-from-inside.md`](docs/how-to/edit-vm-config-from-inside.md).

### Removed

- `d2b vm konsole` is removed. The subcommand was a thin wrapper that
  re-exec'd `d2b vm exec -it <vm> -- <login-shell> -l` inside a host
  terminal emulator; operators now invoke `d2b vm exec -it` directly.
  All references (CLI surface, shell completions, manpage, and reference
  docs) are dropped accordingly.

### Changed

- `d2b vm exec` now runs the requested command as the VM's
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
- `d2b config sync` on a guest-control VM now pulls the editable
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
  OpenTelemetry Collector services that export OTLP over d2b's
  broker-supervised Unix/vsock transport.
- Retired Grafana credential-file options are now documented as
  compatibility shims; native SigNoz credentials can be sourced from
  `d2b.observability.signoz.{jwtSecretFile,rootPasswordFile,clickhousePasswordFile}`.
- `retention.*` and `sampling.*` remain compatibility shims for the
  retired Tempo/Loki backend and warn when changed; native
  SigNoz/ClickHouse retention is operator-managed.
- Per-VM store isolation is moving to the Rust-owned `store-view/live`
  hardlink pool
  ([ADR 0027](docs/adr/0027-store-view-hardlink-live-pool.md)). The
  broker `StoreSync` path is the canonical writer for store-view
  metadata and live pool updates; host activation no longer
  builds/sweeps store-view closures. The guest readiness marker
  `store-view/live/.d2b-marker-<vm>` is a zero-length file, and each
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
  `d2b-wayland-filter` proxy instead of the former
  `wayland-proxy-virtwl` guest relay.
- `d2b.vms.<vm>.graphics.xwayland.enable = true` now fails eval
  during the Wayland-only migration. X11 application support will return
  through a separately validated helper path.

### Security

- Graphics VMs that opt into cross-domain forwarding now route guest
  Wayland traffic through a host-jailed `d2b-wayland-filter` process
  before reaching the real host compositor. The GPU sidecar connects to
  the per-VM filter socket; the dedicated `d2b-<vm>-wlproxy`
  principal is the VM-specific role with compositor socket access.
- Per-VM store isolation: the daemon-native virtiofsd `ro-store` runner
  served the host's entire `/nix/store` to every guest, so a guest's
  `/nix/store` exposed all host store paths instead of only the VM's own
  closure. virtiofsd now serves the per-VM closure-only hardlink farm
  (`/var/lib/d2b/vms/<vm>/store`), restoring the isolation the legacy
  `BindReadOnlyPaths /nix/store -> per-VM farm` provided; a guest's
  `/nix/store` now contains only its own closure.
- StoreSync observability export confinement: Grafana Alloy is granted
  focused POSIX ACLs (`u:alloy:--x` traverse on `<stateDir>` and
  `<stateDir>/observability`, `u:alloy:r-x` + a `default:u:alloy:r--`
  ACL on the export dir) to read the StoreSync export and nothing else
  under the broker state dir. Alloy is never added to the `d2bd`
  group and gets no read access to the unified broker audit log
  (`<stateDir>/audit/broker-*.jsonl`) or the privileged daemon socket.
  The export itself is a redacted projection, so a host-Alloy compromise
  exposes only the allow-listed StoreSync fields already destined for
  Loki, not the host-confidential audit stream.

### Fixed

- The host OTel bridge is now represented as a daemon/broker process role
  (`otel-host-bridge`) so readiness can track the broker-spawned runner.
- Observability relay ACL setup now excludes the host bridge principal
  from broad obs-VM state directory grants and uses the d2b-owned OTel
  runtime path for the bridge egress socket.
- TPM-enabled guests now flush stale loaded/saved TPM sessions during
  early boot before SRK provisioning. This prevents swtpm session-handle
  exhaustion from breaking TPM-bound credentials while preserving NVRAM
  and persistent handles.
- VM start (`d2b up` / `switch`) no longer aborts with
  `SpawnRunner failed ... broker-error` ("Invalid cross-device link")
  while building the per-VM store-view hardlink farm on hosts where
  `/nix/store` is bind-mounted read-only on top of itself (the stock
  NixOS layout). `link(2)` is rejected across that vfsmount boundary
  even when both paths share the same underlying filesystem, so the
  broker's in-process farm build failed with `EXDEV`. The broker now
  builds the farm inside a private mount namespace where `/nix/store`
  is lazily detached (mirroring the existing activation-time
  `d2b-store-sync` workaround), via the `d2b-activation-helper
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
  (`/var/lib/d2b/vms/<vm>`, `/run/d2b/vms/<vm>`) owned by a
  transient runner principal with a clipped POSIX ACL mask. The
  vm-start directory prepares now preserve the ownership + mode that
  host activation establishes (`d2bd:users 2770` plus per-runner
  ACLs) on an existing directory, so runners (virtiofsd, gpu, video)
  keep write access to their per-VM runtime dir and the ownership-matrix
  preflight no longer trips.
- `d2b switch` / `boot` / `test` no longer fail with `broker-error`
  ("no store-view intent in the trusted bundle"). The per-VM closure
  artifact now emits a populated `hostGeneration` (a deterministic,
  content-derived store-view generation), so the broker builds a
  store-view intent for every VM instead of skipping it. Previously
  live activation was impossible and the only way to apply a new
  generation was `d2b down <vm> --apply` followed by
  `d2b up <vm> --apply`. The per-VM `/nix/store` hardlink farm now
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
  the working `journalctl -u d2b-priv-broker` instead of the
  `d2b audit --strict` command (which returns `not-yet-implemented`).
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
into a single `d2b` group — a breaking change for consumer
configs that referenced the legacy group names (see
**Changed (breaking)** below).

### Added

- `d2b vm start --apply` readiness split into `process-alive` +
  `api-ready` DAG nodes. `--no-wait-api` opts into exit-0 once the
  process is alive; the strict-API default is preserved.
- `d2b vm status --json` surfaces the new `api_ready` field
  (`yes` / `pending` / `timeout` / error).
- `d2b host doctor` ships four new probes
  (`check_seccomp_bpf_loaded`, `check_pre_ns_posture`,
  `check_broker_reap_health`, `check_bridge_ipv6_sysctl`); see
  [`docs/reference/doctor.md`](docs/reference/doctor.md).
- `writableStoreOverlay` re-enabled. The broker provisions the per-VM
  overlay disk via the new `SpawnRunnerPlanOp::DiskInit` op
  (`mkfs.ext4` on first spawn). Size override via
  `d2b.vms.<vm>.writableStoreOverlaySize` (default 1 GiB).
- `tests/integration/live/live-vm-smoke.sh` (`--lite` / `--full`) is the maintainer
  pre-tag gate (`make pre-tag` / `make smoke-lite`); results land in
  `${TMPDIR:-/tmp}/d2b-smoke-run-log.txt`.
- New ADRs:
  [ADR 0022](docs/adr/0022-stabilization-mode-releases.md)
  (stabilization-mode releases) and
  [ADR 0023](docs/adr/0023-runner-role-lifecycle-matrix.md)
  (runner-role lifecycle matrix).
- New runbooks:
  [`docs/how-to/recovery-pre-ns-role-failure.md`](docs/how-to/recovery-pre-ns-role-failure.md),
  [`docs/how-to/route-conflicts.md`](docs/how-to/route-conflicts.md).
- Graphics VMs can opt into the daemon-spawned virtio-media H264 decode
  path with `d2b.vms.<vm>.graphics.videoSidecar = true`. The path uses
  the vendored patched Cloud Hypervisor `--vhost-user-media` support and a
  patched crosvm `device video-decoder --backend vaapi` runner; no per-VM
  systemd unit or stock-binary fallback is introduced.
- Graphics VMs can opt into experimental guest VA-API video forwarding with
  `d2b.vms.<vm>.graphics.virglVideo = true`. The switch is default-off
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
  `waitid(P_PIDFD)` and reports `ChildReaped` to `d2bd`.
- Bridge IPv6 sysctls (`disable_ipv6 = 1` on `br-*-up`) are now
  applied at boot via `boot.kernel.sysctl`.
- `d2b-priv-broker` may drop `CAP_NET_ADMIN` from its minijail
  bounding set when pre-created TAP fds are passed through.
- `umask` is plumbed end-to-end through `MinijailProfile` →
  `RoleProfile` → `SpawnRunnerPlan`; sidecar profiles default to
  `0o007`.

### Changed (breaking)

- Unified the legacy `d2b-launcher` and `d2b-launchers` Unix
  groups into a single `d2b` group. The activation script re-chgrps
  state files automatically on the next `nixos-rebuild switch` using a
  fd-safe numeric-gid migration helper. Consumer NixOS configs that
  reference the legacy group names in `users.<name>.extraGroups` must
  update to `"d2b"`. Required post-switch step:
  `sudo systemctl restart d2bd.service`. See
  [docs/how-to/migrate-d2b-v1-1-to-v1-2.md](docs/how-to/migrate-d2b-v1-1-to-v1-2.md).
  The broker caller-role audit label remains `"d2b-launcher"` for
  audit-format stability; see
  [docs/reference/naming-conventions.md](docs/reference/naming-conventions.md#broker-caller-role-audit-labels).
  `OperationFields::DeregisterRunnerPidfd { vm_id, role_id }` now
  appears in broker audit logs on successful `vm stop` cleanup for
  per-VM-UID runners; scripts that previously matched the old broker
  error exit see the corrected successful behavior instead.

  Note: the legacy `d2b-launcher` and `d2b-launchers` Unix
  groups remain on the system as empty v1.2 migration tombstones (zero
  membership, gid preserved in `/etc/group`). `getent group
  d2b-launcher` will still return a record with an empty member
  list. They are slated for removal in a v1.3 follow-up.

### Fixed

- Disk-init dispatch: `d2bd` now invokes `BrokerRequest::DiskInit`
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
- `d2b vm stop` no longer fails with `pidfd_table SIGTERM failed`
  when the runner runs as a per-VM dedicated UID: the daemon falls back
  to a broker-mediated signal on EPERM and deregisters the broker-side
  pidfd registry after successful termination.
- `d2b vm konsole` no longer reports `ssh key not found` when the
  parent directory is unreadable: the CLI distinguishes ENOENT from
  EACCES and emits an actionable error pointing at `d2b` group
  membership.
- `/var/lib/d2b/` now grants execute-only ACL traversal to the
  lifecycle group so the CLI can resolve keys and bundles without
  widening read access.
- Video sidecars now run as a dedicated `d2b-<vm>-video` principal, and
  activation/broker ACL refreshes deny that principal access to host
  Wayland, PipeWire, and Pulse sockets while preserving GPU cross-domain
  access for `d2b-<vm>-gpu`.

### Documentation

- ADRs 0003, 0011, 0021 received "Updated v1.2" subsections
  describing the broker-pre-NS extensions and reap responsibility.

### Deferred

- Drop the empty `d2b-launcher` and `d2b-launchers` Unix group
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
  `/run/d2b/vms/<vm>/tpm.sock`; both halves of the wiring update
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
- **`d2b vm konsole <vm>`** — opens an SSH session to a VM in a
  host terminal. Resolves the key from the bundle's
  `managed_keys.effective_key_path` and detaches via `setsid`.
- **Atomic cgroup placement** via `clone3(CLONE_INTO_CGROUP)`. New
  per-VM `<slice>/<vm>/<role>/` taxonomy (the per-VM interior node
  stays process-free).
- **USBIP guest attach/detach** routed through hardened SSH argv.
- **Pidfs runtime self-probe**: `d2bd` hard-refuses to start on
  kernels without pidfs unless
  `D2B_ALLOW_PIDFS_PROBE_SOFT_FAIL=1` is set.
- **`RenderDnsmasqEnvConf`** pure-Rust dnsmasq config renderer as a
  broker host-prep op.
- A real syn-based AST walker
  (`tests/tools/no-bash-ast-walker/`) backs
  `tests/no-bash-exec-eval.sh`.

### Fixed

- `fchownat(AT_EMPTY_PATH)` replaces broken `fchown` on `O_PATH`
  descriptors in the cgroup module.

## [1.1.0] - 2026-05-31

Daemon-only follow-through. D2b now owns its per-VM microVM
substrate end-to-end; the `microvm.nix` flake input is gone.

### Added

- **`nixos-modules/vm-options.nix`** declares the per-VM option set
  (hypervisor, vcpu, mem, kernel, shares, devices, volumes, …).
- **`nixos-modules/vm-evaluator.nix`** evaluates per-VM modules with
  the upstream NixOS evaluator (`eval-config.nix`). The
  `d2b.vms.<vm>.computed` option exposes the result.
- Rust runner-argv generators in `packages/d2b-host/`
  (cloud-hypervisor, virtiofsd, swtpm, gpu, audio, usbip,
  vsock-relay, otel-host-bridge) are now the canonical argv source.
- Typed CLI envelopes for `daemon-down` (exit 1) and
  `not-yet-implemented` (exit 78). The Rust CLI never invokes bash.

### Removed

- `microvm.nix` flake input dropped from `flake.nix`. Consumers who
  only inherited the input via `d2b.inputs.microvm.follows = …`
  need no flake change; consumers who declared `microvm.url`
  themselves can drop the input if they don't use microvm directly.
- `d2b.vms.<vm>.supervisor` option removed. Setting it now
  fails eval with a typed friendly message.
- `d2b-vfsd-watchdog@.{service,timer}` retired (wedge detection
  moved into the broker's virtiofsd `SpawnRunner` pidfd supervisor).
- `host-otel-relay-acl.nix` retired; OTel host-bridge ACL moved
  into the broker pre-spawn pipeline.

### Changed

- Kernel floor uplifted to **Linux ≥ 6.9** (`pidfs`-backed pidfd
  identity is required). See
  [ADR 0008](docs/adr/0008-supported-platforms-and-rejected-targets.md).
- `d2b.daemonExperimental.enable` is now obsolete and a no-op;
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
from the v0.x bash CLI + per-VM systemd templates: `d2bd` and
`d2b-priv-broker` are the only persistent root surfaces.

### Removed (breaking)

- **Bash CLI deleted.** `nixos-modules/cli.nix`, the
  `share/d2b/cli.sh` entrypoint, and every bash subcommand are
  gone. The Rust `d2b` binary is the sole CLI; there is no
  fallback bridge. `D2B_LEGACY_BASH_OPT_IN` and
  `D2B_NATIVE_ONLY` are no-ops.
- **Per-VM systemd templates retired.** `d2b@<vm>.service`,
  `d2b-<vm>-{gpu,swtpm,video,snd}.service`, and
  `d2b-known-hosts-refresh@<vm>.service` are deleted. Every
  per-VM lifecycle step runs inside `d2bd`'s DAG executor;
  spawned runners (cloud-hypervisor, virtiofsd, swtpm,
  vhost-user-sound, USBIP attach) are launched by the broker's
  `SpawnRunner` op and handed back as pidfds via `OpenPidfd` /
  `SCM_RIGHTS`.
- **Host singletons retired.**
  `d2b-audit-check.{service,timer}`,
  `d2b-ch-exporter.service`,
  `d2b-net-route-preflight.service`,
  `d2b-otel-host-bridge.service`, and per-env
  `d2b-sys-<env>-usbipd-*` units are deleted. Their work moved
  into `d2bd` (Prometheus exposition, net-route preflight,
  USBIP state machine) or into broker ops (`ExportBrokerAudit`,
  `UsbipBindFirewallRule`, `SpawnRunner{role: Usbip}`).
- **Polkit per-VM allowlists removed.** `d2b-launchers` group
  membership + `SO_PEERCRED` on `public.sock` is the only lifecycle
  authorisation surface.

### Changed (breaking)

- **Manifest `manifestVersion`: 2 → 3.** No compatibility window;
  the daemon and CLI reject v2 bundles with
  `manifest-version-mismatch`. Operators must rebuild the manifest.
- **Cgroup v2 slice** consolidated to a single `d2b.slice`
  delegated to the `d2bd` uid by the broker; see
  [ADR 0011](docs/adr/0011-cgroup-delegation-and-ownership.md).
- `d2b_host::DeviceClass` gained `Udmabuf` for GPU sidecar
  ioctls; `modules_disabled` is fail-closed in the broker's
  `ModprobeIfAllowed` path.

### Added

- **`d2b host validate` / `host reconcile`** — host-side
  preflight + degraded-mode recovery for the daemon's net-route
  monitor.
- **Broker audit** (`OpAuditRecord`) at
  `/var/lib/d2b/audit/broker-<utc-date>.jsonl`
  (`0640 root:d2bd`, append-only, daily rotation, 14-day
  retention by default; override with
  `d2b.site.audit.retentionDays`).
- **`docs/how-to/migrate-d2b-v0-to-v1.md`** is the
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
  `d2b-<vm>-video.service` running as the GPU sidecar user with
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

- **Observability subsystem** (`d2b.observability.enable`,
  default `false`). When enabled, the framework auto-declares the
  `obs` env (default `lanSubnet = 10.40.0.0/24`,
  `uplinkSubnet = 203.0.113.0/30`) and the `sys-obs-stack` VM that
  runs Grafana + Prometheus + Loki + Tempo + a central Alloy OTLP
  receiver. Retention defaults: metrics 30d, logs 14d, traces 7d
  (all per-knob configurable via
  `d2b.observability.retention.{metrics,logs,traces}`).
- **Per-VM guest agent** (opt-in via
  `d2b.vms.<vm>.observability.enable`). Each monitored guest
  runs Alloy scraping node metrics + journald (each
  individually toggleable via
  `vm.observability.{scrapeJournal,scrapeNodeMetrics}`), receives
  in-VM OTLP on a UDS, and exports over virtio-vsock through the
  hardened `d2b-otel-vsock-out.service` (socat sidecar:
  `RestrictAddressFamilies=[AF_UNIX AF_VSOCK]`,
  `DeviceAllow=/dev/vsock`, `restartIfChanged=false`).
- **Host-side forwarder** (`services.alloy` on the host, forwarder
  mode, no storage). Scrapes d2b sidecar units' journald + node
  metrics + the loopback CH-exporter `/metrics`. Pushes all signals
  through `d2b-otel-host-bridge.service` to the obs VM.
- **Cloud Hypervisor metrics exporter**
  (`d2b-ch-exporter.service`, pure-Bash + jq + curl + socat —
  no new language runtime in the host closure). Polls each VM's CH
  REST socket (`/vmm.ping`, `/vm.info`, `/vm.counters`), exposes
  Prometheus text on `127.0.0.1:9101/metrics`. Counter allowlist
  pinned to Cloud Hypervisor v50 device IDs (`_net*`, `_disk*`,
  `_fs*`, `_pmem*`, `__rng`, `__balloon`, `__console`); unknown
  schema rolls into `d2b_vm_unknown_counters_total`. Topology
  labels (`bridge`, `tap`, `tpm`, `graphics`, `audio`,
  `usbip_yubikey`) are off by default to keep the security-posture
  surface narrow — flip
  `d2b.observability.ch.exporter.includeTopologyLabels` on for
  debug. Detects both `microvm@<vm>.service` and
  `d2b-<vm>-gpu.service` so graphics VMs are reported running.
- **Vsock transport** — no IP between VMs, no SSH credentials
  between observer and observed. Cloud Hypervisor `--vsock cid=N,...`
  is appended to every observability-enabled VM and to
  `sys-obs-stack`; a per-VM `d2b-otel-relay@<vm>.service` (socat
  host relay, `RestrictAddressFamilies=[AF_UNIX]`) stitches
  workload-VM vsock to obs-VM vsock at the host. Relay is wired
  via `microvm@%i.service.wants` for headless VMs and via
  per-VM `wants` on `d2b-<vm>-gpu.service` for graphics VMs
  (graphics VMs do not use `microvm@`).
- **CLI lifecycle telemetry** — `d2b up/down/switch/boot/test/
  rollback/gc/usb/audio` emit OTel spans via `otel-cli` and
  structured JSON journald events for every high-value lifecycle
  step. Spans are populated with allowed labels only (`vm.name`,
  `vm.env`, `vm.role`, `d2b.subcommand`, `systemd.unit`, `tap`,
  `bridge`, `static_ip`, `generation`) — never command output, key
  paths, or Nix store paths. `d2b_span_start` generates `trace_id` +
  `span_id` locally via `/dev/urandom` so Loki↔Tempo correlation
  works even when no upstream OTLP collector endpoint is configured;
  honors otel-cli's traceparent when one is. `otel-cli` is
  module-time-gated into `runtimeInputs` via
  `d2b.observability.cli.traces.enable` (default `true`); hosts
  with observability disabled pay zero closure cost.
- **6 provisioned Grafana dashboards** under the "D2b" folder:
  D2b Overview, VM Resources, Lifecycle Traces, Logs, Per-VM
  Store, Obs VM Health. Default refresh 30s. Tempo→Loki
  trace-to-logs correlation via `derivedFields`.
- **8 Prometheus alert rules**: `D2bVMDown`,
  `D2bNetVMDownWithRunningWorkloads`,
  `D2bObsVMUnreachableFromHost`, `D2bVsockRelayDown`,
  `D2bCHAPISocketMissing`, `D2bStoreSyncFailure`,
  `D2bGuestTelemetryMissing`, `D2bObsVMStackUnhealthy`.
  Each rule individually toggleable via
  `d2b.observability.alerts.<name>.enable`. Notification
  channels are intentionally unconfigured — operators choose
  Alertmanager / Grafana contact-points.
- **Grafana auth**: defaults to authenticated access as
  `d2b-admin`. Password is generated at activation and stored
  at `/var/lib/d2b-observability/grafana-admin-password` inside
  `sys-obs-stack`, or sourced from sops/agenix via
  `d2b.observability.grafana.adminPasswordFile`. Session signing
  key follows the same pattern via
  `d2b.observability.grafana.secretKeyFile`. Anonymous Viewer
  is opt-in only for trusted single-host LANs via
  `d2b.observability.grafana.anonymousViewer.enable`; the login
  form remains available even in that mode.
- **Eval assertions**: vsock CID uniqueness across enabled VMs
  (reserved CID 1000 for `d2b.observability.vmName`),
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
  who do not enable `d2b.observability.enable` see the new
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
  (`d2b-otel-relay`, `d2b-otel-bridge`,
  `d2b-ch-exporter`) with execute-only ACLs on per-VM state
  directories and `rw` ACLs only on the per-port vsock sockets
  they need (`vsock.sock_14317`, not the base `vsock.sock`).
  Activation-time ACL refresh is idempotent and revokes stale
  grants when an observed VM is later disabled.
- `d2b-otel-acl-refresh` rejects symlinked state paths,
  validates resolved paths stay under the state root, and uses
  `setfacl --physical` when available — closes the TOCTOU
  window on a group-writable state tree.
- Grafana `secret_key` and admin password are never written to
  the world-readable Nix store. Both are generated atomically at
  activation (write-to-tmp + `mv -f`) and loaded via systemd
  `LoadCredential` into `/run/credentials/grafana.service/`, or
  sourced from operator-supplied files via
  `d2b.observability.grafana.{secretKeyFile,adminPasswordFile}`.
- Loki query selectors in shipped dashboards never default to a
  whole-namespace scan: every variable-driven selector requires
  a non-empty match (`.+`, not `.*`), and the trace-to-logs
  derivedField is scoped by trace-derived `vm`/`env` labels.
- Alert annotation templates carry `vm` and `env` only; full
  unit/job names stay inside dashboards (not exported to
  whichever notification backend an operator wires up).
- CLI span attribute extras are filtered through an allowlist
  in `d2b_filter_attrs`: caller-supplied keys outside
  `{step, result, systemd_unit, tap, bridge, static_ip, generation,
  vm_role}` are dropped with a journald warning, as are values
  matching common secret/store-path patterns.
- The guest UDS→vsock relay is fork-bounded
  (`max-children=16`, `TasksMax=32`, `MemoryMax=64M`,
  `LimitNOFILE=1024`) to bound in-guest DoS surface.
- The host telemetry bridge runs as `alloy` with
  `SupplementaryGroups=[kvm]` (no over-broad `d2b-otel-host-bridge`
  user) and connects to a narrowed
  `/run/d2b/alloy/` subdirectory rather than the shared
  `/run/d2b/` root.
- Documented trust-concentration risk: `sys-obs-stack` has read
  access to every monitored VM's telemetry; treat as privileged
  infrastructure. Single-host single-VM by design (multi-host
  is explicitly out of scope for v0.2.0).

### Deferred to v0.3.0

- **`D2bVMStuckWithoutSSH` alert** — needs a new
  CH-exporter metric (`d2b_vm_ssh_ready`) before the rule
  can be defined non-trivially.
- **`d2b_vm_store_path_count`** — the Per-VM Store
  dashboard references this metric today but it is currently
  **future-work absent**: no exporter emits it yet. The dashboard
  panel renders empty until a future store-path-count exporter
  lands (planned for v0.3.0). The `obs-metric-references`
  test gate treats it as a documented future-work exception
  rather than an unknown metric.
- **`d2b_vm_counter_net_tx_bytes` and
  `d2b_vm_counter_net_rx_bytes`** — referenced by the VM
  Resources network panel for legacy compatibility; the actual
  emitted metric names are `d2b_vm_counter_virtio_net_*`
  (CH v50 device naming). Documented as **future-work absent**
  pending dashboard query simplification — both legacy and
  modern names will resolve via Prometheus `or` until the legacy
  names are removed.
- **Stable relay-binary interface.**
  `d2b.observability.transport.relayPackage` still
  requires a `bin/socat`-compatible CLI today. Non-socat
  relays need a dedicated compatibility interface before the
  socat-compatible path can be removed.
- **VM-runner abstraction.** Today the framework leaks the
  runner-unit name (`microvm@<vm>` for headless,
  `d2b-<vm>-gpu` for graphics) into the relay wiring, and
  the observability code has to wire to both. A runner-agnostic
  abstraction is required before per-VM sidecar wiring can stay
  on a single name.


### Changed

- **sshd host keys are now generated on the HOST and shared into
  every guest read-only via virtiofs.** A new module
  `nixos-modules/host-ssh-host-keys.nix` provisions per-VM ed25519
  host keys at host activation under
  `${d2b.site.stateDir}/vms/<name>/sshd-host-keys/` (mode 0400
  root:root). `nixos-modules/store.nix` shares the directory into
  the guest at `/run/d2b-sshd-host-keys/` (virtiofs tag
  `d2b-ssh-host`). A new `nixos-modules/guest-sshd-host-keys.nix`,
  imported into every enabled VM by `host.nix`, points
  `services.openssh.hostKeys` at the shared path and disables the
  NixOS `ssh-keygen -A` activation hook. **Why**: pre-v0.2.0 each
  guest regenerated its sshd host keys on first boot and stored
  them on the tmpfs overlay over the read-only nix store, so they
  were ephemeral. Every VM restart regenerated them, the host's
  `known_hosts.d2b` pinned the first observed set and refused
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
  `${d2b.site.stateDir}/observability/` (default
  `/var/lib/d2b/observability/`, mode 0400 root:root) and
  shares them read-only into the stack VM via virtiofs at
  `/run/d2b-obs-secrets/`. The in-VM activation scripts that
  used to generate these secrets in
  `/var/lib/d2b-observability/` (inside `sys-obs-stack`) have
  been removed. **Why**: putting both secrets inside the VM
  pointed the trust flow the wrong way — anything on the host
  that needed the Grafana admin password (a launcher, a health
  probe, a backup) had to cross the VM boundary to read it, which
  in practice forced consumers to add an SSH-able operator
  account + sudoers rule inside `sys-obs-stack` just to claw the
  password back out. With this change, host-side
  `sudo cat ${d2b.site.stateDir}/observability/grafana-admin-password`
  is the supported path; no operator account inside the stack VM
  is required. The `d2b.observability.grafana.{secretKeyFile,
  adminPasswordFile}` overrides still work for sops-nix / agenix
  users.
- **Consumer extensions of the auto-declared observability VM are
  now allowed.** The pre-v0.2.0 assertion that rejected any
  user-side definition under `d2b.vms.<obsCfg.vmName>` was
  removed. The framework's auto-declaration block uses
  `lib.mkDefault` for every value, so a consumer override
  (e.g. `d2b.vms.sys-obs-stack.ssh.user = "root"`) merges
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
- **`services.alloy` /run/d2b/alloy via `RuntimeDirectory`,
  not tmpfiles**, on host + every guest + stack VM. The previous
  tmpfiles rule could not chown to the DynamicUser-allocated
  `alloy` UID at activation time; the directory either never
  appeared or was owned by `nobody:nogroup`, breaking
  `d2b-otel-host-bridge` setfacl + alloy's writability
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
  `/run/d2b-obs-secrets/grafana-secret-key` (was the in-VM
  `/var/lib/d2b-observability/grafana-secret-key`).

### Migration

- Fresh installs land on the new layout with no operator action.
- Pre-existing installs that booted v0.2.0 with the in-VM
  observability secret generator will see a **password rotation**
  at the next `nixos-rebuild switch`: the new host-generated
  secret displaces the old in-VM one. Operators should fetch the
  new password via
  `sudo cat /var/lib/d2b/observability/grafana-admin-password`
  on the host.
- Pre-existing installs that had ephemeral in-VM sshd host keys
  pinned in `/var/lib/d2b/known_hosts.d2b` will see a
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
  the keys directory (`/var/lib/d2b/keys/`) lacked a traverse
  ACL for `d2b-launcher`. The directory had a
  `group:d2b-launcher:--x` ACL entry, but both the tmpfiles
  rule and the activation script's `install -d -m 0700` set the
  directory mode to `0700`, which forces the POSIX ACL mask to
  `---` and neutralizes the named-group entry. Fix: add
  `setfacl -m "g:d2b-launcher:--x"` on the keys directory
  in the activation script, after the `install -d`, so the mask
  is recalculated to include `--x`.

- **`nixos-modules/host-known-hosts.nix`** + **`nixos-modules/cli.nix`**
  (`vmLaunchScript`): graphics-VM per-VM `.desktop` launchers
  silently did nothing when the pinned host key in
  `known_hosts.d2b` was stale. Two coupled bugs:
  1. `d2b-known-hosts-refresh@%i.service` was wanted only by
     `microvm@%i.service`, but graphics VMs bypass that template
     (the GPU sidecar runs cloud-hypervisor directly). The
     refresh therefore only fired during `nixos-rebuild`
     activation — often tens of minutes before the user actually
     launched the graphics VM — and every one of those
     activation-time refreshes timed out because the VM wasn't
     running yet. The pinned key stayed stale across rebuilds.
     Fix: also `Wants=d2b-known-hosts-refresh@<vm>.service`
     from `d2b-<vm>-gpu.service` for graphics-enabled VMs,
     with a matching `After=d2b-%i-gpu.service` on the
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
     `sudo systemctl start d2b-known-hosts-refresh@<vm>.service`).

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

- **`d2b list` status label**: `[pending switch]` →
  `[pending restart]`. The label tracks the *recommended action*,
  and the recommended action for unit-file drift after a host
  `nixos-rebuild switch` is `d2b restart <vm>` (clean down+up
  cycles the running closure over the staged unit files); `d2b
  switch <vm>` is the heavier per-VM-closure-rebuild path for
  VM-NixOS-module edits. CLI messages in `d2b status` and the
  `d2b list` trailer updated to match.

- **`systemd.targets.microvms.wants` is now `lib.mkForce []`** on
  every consumer. Previously v0.1.3 narrowed the list to
  autostart=true VMs; v0.1.6 narrows further to `[]` so all
  autostart wiring goes through `systemd.targets.multi-user.wants
  -> d2b@<vm>.service` exclusively. Removes the duplicate
  boot path (target.wants pulling `microvm@<vm>` directly,
  bypassing the framework wrapper).

### Added (assertions)

- **`graphics.enable + autostart` is now an eval-time error.** A
  graphics VM with `autostart = true` would boot through the
  upstream microvm@<vm> runner without the GPU sidecar's
  Wayland-socket bind, leaving the VM with no display. The
  assertion's remediation message points at `d2b up <vm>`
  from a Plasma terminal.

### Added (tests)

- `tests/unit/smoke/smoke-eval-extraspecialargs.nix` — regression for v0.1.1
  `extraSpecialArgs` propagation through `nixos-modules/host.nix:165`.
- `tests/net-vm-network-eval.sh` extended — regression for v0.1.2
  `ConfigureWithoutCarrier` + route entry on the host's uplink bridge.
- `tests/autostart-wiring-eval.sh` — covers `d2b@<vm>` as
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
  - `d2b restart <vm> [--force]` (v0.1.5)
  - `pending-restart` indicator semantics in `d2b list` /
    `d2b status` (v0.1.5)
  - `d2b.site.extraSpecialArgs` consumer-side escape hatch
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
  - Required minimum `d2b = github:vicondoa/d2b/v0.1.6`
    (or later) — earlier versions exposed framework bugs that
    blocked real-world graphics + TPM bring-up. (Aligned with
    the CHANGELOG; v0.1.6 is the first release where the docs
    match the shipping code.)
  - New "After every rebuild" step in the procedure: check
    `d2b list` for `[pending restart]`, apply with
    `d2b restart <vm>`. Cross-links to the cli-contract's
    pending-restart section.
  - New troubleshooting note: `d2b status <vm>` shows
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
    `d2bVmStatePerms` activation script handles it.
  - Updated the "DO NOT WIPE" warning to also point at the
    `pending-restart` indicator as the right signal for
    "TPM-bound creds may be re-read after restart".
  - New "Lifecycle (v0.1.5+)" subsection documenting
    `d2b-<vm>-swtpm.service`'s `unitConfig.X-RestartIfChanged
    = false`.

- **`docs/reference/components-audio.md`**:
  - New "Lifecycle (v0.1.5+)" subsection documenting
    `d2b-<vm>-snd.service`'s `unitConfig.X-RestartIfChanged
    = false`.

- **`AGENTS.md`**:
  - New "VM lifecycle policy" subsection documenting
    `restartIfChanged = false` as a framework invariant for
    contributors.
  - New convention: per-VM `wantedBy` ALWAYS via
    `systemd.targets.multi-user.wants` symlinks, never via
    per-instance `systemd.services."d2b@${name}"`
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
  run cloud-hypervisor via the `d2b-<vm>-gpu.service` sidecar
  (the GPU sidecar replaces the upstream runner), so the audit
  was blanket-skipping all graphics VMs even when they were
  running. Now: a VM is "running" if any of `d2b@<vm>`,
  `microvm@<vm>`, or `d2b-<vm>-gpu` is active.

- **`nixos-modules/cli.nix`** (`d2b list` / `d2b status`):
  pending-drift messages used to recommend `d2b switch <vm>`,
  which is the heavier per-VM-closure-rebuild path. The correct
  remediation for unit-file drift after a host `nixos-rebuild
  switch` is `d2b restart <vm>` (clean down+up cycles the
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

- **`d2b restart <vm> [--force]`** — convenience wrapper around
  `down <vm>` + `up <vm>`. Idempotent (a stopped VM is just brought
  up). Graphics VMs still require a Wayland session for the up
  step. The `--force` flag is forwarded to the down step (lets you
  cycle a net VM without first stopping the env's workloads). Used
  in tandem with the new `pending-restart` indicator below: when
  `d2b list` flags a VM, `d2b restart <vm>` applies the
  pending config.

- **`pending-restart` signal in `d2b list` / `d2b status`.**
  Compares each VM's `current` symlink (latest declared closure)
  against `booted` (the closure the running VM actually exec'd).
  If they differ AND the VM is up, both UIs flag the VM:

  ```
  NAME             ENV    GRAPHICS TPM   USBIP   STATIC_IP       STATUS
  work-aad         work   true     true  true    10.20.0.10      systemd [pending restart]
  ```

  And `d2b status work-aad` adds:

  ```
  pending-restart: YES — unit files changed; run `d2b restart work-aad` to apply
    booted : /nix/store/...-microvm-cloud-hypervisor-work-aad
    current: /nix/store/...-microvm-cloud-hypervisor-work-aad
  ```

  Note: v0.1.5 originally shipped the label as `[pending switch]`
  with a `run d2b switch <vm>` recommendation; v0.1.6 renamed
  the label to `[pending restart]` and the message to recommend
  `d2b restart <vm>` (the correct action for unit-file drift
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
  changes via `d2b restart <vm>` (or `d2b switch <vm>`
  for a per-VM closure rebuild + live activation).

  Services covered:
  - `d2b@<vm>.service` (user-facing wrapper)
  - `microvm@<vm>.service` (upstream runner; framework was
    overriding upstream's existing flag back to true via the
    host-known-hosts.nix drop-in)
  - `microvm-virtiofsd@<vm>.service` (per-VM virtiofs daemon;
    framework adds hardening stanza)
  - `d2b-<vm>-swtpm.service`
  - `d2b-<vm>-snd.service`
  - `d2b-<vm>-gpu.service`

- **`d2b-<vm>-gpu.service` updates the per-VM `booted`
  symlink.** Upstream microvm.nix's
  `microvm-set-booted@<vm>.service` only runs as part of
  `microvm@<vm>.service`'s lifecycle — but graphics VMs bypass
  that template (the GPU sidecar runs microvm-run directly).
  Pre-v0.1.5, `/var/lib/d2b/vms/<vm>/booted` simply didn't
  exist for graphics VMs, so the new pending-restart check
  couldn't compute anything. Added `ExecStartPre`
  (`+`-prefixed → root) that mirrors
  `microvm-set-booted_-start`:
  `rm -f booted && ln -s $(readlink current) booted`. Cleared
  by `ExecStopPost`.

- **`d2b-load-host-keys.service` primary-group resolution.**
  Pre-v0.1.5 the script assumed the guest user's primary group
  matched the username (`install -d ... -g "$SSH_USER"`). This
  only holds when the consumer's VM config sets
  `users.users.<u>.group = "<u>"` or uses DynamicUser. NixOS's
  `isNormalUser = true` default puts the user in the `users`
  group, breaking the install with
  `install: invalid group '<u>'`. Result: no d2b-managed
  pubkey ever reached the guest's `authorized_keys`, and SSH
  only worked for keys baked statically into
  `users.users.<u>.openssh.authorizedKeys.keys`.

  Now: resolve GID via `getent passwd | cut -d: -f4`, then GID →
  name via `getent group`. Works for both
  `users.users.<u>.group = "<u>"` and the NixOS default.

## [0.1.4] - 2026-05-19

Patch release. Four framework bugs surfaced during the first real
consumer migration's VM bring-up (paydro's /etc/nixos, after v0.1.3
got `d2b@<vm>` units working but the actual graphics+TPM VM
refused to boot).

### Fixed

- **`nixos-modules/host-sidecars.nix`**: per-VM GPU sidecar
  (`d2b-<vm>-gpu.service`) had `DevicePolicy = "closed"` without
  `/dev/net/tun` in `DeviceAllow`. Cloud-hypervisor needs to
  `open("/dev/net/tun")` + `ioctl(TUNSETIFF, …)` to attach to the
  VM's tap (created earlier by upstream microvm.nix's
  `microvm-tap-interfaces@<vm>.service` helper); without it
  graphics VMs crash in early boot with "Cannot create virtio-net
  device / Couldn't open /dev/net/tun / Operation not permitted".
  Added `/dev/net/tun rw` to DeviceAllow.

- **`nixos-modules/host-activation.nix`**: `d2bVmStatePerms`
  granted ACL rwx on `/var/lib/d2b/vms/<vm>/` to
  `d2b-<vm>-gpu` but not to `d2b-<vm>-swtpm`. The swtpm
  service starts as the swtpm user, opens its `StateDirectory=`
  (which systemd creates at the correct path), then tries to read
  `tpm2-00.permall` — and EACCESes because traversing the parent
  dir requires +x for the swtpm user. libtpms enters failure mode
  and the VM boots with a freshly-initialised TPM, triggering
  Entra/Intune device-tampering alerts for tenant-enrolled VMs.
  Added `setfacl -m "u:d2b-<vm>-swtpm:--x" <stateDir>` (gated
  on `vm.tpm.enable`).

- **`nixos-modules/base.nix`**: `d2b-load-host-keys.service`
  inside the guest referenced `${"$"}{pkgs.coreutils}/bin/getent` —
  but `getent` is in glibc, not coreutils. The lookup silently
  failed with "No such file or directory" and the script printed
  `user '<u>' not found in /etc/passwd — skipping` even though the
  user existed. Result: d2b-managed pubkeys + the consumer's
  `userAuthorizedKeys` never reached the guest's
  `authorized_keys` — SSH worked only via any pubkey statically
  baked into the VM's `users.users.<u>.openssh.authorizedKeys.keys`.
  Fixed path to `${"$"}{pkgs.glibc.getent}/bin/getent`.

- **`nixos-modules/cli.nix`** (audit `--strict`): the
  `bridge_isolated_workload.<vm>` check ran unconditionally and
  STRICT-FAILed when the VM wasn't running (the workload tap
  doesn't exist on the bridge, so jq returned null). With the
  framework's default `d2b.vms.<vm>.autostart = false`, this
  blocked every post-activation `d2b-audit-check.service`
  hook → `nixos-rebuild switch` returned non-zero exit code 4.
  Added a `systemctl is-active microvm@<vm>` precondition that
  emits `AUDIT SKIP [bridge_isolated_workload.<vm>]: VM not
  running` (mirrors the existing virtiofsd skip-when-down
  semantic).

## [0.1.3] - 2026-05-19

Patch release. Two more framework bugs surfaced during the first
real consumer migration, both around the d2b@<vm> wrapper +
microvm.nix interaction.

### Fixed

- **`nixos-modules/host-wrapper.nix`**: per-VM `d2b@<vm>.service`
  units for `autostart=true` VMs were emitted as separate unit files
  (via `systemd.services."d2b@${name}"`) that NixOS materialised
  WITHOUT the template's `ExecStart`/`ExecStop`/`PropagatesStopTo`/
  `Type=oneshot` settings — so systemd refused them at boot with
  "Service has no ExecStart=, ExecStop=, or SuccessAction=. Refusing."

  Fix: drop the per-instance `systemd.services` declarations and
  use `systemd.targets.multi-user.wants` symlinks instead. systemd
  then resolves each `d2b@<vm>.service` against the template
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
  on-demand via `d2b up <vm>`.

## [0.1.2] - 2026-05-19

Patch release. Surfaced during the first real consumer migration to
v0.1.x — a runtime bootstrap deadlock between
`d2b-net-route-preflight.service` and the per-env uplink bridge.

### Fixed

- **`nixos-modules/network.nix`**: per-env uplink bridge
  (`br-<env>-up`) now has `networkConfig.ConfigureWithoutCarrier =
  true`. Without it, networkd refuses to apply the Address + static
  Route to the env's LAN subnet until the bridge has carrier. But
  carrier only appears when the per-env net VM attaches its uplink
  tap to the bridge, and the net VM start is gated on
  `d2b-net-route-preflight.service`, which checks the static
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

- **`d2b.site.extraSpecialArgs`** (`attrsOf unspecified`,
  default `{}`). Merged into every per-VM
  `microvm.vms.<vm>.specialArgs` after the framework's own
  baseline. Consumer keys take precedence on collision, so a
  consumer that wants its full flake `inputs` (rather than just
  d2b's narrower input set) visible inside per-VM modules
  can set:
  ```nix
  d2b.site.extraSpecialArgs = { inherit inputs; };
  ```
  Mirrors `home-manager.extraSpecialArgs` from the Home-Manager
  NixOS module — same semantics, same intent.

### Fixed

- **`scripts/migrate-d2b-v0.1.0.sh`**: `[[ -d "$dir" ]] && info ...`
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
- `templates.default` (`nix flake init -t github:vicondoa/d2b`)
- `flake.checks.<sys>.eval-{minimal,multi-env,template,graphics}`
- `d2b@<vm>.service` lifecycle wrapper + the eight `d2b` CLI
  verbs (`up`, `down`, `status`, `list`, `switch`, `build`, `boot`,
  `test`, `rollback`, `generations`, `gc`, `audio`, `usb`, `console`,
  `keys`)
- `manifestVersion = 1` JSON contract (`/run/current-system/sw/share/d2b/vms.json`)
- VM-name regex `^[a-z][a-z0-9-]*$`, reserved prefixes `sys-` and
  exact name `launcher`
- Per-env isolated network (auto-declared `sys-<env>-net` net VM,
  point-to-point uplink, isolated LAN bridge, dnsmasq, nftables NAT)
- Per-VM `/nix/store` hardlink farm
- D2b-managed SSH keys
- Components: `graphics`, `tpm`, `usbip`, `audio`, `home-manager`

**Composition:** Sibling flake [`vicondoa/entrablau.nix`][entrablau] (also
v0.1.0) provides Entra ID device-join via the per-VM
`d2b.vms.<vm>.config.imports = [ inputs.entrablau.nixosModules.default ]`
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
- **`examples/multi-env/`** — two parallel `d2b.envs.<env>`
  instances (work + personal) demonstrating per-env LAN
  isolation, per-env net VMs, per-env USBIP backends, and the
  route-preflight fail-closed gate.
- **`examples/with-entra-id/`** — composition with the sibling
  [`vicondoa/entrablau.nix`][entrablau] flake; shows how
  the two trees meet at `d2b.vms.<vm>.config.imports`
  without either flake depending on the other.
- **`templates/default/`** — `nix flake init` scaffold with
  seven numbered placeholder markers and a matching
  `assertions = [ … ]` block. `nix flake check` on an un-edited
  scaffold fails with actionable messages until each sentinel is
  replaced.
- **`flake.templates.default`** — wires the template above so
  consumers can `nix flake init -t github:vicondoa/d2b`.
- **Manifest contract is now a documented, versioned interface.**
  - `nixos-modules/manifest.nix` — typed `config.d2b.manifest`
    `attrsOf submodule` option. Replaces the inline manifest
    construction previously folded into `cli.nix`. The Nix module
    system catches schema regressions at eval time.
  - `docs/reference/manifest-schema.md` + `docs/reference/manifest-schema.json`
    (JSON Schema Draft 2020-12) — the v1 public manifest contract
    for downstream consumers such as the Rust CLI. The
    JSON Schema is the canonical type spec; the prose doc is a
    field-by-field walkthrough + compatibility policy.
  - `docs/reference/cli-contract.md` — behavioural contract for any
    `d2b` CLI implementation (lifecycle FSM, signal semantics,
    exit codes, JSON vs human output, what is/is-not in scope).
  - `d2b.site.flakePath` is now derived as the CLI's default
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
- **`d2b.site.*` public option surface.** Site-specific knobs
  extracted from previously-hardcoded references to the
  maintainer's host setup. Every option is opt-in; defaults give a
  fully headless framework with no Wayland integration. Public
  options:
  - `d2b.site.stateDir` — root of every d2b-managed state
    file (default `/var/lib/d2b`). **Advisory only in v0.1.0**
    (see option description); full threading lands in v0.2.0.
  - `d2b.site.keysDir` — directory for framework-managed
    per-VM SSH keys (default `${stateDir}/keys`). Same advisory
    caveat for v0.1.0.
  - `d2b.site.waylandUser` — primary Wayland user; required
    for any VM with `graphics.enable = true` or `audio.enable =
    true`.
  - `d2b.site.launcherUsers` — users added to the
    `d2b-launcher` group (polkit grant for VM start/stop).
  - `d2b.site.userAuthorizedKeys` — global authorized SSH
    keys merged into every VM at boot. Validated at eval time
    against an allowlist of supported key types; private-key
    markers rejected.
  - `d2b.site.yubikey.enable` — host-side Yubico udev rules +
    `usbip-host` kernel module. Default true.
  - `d2b.site.flakePath` — default flake reference for the
    `d2b` CLI's lifecycle subcommands (`build`, `switch`,
    `boot`, `test`). Nullable.
- **`d2b.vms.<vm>.userAuthorizedKeys`** — per-VM
  authorized SSH keys, merged with `site.userAuthorizedKeys`.
- **`d2b.audio.users`** — host-side option propagating an
  audio-group membership list into the guest. Default falls back
  to `[ vm.ssh.user ]` when unset.
- **Framework-managed per-VM SSH keys.** Activation
  (`nixos-modules/host-keys.nix`) generates an Ed25519 keypair
  per enabled VM under `<keysDir>/<vm>_ed25519`. Atomic via
  staging + `mv -T`; protected by `flock` on `<keysDir>/.lock`.
  The pubkey is staged under
  `<stateDir>/vms/<vm>/host-keys/host.pub` and injected into the
  guest at boot via virtiofs.
- **`d2b keys` CLI subcommands.**
  - `d2b keys list [--json]` — fingerprint + path + mtime
    per VM.
  - `d2b keys show <vm>` — print the pubkey.
  - `d2b keys rotate <vm>` — atomic rotate-and-verify with
    SHA256-fingerprint-based old-key scrub + 3-generation
    retention (see Changed entry above).
- **`d2b-load-host-keys.service`** (guest-side) — fail-closed
  service that reads `/run/d2b-host-keys/` and writes the
  union of `host.pub` + user-authorized-keys into the SSH user's
  `~/.ssh/authorized_keys`.
- **`scripts/migrate-d2b-v0.1.0.sh`** — one-shot host migration
  script for consumers upgrading from a pre-public in-tree d2b
  layout. Preserves TPM state byte-for-byte. Has `--dry-run` and
  `--rollback`. Committed under `scripts/` so CI can shellcheck it.
- **`tests/unit/smoke/smoke-eval.nix`** — minimal consumer-style nixosSystem
  that imports `d2b.nixosModules.default` and exercises the
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
- Mechanical lift of d2b modules from `/etc/nixos/modules/d2b/`
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
- `systemd.services."d2b@"` wrapper template with explicit
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
- **`d2b.vms.<vm>.graphics.enable` and
  `d2b.vms.<vm>.audio.enable` now refuse to evaluate on
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
  `/run/current-system/sw/share/d2b/vms.json` is world-readable,
  so exposing a per-VM private-key path leaks the location of
  secret material to every local user. The CLI now resolves the
  private-key path locally at Nix-eval time from
  `d2b.site.keysDir` (or per-VM `ssh.keyPath` override) and
  bakes a static per-VM mapping into the shell wrapper. Consumers
  reimplementing the CLI should mirror that: read
  `d2b.site.keysDir` from their own privileged config access,
  not from this world-readable file. The PUBLIC key path is not
  currently exposed; if a use case warrants it, a future
  `sshPubKeyPath` field is the recommended addition. `manifestVersion`
  stays at `1` — the schema was published moments before release and
  no external consumers exist yet, so this is a free pre-v0.1.0 break.
- `docs/reference/manifest-schema.json`: `manifestVersion.minimum`
  raised from `0` to `1`. The schema is the contract for v1+;
  pre-v1 manifest stubs are no longer valid under this schema.
- `docs/reference/cli-contract.md`: subcommand inventory reconciled
  with `d2b --help`. `audit` now correctly documents the
  `--strict` + `--human` flags (`--human` auto-enables on TTY);
  `rotate-known-host <vm>` (the companion to `trust`) added to the
  subcommand table and to the human/JSON output section.
- `docs/reference/cli-contract.md`: the What-is-not-in-this-contract
  section expanded. Spells out that microvm.nix internal lifecycle,
  swtpm internals, virtiofsd implementation, and polkit grant
  specifics are framework-internal; and draws the line between
  contract-bound unit names (`d2b@<vm>.service`,
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
- **`d2b.vms.<vm>.ssh.keyPath` is NOT removed.** Earlier commit
  messages claimed otherwise; that was a mis-description of the
  change. The option still exists. What changed is its effective
  default: when left unset (`null`), the CLI now derives the SSH-key
  path from `d2b.site.keysDir` as `<keysDir>/<vm>_ed25519`,
  matching the framework-managed Ed25519 key generated by
  `host-keys.nix` on every activation. Consumers who explicitly set
  a path still win; the option's `null` default lets the framework
  pick. This makes the framework-managed key the zero-config happy
  path while keeping the option-shape stable for consumers supplying
  their own keys (e.g. a hardware-backed Yubikey-resident key).
- Net VM `users.allowNoPasswordLogin` is set to `lib.mkDefault true`.
  Net VMs receive SSH keys via runtime injection
  (`d2b-load-host-keys.service` reads
  `<stateDir>/vms/<vm>/host-keys/` over virtiofs); they have no
  eval-time authorized_keys. Without the flag, NixOS module-eval
  fires the `users.allowNoPasswordLogin` assertion before runtime
  injection runs. Sealed-appliance consumers can override with
  `mkForce`.
- GPU sidecar (`d2b-<vm>-gpu.service`) hardening tightened:
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
- Route preflight oneshot (`d2b-net-route-preflight.service`) now
  FAILS CLOSED on conflict — exit 1 on any env-vs-route mismatch
  instead of WARN+exit 0. `RemainAfterExit=true`, `Before=` each
  enabled d2b-managed VM unit, `RequiredBy=` each wrapper, so a
  stale host route blocks VM start until the operator clears it.
- **BREAKING.** Option namespace renamed:
  - `d2b.networks.<env>` → `d2b.envs.<env>`;
  - `d2b.networks.<env>.routerName` →
    `d2b.envs.<env>.netName`;
  - `d2b.networks.<env>.extraRouterConfig` →
    `d2b.envs.<env>.extraNetConfig`.
- **BREAKING.** Per-env auto-declared VM renamed:
  `<env>-router` → `sys-<env>-net`.
- **BREAKING.** Systemd unit naming convention:
  - `swtpm@<vm>` → `d2b-<vm>-swtpm`;
  - `d2b-snd@<vm>` → `d2b-<vm>-snd`;
  - `d2b-gpu-<vm>` → `d2b-<vm>-gpu`;
  - `d2b-store-sync@<vm>` → `d2b-<vm>-store-sync`;
  - `usbipd-d2b` → `d2b-sys-usbipd`;
  - `usbipd-d2b-<env>` → `d2b-sys-<env>-usbipd-proxy`.
- **BREAKING.** System users/groups renamed: `d2b-gpu-<vm>` →
  `d2b-<vm>-gpu`, `d2b-snd-<vm>` → `d2b-<vm>-snd`,
  `swtpm-<vm>` → `d2b-<vm>-swtpm`.
- **BREAKING.** State-dir layout:
  - `<stateDir>/<vm>/` → `<stateDir>/vms/<vm>/`;
  - `<stateDir>/<env>-router/` → `<stateDir>/vms/sys-<env>-net/`;
  - `<stateDir>/swtpm/<vm>/` → `<stateDir>/vms/<vm>/swtpm/`;
  - `/run/d2b-snd/<vm>/snd.sock` →
    `/run/d2b/vms/<vm>/snd.sock`.
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
- CLI: `d2b up/down/status` now target `d2b@<vm>.service`
  (the user-facing wrapper) instead of `microvm@<vm>.service`
  directly. Lifecycle propagates via the wrapper's BindsTo /
  ExecStop. Diagnostic flows (`status --verbose`, `journalctl`
  examples) keep their `microvm@<vm>` references but label them
  `backend` / `implementation detail`.
- CLI: `d2b list` / `d2b status` output tag for system VMs
  changed from `(router)` to `(net-vm)`. Helper renames:
  `ensure_router_up` → `ensure_net_vm_up`, `router_active` →
  `net_vm_active`, `IS_ROUTER` → `IS_NET_VM`. User-facing prose
  `router` / `router VM` → `net` / `net VM` (kept `routing/routes`
  only where describing the network function).
- `d2b-launcher` polkit grant tightened to an exact-unit allowlist
  generated at NixOS eval time from `cfg.vms` + `cfg.envs`, restricted
  to `start` / `stop` / `restart` verbs only. Drops the bare
  `microvm@*` prefix wildcard; default-deny invariant restored.
  Recovery / debugging paths can still authenticate via sudo or
  polkit-prompt.
- Pre-v0.1.0 breaking changes do not get a deprecation period. There
  is no compat shim for the old `d2b.networks` namespace or for
  any of the renamed unit / user / state-dir identifiers.
- The first tagged release is `v1.0.0`; the v0.x line never tagged a
  public release. These v0.x entries were the in-flight roadmap during
  the development branch and are preserved as historical record of how
  the architecture got to v1.0.
- v1.0.0 ships in lockstep with
  [`vicondoa/entrablau.nix`][entrablau] v1.0.0; consumers
  using both should pin matching tags.

### Fixed

- `tests/{static,d2b-store,audio,lib}.sh` no longer assume
  `ROOT=/etc/nixos`; the value is derived from the script's own path
  so the suite runs from any clone.
- `tests/integration/live/d2b-store.sh:33` SC2157 (preexisting).
- Host-specific `D2B_FILES` entries (`vms/personal-dev.nix`,
  `vms/work-aad.nix`) dropped or guarded so the static gate stays
  useful for the public flake.
- `tests/integration/live/audio.sh` `D2B_WAYLAND_USER` resolution chain genericized
  (no longer hardcoded to the maintainer's host user).
- README polish: `microVM` is defined inline on first use; a
  maintainer-anecdote phrasing was replaced with neutral wording;
  an encrypted-backup callout was added for `/var/lib/d2b/`.
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
  `default = { }` on the readOnly `d2b.manifest` option. The
  nixpkgs module system treats `default` as an extra definition;
  combined with `readOnly = true` and the matching
  `config.d2b.manifest = …` assignment, it produced
  `set multiple times` only when a graphics VM was synthesized. See
  `tests/unit/smoke/smoke-eval-graphics.nix` for the regression test.
- Inter-env CIDR overlap check now performs real IPv4 prefix
  arithmetic (`lib.cidrOverlaps` in `nixos-modules/lib.nix`) instead
  of exact-string equality. Containment (e.g. `10.0.0.0/16` ⊃
  `10.0.1.0/24`) is rejected. Env-vs-`hostLanCidrs` is checked under
  the same helper.
- `d2b.site.yubikey.enable = false` actually gates the host-side
  udev rules + `usbip-host` kernel module. Previous code declared the
  option but never read it.
- `d2b keys rotate <vm>` now scrubs the OLD pubkey from the
  guest's `~/.ssh/authorized_keys` (matched by SHA256 fingerprint)
  AFTER the new key is verified — rotation used to leave the old key
  authorized forever. Retention bounded: 3 most recent generations
  under `<keysDir>/old/<ts>/`; older are pruned post-rotation. Help
  text updated.

### Removed

- **`d2b.vms.<vm>.entra-id.*` option removed.** Himmelblau /
  Microsoft Entra ID support has moved out of the d2b framework
  and into the sibling `vicondoa/entrablau.nix` flake. To migrate,
  add the flake as an input and import its module into the VM's guest
  config:

  ```nix
  inputs.entrablau.url = "github:vicondoa/entrablau.nix";

  d2b.vms.<vm>.config.imports = [
    inputs.entrablau.nixosModules.default
  ];

  # Move each `d2b.vms.<vm>.entra-id.<key>` into the guest
  # config; see the entrablau README for the new schema.
  ```

  The `d2b.vms.<vm>.entra-id` attribute is kept as a hidden
  stub option so leftover assignments produce a readable assertion
  error (with migration instructions) instead of a cryptic
  `option does not exist` message from the module system. Final
  removal of the stub is tracked for v0.2.0.

- Three host-side activation scripts removed from
  `nixos-modules/host-activation.nix`:
  - **`d2bSbctlBackup`** — moved maintainer-specific
    `*-backup.tar.gz` files from `$HOME` into `/var/lib/sbctl/backup/`.
    Not a framework concern. Consumers who relied on this should
    handle their own backup-file relocation outside d2b.
  - **`d2bStoreChownRepair`** — one-shot repair for a past chown
    bug (an earlier `modules/d2b/store.nix` revision leaked
    `group=kvm` into `/nix/store` inodes via the per-VM hardlink
    farm). New installs are unaffected. Consumers upgrading from a
    pre-public d2b that ran with the buggy revision should run the
    historical repair script from `/etc/nixos` once and then drop the
    activation script there; the bug cannot recur in public code.
  - **`d2bMigrateState`** — one-shot renamer
    (`/var/lib/microvms/<vm>` → `/var/lib/d2b/vms/<vm>`, plus
    `/var/lib/swtpm/<vm>` → `vms/<vm>/swtpm/`). New installs land
    directly on the current layout. Pre-public consumers should use
    the migration script (or perform the moves manually following the
    same logic) before switching to the public flake.

  These deletions remove all host-specific bias from the public
  framework's activation logic. The remaining two activation scripts
  (`d2bVmStatePerms`, `d2bNetVmVarImgPerms`, formerly
  `d2bRouterVarImgPerms`) only adjust file ownership on per-VM
  disk images and contain no host-specific assumptions.

### Known gaps

- **USBIP per-env units materialise even when no VM opts in.** Each
  `d2b.envs.<env>` declares `d2b-sys-<env>-usbipd-backend.service`
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
  `d2b.envs.<env>` can each reach the net VM (and via NAT,
  the upstream LAN) but cannot directly reach each other.
  Documented in `docs/explanation/design.md` and the
  `d2b.hostLanCidrs` option text. A future opt-in
  (e.g. `d2b.envs.<env>.intraLanIsolation = false`) is on the
  v0.2.0 wishlist.

[entrablau]: https://github.com/vicondoa/entrablau.nix
