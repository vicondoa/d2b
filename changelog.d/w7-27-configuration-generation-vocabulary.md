### Fixed

- The shared contract crates carry family-neutral vocabulary again: the
  activation-runner `targetGeneration` field and its constructor take
  the existing `ConfigurationGeneration` identity instead of a new
  activation-nixos-specific ordinal type declared beside it. The field
  keeps its nonzero invariant, its serde-transparent u64 wire bytes,
  and the same definition-reference rendering in the committed
  EphemeralProcess schema.
