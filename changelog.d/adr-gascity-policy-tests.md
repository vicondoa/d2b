### Added

- Contributor policy coverage that keeps the documented panel preflight and the
  `Makefile` in agreement: the operator command in
  `docs/contributing/copilot-agents.md` and the notice beside it may only
  describe the preflight that actually exists, and every mixed combination -
  including a documented `make` target that was never written - fails the gate
  instead of shipping a contributor doc that points at nothing.
- A multi-root safe-type census predicate, with an accepted corpus and planted
  rejected fixtures, that recursively traverses every struct field, enum
  variant, and variant field of modelled type graphs. It fails closed on raw
  text, paths, unresolved types, unsupported cycles, empty root sets, missing
  roots, and roots that govern no structure. It is a reusable predicate over
  modelled type metadata; it does not yet inspect any shipped type. The policy
  binary is wired into the enforcing `test-policy` lane.
- Migration-remedy output controls, as a modelled decision and renderer audit
  with an accepted corpus and planted rejected fixtures. A conflicting update
  must print the exact sorted paths it already computed, plus the fetch, rebase
  onto `origin/v3`, `git add`, continue, abort, and rerun steps in a runnable
  order; no refusal may name a 40-hex object name or any other ref as a rebase
  target, and an unpublished migration is a typed refusal carrying no git
  command, since a pinned revision is the precondition a migration must satisfy
  and never a place to land a branch. Nothing here runs git or reads a
  repository, and no migration command exists yet for it to describe.

### Fixed

- The contributor-doc scans now fail closed on directory enumeration. An
  unreadable `docs/contributing/` entry was discarded, so the scan silently
  shrank to the files it could read and reported a clean pass over them; an
  entry error now fails the gate, and a listing with no Markdown in it fails
  rather than clearing an empty set.
