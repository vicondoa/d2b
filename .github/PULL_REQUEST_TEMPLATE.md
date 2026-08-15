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
      `make test-host-integration` for NixOS, daemon, or host behavior,
      and `make test-hardware` for real device or full microVM-boot behavior.
- [ ] **Hardware checks are conditional:** if the change touches graphics/GPU,
      video decode, USBIP/YubiKey, hardware-TPM, or a full d2b-microVM boot,
      paste the real-device result; otherwise state why that tier is not
      applicable.
- [ ] **Changed tests are inventoried:** wire new tests into the appropriate
      target and update `tests/migration-ledger.toml` when the test model
      requires a retirement or inventory row.
- [ ] **Changelog updated** for code or user-visible behavior, using
      `CHANGELOG.md` or a `changelog.d/` fragment.
- [ ] **Docs + CI updated in lockstep** where applicable: `docs/**`,
      `AGENTS.md`, `tests/README.md`, and `.github/workflows/*`.

## Notes

<!-- Migration ledger rows, successor ids touched, release notes, deferrals, etc. -->
