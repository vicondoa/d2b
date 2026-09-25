# `w3-10-contracts-resource-status-size.md`

### Changed

- Building a v3 `ResourceStatus` now enforces the 64 KiB status-size bound
  with a single serialization pass instead of re-canonicalizing the complete
  status after the resource layer was already serialized, so status writes
  pay one full serialization instead of two. The bound and its error
  behavior are unchanged; canonical validation still happens at the storage
  boundary where the status bytes are produced.