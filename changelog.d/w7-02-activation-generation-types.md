### Changed

- The activation-runner `targetGeneration` contract field is typed as
  the nonzero `ConfigurationGeneration` identity instead of a bare
  `u64`, so a zero ordinal is refused when the input decodes rather
  than by a manual check in the constructor. The wire bytes are
  unchanged, and the committed EphemeralProcess schema now carries the
  ordinal as a definition reference instead of an inline minimum-1
  integer.
- The activation-nixos status `observedGeneration` field is typed as
  `ObservedGeneration` (zero meaning none) instead of a bare `u64`. The
  transparent newtype renders the same integer schema, and the two
  hand-committed activation-nixos schemas now carry its full rendering
  (`format: uint64`, `minimum: 0`).

### Removed

- `ActivationRunnerInputError` and its `GenerationInvalid` variant: the
  nonzero ordinal carries the invariant, so `ActivationRunnerInput::new`
  is infallible. The duplicated `target_generation == 0` guards in
  `d2b-process-conformance` and `d2b-provider-process` went with it.
