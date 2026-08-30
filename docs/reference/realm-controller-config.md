# Historical controller metadata

**Diataxis category:** historical reference.

`realm-controllers.json` is retained only as a transitional private artifact
for current daemon bridge compatibility. It is not a current hierarchy,
lifecycle authority, or public configuration surface.

Current ownership is:

- Zone resources and compiler-only topology are authored in Nix;
- the Guest controller owns direct child Resources and lifecycle;
- specialized Providers own effects; and
- the daemon and broker fence by Zone/Guest identity, generation, and
  revision.

See [`manifest-bundle.md`](./manifest-bundle.md) and
[`zone-control-nix.md`](./zone-control-nix.md).
