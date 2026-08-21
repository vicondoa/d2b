<!-- d2b PR template. Record validation evidence for the changed surface.
     `make check` is an available aggregate, not a prerequisite for review.

     Do not include AI agent, assistant, or model metadata in this PR body. -->

## Summary

<!-- What changed and why. -->

## Validation evidence

- [ ] **Focused tests for the changed components** were run; list exact
      commands and results.
- [ ] **Wider lanes are conditional.** Run the applicable public lane when the
      changed surface needs it, and explain any deliberate omission:
      `make test-integration` for container behavior,
      and `make test-host-integration` for NixOS, daemon, or host behavior.
- [ ] **Changed tests are owner-local:** wire new tests into the owning Bazel
      target and delete superseded evidence or migration scripts.
- [ ] **Changelog updated** for code or user-visible behavior, using
      `CHANGELOG.md` or a `changelog.d/` fragment.
- [ ] **Docs + CI updated in lockstep** where applicable: `docs/**`,
      `AGENTS.md`, `tests/README.md`, and `.github/workflows/*`.

## Notes

<!-- Migration ledger rows, successor ids touched, release notes, deferrals, etc. -->
