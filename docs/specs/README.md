# ADR specification sets

This directory contains the surviving normative specifications for
[ADR 0046](../adr/0046-d2b-3-provider-control-plane.md). The parent decision
and these Markdown files are the source of truth for the planned provider
control plane. Contributor workflow, delivery records, and generated
validation indexes are not part of this set.

## Foundation and platform

- [Decision register](ADR-046-decision-register.md)
- [Terminology and identities](ADR-046-terminology-and-identities.md)
- [Resource object model](ADR-046-resource-object-model.md)
- [Resource store](ADR-046-resource-store-redb.md)
- [Resource API and authorization](ADR-046-resource-api-and-authorization.md)
- [Resource reconciliation](ADR-046-resource-reconciliation.md)
- [Primitive resource composition](ADR-046-primitive-resource-composition.md)
- [ComponentSession and bus](ADR-046-componentsession-and-bus.md)
- [Zone routing](ADR-046-zone-routing.md)
- [Provider model and packaging](ADR-046-provider-model-and-packaging.md)
- [Provider state](ADR-046-provider-state.md)
- [Core controllers](ADR-046-core-controllers.md)
- [Components, processes, and sandbox](ADR-046-components-processes-and-sandbox.md)
- [Nix configuration](ADR-046-nix-configuration.md)
- [Current-code migration map](ADR-046-current-code-migration-map.md)
- [Feasibility and spikes](ADR-046-feasibility-and-spikes.md)
- [Reset and cutover](ADR-046-reset-and-cutover.md)

## Resource catalog

- [Zone control resources](ADR-046-resources-zone-control.md)
- [Host, guest, process, and user resources](ADR-046-resources-host-guest-process-user.md)
- [Volume resources](ADR-046-resources-volume.md)
- [Network resources](ADR-046-resources-network.md)
- [Device resources](ADR-046-resources-device.md)
- [Credential resources](ADR-046-resources-credential.md)

## Cross-cutting contracts

- [CLI and operations](ADR-046-cli-and-operations.md)
- [Telemetry, audit, and support](ADR-046-telemetry-audit-and-support.md)
- [Security and threat model](ADR-046-security-and-threat-model.md)

## Provider dossiers

The installed Provider dossiers and their owned ResourceTypes are indexed in
[`providers/README.md`](providers/README.md).

## Required metadata

Each specification begins with a metadata table containing:

| Field | Meaning |
| --- | --- |
| Spec ID | Stable `ADR-046-<spec-name>` identifier |
| Parent | `ADR 0046` |
| Status | `Proposed` or `Accepted` |
| Version | Monotonic integer |
| Baseline | Exact source revision analyzed |
| Normative | `Yes` |
| Owners | Contract and future implementation owners |
| Depends on | Exact ADR 0046 specification IDs |
| Supersedes | Prior specification/version, if any |

## Evidence and current-code fit

Claims distinguish implemented-and-reachable, implemented-but-unwired,
generated-or-eval-contract, test-only-or-preview, ADR-only, and
unknown-requires-spike behavior. Current behavior is cited by exact source,
symbol, and test evidence. Each design section records the retained behavior,
required delta, reuse path, replacement or deletion condition, feasibility
proof, and future owner.

Implementation work items identify their owner, current source, destination,
design, integration path, data migration, validation, removal proof, state,
and evidence. An accepted design does not imply that every implementation item
has landed.

## Resource terminology

A resource belongs to exactly one Zone. A canonical resource reference is:

```text
<ResourceType>/<resource_name>
```

Fields ending in `Ref` contain a canonical same-Zone resource reference.
Standard ResourceTypes use Zone-unique short names; vendor ResourceTypes use a
qualified name. Cross-Zone references require an explicitly reviewed contract.
