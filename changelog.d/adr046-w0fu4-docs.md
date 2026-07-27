### Fixed

- Brought every complete ADR-046 resource envelope in `docs/specs/**` into the
  D088/D107 three-layer status shape. Each envelope's `status` now carries both
  the universal `status.update` currency object and the `status.resource`
  ResourceType-common base (`{}` where the type declares no common fields), so
  the D107 "resource layer present on every resource" contract holds
  universally. ResourceType-specific status containers that sat directly under
  `status` (for example `device:`) are nested under `status.resource`, and the
  Endpoint base-status fields that were authored flat are grouped there too.
- Marked the D116 Nix counter-example in `ADR-046-nix-configuration.md` as an
  intentional negative example with an explicit
  `d2b-lint: expect-d116-eval-error` marker, so the eval-time-rejection
  teaching block is exempt from the `defaultUserRef` structural lint without
  weakening detection of real declarations.

### Changed

- Aligned much of the D094 test-runtime ledger prose across the decision
  register, validation/delivery §10.16, feasibility/spikes, streamline, and the
  generated implementation graph toward the ledger's actual scope: enforced
  aggregate per-crate process CPU, advisory per-test wall clock, no baseline,
  and no historical-regression claim. Growing the census to a real multi-crate
  shard inventory and adding a cross-machine reference baseline are recorded as
  the deferred follow-up `runtime-ledger-full-census-and-real-shards`.
  Deleted the synthetic `runtime-ledger-baseline.json`. Retired
  baseline/regression/shard references that survived in the code, the
  `Makefile`, and several docs are reconciled in a later follow-up.
