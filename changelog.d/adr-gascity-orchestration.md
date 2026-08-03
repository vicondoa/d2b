### Added

- Accepted ADR 0053, which defines an optional Gas City contributor workflow
  for this repository. The design extends Gas City's native build formulas,
  preserves standalone contributor tools, makes Gas City orchestrate the
  binding ten-seat panel, opens pull requests with canonical panel evidence,
  and keeps merge as an explicit human action.
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
  with an accepted corpus and planted rejected fixtures. The canonical
  repository is reached through a remote named `upstream` at one of two exact
  URLs, one per transport - `https://github.com/vicondoa/d2b.git` and
  `git@github.com:vicondoa/d2b.git` - and the one supported target is
  `upstream/v3`. An `upstream` already configured at either one is accepted and
  proceeds. A conflicting update prints the sorted paths it predicts will
  conflict as an advisory planning list, then `git fetch upstream` and
  `git rebase upstream/v3`, then the per-stop sequence `git status --short`,
  `git add <resolved-paths-for-this-stop>` and `git rebase --continue`, with
  `git rebase --abort` as the way out and the rerun last, in an order that works
  when it is run. It renders no bulk `git add` over the predicted paths: that
  set is the union across the whole replay, so pasting it stages files the
  rebase has not reached and turns a conflict resolution into an unrelated
  committed change. The audit parses every rendered command line instead of
  scanning it for keywords, so an unrecognised subcommand, flag, or form is
  rejected rather than skipped, a 40-hex object name is rejected anywhere on the
  line including inside a flag assignment such as `--onto=<sha>`, and the only
  admitted rebase target is `upstream/v3`. An unpublished migration, and a
  canonical `upstream` whose `v3` is simply absent, are typed refusals carrying
  no git command at all, since a pinned commit is the precondition a migration
  must satisfy and never a place to land a branch. Nothing here runs git or
  reads a repository, and no migration command exists yet for it to describe.
- Within that same model, `origin` stays the contributor's own remote and no
  rendered output touches it. It is what they push through, and a migration that
  renames or re-points it to perform a read-only fetch breaks `git push`, every
  configured tracking branch, and every script built on either. The audit
  rejects any command whose object is `origin`, in any position, along with any
  ref under it, any push-remote reconfiguration, and any mention of it in prose.
  Its URL is read for exactly one decision and is never printed: a recognised
  GitHub HTTPS origin, including a contributor fork, selects the HTTPS canonical
  upstream, the scp-like `git@github.com:<owner>/<repo>.git` origin selects the
  SSH one, and no `origin`, another host, or a URL that parses into no owner and
  repository all select HTTPS. The rule is total, so the wrapper asks nothing
  and two runs in the same tree render the same command; matching what the
  contributor already uses matters because handing an SSH-only checkout an
  HTTPS remote produces a credential prompt rather than a fetch. The repair is
  asserted per origin against the exact URL it must render, so a constant that
  happens to match one case cannot pass for the selection.
- Within that same model, three separate conditions produce no usable
  `upstream/v3`, and each names the one that actually caused it and prints the
  repair for it. A missing `upstream` remote - the ordinary first run for
  someone who cloned their fork - renders `git remote add upstream` with the
  canonical URL its `origin` selected, `git fetch upstream` and the rerun, in
  that order, and names no rebase, because nothing has been attempted and the
  target does not resolve yet. Rendering the canonical URL of the transport that
  was not selected is rejected on its own terms, since both URLs are canonical
  and no URL check would catch it. An `upstream` that exists and points at a
  third value is read, never rewritten: the refusal renders
  `git remote get-url upstream`, names both accepted canonical URLs in prose so
  the contributor knows what would satisfy the check, and asks them to choose
  the arrangement they want, because that remote may be a mirror or a second
  project and only they know. A canonical `upstream` whose branch is simply
  absent renders no git command and says the remedy is outside the tree: wait
  for the branch to be published, or contact the repository owner. A generic
  "restore access" or "check your network" message is rejected everywhere in
  this output, because it covers all three at once and sends someone to debug a
  network that is working. The three render three distinct command sets, so none
  of them can be collapsed into another. The audit admits only two remote forms
  and only those two URLs: `git remote rename`, `git remote set-url` and
  `git remote remove` are on no list in any form, and another repository,
  another owner, another host, another scheme, a query or fragment, an `ssh://`
  spelling of the right target, and any URL carrying userinfo that is not
  GitHub's fixed scp-like service account, a token, or an `x-access-token` form
  are all rejected, because a remote URL is written verbatim into plain
  `.git/config`. That service account is admitted rather than read as a
  credential: it is the same constant in every SSH clone, and the key it
  authenticates with is on disk and not in the URL, so rejecting it would refuse
  the ordinary clone every SSH-only contributor already has. The URL a
  contributor already configured is never echoed back at them: the refusal that
  reports it has no field to hold it.

### Fixed

- The contributor-doc scans now fail closed on directory enumeration. An
  unreadable `docs/contributing/` entry was discarded, so the scan silently
  shrank to the files it could read and reported a clean pass over them; an
  entry error now fails the gate, and a listing with no Markdown in it fails
  rather than clearing an empty set.
- The lint's own failure diagnostics no longer widen what a failing scan
  discloses. The directory-scan fault and every migration-output rejection
  report the condition they found and redact the payload behind it - the
  directory, the operating system's entry message, repository paths, command
  lines, remote names, and remote URLs - so a gate failure message cannot become
  the leak. Equality still compares the whole payload, so the assertions that
  pin exactly which path or URL was rejected are unchanged, and tests assert
  that planted sensitive values never reach the rendered diagnostic.
