### Changed

- Resource reconciliation scans the obsolete-child set by key only, without
  cloning full stored rows, and hex digests render into a pre-sized string
  instead of allocating a fresh string per byte.
- The resource runtime no longer depends on `parking_lot` at runtime; its
  test-only uses moved to dev-dependencies.