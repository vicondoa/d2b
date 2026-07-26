### Changed

- Made the resource-store dependency gate explicit: redb adoption and
  performance-sensitive store work remain blocked on unexecuted feasibility
  evidence, while engine-neutral codecs, table contracts, errors, and
  transaction semantics may proceed with small-scale hermetic tests.
- Froze the ten-table on-disk schema and codec discriminants, the closed store
  error mapping, and the source-versus-integrator ownership of generated
  storage-contract artifacts.
- Aligned label and annotation keys with the canonical JSON 64-byte object-key
  ceiling.
