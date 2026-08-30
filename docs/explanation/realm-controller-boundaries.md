# Historical realm controller boundaries

**Diataxis category:** historical explanation.

This page preserves the predecessor realm-controller design. It is not a
current lifecycle or host-mutation contract.

The current boundary is the Zone Resource plane: d2bd supervises Zone
runtimes, the Guest controller owns direct child Resources, specialized
controllers own effects, and the broker owns approved host mutation.

See [`daemon-lifecycle.md`](./daemon-lifecycle.md) and
[`../reference/zone-control-nix.md`](../reference/zone-control-nix.md).
