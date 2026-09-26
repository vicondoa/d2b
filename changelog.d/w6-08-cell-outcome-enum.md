### Changed

- The durable cell-store record now carries the typed outcome enum instead of a
  hand-matched string: the serialized spellings (`unknown` / `completed`) are
  unchanged, so existing durable files load as before and the format version
  stays at 1. An unknown outcome string in a durable file still fails closed
  as a corrupt store.