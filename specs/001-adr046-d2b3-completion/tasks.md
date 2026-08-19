---
description: "Task list for completing the ADR-046 Provider control plane (d2b 3.0)"
---

# Tasks: Complete the ADR-046 Provider Control Plane (d2b 3.0)

**Input**: Design documents from `/specs/001-adr046-d2b3-completion/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/](./contracts/),
[spec-coverage.md](./spec-coverage.md)

## How this task list is organized

This is not a greenfield feature. The task set preserves implementation work for the live
Zone resource plane, Providers, audited privilege, cutover, compatibility, recovery, and
release surfaces. Tasks are grouped by implementation sequence and parallel file-disjoint
areas. Sequence labels describe dependency order.

Ready implementation tasks may run in parallel when their owned files are disjoint. Focused
component tests are required for each changed surface, with host, live, hardware, container,
and performance validation conditional on the surface.

## Format: `[ID] [P?] [Story] WorkItemId - destination label (reuseAction display)`

- **[P]**: Parallelizable with other ready tasks after all of its declared prerequisites
  complete, with no file-overlap edge among the tasks launched together. The marker does not
  remove an incoming dependency or mean immediate readiness. It is an implementation
  scheduling hint only.
- **[Story]**: US1 live resource plane, US2 Providers, US3 cutover, US4 release.
- Text after `WorkItemId` is a **non-authoritative navigation label**, not a writable path
  list or a substitute for the owning contract. Labels use balanced path syntax but may omit
  destinations and descriptive detail.

## Task detail and traceability

Each task points to its owning requirement, contract, implementation area, and validation
obligation. The product specification and the contracts in this directory are authoritative
for behavior; existing code is authoritative where it differs from historical prose. A task is complete when its
implementation and focused validation are complete,
including removal proof where it retires a path.

Cross-provider acceptance tasks T604, T479, and T480 coordinate the Volume, Network, Device,
and guest-runtime checks without owning another component's implementation. Their evidence is
candidate-bound to the affected release state, uses the exact resource identities in `spec.md`,
and is validated by the public test targets required by the changed surfaces.

## Implementation sequencing and validation

Implementation increments follow dependency order and require no retired delivery approval.
Each changed surface requires focused tests; wider Layer-1, container, host, live, hardware,
and performance lanes are conditional. make check is available as an optional broad check and
is not a pre-PR requirement.

Recovery evidence remains a product safety control: validate candidate, commit, tree, preview, host, operator, and restore-instruction bindings before irreversible cutover. The recovery validator fails closed on malformed, stale, expired, mismatched, skipped, or unavailable evidence.

## Cross-cutting implementation prerequisites

The restored planning history identified two implementation prerequisites that remain relevant:

- [X] T007 [US1] Validate RSS corrections (range-seek replay, streaming decode, and shared immutable ChangeBatch fan-out) in proofs/redb-resource-store-spike/.
- [X] T576 [US1] Inventory migration-map DELETE and REPLACE rows that lack removal proof and assign each missing proof to the implementation that retires its path (FR-023).

Historical specification-quality, delivery-tooling, review, and wave-entry tasks are not
current implementation requirements.

## Implementation sequence: Primitive resource composition and Zone routing

**Requirements**: see spec-coverage.md traceability tables | **Story**: US1 | **Work items**: 19 | **Parallel groups**: 2

### Group `wi:ADR-046-primitive-resource-composition` (3 items)

- [X] T009 [P] [US1] `ADR046-primitives-001` - `packages/d2b-contracts/src/v3/host.rs` (adapt)
- [X] T010 [P] [US1] `ADR046-primitives-002` - `packages/d2b-provider-system-systemd/` (adapt)
- [X] T011 [P] [US1] `ADR046-primitives-003` - `packages/d2b-provider-volume-*/` (adapt)

### Group `wi:ADR-046-zone-routing` (16 items)

- [X] T012 [P] [US1] `ADR046-routing-001` - `packages/d2b-contracts/src/v3/zone_routing.rs` (adapt)
- [X] T013 [US1] `ADR046-routing-002` - `packages/d2b-zone-routing/src/engine.rs` (adapt)
- [X] T014 [US1] `ADR046-routing-003` - `packages/d2b-zone-routing/src/resolver.rs` (ZoneEntrypointResolver) (adapt)
- [X] T015 [US1] `ADR046-routing-004` - `packages/d2b-core-controller/src/zone_links.rs` (adapt)
- [X] T016 [US1] `ADR046-routing-005` - `packages/d2b-bus/src/zone_route.rs` (cross-Zone bus routing) (adapt)
- [X] T017 [US1] `ADR046-routing-006` - `packages/d2b-zone-routing/tests/route_engine_vectors.rs` (adapt)
- [X] T018 [P] [US1] `ADR046-routing-007` - `packages/d2b-bus/src/session/` (adapt)
- [X] T019 [US1] `ADR046-routing-008` - `packages/d2b-bus/src/transport/unix.rs` (adapt)
- [X] T020 [US1] `ADR046-routing-009` - `packages/d2b-contracts/src/v3/zone_session.rs` (adapt)
- [X] T021 [US1] `ADR046-routing-010` - `packages/d2b-resource-client/` (adapt)
- [X] T022 [US1] `ADR046-routing-011` - `nixos-modules/options-zones.nix` (new structural base) (adapt)
- [X] T023 [US1] `ADR046-routing-012` - `nixos-modules/zone-resources-json.nix` (new) (adapt)
- [X] T024 [US1] `ADR046-routing-013` - `packages/d2b-core-controller/src/configuration.rs` (defined by ADR-046-core-controllers) (adapt)
- [X] T025 [US1] `ADR046-routing-014` - `packages/d2b-provider/src/` (adapted in place) (adapt)
- [X] T026 [US1] `ADR046-routing-015` - `packages/d2b-provider-toolkit/src/` (adapted in place) (adapt)
- [X] T027 [US1] `ADR046-routing-016` - `packages/d2b-zone-routing/src/service.rs` (adapt)

## Implementation sequence: Provider model and packaging

**Requirements**: see spec-coverage.md traceability tables | **Story**: US1 | **Work items**: 4 | **Parallel groups**: 1

### Group `wi:ADR-046-provider-model-and-packaging` (4 items)

- [X] T031 [P] [US1] `ADR046-provider-001` - `packages/d2b-contracts/src/v3/provider.rs` (adapt)
- [X] T032 [P] [US1] `ADR046-provider-002` - one `packages/d2b-provider-<base>-<implementation>/` per Provider with mandatory src/ (adapt)
- [X] T033 [P] [US1] `ADR046-provider-003` - `packages/d2b-provider-system-core/` (adapt)
- [X] T034 [US1] `ADR046-provider-004` - `packages/d2b-contracts/src/v3/semantic_services/{mod,audio,security_key,telemetry,usb}.rs` (create)

## Implementation sequence: Components/processes/sandbox, core controllers, provider state, network and credential resources

**Requirements**: see spec-coverage.md traceability tables | **Story**: US1 | **Work items**: 31 | **Parallel groups**: 6

### Group `wi:ADR-046-components-processes-and-sandbox` (1 item)


### Group `wi:ADR-046-core-controllers` (1 items)

- [x] T040 [P] [US1] `ADR046-core-001` - `packages/d2b-core-controller/src/{main,configuration,api_catalog,authz,providers,controllers,ownership,watches,cleanup,zone_links,budgets,store}.rs` (adapt)

### Group `wi:ADR-046-provider-state` (12 items)

- [x] T041 [US1] `ADR046-pstate-001` - `packages/d2b-contracts/src/v3/volume_state.rs` (adapt)
- [x] T042 [US1] `ADR046-pstate-002` - `packages/d2b-contracts/src/v3/provider.rs` (component descriptor `stateNamespaces` field) (adapt)
- [x] T043 [US1] `ADR046-pstate-003` - `packages/d2b-provider-volume-local/` (adapt)
- [X] T044 [US1] `ADR046-pstate-004` - `packages/d2b-provider-volume-local/src/migration.rs` (adapt)
- [X] T045 [US1] `ADR046-pstate-005` - `packages/d2b-provider-volume-local/src/sealing.rs` (adapt)
- [X] T046 [US1] `ADR046-pstate-006` - `packages/d2b-provider-volume-local/src/snapshot.rs` (adapt)
- [X] T047 [US1] `ADR046-pstate-007` - `packages/d2b-provider-volume-local/src/relocation.rs` (adapt)
- [x] T048 [US1] `ADR046-pstate-008` - `packages/d2b-provider-volume-local/src/audit.rs` (adapt)
- [x] T049 [US1] `ADR046-pstate-009` - `packages/d2b-provider-volume-local/tests/state.rs` (ported hermetic atomic/lock/quarantine/lease tests) (adapt)
- [x] T050 [US1] `ADR046-pstate-010` - `nixos-modules/zone-resources.nix` (per-Zone bundle emitter NixOS module) (adapt)
- [x] T051 [US1] `ADR046-pstate-011` - `packages/xtask/src/provider_crate_policy.rs` (adapt)
- [X] T052 [US1] `ADR046-pstate-012` - `packages/d2b-core-controller/src/optional_state_admission.rs` (adapt)

### Group `wi:ADR-046-resources-credential` (8 items)

- [x] T053 [US1] `ADR046-credential-001` - `packages/d2b-contracts/src/v3/credential.rs` (adapt)
- [x] T054 [US1] `ADR046-credential-002` - `packages/d2b-contracts/proto/v3/credential.proto` (adapt)
- [x] T055 [US1] `ADR046-credential-003` - `packages/d2b-provider-credential-secret-service/src/{lib.rs, controller.rs, service.rs, main.rs}` (adapt)
- [x] T056 [US1] `ADR046-credential-004` - `packages/d2b-provider-credential-entra/src/{lib.rs, controller.rs, service.rs, main.rs}` (adapt)
- [x] T057 [US1] `ADR046-credential-005` - `packages/d2b-provider-credential-managed-identity/src/{lib.rs, controller.rs, service.rs, main.rs}` (adapt)
- [X] T058 [US1] `ADR046-credential-006` - `packages/d2b-provider-credential-<impl>/src/controller.rs` (adapt)
- [x] T059 [US1] `ADR046-credential-007` - `nixos-modules/options-resources.nix` (adapt)
- [X] T060 [US1] `ADR046-credential-008` - `packages/d2b-provider-credential-<impl>/src/audit.rs` (adapt)

### Group `wi:ADR-046-resources-network` (8 items)

- [x] T061 [P] [US1] `ADR046-network-001` - `packages/d2b-contracts/src/v3/network.rs`: NetworkSpec (adapt)
- [x] T062 [US1] `ADR046-network-002` - `packages/d2b-provider-network-local/src/ifname.rs` (adapt)
- [x] T063 [US1] `ADR046-network-003` - `packages/d2b-provider-network-local/` - artifact catalog integration for net-VM nixos-system artifact resolution (adapt)
- [X] T064 [US1] `ADR046-network-004` - `nixos-modules/resources-network.nix`: Nix resource object emitter for Network ResourceType (adapt)
- [X] T065 [US1] `ADR046-network-005` - `packages/d2b-provider-network-local/src/controller.rs`: async NetworkReconciler (adapt)
- [X] T066 [US1] `ADR046-network-006` - `tests/unit/nix/cases/net-vm-network.nix` (adapted to v3 resource API) (adapt)
- [X] T067 [US1] `ADR046-network-007` - `Provider/device-usbip` owns one relay Process/Endpoint authority per Network and calls the typed UsbipEffectPort for the shared closed `ApplyNftablesProjection` request with closed action enum `Apply/Remove` (adapt)
- [X] T068 [US1] `ADR046-network-009` - `packages/d2b-contracts/src/v3/network.rs` external-attachment sharing schema/status (adapt)

The checked T064-T068 rows record accepted implementation history and the unresolved
Network findings remain in `implementation-debt.md` sections 15.1, 15.3, 16.2,
16.5, 18.2, and 18.3. The active product requirement is the production typed-broker
Network effect, single-consumption external-NIC authority lease with all named denials,
and executable repository-routed mDNS, bridge, east-west, nftables, persistent-TAP,
macvtap, disruptive-update, deletion, status, and raw-identity-exclusion coverage.

The intended retained Network hardening is double opt-in: east-west access requires both the
Network resource and Host/site acknowledgement. Its executable matrix is closed over all four
combinations: Network false/Host false denies, Network false/Host true denies, Network
true/Host false denies, and Network true/Host true allows. Each case must assert both host
bridge-port isolation and net-VM forwarding behavior for the derived bridge and owned net-VM
of the same Network identity.

That target currently conflicts with the existing
`ADR-046-resources-network`, which normatively makes `Network.spec.isolation.allowEastWest`
the sole opt-in and requires no Zone/Host gate. Existing code also lacks the production
adapter from `NetworkEffectPort` to the broker/net-VM path. Sole Network opt-in is therefore a recorded nonconformance, not an alternative implementation.
The Network implementation must use a versioned double-opt-in contract that removes every
current-facing sole opt-in path and covers all four Network/Host cases. A feature-local matrix,
single-opt-in assertion, declaration-only fixture, fake adapter, or stale evidence cannot
resolve the conflict. Focused production tests revalidate the cases on the exact
implementation tree; cross-provider acceptance does not transfer ownership of the Network
implementation.

### Group `wi:core-config-hub:w4` (1 items)

- [x] T069 [US1] `ADR046-network-008` - `packages/d2b-core-controller/src/configuration.rs`: bundle application (create)

## Historical implementation grouping: Production store engine and watch, resource catalog, telemetry, CLI, Nix configuration

**Requirements**: see spec-coverage.md traceability tables | **Story**: US1 | **Work items**: 146 | **Parallel groups**: 12

**US1 scope boundary**: this historical grouping is a partial production-plane checkpoint,
not US1 completion. It pins the later operator acceptance set as exactly
`Volume/acceptance-state`, `Network/acceptance-net`, and `Device/acceptance-tpm` in Zone
`acceptance`, with the exact Provider installs, configs, effects, readiness, and Device
cleanup frozen in `spec.md`.
Support resources cannot substitute. Earlier history remains byte-preserved but its sole-opt-in
Network result is nonconforming. The Network contract must remove stale sole-opt-in paths and retain the production
implementation plus the four-case matrix in the Network implementation tasks. This checkpoint
does not claim the prospective positive operator result. Focused cross-provider tests
coordinate acceptance without taking ownership of Provider implementation.
Guest runtime-effect acceptance remains fail-closed until its runtime Provider implementation
and exact host-integration evidence are complete.

### Group `wi:ADR-046-cli-and-operations` (13 items)

- [ ] T073 [US1] `ADR046-cli-001` - `packages/d2b/src/lib.rs` (adapt)
- [ ] T074 [US1] `ADR046-cli-002` - `packages/d2b/src/guest.rs` (`d2b guest start/stop/restart/list/status`) (adapt)
- [ ] T075 [US1] `ADR046-cli-003` - `packages/d2b/src/exec.rs` (`d2b exec run/attach/wait/status/list/logs/kill`) (adapt)
- [ ] T076 [US1] `ADR046-cli-004` - `packages/d2b/src/shell.rs` (`d2b shell open/attach/list/detach/kill/status`) (adapt)
- [ ] T077 [US1] `ADR046-cli-005` - `packages/d2b/src/provider.rs` (adapt)
- [ ] T078 [US1] `ADR046-cli-006` - `packages/d2b/src/complete.rs` (`d2b complete bash/zsh/fish`) (adapt)
- [ ] T079 [US1] `ADR046-cli-007` - `packages/d2b/src/activation.rs` (`d2b activation build/generations/switch/boot/test/rollback/gc/migrate/keys/trust/rotate-known-host/config`) (adapt)
- [ ] T080 [US1] `ADR046-cli-008` - `packages/d2b/src/host.rs` (all `d2b host` subcommands) (adapt)
- [ ] T081 [US1] `ADR046-cli-009` - `packages/d2b/src/zone.rs` (`d2b zone get/list/status`) (adapt)
- [ ] T082 [US1] `ADR046-cli-010` - `packages/d2b/src/resource.rs` (standard `d2b get/list/watch/create/update-spec/delete/status` top-level verbs) (adapt)
- [ ] T083 [US1] `ADR046-cli-011` - Nix: `nixos-modules/options-zones.nix` (replace)
- [ ] T084 [US1] `ADR046-cli-012` - `packages/d2b/src/endpoint.rs` (`d2b endpoint get/list/watch/status/resolve`) (adapt)
- [ ] T085 [US1] `ADR046-cli-013` - `packages/d2b/src/share.rs` (`d2b export …` and `d2b import …` nouns) (adapt)

### Group `wi:ADR-046-nix-configuration` (35 items)

- [ ] T086 [US1] `ADR046-nix-001` - `nixos-modules/options-zones.nix` (adapt)
- [ ] T087 [US1] `ADR046-nix-002` - `Network` resource fields in `nixos-modules/options-zones-resources.nix` (adapt)
- [ ] T088 [US1] `ADR046-nix-003` - `nixos-modules/options-site.nix` (retained) (adapt)
- [ ] T089 [US1] `ADR046-nix-004` - `nixos-modules/index.nix` (rewritten) (adapt)
- [ ] T090 [US1] `ADR046-nix-005` - `nixos-modules/bundle-zones.nix` (per-Zone bundle derivation) (adapt)
- [ ] T091 [US1] `ADR046-nix-006` - `nixos-modules/resources-zones-processes.nix` (adapt)
- [ ] T092 [US1] `ADR046-nix-007` - `nixos-modules/resources-zones-volumes.nix` (adapt)
- [ ] T093 [US1] `ADR046-nix-008` - Compiler-only `parentZone` map in `nixos-modules/options-zones.nix` (adapt)
- [ ] T094 [US1] `ADR046-nix-009` - Provider/display-wayland and Provider/shell-terminal Process configs in `zones/<z>/resource-bundle.json` (adapt)
- [ ] T095 [US1] `ADR046-nix-010` - User-only `Host` resource in `zones/<z>/resource-bundle.json` (adapt)
- [ ] T096 [P] [US1] `ADR046-nix-011` - historical retained baseline for `nixos-modules/privileges-json.nix` (copy-unchanged)

T096's historical disposition did not implement a handoff operation. Code canon confirms the
operation is absent. Prospective authority was external to this historical block.
- [ ] T097 [US1] `ADR046-nix-012` - `nixos-modules/closures-json.nix` (adapt)
- [ ] T098 [US1] `ADR046-nix-013` - Per-Zone `zones/<z>/resource-bundle.json` (`schemaVersion`) (replace)
- [ ] T099 [US1] `ADR046-nix-014` - `nixos-modules/assertions.nix` (adapt)
- [ ] T100 [US1] `ADR046-nix-015` - Same files (adapt)
- [ ] T101 [US1] `ADR046-nix-016` - Network reconciliation by `Provider/network-local` Process resources (copy-unchanged)
- [ ] T102 [US1] `ADR046-nix-017` - Per-VM store reconciliation by `Provider/volume-virtiofs` EphemeralProcess/Process resources (copy-unchanged)
- [ ] T103 [US1] `ADR046-nix-018` - `Provider/device-tpm` (replace)
- [ ] T104 [US1] `ADR046-nix-019` - `docs/reference/schemas/v3/<ResourceType>.json` for each ResourceType (adapt)
- [ ] T105 [US1] `ADR046-nix-020` - Configuration-publication controller handler in `packages/d2b-core-controller/src/configuration.rs` (create)
- [ ] T106 [US1] `ADR046-nix-021` - `packages/d2b-contract-tests/tests/provider-crate-layout.rs` (create)
- [ ] T107 [US1] `ADR046-nix-022` - `nixos-modules/artifact-catalog.nix` (new emitter) (create)
- [ ] T108 [US1] `ADR046-nix-023` - `packages/d2b-bus/src/session/` (new crate `d2b-bus`) (adapt)
- [ ] T109 [US1] `ADR046-nix-024` - `packages/d2b-bus/src/session/` (same crate as ADR046-nix-023). (adapt)
- [ ] T110 [US1] `ADR046-nix-025` - `packages/d2b-bus/src/session/`. (adapt)
- [ ] T111 [US1] `ADR046-nix-026` - `packages/d2b-bus/src/transport/unix/`. (adapt)
- [ ] T112 [P] [US1] `ADR046-nix-027` - `packages/d2b-contracts/src/v3/component_session.rs`. (adapt)
- [ ] T113 [US1] `ADR046-nix-028` - `packages/d2b-contracts/src/v3/services/`. (adapt)
- [ ] T114 [US1] `ADR046-nix-029` - `packages/d2b-provider/src/` (adapt in place). (adapt)
- [ ] T115 [US1] `ADR046-nix-030` - `packages/d2b-provider-toolkit/src/` (adapt in place). (adapt)
- [ ] T116 [US1] `ADR046-nix-031` - `nixos-modules/resources-sharing.nix` (create)
- [ ] T117 [US1] `ADR046-nix-032` - `packages/d2b-client/src/` (adapt in place). (adapt)
- [ ] T118 [US1] `ADR046-nix-033` - `packages/d2b-bus/src/routing/zone_service.rs`. (adapt)
- [ ] T119 [US1] `ADR046-nix-034` - `packages/d2bd/src/provider_registry.rs` (adapt in place). (adapt)
- [ ] T120 [US1] `ADR046-nix-035` - `packages/d2bd/src/provider_effects.rs` (adapt in place). (adapt)

### Group `wi:ADR-046-resources-device` (7 items)

- [ ] T121 [P] [US1] `ADR046-device-001` - `packages/d2b-contracts/src/v3/device.rs` (adapt)
- [ ] T122 [US1] `ADR046-device-002` - `packages/d2b-provider-device-tpm/src/` (adapt)
- [ ] T123 [US1] `ADR046-device-003` - `packages/d2b-provider-device-usbip/src/` (adapt)
- [ ] T124 [US1] `ADR046-device-004` - `packages/d2b-provider-device-security-key/src/` (adapt)
- [ ] T125 [US1] `ADR046-device-005` - `packages/d2b-provider-device-gpu/src/` (adapt)
- [ ] T126 [US1] `ADR046-device-006` - `nixos-modules/resources-device.nix` (adapt)
- [ ] T127 [US1] `ADR046-device-008` - `packages/xtask/src/main.rs` (`check-provider-layout` subcommand) (adapt)

### Group `wi:ADR-046-resources-host-guest-process-user` (22 items)

- [ ] T128 [P] [US1] `ADR046-exec-001` - `packages/d2b-contracts/src/v3/host.rs` (adapt)
- [ ] T129 [US1] `ADR046-exec-002` - `packages/d2b-contracts/src/v3/process_provider.rs`: LaunchTicket (adapt)
- [ ] T130 [US1] `ADR046-exec-003` - `packages/d2b-provider-system-core/src/host.rs`: HostReconciler (adapt)
- [ ] T131 [US1] `ADR046-exec-004` - `packages/d2b-provider-system-core/src/user.rs`: UserReconciler (adapt)
- [ ] T132 [US1] `ADR046-exec-005` - `packages/d2b-provider-system-core/src/host.rs` (continued) (adapt)
- [ ] T133 [US1] `ADR046-exec-006` - `packages/d2b-provider-system-systemd/src/`: launch.rs (opaque EffectPort requests) (adapt)
- [ ] T134 [US1] `ADR046-exec-007` - `packages/d2b-provider-system-minijail/src/`: sandbox_compiler.rs (adapt)
- [ ] T135 [US1] `ADR046-exec-008` - `packages/d2b-process-conformance/src/`: shared conformance test matrix run against both system-systemd and system-minijail providers (adapt)
- [ ] T136 [US1] `ADR046-exec-009` - `packages/d2b-provider-system-core/src/host.rs` (user-only no-isolation Host) (adapt)
- [ ] T137 [US1] `ADR046-exec-010` - `packages/d2b-provider-system-systemd/src/guest_exec.rs` (guest-domain EphemeralProcess launch via systemd-run inside guest) (adapt)
- [ ] T138 [US1] `ADR046-exec-011` - guest-domain process attachment becomes a ComponentSession named stream to the EphemeralProcess running in the guest (adapt)
- [ ] T139 [US1] `ADR046-exec-012` - `nixos-modules/options-zones.nix`: `d2b.zones.<zone>.resources` option as `types.attrsOf (types.submodule resourceModule)` where each resource module has `type` (required enum) (adapt)
- [ ] T140 [US1] `ADR046-exec-014` - `nixos-modules/zone-bundle.nix`: Zone resource bundle emitter (adapt)
- [ ] T141 [US1] `ADR046-exec-016` - `packages/d2b-bus-session/src/`: all above modules verbatim (adapt)
- [ ] T142 [US1] `ADR046-exec-017` - `packages/d2b-bus-session-unix/src/`: all above modules verbatim (adapt)
- [ ] T143 [US1] `ADR046-exec-018` - `packages/d2b-bus-wire/src/session.rs`: v3 bus protocol constants and wire types (adapt)
- [ ] T144 [US1] `ADR046-exec-019` - `packages/d2b-provider-runtime/src/`: `registry.rs` (adapt)
- [ ] T145 [US1] `ADR046-exec-020` - `packages/d2b-provider-toolkit/src/`: retain all modules verbatim (adapt)
- [ ] T146 [US1] `ADR046-exec-021` - `packages/d2b-bus-contracts/src/generated_v3_services/` (adapt)
- [ ] T147 [US1] `ADR046-exec-022` - `packages/d2b-bus-client/src/`: all above modules (adapt)
- [ ] T148 [US1] `ADR046-exec-023` - `packages/d2b-zone-router/src/`: `router.rs` (adapt)
- [ ] T149 [US1] `ADR046-user-session-001` - `packages/d2b-core-controller/src/user_session_authority.rs` (or a core/user-agent per-session agent Process under `Provider/system-systemd`) (adapt)

### Group `wi:ADR-046-resources-volume` (6 items)

- [ ] T150 [P] [US1] `ADR046-volume-001` - `packages/d2b-contracts/src/v3/volume.rs` (adapt)
- [ ] T151 [US1] `ADR046-volume-002` - `packages/d2b-provider-volume-local/src/` (adapt)
- [ ] T152 [US1] `ADR046-volume-003` - `packages/d2b-provider-volume-virtiofs/src/` (adapt)
- [ ] T153 [US1] `ADR046-volume-004` - `nixos-modules/resources-volume.nix` (adapt)
- [ ] T154 [US1] `ADR046-volume-005` - `packages/d2b-provider-volume-local/src/` (create)
- [ ] T155 [US1] `ADR046-volume-006` - `nixos-modules/resources-volume.nix` (create)

### Group `wi:ADR-046-resources-zone-control` (26 items)

- [ ] T156 [US1] `ADR046-client-001` - `packages/d2b-client/src/` (adapt)
- [ ] T157 [US1] `ADR046-pkg-001` - `packages/d2b-contract-tests/tests/policy_provider_crate_layout.rs` (create)
- [ ] T158 [US1] `ADR046-provider-agent-001` - `packages/d2b-provider/src/agent.rs` (v3 provider agent dispatch) (adapt)
- [ ] T159 [US1] `ADR046-wire-001` - `packages/d2b-contracts/src/v3/{services,state,identity,provider}.rs` (adapt)
- [ ] T160 [US1] `ADR046-zone-control-001` - `packages/d2b-contracts/src/v3/zone.rs` (adapt)
- [ ] T161 [US1] `ADR046-zone-control-002` - `packages/d2b-contracts/src/v3/zone_link.rs` (adapt)
- [ ] T162 [US1] `ADR046-zone-control-003` - `packages/d2b-contracts/src/v3/provider.rs` (adapt)
- [ ] T163 [US1] `ADR046-zone-control-004` - `packages/d2b-contracts/src/v3/role.rs` (adapt)
- [ ] T164 [US1] `ADR046-zone-control-005` - `packages/d2b-contracts/src/v3/role_binding.rs` (adapt)
- [ ] T165 [US1] `ADR046-zone-control-006` - `packages/d2b-resource-api/src/authz.rs` (adapt)
- [ ] T166 [US1] `ADR046-zone-control-007` - `nixos-modules/options-zones.nix` (adapt)
- [ ] T167 [US1] `ADR046-zone-control-008` - `packages/d2b-contracts/src/v3/host.rs` (adapt)
- [ ] T168 [US1] `ADR046-zone-control-009` - `packages/d2b-contracts/src/v3/quota.rs` (create)
- [ ] T169 [US1] `ADR046-zone-control-010` - `packages/d2b-contracts/src/v3/emergency_policy.rs` (create)
- [ ] T170 [US1] `ADR046-zone-control-011` - `packages/d2b-bus/src/{lifecycle,engine,driver,streams,transport,error}.rs` (adapt)
- [ ] T171 [US1] `ADR046-zone-control-012` - `packages/d2b-bus-unix/src/{adapter,socket,pidfd,credit,descriptor,error,systemd}.rs` (adapt)
- [ ] T172 [US1] `ADR046-zone-control-013` - `packages/d2b-contracts/src/v3/component_session.rs` (new v3 namespace in existing contracts crate) (adapt)
- [ ] T173 [US1] `ADR046-zone-control-014` - `nixos-modules/options-zones.nix` (create)
- [ ] T174 [US1] `ADR046-zone-control-015` - `packages/d2b-resource-compiler/src/{main,bundle,schema,validator,digest,sort,secret_lint,generation}.rs` (create)
- [ ] T175 [US1] `ADR046-zone-control-017` - `packages/d2b-provider/src/{registry,rpc}.rs` (adapt)
- [ ] T176 [US1] `ADR046-zone-control-018` - `packages/d2b-core-controller/src/zone_link.rs` (ZoneLink handler) (adapt)
- [ ] T177 [US1] `ADR046-zone-control-019` - `packages/d2b-contracts/src/v3/{resource_export,resource_import}.rs` (adapt)
- [ ] T178 [US1] `ADR046-zone-control-020` - `packages/d2b-core-controller/src/export_import_projection.rs` (local qualified Service projection lifecycle owned by `ResourceImport`) (create)
- [ ] T179 [US1] `ADR046-zone-control-022` - `packages/d2b-core-controller/src/authority.rs` (adapt)
- [ ] T180 [US1] `ADR046-zone-control-023` - `packages/d2b-core-controller/src/{quota,emergency_policy}.rs` (adapt)
- [ ] T181 [US1] `ADR046-zone-control-024` - `packages/d2b-core-controller/src/authority.rs` (Host-global index scope + hardware admission) (adapt)

### Group `wi:ADR-046-telemetry-audit-and-support` (26 items)

The telemetry and audit implementation must remove raw Zone, resource, operation,
correlation, and trace identities from audit and telemetry. Audit records use distinct typed
domain-separated fixed digests. Logs and spans use a typed digest only where correlation is
required; metrics and OTEL resource attributes carry no raw or digested identity. T205 owns
the table-driven redaction/cardinality/no-relabel guard for the complete producer set. The
existing audit and telemetry crates own derivation; no secrets service or new runtime boundary
is added.

- [ ] T182 [P] [US1] `ADR046-audit-001` - `packages/d2b-audit/src/{hash_chain.rs,segment.rs,rate_limit.rs,record_types.rs,sink.rs,export.rs}` (adapt)
- [ ] T183 [US1] `ADR046-audit-002` - `packages/d2b-resource-store-redb/src/audit.rs` (adapt)
- [ ] T184 [US1] `ADR046-audit-003` - `packages/d2b-session/src/audit.rs` (adapt)
- [ ] T185 [US1] `ADR046-audit-004` - `packages/d2b/src/zone_audit.rs` (new `d2b zone audit export` subcommand) (adapt)
- [ ] T186 [US1] `ADR046-doctor-001` - `packages/d2b/src/zone_doctor.rs` (adapt)
- [ ] T187 [US1] `ADR046-doctor-002` - `packages/d2b/src/zone_support_bundle.rs` (adapt)
- [ ] T188 [US1] `ADR046-host-posture-001` - `packages/d2b-provider-system-core/src/{host_reconciler.rs,host_status.rs,host_process_audit.rs}` (adapt)
- [ ] T189 [US1] `ADR046-reuse-001` - `packages/d2b-session/` copied verbatim (adapt)
- [ ] T190 [US1] `ADR046-reuse-002` - `packages/d2b-session-unix/` copied verbatim. (adapt)
- [ ] T191 [US1] `ADR046-reuse-003` - `packages/d2b-client/` copied (adapt)
- [ ] T192 [US1] `ADR046-reuse-004` - `packages/d2b-provider/` and `packages/d2b-provider-toolkit/` copied with v3 session admission and bus routing adaptations. (adapt)
- [ ] T193 [US1] `ADR046-reuse-005` - `packages/d2b-provider-observability-otel/src/agent.rs` adapted (adapt)
- [ ] T194 [US1] `ADR046-reuse-006` - `packages/d2b-bus/src/routing.rs` adapted from `service_v2.rs` (adapt)
- [ ] T195 [US1] `ADR046-reuse-007` - `packages/d2b-bus/src/service_router.rs` and `packages/d2b-core-controller/src/provider_effects.rs`. (adapt)
- [ ] T196 [US1] `ADR046-reuse-008` - `packages/d2b-contract-tests/tests/component_session_v2_vectors.rs` and `tests/noise_vectors.rs` copied verbatim. (adapt)
- [ ] T197 [US1] `ADR046-reuse-009` - `packages/d2b-telemetry/src/session_metrics_sink.rs`. (adapt)
- [ ] T198 [P] [US1] `ADR046-telem-001` - `packages/d2b-telemetry/src/{trace_context.rs,audit_hash.rs,emitter.rs,meter_registry.rs,metric_label_policy.rs,redaction_guard.rs}` (adapt)
- [ ] T199 [US1] `ADR046-telem-002` - `packages/d2b-resource-store-redb/src/metrics.rs` (adapt)
- [ ] T200 [US1] `ADR046-telem-003` - `packages/d2b-resource-api/src/metrics.rs` (adapt)
- [ ] T201 [US1] `ADR046-telem-004` - `packages/d2b-core-controller/src/metrics.rs` (adapt)
- [ ] T202 [US1] `ADR046-telem-005` - `packages/d2b-provider-supervisor/src/metrics.rs` (adapt)
- [ ] T203 [US1] `ADR046-telem-006` - `packages/d2b-provider-observability-otel/src/` (adapt)
- [ ] T204 [US1] `ADR046-telem-007` - `packages/d2b-provider-observability-otel/src/nix/journald.nix` (new Nix fragment) (adapt)
- [ ] T205 [US1] `ADR046-telem-008` - `packages/d2b-contract-tests/tests/policy_telemetry_redaction.rs` (new) (adapt)
- [ ] T206 [P] [US1] `ADR046-telem-009` - `nixos-modules/resources.nix` (adapt)
- [ ] T207 [US1] `ADR046-telem-010` - `nixos-modules/resources-bundle.nix` (build-time validation step 4 in the `resources-bundle` derivation) (adapt)

### Group `wi:core-config-hub:w5` (6 items)

- [ ] T208 [US1] `ADR046-device-007` - `packages/d2b-core-controller/src/configuration.rs` (create)
- [ ] T209 [US1] `ADR046-exec-013` - `packages/d2b-core-controller/src/cleanup.rs`: EphemeralProcess TTL cleanup controller handler (create)
- [ ] T210 [US1] `ADR046-exec-015` - `packages/d2b-core-controller/src/configuration.rs`: `ZoneConfigController` (create)
- [ ] T211 [US1] `ADR046-telem-011` - `packages/d2b-core-controller/src/{configuration.rs, ownership.rs}` (adapt)
- [ ] T212 [US1] `ADR046-zone-control-016` - `packages/d2b-core-controller/src/configuration/{mod,bundle_apply,generation_transition}.rs` (adapt)
- [ ] T213 [US1] `ADR046-zone-control-021` - `packages/d2b-core-controller/src/{coordinator,configuration,zonelink}.rs` (adapt)

### Group `wi:reconciliation-real-backend:w5` (1 items)

- [ ] T214 [US1] `ADR046-reconcile-003` - `packages/d2b-controller-toolkit/benches/reaction.rs` (adapt)

### Group `wi:resource-store-backend:w5` (1 items)

- [ ] T215 [US1] `ADR046-store-004` - `packages/d2b-resource-store-redb/src/lib.rs` (adapt)

### Group `wi:resource-store-integration:w5` (2 items)

- [ ] T216 [US1] `ADR046-store-003` - `packages/d2b-contracts/src/v3/storage.rs` (adapt)
- [ ] T217 [US1] `ADR046-store-005` - `packages/d2b-resource-store-redb/src/backup.rs` (adapt)

### Group `wi:resource-store-watch:w5` (1 items)

- [ ] T218 [US1] `ADR046-store-002` - `packages/d2b-resource-store-redb/src/revision_log.rs` (adapt)

- [X] T577 [US1] **Publish the desktop-companion inventory** as a versioned reference document naming each companion, its exact consumed surface, and its verification status (FR-039, contracts/companion-contracts.md CO-1). Published at W5, not at release, so companions have time to adapt. **Done**: `docs/reference/companion-contracts.md` revision 2 is current; revision 1 landed at `b72b205f`. All four inventory rows read "Pending live-host verification", and `weezterm` is excluded by a recorded negative surface-consumption determination, so publication claims no compatibility
- [X] T578 [US1] **Publish the replacement contracts the companions consume**, early enough for them to adapt given that no preview release may be published (contracts/companion-contracts.md CO-2, FR-045). **Done**: `docs/reference/zone-cli-contract.md` revision 1, landed at `b72b205f`. CO-5 remains the W5 exit condition: every "surface consumed" cell in the inventory must resolve to a committed contract at a public ref
- [X] T579 [US1] **Resolve the FR-039 / FR-045 tension before these contracts publish** (CHK025). FR-039 blocks release on external repositories while FR-045 forbids the preview build they would adapt against. **Done, out of order**: T577 and T578 published first, so the resolution was encoded in shipped prose before any requirement said it. Closed by **FR-061** (contract/artifact boundary, publish-adapt-verify sequencing, per-stage refusals) and **FR-062** (the adaptation assumption recorded as unvalidated with a mitigation, a detection point, and an escalation path). FR-045 remains unchanged. See `checklists/coverage.md`, "The W5 date-bound gate"

<!-- RETIRED-W5-MANIFEST-END -->

### Historical production resource-plane task record

Every unchecked task in this historical subsection is retained as planning
evidence only. Do not dispatch it as recovery, do not change its checkbox to reconstruct the
merged history, and do not treat it as a current release prerequisite.

<!-- RETIRED-W5-PLAN-BEGIN -->

- [ ] T589 [US1] **Publish the shared Resource API, store, controller, and bus contracts.** Implement authenticated registrar admission, policy bootstrap, audit journal/status, required-Zone `(Zone, operation_id)` `InspectOperation`, UUIDv7 issuance/expiry, and the exact session/bus protocol. The same operation ID is permitted independently in different Zones and no host-global operation index is created. **Done when** focused Resource API, store, controller, bus, schema, fixture, public wire/API contract, and capability-boundary tests pass, including authorization refusals, replay binding, pending-audit recovery, and Zone isolation. The Version 2 contract, schemas, fixtures, and focused tests are kept consistent directly from their owning product contracts.
- [ ] T590 [P] [US1] **Install and recover the single-owner Zone resource policy without a bootstrap cycle.** Depends on T589. Owned files: `packages/d2b-resource-api/src/authz.rs`, `packages/d2b-core-controller/src/rbac.rs`, and new focused tests under `packages/d2b-resource-api/tests/production_policy.rs`. `ZoneResourceRuntime` owns each `PolicyBootstrapRead` and requests installation, but `d2b-resource-api` alone parses and compiles policy into the immutable `PolicySet` interpreted by `NativeAuthorizer`. For initial install and restart, consume the one-shot capability to read only this Zone's policy-input envelopes at the exact durable nonzero `policy_revision`; it has no public subject, general read/mutation operation, clone, copy, default, public construction, conversion, trait-based mint, reconstruction, or reuse path. A failed installation attempt consumes the capability. After installation, perform every normal policy read/update through an authenticated Resource API session. Authorize T589's `InspectOperation` only for the registrar-derived subject and explicitly selected Zone. A wrong subject or replay-binding mismatch within that Zone, or an ID absent from that Zone, returns the same non-observing result as unknown and never exposes another Zone's operation; if the same ID independently exists in the selected Zone, that Zone's record is returned. On revision advance, compile the exact committed revision before atomic replacement, invalidate cached allows, and report ready only when installed revision and Zone UID equal live durable metadata. Refuse revision zero, stale/missing/cross-Zone/invalid policy, a caller claim, reusable bootstrap access, and any fallback to a constant or partial set. **Done when** focused tests cover first install, authenticated revision advance, restart recovery of the advanced revision, failed-attempt consumption, capability non-reuse, same-subject/Zone operation inspection, wrong-subject indistinguishability, and same-ID independent records in two Zones; defining-crate compiler ambiguity assertions and external compile-fail fixtures prove construction, field access, `Default`, `Clone`/`Copy`, `From`/`TryFrom`, conversion, and capability reconstruction are impossible; `make test-rust` runs the Rust and compile-fail/doctest companions; and every failure leaves only the affected Zone unpublished, degraded, and denied.
- [ ] T591 [P] [US1] **Restore the D106 store boundary and make it exhaustive.** Depends on T589. Owned files: `packages/d2b-resource-store-redb/src/transaction.rs`, `packages/d2b-resource-store/tests/d106_policy.rs`, and `packages/d2b-contract-tests/tests/policy_resource_mutation_seal.rs`. Preserve T589's frozen policy-neutral transactional audit hook. Remove redb deserialization or ownership of `RoleSpec`, `RoleBindingSpec`, `PolicySet`, and all other RBAC DTOs. Move policy-shape interpretation to the Resource API policy owner while retaining policy-neutral canonical-envelope, installed-schema, structural, atomicity, revision, and seal checks in the store. Expand the guard from three hand-picked source files to the full store/redb crate source and dependency graph. The scan MUST enumerate a nonempty source set independently for each store crate and a nonempty resolved dependency set; an empty, missing, or filtered-away input is a failure. Add a hermetic poison fixture that injects both a forbidden RBAC DTO use and a forbidden Resource API dependency and proves the existing test-policy/fixture-contract path rejects them. **Done when** the policy test proves neither store crate depends on the Resource API or contains/imports/deserializes an RBAC policy DTO, the poison negative fails for the intended D106 reasons through existing `make test-policy` and fixture-contract gates, the native evaluator remains the only allow issuer, and authorized Role/RoleBinding mutations still pass through the sealed generic envelope path.
- [ ] T592 [US1] **Complete durable store identity recovery, authoritative audit, and target-broker handoff adoption.** Depends on T591 and is the serialized writer of `packages/d2b-resource-store-redb/src/transaction.rs`. Owned source scopes are `packages/d2b-resource-store-redb/src/{lib.rs,actor.rs,audit.rs,migration.rs,backup.rs,tests.rs,transaction.rs}`, `packages/d2b-audit/src/{lib.rs,sink.rs,record_types.rs,segment.rs,export.rs}`, `packages/d2b-contracts/src/{lib.rs,broker_wire.rs}`, `packages/d2b-core/src/{privileges.rs,privileges_w3.rs}`, `packages/d2b-priv-broker/src/{bootstrap.rs,lib.rs,main.rs,protocol.rs,runtime.rs,live_handlers.rs,sys.rs,audit.rs,ops/mod.rs,ops/audit_op.rs}`, `packages/d2bd/src/{lib.rs,daemon_version.rs}`, `nixos-modules/{options-zones.nix,resources.nix,resources-bundle.nix,privileges-json.nix}`, `tests/unit/nix/cases/zone-audit.nix`, its generated schema/catalogue outputs and focused policy/compatibility tests, both Cargo lockfiles, and the accepted resource-store/audit normative specs. T592 owns the physical store migration, immutable mutation journal and separate export state, fd-anchored broker export/prune path, typed durable `(Zone, operation_id)` `InspectOperation` backend, UUIDv7 issuance/expiry and per-Zone durable retention clock, protocol-5 target adoption, accepted-socket peer-pidfd broker op and the sole approved FFI quarantine, exact source-to-target coordinator transfer, target catalogues/snapshots, and generated outputs. It creates no host-global operation-ID index. It consumes the installed source generation read-only and never creates a source compatibility actor, unit, override, or target-only substitute.

  SC-002 and source-floor detail come from the owning contracts, schemas, fixtures, and
  focused tests. Keep those artifacts consistent and fail closed on missing, stale,
  wrong-owner, non-ancestor, or invalid source inputs before mutation.

  **Done when** store migration/rollback and crash recovery preserve advanced identities; every committed privileged mutation has one immutable transactional journal row and durable export state; required-Zone same-ID inspection is replay-bound; cross-subject/request variants are non-observing; concurrent same-ID operations in two Zones each apply exactly once; response loss and restart return the selected Zone's original result; malformed/future/expired/clock-discontinuous IDs and post-prune reuse refuse before mutation; raw identifier, trace, path, and peer-identity canaries remain absent; `zone-audit.nix` pins option placement, every default and bound, and missing/unknown/out-of-range failures; protocol/catalogue/schema/snapshot/privilege parity and both lockfiles move atomically; accepted-socket fd transfer is close-on-exec and leak-free on every error; target/apply/GC-root and live apply-peer substitutions refuse before mutation; source-to-target coordinator ownership transfers exactly once through the three existing units; every T592 generated Version 2 row has its assigned implementation and enforcing test; and `make test-rust`, `make test-policy`, enabled `make test-fixture-contracts`, `make test-drift`, and the owned Nix tests pass without skip.
- [ ] T593 [US1] **Publish the authenticated Resource API and watch route.** Depends on T592. Owned files: `packages/d2b-bus/src/{router.rs,registry.rs,authorization.rs,operations.rs,streams.rs,session_seam_tests.rs,transport/unix.rs}`, `packages/d2b-resource-api/src/{adapter.rs,watch.rs}`, `packages/d2b-contracts/src/v3/services.rs`, `packages/d2b-session-unix/src/{lib.rs,adapter.rs,descriptor.rs,pidfd.rs,socket.rs,error.rs,subject.rs,zone_admission.rs}`, `packages/d2b-session-unix/tests/{subject_mapping.rs,unix_session.rs}` plus new compile-fail fixtures in that directory, `packages/d2b-bus/tests/{production_resource_route.rs,public_mint_surface.rs}`, and the existing ComponentSession contract reference (`docs/specs/ADR-046-componentsession-and-bus.md`). T593 may not create or edit a project-authored FFI crate, `packages/d2b-priv-broker/src/sys.rs`, a Cargo manifest, or a lockfile; the broker wire, FFI, and dependency boundary remain fixed by the owning contracts. Replace the unregistered production seam with a route whose registration consumes the authenticated ComponentSession admission. At Unix accept, transfer the accepted socket to the typed `OpenPeerPidfdFromAcceptedSocket` broker operation with `SCM_RIGHTS` and consume its returned `OwnedFd` pidfd; `pidfd_open(SO_PEERCRED.pid)` is forbidden. Use receive helpers that set `MSG_CMSG_CLOEXEC`, reject truncated control data, require exactly one expected fd, and close all excess or error-path fds. No request or session type carries a raw descriptor integer, credential tuple, or numeric PID. The session adapter, descriptor, bus Unix transport, and session seam must all consume the same accepted-socket evidence object; none may reacquire credentials, accept a caller-supplied verifier, or construct evidence from a credential tuple or numeric PID. Treat `SO_PEERPIDFD` support as part of the kernel floor and fail closed with an actionable unsupported-kernel error when the broker returns that typed refusal. Require `FD_CLOEXEC`, verify the `SO_PEERCRED` tuple, expected process generation/start identity, expected cgroup, and liveness against that exact fd, and consume all evidence into one private registrar issuer. Reject a dead fd, credential/generation/cgroup mismatch, ambiguous evidence, or any numeric-PID-only path. Remove the public `ZoneBootstrapIdentity::verify` path, its `Clone` implementation and identity/evidence accessors, the `VerifiedUnixPeer::credentials` accessor, caller-supplied verifier and credential constructors, and every direct or transitive re-export that permits external issuance; neither type may expose construction fields, `Clone`, `Copy`, `Default`, conversions, raw credentials, pidfd, generation, or cgroup evidence. `ZoneRegistrar` exclusively derives and propagates the subject from its private mapping; requests and stream frames carry no subject claim. Register exact-Zone ResourceService and controller routes; add required-Zone `InspectOperation` to the closed service/method catalogue, authorization map, and router without a selector-free or host-global route; admit watch replay/live delivery through ZoneBus; and expose one registration/readiness observation from actual owned handles. Pin accepted-socket transfer to the typed broker operation, private registrar issuance, consumed evidence, and the sealed public surface. **Done when** same-Zone authenticated Get/List/Watch/InspectOperation reaches the real service; cross-Zone, self-named, unregistered, reused-admission, direct-WatchService, missing/extra/truncated/malformed ancillary fd, post-receive decode failure, numeric-PID-only, post-credential PID reuse, dead-pidfd, credential/generation/cgroup mismatch, unsupported `SO_PEERPIDFD`, and ambiguity paths are denied with stable descriptor counts and no exec leak; existing adapter, descriptor, Unix transport, subject-mapping, Unix-session, and session-seam tests use accepted-socket evidence and reject all caller-supplied verifier/credential paths; external compile-fail fixtures and `packages/d2b-bus/tests/public_mint_surface.rs` prove no public verifier, constructor, clone, credential/evidence accessor, conversion, re-export, or alternate issuer survives; source policy proves `d2b-session-unix` retains workspace `unsafe_code = "forbid"` with no local syscall/raw-fd fallback or project-authored FFI dependency; and neither `UnregisteredBusAdapter` nor a fixed endpoint can satisfy production publication.
- [ ] T594 [P] [US1] **Bind controller fan-in, effects, and cleanup to the durable replay/adoption ledger.** Depends on T589. Owned files: `packages/d2b-core-controller/src/{runtime.rs,resource_store.rs,provider_effects.rs,cleanup.rs,watches.rs,controllers.rs}`, `packages/d2b-controller-toolkit/src/{context.rs,runner.rs}`, and their existing focused unit tests. Register the production endpoint, consume admitted watch frames into the bounded fan-in, and record every post-commit effect intent before `EffectPort`. Bind each ledger entry to Zone, controller generation, resource UID, committed revision, operation id, and effect ordinal; reuse that key for idempotent dispatch/adoption. On restart relist and adopt/replay pending entries before cleanup. Complete cleanup only by compare-and-set on the same UID and exact nonzero expected revision. **Done when** unit crash-window tests prove no effect before commit or ledger durability, replay/adoption after every later crash point, no lost cleanup intent, and denial of stale, zero, wrong-UID, wrong-generation, or ambiguous completion.
- [ ] T605 [US1] **Correct and pin the system-core Zone handler contract.** Depends on T593. Sole writable ownership: `packages/d2b-contracts/src/v3/zone.rs`; the existing lowest-layer guard `packages/d2b-contract-tests/tests/policy_contracts.rs`; existing governing contract references `docs/specs/providers/ADR-046-provider-system-core.md` and `docs/specs/ADR-046-resources-zone-control.md`; and paired reference page `docs/reference/resource-plane-runtime.md` (adapt). Add `ZoneHandlerName::SystemCoreHost` and `ZoneHandlerName::SystemCoreUser`; the one exact serialized spelling is `system-core-host` and `system-core-user`, matching the committed kebab-case rule. State explicitly that internal/telemetry `handler` labels remain `system_core_host` and `system_core_user` while those underscore values are forbidden in serialized `Zone.status.handlers[]`. The Zone status-handler contract MUST accept exactly one record with each serialized name, phase, and `lastReconciledAt`, reject duplicate or missing records, underscore/wrong-name substitution, and preserve `ZoneHandlerName::ProviderLifecycle` as a distinct allowed value that cannot substitute for either. Treat `packages/xtask/src/zone_schema.rs`, `docs/reference/schemas/v3/core.d2bus.org_Zone.schema.json`, downstream T595/T599 consumers, as read-only inputs: because `ZoneSpec` is unchanged, generator execution MUST leave the committed desired-state schema byte-identical. **Done when** focused `d2b-contracts` tests prove both exact wire round-trips, underscore rejection, exactly-one-each list acceptance, duplicate/missing/wrong-name rejection, and `ProviderLifecycle` preservation/non-substitution; the existing governing contract references, targeted guard, and paired reference page pin the same pre-consumer distinction plus T593's removal of public peer/bootstrap issuance and evidence access; the Zone desired schema is byte-identical before and after its existing generator; and the targeted contract tests pass. T605 validates the contract independently of later consumers; T595 owns the emitter, T599 owns later consumer reconciliation, and focused generated-output and contract checks remain clean.
- [ ] T595 [US1] **Compose the production Zone runtime and host-generation path.** Depends on T590, T592, T594, and T605. Sole owned files are `packages/d2bd/src/{resource_runtime.rs,lib.rs}`, `packages/d2bd/Cargo.toml`, `packages/d2b/src/{lib.rs,dispatch.rs,host_generation.rs}`, `packages/d2b/Cargo.toml`, `nixos-modules/{bundle-zones.nix,host-daemon.nix,host-broker.nix,options-site.nix}`, `flake.nix`, `examples/{minimal,graphics-workstation,multi-env,with-entra-id,with-observability}/{configuration.nix,flake.nix}`, `templates/default/{configuration.nix,flake.nix}`, `tests/unit/nix/cases/host-generation-rebuild-ref.nix`, `tests/host-integration/host-generation-handoff.nix`, the accepted Nix configuration spec, and focused tests in those owners. T595 writes `packages/d2b/src/dispatch.rs` before dependent T599 takes its later serialized ownership; they never write it concurrently. Compose T590-T594 into one daemon-owned per-Zone runtime: ingest each installed resource bundle automatically, install policy, register authenticated ResourceService/watch/controller routes, recover effects and audit, expose required-Zone durable operation inspection, and publish readiness only from live owned handles including T605's exact system-core handler names. Startup and shutdown visit every Zone and isolate failures.

  The deployment entrypoint remains unprivileged, validates one bounded flake/configuration target, builds and stages immutable bytes, obtains public-socket lifecycle authorization, and submits one opaque intent through `d2b host-generation` subcommands. The target closure and installed generation both expose only `bin/d2b`; no `d2b-host-generation-deploy` executable, alias, wrapper, package output, or completion entry exists. Privileged mutation is performed only by the installed source broker before transfer and T592's target broker after transfer, under the broker-owned durable coordinator and existing broker service. The caller cannot select an intent, privileged executable, command, path, or authority token. Target/apply/GC-root and live peer identity are revalidated before each mutation; all ambiguity, substitution, restart, and response-loss paths fail closed or resume the same intent. No new unit or daemon-owned rollback path is permitted.

  T595 consumes the source-floor contracts, digests, fixtures, poison cases, transition identifiers, and counts defined by the owning product artifacts. Missing or invalid inputs block implementation; no runtime-derived substitute is accepted.

  **Done when** startup and deployment switches automatically ingest add/change/remove bundles with no duplicate logical effect; readiness, required-Zone operation inspection, audit, restart adoption, and per-Zone failure isolation use the production route; packaging/help/completion tests prove `d2b` is the sole public binary and all host-generation operations are its subcommands; every flake, example, and template fixture owned above supplies an explicit valid `hostGenerationRebuildRef`, while the focused Nix case proves no default, exact 2048-byte acceptance, 2049-byte refusal, grammar bounds, and missing-reference evaluation; the parameterized migration and rollback VM test exercises the existing broker service and exact ownership transfer with no skip; source/target/apply/GC-root/peer substitutions and unauthorized caller classes mutate nothing; raw Nix stderr and private identities never escape; bundle/schema/reference/changelog outputs stay synchronized; and the owned focused Rust, Nix, fixture-contract, drift, and conditional host-integration checks pass.
- [ ] T596 [P] [US1] **Add authenticated publication, watch, readiness, and Zone-isolation acceptance coverage.** Depends on T595. Sole owned file: new `packages/d2bd/tests/resource_plane_authenticated.rs`. Enter through the production daemon Unix session boundary, registrar, ZoneBus route, ResourceService, store, and controller endpoint. Consume T605's contract evidence and cover authoritative same-Zone Get/List/Watch, cross-Zone denial and audit, caller-supplied subject rejection, consumed-admission reuse, partial-readiness non-publication, exact `Provider/system-core` registration ownership, and an actual `Zone.status.handlers[]` list containing exactly one `system-core-host` and one `system-core-user` record with `phase` and `lastReconciledAt`, backed by active, initialized, current handlers. Prove ComponentSession admission is bound to the accepted peer's live pidfd and expected generation/cgroup evidence; after daemon restart require a newly opened pidfd for the rediscovered peer. Reject numeric-PID-only admission, stale evidence after numeric PID reuse, start-time/generation/cgroup mismatch, dead peer/`ESRCH`, and multiple plausible peers. Reject duplicate, missing, underscore/wrong-name required records and `provider-lifecycle` substitution. Run the three-Zone open/close matrix with failures in the first and middle positions; remove the Provider registration and each required list record in turn and prove only that Zone degrades. No Wave 6 dossier is required. The test must assert every Zone was visited and later healthy Zones remain operable. Direct service calls, `ProductionWatchHarness`, fake endpoints, status-only Provider substitutes, and readiness mutation helpers are forbidden in this file. **Done when** all cases pass against production owners, fresh-pidfd and every PID-reuse/mismatch/`ESRCH`/ambiguity negative pass, the emitted list shape matches T605, and removing or corrupting any required readiness owner makes the affected Zone return its specific actionable refusal.
- [ ] T597 [P] [US1] **Add restart effect-replay and cleanup-revision acceptance coverage.** Depends on T595. Sole owned files: new `packages/d2bd/tests/resource_plane_restart.rs` and new `packages/d2b-core-controller/tests/effect_replay.rs`. Crash after generation commit, after ledger durability, after effect dispatch, after adoption, and before completion; reopen through the broker-owned store path and assert each outstanding effect is replayed or adopted exactly once. Exercise pending cleanup across restart and reject zero, stale, wrong-UID, wrong-controller-generation, and ambiguous completion without changing durable state. **Done when** the matrix observes zero lost intents, zero duplicate logical effects, and adopt-before-cleanup ordering in every case.
- [ ] T598 [P] [US1] **Add authoritative audit, pending-result, replay-binding, retention, and redaction acceptance coverage.** Depends on T595. Sole owned file: new `packages/d2bd/tests/resource_plane_audit.rs`. Mutate through the authenticated production Resource API, including a multi-mutation batch; crash at every mutation/journal commit, segment append, file sync, directory sync, export-completion, rotation, journal-prune, segment-prune, operation-expiry, and operation-prune boundary; reopen; and compare immutable authoritative journal rows with exported logical records by fixed operation digest plus mutation ordinal. Include sink unavailable, disabled callback, incomplete export, hash-chain mismatch, duplicate replay, record oversize, invalid/default/boundary audit configuration, post-export journal retention, early-journal-prune refusal, and prune/sync-failure typed-health negatives. Prove the journal row commits transactionally with the privileged mutation before any effect is success-shaped; segment export and its completion cursor are separate and cannot rewrite or delete an unexported row; an exported row becomes deletion-eligible only after durable completion plus `audit.retentionDays`. After committed export-pending state, require `CommittedPendingAudit` through T589's `PendingAuditStatus` protobuf field, including `DeleteResponse` and batch ordinals, with the exact canonical `ResourceStatus` composite and no ordinary success or rollback claim. Inspect the same operation through T589's typed ResourceService method and T592 durable backend before and after restart only with a required Zone and exact replay-binding match to the original registrar-derived subject, canonical semantic request, target, verb, expected revision, and idempotency data in that Zone. Prove cross-subject and altered-request/target/verb/revision/idempotency/restart mismatches deny without observation or reapplication; concurrently submit the same opaque ID in two Zones and prove both independent operations commit once. Exercise commit-then-response-loss, restart, UUIDv7 malformed/future/expired/overflow cases, retention-clock rollback, prune at `expiresAt`, and reuse after prune; every old-ID mutation or inspection refuses before mutation. Retry a different ID and prove normal revision/conflict behavior. Inject distinct raw operation, correlation, subject, Zone, resource, and trace canaries; require only typed domain-separated fixed digests in journal rows, audit segments, and exports, no digest-class relabel, no identity in metrics or OTEL resource attributes, and no raw canary in errors, logs, metrics, spans, or redacted `Debug`. **Done when** every committed privileged mutation has an immutable authoritative row at commit, ordinary success waits for segment file and directory durability plus completion durability, multi-mutation restart yields exactly one export per ordinal, same-Zone same-ID apply count is one, cross-Zone same-ID apply count is one per Zone, all replay-binding and expiry cases deny as specified, the exact composite round-trips through every mutation response and required-Zone `InspectOperation`, all raw canaries remain absent, fixed-digest constructor and record-size limits hold, configured segment/journal retention and the fixed 30-day operation retention prune correctly without ID reuse, every prune/sync failure degrades health, status observability is stable across restart until expiry, and every audit/export failure leaves the affected Zone unpublished with an actionable typed refusal.
  The redaction matrix must enter through every migrated producer named by T592 and through
  T592's broker drain request. Its raw-canary error assertions cover backend and internal
  error contexts; the bounded direct operator-response exception is owned by T599 and is not
  an audit, telemetry, log, span, metric, or `Debug` exception. It covers valid-present,
  absent, and malformed trace context:
  present yields only the typed trace digest, absent stays absent, and malformed refuses
  before mutation, with no fabrication or cross-class relabel. Distinct root-path and opaque
  storage-handle canaries must also remain absent from fixed `Debug` output for every
  sensitive DTO, error, `SegmentWriter`, sink, exporter, directory owner, and broker owner.
- [ ] T599 [P] [US1] **Reconcile CLI and reference promises with emitted behavior (FR-019, FR-074).** Depends on T595. Sole owned files: `packages/d2b/src/{dispatch.rs,resource.rs,context.rs}`, `packages/d2b-contracts/src/cli_output.rs`, existing CLI contract reference `docs/specs/ADR-046-cli-and-operations.md`, `docs/reference/{zone-cli-contract.md,desktop-wrapper.md,companion-contracts.md,cli-contract.md,components-audio.md,components-usbip.md,components-usb-security-key.md,resource-client.md}`, `packages/d2b-contract-tests/tests/{policy_cli_consumers.rs,policy_docs.rs}`, focused CLI DTO/schema tests in the owning crates, and task-local `changelog.d/cli-operation-recovery.md` for the owning release task to fold. Its `dispatch.rs` ownership is the explicit later serial handoff from T595; T599 preserves and reconciles T595's frozen `d2b host-generation` namespace. Implement the recovery contract in `contracts/operator-cli.md` only through T589's typed store/ResourceService request and response, T590 authorization, T593 method catalogue/router, and T595 daemon/client path; an in-memory map or CLI-only synthesized result is forbidden. Every mutating generic and typed resource verb accepts `--operation-id <OPAQUE_ID>`; the ID is exactly 16 UUIDv7-layout bytes rendered as lowercase 32-hex without separators; an initial call emits it; an exact same-Zone retry reuses the original operation/idempotency binding; and `d2b --zone <ZONE> op inspect --operation-id <OPAQUE_ID> [--watch]` remains the accepted required-Zone status command rather than creating a competing command or host-global lookup. The same opaque ID is permitted independently in different Zones. Own the versioned operation-recovery DTO in `cli_output.rs` and its generated `JsonSchema` checks. Implement the recovery contract's pending exit 75 and replay-mismatch exit 76 for resource mutations/inspection, retain the existing meanings for unrelated exec commands, require `zoneRef` and `schemaVersion: 2` in every recovery success/error JSON envelope, add the exact closed remediation-action enum, and update the stable error-class and exit tables. Migration guidance must tell Version 1 consumers to require `schemaVersion`, upgrade parsing before using recovery, treat a missing or `1` version as the old 0/1/2 contract, and never reinterpret or silently migrate an arbitrary Version 1 operation ID; the v3 clean cutover has no persisted Version 1 recovery-state import. Human and JSON remediation may contain only a closed action such as `inspect-operation`, `retry-identical-operation`, `start-new-operation`, `wait-for-audit-export`, or `verify-operation-context`; it must never embed Zone or operation IDs in executable text, argv arrays, shell fragments, or free-form remediation. Raw Zone and operation ID appear only in their bounded `zoneRef` and `operationId` status fields. Pin mutation and inspection exits plus exact human/JSON pending/final/not-found/expired/refusal shapes, mandatory envelope fields, DTO schema, UUIDv7 issuance/expiry bounds, required Zone, cross-Zone same-ID independence, action enum, and absence of executable remediation vectors. Compare exact `d2b --help`, subcommand help, JSON output, capability keys, typed refusals, public wire fields, binary outputs, and completions. `d2b` remains the sole public binary: inspect, repair, restoration, authorization, and apply are all `d2b host-generation` subcommands, and no standalone executable, wrapper, alias, or migration fallback is emitted. Resource status documentation must expose committed-pending-audit through T589's additive protobuf status field and the exact `ResourceStatus.phase`, `outcome.code`, `update.state`, and `update.operation_id` composite; never claim success or rollback. Reconcile every downstream status consumer owned by this task with T605's paired contract and T595's emitted `Zone.status.handlers[]`: system-core readiness is attributed to `Provider/system-core` plus exactly one `system-core-host` and one `system-core-user`; underscore labels and `provider-lifecycle` cannot substitute. Candidate absence of a command or field is a defect, not permission to delete its promise, unless the same change follows the explicit parity or FR-042 retirement path with replacement, migration guidance, owner, release treatment, and contract tests. Do not add a fallback or claim companion verification. **Done when** every documented desktop-wrapper, companion, audio, USB, security-key, host-generation, and resource operation is present beneath emitted `d2b` behavior or has an explicit parity/retirement record; operation inspection reaches the durable backend; pending-audit recovery matches the Version 2 contract; exact tests cover Version 1 migration refusal, required `zoneRef`/`schemaVersion`, required Zone, UUIDv7 IDs and expiry, cross-Zone reuse, exits, all remediation actions, sole-binary packaging/help/completions, and no Zone/ID-bearing argv or executable remediation; T595's emitter and all T599-owned consumers match T605's exact names and non-substitution rule; and focused docs/DTO/schema/contract checks are clean.
  Direct Version 2 operator CLI/JSON status and recovery responses are the sole raw-identity
  output exception: a bounded `zoneRef` and, where the exact envelope specifies it, bounded
  `operationId` may echo only the values supplied, generated, or received by that operator as
  recovery coordinates. T599's tests must prove those fields remain confined to that direct
  response and never become telemetry labels, spans, exported audit identities, or unrelated
  error context. An envelope such as `operation-not-found` that omits `operationId` must not
  add it as unrelated context.
  Preserve the accepted `op inspect` controls as
  `[--watch] [--deadline <DURATION> | --no-deadline]`: test each flag, their mutual-exclusion
  refusal, default-deadline behavior, and signal cancellation with no deadline. Human recovery
  narrows the preceding shared-remediation clause: JSON alone carries a closed action. Human
  mode instead
  renders the exact safe static `d2b op inspect` guidance from `contracts/operator-cli.md`
  without flags, identifiers, argv, or shell text; machine output retains only the closed
  remediation-action enum and never gains a free-form guidance field.
  T599 additionally owns the public versioned runbook
  `docs/how-to/host-generation-recovery-v1.md` and the generated closed mapping
  `docs/reference/host-generation-recovery-actions-v1.json`. Every public recovery action in
  `contracts/operator-cli.md` must resolve to either an exact CLI invocation or the
  identically named runbook anchor with a named operator role; bare procedure names are
  ineligible.   The historical T599 design had link and contract tests enumerate the emitted action set and fail on a
  missing, extra, duplicate, unowned, or broken mapping. The historical plan required both
  artifacts to be committed and referenced by the CLI contract.
  For resource mutations, T599 generates an omitted `--operation-id` client-side before
  transport creation. The commit-then-response-loss test requires exact human and JSON
  output containing that bounded ID and `zoneRef`, action `inspect-operation`, and recovery
  through `d2b op inspect` with zero second mutation; generating a replacement ID after an
  ambiguous response was forbidden by that historical plan.
## Implementation sequence: Remaining Provider dossiers in five file-disjoint families

These groups contain Provider implementation and focused validation for the remaining
resource families. Start file-disjoint groups when their implementation dependencies are met;
no approval or delivery-state artifact is required.

- [ ] T221 [US2] Implement the remaining Network and Provider work with focused validation. Require
  the Network double-opt-in contract
  Network.spec.isolation.allowEastWest && d2b.site.allowUnsafeEastWest, both defaults false,
  and retain all four Network/Host cases. Confirm the broker handoff and recovery contracts,
  then run the applicable conditional host/live checks.

### Group `wi:ADR-046-provider-activation-nixos` (7 items)

NIX-8 and NIX-9 are code-canon gaps, not landed earlier grouped work. The exact searches recorded in
`research.md` found no production handoff contract and no rebuild-reference option.
Prospective ownership resolves from owning technical specifications and committed code.

- [ ] T222 [P] [US2] `ADR046-activation-001` - complete the shared activation contract;
  depends on `ADR046-audit-001` and `ADR046-cli-001`
- [ ] T223 [P] [US2] `ADR046-activation-002` - docs/reference/schemas/v3/activation-nixos.d2bus.org.NixosGeneration.json and packages/d2b-contracts/src/activation_nixos.rs (create)
- [ ] T224 [US2] `ADR046-activation-003` - packages/d2b-provider-activation-nixos/src/controller/ (replace)
- [ ] T225 [US2] `ADR046-activation-004` - packages/d2b-provider-activation-nixos/src/runner/ (adapt)
- [ ] T226 [P] [US2] `ADR046-activation-005` - packages/d2b/src/activation.rs (replace)
- [ ] T227 [P] [US2] `ADR046-activation-006` - complete the activation options contract;
  depends on `ADR046-activation-001` and `ADR046-nix-003`; update `options-site.nix` after
  those dependencies
- [ ] T228 [US2] `ADR046-activation-007` - packages/d2b/src/lib.rs (delete-after-cutover)

### Group `wi:ADR-046-provider-audio-pipewire` (13 items)

- [ ] T229 [P] [US2] `ADR046-audio-001` - `packages/d2b-provider-audio-pipewire/src/audio_policy.rs` (copy-unchanged)
- [ ] T230 [US2] `ADR046-audio-002` - `packages/d2b-provider-audio-pipewire/src/argv.rs` (component template renderer) (adapt)
- [ ] T231 [US2] `ADR046-audio-004` - `packages/d2b-provider-audio-pipewire/src/mediator/enforcement.rs` (adapt)
- [ ] T232 [US2] `ADR046-audio-005` - `packages/d2b-provider-audio-pipewire/src/{resource_type,admission,provider_extension}.rs` (adapt)
- [ ] T233 [US2] `ADR046-audio-006` - `packages/d2b-provider-audio-pipewire/src/controller/audio_service.rs` (adapt)
- [ ] T234 [US2] `ADR046-audio-007` - `packages/d2b-provider-audio-pipewire/src/mediator/mod.rs` (create)
- [ ] T235 [US2] `ADR046-audio-008` - `nixos-modules/components/audio/v3-resource.nix` (replace)
- [ ] T236 [US2] `ADR046-audio-009` - `packages/d2b-provider-audio-pipewire/tests/minijail_contract.rs` (provider-local) (adapt)
- [ ] T237 [US2] `ADR046-audio-010` - `packages/d2b-provider-audio-pipewire/src/telemetry.rs` (adapt)
- [ ] T238 [US2] `ADR046-audio-011` - `packages/d2b-provider-audio-pipewire/src/guest_agent/mod.rs` (adapt)
- [ ] T239 [US2] `ADR046-audio-012` - `packages/d2b-provider-audio-pipewire/src/share_adapter.rs` (adapt)
- [ ] T240 [US2] `ADR046-audio-013` - `packages/d2b-provider-audio-pipewire/src/authority.rs` (speaker mixer + mic arbiter) (adapt)
- [ ] T241 [US2] `ADR046-audio-014` - `packages/d2b-provider-audio-pipewire/src/streams.rs` (adapt)

### Group `wi:ADR-046-provider-clipboard-wayland` (12 items)

- [ ] T242 [P] [US2] `ADR046-clipboard-001` - packages/d2b-provider-clipboard-wayland/ with src (create)
- [ ] T243 [US2] `ADR046-clipboard-002` - packages/d2b-provider-clipboard-wayland/src/clipd_host/ service binary modules such as service (adapt)
- [ ] T244 [US2] `ADR046-clipboard-003` - packages/d2b-provider-clipboard-wayland/src/controller/ and clipboard-controller binary (create)
- [ ] T245 [US2] `ADR046-clipboard-004` - packages/d2b-provider-clipboard-wayland/src/picker_session/ and picker-session binary (adapt)
- [ ] T246 [US2] `ADR046-clipboard-005` - packages/d2b-provider-clipboard-wayland service descriptors and generated Rust async ttrpc bindings (create)
- [ ] T247 [P] [US2] `ADR046-clipboard-006` - nixos-modules/providers/clipboard-wayland.nix and d2b.artifacts.clipboard-wayland catalog entry (replace)
- [ ] T248 [US2] `ADR046-clipboard-007` - packages/d2b-provider-clipboard-wayland/src/controller/rbac.rs or equivalent controller reconcile module (create)
- [ ] T249 [US2] `ADR046-clipboard-008` - packages/d2b-provider-clipboard-wayland/src/service/audit.rs and packages/d2b-provider-clipboard-wayland/src/service/metrics.rs (adapt)
- [ ] T250 [US2] `ADR046-clipboard-009` - packages/d2b-provider-clipboard-wayland/tests/ (extract)
- [ ] T251 [US2] `ADR046-clipboard-010` - packages/d2b-provider-clipboard-wayland/integration/ (create)
- [ ] T252 [US2] `ADR046-clipboard-011` - packages/d2b-contract-tests/tests/policy_clipboard.rs (adapt)
- [ ] T253 [US2] `ADR046-clipboard-012` - nixos-modules/default.nix (delete-after-cutover)

### Group `wi:ADR-046-provider-credential-entra` (1 items)

- [ ] T254 [US2] `ADR046-cred-entra-001` - `packages/d2b-provider-credential-entra/src/{lib.rs,controller.rs,service.rs,controller_main.rs,agent_main.rs,audit.rs,telemetry.rs}` (adapt)

### Group `wi:ADR-046-provider-credential-managed-identity` (5 items)

- [ ] T255 [US2] `ADR046-cred-mi-001` - `packages/d2b-provider-credential-managed-identity/src/{lib.rs, controller.rs, agent.rs, service.rs, audit.rs, telemetry.rs}` (adapt)
- [ ] T256 [US2] `ADR046-cred-mi-002` - packages/d2b-provider-credential-managed-identity/src/controller.rs (adapt)
- [ ] T257 [US2] `ADR046-cred-mi-003` - nixos-modules/options-resources.nix (replace)
- [ ] T258 [US2] `ADR046-cred-mi-004` - packages/d2b-provider-credential-managed-identity/src/{audit.rs,telemetry.rs} (adapt)
- [ ] T259 [US2] `ADR046-mi-topology-001` - packages/d2b-provider-credential-managed-identity/src/{controller.rs,agent.rs} (adapt)

### Group `wi:ADR-046-provider-credential-secret-service` (6 items)

- [ ] T260 [P] [US2] `ADR046-cred-ss-001` - packages/d2b-contracts/src/v3/credential.rs (adapt)
- [ ] T261 [P] [US2] `ADR046-cred-ss-002` - packages/d2b-contracts/proto/v3/credential.proto (create)
- [ ] T262 [US2] `ADR046-cred-ss-003` - `packages/d2b-provider-credential-secret-service/src/{lib.rs, controller.rs, service.rs, main.rs}` (adapt)
- [ ] T263 [P] [US2] `ADR046-cred-ss-004` - packages/d2b-provider-credential-<impl>/src/controller.rs (create)
- [ ] T264 [P] [US2] `ADR046-cred-ss-005` - nixos-modules/options-resources.nix (create)
- [ ] T265 [P] [US2] `ADR046-cred-ss-006` - packages/d2b-provider-credential-secret-service/src/{audit.rs,telemetry.rs} (adapt)

### Group `wi:ADR-046-provider-device-gpu` (9 items)

- [ ] T266 [P] [US2] `ADR046-gpu-001` - `packages/d2b-provider-device-gpu/` with `src/` (extract)
- [ ] T267 [US2] `ADR046-gpu-002` - `packages/d2b-provider-device-gpu/src/{controller.rs,telemetry.rs}` (adapt)
- [ ] T268 [US2] `ADR046-gpu-003` - `packages/d2b-provider-device-gpu/src/probe.rs` (create)
- [ ] T269 [US2] `ADR046-gpu-004` - `packages/d2b-provider-device-gpu/src/arbitration.rs` (create)
- [ ] T270 [US2] `ADR046-gpu-005` - `packages/d2b-provider-device-gpu/src/worker_gpu.rs` (adapt)
- [ ] T271 [US2] `ADR046-gpu-006` - `packages/d2b-provider-device-gpu/src/worker_video.rs` (adapt)
- [ ] T272 [US2] `ADR046-gpu-007` - `nixos-modules/assertions.nix` (new GPU Device eval assertions) (adapt)
- [ ] T273 [US2] `ADR046-gpu-008` - `packages/d2b-provider-device-gpu/` component descriptor (create)
- [ ] T274 [US2] `ADR046-gpu-009` - `packages/d2b-provider-device-gpu/README.md` (create)

### Group `wi:ADR-046-provider-device-security-key` (35 items)

- [ ] T275 [US2] `ADR046-security-key-001` - Move to `packages/d2b-provider-device-security-key/src/session.rs` and `cid.rs` (adapt)
- [ ] T276 [US2] `ADR046-security-key-002` - Move to `packages/d2b-provider-device-security-key/src/relay.rs` (adapt)
- [ ] T277 [US2] `ADR046-security-key-003` - Adopt `main.rs` and `uhid.rs` as the v3 Process binary entry point (adapt)
- [ ] T278 [US2] `ADR046-security-key-004` - Preserve revalidation logic (adapt)
- [ ] T279 [US2] `ADR046-security-key-005` - Adapt to v3 Zone/ResourceRef identifiers (adapt)
- [ ] T280 [US2] `ADR046-security-key-006` - Move to `packages/d2b-provider-device-security-key/tests/` (adapt)
- [ ] T281 [US2] `ADR046-security-key-007` - Move to `packages/d2b-provider-device-security-key/tests/` (adapt)
- [ ] T282 [P] [US2] `ADR046-security-key-008` - New crate `packages/d2b-provider-device-security-key/` with `src/` (create)
- [ ] T283 [US2] `ADR046-security-key-009` - `packages/d2b-provider-device-security-key/src/controller.rs` (create)
- [ ] T284 [US2] `ADR046-security-key-010` - `packages/d2b-provider-device-security-key/src/relay.rs` (create)
- [ ] T285 [US2] `ADR046-security-key-011` - `packages/d2b-provider-device-security-key/src/session.rs` (create)
- [ ] T286 [US2] `ADR046-security-key-012` - `packages/d2b-provider-device-security-key/src/cid.rs` (create)
- [ ] T287 [US2] `ADR046-security-key-013` - `packages/d2b-provider-device-security-key/src/probe.rs` (create)
- [ ] T288 [US2] `ADR046-security-key-014` - `packages/d2b-provider-device-security-key/src/descriptor.rs` (create)
- [ ] T289 [US2] `ADR046-security-key-015` - `nixos-modules/minijail-profiles.nix` entries for relay and controller (create)
- [ ] T290 [US2] `ADR046-security-key-016` - Provider descriptor Process templates and owned CTAPHID `Endpoint` template for `Provider/device-security-key` (create)
- [ ] T291 [US2] `ADR046-security-key-017` - Signed Provider descriptor JSON for `Provider/device-security-key` in the provider package (create)
- [ ] T292 [US2] `ADR046-security-key-018` - v3 `SecurityKeyOpenDevice` broker op and Core LaunchTicket DeviceGrant resolution path (create)
- [ ] T293 [US2] `ADR046-security-key-019` - `nixos-modules/` resource compiler/eval assertions for physical Device (create)
- [ ] T294 [US2] `ADR046-security-key-020` - `nixos-modules/components/security-key-guest.nix` migration gate `d2b.securityKey._legacySystemdUnit` (create)
- [ ] T295 [US2] `ADR046-security-key-021` - Core `device-grant` audit and Provider controller Service/Binding ceremony lifecycle audit (create)
- [ ] T296 [US2] `ADR046-security-key-022` - Provider/controller bounded telemetry emitter and observability-otel handoff for security-key metrics (create)
- [ ] T297 [US2] `ADR046-security-key-023` - `packages/d2b-provider-device-security-key/README.md` (create)
- [ ] T298 [US2] `ADR046-security-key-024` - Authority/projection Service Endpoint and Binding private Endpoint resolution (create)
- [ ] T299 [US2] `ADR046-security-key-025` - `d2b-contracts` neutral `SecurityKeyEffectPort` trait/types (create)
- [ ] T300 [US2] `ADR046-security-key-026` - `packages/d2b-provider-device-security-key/src/{resource_type,provider_extension,admission}.rs` (create)
- [ ] T301 [US2] `ADR046-security-key-027` - Provider descriptor state declaration (create)
- [ ] T302 [US2] `ADR046-security-key-028` - `packages/d2b-provider-device-security-key/src/share_adapter.rs` (adapt)
- [ ] T303 [US2] `ADR046-security-key-029` - `packages/d2b-provider-device-security-key/src/{authority,relay,streams}.rs` (adapt)
- [ ] T304 [US2] `ADR046-security-key-030` - Removed from daemon (delete-after-cutover)
- [ ] T305 [US2] `ADR046-security-key-031` - Removed from daemon startup (delete-after-cutover)
- [ ] T306 [US2] `ADR046-security-key-032` - Removed from guest Nix module (delete-after-cutover)
- [ ] T307 [US2] `ADR046-security-key-033` - Removed from `packages/d2b-contract-tests/tests/` (delete-after-cutover)
- [ ] T308 [US2] `ADR046-security-key-034` - Removed from `d2b-core/src/processes.rs` (delete-after-cutover)
- [ ] T309 [US2] `ADR046-security-key-035` - Removed from contracts and broker (delete-after-cutover)

### Group `wi:ADR-046-provider-device-tpm` (13 items)

- [ ] T310 [P] [US2] `ADR046-device-tpm-001` - packages/d2b-provider-device-tpm/{src/,tests/,integration/README.md,README.md} (adapt)
- [ ] T311 [US2] `ADR046-device-tpm-002` - packages/d2b-provider-device-tpm/src/effect_port.rs (wrap)
- [ ] T312 [US2] `ADR046-device-tpm-003` - packages/d2b-provider-device-tpm/src/controller.rs (replace)
- [ ] T313 [US2] `ADR046-device-tpm-004` - packages/d2b-provider-device-tpm/src/resources.rs (replace)
- [ ] T314 [US2] `ADR046-device-tpm-005` - packages/d2b-provider-device-tpm/src/resources.rs (adapt)
- [ ] T315 [US2] `ADR046-device-tpm-006` - packages/d2b-provider-device-tpm/src/resources.rs (adapt)
- [ ] T316 [US2] `ADR046-device-tpm-007` - packages/d2b-provider-device-tpm/src/status.rs (create)
- [ ] T317 [US2] `ADR046-device-tpm-008` - packages/d2b-provider-device-tpm/src/{effect_port.rs,status.rs} (replace)
- [ ] T318 [US2] `ADR046-device-tpm-009` - packages/d2b-provider-device-tpm/tests/marker_fail_closed.rs (adapt)
- [ ] T319 [US2] `ADR046-device-tpm-010` - packages/d2b-provider-device-tpm/src/resources.rs (create)
- [ ] T320 [US2] `ADR046-device-tpm-011` - nixos-modules/options-resources.nix and Nix eval/golden tests for §17.1 Device JSON (replace)
- [ ] T321 [US2] `ADR046-device-tpm-012` - packages/d2b-provider-device-tpm/src/controller.rs (adapt)
- [ ] T322 [US2] `ADR046-device-tpm-013` - packages/d2bd/src/* (delete-after-cutover)

### Group `wi:ADR-046-provider-device-usbip` (9 items)

- [ ] T323 [P] [US2] `ADR046-usbip-001` - packages/d2b-contracts/src/usbip_effect_port.rs (create)
- [ ] T324 [US2] `ADR046-usbip-002` - packages/d2b-core/src/device_usbip_adapter.rs (adapt)
- [ ] T325 [US2] `ADR046-usbip-003` - packages/d2b-provider-device-usbip/ (create)
- [ ] T326 [US2] `ADR046-usbip-004` - packages/d2b-provider-device-usbip/src/{controller,reconcile,export_import}.rs (adapt)
- [ ] T327 [US2] `ADR046-usbip-005` - packages/d2b-provider-device-usbip/src/reconcile.rs (adapt)
- [ ] T328 [US2] `ADR046-usbip-006` - packages/d2b-provider-device-usbip/src/status.rs (adapt)
- [ ] T329 [US2] `ADR046-usbip-007` - packages/d2b-provider-device-usbip/{src,tests,integration/README.md} (adapt)
- [ ] T330 [US2] `ADR046-usbip-008` - nixos-modules/components/usbip.nix (adapt)
- [ ] T331 [US2] `ADR046-usbip-009` - packages/d2bd/src/ (delete-after-cutover)

### Group `wi:ADR-046-provider-display-wayland` (4 items)

- [ ] T332 [US2] `ADR046-display-001` - `packages/d2b-provider-display-wayland/src/` (adapt)
- [ ] T333 [US2] `ADR046-display-002` - Zone bundle emitter for `WaylandSession` / `WaylandPolicy` ResourceSpecs under `d2b.zones.<zone>.resources.*` (adapt)
- [ ] T334 [US2] `ADR046-display-003` - `packages/d2b-provider-display-wayland/src/audit.rs` (adapt)
- [ ] T335 [US2] `ADR046-display-004` - `packages/d2b-provider-display-wayland/integration/` (create)

### Group `wi:ADR-046-provider-network-local` (20 items)

**Current implementation remains in this provider grouping:** `ADR046-nl-001` through `ADR046-nl-020` cover the
production implementation. The `ADR-046-resources-network` contract must require
`Network.spec.isolation.allowEastWest && d2b.site.allowUnsafeEastWest`, default both false,
remove every current-facing sole Network-opt-in path. The implementation includes the
production adapter, site-gate transport, schema migration, and all four real
emitter/controller/broker/net-VM cases. Focused cross-provider tests execute the closed
predicates without taking ownership of Network implementation.

- [ ] T336 [P] [US2] `ADR046-nl-001` - preserve the landed `d2b-provider-network-local::controller::NetworkEffectPort`; implement the authoritative generated destination and typed broker adapter with no direct host mutation (adapt/create)
- [ ] T337 [US2] `ADR046-nl-002` - Broker wire contract and broker/core adapter operation table for `DeletePersistentTap` (adapt)
- [ ] T338 [P] [US2] `ADR046-nl-003` - `d2b-contracts` opaque byte-array newtypes (create)
- [ ] T339 [US2] `ADR046-nl-004` - Core LaunchTicket builder and dependency resolver that walks `Guest.ownerRef: Network/<name>` to resolved tap FDs. (create)
- [ ] T340 [P] [US2] `ADR046-nl-005` - `d2bd` Network adapter maps opaque Provider intents only to typed broker operations; no `d2b-host` mutation API is imported or called (adapt)
- [ ] T341 [US2] `ADR046-nl-006` - `packages/d2b-provider-network-local/src/{controller.rs,metrics.rs}` (adapt)
- [ ] T342 [P] [US2] `ADR046-nl-007` - `packages/d2b-provider-network-local/src/process_specs.rs` agent template plus agent service implementation in the net-VM artifact. (create)
- [ ] T343 [P] [US2] `ADR046-nl-008` - `packages/d2b-provider-network-local/src/config_volume.rs`. (adapt)
- [ ] T344 [P] [US2] `ADR046-nl-009` - `packages/d2b-provider-network-local/src/process_specs.rs`. (adapt)
- [ ] T345 [P] [US2] `ADR046-nl-010` - `net-vm-base` nixos-system artifact and artifact catalog entry `d2b.artifacts.net-vm-base`. (adapt)
- [ ] T346 [P] [US2] `ADR046-nl-011` - Nix module resource emission for `Provider/network-local` (adapt)
- [ ] T347 [P] [US2] `ADR046-nl-012` - Nix flake/resource schema checks for declared Networks and provider `validate.rs` parity. (adapt)
- [ ] T348 [P] [US2] `ADR046-nl-013` - `packages/d2b-provider-network-local/tests/schema_roundtrip.rs` (adapt)
- [ ] T349 [US2] `ADR046-nl-014` - `packages/d2b-provider-network-local/tests/controller_state.rs`. (create)
- [ ] T350 [P] [US2] `ADR046-nl-015` - `packages/d2b-provider-network-local/integration/host_fabric.rs` (adapt)
- [ ] T351 [P] [US2] `ADR046-nl-016` - Process templates for agent and dnsmasq plus sandbox/eval tests. (adapt)
- [ ] T352 [P] [US2] `ADR046-nl-017` - `packages/d2b-provider-network-local/README.md`. (create)
- [ ] T353 [P] [US2] `ADR046-nl-018` - Device-usbip EffectPort/adapter owns USBIP rules (adapt)
- [ ] T354 [P] [US2] `ADR046-nl-019` - Provider descriptor (create)
- [ ] T355 [P] [US2] `ADR046-nl-020` - Network schema/Provider descriptor (adapt)

- [ ] T604 [US1] Coordinate focused operator-activation and daemon-restart acceptance. Own the
  listed contract tests, host fixtures, case-id fixtures, Makefile recipe, and changelog entry.
  Validate automatic startup, declaration/removal ingestion, real effects and Ready status for
  Volume/acceptance-state, Network/acceptance-net, and Device/acceptance-tpm, including Device
  cleanup and preserved TPM state. Run the public fixture and host targets with no skips.
  Author and validate the operator-nix-activation-cleanup check; candidate-specific evidence is
  emitted only by T479 after the exact release tree is converged.

### Group `wi:ADR-046-provider-notification-desktop` (6 items)

- [ ] T356 [P] [US2] `ADR046-notify-001` - `packages/d2b-provider-notification-desktop/src/{types,redact,action_nonce}.rs` (adapt)
- [ ] T357 [US2] `ADR046-notify-002` - `packages/d2b-provider-notification-desktop/src/stream_admission.rs` (adapt)
- [ ] T358 [US2] `ADR046-notify-003` - `packages/d2b-provider-notification-desktop/src/controller.rs` (create)
- [ ] T359 [US2] `ADR046-notify-004` - `packages/d2b-provider-notification-desktop/src/host_sink.rs` (adapt)
- [ ] T360 [US2] `ADR046-notify-005` - `packages/d2b-provider-notification-desktop/src/guest_source.rs` (create)
- [ ] T361 [US2] `ADR046-notify-006` - Nix: Zone resource authoring in `nixos-modules/` (adapt)

### Group `wi:ADR-046-provider-observability-otel` (6 items)

- [ ] T362 [US2] `ADR046-otel-001` - `packages/d2b-provider-observability-otel/src/{forwarder_bin,controller,binding}.rs` (adapt)
- [ ] T363 [US2] `ADR046-otel-002` - `packages/d2b-provider-observability-otel/src/{collector_bin,emitter_socket,ingress_policy,exporter,controller,service,binding}.rs` (adapt)
- [ ] T364 [US2] `ADR046-otel-003` - `packages/d2b-provider-observability-otel/src/nix/journald.nix` (adapt)
- [ ] T365 [US2] `ADR046-otel-004` - `packages/d2b-contract-tests/tests/policy_observability.rs` (updated) (adapt)
- [ ] T366 [US2] `ADR046-otel-005` - `packages/d2b-provider-observability-otel/src/share_adapter.rs` (adapt)
- [ ] T367 [US2] `ADR046-otel-006` - `packages/d2b-provider-observability-otel/src/{authority,service,binding,projection}.rs` (adapt)

### Group `wi:ADR-046-provider-runtime-azure-container-apps` (7 items)

- [ ] T368 [US2] `ADR046-aca-001` - `packages/d2b-provider-runtime-azure-container-apps/src/controller.rs` (replace)
- [ ] T369 [US2] `ADR046-aca-002` - `packages/d2b-provider-runtime-azure-container-apps/src/deployment_service.rs` (adapt)
- [ ] T370 [US2] `ADR046-aca-003` - `packages/d2b-contracts/src/provider_effects/aca.rs` (adapt)
- [ ] T371 [US2] `ADR046-aca-004` - ACA sandbox-agent Endpoint/session controller (replace)
- [ ] T372 [US2] `ADR046-aca-005` - `packages/d2b-provider-runtime-azure-container-apps/src/types.rs` (adapt)
- [ ] T373 [US2] `ADR046-aca-006` - `nixos-modules/` (generated Guest resource options) (replace)
- [ ] T374 [US2] `ADR046-aca-007` - `nixos-modules/` (create)

### Group `wi:ADR-046-provider-runtime-azure-virtual-machine` (9 items)

- [ ] T375 [P] [US2] `ADR046-azure-vm-001` - `src/{lib.rs,config.rs,schema.rs,error.rs,effect/mod.rs}` (adapt)
- [ ] T376 [US2] `ADR046-azure-vm-002` - `src/effect/{mod.rs,real.rs,fake.rs,rate_limit.rs}` (adapt)
- [ ] T377 [US2] `ADR046-azure-vm-003` - `src/controller/{mod.rs,lifecycle.rs,idempotency.rs}` (adapt)
- [ ] T378 [US2] `ADR046-azure-vm-004` - `src/controller/bootstrap.rs` (adapt)
- [ ] T379 [US2] `ADR046-azure-vm-005` - `src/credential.rs` (adapt)
- [ ] T380 [US2] `ADR046-azure-vm-006` - `src/controller/idempotency.rs` (adapt)
- [ ] T381 [US2] `ADR046-azure-vm-007` - `nixos-modules/` (Provider/Guest resource emitters) (adapt)
- [ ] T382 [US2] `ADR046-azure-vm-008` - `src/{telemetry.rs,audit.rs}` (adapt)
- [ ] T383 [P] [US2] `ADR046-azure-vm-009` - `tests/` (adapt)

### Group `wi:ADR-046-provider-runtime-cloud-hypervisor` (7 items)

- [ ] T384 [P] [US2] `ADR046-ch-001` - complete the cloud-hypervisor Provider implementation and its validation surface: `packages/d2b-provider-runtime-cloud-hypervisor/src/controller.rs`, `tests/host-integration/runtime-cloud-hypervisor-guest-preflight.nix`, the corresponding discovery/build recipe in `Makefile`, and end-to-end real-KVM/guest-control validation through exact attr `vmChecks.x86_64-linux.runtime-cloud-hypervisor-guest-preflight`
- [ ] T385 [US2] `ADR046-ch-002` - `packages/d2b-provider-runtime-cloud-hypervisor/src/bootstrap_graph.rs` (replace)
- [ ] T386 [US2] `ADR046-ch-003` - `packages/d2b-provider-runtime-cloud-hypervisor/src/vmm_argv.rs` (adapt)
- [ ] T387 [US2] `ADR046-ch-004` - `packages/d2b-provider-runtime-cloud-hypervisor/nix/` (Nix emitter) (adapt)
- [ ] T388 [US2] `ADR046-ch-005` - `packages/d2b-provider-runtime-cloud-hypervisor/src/health.rs` (adapt)
- [ ] T389 [US2] `ADR046-ch-006` - `packages/d2b-provider-runtime-cloud-hypervisor/src/metrics.rs` (replace)
- [ ] T390 [US2] `ADR046-ch-007` - `packages/d2b-provider-runtime-cloud-hypervisor/src/state.rs` (replace)

### Group `wi:ADR-046-provider-runtime-qemu-media` (19 items)

- [ ] T391 [P] [US2] `ADR046-qemu-media-001` - packages/d2b-provider-runtime-qemu-media/{src/lib.rs,tests/provider_layout.rs,integration/mod.rs,README.md} (create)
- [ ] T392 [US2] `ADR046-qemu-media-002` - packages/d2b-provider-runtime-qemu-media/src/types/guest.rs (adapt)
- [ ] T393 [US2] `ADR046-qemu-media-003` - packages/d2b-provider-runtime-qemu-media/src/config.rs (adapt)
- [ ] T394 [US2] `ADR046-qemu-media-004` - packages/d2b-provider-runtime-qemu-media/src/{descriptor.rs,state.rs} (create)
- [ ] T395 [US2] `ADR046-qemu-media-005` - packages/d2b-provider-runtime-qemu-media/src/controller/volume.rs (adapt)
- [ ] T396 [US2] `ADR046-qemu-media-006` - packages/d2b-provider-runtime-qemu-media/src/controller/media_watch.rs (adapt)
- [ ] T397 [US2] `ADR046-qemu-media-007` - packages/d2b-provider-runtime-qemu-media/src/controller/device_watch.rs (create)
- [ ] T398 [US2] `ADR046-qemu-media-008` - packages/d2b-provider-runtime-qemu-media/src/controller/display.rs (create)
- [ ] T399 [US2] `ADR046-qemu-media-009` - packages/d2b-provider-runtime-qemu-media/src/controller/process_builder.rs (adapt)
- [ ] T400 [US2] `ADR046-qemu-media-010` - packages/d2b-provider-runtime-qemu-media/src/qmp/ (adapt)
- [ ] T401 [US2] `ADR046-qemu-media-011` - packages/d2b-provider-runtime-qemu-media/src/controller/hotplug.rs (adapt)
- [ ] T402 [US2] `ADR046-qemu-media-012` - packages/d2b-provider-runtime-qemu-media/src/controller/network.rs (create)
- [ ] T403 [US2] `ADR046-qemu-media-013` - packages/d2b-provider-runtime-qemu-media/src/controller/reconcile.rs (create)
- [ ] T404 [US2] `ADR046-qemu-media-014` - packages/d2b-provider-runtime-qemu-media/src/controller/status.rs (create)
- [ ] T405 [US2] `ADR046-qemu-media-015` - packages/d2b-provider-runtime-qemu-media/src/audit.rs (create)
- [ ] T406 [US2] `ADR046-qemu-media-016` - packages/d2b-provider-runtime-qemu-media/src/telemetry.rs (create)
- [ ] T407 [US2] `ADR046-qemu-media-017` - nixos-modules/options-guest-qemu-media.nix (adapt)
- [ ] T408 [US2] `ADR046-qemu-media-018` - packages/d2b-provider-runtime-qemu-media/tests/conformance_guest.rs (adapt)
- [ ] T409 [US2] `ADR046-qemu-media-019` - packages/d2b-provider-runtime-qemu-media/integration/ (create)

### Group `wi:ADR-046-provider-shell-terminal` (13 items)

- [ ] T410 [P] [US2] `ADR046-sterm-001` - `packages/d2b-provider-shell-terminal/src/resources/{pool,session}.rs` (create)
- [ ] T411 [P] [US2] `ADR046-sterm-002` - `packages/d2b-provider-shell-terminal/src/bin/d2b-shell-terminal-controller.rs` (create)
- [ ] T412 [P] [US2] `ADR046-sterm-003` - `packages/d2b-provider-shell-terminal/src/bin/d2b-shell-session-supervisor.rs` (adapt)
- [ ] T413 [P] [US2] `ADR046-sterm-004` - `packages/d2b-provider-shell-terminal/src/process_templates.rs` (replace)
- [ ] T414 [P] [US2] `ADR046-sterm-005` - `packages/d2b-provider-shell-terminal/src/service/open_session.rs` (create)
- [ ] T415 [P] [US2] `ADR046-sterm-006` - `packages/d2b-provider-shell-terminal/src/session/{pty,ring}.rs` (adapt)
- [ ] T416 [P] [US2] `ADR046-sterm-007` - `packages/d2b-provider-shell-terminal/src/session/adopt.rs` (adapt)
- [ ] T417 [P] [US2] `ADR046-sterm-008` - `packages/d2b-provider-shell-terminal/src/host_rules.rs` (replace)
- [ ] T418 [P] [US2] `ADR046-sterm-009` - `packages/d2b-provider-shell-terminal/src/guest_rules.rs` (replace)
- [ ] T419 [P] [US2] `ADR046-sterm-010` - `packages/d2b-provider-shell-terminal/src/authz.rs` (replace)
- [ ] T420 [P] [US2] `ADR046-sterm-011` - `packages/d2b-provider-shell-terminal/src/{audit,telemetry}.rs` (create)
- [ ] T421 [P] [US2] `ADR046-sterm-012` - `packages/d2b-provider-shell-terminal/src/migration.rs` (delete-after-cutover)
- [ ] T422 [P] [US2] `ADR046-sterm-013` - `packages/d2b-provider-shell-terminal/src/service/{controller,supervisor}.rs` (adapt)

### Group `wi:ADR-046-provider-system-core` (1 items)

- [ ] T423 [US2] `ADR046-system-core-001` - complete the system-core Provider implementation;
  coordinate with the Provider, core, Zone-control, CLI, execution, state, telemetry, and
  audit tasks; serialize the later Zone contract/CLI writer

### Group `wi:ADR-046-provider-system-minijail` (6 items)

- [ ] T424 [US2] `ADR046-minijail-001` - `packages/d2b-provider-system-minijail/src/sandbox_compiler.rs` (adapt)
- [ ] T425 [US2] `ADR046-minijail-002` - Provider-side opaque request builder in `packages/d2b-provider-system-minijail/src/launch.rs` (adapt)
- [ ] T426 [US2] `ADR046-minijail-003` - Broker-side: `d2b-priv-broker` retains `SpawnRunner` and user-namespace pre-establishment (adapt)
- [ ] T427 [US2] `ADR046-minijail-004` - Broker-side parent wait/reap and typed terminal relay in `packages/d2b-priv-broker/src/` (adapt)
- [ ] T428 [US2] `ADR046-minijail-005` - `packages/d2b-provider-system-minijail/src/` - controller binary entry point (adapt)
- [ ] T429 [US2] `ADR046-minijail-006` - `nixos-modules/` - v3 Nix `Process`/`EphemeralProcess` resource authoring (adapt)

### Group `wi:ADR-046-provider-system-systemd` (3 items)

- [ ] T430 [US2] `ADR046-systemd-001` - `packages/d2b-provider-system-systemd/src/controller.rs` (async reconcile loop) (adapt)
- [ ] T431 [US2] `ADR046-systemd-002` - `nixos-modules/` (Provider ResourceSpec emission for `system-systemd`) (adapt)
- [ ] T432 [US2] `ADR046-systemd-003` - `packages/d2b-provider-system-systemd/tests/conformance.rs` (adapt)

### Group `wi:process-provider-integration:w6` (1 item)

- [ ] T039 [US1] `ADR046-process-002` - `packages/d2b-provider-system-systemd/`, `packages/d2b-provider-system-minijail/` (adapt). The production composition and conditional Layer 2 evidence remain outstanding beyond the existing hermetic surfaces.

### Group `wi:ADR-046-provider-transport-azure-relay` (7 items)

- [ ] T433 [P] [US2] `ADR046-transport-relay-001` - `packages/d2b-provider-transport-azure-relay/src/relay_transport.rs` (adapt)
- [ ] T434 [US2] `ADR046-transport-relay-002` - `packages/d2b-provider-transport-azure-relay/src/credential_client.rs` (create)
- [ ] T435 [US2] `ADR046-transport-relay-003` - `packages/d2b-provider-transport-azure-relay/src/reconnect.rs` (create)
- [ ] T436 [US2] `ADR046-transport-relay-004` - `packages/d2b-provider-transport-azure-relay/src/transport_settings.rs` (create)
- [ ] T437 [US2] `ADR046-transport-relay-005` - `packages/d2b-provider-transport-azure-relay/src/backpressure.rs` (adapt)
- [ ] T438 [US2] `ADR046-transport-relay-006` - `packages/d2b-provider-transport-azure-relay/src/{metrics.rs, audit.rs}` (create)
- [ ] T439 [P] [US2] `ADR046-transport-relay-007` - `packages/d2b-provider-transport-azure-relay/src/tests/integration/README` (create)

### Group `wi:ADR-046-provider-transport-unix` (11 items)

- [ ] T440 [US2] `ADR046-transport-unix-001` - `packages/d2b-provider-transport-unix/src/credit.rs` (adapt)
- [ ] T441 [US2] `ADR046-transport-unix-002` - `packages/d2b-provider-transport-unix/src/{seqpacket,identity,socket}.rs` (adapt)
- [ ] T442 [US2] `ADR046-transport-unix-003` - `packages/d2b-provider-transport-unix/src/{stream,socket}.rs` (adapt)
- [ ] T443 [US2] `ADR046-transport-unix-004` - `packages/d2b-provider-transport-unix/src/credit.rs` (adapt)
- [ ] T444 [US2] `ADR046-transport-unix-005` - `packages/d2b-provider-transport-unix/src/descriptor.rs` (adapt)
- [ ] T445 [US2] `ADR046-transport-unix-006` - complete the shared broker and transport
  contract after `ADR046-transport-unix-002`, `ADR046-session-001`, `ADR046-bus-001`, and
  `ADR046-activation-001`; preserve dependency ordering for the shared contract writer
- [ ] T446 [US2] `ADR046-transport-unix-007` - `packages/d2b-provider-transport-unix/src/{portal,service}.rs` (adapt)
- [ ] T447 [US2] `ADR046-transport-unix-008` - `packages/d2b-provider-transport-unix/` crate Cargo.toml binary target `d2b-transport-unix-service` (adapt)
- [ ] T448 [US2] `ADR046-transport-unix-009` - `docs/reference/schemas/v3/providers/transport-unix.transport-binding.json` (create)
- [ ] T449 [US2] `ADR046-transport-unix-010` - `packages/d2b-provider-transport-unix/src/{audit,metrics}.rs` (create)
- [ ] T450 [US2] `ADR046-transport-unix-011` - `packages/d2b-provider-transport-unix/integration/` and `integration/README.md` (adapt)

### Group `wi:ADR-046-provider-transport-vsock` (7 items)

- [ ] T451 [US2] `ADR046-vsock-001` - `packages/d2b-provider-transport-vsock/src/effect_port.rs` (create)
- [ ] T452 [US2] `ADR046-vsock-002` - `packages/d2b-provider-transport-vsock/src/framing.rs` and `src/bridge.rs` (adapt)
- [ ] T453 [US2] `ADR046-vsock-003` - `packages/d2b-provider-transport-vsock/src/service.rs` (adapt)
- [ ] T454 [US2] `ADR046-vsock-004` - `d2b-core-controller` child Zone runtime `LiveVsockEffectPort` (adapt)
- [ ] T455 [P] [US2] `ADR046-vsock-005` - ProviderDeployment Volume creation/deletion path plus `packages/d2b-provider-transport-vsock/tests/state_volume.rs`. (create)
- [ ] T456 [US2] `ADR046-vsock-006` - `packages/d2b-provider-transport-vsock/integration/host_guest.rs` and `integration/no_fd_transfer.rs`. (create)
- [ ] T457 [P] [US2] `ADR046-vsock-007` - Remove legacy paths from `d2b-host` and `d2bd` (delete-after-cutover)

### Group `wi:ADR-046-provider-volume-local` (13 items)

- [ ] T458 [US2] `ADR046-vl-001` - `d2b-contracts/src/v3/volume_layout.rs` (adapt)
- [ ] T459 [US2] `ADR046-vl-002` - Full `packages/d2b-provider-volume-local/` scaffold per §Crate layout: `src/` (adapt)
- [ ] T460 [US2] `ADR046-vl-003` - `src/controller.rs` (adapt)
- [ ] T461 [US2] `ADR046-vl-004` - `src/store_view.rs` (adapt)
- [ ] T462 [US2] `ADR046-vl-005` - `src/swtpm_volume.rs` (adapt)
- [ ] T463 [US2] `ADR046-vl-006` - `src/source.rs` (block-image and tmpfs branches) (create)
- [ ] T464 [US2] `ADR046-vl-007` - `src/{migration,snapshot,sealing}.rs` (adapt)
- [ ] T465 [US2] `ADR046-vl-008` - `src/relocation.rs` (create)
- [ ] T466 [US2] `ADR046-vl-009` - `src/audit.rs` (adapt)
- [ ] T467 [US2] `ADR046-vl-010` - `nixos-modules/zone-resources.nix` (per §ADR046-pstate-010) (adapt)
- [ ] T468 [US2] `ADR046-vl-011` - `packages/xtask/src/provider_crate_policy.rs` (adapt)
- [ ] T469 [US2] `ADR046-vl-012` - `packages/d2b-host/src/volume_effect_adapter.rs` (or the equivalent host-runtime crate designated by the Zone broker owner) (adapt)
- [ ] T470 [US2] `ADR046-vl-013` - Zone core ProviderDeployment controller-start path (outside `d2b-provider-volume-local`) (create)

### Group `wi:ADR-046-provider-volume-virtiofs` (7 items)

- [ ] T471 [US2] `ADR046-vvfs-001` - `packages/d2b-provider-volume-virtiofs/src/virtiofsd_argv.rs` (adapt)
- [ ] T472 [US2] `ADR046-vvfs-002` - `packages/d2b-provider-volume-virtiofs/src/user_ns.rs` (conformance kit) (extract)
- [ ] T473 [US2] `ADR046-vvfs-003` - `packages/d2b-provider-volume-virtiofs/src/controller.rs` (adapt)
- [ ] T474 [US2] `ADR046-vvfs-004` - `packages/d2b-provider-volume-virtiofs/src/readiness.rs` (adapt)
- [ ] T475 [US2] `ADR046-vvfs-005` - `packages/d2b-provider-volume-virtiofs/src/controller.rs` (pre-launch prerequisite check) (adapt)
- [ ] T476 [US2] `ADR046-vvfs-006` - `nixos-modules/resources-volume.nix` (store-view and user Volume attachment emission) (adapt)
- [ ] T477 [US2] `ADR046-vvfs-export-001` - `packages/d2b-provider-volume-virtiofs/src/export.rs` (create)

### Group `wi:core-controller-coordination:w6` (1 items)

- [ ] T478 [US2] `ADR046-core-002` - `packages/d2b-core-controller/tests/system_core_coordination.rs` (adapt)

- [ ] T479 [US2] Coordinate the exact-tree acceptance. Run the T604 operator check and the
  Cloud Hypervisor plus daemon-restart host cases together once against the same tree. Emit one
  operator-nix-activation-cleanup result and one w6-cloud-hypervisor-guest-acceptance result,
  each bound to that tree, and retain it only when both pass. Missing, duplicate, skipped,
  status-only, fake-boundary, wrong-resource, or wrong-tree evidence fails.
- [ ] T480 [US2] Revalidate the T479 Volume, Network, Device, Guest, and host-continuity results
  against the exact merged tree. Confirm the four Network/Host double-opt-in cases, candidate
  and tree identity, removal proofs, changelog, and applicable CI or host evidence. A content
  change invalidates affected evidence and requires focused checks to run again. Verify the
  merged tree byte-for-byte before release publication and clean external build residue.

**Checkpoint**: Provider implementation, provider effects, host continuity, and exact-tree acceptance
have focused evidence. Full US1 completion remains dependent on the Cloud Hypervisor Guest
Provider effect and the accepted Volume/Network/Device identities.

---

## Historical implementation grouping: Feasibility closure, reset and cutover, security, and release support

**Requirements**: see spec-coverage.md traceability tables | **Story**: US3 | **Work items**: 73 | **Parallel groups**: 5

- [ ] T481 [US3] Confirm cutover implementation dependencies, owned files, and focused validation. Run host, live, hardware, or container checks only for changed surfaces.

### Group `wi:ADR-046-feasibility-and-spikes` (10 items)

- [ ] T482 [US3] `ADR046-feasibility-002` - `proofs/process-fastlaunch-spike/` (adapt)
- [ ] T483 [P] [US3] `ADR046-feasibility-003` - `proofs/effectport-async-spike/` (adapt)
- [ ] T484 [P] [US3] `ADR046-feasibility-004` - `proofs/provider-packaging-spike/` (adapt)
- [ ] T485 [P] [US3] `ADR046-feasibility-005` - `proofs/bus-routing-noise-spike/` (adapt)
- [ ] T486 [P] [US3] `ADR046-feasibility-006` - `proofs/provider-state-export-spike/` (adapt)
- [ ] T487 [P] [US3] `ADR046-feasibility-007` - `proofs/process-provider-conformance-spike/` (adapt)
- [ ] T488 [P] [US3] `ADR046-feasibility-008` - `proofs/nix-authoring-spike/` (adapt)
- [ ] T489 [P] [US3] `ADR046-feasibility-009` - `proofs/cli-discovery-spike/` (adapt)
- [ ] T490 [US3] `ADR046-feasibility-010` - `proofs/e2e-composition-spike/` (adapt)
- [ ] T491 [US3] `ADR046-feasibility-011` - `proofs/test-runtime-budget-spike/` (adapt)

### Group `wi:ADR-046-reset-and-cutover` (11 items)

- [ ] T492 [P] [US3] `ADR046-reset-001` - `packages/d2b-cutover/src/{inventory,snapshot,checkpoint}.rs` (adapt)
- [ ] T493 [US3] `ADR046-reset-002` - `packages/d2b-cutover/src/{bundle_validate,trust_preflight}.rs` (adapt)
- [ ] T494 [US3] `ADR046-reset-003` - `packages/d2b-cutover/src/{consent,drain,disposition}.rs` (adapt)
- [ ] T495 [US3] `ADR046-reset-004` - `packages/d2b-cutover/src/adopt.rs` (adapt)
- [ ] T496 [US3] `ADR046-reset-005` - `packages/d2b-cutover/src/{store_bootstrap,provider_sequence}.rs` (create)
- [ ] T497 [US3] `ADR046-reset-006` - `packages/d2b-cutover/src/{zonelink_cutover,guest_activation}.rs` (adapt)
- [ ] T498 [US3] `ADR046-reset-007` - `packages/d2b-cutover/src/{verify,doctor,degraded}.rs` (adapt)
- [ ] T499 [US3] `ADR046-reset-008` - `packages/d2b-cutover/src/finalize.rs` (create)
- [ ] T500 [US3] `ADR046-reset-009` - `packages/d2b-cutover/src/{journal,rollback,hold}.rs` (adapt)
- [ ] T501 [US3] `ADR046-reset-010` - `packages/d2b-cutover/src/reset_scope.rs` (adapt)
- [ ] T502 [US3] `ADR046-reset-011` - `tests/integration/live/cutover-real-host.sh` (create)

### Group `wi:ADR-046-security-and-threat-model` (19 items)

- [ ] T503 [US3] `ADR046-security-001` - `packages/d2b-contract-tests/tests/policy_telemetry_redaction.rs` (adapt)
- [ ] T504 [US3] `ADR046-security-002` - `packages/d2b-session/tests/noise_conformance.rs` (adapt)
- [ ] T505 [US3] `ADR046-security-003` - `packages/d2b-resource-store/tests/rbac_property.rs` (adapt)
- [ ] T506 [US3] `ADR046-security-004` - `packages/d2b-bus/fuzz/fuzz_targets/zonelink_frame.rs` (adapt)
- [ ] T507 [P] [US3] `ADR046-security-005` - `packages/xtask/src/effectport_boundary_check.rs` (adapt)
- [ ] T508 [US3] `ADR046-security-006` - `packages/d2b-provider-system-minijail/tests/launchticket_toctou.rs` (adapt)
- [ ] T509 [US3] `ADR046-security-007` - `packages/d2b-contract-tests/tests/quarantine_not_kill_matrix.rs` (adapt)
- [ ] T510 [US3] `ADR046-security-008` - `packages/d2b-provider-system-core/tests/no_isolation_propagation.rs` (adapt)
- [ ] T511 [US3] `ADR046-security-009` - `packages/d2b-provider-volume-local/tests/marker_tamper_fault_injection.rs` (adapt)
- [ ] T512 [US3] `ADR046-security-010` - `packages/d2b-contract-tests/tests/zero_secret_invariant.rs` (adapt)
- [ ] T513 [US3] `ADR046-security-011` - `packages/d2b-provider-{clipboard-wayland,shell-terminal,device-security-key,notification-desktop}/tests/stream_redaction.rs` (adapt)
- [ ] T514 [US3] `ADR046-security-012` - `packages/d2b-audit/tests/privileged_fail_closed.rs` (adapt)
- [ ] T515 [US3] `ADR046-security-013` - `packages/d2b-bus/tests/dos_ceiling_fault_injection.rs` (adapt)
- [ ] T516 [US3] `ADR046-security-014` - `packages/d2b/src/commands/{doctor,support_bundle}.rs` (adapt)
- [ ] T517 [US3] `ADR046-security-015` - `packages/d2b-core-controller/src/reset.rs` (adapt)
- [ ] T518 [P] [US3] `ADR046-security-016` - `tests/unit/gates/security-matrix-coverage.sh` (adapt)
- [ ] T519 [US3] `ADR046-security-017` - `tests/integration/containers/malicious-child-zone.rs` (adapt)
- [ ] T520 [P] [US3] `ADR046-security-018` - `docs/reference/security-manual-validation-checklist.md` (adapt)
- [ ] T521 [US3] `ADR046-security-019` - `packages/d2b-contract-tests/tests/minijail_process_ownership.rs` (adapt)

### Group `wi:ADR-046-streamline` (24 items)

- [ ] T527 [US3] `ADR046-streamline-006` - `packages/d2b-resource-store-redb/tests/provider_state_graph.rs` (or the eventual crate implementing Zone resource storage) (create)
- [ ] T528 [US3] `ADR046-streamline-007` - `packages/d2b-contract-tests/tests/policy_effectport_boundary.rs` (adapt)
- [ ] T535 [P] [US3] `ADR046-streamline-014` - `tests/tools/run-layer.sh` extension (this repository already has `tests/tools/run-layer.sh` and `layer1-jobs.py` bounded-parallelism precedent) plus fake `EffectPort`/`ResourceClient` stub crates under `packages/d2b-provider-toolkit-fakes/` (adapt)
- [ ] T542 [US3] `ADR046-streamline-021` - `packages/d2b-contract-tests/tests/policy_test_determinism.rs` (create)

- [ ] T548 [US3] Implement one hermetic recovery-point validator for FR-043. Decode bounded
  integer timestamps, use checked expiration arithmetic, validate every candidate/commit/tree,
  preview, host, operator, and restore-instruction binding, and fail on malformed, duplicate,
  stale, expired, skipped, ignored, or empty evidence.
- [ ] T580 [US3] Converge the cutover implementation and validate one external version-1
  recovery record before irreversible mutation. Run focused integration and conditional host,
  live, hardware, and reset/cutover tests against the exact release tree.
- [ ] T555 [US3] Validate cutover recovery and audited privilege across broker restart, daemon
  startup failure, peer identity change, mutation crash windows, export failure, and rollback.
  Confirm no raw identity or evaluator output escapes through human, JSON, wire, log, metric,
  span, audit, panic, or Debug surfaces.
- [ ] T556 [US3] Record the cutover result and release evidence only after focused validation
  passes. Verify the exact tree, changelog, generated outputs, and applicable CI results before
  publication; rerun affected checks when content changes.

**Checkpoint**: Cutover, recovery, security, and release requirements have focused evidence
for the exact implementation tree.

---

## Historical implementation grouping: Release friction closure

This is conditional release work. It addresses verified implementation friction that remains
against the product requirements and focused validation; it does not create retired delivery
state or an ADR gate.

- [ ] T557 [US4] Triage release friction by component and create focused implementation tasks
  with explicit owners and tests.
- [ ] T558 [US4] Confirm the changed files, validation map, and applicable conditional lanes
  before implementation begins.
- [ ] T559 [US4] Implement the triaged fixes on a clean release branch, run focused tests, and
  resolve all content-changing results.
- [ ] T560 [US4] Freeze the exact clean release tree only after the applicable release
  conditions, generated outputs, changelog, integration evidence, and CI agree.
- [ ] T561 [US4] Verify the merged release tree identity before tagging or publishing.

## Release phase: Publish d2b 3.0

**Story**: US4. Release work is conditional on the affected product and compatibility
surfaces, and uses focused evidence from the exact release tree.

- [ ] T562 [US4] Confirm the closing product contracts and their implementation evidence
  are complete for the release tree.
- [ ] T563 [US4] Run every DELETE and REPLACE removal proof against the exact release tree.
- [ ] T564 [US4] Run the applicable focused validation required by the changed surfaces,
  including conditional manual hardware, live-host, cloud, reset, and cutover checks.
- [ ] T568 [US4] Re-derive the desktop-companion set from repository inputs and public contract
  use; pin each companion revision and consumed surface, and update the inventory when needed.
- [ ] T569 [US4] Exercise each companion against the exact release tree on a live host. Every
  consumed surface must be Conformant or explicitly Retired under FR-063; Blocked or
  unclassifiable behavior holds release.
- [ ] T570 [US4] Confirm capability parity for every migration path that promises a successor.
- [ ] T571 [US4] Publish the explicit retirement list with justification, owner, restoration
  condition, and release-note entry for each lawful retirement (FR-042).
- [ ] T572 [US4] Confirm no foundation surface remains deliberately unwired from production.
- [ ] T566 [US4] Write the final release state: fold changelog fragments, set the 3.0.0 version
  consistently, select the explicit source fallback or a matching prebuilt manifest, and keep
  publication manual-only with identity verification against the merged release tree.
- [ ] T560 [US4] Freeze the exact clean release tree only after all release conditions, focused
  validation, integration/CI evidence, and generated outputs agree. Any content change
  invalidates prior evidence and requires the affected checks to run again.
- [ ] T561 [US4] Merge and publish only the verified release tree. Do not edit or regenerate
  content during publication; verify the merged tree identity before tagging or releasing.

## Implementation strategy

Keep public interfaces and generated outputs in lockstep with their owning implementation.
Prefer focused, hermetic tests and negative fixtures. Run broader container, host, live,
hardware, or performance checks only for changed components, and record deliberate omissions.
The release candidate must satisfy the product success criteria in spec.md, including recovery,
compatibility, capability parity, security, cutover, and operator-visible behavior.
