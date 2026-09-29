### Fixed

- A typed `w1-swtpm` device-worker launch that resolves no trusted swtpm
  identity is now refused by name at the argv fence, instead of being admitted
  with the fence, the state-directory traverse grant and the NVRAM health check
  all skipped. The three controls key off the same identity, so admitting the
  launch dropped all three at once and spawned unfenced against a state
  directory that nothing had provisioned. Reachable whenever the daemon and the
  Volume controller disagree about that directory, which
  `resource_backed_identity` returns `None` for by design. The scope-less
  legacy path, which has no identity to fence against, is unchanged.
