# ADR 0046 implementation graph (generated)

> **Generated index — not a normative member.** This file and its companion
> [`ADR-046-implementation-graph.json`](ADR-046-implementation-graph.json) are
> deterministically generated from
> [`ADR-046-spec-set.json`](ADR-046-spec-set.json),
> [`ADR-046-work-items.json`](ADR-046-work-items.json), and the 8-wave topology
> in [`ADR-046-validation-and-delivery.md` §3](ADR-046-validation-and-delivery.md).
> They are **not** among the 55 `ADR-046-spec-set.json` members. This Proposed
> documentation set does not yet contain the future `xtask` generator; its
> disposable regeneration script was removed after producing these bytes.

The graph maps every member spec and every work item exactly once to a
dependency-ordered launch wave (`W0`–`W7`) and a file-disjoint parallel group.
It includes every resolved security-key work-item dependency; no lexical
tie-break or omitted dependency is used.

## Counts

| Metric | Value |
| --- | --- |
| Waves | 8 |
| Spec nodes | 55 |
| Work-item nodes | 543 |
| Total nodes | 598 |
| Edges | 1908 |
| Max topological rank | 22 |

## Waves (W0–W7)

| Wave | Specs | #Specs | #Work items | Parallel groups |
| --- | --- | --- | --- | --- |
| W0 | current-code-migration-map, decision-register, resource-api-and-authorization, resource-object-model, resource-store-redb, terminology-and-identities | 6 | 10 | W0-foundation-chain, W0-reference-docs |
| W1 | componentsession-and-bus, resource-reconciliation | 2 | 6 | W1-reconcile-and-bus |
| W2 | primitive-resource-composition, zone-routing | 2 | 19 | W2-composition-and-routing |
| W3 | provider-model-and-packaging | 1 | 4 | W3-provider-contract |
| W4 | components-processes-and-sandbox, core-controllers, provider-state, resources-credential, resources-network | 5 | 32 | W4-parallel-specs |
| W5 | cli-and-operations, nix-configuration, resources-device, resources-host-guest-process-user, resources-volume, resources-zone-control, telemetry-audit-and-support | 7 | 141 | W5-parallel-specs |
| W6 | provider-activation-nixos, provider-audio-pipewire, provider-clipboard-wayland, provider-credential-entra, provider-credential-managed-identity, provider-credential-secret-service, provider-device-gpu, provider-device-security-key, provider-device-tpm, provider-device-usbip, provider-display-wayland, provider-network-local, provider-notification-desktop, provider-observability-otel, provider-runtime-azure-container-apps, provider-runtime-azure-virtual-machine, provider-runtime-cloud-hypervisor, provider-runtime-qemu-media, provider-shell-terminal, provider-system-core, provider-system-minijail, provider-system-systemd, provider-transport-azure-relay, provider-transport-unix, provider-transport-vsock, provider-volume-local, provider-volume-virtiofs | 27 | 257 | W6-credentials, W6-interaction, W6-storage-network-device, W6-system-host-guest, W6-transport-observability-activation |
| W7 | feasibility-and-spikes, reset-and-cutover, security-and-threat-model, streamline, validation-and-delivery | 5 | 74 | W7-closing |

## Dependency DAG (waves and prep barriers)

```mermaid
flowchart LR
  subgraph W0["W0"]
    current_code_migration_map["current-code-migration-map"]
    decision_register["decision-register"]
    resource_api_and_authorization["resource-api-and-authorization"]
    resource_object_model["resource-object-model"]
    resource_store_redb["resource-store-redb"]
    terminology_and_identities["terminology-and-identities"]
  end
  subgraph W1["W1"]
    componentsession_and_bus["componentsession-and-bus"]
    resource_reconciliation["resource-reconciliation"]
  end
  subgraph W2["W2"]
    primitive_resource_composition["primitive-resource-composition"]
    zone_routing["zone-routing"]
  end
  subgraph W3["W3"]
    provider_model_and_packaging["provider-model-and-packaging"]
  end
  subgraph W4["W4"]
    components_processes_and_sandbox["components-processes-and-sandbox"]
    core_controllers["core-controllers"]
    provider_state["provider-state"]
    resources_credential["resources-credential"]
    resources_network["resources-network"]
  end
  subgraph W5["W5"]
    cli_and_operations["cli-and-operations"]
    nix_configuration["nix-configuration"]
    resources_device["resources-device"]
    resources_host_guest_process_user["resources-host-guest-process-user"]
    resources_volume["resources-volume"]
    resources_zone_control["resources-zone-control"]
    telemetry_audit_and_support["telemetry-audit-and-support"]
  end
  subgraph W6["W6"]
    provider_activation_nixos["provider-activation-nixos"]
    provider_audio_pipewire["provider-audio-pipewire"]
    provider_clipboard_wayland["provider-clipboard-wayland"]
    provider_credential_entra["provider-credential-entra"]
    provider_credential_managed_identity["provider-credential-managed-identity"]
    provider_credential_secret_service["provider-credential-secret-service"]
    provider_device_gpu["provider-device-gpu"]
    provider_device_security_key["provider-device-security-key"]
    provider_device_tpm["provider-device-tpm"]
    provider_device_usbip["provider-device-usbip"]
    provider_display_wayland["provider-display-wayland"]
    provider_network_local["provider-network-local"]
    provider_notification_desktop["provider-notification-desktop"]
    provider_observability_otel["provider-observability-otel"]
    provider_runtime_azure_container_apps["provider-runtime-azure-container-apps"]
    provider_runtime_azure_virtual_machine["provider-runtime-azure-virtual-machine"]
    provider_runtime_cloud_hypervisor["provider-runtime-cloud-hypervisor"]
    provider_runtime_qemu_media["provider-runtime-qemu-media"]
    provider_shell_terminal["provider-shell-terminal"]
    provider_system_core["provider-system-core"]
    provider_system_minijail["provider-system-minijail"]
    provider_system_systemd["provider-system-systemd"]
    provider_transport_azure_relay["provider-transport-azure-relay"]
    provider_transport_unix["provider-transport-unix"]
    provider_transport_vsock["provider-transport-vsock"]
    provider_volume_local["provider-volume-local"]
    provider_volume_virtiofs["provider-volume-virtiofs"]
  end
  subgraph W7["W7"]
    feasibility_and_spikes["feasibility-and-spikes"]
    reset_and_cutover["reset-and-cutover"]
    security_and_threat_model["security-and-threat-model"]
    streamline["streamline"]
    validation_and_delivery["validation-and-delivery"]
  end
  W0 --> W1 --> W2 --> W3 --> W4 --> W5 --> W6 --> W7
  resource_object_model -. prep .-> resource_api_and_authorization
  resource_store_redb -. prep .-> resource_api_and_authorization
  terminology_and_identities -. prep .-> resource_api_and_authorization
  decision_register -. prep .-> resource_object_model
  terminology_and_identities -. prep .-> resource_object_model
  resource_object_model -. prep .-> resource_store_redb
  terminology_and_identities -. prep .-> resource_store_redb
  reset_and_cutover -. prep .-> streamline
  decision_register -. prep .-> terminology_and_identities
  feasibility_and_spikes -. prep .-> validation_and_delivery
  reset_and_cutover -. prep .-> validation_and_delivery
  security_and_threat_model -. prep .-> validation_and_delivery
  streamline -. prep .-> validation_and_delivery
```

Solid arrows show wave launch order. Dotted arrows show same-wave shared
contract prep. Work-item dependencies and actual file-overlap barriers are
fully represented in the JSON.

## Shared prep and file-overlap barriers

| Prerequisite | Consumer | Type |
| --- | --- | --- |
| `ADR046-nix-014` | `ADR046-cli-011` | file-overlap-order |
| `ADR046-core-001` | `ADR046-device-007` | file-overlap-order |
| `ADR046-core-001` | `ADR046-exec-013` | file-overlap-order |
| `ADR046-core-001` | `ADR046-exec-015` | file-overlap-order |
| `ADR046-core-001` | `ADR046-network-008` | file-overlap-order |
| `ADR046-device-006` | `ADR046-nix-014` | file-overlap-order |
| `ADR046-cli-011` | `ADR046-nix-019` | file-overlap-order |
| `ADR046-nix-019` | `ADR046-nix-031` | file-overlap-order |
| `ADR046-transport-unix-009` | `ADR046-qemu-media-017` | file-overlap-order |
| `ADR046-core-001` | `ADR046-telem-011` | file-overlap-order |
| `ADR046-gpu-007` | `ADR046-transport-unix-009` | file-overlap-order |
| `ADR046-qemu-media-017` | `ADR046-usbip-008` | file-overlap-order |
| `ADR046-core-001` | `ADR046-zone-control-016` | file-overlap-order |
| `ADR046-core-001` | `ADR046-zone-control-021` | file-overlap-order |
| `ADR-046-resource-object-model` | `ADR-046-resource-api-and-authorization` | shared-contract |
| `ADR-046-resource-store-redb` | `ADR-046-resource-api-and-authorization` | shared-contract |
| `ADR-046-terminology-and-identities` | `ADR-046-resource-api-and-authorization` | shared-contract |
| `ADR-046-decision-register` | `ADR-046-resource-object-model` | shared-contract |
| `ADR-046-terminology-and-identities` | `ADR-046-resource-object-model` | shared-contract |
| `ADR-046-resource-object-model` | `ADR-046-resource-store-redb` | shared-contract |
| `ADR-046-terminology-and-identities` | `ADR-046-resource-store-redb` | shared-contract |
| `ADR-046-reset-and-cutover` | `ADR-046-streamline` | shared-contract |
| `ADR-046-decision-register` | `ADR-046-terminology-and-identities` | shared-contract |
| `ADR-046-feasibility-and-spikes` | `ADR-046-validation-and-delivery` | shared-contract |
| `ADR-046-reset-and-cutover` | `ADR-046-validation-and-delivery` | shared-contract |
| `ADR-046-security-and-threat-model` | `ADR-046-validation-and-delivery` | shared-contract |
| `ADR-046-streamline` | `ADR-046-validation-and-delivery` | shared-contract |

Only the listed `file-overlap-order` edges constrain shared files. Provider
integration ordering that touches disjoint crate trees is not represented as
file overlap. The former `wi:core-config-hub` is split into
`wi:core-config-hub:w4` and `wi:core-config-hub:w5`; each parallel group is
single-wave. The seven `assertions.nix` edges form the minimal per-wave chains
`ADR046-device-006` → `ADR046-nix-014` → `ADR046-cli-011` →
`ADR046-nix-019` → `ADR046-nix-031` in W5 and `ADR046-gpu-007` →
`ADR046-transport-unix-009` → `ADR046-qemu-media-017` →
`ADR046-usbip-008` in W6. W2 has one writer. These edges order only the shared
file; all other destinations retain their existing parallelism.

## Parallel groups

| Parallel group | Wave | #Nodes |
| --- | --- | --- |
| `W0-foundation-chain` | W0 | 4 |
| `W0-reference-docs` | W0 | 2 |
| `W1-reconcile-and-bus` | W1 | 2 |
| `W2-composition-and-routing` | W2 | 2 |
| `W3-provider-contract` | W3 | 1 |
| `W4-parallel-specs` | W4 | 5 |
| `W5-parallel-specs` | W5 | 7 |
| `W6-credentials` | W6 | 3 |
| `W6-interaction` | W6 | 5 |
| `W6-storage-network-device` | W6 | 7 |
| `W6-system-host-guest` | W6 | 7 |
| `W6-transport-observability-activation` | W6 | 5 |
| `W7-closing` | W7 | 5 |
| `wi:ADR-046-cli-and-operations` | W5 | 13 |
| `wi:ADR-046-components-processes-and-sandbox` | W4 | 2 |
| `wi:ADR-046-componentsession-and-bus` | W1 | 3 |
| `wi:ADR-046-core-controllers` | W4 | 1 |
| `wi:ADR-046-decision-register` | W0 | 1 |
| `wi:ADR-046-feasibility-and-spikes` | W7 | 11 |
| `wi:ADR-046-nix-configuration` | W5 | 35 |
| `wi:ADR-046-primitive-resource-composition` | W2 | 3 |
| `wi:ADR-046-provider-activation-nixos` | W6 | 7 |
| `wi:ADR-046-provider-audio-pipewire` | W6 | 13 |
| `wi:ADR-046-provider-clipboard-wayland` | W6 | 12 |
| `wi:ADR-046-provider-credential-entra` | W6 | 1 |
| `wi:ADR-046-provider-credential-managed-identity` | W6 | 5 |
| `wi:ADR-046-provider-credential-secret-service` | W6 | 6 |
| `wi:ADR-046-provider-device-gpu` | W6 | 9 |
| `wi:ADR-046-provider-device-security-key` | W6 | 35 |
| `wi:ADR-046-provider-device-tpm` | W6 | 13 |
| `wi:ADR-046-provider-device-usbip` | W6 | 9 |
| `wi:ADR-046-provider-display-wayland` | W6 | 4 |
| `wi:ADR-046-provider-model-and-packaging` | W3 | 4 |
| `wi:ADR-046-provider-network-local` | W6 | 20 |
| `wi:ADR-046-provider-notification-desktop` | W6 | 6 |
| `wi:ADR-046-provider-observability-otel` | W6 | 6 |
| `wi:ADR-046-provider-runtime-azure-container-apps` | W6 | 7 |
| `wi:ADR-046-provider-runtime-azure-virtual-machine` | W6 | 9 |
| `wi:ADR-046-provider-runtime-cloud-hypervisor` | W6 | 7 |
| `wi:ADR-046-provider-runtime-qemu-media` | W6 | 19 |
| `wi:ADR-046-provider-shell-terminal` | W6 | 13 |
| `wi:ADR-046-provider-state` | W4 | 12 |
| `wi:ADR-046-provider-system-core` | W6 | 1 |
| `wi:ADR-046-provider-system-minijail` | W6 | 6 |
| `wi:ADR-046-provider-system-systemd` | W6 | 3 |
| `wi:ADR-046-provider-transport-azure-relay` | W6 | 7 |
| `wi:ADR-046-provider-transport-unix` | W6 | 11 |
| `wi:ADR-046-provider-transport-vsock` | W6 | 7 |
| `wi:ADR-046-provider-volume-local` | W6 | 13 |
| `wi:ADR-046-provider-volume-virtiofs` | W6 | 7 |
| `wi:ADR-046-reset-and-cutover` | W7 | 11 |
| `wi:ADR-046-resource-api-and-authorization` | W0 | 2 |
| `wi:ADR-046-resource-object-model` | W0 | 2 |
| `wi:ADR-046-resource-reconciliation` | W1 | 3 |
| `wi:ADR-046-resource-store-redb` | W0 | 3 |
| `wi:ADR-046-resources-credential` | W4 | 8 |
| `wi:ADR-046-resources-device` | W5 | 7 |
| `wi:ADR-046-resources-host-guest-process-user` | W5 | 22 |
| `wi:ADR-046-resources-network` | W4 | 8 |
| `wi:ADR-046-resources-volume` | W5 | 6 |
| `wi:ADR-046-resources-zone-control` | W5 | 26 |
| `wi:ADR-046-security-and-threat-model` | W7 | 19 |
| `wi:ADR-046-streamline` | W7 | 24 |
| `wi:ADR-046-telemetry-audit-and-support` | W5 | 26 |
| `wi:ADR-046-terminology-and-identities` | W0 | 2 |
| `wi:ADR-046-validation-and-delivery` | W7 | 9 |
| `wi:ADR-046-zone-routing` | W2 | 16 |
| `wi:core-config-hub:w4` | W4 | 1 |
| `wi:core-config-hub:w5` | W5 | 6 |
| `wi:core-controller-coordination:w6` | W6 | 1 |

## Critical path (longest dependency chain)

1. `ADR-046-decision-register`
2. `ADR-046-terminology-and-identities`
3. `ADR-046-resource-object-model`
4. `ADR-046-resource-store-redb`
5. `ADR-046-resource-api-and-authorization`
6. `ADR-046-resource-reconciliation`
7. `ADR-046-primitive-resource-composition`
8. `ADR-046-provider-model-and-packaging`
9. `ADR-046-core-controllers`
10. `ADR-046-nix-configuration`
11. `ADR-046-provider-runtime-qemu-media`
12. `ADR046-qemu-media-001`
13. `ADR046-qemu-media-003`
14. `ADR046-qemu-media-005`
15. `ADR046-qemu-media-006`
16. `ADR046-qemu-media-009`
17. `ADR046-qemu-media-010`
18. `ADR046-qemu-media-011`
19. `ADR046-qemu-media-013`
20. `ADR046-qemu-media-014`
21. `ADR046-qemu-media-015`
22. `ADR046-qemu-media-018`
23. `ADR046-qemu-media-019`

## Regeneration findings (D095–D098)

- Regenerated from 55 member specs and 543 current work items; every declared heading is represented exactly once.
- `ADR046-provider-004` owns the common D098 Service/Binding base DTOs and schemas; the four implementation Providers own only strict extensions and controllers.
- `ADR046-zone-control-024` owns the shared Core-derived `physical-usb-backing` tuple; both the security-key and USB effect DAGs depend on it.
- Every `ADR046-security-key-*` dependency in `Dependency/owner` is encoded. The dependency subgraph is acyclic and uses no generator tie-break.
- Fourteen file-overlap barriers cover only the shared core
  configuration/cleanup files and `nixos-modules/assertions.nix`. Each appears
  both as a
  `file-overlap-order` edge and in the dependent node's `prerequisites`, so the
  ready-wave query enforces it. Soft cross-Provider integration order remains
  file-disjoint and concurrent.
- No repository generator exists at this Proposed stage. `ADR046-streamline-001`/`024` and `ADR046-delivery-004`/`009` own the future canonical implementation and policy gate.

## Ready-wave algorithm

A node is ready when every id in `prerequisites` is done:

```bash
jq --argjson done "$DONE" '
  .nodes[] | select((.prerequisites - $done) | length == 0)
  | {id, kind, wave, parallelGroup, topologicalRank}
' docs/specs/ADR-046-implementation-graph.json
```

A ready, file-disjoint group left unlaunched without a recorded blocker violates
the anti-serialization invariant.

## References

- [ADR 0046](../adr/0046-d2b-3-provider-control-plane.md)
- [Decision register](ADR-046-decision-register.md)
- [Validation and delivery](ADR-046-validation-and-delivery.md)
- [Spec-set manifest](ADR-046-spec-set.json)
- [Work-item manifest](ADR-046-work-items.json)
