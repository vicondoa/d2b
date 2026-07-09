# Changelog

All notable changes to d2b are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Pre-1.0 minor releases may break public APIs. When practical,
deprecations ship one minor release before removal.

## [Unreleased]

### Fixed

- Fixed `realm-controllers.json` workload identity emitter: workload identity
  fields are now nested under `identity: { ... }` matching the
  `RealmControllerLocalWorkload.identity: Option<WorkloadIdentity>` Rust DTO
  field instead of being flat-merged into the workload root (which violated
  `deny_unknown_fields` on `RealmControllerLocalWorkload`). Field names now
  match `WorkloadIdentity`: required `workloadId`, `realmId`, `realmPath`
  (emitted as a label array via `lib.splitString`), `canonicalTarget`; optional
  `legacyVmName`, `runtimeKind`, `providerId` (renamed from the incorrect
  `runtimeProviderId` key). The `kind` field is removed from the identity block
  (it has no corresponding field in `WorkloadIdentity`). Transitional env-based
  workload entries correctly omit the identity object entirely.
- Fixed `launcher.app.targetRealm` nix-unit test to use a valid
  `WorkloadTarget`-format address ending in `.d2b` (`corp-laptop.alt.d2b`).
  Added `realmWorkloadTargetAssertions` in `assertions.nix` to reject invalid
  `targetRealm` values at eval time (must match
  `<workload>.<realmPath>.d2b` with `[a-z][a-z0-9-]*` labels).
- Fixed `realm_workload_schema_contract` contract tests to assert the correct
  nested `WorkloadIdentity` shape and field names. The old tests checked for
  the wrong field name `runtimeProviderId` (renamed to `providerId` in the DTO)
  and did not verify nested identity structure or guard against the invalid
  `kind` key. Updated tests now verify: `identity =` nesting, presence of all
  required identity field names (`workloadId`, `realmId`, `realmPath`,
  `canonicalTarget`, `providerId`), and absence of `kind = workloadRow.kind`
  and `runtimeProviderId =` as JSON keys.

- Narrowed the group ACL on host-local realm run directories from `g::rwx` to
  `g::r-x` so realm-access-group members can traverse and list but cannot
  write. The previous `g::rwx` ACL combined with the sticky `1770` base mode
  allowed a local group member to pre-create `broker.sock` or `daemon.lock`,
  preventing the daemon from binding its sockets or acquiring its lock (local
  DoS). The base mode `1770` and the explicit daemon-user `u:<user>:rwx` ACL
  entry are unchanged; the mask `m::rwx` is unchanged so the daemon retains
  full effective access.
- Granted host-local realm daemon users read ACLs for the shared
  `realm-controllers.json` and `realm-identity.json` metadata files so startup
  can load realm metadata after switching to per-realm principals.
- Gave host-local realm daemon users traverse access to `/run/d2b` and made
  realm run directories match the daemon lock-parent contract
  (`root:<realm-access-group>` sticky `1770`) so realm daemons can validate
  their own lock files.
- Ordered generated host-local realm daemon units after the root privileged
  broker socket/service so switch-time activation does not race broker
  socket-activation startup.
- Made the privileged broker's generated UID/GID environment file optional at
  `ExecStartPre` time so socket activation works on a fresh `/run` without a
  pre-existing `/run/d2b/broker/priv-broker.env`.
- Aligned realm gateway target routing and generated entrypoints with ADR 0043's
  canonical `<workload>.<realm>.d2b` form so the Rust CI gate no longer hangs
  on stale node-qualified gateway tests.
- Updated the `crossbeam-epoch` lockfile entry to a non-vulnerable release for
  the RustSec advisory check.

### Changed

- Amended ADR 0043 (realm-native control plane) to specify: hierarchical
  cgroup layout (`d2b.slice/<realm>/<workload>/<role>`), `mkRemovedOptionModule`
  tombstones for retired `d2b.envs` and `d2b.vms` pointing to the v1.2-to-v2
  migration guide, `internal = true; visible = false` for generated substrate
  options, 1:1 state-path mapping from workload id to legacy
  `/var/lib/d2b/vms/<vm>` with no implicit activation-time state moves,
  desktop JSON realm-to-workload association requirement, CLI transition
  behavior for old `d2b vm`/`d2b env` commands, MAC preservation and
  interface-rename/firewall-drift warnings for net VM renamed from
  `sys-<env>-net` to `sys-<realm>-net`, eval-time assertion requirement for
  cross-realm `externalNetwork` uplink conflicts, and explicit
  workload/provider telemetry label bounding and workload-identity audit
  redaction rules.
- Amended ADR 0043 with further design requirements from the R2 panel review:
  per-realm run directory `r-x` group-class ACL invariant (code hotfix PR #263
  tracked separately); `/etc/d2b/realm-identity.json` public-identity-only
  constraint; additive vs breaking schema versioning rule for
  `realm-controllers.json`/display-list shape changes; mandatory strongly typed
  `WorkloadTarget` parser in `d2b-core` with no ad hoc string splitting;
  `SpawnRunner` typed/polymorphic envelope separating universal workload identity
  from provider-specific backend config; and a Visual presentation requirements
  section codifying Waybar left-border realm accents, wlcontrol realm group card
  borders, realm-colored Wayland rail, and wlterm/clip-picker realm grouping.

### Added

- `d2bd` now populates `workloadIdentity` in `ListEntry` and `VmStatus` public
  wire responses from realm workload metadata when a `realm-controllers.json`
  config is present. Fields populated: `workloadId`, `realmId`, `realmPath`,
  `canonicalTarget`, and `legacyVmName` where available.
- New `WorkloadTargetIndex` internal module in `d2bd` provides index-backed
  resolution of VM targets from canonical targets (`<workload>.<realm>.d2b`),
  workload-id aliases, and legacy VM name fast-path. Alias resolution is
  unambiguous-only: ambiguous workload-id matches fail closed.
- Two new `TypedError` variants: `WorkloadTargetNotFound` (exit code 2) and
  `WorkloadAliasConflict` (exit code 2), returned when a `vm` filter specifies a
  canonical target not in the index or an ambiguous workload-id alias.
- The `vm` filter in list and status requests now resolves canonical targets and
  unambiguous workload-id aliases through the workload index before matching
  against manifest entries.
- Added `canonical_target` field to `ListItemOutputV2` and `StatusVmOutputV2`
  CLI output structs. When the daemon advertises workload identity for a VM,
  the field is populated with the canonical workload target address
  (e.g. `corp-vm.work.d2b`); it is absent (not serialized) when no workload
  identity is available, preserving backward compatibility with old daemons.
- `d2b vm list` human output now shows a `WORKLOAD TARGET` column when at least
  one listed VM has a canonical workload target.
- `d2b vm status` human output now shows a `workload target:` line when the
  queried VM has a canonical workload target.
- `d2b vm <verb>` commands emit a non-fatal compatibility note to stderr when a
  bare VM name is used and the daemon has advertised a canonical workload target
  for it, suggesting the canonical form (e.g.
  `note: target 'corp-vm' is a bare VM name; consider using 'corp-vm.work.d2b'`).
  The bare-name local fast path continues to work unchanged.
- Env-qualified VM names missing the required `.d2b` suffix
  (e.g. `corp-vm.work` instead of `corp-vm.work.d2b`) are now rejected
  fail-closed with error code `old-env-style-target` and a clear remediation
  message suggesting the canonical form. This closes the migration UX gap
  for operators moving from legacy env-scoped targeting.

- Added `d2b.realms.<realm>.workloads.<workload>` option for declaring
  realm-owned workloads. Each workload optionally references an existing
  `d2b.vms.<vm>` substrate via `vmRef` for runtime kind and provider id
  derivation, and carries stable launcher metadata (`label`, `icon`,
  `actionId`, `capabilityRefs`, `preflightRefs`) for desktop consumers.
- Extended the internal `_index.realms` with a `workloads` sub-index
  (flat `all`/`enabled` lists and a `byVm` map) and `externalNetworkConflicts`
  advisory data for cross-realm attachment interface collisions.
- Added `targetAddress` (`<workload>.<realmPath>.d2b`), `substrateId`,
  `runtimeKind`, and `runtimeProviderId` fields to each realm workload index
  row; each realm row now exposes `workloads`, `workloadNames`, and
  `enabledWorkloadNames`.
- Added `realm-workloads-launcher.json` bundle artifact (installed at
  `root:d2bd 0640`) containing stable desktop launcher metadata for all
  enabled realm workloads. Includes `targetAddress`, `actionId`, `label`,
  `icon`, `capabilityRefs`, `preflightRefs`, `runtimeKind`, `runtimeProviderId`,
  advisory `vsockCid`, and explicit invariant markers confirming no secrets,
  credentials, provider tokens, command payloads, or opaque session handles.
- Updated `realm-controllers.json` generation to prefer explicit
  `realm.workloads` declarations (with `vmRef`) as the primary source for
  local runtime workload entries, falling back to env-based matching for VMs
  not covered by explicit declarations. Backward compat is fully preserved for
  realms without workload declarations.
- Added cross-realm vsock CID collision assertion in `assertions.nix`: fires
  when two workloads in different realms reference different NixOS VMs that
  compute to the same vsock CID.
- Added cross-realm external network attachment conflict detection: advisory
  assertion records when realms share an attachment interface across their
  associated envs; demoted to non-failing in metadata-only runtime state with
  a clear upgrade note for when realm-native networking activates.
- Added nix-unit coverage for realm-owned workload index and launcher metadata
  contracts: workload index row fields (`targetAddress`, `substrateId`,
  `runtimeKind`, `runtimeProviderId`, `capabilityRefs`), all/enabled/byVm index
  accessors, `realm-workloads-launcher.json` shape and invariants, bundle
  artifact registration, cross-realm vsock CID collision assertion,
  cross-realm external-network conflict index, and empty-realm edge cases.
- Added Rust contract tests (`realm_workload_schema_contract`) for realm
  workload DTOs and artifacts: schema presence and definition of
  `WorkloadIdentity` / `RealmTarget` in `realm-controllers.json` and
  `wire-protocol.json`; additive-field invariant verifying `identity` and
  `workloadIdentity` are not in `required[]`; wire/CLI schema separation
  (workload identity travels in the daemon-wire schema only, not in CLI output
  schemas); `realm-workloads-launcher.json` emitter contract markers
  (`noSensitiveCommandPayloads`, `canonicalTarget`, `appCommand`, `actions`,
  classification); controller config emitter wires identity fields and does not
  reference removed field `vmRef`; `deny_unknown_fields` source-lint for
  `WorkloadIdentity` and sibling structs; module-level version policy doc gate
  (`bundleVersion` + `schemaVersion` bump requirement); absence of sensitive
  credential fields in `realm-controllers.json`.
- Extended realm workload index rows with `canonicalTarget` (derived from
  `launcher.app.targetRealm` override or the standard `<workload>.<realm>.d2b`
  formula), `appCommand` (from `launcher.app.command`), and `actions` (from
  `launcher.actions`) so downstream emitters have full desktop launch metadata
  without accessing raw workload options.
- Extended `realm-workloads-launcher.json` to expose `canonicalTarget`,
  `appCommand`, and `actions` (each action carries `id`, `label`, and
  `command`). Commands are static operator-declared launch metadata, not
  sensitive payloads; the invariant is refined to `noSensitiveCommandPayloads`
  with accompanying security contract notes.
- Extended `realm-controllers.json` workload entries for explicit realm
  workload declarations: each entry now carries `kind`, `realmPath`,
  `canonicalTarget`, `legacyVmName`, `runtimeKind`, and `runtimeProviderId`
  alongside existing runtime/path fields. Transitional env-based entries
  remain unchanged. Fixed a bug where the emitter still referenced the removed
  `vmRef` field instead of the correct `legacyVmName`.

- Added stacked-PR workflow documentation to AGENTS.md covering branch naming,
  PR-only merges, panel/review evidence requirements, integrator ownership of
  CI, retarget/rebase, merge sequencing, and helper-script constraints.
- Added `WorkloadIdentity`, `WorkloadTarget`, `WorkloadBackend`, and
  `WorkloadRuntimeIntent` types to `d2b-core::workload_identity`. `WorkloadTarget`
  is a type alias for `RealmTarget` that makes the bundle-artifact parse boundary
  explicit (no ad hoc string splitting past `WorkloadTarget::parse`).
  `WorkloadIdentity` carries the universal realm-scoped identity (workload id,
  workload name, realm id, realm path, canonical target, optional legacy VM name,
  runtime kind, provider id) independently of any backend runtime config.
  `WorkloadBackend` provides the typed envelope separating the universal identity
  from provider-specific runtime details (`LocalVm` / `LocalQemuMedia`).
  `WorkloadRuntimeIntent` combines both for process-intent DTOs. Module-level
  doc comment codifies the additive-vs-breaking DTO version policy.
- Extended `RealmControllerLocalWorkload` in `d2b-core` with an additive
  `identity: Option<WorkloadIdentity>` field. `None` for bundle artifacts emitted
  by Nix before this change; present once the emitter is updated.
- Added optional `workload_identity: Option<WorkloadIdentity>` to the
  `ListEntry` and `VmStatus` public output structs in `d2b-contracts`. `None`
  for classical `d2b.vms` VMs not yet associated with a realm; present for
  realm-adopted workloads.

- **Realm workload and network option schema** (`d2b.realms.<realm>.workloads`
  and `d2b.realms.<realm>.network`): the v2 public surface for workloads and
  network declarations.

  - `d2b.realms.<realm>.workloads.<workload>` supports `kind = "local-vm"`,
    `"qemu-media"`, and `"provider-placeholder"`.  Each workload carries
    `localVm.*` (config, memoryMiB, vcpus, networkIndex, ssh, graphics, tpm,
    autostart), `qemuMedia.*` (source, removableSlots, resources, security),
    and desktop-launcher metadata (`launcher.*`: label, icon id/name,
    app.command, app.targetRealm, actions, capabilities).

  - `d2b.realms.<realm>.network` is extended from the stub `network.*` block
    with the full env-replacement shape: `lanSubnet`, `uplinkSubnet`, `mtu`,
    `mssClamp`, `lan.allowEastWest`, `externalNetwork.*` (attachment, egress,
    portForwards, mDNS), and `ui.accentColor`.  `network.mode` drives
    behaviour: `none` (default/safe), `inherit-env` (delegate to existing
    `d2b.envs`), `declared` (realm owns network), `external`.

  - State path policy: `workload.stateDir` defaults to
    `/var/lib/d2b/vms/<workload-id>` preserving 1:1 mapping with legacy
    `d2b.vms.<vm>`.  Use `legacyVmName` to explicitly reference an existing VM
    entry when the workload id differs.

  - Tombstone/migration UX: descriptions on `d2b.envs` and `d2b.vms` updated
    to note they are transitional surfaces pointing at the replacement
    declarations and the v1.2-to-v2 migration guide.  Soft advisory warnings
    (`config.warnings`) fire when a realm links to existing `d2b.envs` entries
    but has no workloads and `network.mode = "none"`, nudging toward completion
    of the transition without blocking activation.

  - Updated `docs/how-to/migrate-d2b-v1-2-to-v2.md` with step-by-step
    instructions for declaring realm workloads and optionally switching to the
    realm-declared network.

- Added ADR 0043: Realm-native control plane, documenting the realm-as-control
  plane architecture, per-realm daemon/broker/socket/state/audit boundaries,
  strict parent/child routing, dynamic relay discovery, realm-qualified VM
  addresses, and migration from legacy local grouping into first-class realms.
- Added tree route admission and decision support for signed-expiring route
  advertisements, strict descendant namespaces, replay/expiry checks,
  loop/multiparent refusal, nearest-common-ancestor path decisions, bounded
  discovery queue decisions, direct shortcut metadata, and low-cardinality route
  audit/telemetry events without enabling live transport.

- Added topology validation coverage for bounded discovery queues,
  unauthenticated-peer drop-new/rate-limit behavior, route-advertisement
  expiry/replay rejection, capability denials, direct-shortcut policy denials,
  no-raw-tunnel route decisions, generated schema/docs exposure, and
  ancestor-mediated tree route decisions.
- Documented the metadata-only realm discovery and strict tree routing contract,
  including parent/child route advertisements, namespace validation,
  queue/rate/replay bounds, direct shortcut constraints, correlation/audit
  chaining, and the explicit no VPN/overlay/SSH/raw-tunnel runtime boundary.
- Added tree routing/discovery data models for bounded
  discovery queues, unverified-peer/session admission, replay windows, signed
  route advertisements, namespace allocations, route decisions, direct shortcut
  metadata, audit labels, and low-cardinality telemetry counters.
- Added an in-memory realm identity metadata store for
  enrollment pins, controller-generation rotation, revocation-list merge,
  recovery state transitions, teardown directives, and redacted lifecycle audit
  metadata.
- Added metadata-only realm identity lifecycle models for
  identity refs/fingerprints, controller-generation credentials, enrollment
  trust anchors/key pins, key rotation, revocation-list propagation, session
  teardown directives, recovery procedures, and redacted audit metadata.
- Documented the realm identity lifecycle contract for refs/fingerprints versus
  key material, parent trust anchors, child key pins, controller-generation
  rotation, revocation lists, teardown directives, recovery metadata, and the
  live-enforcement boundary.
- Documented the realm access resolver contract for canonical realm target
  grammar, alias/default-realm resolution, direct host-local access bindings,
  capability preflight, typed resolver diagnostics, and contract-only routing
  boundaries.
- Documented the local-root allocator contract for typed host-resource
  leases, opaque resource ids, deterministic acquisition order, reconciliation,
  quarantine/reclaim, immutable host-file boundaries, and contract-only status.
- Added local-root allocator data models for realm host-resource
  leases, opaque resource ids, allocation/reconciliation responses, bounded
  allocator audit/metric metadata, and generated JSON schema coverage.
- Added a pure local-root allocator engine over fake ledger,
  observation, and liveness backends for deterministic lease allocation,
  idempotency replay, reconciliation decisions, and bounded low-cardinality
  audit/metric metadata.
- Added private `allocator.json` bundle metadata rooted in `d2b.realms`, covering
  enabled realms, metadata-only local-root allocator resource requests,
  path/socket partitions, provider placement, and the transitional env bridge
  without starting an allocator runtime service.
- Added private `realm-controllers.json` bundle metadata rooted in `d2b.realms`,
  covering deterministic per-realm daemon, broker, socket, state, audit,
  allocator binding, provider placement, and direct-access metadata.
- Added metadata-only local runtime provider/workload rows to
  `realm-controllers.json`, binding host-local realms to existing VM runtime
  providers, preserved VM state/run/store-view paths, and explicit runtime
  operation capability summaries without moving state during activation.
- Added a host-provider qemu-media runtime adapter that validates the typed
  qemu-media argv scaffold, redacts argv/path inputs from debug output, and
  fails closed for lifecycle calls until daemon runtime control is wired.
- Extended the realm access resolver so host-local `localRuntime` metadata
  contributes bounded operation capabilities to capability preflight, while
  preserving typed denials for missing capabilities.
- Added canonical realm-target and picker/clipd capability-preflight metadata
  to the clipboard picker protocol so desktop pickers can display trusted
  d2b-provided VM identity without using guest titles or app ids as authority.
- Added d2b-asserted Wayland proxy `realmTarget` / `--realm-target` metadata so
  downstream desktop tools can identify proxied VM windows from host-provided
  realm metadata while preserving the existing app-id/title rewrite behavior.
- Added trusted identity source and display capability-preflight metadata to
  `d2b vm display list --json` so desktop helpers can consume bounded
  d2b-provided realm target state instead of guest window metadata.
- Added an explicit ACA guestd endpoint provider seam that advertises no
  guestd/persistent-shell capability for execute-only sandboxes and fails closed
  before any Azure data-plane call when endpoint status is requested.
- Tightened remote full-host registration so provider-managed-isolation
  capability sets cannot be retained as full-host nodes.
- Rewrote the v1.2-to-v2 migration guide for the realm-native metadata-first
  transition, including local VM preservation, host-local realm declarations,
  provider/remote fail-closed behavior, desktop metadata, cleanup, and rollback.
- Added host-local realm control-plane materialization from `d2b.realms`,
  including deterministic bounded unit names, daemon/broker users and groups,
  socket access groups for allowed users, runtime/state/audit directories, and
  parent-before-child ordering.
- Added Rust daemon, broker, and bundle-resolver loading for
  `realm-controllers.json` artifacts, including strict parsing and validation
  while keeping runtime realm routing inert.
- Added private `realm-identity.json` bundle metadata and strict daemon,
  broker, and bundle-resolver loading for realm identity refs/fingerprints only,
  without loading secret material or enabling live trust sessions.
- Added Layer-1 realm identity coverage for strict data-model/store behavior, rendered
  `realm-identity.json` bundle/storage contracts, loader redaction, schema drift,
  and eval-time rejection of secret-shaped identity refs.
- Added realm access resolver contract models for canonical target resolution,
  direct host-local bindings that preserve `SO_PEERCRED`, alias/default-realm
  diagnostics, conflict candidates, capability preflight, and stale/missing
  realm-controller refusals without changing runtime routing.
- Added d2bd local-root realm access resolver helpers that select direct Unix
  socket bindings from realm-controller metadata without introducing a byte
  proxy or changing the public socket protocol.
- Added Layer-1 realm access coverage for CLI target routing diagnostics,
  host-local resolver fail-closed paths, no-proxy direct socket semantics, and
  generated schema/docs exposure.
- Added Layer-1 coverage for host-local realm controller units, sockets,
  principals, tmpfiles paths, disabled-realm omissions, bundle classification,
  daemon/broker loading defaults, and bundle-resolver loading.


- Env net VMs can now opt into external network plumbing with a macvtap-backed
  `external0` NIC, separate workload-to-home egress NAT, and explicit
  external network port forwards.
- `d2b-wayland-proxy` now draws compositor-agnostic, per-VM colored borders
  with an optional VM-name label for proxied graphics windows. Borders use the
  existing `d2b.vms.<vm>.ui.border` color model, are enabled by default with
  the Wayland proxy, and can be disabled per VM.
- Added the `d2b-clipd` Rust crate with picker NDJSON DTOs, bounded framing,
  clipboard policy primitives, FD safety models, picker supervision,
  fail-closed audit / droppable metrics queues, host data-control integration,
  and the clipboard fallback arm control socket.
- Renamed the host-side Wayland package/binary to `d2b-wayland-proxy`.
- Renamed the per-VM graphics options from `graphics.waylandFilter.*` to
  `graphics.waylandProxy.*`; the old option path remains as a compatibility
  alias for this release.
- `d2b-wayland-proxy` now virtualizes the standard guest clipboard locally:
  it advertises a synthetic `wl_data_device_manager`, never binds guest
  `wl_data_*` objects into the host compositor clipboard namespace, routes
  same-VM transfers inside the proxy, and keeps primary-selection, privileged
  data-control, and DND denied.
- `d2b-wayland-proxy` virtual clipboard now correctly sends `wl_data_source.cancelled`
  to the previous source owner when a new selection supersedes it; `vm_name`
  attribution is threaded through all four virtual clipboard handlers
  (`VirtualDataDeviceManagerHandler`, `VirtualDataSourceHandler`,
  `VirtualDataDeviceHandler`, `VirtualOfferHandler`) and logged at each
  clipboard lifecycle event (source created/destroyed, MIME announced,
  selection set, offer received); source-gone EOF paths fail closed with EOF.
- `d2b-clipd` now has host/Niri integration with tolerant Niri JSON
  IPC models, bounded Unix-socket request/response helpers, focused-window
  best-effort attribution cache behavior, and fallback arming state-machine tests.
- `d2b-clipd` now has a real desktop notification backend for bounded,
  content-free fallback-ready and failure notifications.
- The flake now exports `packages.<system>.d2b-clipd` so host configurations can
  wire the clipboard authority user service without local package workarounds.
- Added clipboard architecture Nix/docs/schema wiring: a default-off
  `d2b.site.clipboard` module, user-service wiring for `d2b-clipd`, explicit
  picker package/path configuration with no bundled GPL input, bridge runtime
  path policy, eval assertions, and reference docs for the authority split,
  picker protocol, and policy caps.
- Added `d2b clipboard arm` CLI subcommand for the explicit picker-driven
  paste action; it sends an arm request to the running `d2b-clipd`
  control socket with bounded read/write deadlines, reports structured
  `--json` failures, and treats picker launch/handshake failure as a typed
  daemon failure rather than success-shaped clipboard writes.
- Added clipboard test gates: scaffold-detection test asserting `d2b-clipd`
  uses no `thread::park()` stub; picker handshake integration test
  (`CLIPD_TEST_PICKER` env gated); `policy_clipboard` contract tests verifying
  flake export of `d2b-clipd`, `Clipboard` subcommand existence in the CLI,
  Wayland proxy synthetic-clipboard invariant, and no-regression checks against
  reintroducing a substrate-gap marker.
- `d2b-clipd` now implements the host-authority event loop:
  connects to the host Wayland compositor via `ext-data-control-v1`
  (preferring the stable extension, falling back to `zwlr-data-control-v1`
  when only the WLR variant is advertised); subscribes Niri IPC events for
  focused-window attribution via `$NIRI_SOCKET`; uses the Niri event-stream
  cache for native clipboard events so the Wayland event loop does not block
  on synchronous compositor IPC; materialises host copy
  events with `FocusedWindowGuess` attribution; holds paste write-FDs open
  until the picker resolves or a 30-second deadline fires; launches the
  picker process over a `socketpair` using `CommandPickerSpawner`; falls
  back to native-paste arming (no synthetic input) when no picker command is
  configured; logs notification errors without exposing raw clipboard content.
- `d2b-clipd` now writes selected payloads to Wayland transfer FDs from a
  bounded helper task instead of blocking the daemon event loop, and bridge /
  picker failures emit content-free, rate-limited warnings such as
  `connect-failed`, `handoff-failed`, and picker-closed-before-selection.
- Added the `d2b-clip-debug wl-copy <text>` and
  `d2b-clip-debug wl-paste [mime]` Wayland probe binary for local clipboard
  validation without relying on session-only helper binaries.
- Staged the console/audio contract surface: public
  `ConsoleOp`/`AudioOp` wire DTOs, audio CLI JSON DTOs, provider
  console/audio capability descriptors for Cloud Hypervisor NixOS,
  qemu-media, and ACA sandboxes, generated schemas, the provider
  capability matrix reference, and a step-by-step console/audio how-to.
  The docs cover broker-owned qemu chardev posture, console stream QoS,
  OFD audio lock semantics, provider-specific enforcement modes, and
  d2b-wlcontrol badge/control constraints.
- Console output ring buffer (`d2b-core::console_ring`) with monotonic
  offset tracking, per-byte drop accounting, and fast-forward detection
  for slow clients. EOF flag propagation notifies waiters when the VM console
  closes.
- `d2b console <vm>` CLI command attaches to the VM console via the daemon,
  polls output in a 200 ms raw-mode loop, forwards stdin, supports Ctrl-]
  detach, and exits cleanly on EOF or session expiry.
- `ConsoleOp` daemon dispatch: `d2bd` handles all console operations
  (Attach, ReadOutput, WriteStdin, Resize, Wait, Close) via a per-VM
  `ConsoleSessionTable`. Cloud Hypervisor VMs get a d2bd-internal tokio
  drainer task that connects to the CH serial socket and reconnects on
  drop; qemu-media and ACA targets return typed errors directing operators
  to use the appropriate broker-fd or provider-relay path.
- `QemuMediaArgvInput.console_fd` field: when provided, QEMU emits
  `-chardev socket,id=con0,fd=N -serial chardev:con0` instead of
  `-serial none`.
  Accepts only fds >= 3 (rejects stdin/stdout/stderr).
- `d2bd` now dispatches `AudioOp` (status, set-volume, mute/off) for all
  provider types. Cloud Hypervisor NixOS VMs use OFD-locked atomic
  reads and writes of `/run/d2b/audio/<vm>.json` guarded by
  `/run/d2b/locks/audio-<vm>.lock`; qemu-media VMs report
  guest-enforcement as unsupported; ACA sandbox VMs route exclusively
  through provider guest-control (no local audio state is created).
  Provider capability resolution runs before any state access. Host
  PipeWire enforcement and guestd `AudioStatus` / `AudioSet` integration
  are connected, and responses report `host-and-guest`, `host-only`,
  `guest-only`, or `unsupported` according to the enforcement actually
  applied.
- `guestd` `AudioStatus` and `AudioSet` handlers now use real `wpctl`
  argv-only subprocesses targeting the workload user's PipeWire session.
  The `--wpctl-path` flag (set to `wireplumber/bin/wpctl` by the guest
  audio component) enables the runtime; capabilities are only advertised
  when the binary exists and the workload UID is known at startup.
  `PIPEWIRE_RUNTIME_DIR` is set per-user so wpctl never touches root's
  PipeWire socket. Level > 100 returns `AudioLevelOutOfRange`; missing
  PipeWire returns typed `AudioPipeWireUnavailable`.
- `audioService` (`d2b-<vm>-snd.service`) is fully retired: the field
  is unconditionally `null` in all manifest and daemon-access paths;
  `ProcessRole::Audio` is the sole source of truth for audio runner
  identity.
- `d2bd` now preserves a `TypedError::OtelHostBridgeReadinessTimeout` typed
  error as a structured `degraded` field in the `vm start` success JSON
  envelope when the OtelHostBridge readiness gate times out in non-strict
  mode. Operators and `d2b host doctor` can detect the degraded condition
  from the structured response without log parsing.
- `d2bd` now recognizes `uid=0` connections as a narrow `HostShutdown`
  authority scoped exclusively to `vmStop` during host-shutdown teardown. This
  fixes the long-standing post-reboot failure where the guarded `ExecStop`
  shutdown hook was rejected with `authz-not-admin` for every VM. Workload VMs
  stop before net VMs. All other admin-only operations (exec, USB attach, key
  rotation, host prepare, audit export) are explicitly denied for this role.
- Wayland proxy startup after reboot is now reliable: the broker grants the
  per-VM wlproxy UID a traverse ACL on the runtime directory
  (`/run/user/<uid>`) before spawning the proxy. This fixes post-reboot
  failures where the `0700` parent directory blocked access to the Wayland
  socket. The broker also verifies the runtime directory exists and is owned by
  the declared Wayland user before granting any ACL. Missing or mis-owned
  directories produce actionable `graphical-session-not-active` errors.
- USBIP backend ACL grant is now retry-safe across transient device
  re-enumeration: the retry loop tolerates device-node changes between verify
  and grant as long as VID/PID identity is stable, revokes ACLs from stale
  nodes before each retry, and treats missing old nodes as benign during revoke
  (kernel removes `/dev/bus/usb/B/D` during re-enumeration).
- Kernel module detection now checks `/sys/module` directory entries in
  addition to `/proc/modules` and `modules.builtin`. Built-in virtio/KVM
  modules compiled as `=y` are now correctly detected as present without
  the `D2B_SKIP_KERNEL_MODULE_CHECK` operator override.
- `d2b list --json` now exposes `guestClosureOutPath` for VMs whose
  bundle closure metadata is available, giving host-side scanners a public
  VM-to-guest-closure mapping for `sbomnix` without private path conventions.


- VM lifecycle CLI: added explicit `--force` / `-f` stop intent for
  `vm stop`, `down`, `vm restart`, and `restart`, with backward-compatible
  public wire serialization that omits `force = false`.
- Broker QMP lifecycle operations for qemu-media now expose typed
  `system_powerdown`, `query-status`, and `quit` requests, with bounded QMP
  parsing and audit-safe lifecycle fields.
- VM shutdowns now attempt provider-aware graceful guest shutdown for supported
  Cloud Hypervisor/qemu-media VMs before forced pidfd cleanup, with bounded
  daemon audit and metrics outcomes.
- Persistent shell CLI: added top-level `d2b shell <vm>` attach and
  management forms for persistent named guest shell sessions.

- UI colors: added a compositor-agnostic d2b color contract under
  `d2b.site.ui`, `d2b.envs.<env>.ui`, and
  `d2b.vms.<vm>.ui`, with resolved JSON and GTK-compatible CSS
  artifacts at
  `/etc/d2b/ui-colors.{json,css}` and a niri backend that renders
  active/inactive/urgent VM borders from the shared model. The CSS artifact
  uses GTK-compatible `@define-color` declarations with underscore names.

- Constellation observability: added `d2b op inspect` for bounded current
  operation and realm-state inspection, with optional TraceContext fields,
  degraded partial results, generated CLI schema coverage, and reference docs
  for redaction/cardinality constraints.

- Realm policy: added `d2b realm list` and `d2b realm inspect` to make
  host-resident vs gateway-backed realms discoverable, documented the
  default-deny cross-realm policy, and added migration guidance for explicit
  realm gateways.

- Display and virtual I/O: added explicit display capability helpers and a
  `d2b vm display list|close` gateway display-session surface that returns
  only bounded non-secret session metadata, including the authorizing
  operation id and principal. Added reference documentation that keeps
  display, clipboard, audio, USB/HID, GPU, video, and provider display
  streaming as separate opt-in capabilities.

- Runtime providers: added the host-side Cloud Hypervisor runtime provider
  adapter and explicit provider-selection policy. `local-cloud-hypervisor`
  remains the default VM runtime, plans carry only bounded provider/workload
  metadata, and crosvm, QEMU, Firecracker, and qemu-media ids fail closed
  rather than silently falling back. Firecracker-shaped selections refuse
  desktop, guest-control, virtiofs/store, graphics, audio, and USB workloads
  before side effects. Added reference documentation for runtime provider
  selection and cross-links from component/runtime docs.

- Constellation: added a preview remote full-host node adapter. A gateway
  guest can now register a remote d2b host as a named node in a realm,
  route typed lifecycle and exec/logs operations to it, and receive typed
  responses through the remote host's own `d2bd`/broker/guest-control
  stack. The adapter validates registration (node id, realm path, schema
  shape, capability set, authenticated gateway principal), tracks heartbeat/liveness,
  gates every routed operation against the node's declared capabilities, and
  enforces the non-tunneling boundary (no raw broker frames, no guest-control
  frame forwarding, no fd/pidfd transfer, no host path or credential
  exposure across the transport session). Remote-side idempotency deduplication
  is layered on top of the gateway-level dedup so reconnect recovery queries
  remote state before retrying side effects. Peer disconnect marks the node
  unavailable immediately, and new-generation re-registration makes
  old-generation operations fail stale. Relay identity remains reachability only
  and is never mapped to a local or realm principal. This adapter is
  **experimental/preview**: it is validated with mock and loopback peer clients
  only. Production transports (Azure Relay over a live WAN, QUIC, SSH),
  remote host install, remote host prepare, and network mutation are not yet
  supported. See `docs/reference/remote-full-host-nodes.md` for the full
  reference.

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
- CLI: `d2b host doctor --read-only` now surfaces
  `storage-lifecycle-report.json` with bounded issue kinds and inline
  remediation for storage/restart/sync contract drift.
- CLI: `d2b host doctor --read-only` now treats the private broker
  socket, optional metrics endpoint absence, and current swtpm namespace
  posture as healthy when those surfaces match the deployed policy.
- CLI: added `d2b host migrate-storage --dry-run`, which emits a
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
- ADR 0032 gateway lifecycle: gateway-mode `d2b vm
  start/stop/restart <aca target>` now routes through lifecycle
  operations backed by the ACA preview REST data plane. Gateway config
  can declare non-secret ACA subscription/resource-group/sandbox-group/
  region/image coordinates, and the provider creates/reuses disk images
  and sandboxes by d2b workload labels instead of shelling out to the
  preview `aca` CLI.
- ADR 0032 ACA display: gateway config can now carry the non-secret ACA
  managed-identity client id used by local validation probes, while the
  live display sender receives a gateway-minted short-lived Relay Send
  bearer instead of the long-lived Relay rule key.
- NixOS: added `d2b.site.usePrebuiltHostTools` so development hosts
  can validate source-built `d2b`, `d2bd`, and activation helper
  binaries before matching release prebuilts exist.
- CI: merging `main` after cutting a new dated changelog section now
  auto-tags the release and publishes pre-built `x86_64-linux` host
  binary tarballs for `d2bd`, `d2b`, `d2b-priv-broker`,
  `d2b-wayland-filter`, and `d2b-activation-helper`, alongside
  `SHA256SUMS`, on the matching GitHub Release.
- CI: after publishing a GitHub Release, the release workflow now
  computes Nix SRI hashes for each tarball, writes `nix/prebuilt.json`,
  and auto-commits the manifest back to `main` so consuming flakes can
  fetch the published host binaries by hash without manual updates.
- `d2b.vms.<vm>.qemuMedia.window.niriBorderColor` lets
  qemu-media host QEMU windows use a VM-specific niri border color.
  qemu-media windows now route through the d2b Wayland filter proxy
  so the generated niri include can match the VM-prefixed app-id
  `d2b.<vm>.*`.
- `qemuMedia` image-file sources can now be declared directly with an
  absolute `path` and `format = "raw"`; physical USB sources continue to
  use opaque refs plus config/probe-driven runtime selection.
- `d2b.vms.<vm>.qemuMedia.bootDrive.slot` adds boot-drive selector
  metadata for future qemu-media runtime planning without changing the
  current QEMU argv shape.
- ADR 0032 realm entrypoints now publish a host-visible
  `realm-entrypoints.json` table, allow separate gateway guests for
  separate realm/env segments, and add `d2b realm enter/run` plus
  manifest-backed realm target routing for gateway-backed VM verbs.
- ADR 0032 auth/audit foundations now define redacted daemon-access
  principal mapping records, tamper-evident audit-chain DTOs, daemon
  audit hash chaining, explicit audit-sink health reports, and
  host-boundary tests proving gateway relay/provider material stays out
  of host daemon artifacts.
- ADR 0032 peer protocol foundations now expose explicit
  handshake-accepted/rejected frames, bind codec schema fingerprints into
  plain and secure peer handshakes, and document the length-delimited
  semantic frame skeleton for future gateway transports.
- Named stream plumbing now supports resumable log cursors,
  deterministic stream draining, and retry-safe cancellation for future
  remote execution and display sessions.
- Remote execution groundwork now supports reliable reconnects, bounded
  retained-log reads, and safe repeated cancellation for future durable
  remote exec sessions.
- Documentation: ADR 0036 records the current qemu-media runtime contract,
  and ADR 0037 defines the shared local hypervisor runtime/service seam for
  qemu-media and Cloud Hypervisor/crosvm workloads.
- Capability negotiation now rejects operations and streams when a
  session lacks the required capability, with typed missing-capability
  errors.
- Gateway credentials can now be enrolled and rotated inside the gateway
  guest as a sealed runtime envelope, while host-side gateway credential
  reads and Relay Send bearer minting are rejected.
- Transport conformance now covers loopback session capacity, byte-exact
  concurrent sessions, shutdown, frame-cap rejection, truncated frames,
  capability intersection, stream backpressure, and retry-safe stream
  cancellation.
- Azure Relay now has a constellation `TransportProvider` adapter that
  wraps Relay WebSocket rendezvous into bounded transport sessions for
  gateway-owned listeners and sandbox senders.
- Local TCP test transport support now proves the transport interface is
  not Azure-specific, with loopback-only binds, explicit URI targets, and
  redacted typed errors for negative network cases.
- Host substrate provider adapters now wrap the existing host-check report
  for NixOS and generic Linux/Ubuntu capability discovery, returning typed
  remediation when prerequisites fail.
- Host-to-realm isolation is now documented and checked with a redacted
  host egress policy artifact, so host daemon/broker/CLI surfaces remain
  free of realm relay credentials and sessions.
- Provider-managed sandboxes: the Azure Container Apps adapter now handles
  provider-layer 429/rate-limit responses with `Retry-After`-aware
  backoff metadata and a shared circuit breaker. When the circuit is open,
  `Backpressure` errors include the remaining open duration. Probe attempts
  have a bounded timeout, stale probes reopen the circuit, and repeated
  transient failures use bounded exponential backoff with jitter. Concurrent
  429 responses from the same request batch can extend an already-open circuit.
  Circuit state is shared across provider instances targeting the same Azure Container Apps
  endpoint, subscription, resource group, and sandbox group so sibling
  instances cannot bypass the breaker for the same upstream. Retry hint
  metadata remains internal to the provider layer; no change to the public
  `ConstellationError` schema.
- Provider-managed sandboxes: Azure Container Apps adapter authentication now
  enforces workload identity first, then managed identity, in production.
  Ambient developer credential chains (Azure CLI tokens, environment
  variable secrets, developer-toolchain fallbacks) are not present in the
  production resolution order. Non-production local-validation contexts
  inject test credentials explicitly and are not a runtime fallback.
- Provider-managed sandboxes: Azure REST error diagnostics are
  now gated by an allowlist. Only case-stable allowlisted `error.code`
  values (or `unknown`), a length-bounded sanitized `error.message`, the
  HTTP status code, and the opaque `x-ms-correlation-request-id` header
  appear in provider errors, structured log spans, and audit records.
  Full response bodies, endpoint URLs, subscription IDs, internal diagnostic
  details, resource IDs, tokens, payload content, and workload output are never
  forwarded.
- Documentation: added `docs/reference/provider-managed-sandboxes.md`
  covering the Azure Container Apps adapter capability matrix, absent capabilities,
  rate-limit/backoff/circuit behavior, credential boundary, diagnostics
  redaction rules, error shapes, `provider-managed-isolation`, and scope
  limitations including the absence of guestd, systemd, broker, KVM, vsock,
  cgroup, namespace, SSH, and full-host lifecycle. Cross-referenced from
  `docs/reference/remote-full-host-nodes.md`.

### Changed

- **BREAKING:** Unsupported operations and streams (file-copy, port-forward,
  clipboard, audio, and device streams) are explicitly rejected with an
  unsupported error instead of falling back to generic byte streams. Older
  clients that relied on fallback byte streams must route only supported
  operations: lifecycle, exec, logs, persistent shell, node health, and display.
- **BREAKING:** Legacy `d2b.gateways` and nested gateway/ACA sandbox
  configuration now fail evaluation with migration errors that point operators
  to `d2b.realms`; configs using those legacy surfaces must migrate before
  evaluation succeeds.
- **BREAKING:** Explicit `d2b://` CLI targets that omit the reserved `.d2b`
  suffix now fail with a target grammar diagnostic instead of falling back to
  local VM routing.
- Routed CLI VM target resolution through the realm access contract DTOs while
  preserving the existing local VM fast path and manifest-backed gateway
  behavior until the daemon access API is implemented.
- Renamed the Rust realm foundation crates from
  `d2b-constellation-*` to `d2b-realm-*` without changing runtime behavior,
  establishing realm-native package/import names for follow-up parser and DTO
  work.
- Aligned realm-core reference pages and generated schema companions
  with `d2b-realm-core` naming and the realm-qualified target
  grammar.
- Kept realm operator-troubleshooting identifiers visible in Rust `Debug`
  output while preserving redaction for credential-, key-, and principal-like
  identifiers.
- Tightened `allocator.json` config typing so realm paths, provider kinds,
  and transitional env-bridge modes carry bounded schema/runtime validation.


- `d2b-wayland-proxy` now treats `graphics.waylandProxy.border.thickness`,
  `graphics.waylandProxy.border.label.position`, `--border-thickness`, and
  `--border-label-position` as deprecated legacy shape knobs; generated d2b
  proxy runners use the fixed-width left wrapper rail and vertical label.
- Reference docs (`cli-contract.md`, `daemon-api.md`, `display-io-capabilities.md`,
  `runtime-provider-selection.md`, `components-audio.md`, `error-codes.md`) now
  point `console` and `audio` surfaces at the provider capability
  matrix.
- The host-side `d2b-wayland-proxy` proxy is source-built from the checked-out
  workspace even when other host tools use release prebuilts, so local eval
  gates do not depend on a matching release tarball for this policy binary.
- The `with-entra-id` eval workflow now overrides GitHub inputs to the
  committed lock revisions and authenticates Nix fetches with the Actions token
  to avoid transient unauthenticated API rate limits.
- Host activation grants `d2bd` narrow access to the Wayland user's
  PipeWire/Pulse sockets so daemon-owned audio policy enforcement does not
  depend on host-local ACL overrides.
- Console drainers now spawn on a daemon-owned Tokio runtime even when console
  attach requests are handled by synchronous public-socket worker threads.
- Clipboard documentation now follows Diataxis placement more closely: the
  architecture overview is indexed as Explanation, while `d2b-clip-debug`
  diagnostic command examples live in the clipboard picker how-to.
- Renamed the project to **d2b: Double Dutch Bus** as an intentional breaking
  change. Commands, packages, services, sockets, Nix options, runtime paths,
  schemas, telemetry identifiers, and generated artifacts now use only `d2b`
  naming; old names are unsupported and no compatibility aliases are provided.


- Public daemon list/status handling now uses a request-scoped artifact snapshot
  so manifest, process, host, and bundle resolver reads are shared within one
  request without cross-request caching.
- VM activation now keeps guest systemd isolated from the host: `switch
  --apply`, `test --apply`, and live `rollback --apply` fail closed when the
  VM is stopped/offline or does not advertise the guest activation capability,
  while `boot --apply` is the explicit offline staging path for the next start.
- `d2bd` now orchestrates live VM activation as broker prepare,
  guest-control activation, and broker commit, with per-VM serialization,
  crash-consistent pending markers, bounded activation metrics, and degraded
  status/list reporting for unresolved activation state.
- Guest-control now exposes authenticated in-guest system activation start/status
  RPCs, with guestd-owned transient systemd units and restart-safe status.
- Examples and the default template now describe the daemon-only lifecycle,
  Rust CLI, and `d2b` group authorization model without stale per-VM
  systemd, polkit, route-preflight, or bash-CLI references.
- Human `d2b status <vm>` output now labels daemon and runner state with
  daemon-owned terms instead of retired per-VM systemd template names.
- Broker user-namespace sync-pipe creation and parent-side sync I/O now use safe
  fd wrappers while preserving `O_CLOEXEC`, cleanup, and reap semantics.
- The PR checklist and policy tests now include an efficiency ratchet for host
  gate N/A justifications, AI/model metadata hygiene, metric label cardinality,
  noisy PID logging, and file-wide unsafe-code allowances.
- Runtime capability projection for qemu-media list/status output now goes
  through focused helpers with direct regression coverage, preserving public JSON
  and human output shape.
- Provider/realm policy coverage now explicitly guards host daemon, broker, and
  bundle artifacts from storing realm credentials, remote registries, or realm
  audit state while keeping capability-denied remote dispatch fail-closed.
- Renamed the internal contract/DTO crate to `d2b-contracts` and added
  workspace taxonomy checks for contract and standalone workspace coverage.
- CI workflow make-target policy coverage moved from a shell meta gate to a Rust
  contract test with pinned successor coverage.
- The Tier 0 first-pass implementation moved under `tests/tools/` while keeping
  the stable `make check-tier0` target.
- `storage-lifecycle-report.json` now includes bounded `contractId` and
  `offendingId` fields on storage/sync contract validation issues, and broker
  storage I/O diagnostics redact absolute managed paths as
  `storage-path#<hash>`.
- QEMU media's redacted registry index is now private to declared daemon/broker
  readers (`0640`) and QEMU media state lives under a dedicated per-VM
  `qemu-media` subdirectory.

- `d2b vm exec` and `d2b vm exec -d` no longer time out with
  `guest-control-timeout` (exit 69) when a long-running GUI application
  (such as Firefox) is starting up in the target VM. During peak startup
  the VM is under heavy virtiofs I/O load; vsock connection setup and the
  six-step authenticated handshake (connect, Hello, broker sign,
  Authenticate, broker sign, Health) each approach their 3-second per-op
  cap, requiring up to 18 s of budget. The establishment deadline was
  raised from 12 s to 20 s (covering all six operations at their full cap
  plus 2 s headroom). The detached-create RPC deadline was raised from
  12 s to 20 s for the same reason.
- Broker VM activation requests now split store-view preparation, guest-completed
  metadata commit, and offline metadata-only staging so the privileged broker no
  longer executes VM `switch-to-configuration` scripts on the host.
- `/run/d2b` tmpfiles ACL ordering now reasserts the ACL mask after per-VM
  traversal entries, so `d2bd` keeps effective write access to its daemon
  lock after a host switch.
- `qemu-media` TAP synchronization locks now render the TAP identifier as a
  resource id instead of a non-path `pathTemplate`, so the generated `sync.json`
  deserializes through the Rust `SyncJson` DTO and `d2bd` can load the bundle
  after hosts with qemu-media VMs switch.
- Broker SIGCHLD reaper startup now installs the child-signal stream before the
  runtime is returned, closing a load-sensitive child-reap race surfaced by the
  broker reap tests.
- Daemon startup no longer lets the diagnostic bridge preflight pre-skip
  autostarted net VMs on cold boot; net VMs now get to run their host-prep DAG
  and workloads degrade only if their env net VM actually fails to start.
- Host OTel collector no longer has directory write authority over
  `/run/d2b/otel`: the collector's access ACL is now `--x` (traverse only,
  no create/unlink authority on `host-egress.sock` or sibling entries). The
  default ACL is retained at `rw` (not `rwx`) with a clamped `rw` mask so the
  collector inherits read+write on `host-egress.sock` when the broker-spawned
  bridge creates the socket after boot, while execute bits never propagate to new
  entries. `StartLimitIntervalSec = 0` ensures systemd does not permanently
  disable the service if the bridge socket is momentarily absent across restarts.
- The `/run/d2b` runtime parent is now root-owned with an explicit
  `d2bd` ACL, avoiding systemd-tmpfiles unsafe-path-transition failures
  that skipped per-VM `guest-control` runtime directories after reboot.
- Broker request handling no longer emits a per-request `Bundle resolver loaded`
  info log, and USBIP proxy reconciliation now treats absent locked hardware as
  a non-fatal ACL-refresh skip instead of spamming paired broker/daemon warnings.
- `storage.json` validation-evidence rows now use bounded contract identifiers
  for actor values, so rendered fixtures deserialize through the Rust storage
  contract DTOs.
- Activation now grants every numeric per-role runtime UID traversal on both
  `/run/d2b` and `/run/d2b/vms`, so broker-spawned runners can reach
  their per-VM runtime socket directories after a host switch.
- Public daemon `status` keeps guest USBIP import state, but now uses a short
  status-specific guest-control budget so stale or slow guest USB probes cannot
  push wlcontrol past its public-socket timeout.
- Public daemon `list` and `status` responses now build per-VM status entries
  in parallel, so slow provider or guest-control probes cannot serially push
  wlcontrol past its public-socket timeout.
- Daemon-native runtime parent directories under `/run/d2b/vms`,
  `/run/d2b-gpu`, `/run/d2b-video`, and `/run/d2b-wlproxy`
  are root-owned again while preserving daemon-owned per-VM leaves, so
  broker path-safety checks no longer reject VM starts after a host switch.
- `d2bd.service` now reports systemd readiness only after the daemon has
  rebound its public socket and completed startup adoption, so post-switch
  scripts no longer race `/run/d2b/public.sock`; daemon updates may restart
  `d2bd` without restarting running VMs, which are re-adopted afterward.
- VM stop/restart now has the Nix and manifest configuration surface for
  provider-aware graceful guest shutdown, including global/per-VM enable and
  1–600 second timeout controls, `manifestVersion = 7`, daemon-config
  rendering, and host-shutdown `d2bd.service` ordering/timeout budgeting.
- Host shutdown and reboot now gracefully stop workload VMs before env net VMs.
- Broker disk initialization now validates existing d2b-owned ext4 raw
  images before treating `ifAbsent` as satisfied, automatically repairs safe
  declared owner/mode drift, safely formats only proven-empty images, and fails
  closed before VM spawn for malformed or ambiguous image data.
- Persistent shell guests now render the shpool daemon unit with a store-backed
  start script instead of an inline multi-line `ExecStart` command, avoiding
  systemd quote parsing failures.
- Persistent shell attach/detach now accepts daemon owner error frames that carry
  the envelope `opId`, matching the successful shell response and exec owner
  framing.
- Persistent shell detach now treats a successful daemon best-effort close as a
  clean owner close even if the first close-attach RPC reports a transient
  guest-control transport error.
- Persistent shell detach now treats the known close-attach transport-unavailable
  response as a successful local detach, matching the daemon's owner-disconnect
  cleanup semantics.
- Persistent shell attach now starts the guest shpool daemon, probes readiness
  through the workload helper, and wires shell terminal RPCs to the PTY-backed
  attach helper instead of returning disabled shell I/O.
- `d2b list --json` now preserves daemon-reported failed lifecycle state as
  `status = "failed"` instead of collapsing it to `unknown`.
- `d2b list` and `d2b status` now use a short provider-status probe
  timeout instead of the graceful shutdown operation timeout, keeping status
  queries responsive when a VM's provider API socket is slow.
- `d2b usb attach <vm> <busid> --apply` now fails immediately for stopped
  VMs with copy-pasteable start-and-retry remediation instead of surfacing a
  generic guest-control transport failure.
- Privileged USB broker IPC now rejects malformed bus IDs, traversal-shaped
  intent identifiers, and invalid module names before host USB/module actions,
  rate-limits direct broker peers plus daemon-forwarded requests by stable
  bounded/evicted UID/role/operation buckets with separate direct/daemon bucket
  pools so peer floods cannot starve daemon-forwarded operations, caps audit
  writes, returns typed fail-secure peer-credential refusals, and redacts
  sensitive USB/bundle details from public error envelopes.
- Broker audit write limiting now gives unprivileged refusal spam a separate
  bounded bucket from privileged operation audit writes and records visible drop
  counters when the limiter refuses audit records while aggregating journal
  warnings for dropped records.
- USBIP bind/unbind broker requests now carry only bundle-resolved opaque
  intent IDs, with broker-side physical VID/PID and bus/port topology checks
  before bind or replay.
- USB serial-correlation key-rotation audit records are now deduplicated per
  previous/current key pair so repeated USB binds during a grace window do not
  consume privileged audit tokens.
- USBIP proxy firewall carve-outs now fail closed unless they can scope TCP/3240
  to the env's uplink bridge, host bridge IP, and host-visible net-VM source IP;
  proxy listeners also reject wildcard bind addresses.
- USBIP detach now fails with an actionable `usbip-revocation-not-isolated`
  error unless immediate stream revocation can first block/withdraw the firewall
  carve-out and then target a proven VM/proxy conntrack or TCP socket tuple
  whose source is not SNAT-obscured and whose anti-spoofing posture is proven,
  preserving the USBIP session claim instead of silently leaving an established
  stream or bouncing unrelated same-env streams.
- USBIP carrier cleanup now keeps withdrawing firewall state and unbinding the
  host carrier when guest detach fails because the VM is dead or unreachable,
  while preserving the failed guest-detach report for degraded/audit visibility.
- USBIP step and revocation failures now name the target busid while keeping
  remediation concise and free of raw sysfs paths or serials.
- USBIP attach failure rollback now filters shared per-env backend/proxy
  sidecar checks out of the single-busid rollback order, avoiding disruption
  to unrelated same-env USBIP streams.
- USBIP host unbind now drains bounded helper stderr concurrently so verbose
  failures cannot stall detach/restart cleanup before the helper exits.
- USBIP bind now revokes the backend device ACL and unbinds the host carrier if
  the terminal broker success audit record cannot be written, avoiding
  unaudited bind state after audit rate limiting or write failures.
- USBIP bind/unbind error paths now release busid locks after failed bind
  convergence, ACL grant rollback, or post-unbind ACL revoke failures unless the
  device is still proven bound to `usbip-host` for manual recovery.
- VM start now reconciles same-host-session same-VM USBIP claims after guest-control
  readiness by replaying host bind/proxy state and re-importing in-guest
  devices; stop/restart cleanup now preserves the session claim and refuses
  sysfs host unbind unless firewall withdrawal plus targeted stream cleanup can
  be proven first.
- VM stop/restart now suppresses USBIP cleanup degradation when the trusted
  bundle is unavailable and no matching host-session lock exists, while still
  warning if a matching lock exists or lock probing fails.
- USB serial-correlation HMAC key rotation windows now emit a broker audit
  record with only key IDs and rotation metadata, preserving the forensic trail
  without logging key material or raw serials.
- `d2b usb probe` and `d2b status` now split USB session claim,
  host, guest, topology/policy, degraded reason, and remediation state so stale
  lock-only USBIP claims are not reported as healthy bound devices.
- USBIP reference docs and CLI output artifacts now document the host-session
  claim versus active carrier model, including that `/run/d2b/locks/usbip`
  survives VM stop/restart and daemon restart but not host reboot, plus restart
  reconciliation, probe JSON schema, degraded reasons, prerequisites, and
  copy-paste remediation commands.
- Required USBIP policy failures during VM-start claim replay now fail before
  device exposure and roll back boot, while runtime absence/proxy/guest
  availability issues remain visible degraded USB state.
- VM stop/restart USBIP cleanup now has a reusable daemon plan that detaches
  guest imports, withdraws host carrier/firewall/flow state when safely
  isolated, preserves same-VM USBIP session claims for restart, and reserves
  claim release for successful explicit detach.
- `/run/d2b/locks/usbip` is now created by tmpfiles before daemon/broker
  startup as `root:d2bd 0750`, keeping USBIP lock claims broker-written
  while allowing daemon status reads.
- `d2b-priv-broker.service` now explicitly uses `Delegate=true` and
  `KillMode=process` in `d2b.slice` so broker restarts do not tear down
  broker-spawned runner cgroups.
- Broker-spawned runners now use `clone3(CLONE_INTO_CGROUP)` against the
  systemd-delegated `d2b.slice` role leaf when available, with the legacy
  `cgroup.procs` attach kept only as the fork fallback; `d2b.slice`
  delegates the required `cpuset` controller as well.
- The broker's `cgroup.procs` fallback now writes the child PID from the parent
  before releasing user-namespace runner children, avoiding in-namespace
  cgroupfs permission failures.
- Runtime per-VM socket directories and store-view top-level directories are now
  created and postured by tmpfiles instead of ad-hoc activation mkdir/chown/chmod
  snippets.
- Host tmpfiles ACL rules now append entries instead of replacing previous ACLs
  on the same path.
- Per-VM state-root posture, static state traversal ACLs, TPM parent traversal
  ACLs, and next-generation runtime leaf directories are now tmpfiles-owned;
  net-VM `var.img` creation/posture remains broker `DiskInit`-owned instead of
  being repaired by host activation.


- CI: nix-unit eval coverage is now split into multiple
  `nix-unit-<shard>` flake checks plus a cheap global `nix-unit`
  presence/pin check, so PR flake evaluation can fan the slow corpus
  out across the existing x86 matrix.
- **Breaking:** VMs with `d2b.vms.<vm>.usbip.yubikey = true` must
  now also enable `d2b.vms.<vm>.guest.control.enable = true`. USBIP
  guest attach/detach is owned by guestd over authenticated
  guest-control; there is no SSH fallback.
- Broker: `d2b-priv-broker.service` now defaults `RUST_LOG` to
  `info` instead of `debug`, keeping high-volume broker diagnostics out
  of normal journal/OTel log exports unless an operator opts into debug
  logging.
- CI: the PR aarch64 flake leg now runs only the lightweight
  `smoke-eval-aarch64.nix` check instead of the full native aarch64
  flake sweep.
- NixOS module: `d2b.site.usePrebuiltHostTools = false` now also
  forces `d2b-priv-broker` to build from the local source checkout,
  keeping the broker wire/bundle parser aligned with `d2bd`.
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
- Daemon audit JSONL records now carry `prev_hash` and `record_hash`
  chain fields, with verifier and sink-health helpers that report
  tampering, write unavailability, and retention-floor degradation
  without exposing filesystem paths or credential-shaped detail.
- `qemu-media` VMs now emit a typed QMP-only QEMU runner process node
  instead of being absent from `processes.json`; they still do not emit
  Cloud Hypervisor, store/virtiofs, or guest-control runner data.
- Daemon and CLI list/status output now include positive
  `runtimeCapabilities` and `serviceCapabilities` alongside unsupported
  capability summaries.
- Runtime/provider capability metadata now carries shared `operations`
  and `services` summaries for both Cloud Hypervisor-backed NixOS VMs and
  qemu-media VMs while keeping the legacy flat capability booleans.

### Fixed

- Hardened route refresh handling so stale/equal advertisements cannot downgrade
  topology or capabilities, expired entries are treated absent without hot-path
  full sweeps, admissions remain atomic without full map cloning, and normal
  proactive refresh sequences do not exhaust replay capacity.
- Added fixed in-memory capacities to the pure realm route engine so valid
  unexpired parent, route, and replay advertisement state rejects new entries
  fail-closed instead of growing without bound.
- Bounded the pure realm route engine's replay/route expiry state, refreshed
  existing route capabilities and ids on newer advertisements, and made
  local-root route capability decisions explicit.
- Aligned protobuf authorization-scope encoding with identity lifecycle scopes
  and route contracts.
- Documented the private `realm-controllers.json` contract for deterministic
  host-local realm controller unit/socket naming, direct realm socket
  authorization, local-root allocator resolution, state/audit separation, and
  the access-layer/routing boundary.
- Added Layer-1 allocator coverage for rendered `allocator.json` bundle wiring,
  realm allocator metadata, fake-engine conflict/replay/reconcile paths, and
  bounded reconciliation reports.
- Added the public `d2b.realms.<realm>` Nix option schema foundation for
  the realm-native control plane without changing existing `d2b.envs`
  runtime behavior, with normalized realm index metadata, eval-time
  assertions for realm identity/parent/path uniqueness, and reference
  documentation for placement, user access, provider/relay/policy/key
  references, and the transitional env/network bridge.
- Added Layer-1 nix-unit coverage for the realm option schema,
  normalized realm index, parent/path collision assertions, legacy gateway/ACA
  migration guidance, and minimal/multi-env eval compatibility.
- Added realm `placementProvider` metadata and AF_UNIX socket path length
  assertions for realm public and broker socket declarations.
- Added the `RealmTarget` parser with canonical realm-qualified
  rendering, bare-alias ambiguity diagnostics, and old node-qualified target
  migration errors.
- Added an accepted realm-native control-plane decision record, superseding the
  host-centric constellation model with per-realm daemon, broker, state, and
  audit boundaries; first-class `home`, `dev`, and `work` realms to replace
  the legacy grouping model; and a clean cutover from old realm/ACA sandbox
  surfaces into `d2b.realms`.
- Added core realm data models for controller placement,
  access bindings, provider/workload placement summaries, tree route
  advertisements, enrollment/key lifecycle metadata, and migration-error
  envelopes.
- Documented the stable public-socket discovery contract for persistent shells
  so desktop clients such as `d2b-wlterm` can use `List`/`Status` plus
  `ShellOp::List` without scraping human CLI output or leaking terminal state.
- Added typed deserialization support for public daemon response envelopes so
  downstream clients can parse `PublicResponse` data without falling back to
  untyped JSON.
- Exported `packages.<system>.d2b-wayland-proxy` and added a supported
  `--host-terminal` launch path for VM-bound host terminals. The launcher creates
  randomized single-use Wayland and WezTerm mux sockets under a private
  `$XDG_RUNTIME_DIR` directory, waits for proxy readiness before launching the
  foreground child, preserves d2b clipboard mediation, and keeps privileged
  Wayland globals hidden.
- Documented the optional desktop terminal integration stack (`d2b-toolkit`,
  `d2b-wlterm`, and WeezTerm) with exact flake-input follow boilerplate,
  Home Manager wiring, Waybar setup, and validation commands.
- Added CTAP/WebAuthn security-key proxy: `d2b.host.usb.securityKey.*` and
  `d2b.vms.<vm>.usb.securityKey.enable`. The host broker (`d2bd`) serializes
  CTAP HID traffic from opted-in VMs to a host-attached FIDO2 device (YubiKey
  or equivalent) over AF_VSOCK, without USB device ownership transfer. Each
  opted-in VM receives a daemon-supervised virtual FIDO2 HID device
  (`/dev/hidraw*`) via Linux `/dev/uhid`; browsers and `libfido2` treat it as
  a normal local security key.
- Added `d2b usb security-key status|sessions|cancel|test` subcommands for
  lease inspection, session history, stuck-request cancellation, and
  per-VM smoke checks.
- Added structured notification/event emission for security-key lifecycle
  events (ceremony start, user-presence wait, contention, failure, lease
  revocation) through the d2b notification subsystem, with durable
  `/run/d2b/usb-sk/events.jsonl` and a machine-readable
  `/run/d2b/usb-sk/lease.json` state file.
- Notification actions (`Cancel active request`, `Open status`) include
  single-use, high-entropy nonces bound to session/action/expiry; `d2bd`
  rejects missing, expired, reused, or mismatched action tokens.
- Added eval-time assertions: `usb.securityKey.enable = true` and
  `usbip.yubikey = true` are mutually exclusive for the same VM; per-VM
  security-key opt-in requires the host `usb.securityKey.enable = true`;
  per-VM opt-in requires `guest.control.enable = true`.
- Added Diataxis documentation:
  - How-to: [`docs/how-to/use-usb-security-key.md`](docs/how-to/use-usb-security-key.md)
  - Migration: [`docs/how-to/migrate-usbip-yubikey-to-security-key.md`](docs/how-to/migrate-usbip-yubikey-to-security-key.md)
  - Reference (options, CLI, event/notification JSON): [`docs/reference/components-usb-security-key.md`](docs/reference/components-usb-security-key.md), [`docs/reference/usb-security-key-events.md`](docs/reference/usb-security-key-events.md)
  - Explanation (CTAP proxy architecture, why not USB sharing, comparison with USBIP and Qubes): [`docs/explanation/usb-security-key-architecture.md`](docs/explanation/usb-security-key-architecture.md)
- Added the `d2b-notify` crate: a reusable notification/event mechanism for
  d2b desktop UX, including:
  - Typed `SecurityKeyEvent` enum covering all CTAP/WebAuthn ceremony
    lifecycle phases: `Started`, `TouchNeeded`, `Busy`, `Queued`, `Blocked`,
    `TimedOut`, `Failed`, `Canceled`, `Completed`.
  - `ActionNonceStore`: single-use CSPRNG nonces (32 bytes, 64-char hex)
    bound to `session_id`/`action_key`/expiry, with fail-closed validation
    that prevents notification-action replay by hostile desktop clients.
  - `SkNotifyState`: durable JSON state format (schema version 1) written by
    the host runtime to `/run/d2b/notify/sk-state.json`; read by the Waybar
    helper and `d2b-wlcontrol`.
  - `WaybarBlock` + `waybar_block_from_state`: Waybar JSON-protocol block
    derived from the current ceremony state, with per-state CSS classes
    (`d2b-sk-idle`, `d2b-sk-touch`, `d2b-sk-busy`, `d2b-sk-active`).
  - `WlcontrolSkStatus`: data contract for the `d2b-wlcontrol` status/action
    surface, with pluggable per-ceremony action builder for nonce-backed
    buttons.
  - `d2b-sk-waybar-helper` binary: reads the durable state file and emits
    one Waybar JSON line to stdout; suitable as a `custom/d2b-sk` `exec`
    target.
  - Pluggable `Notifier` trait + `RecordingNotifier` for hermetic tests;
    per-event builders for all user-visible ceremony transitions.
- Added `nixos-modules/notifications.nix`: NixOS module with options
  `d2b.notifications.enable`, `d2b.notifications.statusHelper.{enable,
  package,executablePath}`, `d2b.notifications.integrations.waybar.enable`,
  `d2b.notifications.securityKey.{enable,staleEntryTtlSecs}`, and
  `d2b.notifications.runtime.stateDir`; creates the
  `/run/d2b/notify` tmpfiles directory on activation.
- Added `d2b usb security-key` subcommand family exposing four operator surfaces
  for the CTAP/WebAuthn security-key proxy feature:
  - `d2b usb security-key status [--json|--human]` — show proxy health,
    configured physical keys, per-VM virtual-device health, and current lease.
  - `d2b usb security-key sessions [--json|--human]` — list recent and active
    security-key request sessions, VM, RP ID, outcome, and timeout.
  - `d2b usb security-key cancel {<session-id>|--current} [--dry-run|--apply]
    [--json|--human]` — cancel a stuck security-key request session; `--dry-run`
    shows the planned `SecurityKeyProxyCancelSession` broker op.
  - `d2b usb security-key test <vm> [--dry-run] [--json|--human]` — smoke-check
    that the guest virtual HID device and the host broker's physical-key
    visibility are healthy; `--dry-run` shows the two planned checks.
- Added USB security-key wire contract types in `d2b-contracts::public_wire`:
  `UsbSecurityKeyStatusRequest/Response`, `UsbSecurityKeySessionsRequest/Response`,
  `UsbSecurityKeyCancelRequest/Response`, `UsbSecurityKeyTestRequest/Response`,
  and supporting DTOs (`UsbSkPhysicalKeyStatus`, `UsbSkVirtualDeviceStatus`,
  `UsbSkLeaseStatus`, `UsbSkLeaseState`, `UsbSkSession`, `UsbSkSessionOutcome`,
  `UsbSkTestCheck`).
- Added CLI output types in `d2b-contracts::cli_output`:
  `UsbSkStatusOutputV1`, `UsbSkSessionsOutputV1`, `UsbSkCancelDryRunOutputV1`,
  `UsbSkTestDryRunOutputV1`.
- Added CLI golden tests under `packages/d2b/tests/usb_sk_contract.rs` covering:
  `usb security-key --help`, `cancel --current --dry-run`, `test <vm> --dry-run`,
  and `not-yet-implemented` exit-78 envelope for all live paths.
- Extended `packages/d2b/tests/cli_json_output_contract.rs` with
  `usb_security_key_dry_run_outputs_match_goldens`,
  `usb_security_key_status_not_yet_implemented`, and
  `usb_security_key_sessions_not_yet_implemented` tests.
- The use of "security key" as the user-facing term for CTAP/WebAuthn
  authenticators is now established in CLI help, JSON envelopes, and docs.
  The FIDO/CTAP terminology is reserved for diagnostic output and technical docs.

  The live paths (`status`, `sessions`, `cancel --apply`, `test <vm>` without
  `--dry-run`) emit exit 78 with a `not-yet-implemented` envelope until the
  daemon broker handler lands in a later workstream.
- Added `d2b.host.usb.securityKey.enable` and `d2b.host.usb.securityKey.devices`
  (stable FIDO device selector submodule with `vendorId`, `productId`, `serial`,
  and `label` fields) to declare the host USB security-key proxy.
- Added `d2b.vms.<name>.usb.securityKey.enable` per-VM opt-in for CTAP/HID
  relay to a host-proxied FIDO security key, guarded behind the new host option.
- Eval-time assertions: VM `usb.securityKey.enable` requires the host proxy to
  be enabled; `usb.securityKey.enable` and `usbip.yubikey` are mutually
  exclusive for the same VM (phase-1 constraint); device `vendorId` values must
  be within the FIDO-class allowlist; device labels must be unique.
- Rust DTO module `d2b_contracts::security_key` with typed wire contracts:
  `SecurityKeyStatusResponse`, `SecurityKeySessionsResponse`,
  `SecurityKeyCancelRequest/Response`, `SecurityKeyEvent` (7 variants),
  `SecurityKeyOpenDeviceRequest`, `SecurityKeyApplyUdevRulesRequest`, and
  opaque-ID newtypes `SecurityKeySessionId` / `SecurityKeyDeviceLabel`.
- `PublicRequest` / `PublicResponse` variants for `UsbSecurityKeyStatus`,
  `UsbSecurityKeySessions`, and `UsbSecurityKeyCancel`.
- `BrokerRequest` variants `SecurityKeyOpenDevice` and
  `SecurityKeyApplyUdevRules` with `op_name()` dispatch arms.
- `W3BrokerOperation::SecurityKeyOpenDevice` and
  `W3BrokerOperation::SecurityKeyApplyUdevRules` with wire tags, flags, and
  capability advertisement.
- Privilege matrix rows for `usb security-key` (public) and the two new broker
  operations; dispositions doc stubs for both broker ops.
- `usb security-key status`, `usb security-key sessions`, and
  `usb security-key cancel` CLI contract stubs in
  `docs/reference/cli-contract.md` (daemon not yet wired, phase 1).
- Nix-unit eval cases (`tests/unit/nix/cases/usb-security-key.nix`) and
  assertion rejection cases for all new eval-time constraints.
- Contract + policy tests in `packages/d2b-contract-tests/tests/usb_sk_contract.rs`
  (20 tests).
- Added `d2b.vms.<name>.usb.securityKey.enable` option. When `true`, the guest
  VM gets a virtual FIDO2 HID device via a CTAPHID UHID frontend relay
  (`d2b-sk-frontend`). The guest-side binary opens `/dev/uhid`, creates a
  virtual HID device, and relays 64-byte CTAPHID reports over an AF_VSOCK
  connection to the host broker (VSOCK port 14320). Firefox and libfido2
  discover the virtual `/dev/hidraw*` device via the `fido` group; no root
  or physical USB access is required inside the guest.
- `d2b-sk-frontend` static guest binary: fully static (musl) binary for the
  guest CTAPHID UHID relay frontend. Implements exponential backoff VSOCK
  reconnect (1 s–60 s), clean UHID device recreation across reconnects, and a
  simple 4-byte length-prefix framing protocol. Uses VSOCK port 14320.
- Host DAG node `sk-frontend` (role `security-key-frontend`): a no-runner
  tracking node whose readiness predicate fires when the host broker's vsock
  socket (`<stateDir>/vsock.sock_14320`) appears. Edge: `cloud-hypervisor →
  sk-frontend`.
- Mutual exclusion assertion: `d2b.vms.<name>.usbip.yubikey` and
  `d2b.vms.<name>.usb.securityKey.enable` cannot both be `true` for the same VM
  (both claim the FIDO2 device endpoint).
- `qemu-media` runtime incompatibility assertion for `usb.securityKey.enable`
  (the CTAPHID proxy requires the Cloud Hypervisor / nixos runtime).
- `securityKeyVsockPort = 14320` constant added to `d2b` lib for use by host
  broker and guest component modules.
- Nix eval tests (`security-key-gating.nix`): manifest `securityKey` field
  gating, DAG node presence/absence, assertion firing for the yubikey and
  qemu-media conflicts.
- Contract tests (`minijail_sk_frontend.rs`): source-grep assertions for the
  sk-frontend minijail profile block in `minijail-profiles.nix`, including role,
  seccompPolicyRef presence, and empty capability set; compile-time
  `ProcessRole::SecurityKeyFrontend` variant and serde round-trip check.
- Added `OpenHidrawSecurityKey` broker op: resolves a configured FIDO security-key
  stable selector, opens the physical `hidraw` node, and passes the fd to `d2bd` via
  `SCM_RIGHTS`. Includes the privilege-matrix row, audit fields, and dispatch wiring.
- Added `d2bd::security_key` session management module: CTAPHID relay with CID
  isolation/translation, a one-active-ceremony-per-key lease state machine (default
  120s ceremony timeout, 15s queue-wait timeout), length-prefixed 64-byte report
  framing, and a `SO_PEERCRED`-based per-VM socket peer authentication check. Raw CTAP
  payloads, PINs, and credential material are never logged.

- Added the `d2b.envs.<env>.externalNetwork.*` option and normalized-index metadata
  surface for net-VM-owned external network attachment, egress, port-forward, and mDNS
  policy.
- Net VMs can opt into external network mDNS reflection and an optional `.local`
  dnsmasq bridge without running Avahi or opening UDP/5353 on the host.
- Nix-unit coverage and minimal eval wiring for opt-in per-env external network net VM
  interfaces, egress carve-outs, port forwards, mDNS reflection, and `.local`
  forwarding.


- Host-local realm daemons now emit only strict `DaemonConfig` fields
  and use realm-scoped daemon state directories, with realm brokers explicitly
  loading the shared `realm-controllers.json` contract.
- Allocator metric bounding now aggregates repeated low-cardinality
  label sets before applying the event cap, preserving counts instead of
  dropping later samples.
- Allocator metadata now emits one namespace-boundary resource
  request per networked realm, avoiding duplicate resource ids for realms
  spanning multiple enabled environments.
- Realm audit, operation, and typed-error envelopes now carry the
  cross-realm correlation id needed to reconstruct rejected routes.
- Operation responses now carry the same required correlation id as
  requests so response-frame return paths can emit correlated audit and trace
  records.
- `realm-controllers.json` validation now accepts host-local materialized
  unit/socket metadata when the artifact declares that systemd units are
  materialized, while still rejecting drift when it claims none are emitted.
- Hardened host-local realm materialization by making realm runtime
  directories non-group-writable, moving default per-realm audit directories out
  of daemon-owned realm state, avoiding global systemd manager environment
  mutation for broker uid/gid discovery, and removing host paths from new
  startup/config tracing fields.
- Host-local realm allowed users who are also `d2b.site.launcherUsers` now keep
  both the canonical `d2b` lifecycle group and their deterministic realm
  socket-access groups.
- Realm Unix socket access-binding DTOs now reject paths longer than the Linux
  `sockaddr_un.sun_path` limit before bind/connect.
- Realm capability-negotiation JSON now rejects unknown outer envelope fields
  while still preserving unknown future capability tokens inside the
  capability set.
- Host-local realm access preflight now derives advertised capabilities from
  enabled provider refs and denies missing required capabilities instead of
  echoing every request as satisfied.
- Aligned realm-core schema generation and identifier validation:
  `xtask gen-schemas` now emits `d2b-realm-core.json`, and realm reference
  tokens reject leading punctuation consistently with their JSON schemas.
- Aligned allocator reference docs with the generated realm-core
  schema roots and documented the future repair-path shape for fail-closed
  allocator reconciliation states.
- `d2b-wayland-proxy --host-terminal` now waits until the proxy-owned wrapper
  toplevel has acked its initial configure before attaching the VM identity rail,
  preventing Wayland compositors from rejecting proxied WeezTerm windows during
  startup.
- Guest VMs now cap persistent systemd journals by default so `/var/log/journal`
  cannot fill small per-VM `/var` images and corrupt NixOS activation state.
- Security-key host relay now drives the physical hidraw fd through
  cancellation-safe `AsyncFd` I/O so guest disconnects cannot leave orphaned
  reader threads racing future sessions.
- Security-key guest frontend now strips the kernel-supplied zero report-ID byte
  from UHID output reports before forwarding CTAPHID frames to the host broker.
- Security-key guest udev rules now match the virtual UHID FIDO device by its
  HID parent identity and grant the standard `plugdev` browser/FIDO access group
  so Firefox and libfido2 can open the guest `/dev/hidraw*` node without root.
- Security-key guest frontend now drives `/dev/uhid` through nonblocking
  `AsyncFd` I/O instead of `tokio::fs::File`, avoiding `ESPIPE` failures on
  character devices during CTAPHID response injection.
- Security-key proxy broker now accepts descriptor-verified FIDO hidraw devices
  even when the host udev group is not one of the fallback FIDO groups. The
  group allowlist remains required only for the descriptor-unreadable fallback
  path.
- Security-key accept loops now run on a daemon-owned runtime thread so the
  per-VM VSOCK socket remains listening after the VM-start readiness transaction
  returns.
- Security-key accept-loop listener initialization now happens inside the
  daemon-owned Tokio runtime context instead of before the runtime is entered.
- Security-key guest frontend now emits the correct UHID_CREATE2 layout
  (`bus` is a 16-bit field), allowing the virtual FIDO HID device to register
  with the guest kernel.
- Security-key host listener sockets now use mode `0770` so inherited per-VM
  ACLs can grant Cloud Hypervisor write/connect access without making the
  listener world-accessible.
- Security-key guest frontend now writes the full UHID_CREATE2 payload,
  including `dev_flags` and padding fields required by current Linux kernels.
- Security-key guest frontend now uses the correct Linux UHID event type
  numbers (`CREATE2 = 11`, `INPUT2 = 12`) so the virtual FIDO HID device is
  actually created rather than sending an input report to no device.
- Security-key guest frontend now zero-extends short UHID events from the kernel
  instead of treating lifecycle events shorter than the maximum union size as
  fatal.
- Security-key host relay now prefixes physical hidraw writes with a zero report
  ID byte, matching Linux hidraw/libfido2 behavior for unnumbered FIDO reports.
- Store-sync activation now creates newly declared per-VM `/run/d2b/<vm>`
  leaves before writing `next-generation`, without recursively creating or
  changing the `/run/d2b` parent posture.


- `d2b-wayland-proxy` now tears down proxy-owned wrapper toplevels when guests
  destroy the XDG role or disconnect abruptly, preventing dead VM identity rails
  from outliving the guest window.
- `d2b-wayland-proxy` now translates pointer focus and motion from wrapper
  content coordinates into guest-surface coordinates while suppressing only the
  trusted rail, preventing clicks in wrapped guest windows from crashing clients.
- `d2b-wayland-proxy` now presents proxy-drawn VM identity rails through a
  proxy-owned wrapper toplevel, so host compositor borders and focus rings wrap
  the VM rail and guest content together without copying guest buffers.
- `d2b-wayland-proxy --host-terminal` now launches child terminals with a
  private runtime directory and relative `WAYLAND_DISPLAY`, while accepting
  ACL-protected user runtime directories, so host terminals connect to the
  intended single-use proxy socket.
- `d2b-wayland-proxy` treats `ENOTCONN` during nonblocking clipboard bridge
  handoff as retryable backpressure, preserving pending FDs until the bridge
  socket finishes connecting.
- Added host-integration coverage for the live `d2b-wayland-proxy`
  AF_UNIX client-to-upstream relay path.
- Updated the Windows notification transitive dependency to remove the runtime
  `quick-xml` advisory path, and documented a temporary build-time
  `wayland-scanner` advisory exception until that code generator publishes a
  fixed `quick-xml` release.
- Default proxy-drawn VM-name labels render in the wrapper rail so the VM
  identity remains visible without overlaying guest buffers.
- `d2b-wayland-proxy` now preserves the last committed surface size after a
  guest destroys the current buffer object, while still clearing decorations on
  committed `attach(NULL)`.
- USBIP driver helper retries now treat transient `ETXTBSY` / "text file busy"
  spawn failures as retryable instead of reporting the helper as missing.
- `d2b-wayland-proxy` now advertises its virtual clipboard manager even when
  the host compositor omits `wl_data_device_manager`, and denies unstable
  text-input v3 forwarding by default to avoid guest app crashes on invalid
  seat-bound requests. Clipboard-boundary allow overrides now remain denied
  and emit stable `W-*` diagnostics.
- Clipboard paste selection offers no longer send drag-and-drop action events
  for normal selections, fixing GTK/Firefox startup through the proxy; `d2b-clipd`
  now requests the exact pending paste MIME from the picker and installs bridge
  sockets with deterministic peer-connect permissions.
- `d2b-clipd` now clears stale host-selection state when it observes its own
  bridge-published VM selection, matches bridge send requests by source id
  instead of MIME alone, and fulfills all queued Wayland paste FDs after one
  picker selection so VM-to-host and VM-to-VM pastes cannot resolve to EOF after
  the picker selects an item.
- Clipboard picker selection now publishes the selected entry as a fresh
  d2b-owned host selection and triggers paste replay, while VM destination paste
  requests open the picker first and serve the replayed transfer immediately
  from the published selection instead of holding a Wayland transfer FD across
  picker interaction.
- Clipboard bridge hardening now binds per-VM bridge sockets under a temporary
  umask before starting background helper threads, uses anonymous memfds for
  virtual-keyboard keymaps, closes bridge streams after partial refresh writes,
  backs off failed nonblocking bridge connects, queues bridge handoffs across
  transient send backpressure, bounds guest-controlled proxy diagnostic label
  cardinality, and prunes stale host-backed virtual offers.
- VM clipboard history now aggregates all MIME variants for the same exact VM
  source under an injective bridge entry id, and the bridge-published selection
  echo guard no longer depends on a time window.
- `d2b-clip-debug` and picker Wayland polling now process readable events
  before hangup/error handling so final Wayland events are not dropped on
  disconnect.
- The flake's `d2b-clipd` package now installs only the daemon binary, keeping
  diagnostic probe binaries out of the daemon package closure.
- `d2b-clipd` now emits an accurate user-visible reason when virtual-keyboard
  paste replay fails, avoids high-cardinality `key=value` fields in routine
  bridge logs, and sends bridge refreshes with fail-closed backpressure handling
  on newly accepted proxy streams.
- Clipboard audit and metric queues now flush metadata-only events instead of
  silently discarding them, and the clipboard user service escapes systemd
  specifiers while rejecting dot-segment bridge roots.
- Clipboard replay now records d2b-owned data-control source ids before
  flushing Wayland events, so VM-origin copies are not misclassified as unknown
  host paste requests, and suppresses source-VM selection echoes so copying
  inside a VM does not open a spurious picker or create empty host entries.
- VM-origin copies now stay inside d2b's bridge/history state until an explicit
  host or VM paste request opens the picker, so copy operations no longer publish
  a host data-control discovery source that applications can probe.
- Clipboard discovery sources now suppress their own compositor selection echoes
  for every focused destination, preventing copy-time host entries and ensuring
  the picker opens only from the later paste request.
- Clipboard paste requests into a VM now direct-serve only the one-shot
  user-selected replay publication, so later VM-destination pastes open the
  picker instead of silently reusing stale published selection state.


- The console drain path is now treated as a long-lived daemon-internal tokio
  task rather than a broker-spawned runner or one-shot readiness probe.
- `d2b console` dispatch: launcher peers can no longer access another user's VM
  console session. The per-session owner UID is now tracked at `Attach` time;
  `ReadOutput`, `WriteStdin`, `Resize`, `Wait`, and `Close` reject non-admin
  peers whose UID does not match the session owner (`AuthzNotAdmin`).
- QEMU console chardev now uses `-chardev socket,id=con0,fd=N` instead of the
  generic `-chardev fd` backend for correct socketpair semantics.
- `d2b console` FSM: bytes preceding the Ctrl-] detach character in a stdin
  chunk are now forwarded to the VM before closing. Stdin buffer increased from
  256 to 4096 bytes.
- `d2b console` prints an operator hint when connected to a qemu-media VM
  noting that the serial console may appear blank until the guest writes to
  `/dev/ttyS0`.
- `d2bd` audio dispatch now calls guestd `AudioSet` RPCs for Cloud Hypervisor
  NixOS VMs instead of statically defaulting to `HostOnly`. `combined_audio_applied`
  returns `HostAndGuest` when both host and guest succeed, `HostOnly` when only
  host applies, `GuestOnly` for ACA sandboxes, and `Unsupported` on full failure.
  qemu-media VMs never call guestd; ACA VMs fail closed when guestd is
  unreachable.
- `wpctl` subprocesses in guestd now drop to `workload_uid` before exec and set
  both `PIPEWIRE_RUNTIME_DIR` and `XDG_RUNTIME_DIR` to `/run/user/<uid>`, so
  WirePlumber locates the correct per-user socket. In d2bd the host PipeWire uid
  is derived from `metadata(pipewire_runtime_dir).uid()` and passed via
  `CommandExt::uid()` without shell string construction.
- `wpctl` subprocess failures in guestd now capture bounded sanitized stderr for
  operator diagnostics; d2bd host-side failures log only static messages so
  PipeWire node identifiers, paths, and volume values do not leak.
- OFD lock unlock now uses `F_OFD_SETLK` (non-blocking release) instead of the
  incorrect `F_OFD_SETLKW` (blocking wait), which is semantically wrong for a lock
  release path.
- Audio lock file opens now use `OpenOptions::create(true).write(true)` instead of
  `custom_flags(libc::O_CREAT)`, which previously required undocumented write
  permission to function correctly.
- `d2b audio` CLI is fully implemented: `status`, `mic on|off`, `speaker on|off`,
  and `off` subcommands send typed `AudioOp` requests to the daemon public socket
  and render results as human text or `--json`. `d2b audio status --json` emits
  `AudioStatusResult` JSON for d2b-wlcontrol consumers.
- Static USBIP declarations now reconcile on strict VM start after a host reboot
  even when volatile `/run/d2b/locks/usbip/*` claim files are gone. The
  daemon replays declared per-VM bind intents through the existing broker policy
  and OFD-lock path, while VM stop cleanup still touches only same-owner
  persisted claims. No-wait/autostarted VMs schedule a bounded background
  reconciliation worker that aborts if the VM runner is no longer supervised.
- `d2b host shutdown-hook --apply` no longer fails with `authz-not-admin`
  on every VM: the new `HostShutdown` role permits `vmStop` from the guarded
  `ExecStop` path (uid=0) while preserving the daemon-restart continuation
  contract (`KillMode=process` + `Restart=on-failure`).
- Wayland-proxy VM start no longer fails with `runner-exited:wayland-proxy`
  after a fresh login when `/run/user/<uid>` is `0700`: the broker now grants
  a traverse ACL on the runtime directory immediately before proxy spawn.
- Daemon startup no longer emits spurious `kernel-module-check: fatal misses`
  for built-in virtio modules on hosts with `=y` kernel config; the
  `D2B_SKIP_KERNEL_MODULE_CHECK` workaround is no longer needed for these
  hosts.
- qemu-media host activation now repairs the `/run` ACL mask for the
  qemu-media runner UID, so a switched host cannot leave
  `/run/d2b/vms/<vm>` with `mask::r-x` and prevent QEMU from creating its QMP
  socket before boot-media auto-enrollment runs.

- Live guest activation timeouts are now configurable globally via
  `d2b.daemon.lifecycle.liveActivation.timeoutSeconds` and per VM via
  `d2b.vms.<vm>.lifecycle.liveActivation.timeoutSeconds`, allowing
  identity-bound guests to wait longer for operator-mediated user-session flows
  such as Entra/Himmelblau hello/PIN.
- `d2bd` now publishes a daemon-side public status read model for unfiltered
  list/status requests, including read-model generation and source-fingerprint
  metadata for wlcontrol and other fast-refresh clients.
- **USB explicit attach** (`d2b usb attach <vm> <present-busid> --apply`):
  `d2b usb attach` now supports attaching any physically-present USB device
  to a USB-capable VM without requiring static busid/vendor allowlists in the
  NixOS bundle configuration. The new explicit path performs three fail-closed
  pre-flight checks before any broker or firewall mutation: sysfs presence
  (`/sys/bus/usb/devices/<busid>/idVendor`), USB-capable gate
  (`RuntimeCapabilityGate::UsbHotplug`), and active claim exclusivity (OFD lock
  under `/run/d2b/locks/usbip/<busid>`). When a static bundle intent exists
  for the busid, the declared path is used (existing behavior preserved). When no
  declared intent is found, the explicit path dispatches two new broker ops —
  `UsbipExplicitBind` and `UsbipExplicitFirewallRule` — which perform per-device
  backend ACL grant (without allowlist validation), scoped nftables carveout
  install preserving all active declared and explicit carveouts, and compensating
  rollback on any failure. New typed errors `UsbipBusidNotPresent` (exit 67) and
  `UsbipExplicitClaimConflict` (exit 67) surface actionable operator guidance for
  pre-flight rejections.
- `UsbipClaimSource` enum in `d2b-contracts` models whether an active daemon
  USB claim originates from a static bundle declaration (`Declared { firewall_ref,
  bind_ref }`) or an explicit present-busid attach (`Explicit`).
- `UsbipDaemonClaimRecord` DTO in `d2b-contracts` captures the in-process
  daemon representation of an active busid claim including VM, env, proxy port,
  source, and the OFD lock path.
- `build_usbip_explicit_plan` in the daemon USB state machine builds a per-busid
  bring-up plan without requiring a bundle resolver or pre-declared intents.
- `UsbipExplicitBind` and `UsbipExplicitFirewallRule` broker wire ops carry raw
  busid, vm, env, and per-env uplink IPs for the per-device backend model;
  validated by the same busid shape validator as the declared path.
- 38 new focused Rust tests: 8 focused unit tests in `usbip_state_machine`, 15
  contract tests in `usbip_explicit_attach_contract` covering explicit plan
  shape, claim source enum, lock path derivation, broker op round-trips,
  deny-unknown-fields, per-device backend model policy, firewall env scope, sysfs
  presence pre-flight, and codebase policy gates; 2 audit roundtrip tests in the
  broker for the new `UsbipExplicitBind` and `UsbipExplicitFirewallRule`
  `OperationFields` variants; 2 JSON-schema contract tests in
  `usb_json_contract`; and 3 network-scoping contract tests in
  `usb_network_scoping`.

- `d2b switch` now threads the configured live activation timeout into
  guest-control and includes identity-flow recovery guidance when guest
  activation times out.
- Successful activation commits now publish split store-view `state/current` and
  `meta/current` pointers in addition to the legacy activation marker, keeping
  daemon StoreSync metadata aligned after live switches.
- Public status/list read-model snapshots now invalidate when runner pidfd state
  changes, preventing cached lifecycle state from surviving VM start/stop
  transitions.
- USBIP bind now uses the same bounded isolated driver helper path as unbind, so
  a slow or stuck kernel driver bind cannot pin the broker control path
  indefinitely.
- `d2b usb detach <vm> <busid> --apply` now reaches the broker
  `UsbipUnbind` cleanup path instead of stopping at a hardcoded ambiguous-flow
  refusal, so stale USBIP host claims can be released and subsequent attaches can
  recover without raw `usbip` commands.
- `d2bd.service` now relies on declarative tmpfiles ACLs for `/run/d2b`
  instead of an imperative root `ExecStartPre`; the tmpfiles rules keep the ACL
  mask writable for the daemon while the `d2b` operator group remains
  narrowed to traversal by the explicit group ACL.
- Host activation now preserves the `/run/d2b` ACL mask as `rwx` when
  reasserting runtime directory posture, preventing switch-time activation from
  clipping the `d2bd` daemon's write access to `r-x`.
- USB probe/status no longer marks a declared USBIP device `degraded` with
  `probe-incomplete` after guest-control confirms that the busid is already
  imported in the guest.

### Removed

- Removed obsolete references to the legacy Wayland proxy from
  documentation and comments.


- Removed the public `d2b usb enroll` CLI and daemon/public-wire verb.
  QEMU-media USB boot-drive remediation now points operators to
  `qemuMedia.source.usbSelector.byIdName` and `d2b usb probe`, while
  running qemu-media VMs still use QMP-backed `d2b usb attach` /
  `detach` hotplug.

- CLI/docs: added the missing `vm display` authorization-matrix row so the
  declared display-session management command is covered by the generated
  privileges contract.
- Tests: narrowed the host realm-relay dependency policy to actual relay/runtime
  crates so the neutral constellation provider trait crate can remain in
  host-side runtime-provider code.
- Documentation: updated the public manifest schema to include the runtime
  operation capability, autostart policy, and service summary fields already
  emitted by the manifest.
- Tests: the per-example flake gate now evaluates scratch copies with the
  `d2b` lock target rewritten to the current `git+file` checkout,
  preserving each example's lock graph and external pins while avoiding mutable
  `path:../..` lock failures.
- Tests: the broker reap-health zombie canary now accepts the transient
  uninterruptible-sleep proc state seen on busy CI runners before child teardown.
- CLI/daemon: qemu-media USB attach/detach `--apply --json` now emits a
  JSON success envelope, and qemu-media list/status service capabilities no
  longer advertise `virtiofsd`.
- Proof tests: the Cloud Hypervisor connect proof now binds fixture
  sockets under a short relative target path so long worktree paths do
  not exceed Unix domain socket pathname limits.
- NixOS: all host-tool selectors, including the activation helper and
  Wayland filter process descriptor, now honor
  `d2b.site.usePrebuiltHostTools = false`. Cross-system flake eval
  and host-integration fixtures use source-built mode so pre-release
  daemon/config schema changes are validated together where release
  prebuilts are unavailable or stale. The qemu-media nix-unit cases now
  keep artifact coverage on aarch64 while treating qemu-media's
  x86-only platform assertion as expected.
- Tests: d2b CLI unit-test sockets now use short paths under the
  system temporary directory and exec mock-daemon tests use real
  manifests, avoiding host/worktree-dependent `ENAMETOOLONG` failures
  and pre-connect mock-server hangs.
- Tests: d2bd public status tests now load temp bundle artifacts
  through the current-user test verification policy with production-like
  `0640` modes, preserving bundle tamper checks while keeping local
  unit tests runnable as an unprivileged user.
- Tests: the privileges-matrix policy test now ignores CLI grouping
  forms such as `realm` instead of treating them as broker/public
  operation ids, and the exec-runner natural-exit test waits for the
  bounded drain thread to publish stream EOF.
- Tests: the source-filter policy now treats `gateway-vm.nix` as a
  consumer of centralized host-tool packages rather than a Rust package
  source builder, while still rejecting ad-hoc source filters there.
- Tests: updated the Cloud Hypervisor runner-shape contract snapshot to
  include the guest-control `d2b-gctl` virtiofs share emitted by the
  current bundle.
- Tests: refreshed the USB attach/detach dry-run goldens to describe the
  guestd import/detach path instead of the retired SSH fallback.
- Tests: the Rust gate now runs the broker default/layer1/fake-backends
  feature passes serially by default so SIGCHLD reaper tests do not
  contend over process-global signal state; `D2B_PARALLEL_BROKER=1`
  remains available for local timing experiments.
- Tests: the stub-no-socket gate now checks for unexpected runtime
  entries instead of live `/run/d2b` directory mtime changes, so
  unrelated daemon activity on a shared host does not fail the gate.
- CI: flake-check shards now retry through `nix-instantiate` when the
  hosted-runner `nix eval` process segfaults while instantiating a
  single check derivation.
- CI: the `eval-with-observability` flake check now validates the
  observability example's assertions, manifest toggle, stack VM, and
  workload opt-in without forcing the full system toplevel derivation,
  avoiding a hosted-runner Nix evaluator crash.
- ADR 0032 ACA display: the daemon-owned verified Relay listener now
  survives the synchronous `gatewayDisplay` request runtime, so Waypipe
  sessions remain connected after `d2b vm exec <aca target>` returns
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
- `qemu-media` restart now cleans up leftover dependency sidecars when
  the primary qemu-media runner has exited, so a stale Wayland proxy
  pidfd from a failed boot no longer blocks the next start.
- `qemu-media` lockMemory now gives QEMU a bounded proportional memlock
  allowance (guest RAM plus the larger of 2 GiB or 25% headroom), avoiding
  runtime `mlockall` failures for larger media VMs without broadening the
  broker service's memlock limit, and fails before spawn when host
  `MemAvailable` cannot satisfy guest RAM plus fixed QEMU overhead.
- `qemu-media` USB detach now reconciles QMP state idempotently when the
  delete event is missed or the media nodes are already absent, and can
  detach a uniquely attached same-vendor/product ref after a runtime USB
  selector moves without requiring manual re-enrollment first.
- Documentation: sanitized ADR 0032's ACA/Relay live-proof record so the
  architectural validation summary remains without publishing live sandbox,
  disk-image, command-output, or compositor-window identifiers.

- Extended generated `sync.json` coverage for daemon lock roots, per-VM
  lifecycle locks, store-view sync locks, and USBIP lock claims without
  changing live lock implementations.
- Added contract-test policy coverage for host-mutable path/lock surfaces,
  requiring storage/sync contract rows, opaque broker IPC inputs, and a single
  repair owner for new mutable host state.
- ADR 0035 Wave 5 added a contract-test policy that classifies every
  `ProcessRole` and requires runner roles to carry Rust argv-builder plus
  runner matrix/contract coverage before new roles land.
- ADR 0035 Wave 4 decomposed CLI read-model/rendering helpers and daemon
  admission helpers into focused Rust modules while preserving output and
  authorization contracts.
- ADR 0035 Wave 3 moved stable CLI JSON output DTOs into the shared IPC contract
  crate, keeping CLI presentation and schema generation on the same strict
  deserialization contract.
- ADR 0035 Wave 2 normalized internal NixOS VM/env indexing for network and host
  consumers, preserving network isolation semantics while making per-env USBIP
  backend ports an explicit generated host contract.
- ADR 0035 Wave 1 consolidated internal NixOS bundle artifact definitions behind
  a typed central model while preserving generated artifact bytes and private
  install metadata.
- ADR 0035 Wave 0 internal cleanup added deterministic inventory tooling and
  `compat-ADR` bridge-key policy coverage, removed caller-free test/Make
  compatibility aliases, and dropped stale retired bash-CLI option comments.
- USBIP architecture notes/tests now pin the per-env proxy as a generic L4
  forwarder, preserving backend/proxy sidecars during single-busid teardown and
  encoding optimistic backend/export refresh, firewall-before-flow-kill
  revocation ordering, TCP-vs-UDP targeted cleanup rules, fail-closed
  revocation when a selected busid stream cannot be isolated, and explicit
  bounded-drain/exclusive rebind requirements before any same-env stream bounce.
- USBIP restart reconciliation now has a daemon-internal physical topology
  identity model that compares allowed VID/PID with sysfs bus/port topology
  instead of trusting serial-like descriptors, while keeping raw topology out of
  public/status projections.
- USB reconciliation now has closed degraded-reason/status primitives with
  redacted public/event projections, bounded telemetry labels, remediation
  mapping, bounded reconcile correlation IDs, dedupe/rate-limit buckets
  partitioned only by closed event type and bounded source projection, strict
  `other` fallback for capped buckets/static metric labels, and suppressed-event
  summaries with exact dropped counts and windows for later observability
  wiring.
- VM start/stop USB reconciliation now threads the same bounded reconcile
  correlation ID through USB broker requests as broker audit trace context
  without adding it to metric labels.
- USB broker audit records now keep a privileged forensics projection for
  USBIP binds with normalized vendor/product IDs, serial-presence only by
  default, HMAC-SHA256 serial correlation backed by broker-owned root-only key
  material, current/previous-key correlation during rotation windows, and a
  scrubbed rotation-window log/audit shape for observability.
- Guest-control now exposes authenticated, side-effect-free USBIP status/list
  observation backed by the configured guest `usbip` path with closed timeout and
  parser error mapping.
- Persistent shell CLI routing now sends gateway-backed `list`, `detach`, and
  `kill` management forms through the configured realm gateway over the typed
  guest-control exec path, while interactive gateway attach fails closed until
  semantic ADR 0039 attach support lands.
- Constellation persistent shell routing: extended remote full-host routing and
  provider trait seams so ADR 0039 `Shell*` operations require
  `persistent-shell`, target workloads explicitly, preserve mutating
  idempotency semantics, round-trip through the protobuf codec, and stay
  separate from provider exec/durable execution.
- Constellation persistent shell runtime alignment: guestd now gates shell
  capability advertisement on the usable exec/workload-user/helper/shpool
  runtime, reports configured shell limits, uses opaque shell ids, exposes
  core DTO adapters for shell summaries/events, and documents the fail-closed
  provider guestd bootstrap contract.
- Constellation persistent shell contracts: promoted ADR 0039's reserved
  `persistent-shell` capability, `Shell*` operation kinds, shell-authorized PTY
  stream kind, and bounded shell DTOs into the generated core schema contract.
- Constellation persistent shell routing: added ADR 0039 and reference stubs
  reserving the provider/remote contract for ADR 0038 shells, including the
  guestd-compatible provider-agent requirement and the rule that
  `executeShellCommand` is not a persistent-shell channel.
- Persistent shell daemon: started d2bd-side shell control-plane routing with
  admin-gated management operations, guest-control capability checks, shell
  response framing, and attached-owner terminal proxying scaffolding.
- Persistent shell runtime: started guestd-side shell session runtime scaffolding
  with staged Nix/PAM/service wiring, in-memory admission/idempotency tests, and
  fail-closed guest-control shell capability handling.
- Persistent shell contracts: added staged default-off shell option, manifest,
  public/guest-control wire, and authz contract scaffolding for later runtime
  wiring.
- Test orchestration: added a central Layer-1 job manifest that drives local
  `make check`, renders the PR workflow, and adds a stable `check` CI rollup so
  branch protection can require one context while generated job names remain
  implementation details.
- Guest-control internals: started extracting the shared terminal substrate used
  by interactive exec, with compatibility DTO conversions and redaction tests for
  future interactive-terminal reuse.
- Test and contract hygiene: synced the existing operation-inspection
  authorization/golden contracts and shortened Unix-socket paths in CLI,
  daemon-access, broker QMP, and broker integration tests so long worktree paths
  do not exceed platform socket limits.
- Developer tooling: added a standalone static guest shell helper workspace,
  libshpool pin, initial validation/management-output scaffolding, and explicit
  Rust/static supply-chain gate wiring for upcoming guest-control terminal work.

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
  semantic API with its current local-Unix binding), `d2b-host-providers`
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
