### Added

- Rewire the Resource API onto the per-Zone ResourceManager (U8): a
  manager-backed `ResourceStoreBackend` resolves reads, mutations, and
  external WATCH through the single-writer manager while the public
  operation shapes and wire envelopes stay intact; resource generation maps
  onto the existing Exact-revision wire preconditions and external WATCH
  carries epoch+sequence runtime revisions with `RevisionExpired` relist
  semantics (R23-R25).
- Compile the `PolicySet`, role bindings, and bootstrap facts from durable
  Role / RoleBinding / User / Provider spec rows plus the Nix bundle
  generation (KTD6), with the one-way bootstrap latch: a published policy
  revision permanently disables bootstrap and a provisioned Zone never
  re-enters Unprovisioned (R28).
