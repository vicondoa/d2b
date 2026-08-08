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
