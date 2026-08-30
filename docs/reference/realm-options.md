# Historical realm options

**Diataxis category:** historical reference.

The former Realm option namespace is retired. New configuration must use
`d2b.zones.<zone>.resources.<name>`,
`d2b.guestSystems.<zone>.<guest>`, and `d2b.artifacts`.

The Nix module intentionally leaves the old option paths undeclared so NixOS
reports them through ordinary unknown-option behavior. Retain this page only
to explain that migration boundary.

See [`zone-control-nix.md`](./zone-control-nix.md) for the current authoring
contract.
