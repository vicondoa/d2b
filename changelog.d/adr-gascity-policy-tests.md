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
  prints the sorted paths it predicts will conflict as an advisory planning
  list, then `git fetch origin` and `git rebase origin/v3`, then the per-stop
  sequence `git status --short`, `git add <resolved-paths-for-this-stop>` and
  `git rebase --continue`, with `git rebase --abort` as the way out and the
  rerun last, in an order that works when it is run. It renders no bulk
  `git add` over the predicted paths: that set is the union across the whole
  replay, so pasting it stages files the rebase has not reached and turns a
  conflict resolution into an unrelated committed change. The audit parses
  every rendered command line instead of scanning it for keywords, so an
  unrecognised subcommand, flag, or form is rejected rather than skipped, a
  40-hex object name is rejected anywhere on the line including inside a flag
  assignment such as `--onto=<sha>`, and the only admitted rebase target is
  `origin/v3`. A fetch that produced no such ref and an unpublished migration
  are typed refusals carrying no git command at all, since a pinned revision is
  the precondition a migration must satisfy and never a place to land a branch.
  Nothing here runs git or reads a repository, and no migration command exists
  yet for it to describe.

### Fixed

- The contributor-doc scans now fail closed on directory enumeration. An
  unreadable `docs/contributing/` entry was discarded, so the scan silently
  shrank to the files it could read and reported a clean pass over them; an
  entry error now fails the gate, and a listing with no Markdown in it fails
  rather than clearing an empty set.
