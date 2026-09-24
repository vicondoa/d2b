# Historical realm-core reference

**Diataxis category:** historical reference.

This page is retained for migration and ADR traceability. The standalone
`d2b-realm-core` model is not part of the current shared Zone or Guest graph.
Its former identifiers, routing model, and allocator types do not authorize
current operations.

The `packages/d2b-realm-core` crate was deleted from the tree on 2026-09-23
(after this page was written it had no workspace consumers left; the lab
proxy that still needed `WorkloadProviderKind` now defines the type locally).

Use the current contracts instead:

- [`zone-control-nix.md`](./zone-control-nix.md)
- [`zone-cli-contract.md`](./zone-cli-contract.md)
- [`../explanation/daemon-lifecycle.md`](../explanation/daemon-lifecycle.md)
- [`../reference/manifest-bundle.md`](./manifest-bundle.md)
