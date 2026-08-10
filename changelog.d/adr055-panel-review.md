### Changed

- Implemented ADR 0055, replacing open-ended panel rediscovery with one
  comprehensive discovery pass, an orchestrator-assigned stable issue ledger,
  batched fixes, implementation self-verification, and constrained verification
  review. The decision also preserves in-flight legacy review progress through
  automatic generated artifacts, adds the optional `build` expert under the
  existing reviewer floors, makes the standard Copilot panel skill the first
  implementation target, and requires future Gas City orchestration to consume
  the same selection and artifact contracts.
- Added request-bound selected-roster delivery validation with strict
  version-first current and legacy panel formats. Current artifacts carry
  `panel_format_version: 1` while legacy fixed-ten records remain readable
  without that field and retain `rust`; the workspace delivery schema remains
  version `2`.
- Defined approval CLI exit statuses as `0` for approved, `3` for a valid but
  blocked gate, and `2` for an invalid invocation or input.
- Made staging require finalized `--evidence` and derive the evidence-bound
  discovery request, while `adapt-discovery` now consumes an exact complete
  per-seat verdict directory and binds its output to the lifecycle, candidate,
  and selection bytes.
- Added bounded input reads and create-or-compare publication for panel
  artifacts. Current schema-version `4` completion markers byte-bind the
  canonical packet including selected agent definitions and the projected
  dispatch policy. Schema-version `2` predecessors remain readable only as
  either exact historical base artifacts without definitions or that same set
  plus every selected definition, with no dispatch binding; schema-version `3`
  requires exactly the definition-bound set, also without dispatch binding.
  Arbitrary subsets are rejected.
- Added the canonical blocked-verification continuation handoff, which
  promotes admitted late findings and nonpassing issue responses into
  independently published create-or-compare ledger and response files without
  reopening discovery. Consumers require both files before use, and a
  byte-identical retry can complete a partial publication.
- Added full completion validation before discovery uniqueness blocks a
  lifecycle, so marker-only or incomplete scratch packets do not consume the
  discovery slot.
- Documented observed binding fields as same-user process metadata checked
  against the completion-bound policy and definition bytes, not authentication.
- Bound current panel records to the exact selected agent type and staged
  custom-agent definition digest, while retaining explicit legacy record
  readability and rejecting parent-worktree or substituted definitions.
