# Specification Coverage and Traceability

**Feature**: `001-adr046-d2b3-completion` | **Date**: 2026-07-29

## Why this file exists

The ADR-046 specification set is large and normative: 55 member specs and 545 work items, each
carrying exact destination paths, detailed design text, validation obligations, integration
notes, data-migration disposition, and a removal proof. **This plan must not summarize any of
that away.**

This file is the completeness proof. It is generated from the committed manifests, so it
cannot silently drift from them, and it accounts for every spec and every work item exactly
once.

### The binding rule

The manifests are authoritative. For every work item, these fields are **carried verbatim**
into `tasks.md` and into the implementing change; they are never paraphrased, condensed, or
selectively quoted:

| Field | Obligation it creates |
| --- | --- |
| `detailedDesign` | What must be built, including every named constant, algorithm, and prohibition |
| `validation` | The exact tests and evidence required before the item can be `Merged` |
| `destination` | The exact file paths the item may write |
| `integration` | How the item connects to the rest of the plane |
| `removalProof` | The proof required before any superseded path it replaces is deleted |
| `dataMigration` | The migration disposition for state the item touches |
| `currentSource` | The existing code the item adapts, replaces, or extracts from |
| `reuseAction` | One closed scalar governing how existing code may be reused |
| `dependencyOwner` | The item or owner that must land first |

Retrieve any item's full text with:

```bash
jq --arg id ADR046-routing-001 \
  '.items[] | select(.workItemId==$id)' \
  docs/specs/ADR-046-work-items.json
```

### Regeneration and verification

This file is derived. Re-derive and verify coverage with:

```bash
# 55 specs, 545 work items, and the per-wave split must all reconcile
jq -r '[.nodes[].kind] | group_by(.) | map("\(.[0]): \(length)") | .[]' \
  docs/specs/ADR-046-implementation-graph.json
jq -r '[.nodes[] | select(.kind=="work-item")] | group_by(.wave) |
  map("\(.[0].wave): \(length)") | .[]' docs/specs/ADR-046-implementation-graph.json
jq -r '[.items[].implementationState] | group_by(.) |
  map("\(.[0]): \(length)") | .[]' docs/specs/ADR-046-work-items.json
```

A `tasks.md` that does not cover every `Planned` work-item id listed below is incomplete by
definition.

---

## Reconciliation

| Metric | Value | Source |
| --- | --- | --- |
| Member specs | 55 | implementation graph, `kind == "spec"` |
| Work items | 545 | implementation graph and work-item manifest agree |
| Merged | 14 | W0 (8) + W1 (6) |
| Planned | 531 | the scope of this program |
| Graph nodes / edges | 600 / 1949 | 55 + 545 |
| Max topological rank | 22 | |

### Work items by wave

| Wave | Specs | Work items | Cumulative | Status |
| --- | --- | --- | --- | --- |
| W0 | 6 | 8 | 8 | Merged, delivered under waiver (FR-034) |
| W1 | 2 | 6 | 14 | Merged, delivered under waiver (FR-034) |
| W2 | 2 | 19 | 33 | Ready to launch |
| W3 | 1 | 4 | 37 | Serial; gates every Provider dossier |
| W4 | 5 | 32 | 69 | |
| W5 | 7 | 146 | 215 | Carries the corrected store engine (RK-1) |
| W6 | 27 | 257 | 472 | Largest wave; 5 file-disjoint families |
| W7 | 5 | 73 | 545 | Destructive cutover |
| W8 | 0 | recorded at W7 close | 545+ | Terminal; release gate evaluated here |

### Reuse disposition across all 545 items

| `reuseAction` | Count | Meaning for planning |
| --- | --- | --- |
| `adapt` | 391 | Existing code is modified in place or ported |
| `create` | 112 | Net-new code |
| `replace` | 22 | Successor written, predecessor deleted after its removal proof |
| `delete-after-cutover` | 12 | Removal only, gated on cutover |
| `copy-unchanged` | 4 | Verbatim copy |
| `extract` | 3 | Pulled out of an existing module |
| `wrap` | 1 | Wrapped rather than modified |

Every one of the 545 items carries a non-empty `removalProof`, so FR-023's per-path proof
obligation is already itemized in the manifest rather than needing to be invented.

---
## Spec index (all 55)

Each spec's work items are listed in the per-wave sections below.

| Spec | Wave | Work items | Parallel group |
| --- | --- | --- | --- |
| `ADR-046-current-code-migration-map` | W0 | 0 | W0-reference-docs |
| `ADR-046-decision-register` | W0 | 1 | W0-reference-docs |
| `ADR-046-resource-api-and-authorization` | W0 | 2 | W0-foundation-chain |
| `ADR-046-resource-object-model` | W0 | 2 | W0-foundation-chain |
| `ADR-046-resource-store-redb` | W0 | 5 | W0-foundation-chain |
| `ADR-046-terminology-and-identities` | W0 | 2 | W0-foundation-chain |
| `ADR-046-componentsession-and-bus` | W1 | 3 | W1-reconcile-and-bus |
| `ADR-046-resource-reconciliation` | W1 | 3 | W1-reconcile-and-bus |
| `ADR-046-primitive-resource-composition` | W2 | 3 | W2-composition-and-routing |
| `ADR-046-zone-routing` | W2 | 16 | W2-composition-and-routing |
| `ADR-046-provider-model-and-packaging` | W3 | 4 | W3-provider-contract |
| `ADR-046-components-processes-and-sandbox` | W4 | 2 | W4-parallel-specs |
| `ADR-046-core-controllers` | W4 | 2 | W4-parallel-specs |
| `ADR-046-provider-state` | W4 | 12 | W4-parallel-specs |
| `ADR-046-resources-credential` | W4 | 8 | W4-parallel-specs |
| `ADR-046-resources-network` | W4 | 9 | W4-parallel-specs |
| `ADR-046-cli-and-operations` | W5 | 13 | W5-parallel-specs |
| `ADR-046-nix-configuration` | W5 | 35 | W5-parallel-specs |
| `ADR-046-resources-device` | W5 | 8 | W5-parallel-specs |
| `ADR-046-resources-host-guest-process-user` | W5 | 24 | W5-parallel-specs |
| `ADR-046-resources-volume` | W5 | 6 | W5-parallel-specs |
| `ADR-046-resources-zone-control` | W5 | 28 | W5-parallel-specs |
| `ADR-046-telemetry-audit-and-support` | W5 | 27 | W5-parallel-specs |
| `ADR-046-provider-activation-nixos` | W6 | 7 | W6-transport-observability-activation |
| `ADR-046-provider-audio-pipewire` | W6 | 13 | W6-interaction |
| `ADR-046-provider-clipboard-wayland` | W6 | 12 | W6-interaction |
| `ADR-046-provider-credential-entra` | W6 | 1 | W6-credentials |
| `ADR-046-provider-credential-managed-identity` | W6 | 5 | W6-credentials |
| `ADR-046-provider-credential-secret-service` | W6 | 6 | W6-credentials |
| `ADR-046-provider-device-gpu` | W6 | 9 | W6-storage-network-device |
| `ADR-046-provider-device-security-key` | W6 | 35 | W6-storage-network-device |
| `ADR-046-provider-device-tpm` | W6 | 13 | W6-storage-network-device |
| `ADR-046-provider-device-usbip` | W6 | 9 | W6-storage-network-device |
| `ADR-046-provider-display-wayland` | W6 | 4 | W6-interaction |
| `ADR-046-provider-network-local` | W6 | 20 | W6-storage-network-device |
| `ADR-046-provider-notification-desktop` | W6 | 6 | W6-interaction |
| `ADR-046-provider-observability-otel` | W6 | 6 | W6-transport-observability-activation |
| `ADR-046-provider-runtime-azure-container-apps` | W6 | 7 | W6-system-host-guest |
| `ADR-046-provider-runtime-azure-virtual-machine` | W6 | 9 | W6-system-host-guest |
| `ADR-046-provider-runtime-cloud-hypervisor` | W6 | 7 | W6-system-host-guest |
| `ADR-046-provider-runtime-qemu-media` | W6 | 19 | W6-system-host-guest |
| `ADR-046-provider-shell-terminal` | W6 | 13 | W6-interaction |
| `ADR-046-provider-system-core` | W6 | 1 | W6-system-host-guest |
| `ADR-046-provider-system-minijail` | W6 | 6 | W6-system-host-guest |
| `ADR-046-provider-system-systemd` | W6 | 3 | W6-system-host-guest |
| `ADR-046-provider-transport-azure-relay` | W6 | 7 | W6-transport-observability-activation |
| `ADR-046-provider-transport-unix` | W6 | 11 | W6-transport-observability-activation |
| `ADR-046-provider-transport-vsock` | W6 | 7 | W6-transport-observability-activation |
| `ADR-046-provider-volume-local` | W6 | 13 | W6-storage-network-device |
| `ADR-046-provider-volume-virtiofs` | W6 | 7 | W6-storage-network-device |
| `ADR-046-feasibility-and-spikes` | W7 | 11 | W7-closing |
| `ADR-046-reset-and-cutover` | W7 | 11 | W7-closing |
| `ADR-046-security-and-threat-model` | W7 | 19 | W7-closing |
| `ADR-046-streamline` | W7 | 24 | W7-closing |
| `ADR-046-validation-and-delivery` | W7 | 9 | W7-closing |

---

## Complete work-item enumeration (all 545)

Grouped by wave, then spec. `Dest` is the first destination path; the manifest holds the
full list plus `detailedDesign`, `validation`, `integration`, `dataMigration`,
`currentSource`, and `removalProof` for each item. Those fields are the authoritative
obligation and are carried verbatim into tasks.

### W0 - 8 work items

| Work item | Spec | State | Reuse | Dest (first path) |
| --- | --- | --- | --- | --- |
| `ADR046-api-001` | `resource-api-and-authorization` | Merged | adapt | `packages/d2b-contracts/proto/d2b-resource-v3.proto` |
| `ADR046-api-002` | `resource-api-and-authorization` | Merged | adapt | `packages/d2b-resource-api/src/authz.rs` |
| `ADR046-decisions-001` | `decision-register` | Merged | adapt | `docs/specs/ADR-046-decision-register.md` |
| `ADR046-identities-001` | `terminology-and-identities` | Merged | adapt | `packages/d2b-contracts/src/v3/identity.rs` |
| `ADR046-identities-002` | `terminology-and-identities` | Merged | adapt | `nixos-modules/options-zones.nix` |
| `ADR046-object-001` | `resource-object-model` | Merged | adapt | `packages/d2b-contracts/src/v3/resource.rs` |
| `ADR046-object-002` | `resource-object-model` | Merged | adapt | `packages/d2b-resource-store-redb/src/ownership.rs` |
| `ADR046-store-001` | `resource-store-redb` | Merged | adapt | `packages/d2b-contracts/src/v3/error.rs` |

### W1 - 6 work items

| Work item | Spec | State | Reuse | Dest (first path) |
| --- | --- | --- | --- | --- |
| `ADR046-bus-001` | `componentsession-and-bus` | Merged | adapt | `packages/d2b-bus/src/{router |
| `ADR046-feasibility-001` | `feasibility-and-spikes` | Merged | adapt | `proofs/redb-resource-store-spike/` |
| `ADR046-reconcile-001` | `resource-reconciliation` | Merged | adapt | `packages/d2b-controller-toolkit/src/lib.rs` |
| `ADR046-reconcile-002` | `resource-reconciliation` | Merged | adapt | `packages/d2b-core-controller/src/hints.rs` |
| `ADR046-session-001` | `componentsession-and-bus` | Merged | adapt | `packages/d2b-contracts/src/v3/component_session.rs` |
| `ADR046-session-002` | `componentsession-and-bus` | Merged | adapt | `packages/d2b-session-unix/` |

### W2 - 19 work items

| Work item | Spec | State | Reuse | Dest (first path) |
| --- | --- | --- | --- | --- |
| `ADR046-primitives-001` | `primitive-resource-composition` | Planned | adapt | `packages/d2b-contracts/src/v3/host.rs` |
| `ADR046-primitives-002` | `primitive-resource-composition` | Planned | adapt | `packages/d2b-provider-system-systemd/` |
| `ADR046-primitives-003` | `primitive-resource-composition` | Planned | adapt | `packages/d2b-provider-volume-*/` |
| `ADR046-routing-001` | `zone-routing` | Planned | adapt | `packages/d2b-contracts/src/v3/zone_routing.rs` |
| `ADR046-routing-002` | `zone-routing` | Planned | adapt | `packages/d2b-zone-routing/src/engine.rs` |
| `ADR046-routing-003` | `zone-routing` | Planned | adapt | `packages/d2b-zone-routing/src/resolver.rs` (ZoneEntrypointResolver) |
| `ADR046-routing-004` | `zone-routing` | Planned | adapt | `packages/d2b-core-controller/src/zone_links.rs` |
| `ADR046-routing-005` | `zone-routing` | Planned | adapt | `packages/d2b-bus/src/zone_route.rs` (cross-Zone bus routing) |
| `ADR046-routing-006` | `zone-routing` | Planned | adapt | `packages/d2b-zone-routing/tests/route_engine_vectors.rs` |
| `ADR046-routing-007` | `zone-routing` | Planned | adapt | `packages/d2b-bus/src/session/` |
| `ADR046-routing-008` | `zone-routing` | Planned | adapt | `packages/d2b-bus/src/transport/unix.rs` |
| `ADR046-routing-009` | `zone-routing` | Planned | adapt | `packages/d2b-contracts/src/v3/zone_session.rs` |
| `ADR046-routing-010` | `zone-routing` | Planned | adapt | `packages/d2b-resource-client/` |
| `ADR046-routing-011` | `zone-routing` | Planned | adapt | `nixos-modules/options-zones.nix` (new structural base) |
| `ADR046-routing-012` | `zone-routing` | Planned | adapt | `nixos-modules/zone-resources-json.nix` (new) |
| `ADR046-routing-013` | `zone-routing` | Planned | adapt | `packages/d2b-core-controller/src/configuration.rs` (defined by ADR-046-core-controllers) |
| `ADR046-routing-014` | `zone-routing` | Planned | adapt | `packages/d2b-provider/src/` (adapted in place) |
| `ADR046-routing-015` | `zone-routing` | Planned | adapt | `packages/d2b-provider-toolkit/src/` (adapted in place) |
| `ADR046-routing-016` | `zone-routing` | Planned | adapt | `packages/d2b-zone-routing/src/service.rs` |

### W3 - 4 work items

| Work item | Spec | State | Reuse | Dest (first path) |
| --- | --- | --- | --- | --- |
| `ADR046-provider-001` | `provider-model-and-packaging` | Planned | adapt | `packages/d2b-contracts/src/v3/provider.rs` |
| `ADR046-provider-002` | `provider-model-and-packaging` | Planned | adapt | one `packages/d2b-provider-<base>-<implementation>/` per Provider with mandatory src/ |
| `ADR046-provider-003` | `provider-model-and-packaging` | Planned | adapt | `packages/d2b-provider-system-core/` |
| `ADR046-provider-004` | `provider-model-and-packaging` | Planned | create | `packages/d2b-contracts/src/v3/semantic_services/{mod |

### W4 - 32 work items

| Work item | Spec | State | Reuse | Dest (first path) |
| --- | --- | --- | --- | --- |
| `ADR046-core-001` | `core-controllers` | Planned | adapt | `packages/d2b-core-controller/src/{main |
| `ADR046-credential-001` | `resources-credential` | Planned | adapt | `packages/d2b-contracts/src/v3/credential.rs` |
| `ADR046-credential-002` | `resources-credential` | Planned | adapt | `packages/d2b-contracts/proto/v3/credential.proto` |
| `ADR046-credential-003` | `resources-credential` | Planned | adapt | `packages/d2b-provider-credential-secret-service/src/{lib.rs |
| `ADR046-credential-004` | `resources-credential` | Planned | adapt | `packages/d2b-provider-credential-entra/src/{lib.rs |
| `ADR046-credential-005` | `resources-credential` | Planned | adapt | `packages/d2b-provider-credential-managed-identity/src/{lib.rs |
| `ADR046-credential-006` | `resources-credential` | Planned | adapt | `packages/d2b-provider-credential-<impl>/src/controller.rs` |
| `ADR046-credential-007` | `resources-credential` | Planned | adapt | `nixos-modules/options-resources.nix` (generic schema-derived resource options |
| `ADR046-credential-008` | `resources-credential` | Planned | adapt | `packages/d2b-provider-credential-<impl>/src/audit.rs` |
| `ADR046-network-001` | `resources-network` | Planned | adapt | `packages/d2b-contracts/src/v3/network.rs`: NetworkSpec |
| `ADR046-network-002` | `resources-network` | Planned | adapt | `packages/d2b-provider-network-local/src/ifname.rs` |
| `ADR046-network-003` | `resources-network` | Planned | adapt | `packages/d2b-provider-network-local/` - artifact catalog integration for net-VM nixos-system artifact resolution |
| `ADR046-network-004` | `resources-network` | Planned | adapt | `nixos-modules/resources-network.nix`: Nix resource object emitter for Network ResourceType |
| `ADR046-network-005` | `resources-network` | Planned | adapt | `packages/d2b-provider-network-local/src/controller.rs`: async NetworkReconciler |
| `ADR046-network-006` | `resources-network` | Planned | adapt | `tests/unit/nix/cases/net-vm-network.nix` (adapted to v3 resource API) |
| `ADR046-network-007` | `resources-network` | Planned | adapt | `Provider/device-usbip` owns one relay Process/Endpoint authority per Network and calls the typed UsbipEffectPort for the shared closed `ApplyNftablesProjection` request with closed action enum `Apply/Remove` |
| `ADR046-network-008` | `resources-network` | Planned | create | `packages/d2b-core-controller/src/configuration.rs`: bundle application |
| `ADR046-network-009` | `resources-network` | Planned | adapt | `packages/d2b-contracts/src/v3/network.rs` external-attachment sharing schema/status |
| `ADR046-process-001` | `components-processes-and-sandbox` | Planned | adapt | `packages/d2b-process/src/` |
| `ADR046-process-002` | `components-processes-and-sandbox` | Planned | adapt | `packages/d2b-provider-system-systemd/` |
| `ADR046-pstate-001` | `provider-state` | Planned | adapt | `packages/d2b-contracts/src/v3/volume_state.rs` |
| `ADR046-pstate-002` | `provider-state` | Planned | adapt | `packages/d2b-contracts/src/v3/provider.rs` (component descriptor `stateNamespaces` field) |
| `ADR046-pstate-003` | `provider-state` | Planned | adapt | `packages/d2b-provider-volume-local/` (new crate |
| `ADR046-pstate-004` | `provider-state` | Planned | adapt | `packages/d2b-provider-volume-local/src/migration.rs` |
| `ADR046-pstate-005` | `provider-state` | Planned | adapt | `packages/d2b-provider-volume-local/src/sealing.rs` |
| `ADR046-pstate-006` | `provider-state` | Planned | adapt | `packages/d2b-provider-volume-local/src/snapshot.rs` |
| `ADR046-pstate-007` | `provider-state` | Planned | adapt | `packages/d2b-provider-volume-local/src/relocation.rs` |
| `ADR046-pstate-008` | `provider-state` | Planned | adapt | `packages/d2b-provider-volume-local/src/audit.rs` |
| `ADR046-pstate-009` | `provider-state` | Planned | adapt | `packages/d2b-provider-volume-local/tests/state.rs` (ported hermetic atomic/lock/quarantine/lease tests) |
| `ADR046-pstate-010` | `provider-state` | Planned | adapt | `nixos-modules/zone-resources.nix` (per-Zone bundle emitter NixOS module) |
| `ADR046-pstate-011` | `provider-state` | Planned | adapt | `packages/xtask/src/provider_crate_policy.rs` |
| `ADR046-pstate-012` | `provider-state` | Planned | adapt | `packages/d2b-core-controller/src/optional_state_admission.rs` (storage-need admission: reject a declared namespace whose payload is derivable from spec/status/core ledger/external observation with `component-state-not-justified` |

### W5 - 146 work items

| Work item | Spec | State | Reuse | Dest (first path) |
| --- | --- | --- | --- | --- |
| `ADR046-audit-001` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-audit/src/{hash_chain.rs |
| `ADR046-audit-002` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-resource-store-redb/src/audit.rs` |
| `ADR046-audit-003` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-session/src/audit.rs` |
| `ADR046-audit-004` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b/src/zone_audit.rs` (new `d2b zone audit export` subcommand) |
| `ADR046-cli-001` | `cli-and-operations` | Planned | adapt | `packages/d2b/src/lib.rs` |
| `ADR046-cli-002` | `cli-and-operations` | Planned | adapt | `packages/d2b/src/guest.rs` (`d2b guest start/stop/restart/list/status`) |
| `ADR046-cli-003` | `cli-and-operations` | Planned | adapt | `packages/d2b/src/exec.rs` (`d2b exec run/attach/wait/status/list/logs/kill`) |
| `ADR046-cli-004` | `cli-and-operations` | Planned | adapt | `packages/d2b/src/shell.rs` (`d2b shell open/attach/list/detach/kill/status`) |
| `ADR046-cli-005` | `cli-and-operations` | Planned | adapt | `packages/d2b/src/provider.rs` (`d2b provider list/get/status/inspect` |
| `ADR046-cli-006` | `cli-and-operations` | Planned | adapt | `packages/d2b/src/complete.rs` (`d2b complete bash/zsh/fish`) |
| `ADR046-cli-007` | `cli-and-operations` | Planned | adapt | `packages/d2b/src/activation.rs` (`d2b activation build/generations/switch/boot/test/rollback/gc/migrate/keys/trust/rotate-known-host/config`) |
| `ADR046-cli-008` | `cli-and-operations` | Planned | adapt | `packages/d2b/src/host.rs` (all `d2b host` subcommands) |
| `ADR046-cli-009` | `cli-and-operations` | Planned | adapt | `packages/d2b/src/zone.rs` (`d2b zone get/list/status`) |
| `ADR046-cli-010` | `cli-and-operations` | Planned | adapt | `packages/d2b/src/resource.rs` (standard `d2b get/list/watch/create/update-spec/delete/status` top-level verbs) |
| `ADR046-cli-011` | `cli-and-operations` | Planned | replace | Nix: `nixos-modules/options-zones.nix` (unified `d2b.zones.<zone>.resources` attrset |
| `ADR046-cli-012` | `cli-and-operations` | Planned | adapt | `packages/d2b/src/endpoint.rs` (`d2b endpoint get/list/watch/status/resolve`) |
| `ADR046-cli-013` | `cli-and-operations` | Planned | adapt | `packages/d2b/src/share.rs` (`d2b export …` and `d2b import …` nouns) |
| `ADR046-client-001` | `resources-zone-control` | Planned | adapt | `packages/d2b-client/src/` (updated for v3 Zone API |
| `ADR046-device-001` | `resources-device` | Planned | adapt | `packages/d2b-contracts/src/v3/device.rs` |
| `ADR046-device-002` | `resources-device` | Planned | adapt | `packages/d2b-provider-device-tpm/src/` (controller |
| `ADR046-device-003` | `resources-device` | Planned | adapt | `packages/d2b-provider-device-usbip/src/` (controller |
| `ADR046-device-004` | `resources-device` | Planned | adapt | `packages/d2b-provider-device-security-key/src/` (controller |
| `ADR046-device-005` | `resources-device` | Planned | adapt | `packages/d2b-provider-device-gpu/src/` (controller |
| `ADR046-device-006` | `resources-device` | Planned | adapt | `nixos-modules/resources-device.nix` |
| `ADR046-device-007` | `resources-device` | Planned | create | `packages/d2b-core-controller/src/configuration.rs` |
| `ADR046-device-008` | `resources-device` | Planned | adapt | `packages/xtask/src/main.rs` (`check-provider-layout` subcommand) |
| `ADR046-doctor-001` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b/src/zone_doctor.rs` |
| `ADR046-doctor-002` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b/src/zone_support_bundle.rs` |
| `ADR046-exec-001` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-contracts/src/v3/host.rs` |
| `ADR046-exec-002` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-contracts/src/v3/process_provider.rs`: LaunchTicket |
| `ADR046-exec-003` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-provider-system-core/src/host.rs`: HostReconciler |
| `ADR046-exec-004` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-provider-system-core/src/user.rs`: UserReconciler |
| `ADR046-exec-005` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-provider-system-core/src/host.rs` (continued) |
| `ADR046-exec-006` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-provider-system-systemd/src/`: launch.rs (opaque EffectPort requests) |
| `ADR046-exec-007` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-provider-system-minijail/src/`: sandbox_compiler.rs |
| `ADR046-exec-008` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-process-conformance/src/`: shared conformance test matrix run against both system-systemd and system-minijail providers |
| `ADR046-exec-009` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-provider-system-core/src/host.rs` (user-only no-isolation Host) |
| `ADR046-exec-010` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-provider-system-systemd/src/guest_exec.rs` (guest-domain EphemeralProcess launch via systemd-run inside guest) |
| `ADR046-exec-011` | `resources-host-guest-process-user` | Planned | adapt | guest-domain process attachment becomes a ComponentSession named stream to the EphemeralProcess running in the guest |
| `ADR046-exec-012` | `resources-host-guest-process-user` | Planned | adapt | `nixos-modules/options-zones.nix`: `d2b.zones.<zone>.resources` option as `types.attrsOf (types.submodule resourceModule)` where each resource module has `type` (required enum) |
| `ADR046-exec-013` | `resources-host-guest-process-user` | Planned | create | `packages/d2b-core-controller/src/cleanup.rs`: EphemeralProcess TTL cleanup controller handler |
| `ADR046-exec-014` | `resources-host-guest-process-user` | Planned | adapt | `nixos-modules/zone-bundle.nix`: Zone resource bundle emitter |
| `ADR046-exec-015` | `resources-host-guest-process-user` | Planned | create | `packages/d2b-core-controller/src/configuration.rs`: `ZoneConfigController` |
| `ADR046-exec-016` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-bus-session/src/`: all above modules verbatim |
| `ADR046-exec-017` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-bus-session-unix/src/`: all above modules verbatim |
| `ADR046-exec-018` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-bus-wire/src/session.rs`: v3 bus protocol constants and wire types |
| `ADR046-exec-019` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-provider-runtime/src/`: `registry.rs` |
| `ADR046-exec-020` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-provider-toolkit/src/`: retain all modules verbatim |
| `ADR046-exec-021` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-bus-contracts/src/generated_v3_services/`: v3 generated ttrpc stubs for Zone service methods (Resource CRUD |
| `ADR046-exec-022` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-bus-client/src/`: all above modules |
| `ADR046-exec-023` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-zone-router/src/`: `router.rs` (v3 `ZoneOperationRouter` - idempotency semantics copied verbatim |
| `ADR046-host-posture-001` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-provider-system-core/src/{host_reconciler.rs |
| `ADR046-nix-001` | `nix-configuration` | Planned | adapt | `nixos-modules/options-zones.nix` (Zone-level options: `label` |
| `ADR046-nix-002` | `nix-configuration` | Planned | adapt | `Network` resource fields in `nixos-modules/options-zones-resources.nix` |
| `ADR046-nix-003` | `nix-configuration` | Planned | adapt | `nixos-modules/options-site.nix` (retained) |
| `ADR046-nix-004` | `nix-configuration` | Planned | adapt | `nixos-modules/index.nix` (rewritten) |
| `ADR046-nix-005` | `nix-configuration` | Planned | adapt | `nixos-modules/bundle-zones.nix` (per-Zone bundle derivation) |
| `ADR046-nix-006` | `nix-configuration` | Planned | adapt | `nixos-modules/resources-zones-processes.nix` |
| `ADR046-nix-007` | `nix-configuration` | Planned | adapt | `nixos-modules/resources-zones-volumes.nix` |
| `ADR046-nix-008` | `nix-configuration` | Planned | adapt | Compiler-only `parentZone` map in `nixos-modules/options-zones.nix` |
| `ADR046-nix-009` | `nix-configuration` | Planned | adapt | Provider/display-wayland and Provider/shell-terminal Process configs in `zones/<z>/resource-bundle.json` |
| `ADR046-nix-010` | `nix-configuration` | Planned | adapt | User-only `Host` resource in `zones/<z>/resource-bundle.json` (`spec.isolationPosture: "none"` |
| `ADR046-nix-011` | `nix-configuration` | Planned | copy-unchanged | `nixos-modules/privileges-json.nix` (retained) |
| `ADR046-nix-012` | `nix-configuration` | Planned | adapt | `nixos-modules/closures-json.nix` (rewritten |
| `ADR046-nix-013` | `nix-configuration` | Planned | replace | Per-Zone `zones/<z>/resource-bundle.json` (`schemaVersion`) |
| `ADR046-nix-014` | `nix-configuration` | Planned | adapt | `nixos-modules/assertions.nix` |
| `ADR046-nix-015` | `nix-configuration` | Planned | adapt | Same files |
| `ADR046-nix-016` | `nix-configuration` | Planned | copy-unchanged | Network reconciliation by `Provider/network-local` Process resources |
| `ADR046-nix-017` | `nix-configuration` | Planned | copy-unchanged | Per-VM store reconciliation by `Provider/volume-virtiofs` EphemeralProcess/Process resources |
| `ADR046-nix-018` | `nix-configuration` | Planned | replace | `Provider/device-tpm` |
| `ADR046-nix-019` | `nix-configuration` | Planned | adapt | `docs/reference/schemas/v3/<ResourceType>.json` for each ResourceType |
| `ADR046-nix-020` | `nix-configuration` | Planned | create | Configuration-publication controller handler in `packages/d2b-core-controller/src/configuration.rs` |
| `ADR046-nix-021` | `nix-configuration` | Planned | create | `packages/d2b-contract-tests/tests/provider-crate-layout.rs` |
| `ADR046-nix-022` | `nix-configuration` | Planned | create | `nixos-modules/artifact-catalog.nix` (new emitter) |
| `ADR046-nix-023` | `nix-configuration` | Planned | adapt | `packages/d2b-bus/src/session/` (new crate `d2b-bus`) |
| `ADR046-nix-024` | `nix-configuration` | Planned | adapt | `packages/d2b-bus/src/session/` (same crate as ADR046-nix-023). |
| `ADR046-nix-025` | `nix-configuration` | Planned | adapt | `packages/d2b-bus/src/session/`. |
| `ADR046-nix-026` | `nix-configuration` | Planned | adapt | `packages/d2b-bus/src/transport/unix/`. |
| `ADR046-nix-027` | `nix-configuration` | Planned | adapt | `packages/d2b-contracts/src/v3/component_session.rs`. |
| `ADR046-nix-028` | `nix-configuration` | Planned | adapt | `packages/d2b-contracts/src/v3/services/`. |
| `ADR046-nix-029` | `nix-configuration` | Planned | adapt | `packages/d2b-provider/src/` (adapt in place). |
| `ADR046-nix-030` | `nix-configuration` | Planned | adapt | `packages/d2b-provider-toolkit/src/` (adapt in place). |
| `ADR046-nix-031` | `nix-configuration` | Planned | create | `nixos-modules/resources-sharing.nix` |
| `ADR046-nix-032` | `nix-configuration` | Planned | adapt | `packages/d2b-client/src/` (adapt in place). |
| `ADR046-nix-033` | `nix-configuration` | Planned | adapt | `packages/d2b-bus/src/routing/zone_service.rs`. |
| `ADR046-nix-034` | `nix-configuration` | Planned | adapt | `packages/d2bd/src/provider_registry.rs` (adapt in place). |
| `ADR046-nix-035` | `nix-configuration` | Planned | adapt | `packages/d2bd/src/provider_effects.rs` (adapt in place). |
| `ADR046-pkg-001` | `resources-zone-control` | Planned | create | `packages/d2b-contract-tests/tests/policy_provider_crate_layout.rs` (new file |
| `ADR046-provider-agent-001` | `resources-zone-control` | Planned | adapt | `packages/d2b-provider/src/agent.rs` (v3 provider agent dispatch) |
| `ADR046-reconcile-003` | `resource-reconciliation` | Planned | adapt | `packages/d2b-controller-toolkit/benches/reaction.rs` |
| `ADR046-reuse-001` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-session/` copied verbatim |
| `ADR046-reuse-002` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-session-unix/` copied verbatim. |
| `ADR046-reuse-003` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-client/` copied |
| `ADR046-reuse-004` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-provider/` and `packages/d2b-provider-toolkit/` copied with v3 session admission and bus routing adaptations. |
| `ADR046-reuse-005` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-provider-observability-otel/src/agent.rs` adapted |
| `ADR046-reuse-006` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-bus/src/routing.rs` adapted from `service_v2.rs` |
| `ADR046-reuse-007` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-bus/src/service_router.rs` and `packages/d2b-core-controller/src/provider_effects.rs`. |
| `ADR046-reuse-008` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-contract-tests/tests/component_session_v2_vectors.rs` and `tests/noise_vectors.rs` copied verbatim. |
| `ADR046-reuse-009` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-telemetry/src/session_metrics_sink.rs`. |
| `ADR046-store-002` | `resource-store-redb` | Planned | adapt | `packages/d2b-resource-store-redb/src/revision_log.rs` |
| `ADR046-store-003` | `resource-store-redb` | Planned | adapt | `packages/d2b-contracts/src/v3/storage.rs` |
| `ADR046-store-004` | `resource-store-redb` | Planned | adapt | `packages/d2b-resource-store-redb/src/lib.rs` |
| `ADR046-store-005` | `resource-store-redb` | Planned | adapt | `packages/d2b-resource-store-redb/src/backup.rs` |
| `ADR046-telem-001` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-telemetry/src/{trace_context.rs |
| `ADR046-telem-002` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-resource-store-redb/src/metrics.rs` |
| `ADR046-telem-003` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-resource-api/src/metrics.rs` |
| `ADR046-telem-004` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-core-controller/src/metrics.rs` |
| `ADR046-telem-005` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-provider-supervisor/src/metrics.rs` |
| `ADR046-telem-006` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-provider-observability-otel/src/` |
| `ADR046-telem-007` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-provider-observability-otel/src/nix/journald.nix` (new Nix fragment) |
| `ADR046-telem-008` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-contract-tests/tests/policy_telemetry_redaction.rs` (new) |
| `ADR046-telem-009` | `telemetry-audit-and-support` | Planned | adapt | `nixos-modules/resources.nix` (uniform `d2b.zones.<zone>.resources` schema-aware option |
| `ADR046-telem-010` | `telemetry-audit-and-support` | Planned | adapt | `nixos-modules/resources-bundle.nix` (build-time validation step 4 in the `resources-bundle` derivation) |
| `ADR046-telem-011` | `telemetry-audit-and-support` | Planned | adapt | `packages/d2b-core-controller/src/{configuration.rs |
| `ADR046-user-session-001` | `resources-host-guest-process-user` | Planned | adapt | `packages/d2b-core-controller/src/user_session_authority.rs` (or a core/user-agent per-session agent Process under `Provider/system-systemd`) |
| `ADR046-volume-001` | `resources-volume` | Planned | adapt | `packages/d2b-contracts/src/v3/volume.rs` |
| `ADR046-volume-002` | `resources-volume` | Planned | adapt | `packages/d2b-provider-volume-local/src/` (layout engine |
| `ADR046-volume-003` | `resources-volume` | Planned | adapt | `packages/d2b-provider-volume-virtiofs/src/` (controller |
| `ADR046-volume-004` | `resources-volume` | Planned | adapt | `nixos-modules/resources-volume.nix` |
| `ADR046-volume-005` | `resources-volume` | Planned | create | `packages/d2b-provider-volume-local/src/` (block-image |
| `ADR046-volume-006` | `resources-volume` | Planned | create | `nixos-modules/resources-volume.nix` (Nix eval-time schema validation |
| `ADR046-wire-001` | `resources-zone-control` | Planned | adapt | `packages/d2b-contracts/src/v3/{services |
| `ADR046-zone-control-001` | `resources-zone-control` | Planned | adapt | `packages/d2b-contracts/src/v3/zone.rs` |
| `ADR046-zone-control-002` | `resources-zone-control` | Planned | adapt | `packages/d2b-contracts/src/v3/zone_link.rs` |
| `ADR046-zone-control-003` | `resources-zone-control` | Planned | adapt | `packages/d2b-contracts/src/v3/provider.rs` |
| `ADR046-zone-control-004` | `resources-zone-control` | Planned | adapt | `packages/d2b-contracts/src/v3/role.rs` |
| `ADR046-zone-control-005` | `resources-zone-control` | Planned | adapt | `packages/d2b-contracts/src/v3/role_binding.rs` |
| `ADR046-zone-control-006` | `resources-zone-control` | Planned | adapt | `packages/d2b-resource-api/src/authz.rs` |
| `ADR046-zone-control-007` | `resources-zone-control` | Planned | adapt | `nixos-modules/options-zones.nix` |
| `ADR046-zone-control-008` | `resources-zone-control` | Planned | adapt | `packages/d2b-contracts/src/v3/host.rs` (Host resource schema |
| `ADR046-zone-control-009` | `resources-zone-control` | Planned | create | `packages/d2b-contracts/src/v3/quota.rs` |
| `ADR046-zone-control-010` | `resources-zone-control` | Planned | create | `packages/d2b-contracts/src/v3/emergency_policy.rs` |
| `ADR046-zone-control-011` | `resources-zone-control` | Planned | adapt | `packages/d2b-bus/src/{lifecycle |
| `ADR046-zone-control-012` | `resources-zone-control` | Planned | adapt | `packages/d2b-bus-unix/src/{adapter |
| `ADR046-zone-control-013` | `resources-zone-control` | Planned | adapt | `packages/d2b-contracts/src/v3/component_session.rs` (new v3 namespace in existing contracts crate) |
| `ADR046-zone-control-014` | `resources-zone-control` | Planned | create | `nixos-modules/options-zones.nix` |
| `ADR046-zone-control-015` | `resources-zone-control` | Planned | create | `packages/d2b-resource-compiler/src/{main |
| `ADR046-zone-control-016` | `resources-zone-control` | Planned | adapt | `packages/d2b-core-controller/src/configuration.rs` (Phase 3 activation |
| `ADR046-zone-control-017` | `resources-zone-control` | Planned | adapt | `packages/d2b-provider/src/{registry |
| `ADR046-zone-control-018` | `resources-zone-control` | Planned | adapt | `packages/d2b-core-controller/src/zone_link.rs` (ZoneLink handler) |
| `ADR046-zone-control-019` | `resources-zone-control` | Planned | adapt | `packages/d2b-contracts/src/v3/{resource_export |
| `ADR046-zone-control-020` | `resources-zone-control` | Planned | create | `packages/d2b-core-controller/src/export_import_projection.rs` (local qualified Service projection lifecycle owned by `ResourceImport`) |
| `ADR046-zone-control-021` | `resources-zone-control` | Planned | adapt | `packages/d2b-core-controller/src/{coordinator |
| `ADR046-zone-control-022` | `resources-zone-control` | Planned | adapt | `packages/d2b-core-controller/src/authority.rs` |
| `ADR046-zone-control-023` | `resources-zone-control` | Planned | adapt | `packages/d2b-core-controller/src/{quota |
| `ADR046-zone-control-024` | `resources-zone-control` | Planned | adapt | `packages/d2b-core-controller/src/authority.rs` (Host-global index scope + hardware admission) |

### W6 - 257 work items

| Work item | Spec | State | Reuse | Dest (first path) |
| --- | --- | --- | --- | --- |
| `ADR046-aca-001` | `provider-runtime-azure-container-apps` | Planned | replace | `packages/d2b-provider-runtime-azure-container-apps/src/controller.rs` |
| `ADR046-aca-002` | `provider-runtime-azure-container-apps` | Planned | adapt | `packages/d2b-provider-runtime-azure-container-apps/src/deployment_service.rs` |
| `ADR046-aca-003` | `provider-runtime-azure-container-apps` | Planned | adapt | `packages/d2b-contracts/src/provider_effects/aca.rs` (shared `d2b-contracts` provider-effects module |
| `ADR046-aca-004` | `provider-runtime-azure-container-apps` | Planned | replace | ACA sandbox-agent Endpoint/session controller (§§7 |
| `ADR046-aca-005` | `provider-runtime-azure-container-apps` | Planned | adapt | `packages/d2b-provider-runtime-azure-container-apps/src/types.rs` |
| `ADR046-aca-006` | `provider-runtime-azure-container-apps` | Planned | replace | `nixos-modules/` (generated Guest resource options) |
| `ADR046-aca-007` | `provider-runtime-azure-container-apps` | Planned | create | `nixos-modules/` (gateway Guest declaration |
| `ADR046-activation-001` | `provider-activation-nixos` | Planned | adapt | packages/d2b-host/src/bin/d2b-activation-helper.rs |
| `ADR046-activation-002` | `provider-activation-nixos` | Planned | create | docs/reference/schemas/v3/activation-nixos.d2bus.org.NixosGeneration.json and packages/d2b-contracts/src/activation_nixos.rs |
| `ADR046-activation-003` | `provider-activation-nixos` | Planned | replace | packages/d2b-provider-activation-nixos/src/controller/ |
| `ADR046-activation-004` | `provider-activation-nixos` | Planned | adapt | packages/d2b-provider-activation-nixos/src/runner/ |
| `ADR046-activation-005` | `provider-activation-nixos` | Planned | replace | packages/d2b/src/activation.rs |
| `ADR046-activation-006` | `provider-activation-nixos` | Planned | adapt | nixos-modules/providers/activation-nixos.nix |
| `ADR046-activation-007` | `provider-activation-nixos` | Planned | delete-after-cutover | packages/d2b/src/lib.rs |
| `ADR046-audio-001` | `provider-audio-pipewire` | Planned | copy-unchanged | `packages/d2b-provider-audio-pipewire/src/audio_policy.rs` |
| `ADR046-audio-002` | `provider-audio-pipewire` | Planned | adapt | `packages/d2b-provider-audio-pipewire/src/argv.rs` (component template renderer) |
| `ADR046-audio-004` | `provider-audio-pipewire` | Planned | adapt | `packages/d2b-provider-audio-pipewire/src/mediator/enforcement.rs` |
| `ADR046-audio-005` | `provider-audio-pipewire` | Planned | adapt | `packages/d2b-provider-audio-pipewire/src/{resource_type |
| `ADR046-audio-006` | `provider-audio-pipewire` | Planned | adapt | `packages/d2b-provider-audio-pipewire/src/controller/audio_service.rs` |
| `ADR046-audio-007` | `provider-audio-pipewire` | Planned | create | `packages/d2b-provider-audio-pipewire/src/mediator/mod.rs` |
| `ADR046-audio-008` | `provider-audio-pipewire` | Planned | replace | `nixos-modules/components/audio/v3-resource.nix` |
| `ADR046-audio-009` | `provider-audio-pipewire` | Planned | adapt | `packages/d2b-provider-audio-pipewire/tests/minijail_contract.rs` (provider-local) |
| `ADR046-audio-010` | `provider-audio-pipewire` | Planned | adapt | `packages/d2b-provider-audio-pipewire/src/telemetry.rs` |
| `ADR046-audio-011` | `provider-audio-pipewire` | Planned | adapt | `packages/d2b-provider-audio-pipewire/src/guest_agent/mod.rs` |
| `ADR046-audio-012` | `provider-audio-pipewire` | Planned | adapt | `packages/d2b-provider-audio-pipewire/src/share_adapter.rs` |
| `ADR046-audio-013` | `provider-audio-pipewire` | Planned | adapt | `packages/d2b-provider-audio-pipewire/src/authority.rs` (speaker mixer + mic arbiter) |
| `ADR046-audio-014` | `provider-audio-pipewire` | Planned | adapt | `packages/d2b-provider-audio-pipewire/src/streams.rs` |
| `ADR046-azure-vm-001` | `provider-runtime-azure-virtual-machine` | Planned | adapt | `src/{lib.rs |
| `ADR046-azure-vm-002` | `provider-runtime-azure-virtual-machine` | Planned | adapt | `src/effect/{mod.rs |
| `ADR046-azure-vm-003` | `provider-runtime-azure-virtual-machine` | Planned | adapt | `src/controller/{mod.rs |
| `ADR046-azure-vm-004` | `provider-runtime-azure-virtual-machine` | Planned | adapt | `src/controller/bootstrap.rs` |
| `ADR046-azure-vm-005` | `provider-runtime-azure-virtual-machine` | Planned | adapt | `src/credential.rs` |
| `ADR046-azure-vm-006` | `provider-runtime-azure-virtual-machine` | Planned | adapt | `src/controller/idempotency.rs` |
| `ADR046-azure-vm-007` | `provider-runtime-azure-virtual-machine` | Planned | adapt | `nixos-modules/` (Provider/Guest resource emitters) |
| `ADR046-azure-vm-008` | `provider-runtime-azure-virtual-machine` | Planned | adapt | `src/{telemetry.rs |
| `ADR046-azure-vm-009` | `provider-runtime-azure-virtual-machine` | Planned | adapt | `tests/` |
| `ADR046-ch-001` | `provider-runtime-cloud-hypervisor` | Planned | adapt | `packages/d2b-provider-runtime-cloud-hypervisor/src/controller.rs` |
| `ADR046-ch-002` | `provider-runtime-cloud-hypervisor` | Planned | replace | `packages/d2b-provider-runtime-cloud-hypervisor/src/bootstrap_graph.rs` |
| `ADR046-ch-003` | `provider-runtime-cloud-hypervisor` | Planned | adapt | `packages/d2b-provider-runtime-cloud-hypervisor/src/vmm_argv.rs` |
| `ADR046-ch-004` | `provider-runtime-cloud-hypervisor` | Planned | adapt | `packages/d2b-provider-runtime-cloud-hypervisor/nix/` (Nix emitter) |
| `ADR046-ch-005` | `provider-runtime-cloud-hypervisor` | Planned | adapt | `packages/d2b-provider-runtime-cloud-hypervisor/src/health.rs` |
| `ADR046-ch-006` | `provider-runtime-cloud-hypervisor` | Planned | replace | `packages/d2b-provider-runtime-cloud-hypervisor/src/metrics.rs` |
| `ADR046-ch-007` | `provider-runtime-cloud-hypervisor` | Planned | replace | `packages/d2b-provider-runtime-cloud-hypervisor/src/state.rs` |
| `ADR046-clipboard-001` | `provider-clipboard-wayland` | Planned | create | packages/d2b-provider-clipboard-wayland/ with src |
| `ADR046-clipboard-002` | `provider-clipboard-wayland` | Planned | adapt | packages/d2b-provider-clipboard-wayland/src/clipd_host/ service binary modules such as service |
| `ADR046-clipboard-003` | `provider-clipboard-wayland` | Planned | create | packages/d2b-provider-clipboard-wayland/src/controller/ and clipboard-controller binary |
| `ADR046-clipboard-004` | `provider-clipboard-wayland` | Planned | adapt | packages/d2b-provider-clipboard-wayland/src/picker_session/ and picker-session binary |
| `ADR046-clipboard-005` | `provider-clipboard-wayland` | Planned | create | packages/d2b-provider-clipboard-wayland service descriptors and generated Rust async ttrpc bindings |
| `ADR046-clipboard-006` | `provider-clipboard-wayland` | Planned | replace | nixos-modules/providers/clipboard-wayland.nix and d2b.artifacts.clipboard-wayland catalog entry |
| `ADR046-clipboard-007` | `provider-clipboard-wayland` | Planned | create | packages/d2b-provider-clipboard-wayland/src/controller/rbac.rs or equivalent controller reconcile module |
| `ADR046-clipboard-008` | `provider-clipboard-wayland` | Planned | adapt | packages/d2b-provider-clipboard-wayland/src/service/audit.rs and packages/d2b-provider-clipboard-wayland/src/service/metrics.rs |
| `ADR046-clipboard-009` | `provider-clipboard-wayland` | Planned | extract | packages/d2b-provider-clipboard-wayland/tests/ |
| `ADR046-clipboard-010` | `provider-clipboard-wayland` | Planned | create | packages/d2b-provider-clipboard-wayland/integration/ |
| `ADR046-clipboard-011` | `provider-clipboard-wayland` | Planned | adapt | packages/d2b-contract-tests/tests/policy_clipboard.rs |
| `ADR046-clipboard-012` | `provider-clipboard-wayland` | Planned | delete-after-cutover | nixos-modules/default.nix |
| `ADR046-core-002` | `core-controllers` | Planned | adapt | `packages/d2b-core-controller/tests/system_core_coordination.rs` |
| `ADR046-cred-entra-001` | `provider-credential-entra` | Planned | adapt | `packages/d2b-provider-credential-entra/src/{lib.rs |
| `ADR046-cred-mi-001` | `provider-credential-managed-identity` | Planned | adapt | `packages/d2b-provider-credential-managed-identity/src/{lib.rs |
| `ADR046-cred-mi-002` | `provider-credential-managed-identity` | Planned | adapt | packages/d2b-provider-credential-managed-identity/src/controller.rs |
| `ADR046-cred-mi-003` | `provider-credential-managed-identity` | Planned | replace | nixos-modules/options-resources.nix |
| `ADR046-cred-mi-004` | `provider-credential-managed-identity` | Planned | adapt | packages/d2b-provider-credential-managed-identity/src/{audit.rs |
| `ADR046-cred-ss-001` | `provider-credential-secret-service` | Planned | adapt | packages/d2b-contracts/src/v3/credential.rs |
| `ADR046-cred-ss-002` | `provider-credential-secret-service` | Planned | create | packages/d2b-contracts/proto/v3/credential.proto |
| `ADR046-cred-ss-003` | `provider-credential-secret-service` | Planned | adapt | `packages/d2b-provider-credential-secret-service/src/{lib.rs |
| `ADR046-cred-ss-004` | `provider-credential-secret-service` | Planned | create | packages/d2b-provider-credential-<impl>/src/controller.rs |
| `ADR046-cred-ss-005` | `provider-credential-secret-service` | Planned | create | nixos-modules/options-resources.nix |
| `ADR046-cred-ss-006` | `provider-credential-secret-service` | Planned | adapt | packages/d2b-provider-credential-secret-service/src/{audit.rs |
| `ADR046-device-tpm-001` | `provider-device-tpm` | Planned | adapt | packages/d2b-provider-device-tpm/{src/ |
| `ADR046-device-tpm-002` | `provider-device-tpm` | Planned | wrap | packages/d2b-provider-device-tpm/src/effect_port.rs |
| `ADR046-device-tpm-003` | `provider-device-tpm` | Planned | replace | packages/d2b-provider-device-tpm/src/controller.rs |
| `ADR046-device-tpm-004` | `provider-device-tpm` | Planned | replace | packages/d2b-provider-device-tpm/src/resources.rs |
| `ADR046-device-tpm-005` | `provider-device-tpm` | Planned | adapt | packages/d2b-provider-device-tpm/src/resources.rs |
| `ADR046-device-tpm-006` | `provider-device-tpm` | Planned | adapt | packages/d2b-provider-device-tpm/src/resources.rs |
| `ADR046-device-tpm-007` | `provider-device-tpm` | Planned | create | packages/d2b-provider-device-tpm/src/status.rs |
| `ADR046-device-tpm-008` | `provider-device-tpm` | Planned | replace | packages/d2b-provider-device-tpm/src/{effect_port.rs |
| `ADR046-device-tpm-009` | `provider-device-tpm` | Planned | adapt | packages/d2b-provider-device-tpm/tests/marker_fail_closed.rs |
| `ADR046-device-tpm-010` | `provider-device-tpm` | Planned | create | packages/d2b-provider-device-tpm/src/resources.rs |
| `ADR046-device-tpm-011` | `provider-device-tpm` | Planned | replace | nixos-modules/options-resources.nix and Nix eval/golden tests for §17.1 Device JSON |
| `ADR046-device-tpm-012` | `provider-device-tpm` | Planned | adapt | packages/d2b-provider-device-tpm/src/controller.rs |
| `ADR046-device-tpm-013` | `provider-device-tpm` | Planned | delete-after-cutover | packages/d2bd/src/* |
| `ADR046-display-001` | `provider-display-wayland` | Planned | adapt | `packages/d2b-provider-display-wayland/src/` |
| `ADR046-display-002` | `provider-display-wayland` | Planned | adapt | Zone bundle emitter for `WaylandSession` / `WaylandPolicy` ResourceSpecs under `d2b.zones.<zone>.resources.*` |
| `ADR046-display-003` | `provider-display-wayland` | Planned | adapt | `packages/d2b-provider-display-wayland/src/audit.rs` |
| `ADR046-display-004` | `provider-display-wayland` | Planned | create | `packages/d2b-provider-display-wayland/integration/` |
| `ADR046-gpu-001` | `provider-device-gpu` | Planned | extract | `packages/d2b-provider-device-gpu/` with `src/` |
| `ADR046-gpu-002` | `provider-device-gpu` | Planned | adapt | `packages/d2b-provider-device-gpu/src/{controller.rs |
| `ADR046-gpu-003` | `provider-device-gpu` | Planned | create | `packages/d2b-provider-device-gpu/src/probe.rs` |
| `ADR046-gpu-004` | `provider-device-gpu` | Planned | create | `packages/d2b-provider-device-gpu/src/arbitration.rs` |
| `ADR046-gpu-005` | `provider-device-gpu` | Planned | adapt | `packages/d2b-provider-device-gpu/src/worker_gpu.rs` |
| `ADR046-gpu-006` | `provider-device-gpu` | Planned | adapt | `packages/d2b-provider-device-gpu/src/worker_video.rs` |
| `ADR046-gpu-007` | `provider-device-gpu` | Planned | adapt | `nixos-modules/assertions.nix` (new GPU Device eval assertions) |
| `ADR046-gpu-008` | `provider-device-gpu` | Planned | create | `packages/d2b-provider-device-gpu/` component descriptor |
| `ADR046-gpu-009` | `provider-device-gpu` | Planned | create | `packages/d2b-provider-device-gpu/README.md` |
| `ADR046-mi-topology-001` | `provider-credential-managed-identity` | Planned | adapt | packages/d2b-provider-credential-managed-identity/src/{controller.rs |
| `ADR046-minijail-001` | `provider-system-minijail` | Planned | adapt | `packages/d2b-provider-system-minijail/src/sandbox_compiler.rs` |
| `ADR046-minijail-002` | `provider-system-minijail` | Planned | adapt | Provider-side opaque request builder in `packages/d2b-provider-system-minijail/src/launch.rs` |
| `ADR046-minijail-003` | `provider-system-minijail` | Planned | adapt | Broker-side: `d2b-priv-broker` retains `SpawnRunner` and user-namespace pre-establishment |
| `ADR046-minijail-004` | `provider-system-minijail` | Planned | adapt | Broker-side parent wait/reap and typed terminal relay in `packages/d2b-priv-broker/src/` |
| `ADR046-minijail-005` | `provider-system-minijail` | Planned | adapt | `packages/d2b-provider-system-minijail/src/` - controller binary entry point |
| `ADR046-minijail-006` | `provider-system-minijail` | Planned | adapt | `nixos-modules/` - v3 Nix `Process`/`EphemeralProcess` resource authoring |
| `ADR046-nl-001` | `provider-network-local` | Planned | create | `d2b-contracts` trait plus `d2b-core` core adapter |
| `ADR046-nl-002` | `provider-network-local` | Planned | adapt | Broker wire contract and broker/core adapter operation table for `DeletePersistentTap` |
| `ADR046-nl-003` | `provider-network-local` | Planned | create | `d2b-contracts` opaque byte-array newtypes |
| `ADR046-nl-004` | `provider-network-local` | Planned | create | Core LaunchTicket builder and dependency resolver that walks `Guest.ownerRef: Network/<name>` to resolved tap FDs. |
| `ADR046-nl-005` | `provider-network-local` | Planned | adapt | Core adapter imports `d2b-host` modules |
| `ADR046-nl-006` | `provider-network-local` | Planned | adapt | `packages/d2b-provider-network-local/src/{controller.rs |
| `ADR046-nl-007` | `provider-network-local` | Planned | create | `packages/d2b-provider-network-local/src/process_specs.rs` agent template plus agent service implementation in the net-VM artifact. |
| `ADR046-nl-008` | `provider-network-local` | Planned | adapt | `packages/d2b-provider-network-local/src/config_volume.rs`. |
| `ADR046-nl-009` | `provider-network-local` | Planned | adapt | `packages/d2b-provider-network-local/src/process_specs.rs`. |
| `ADR046-nl-010` | `provider-network-local` | Planned | adapt | `net-vm-base` nixos-system artifact and artifact catalog entry `d2b.artifacts.net-vm-base`. |
| `ADR046-nl-011` | `provider-network-local` | Planned | adapt | Nix module resource emission for `Provider/network-local` |
| `ADR046-nl-012` | `provider-network-local` | Planned | adapt | Nix flake/resource schema checks for declared Networks and provider `validate.rs` parity. |
| `ADR046-nl-013` | `provider-network-local` | Planned | adapt | `packages/d2b-provider-network-local/tests/schema_roundtrip.rs` |
| `ADR046-nl-014` | `provider-network-local` | Planned | create | `packages/d2b-provider-network-local/tests/controller_state.rs`. |
| `ADR046-nl-015` | `provider-network-local` | Planned | adapt | `packages/d2b-provider-network-local/integration/host_fabric.rs` |
| `ADR046-nl-016` | `provider-network-local` | Planned | adapt | Process templates for agent and dnsmasq plus sandbox/eval tests. |
| `ADR046-nl-017` | `provider-network-local` | Planned | create | `packages/d2b-provider-network-local/README.md`. |
| `ADR046-nl-018` | `provider-network-local` | Planned | adapt | Device-usbip EffectPort/adapter owns USBIP rules |
| `ADR046-nl-019` | `provider-network-local` | Planned | create | Provider descriptor |
| `ADR046-nl-020` | `provider-network-local` | Planned | adapt | Network schema/Provider descriptor |
| `ADR046-notify-001` | `provider-notification-desktop` | Planned | adapt | `packages/d2b-provider-notification-desktop/src/{types |
| `ADR046-notify-002` | `provider-notification-desktop` | Planned | adapt | `packages/d2b-provider-notification-desktop/src/stream_admission.rs` |
| `ADR046-notify-003` | `provider-notification-desktop` | Planned | create | `packages/d2b-provider-notification-desktop/src/controller.rs` |
| `ADR046-notify-004` | `provider-notification-desktop` | Planned | adapt | `packages/d2b-provider-notification-desktop/src/host_sink.rs` |
| `ADR046-notify-005` | `provider-notification-desktop` | Planned | create | `packages/d2b-provider-notification-desktop/src/guest_source.rs` |
| `ADR046-notify-006` | `provider-notification-desktop` | Planned | adapt | Nix: Zone resource authoring in `nixos-modules/` |
| `ADR046-otel-001` | `provider-observability-otel` | Planned | adapt | `packages/d2b-provider-observability-otel/src/{forwarder_bin |
| `ADR046-otel-002` | `provider-observability-otel` | Planned | adapt | `packages/d2b-provider-observability-otel/src/{collector_bin |
| `ADR046-otel-003` | `provider-observability-otel` | Planned | adapt | `packages/d2b-provider-observability-otel/src/nix/journald.nix` |
| `ADR046-otel-004` | `provider-observability-otel` | Planned | adapt | `packages/d2b-contract-tests/tests/policy_observability.rs` (updated) |
| `ADR046-otel-005` | `provider-observability-otel` | Planned | adapt | `packages/d2b-provider-observability-otel/src/share_adapter.rs` |
| `ADR046-otel-006` | `provider-observability-otel` | Planned | adapt | `packages/d2b-provider-observability-otel/src/{authority |
| `ADR046-qemu-media-001` | `provider-runtime-qemu-media` | Planned | create | packages/d2b-provider-runtime-qemu-media/{src/lib.rs |
| `ADR046-qemu-media-002` | `provider-runtime-qemu-media` | Planned | adapt | packages/d2b-provider-runtime-qemu-media/src/types/guest.rs |
| `ADR046-qemu-media-003` | `provider-runtime-qemu-media` | Planned | adapt | packages/d2b-provider-runtime-qemu-media/src/config.rs |
| `ADR046-qemu-media-004` | `provider-runtime-qemu-media` | Planned | create | packages/d2b-provider-runtime-qemu-media/src/{descriptor.rs |
| `ADR046-qemu-media-005` | `provider-runtime-qemu-media` | Planned | adapt | packages/d2b-provider-runtime-qemu-media/src/controller/volume.rs |
| `ADR046-qemu-media-006` | `provider-runtime-qemu-media` | Planned | adapt | packages/d2b-provider-runtime-qemu-media/src/controller/media_watch.rs |
| `ADR046-qemu-media-007` | `provider-runtime-qemu-media` | Planned | create | packages/d2b-provider-runtime-qemu-media/src/controller/device_watch.rs |
| `ADR046-qemu-media-008` | `provider-runtime-qemu-media` | Planned | create | packages/d2b-provider-runtime-qemu-media/src/controller/display.rs |
| `ADR046-qemu-media-009` | `provider-runtime-qemu-media` | Planned | adapt | packages/d2b-provider-runtime-qemu-media/src/controller/process_builder.rs |
| `ADR046-qemu-media-010` | `provider-runtime-qemu-media` | Planned | adapt | packages/d2b-provider-runtime-qemu-media/src/qmp/ |
| `ADR046-qemu-media-011` | `provider-runtime-qemu-media` | Planned | adapt | packages/d2b-provider-runtime-qemu-media/src/controller/hotplug.rs |
| `ADR046-qemu-media-012` | `provider-runtime-qemu-media` | Planned | create | packages/d2b-provider-runtime-qemu-media/src/controller/network.rs |
| `ADR046-qemu-media-013` | `provider-runtime-qemu-media` | Planned | create | packages/d2b-provider-runtime-qemu-media/src/controller/reconcile.rs |
| `ADR046-qemu-media-014` | `provider-runtime-qemu-media` | Planned | create | packages/d2b-provider-runtime-qemu-media/src/controller/status.rs |
| `ADR046-qemu-media-015` | `provider-runtime-qemu-media` | Planned | create | packages/d2b-provider-runtime-qemu-media/src/audit.rs |
| `ADR046-qemu-media-016` | `provider-runtime-qemu-media` | Planned | create | packages/d2b-provider-runtime-qemu-media/src/telemetry.rs |
| `ADR046-qemu-media-017` | `provider-runtime-qemu-media` | Planned | adapt | nixos-modules/options-guest-qemu-media.nix |
| `ADR046-qemu-media-018` | `provider-runtime-qemu-media` | Planned | adapt | packages/d2b-provider-runtime-qemu-media/tests/conformance_guest.rs |
| `ADR046-qemu-media-019` | `provider-runtime-qemu-media` | Planned | create | packages/d2b-provider-runtime-qemu-media/integration/ |
| `ADR046-security-key-001` | `provider-device-security-key` | Planned | adapt | Move to `packages/d2b-provider-device-security-key/src/session.rs` and `cid.rs` |
| `ADR046-security-key-002` | `provider-device-security-key` | Planned | adapt | Move to `packages/d2b-provider-device-security-key/src/relay.rs` |
| `ADR046-security-key-003` | `provider-device-security-key` | Planned | adapt | Adopt `main.rs` and `uhid.rs` as the v3 Process binary entry point |
| `ADR046-security-key-004` | `provider-device-security-key` | Planned | adapt | Preserve revalidation logic |
| `ADR046-security-key-005` | `provider-device-security-key` | Planned | adapt | Adapt to v3 Zone/ResourceRef identifiers |
| `ADR046-security-key-006` | `provider-device-security-key` | Planned | adapt | Move to `packages/d2b-provider-device-security-key/tests/` |
| `ADR046-security-key-007` | `provider-device-security-key` | Planned | adapt | Move to `packages/d2b-provider-device-security-key/tests/` |
| `ADR046-security-key-008` | `provider-device-security-key` | Planned | create | New crate `packages/d2b-provider-device-security-key/` with `src/` |
| `ADR046-security-key-009` | `provider-device-security-key` | Planned | create | `packages/d2b-provider-device-security-key/src/controller.rs` |
| `ADR046-security-key-010` | `provider-device-security-key` | Planned | create | `packages/d2b-provider-device-security-key/src/relay.rs` |
| `ADR046-security-key-011` | `provider-device-security-key` | Planned | create | `packages/d2b-provider-device-security-key/src/session.rs` |
| `ADR046-security-key-012` | `provider-device-security-key` | Planned | create | `packages/d2b-provider-device-security-key/src/cid.rs` |
| `ADR046-security-key-013` | `provider-device-security-key` | Planned | create | `packages/d2b-provider-device-security-key/src/probe.rs` |
| `ADR046-security-key-014` | `provider-device-security-key` | Planned | create | `packages/d2b-provider-device-security-key/src/descriptor.rs` |
| `ADR046-security-key-015` | `provider-device-security-key` | Planned | create | `nixos-modules/minijail-profiles.nix` entries for relay and controller |
| `ADR046-security-key-016` | `provider-device-security-key` | Planned | create | Provider descriptor Process templates and owned CTAPHID `Endpoint` template for `Provider/device-security-key` |
| `ADR046-security-key-017` | `provider-device-security-key` | Planned | create | Signed Provider descriptor JSON for `Provider/device-security-key` in the provider package |
| `ADR046-security-key-018` | `provider-device-security-key` | Planned | create | v3 `SecurityKeyOpenDevice` broker op and Core LaunchTicket DeviceGrant resolution path |
| `ADR046-security-key-019` | `provider-device-security-key` | Planned | create | `nixos-modules/` resource compiler/eval assertions for physical Device |
| `ADR046-security-key-020` | `provider-device-security-key` | Planned | create | `nixos-modules/components/security-key-guest.nix` migration gate `d2b.securityKey._legacySystemdUnit` |
| `ADR046-security-key-021` | `provider-device-security-key` | Planned | create | Core `device-grant` audit and Provider controller Service/Binding ceremony lifecycle audit |
| `ADR046-security-key-022` | `provider-device-security-key` | Planned | create | Provider/controller bounded telemetry emitter and observability-otel handoff for security-key metrics |
| `ADR046-security-key-023` | `provider-device-security-key` | Planned | create | `packages/d2b-provider-device-security-key/README.md` |
| `ADR046-security-key-024` | `provider-device-security-key` | Planned | create | Authority/projection Service Endpoint and Binding private Endpoint resolution |
| `ADR046-security-key-025` | `provider-device-security-key` | Planned | create | `d2b-contracts` neutral `SecurityKeyEffectPort` trait/types |
| `ADR046-security-key-026` | `provider-device-security-key` | Planned | create | `packages/d2b-provider-device-security-key/src/{resource_type |
| `ADR046-security-key-027` | `provider-device-security-key` | Planned | create | Provider descriptor state declaration |
| `ADR046-security-key-028` | `provider-device-security-key` | Planned | adapt | `packages/d2b-provider-device-security-key/src/share_adapter.rs` |
| `ADR046-security-key-029` | `provider-device-security-key` | Planned | adapt | `packages/d2b-provider-device-security-key/src/{authority |
| `ADR046-security-key-030` | `provider-device-security-key` | Planned | delete-after-cutover | Removed from daemon |
| `ADR046-security-key-031` | `provider-device-security-key` | Planned | delete-after-cutover | Removed from daemon startup |
| `ADR046-security-key-032` | `provider-device-security-key` | Planned | delete-after-cutover | Removed from guest Nix module |
| `ADR046-security-key-033` | `provider-device-security-key` | Planned | delete-after-cutover | Removed from `packages/d2b-contract-tests/tests/` |
| `ADR046-security-key-034` | `provider-device-security-key` | Planned | delete-after-cutover | Removed from `d2b-core/src/processes.rs` |
| `ADR046-security-key-035` | `provider-device-security-key` | Planned | delete-after-cutover | Removed from contracts and broker |
| `ADR046-sterm-001` | `provider-shell-terminal` | Planned | create | `packages/d2b-provider-shell-terminal/src/resources/{pool |
| `ADR046-sterm-002` | `provider-shell-terminal` | Planned | create | `packages/d2b-provider-shell-terminal/src/bin/d2b-shell-terminal-controller.rs` |
| `ADR046-sterm-003` | `provider-shell-terminal` | Planned | adapt | `packages/d2b-provider-shell-terminal/src/bin/d2b-shell-session-supervisor.rs` |
| `ADR046-sterm-004` | `provider-shell-terminal` | Planned | replace | `packages/d2b-provider-shell-terminal/src/process_templates.rs` |
| `ADR046-sterm-005` | `provider-shell-terminal` | Planned | create | `packages/d2b-provider-shell-terminal/src/service/open_session.rs` |
| `ADR046-sterm-006` | `provider-shell-terminal` | Planned | adapt | `packages/d2b-provider-shell-terminal/src/session/{pty |
| `ADR046-sterm-007` | `provider-shell-terminal` | Planned | adapt | `packages/d2b-provider-shell-terminal/src/session/adopt.rs` |
| `ADR046-sterm-008` | `provider-shell-terminal` | Planned | replace | `packages/d2b-provider-shell-terminal/src/host_rules.rs` |
| `ADR046-sterm-009` | `provider-shell-terminal` | Planned | replace | `packages/d2b-provider-shell-terminal/src/guest_rules.rs` |
| `ADR046-sterm-010` | `provider-shell-terminal` | Planned | replace | `packages/d2b-provider-shell-terminal/src/authz.rs` |
| `ADR046-sterm-011` | `provider-shell-terminal` | Planned | create | `packages/d2b-provider-shell-terminal/src/{audit |
| `ADR046-sterm-012` | `provider-shell-terminal` | Planned | delete-after-cutover | `packages/d2b-provider-shell-terminal/src/migration.rs` |
| `ADR046-sterm-013` | `provider-shell-terminal` | Planned | adapt | `packages/d2b-provider-shell-terminal/src/service/{controller |
| `ADR046-system-core-001` | `provider-system-core` | Planned | adapt | `packages/d2b-provider-system-core/src/manifest.rs` |
| `ADR046-systemd-001` | `provider-system-systemd` | Planned | adapt | `packages/d2b-provider-system-systemd/src/controller.rs` (async reconcile loop) |
| `ADR046-systemd-002` | `provider-system-systemd` | Planned | adapt | `nixos-modules/` (Provider ResourceSpec emission for `system-systemd`) |
| `ADR046-systemd-003` | `provider-system-systemd` | Planned | adapt | `packages/d2b-provider-system-systemd/tests/conformance.rs` |
| `ADR046-transport-relay-001` | `provider-transport-azure-relay` | Planned | adapt | `packages/d2b-provider-transport-azure-relay/src/relay_transport.rs` |
| `ADR046-transport-relay-002` | `provider-transport-azure-relay` | Planned | create | `packages/d2b-provider-transport-azure-relay/src/credential_client.rs` |
| `ADR046-transport-relay-003` | `provider-transport-azure-relay` | Planned | create | `packages/d2b-provider-transport-azure-relay/src/reconnect.rs` |
| `ADR046-transport-relay-004` | `provider-transport-azure-relay` | Planned | create | `packages/d2b-provider-transport-azure-relay/src/transport_settings.rs` |
| `ADR046-transport-relay-005` | `provider-transport-azure-relay` | Planned | adapt | `packages/d2b-provider-transport-azure-relay/src/backpressure.rs` |
| `ADR046-transport-relay-006` | `provider-transport-azure-relay` | Planned | create | `packages/d2b-provider-transport-azure-relay/src/{metrics.rs |
| `ADR046-transport-relay-007` | `provider-transport-azure-relay` | Planned | create | `packages/d2b-provider-transport-azure-relay/src/tests/integration/README` |
| `ADR046-transport-unix-001` | `provider-transport-unix` | Planned | adapt | `packages/d2b-provider-transport-unix/src/credit.rs` (imports `MAX_PACKET_ATTACHMENTS=32` |
| `ADR046-transport-unix-002` | `provider-transport-unix` | Planned | adapt | `packages/d2b-provider-transport-unix/src/{seqpacket |
| `ADR046-transport-unix-003` | `provider-transport-unix` | Planned | adapt | `packages/d2b-provider-transport-unix/src/{stream |
| `ADR046-transport-unix-004` | `provider-transport-unix` | Planned | adapt | `packages/d2b-provider-transport-unix/src/credit.rs` |
| `ADR046-transport-unix-005` | `provider-transport-unix` | Planned | adapt | `packages/d2b-provider-transport-unix/src/descriptor.rs` |
| `ADR046-transport-unix-006` | `provider-transport-unix` | Planned | adapt | `packages/d2b-provider-transport-unix/src/admission.rs` |
| `ADR046-transport-unix-007` | `provider-transport-unix` | Planned | adapt | `packages/d2b-provider-transport-unix/src/{portal |
| `ADR046-transport-unix-008` | `provider-transport-unix` | Planned | adapt | `packages/d2b-provider-transport-unix/` crate Cargo.toml binary target `d2b-transport-unix-service` |
| `ADR046-transport-unix-009` | `provider-transport-unix` | Planned | create | `docs/reference/schemas/v3/providers/transport-unix.transport-binding.json` |
| `ADR046-transport-unix-010` | `provider-transport-unix` | Planned | create | `packages/d2b-provider-transport-unix/src/{audit |
| `ADR046-transport-unix-011` | `provider-transport-unix` | Planned | adapt | `packages/d2b-provider-transport-unix/integration/` and `integration/README.md` |
| `ADR046-usbip-001` | `provider-device-usbip` | Planned | create | packages/d2b-contracts/src/usbip_effect_port.rs |
| `ADR046-usbip-002` | `provider-device-usbip` | Planned | adapt | packages/d2b-core/src/device_usbip_adapter.rs |
| `ADR046-usbip-003` | `provider-device-usbip` | Planned | create | packages/d2b-provider-device-usbip/ |
| `ADR046-usbip-004` | `provider-device-usbip` | Planned | adapt | packages/d2b-provider-device-usbip/src/{controller |
| `ADR046-usbip-005` | `provider-device-usbip` | Planned | adapt | packages/d2b-provider-device-usbip/src/reconcile.rs |
| `ADR046-usbip-006` | `provider-device-usbip` | Planned | adapt | packages/d2b-provider-device-usbip/src/status.rs |
| `ADR046-usbip-007` | `provider-device-usbip` | Planned | adapt | packages/d2b-provider-device-usbip/{src |
| `ADR046-usbip-008` | `provider-device-usbip` | Planned | adapt | nixos-modules/components/usbip.nix |
| `ADR046-usbip-009` | `provider-device-usbip` | Planned | delete-after-cutover | packages/d2bd/src/ |
| `ADR046-vl-001` | `provider-volume-local` | Planned | adapt | `d2b-contracts/src/v3/volume_layout.rs` (LayoutEntry |
| `ADR046-vl-002` | `provider-volume-local` | Planned | adapt | Full `packages/d2b-provider-volume-local/` scaffold per §Crate layout: `src/` |
| `ADR046-vl-003` | `provider-volume-local` | Planned | adapt | `src/controller.rs` |
| `ADR046-vl-004` | `provider-volume-local` | Planned | adapt | `src/store_view.rs` |
| `ADR046-vl-005` | `provider-volume-local` | Planned | adapt | `src/swtpm_volume.rs` |
| `ADR046-vl-006` | `provider-volume-local` | Planned | create | `src/source.rs` (block-image and tmpfs branches) |
| `ADR046-vl-007` | `provider-volume-local` | Planned | adapt | `src/{migration |
| `ADR046-vl-008` | `provider-volume-local` | Planned | create | `src/relocation.rs` |
| `ADR046-vl-009` | `provider-volume-local` | Planned | adapt | `src/audit.rs` |
| `ADR046-vl-010` | `provider-volume-local` | Planned | adapt | `nixos-modules/zone-resources.nix` (per §ADR046-pstate-010) |
| `ADR046-vl-011` | `provider-volume-local` | Planned | adapt | `packages/xtask/src/provider_crate_policy.rs` |
| `ADR046-vl-012` | `provider-volume-local` | Planned | adapt | `packages/d2b-host/src/volume_effect_adapter.rs` (or the equivalent host-runtime crate designated by the Zone broker owner) |
| `ADR046-vl-013` | `provider-volume-local` | Planned | create | Zone core ProviderDeployment controller-start path (outside `d2b-provider-volume-local`) |
| `ADR046-vsock-001` | `provider-transport-vsock` | Planned | create | `packages/d2b-provider-transport-vsock/src/effect_port.rs` |
| `ADR046-vsock-002` | `provider-transport-vsock` | Planned | adapt | `packages/d2b-provider-transport-vsock/src/framing.rs` and `src/bridge.rs` |
| `ADR046-vsock-003` | `provider-transport-vsock` | Planned | adapt | `packages/d2b-provider-transport-vsock/src/service.rs` |
| `ADR046-vsock-004` | `provider-transport-vsock` | Planned | adapt | `d2b-core-controller` child Zone runtime `LiveVsockEffectPort` |
| `ADR046-vsock-005` | `provider-transport-vsock` | Planned | create | ProviderDeployment Volume creation/deletion path plus `packages/d2b-provider-transport-vsock/tests/state_volume.rs`. |
| `ADR046-vsock-006` | `provider-transport-vsock` | Planned | create | `packages/d2b-provider-transport-vsock/integration/host_guest.rs` and `integration/no_fd_transfer.rs`. |
| `ADR046-vsock-007` | `provider-transport-vsock` | Planned | delete-after-cutover | Remove legacy paths from `d2b-host` and `d2bd` |
| `ADR046-vvfs-001` | `provider-volume-virtiofs` | Planned | adapt | `packages/d2b-provider-volume-virtiofs/src/virtiofsd_argv.rs` |
| `ADR046-vvfs-002` | `provider-volume-virtiofs` | Planned | extract | `packages/d2b-provider-volume-virtiofs/src/user_ns.rs` (conformance kit) |
| `ADR046-vvfs-003` | `provider-volume-virtiofs` | Planned | adapt | `packages/d2b-provider-volume-virtiofs/src/controller.rs` |
| `ADR046-vvfs-004` | `provider-volume-virtiofs` | Planned | adapt | `packages/d2b-provider-volume-virtiofs/src/readiness.rs` |
| `ADR046-vvfs-005` | `provider-volume-virtiofs` | Planned | adapt | `packages/d2b-provider-volume-virtiofs/src/controller.rs` (pre-launch prerequisite check) |
| `ADR046-vvfs-006` | `provider-volume-virtiofs` | Planned | adapt | `nixos-modules/resources-volume.nix` (store-view and user Volume attachment emission) |
| `ADR046-vvfs-export-001` | `provider-volume-virtiofs` | Planned | create | `packages/d2b-provider-volume-virtiofs/src/export.rs` |

### W7 - 73 work items

| Work item | Spec | State | Reuse | Dest (first path) |
| --- | --- | --- | --- | --- |
| `ADR046-delivery-001` | `validation-and-delivery` | Planned | adapt | `packages/xtask/src/heavy_gate.rs` |
| `ADR046-delivery-002` | `validation-and-delivery` | Planned | adapt | `packages/xtask/src/delivery/snapshot.rs` |
| `ADR046-delivery-003` | `validation-and-delivery` | Planned | adapt | `packages/xtask/src/delivery/validate_import.rs` |
| `ADR046-delivery-004` | `validation-and-delivery` | Planned | adapt | `packages/xtask/src/gen_spec_set.rs` |
| `ADR046-delivery-005` | `validation-and-delivery` | Planned | adapt | `packages/xtask/src/delivery/panel.rs` |
| `ADR046-delivery-006` | `validation-and-delivery` | Planned | adapt | `packages/xtask/src/delivery/{seal |
| `ADR046-delivery-007` | `validation-and-delivery` | Planned | adapt | `packages/xtask/src/test_runtime_ledger.rs` |
| `ADR046-delivery-008` | `validation-and-delivery` | Planned | adapt | `docs/specs/ADR-046-implementation-graph.json` |
| `ADR046-delivery-009` | `validation-and-delivery` | Planned | adapt | `packages/xtask/src/gen_spec_set.rs` |
| `ADR046-feasibility-002` | `feasibility-and-spikes` | Planned | adapt | `proofs/process-fastlaunch-spike/` |
| `ADR046-feasibility-003` | `feasibility-and-spikes` | Planned | adapt | `proofs/effectport-async-spike/` |
| `ADR046-feasibility-004` | `feasibility-and-spikes` | Planned | adapt | `proofs/provider-packaging-spike/` |
| `ADR046-feasibility-005` | `feasibility-and-spikes` | Planned | adapt | `proofs/bus-routing-noise-spike/` |
| `ADR046-feasibility-006` | `feasibility-and-spikes` | Planned | adapt | `proofs/provider-state-export-spike/` |
| `ADR046-feasibility-007` | `feasibility-and-spikes` | Planned | adapt | `proofs/process-provider-conformance-spike/` |
| `ADR046-feasibility-008` | `feasibility-and-spikes` | Planned | adapt | `proofs/nix-authoring-spike/` |
| `ADR046-feasibility-009` | `feasibility-and-spikes` | Planned | adapt | `proofs/cli-discovery-spike/` |
| `ADR046-feasibility-010` | `feasibility-and-spikes` | Planned | adapt | `proofs/e2e-composition-spike/` |
| `ADR046-feasibility-011` | `feasibility-and-spikes` | Planned | adapt | `proofs/test-runtime-budget-spike/` |
| `ADR046-reset-001` | `reset-and-cutover` | Planned | adapt | `packages/d2b-cutover/src/{inventory |
| `ADR046-reset-002` | `reset-and-cutover` | Planned | adapt | `packages/d2b-cutover/src/{bundle_validate |
| `ADR046-reset-003` | `reset-and-cutover` | Planned | adapt | `packages/d2b-cutover/src/{consent |
| `ADR046-reset-004` | `reset-and-cutover` | Planned | adapt | `packages/d2b-cutover/src/adopt.rs` |
| `ADR046-reset-005` | `reset-and-cutover` | Planned | create | `packages/d2b-cutover/src/{store_bootstrap |
| `ADR046-reset-006` | `reset-and-cutover` | Planned | adapt | `packages/d2b-cutover/src/{zonelink_cutover |
| `ADR046-reset-007` | `reset-and-cutover` | Planned | adapt | `packages/d2b-cutover/src/{verify |
| `ADR046-reset-008` | `reset-and-cutover` | Planned | create | `packages/d2b-cutover/src/finalize.rs` |
| `ADR046-reset-009` | `reset-and-cutover` | Planned | adapt | `packages/d2b-cutover/src/{journal |
| `ADR046-reset-010` | `reset-and-cutover` | Planned | adapt | `packages/d2b-cutover/src/reset_scope.rs` |
| `ADR046-reset-011` | `reset-and-cutover` | Planned | create | `tests/integration/live/cutover-real-host.sh` |
| `ADR046-security-001` | `security-and-threat-model` | Planned | adapt | `packages/d2b-contract-tests/tests/policy_telemetry_redaction.rs` |
| `ADR046-security-002` | `security-and-threat-model` | Planned | adapt | `packages/d2b-session/tests/noise_conformance.rs` |
| `ADR046-security-003` | `security-and-threat-model` | Planned | adapt | `packages/d2b-resource-store/tests/rbac_property.rs` |
| `ADR046-security-004` | `security-and-threat-model` | Planned | adapt | `packages/d2b-bus/fuzz/fuzz_targets/zonelink_frame.rs` |
| `ADR046-security-005` | `security-and-threat-model` | Planned | adapt | `packages/xtask/src/effectport_boundary_check.rs` |
| `ADR046-security-006` | `security-and-threat-model` | Planned | adapt | `packages/d2b-provider-system-minijail/tests/launchticket_toctou.rs` |
| `ADR046-security-007` | `security-and-threat-model` | Planned | adapt | `packages/d2b-contract-tests/tests/quarantine_not_kill_matrix.rs` |
| `ADR046-security-008` | `security-and-threat-model` | Planned | adapt | `packages/d2b-provider-system-core/tests/no_isolation_propagation.rs` |
| `ADR046-security-009` | `security-and-threat-model` | Planned | adapt | `packages/d2b-provider-volume-local/tests/marker_tamper_fault_injection.rs` |
| `ADR046-security-010` | `security-and-threat-model` | Planned | adapt | `packages/d2b-contract-tests/tests/zero_secret_invariant.rs` |
| `ADR046-security-011` | `security-and-threat-model` | Planned | adapt | `packages/d2b-provider-{clipboard-wayland |
| `ADR046-security-012` | `security-and-threat-model` | Planned | adapt | `packages/d2b-audit/tests/privileged_fail_closed.rs` |
| `ADR046-security-013` | `security-and-threat-model` | Planned | adapt | `packages/d2b-bus/tests/dos_ceiling_fault_injection.rs` |
| `ADR046-security-014` | `security-and-threat-model` | Planned | adapt | `packages/d2b/src/commands/{doctor |
| `ADR046-security-015` | `security-and-threat-model` | Planned | adapt | `packages/d2b-core-controller/src/reset.rs` |
| `ADR046-security-016` | `security-and-threat-model` | Planned | adapt | `tests/unit/gates/security-matrix-coverage.sh` |
| `ADR046-security-017` | `security-and-threat-model` | Planned | adapt | `tests/integration/containers/malicious-child-zone.rs` |
| `ADR046-security-018` | `security-and-threat-model` | Planned | adapt | `docs/reference/security-manual-validation-checklist.md` (new reference doc |
| `ADR046-security-019` | `security-and-threat-model` | Planned | adapt | `packages/d2b-contract-tests/tests/minijail_process_ownership.rs` |
| `ADR046-streamline-001` | `streamline` | Planned | create | `docs/specs/ADR-046-spec-set.json` |
| `ADR046-streamline-002` | `streamline` | Planned | create | `docs/specs/schemas/*.schema.json` (Tier A: hand-authored-once canonical source checked into the tree |
| `ADR046-streamline-003` | `streamline` | Planned | create | `packages/xtask/src/bin/spec_schema_check.rs` |
| `ADR046-streamline-004` | `streamline` | Planned | create | `docs/specs/providers/TEMPLATE.md` (committed |
| `ADR046-streamline-005` | `streamline` | Planned | create | `packages/d2b-contract-tests/tests/policy_spec_vocabulary.rs` |
| `ADR046-streamline-006` | `streamline` | Planned | create | `packages/d2b-resource-store-redb/tests/provider_state_graph.rs` (or the eventual crate implementing Zone resource storage) |
| `ADR046-streamline-007` | `streamline` | Planned | adapt | `packages/d2b-contract-tests/tests/policy_effectport_boundary.rs` |
| `ADR046-streamline-008` | `streamline` | Planned | create | `packages/d2b-contract-tests/tests/policy_work_items.rs` |
| `ADR046-streamline-009` | `streamline` | Planned | create | `docs/specs/ADR-046-provider-catalog.md` (generated |
| `ADR046-streamline-010` | `streamline` | Planned | adapt | `tests/tools/reconcile-stale-base.sh` (reporting only) plus a documented `git town sync`/`git town` restack procedure this report feeds into |
| `ADR046-streamline-011` | `streamline` | Planned | create | `packages/xtask/src/bin/handoff_manifest.rs` (schema/validator only) |
| `ADR046-streamline-012` | `streamline` | Planned | create | `tests/tools/import-task-db-consistency.sh` |
| `ADR046-streamline-013` | `streamline` | Planned | adapt | `tests/tools/anti-serialization-report.sh` |
| `ADR046-streamline-014` | `streamline` | Planned | adapt | `tests/tools/run-layer.sh` extension (this repository already has `tests/tools/run-layer.sh` and `layer1-jobs.py` bounded-parallelism precedent) plus fake `EffectPort`/`ResourceClient` stub crates under `packages/d2b-provider-toolkit-fakes/` |
| `ADR046-streamline-015` | `streamline` | Planned | adapt | Shared `packages/xtask` regeneration-conflict-detection helper consumed by every `gen-*`/`spec-registry` subcommand |
| `ADR046-streamline-016` | `streamline` | Planned | create | `packages/d2b-contract-tests/tests/policy_no_leaked_decision_prefix.rs` |
| `ADR046-streamline-017` | `streamline` | Planned | adapt | `docs/specs/ADR-046-streamline-evidence-commands.md` (a follow-up artifact outside this task's file scope |
| `ADR046-streamline-018` | `streamline` | Planned | adapt | `tests/tools/worktree-disk-report.sh` |
| `ADR046-streamline-019` | `streamline` | Planned | create | `packages/xtask/src/bin/terminology_check.rs` (`cargo run -p xtask -- terminology-check`) |
| `ADR046-streamline-020` | `streamline` | Planned | create | `packages/d2b-contract-tests/tests/policy_test_placement.rs` |
| `ADR046-streamline-021` | `streamline` | Planned | create | `packages/d2b-contract-tests/tests/policy_test_determinism.rs` |
| `ADR046-streamline-022` | `streamline` | Planned | adapt | `packages/xtask/src/test_runtime_ledger.rs` (shared with `ADR046-delivery-007`) |
| `ADR046-streamline-023` | `streamline` | Planned | adapt | `packages/xtask/src/bin/legacy_test_retirement.rs` (`cargo run -p xtask -- legacy-test-retirement`) |
| `ADR046-streamline-024` | `streamline` | Planned | create | `packages/xtask/src/bin/implementation_graph.rs` (`cargo run -p xtask -- implementation-graph`) |

---

## Cross-cutting obligations that no wave may drop

These are set-wide and are not owned by any single work item, so they are the easiest detail
to lose. Each must be checked at every wave seal, not only at release.

### Gate 0 is standing, not one-time

| Condition | Current state |
| --- | --- |
| All 55 members `Status: Accepted` | 55 / 55 Accepted |
| Decision register has zero open decisions | 129 resolved, 0 open |
| `spec-set.json` and `work-items.json` in exact bijection with the Markdown | Enforced by `make test-drift` |
| Both human review gates on the spec PR | Satisfied at set acceptance |

Gate 0 is **re-evaluated, not waived**, whenever a member's content changes. Any specification
amendment made during implementation re-opens it and invalidates affected validation and panel
evidence.

### The 129 frozen decisions bind implementation

The decision register is normative. Decisions carry through into code and lints, for example:
D024 (implementation requires a separate request), D077 (no Provider process imports the
broker), D086 and D087 (no bootstrap state Volume), D094 (hermetic execution budgets and
placement), D099 (schema-version and migration rule), D103 and D104 and D108 (spec-literal
lints), D116 (envelope `defaultUserRef`), D128 (the failed RSS spike and its four corrections).

A change that contradicts a decision is a specification amendment, not an implementation
choice.

### The 19 ResourceTypes have exclusive owners

| Owning spec | Count | Types |
| --- | --- | --- |
| `resources-zone-control` | 9 | `Zone`, `ZoneLink`, `Provider`, `Role`, `RoleBinding`, `Quota`, `EmergencyPolicy`, `ResourceExport`, `ResourceImport` |
| `resources-host-guest-process-user` | 6 | `Host`, `Guest`, `Process`, `EphemeralProcess`, `User`, `Endpoint` |
| `resources-volume` | 1 | `Volume` |
| `resources-network` | 1 | `Network` |
| `resources-device` | 1 | `Device` |
| `resources-credential` | 1 | `Credential` |

Foundation specs define shared contracts but must not co-own a type. One spec owns each
serialized contract, state machine, ResourceType, controller, Provider dossier, process model,
and security invariant.

### Hard targets are completion gates, not aspirations

Every numeric target in the validation matrix must be met by design change if missed; never by
weakening durability, authorization, or audit, and never by adding a sleep, a timeout, or an
`#[ignore]`.

| Target | Value |
| --- | --- |
| Empty-store readiness | <= 500 ms |
| p95 local Get, bounded List | <= 2 ms |
| p95 crash-safe single-resource mutation | <= 10 ms |
| p95 durable commit to controller handler start | <= 5 ms |
| p95 ready Process commit to launch-attempt start | <= 20 ms |
| Whole-process RSS, no baseline subtraction | <= 24,576 KiB (**currently 25,216 - the one failure**) |
| Aggregate idle RSS | <= 64 MiB |
| `Provider/system-core` / `Provider/system-minijail` | 22 MiB / 12 MiB |
| Per-Provider-crate hermetic suite, aggregate process CPU p95 | <= 3 s |
| Scale fixtures | 10,000 resources; 100 live watches |

### Contended files need integrator prep, not parallel edits

`packages/d2b-contracts/src/v3/volume.rs`; the `packages/Cargo.toml` member list; the
`flake.nix` output list; `nixos-modules/index.nix` and `default.nix`;
`packages/d2b-contract-tests/tests/workspace_policy.rs`; and `docs/specs/ADR-046-spec-set.json`
plus `ADR-046-work-items.json` (integrator-only, last commit of each wave).

`CHANGELOG.md` is resolved differently: **no slice edits it**. Each writes one
`changelog.d/<branch>.md` fragment and the integrator folds them at wave close.

### Deletion is a three-part obligation

For every `RETAIN`, `ADAPT`, `REPLACE`, or `DELETE` path:

1. the successor is integrated and its wave's exit criteria are met, covered by the work
   item's `Validation` tests;
2. the named `removalProof` passes on the candidate snapshot;
3. the deletion is **its own commit**, tagged with the wave and finding that proved the
   successor, never bundled with the commit that landed the successor - so reverting one does
   not silently revert the other.

Migration-map dispositions: 94 `ADAPT`, 32 `REPLACE`, 19 `RETAIN`, 16 `DELETE`. Of the DELETE
rows, 12 name a successor (parity-enforced, FR-041) and 4 do not (explicit retirement, FR-042).
The map supplies explicit removal proofs for only 3 of the 16, so the remaining proofs are
authored with the wave that removes each path.

---

## Detail-preservation checklist

Run this against `tasks.md` before implementation starts.

- [x] Every one of the 531 `Planned` work-item ids appears in `tasks.md` exactly once
      (verified: exact bijection, 0 duplicates, 0 missing, 0 extra)
- [x] Each task **references** its item's authoritative manifest entry and does not paraphrase
      it. **Correction (2026-07-29)**: an earlier draft of this line required
      `detailedDesign` and `validation` to be copied verbatim *into* `tasks.md`. That was
      wrong and inconsistent with this document's own reasoning - duplicating 531 manifest
      entries into Markdown creates exactly the unchecked second source of truth that the
      plan refuses to create elsewhere. The rule is **never paraphrase**; referencing the
      authoritative bytes satisfies it, copying them does not improve on it.
- [x] Each task lists its item's first destination path and points at the full list
- [x] Each task records its `reuseAction`
- [ ] Before starting a task, the implementer retrieves the full manifest entry and treats
      `detailedDesign`, `validation`, and `removalProof` as the task definition
- [x] `dependencyOwner` edges are represented: 91 items are marked free-to-start, the rest
      wait on a named predecessor
- [x] Wave assignment matches the implementation graph, with no item moved between waves
- [x] Parallel groups are preserved so file-disjoint slices launch together (FR-028)
- [x] The 14 `file-overlap-order` edges are recorded as explicit ordering constraints
- [ ] No task contradicts a decision in the register (checked per task at implementation time,
      per FR-047)
- [x] Contended files are integrator-prep, not slice-owned
---

## Requirement traceability (FR/SC to ADR-046 owners)

Answers CHK040 and CHK041. Every functional requirement maps to at least one owning spec and
its work-item prefix, **or is explicitly marked as locally added** so that a locally invented
requirement can never masquerade as an upstream obligation.

Work-item prefixes below are the `ADR046-<prefix>-NNN` families in the manifest; retrieve a
family with:

```bash
jq -r --arg p routing '.items[] | select(.workItemId | startswith("ADR046-\($p)-")) | .workItemId' \
  docs/specs/ADR-046-work-items.json
```

| FR range | Concern | Owning spec(s) | Work-item prefixes |
| --- | --- | --- | --- |
| FR-001 - FR-003 | Declare, record durably, reconcile, survive restart | `resource-object-model`, `resource-store-redb`, `resource-reconciliation` | `object`, `store`, `reconcile` |
| FR-004 | Stale-view rejection, concurrent modification | `resource-store-redb`, `resource-api-and-authorization` | `store`, `api` |
| FR-005 | Dependency-safe retirement, no broad sweep | `resource-reconciliation`, `core-controllers` | `reconcile`, `core` |
| FR-006 | Effect never released before proven durable commit | `resource-reconciliation`, `core-controllers` | `reconcile`, `core` |
| FR-007 | Authorization on proven identity; denials audited | `resource-api-and-authorization` | `api`, `security` |
| FR-008 | Single-owner authenticated session; no self-named subject | `componentsession-and-bus` | `session`, `bus` |
| FR-009 | Cross-Zone default-deny; explicit linking only | `zone-routing`, `resources-zone-control` | `routing`, `zone-control` |
| FR-010 - FR-015 | Provider install, supervise, attribute, re-adopt, isolate | `provider-model-and-packaging`, `provider-state`, `components-processes-and-sandbox` | `provider`, `pstate`, `process`, `primitives` |
| FR-016 - FR-017 | Operator inspection; actionable failure reasons | `cli-and-operations` | `cli`, `doctor` |
| FR-018 | Redaction and bounded label cardinality | `telemetry-audit-and-support` | `telem`, `audit`, `otel` |
| FR-019 | Docs ship with behavior | `validation-and-delivery` | `delivery` |
| FR-020 - FR-022 | Cutover preview, consent, hold, preservation, rollback | `reset-and-cutover` | `reset` |
| FR-023 | Removal only after successor plus proof, own commit | `current-code-migration-map`, `streamline` | `streamline`, `reuse` |
| FR-024 | No dual control plane in the release | `reset-and-cutover`, `streamline` | `reset`, `streamline` |
| FR-025 - FR-033 | Wave gating, seal, evidence, anti-serialization, semaphore, drift, test layers, suite retirement | `validation-and-delivery` | `delivery` |
| FR-034 - FR-036 | W0/W1 waiver, sealed delivery from W2, entry check | `validation-and-delivery` (§4 entry/exit) | `delivery` |
| FR-037 - FR-038 | Deliver W2-W8; satisfy the six-condition release gate | `validation-and-delivery` §15 | `delivery` |
| FR-039 - FR-040 | Companion compatibility as a release blocker | **Locally added** - no ADR-046 owner | none |
| FR-041 - FR-042 | Parity where a successor was promised; explicit retirement otherwise | `current-code-migration-map` | `streamline`, `reuse` |
| FR-043 | Recovery-point attestation before the irreversible phase | **Locally added** - extends `reset-and-cutover` | `reset` (extends) |
| FR-044 - FR-045 | Gated pull-request landing; no intermediate release | `validation-and-delivery` §13 | `delivery` |
| FR-046 | Generated manifests authoritative over prose | **Locally added** - applies the repository's existing-code-is-canon rule | none |
| FR-047 | Conformance to the 129 frozen decisions | `decision-register` | `decisions` |

### Locally added requirements

Four requirements have no upstream ADR-046 owner. Each came from a recorded clarification
decision, not from the specification set, and each is therefore **this program's own
obligation** rather than an inherited one:

| Requirement | Origin | Consequence |
| --- | --- | --- |
| FR-039, FR-040 | Clarification: companions block the release | Adds a release-gate condition the ADR-046 set does not contain. If it is ever dropped, it must be dropped here, not looked for upstream. |
| FR-043 | Clarification: recovery-point attestation required | Tightens `reset-and-cutover`. The owning spec permits proceeding past the rollback boundary without attestation; this program does not. |
| FR-046 | Applies the repository's existing-code-is-canon rule to spec-versus-manifest drift | Governs the recorded W2 destination drift. |

A reviewer checking upstream fidelity should expect these four to have no counterpart in
`docs/specs/`. That is intended, not a coverage gap.

### Success-criteria traceability

| SC range | Traces to |
| --- | --- |
| SC-001 - SC-005 | FR-001, FR-002, FR-011, FR-016, FR-017; validation matrix rows for CLI and reconcile |
| SC-006 - SC-010 | FR-003, FR-006, FR-007, FR-008, FR-009, FR-018; restart/power-loss and redaction matrices |
| SC-011 - SC-013 | The hard numeric targets; `resource-store-redb` and `feasibility-and-spikes` |
| SC-014 - SC-018 | FR-020 - FR-024, FR-041, FR-042; `reset-and-cutover` and `streamline` |
| SC-019 - SC-023, SC-026 | FR-025 - FR-038, FR-044, FR-045; `validation-and-delivery` §4 and §15 |
| SC-024 | FR-039, FR-040 - locally added |
| SC-025 | FR-043 - locally added |
