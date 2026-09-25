### Changed

- Credential delivery records now erase and copy plaintext with relaxed
  atomic ordering instead of needless sequentially-consistent fences.

### Fixed

- The strict Credential wire codec is now unit-tested for round-trip,
  truncation, duplicate-field, non-canonical-varint, and oversize behavior,
  and the Credential controller decision paths are now unit-tested for
  observe, rotation-retry-exhausted, lease-aggregate, health, and
  controller-audit-event behavior.