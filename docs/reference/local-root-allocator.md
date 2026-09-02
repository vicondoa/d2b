# Local-root allocator

**Diataxis category:** historical reference.

This page records the retired `d2b-realm-core` allocator proposal. It is not a
current daemon, broker, or Zone Resource API. No current configuration should
declare a Realm allocator or rely on this document for host mutation.

The current control plane uses Zone-scoped Resource controllers, broker-owned
host effects, anchored paths, OFD locks, and one named repair owner for every
mutable surface. See:

- [`../explanation/daemon-lifecycle.md`](../explanation/daemon-lifecycle.md)
- [`./manifest-bundle.md`](./manifest-bundle.md)
- [`./zone-control-nix.md`](./zone-control-nix.md)
- [ADR 0034](../adr/0034-storage-lifecycle-restart-and-synchronization.md)

Historical allocator types may remain in the standalone prototype or ADR
material for migration context; they do not authorize a current operation.
