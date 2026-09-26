### Fixed

- The `index.json` reader now refuses an undeclared top-level key. Like
  the sibling index types, the index document denies unknown fields, and
  it declares the `schemaVersion`, `executionIndex`, `networkIndex`, and
  `closureIndex` keys the emitter writes, so an emitted document still
  loads while a body carrying a foreign top-level key is rejected as an
  `index.json` parse error. The declared-but-unread keys stay optional,
  so a partial index that omits them still loads.
