### Added

- Contributor policy coverage that keeps the documented panel preflight and the
  `Makefile` in agreement: the operator command in
  `docs/contributing/copilot-agents.md` and the notice beside it may only
  describe the preflight that actually exists, and every mixed combination -
  including a documented `make` target that was never written - fails the gate
  instead of shipping a contributor doc that points at nothing.
- A safe-type census predicate, with an accepted corpus and planted rejected
  fixtures, that recursively traverses every struct field, enum variant, and
  variant field of a modelled type graph and fails closed on raw text, a path,
  an unresolved type, an unsupported cycle, and an empty corpus. It is a
  reusable predicate over modelled type metadata; it does not yet inspect any
  shipped type.
