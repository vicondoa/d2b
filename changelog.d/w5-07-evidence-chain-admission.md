### Fixed

- `EvidenceChain` now deserializes through an admission gate that rejects an empty `identities` list, so a wire payload can no longer produce a chain whose `depth()` underflows or whose `initiating_identity()`/`invoking_identity()` accessors panic. The serialized wire shape is unchanged.