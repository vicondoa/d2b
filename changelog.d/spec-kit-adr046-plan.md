### Added

- Project constitution at `.specify/memory/constitution.md`, defining seven
  non-negotiable principles: daemon-only control plane, broker-mediated audited
  privilege, isolation over convenience, contract-driven compatibility,
  test-layer discipline, panel-gated multi-phase work, and marker-free shipped
  artifacts.
- Spec Kit planning artifacts for the ADR 0046 (d2b 3.0) delivery program under
  `specs/001-adr046-d2b3-completion/`, covering the specification, plan,
  research, complete work-item coverage proof, data model, contract surfaces,
  quickstart, task list, and two continuous registers for deferred findings and
  delivery friction.

### Changed

- Replaced every non-ASCII dash codepoint in the vendored Spec Kit command and
  extension files with the ASCII hyphen, so the tier0 gate that forbids them
  passes across the whole tree.
