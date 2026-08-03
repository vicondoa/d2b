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
  `origin/v3`. An unpublished migration, and a canonical `origin` whose `v3` is
  simply absent, are typed refusals carrying no git command at all, since a
  pinned commit is the precondition a migration must satisfy and never a place
  to land a branch. Nothing here runs git or reads a repository, and no
  migration command exists yet for it to describe.
- Within that same model, the unavailable-target refusal names the condition
  that actually causes it and prints the repair for it. `origin` is usually the
  contributor's own fork, which carries no `v3`, so a generic "restore access"
  message sends someone to debug a network that is working; that phrasing is
  rejected everywhere in this output. The modelled refusal instead states that
  `origin` is not canonical and renders `git remote rename origin fork`,
  `git remote add origin https://github.com/vicondoa/d2b.git`,
  `git fetch origin` and the rerun, in that order, keeping the fork under its
  own name rather than discarding it, and naming no rebase, because nothing has
  been attempted and the target does not resolve yet. The audit admits only
  those two exact remote forms and only that one URL: another repository,
  another scheme, a query or fragment, an ssh spelling, and any URL carrying a
  userinfo component, a token, or an `x-access-token` form are all rejected,
  because a remote URL is written verbatim into plain `.git/config`. When a
  remote named `fork` already exists, the refusal says so and asks for a name
  the contributor chooses: it renames nothing, adds nothing, and never offers a
  generated name such as `fork2`.

### Fixed

- The contributor-doc scans now fail closed on directory enumeration. An
  unreadable `docs/contributing/` entry was discarded, so the scan silently
  shrank to the files it could read and reported a clean pass over them; an
  entry error now fails the gate, and a listing with no Markdown in it fails
  rather than clearing an empty set.
