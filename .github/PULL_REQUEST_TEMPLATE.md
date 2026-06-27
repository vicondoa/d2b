<!-- d2b PR template. The checklist below is MANDATORY and validated by
     tests/unit/meta/pr-checklist-gate.sh.

     Do not include AI agent, assistant, or model metadata in this PR body. -->

## Summary

<!-- What changed and why. -->

## Testing checklist (mandatory)

- [ ] **`make check` passes locally** (paste the summary).
- [ ] **`make test-integration` passes on the host before PR creation**
      (paste the summary), **or** state
      `N/A: pure policy/docs/checklist change with no daemon, broker, NixOS
      module, runtime, network, or generated-artifact behavior change`.
- [ ] **`make test-host-integration` passes on the host before PR creation**
      (paste the summary), **or** state
      `N/A: pure policy/docs/checklist change with no daemon, broker, NixOS
      module, runtime, network, or generated-artifact behavior change`.
- [ ] **Manual `make test-hardware` run** on a NixOS host **with the real
      devices** (GPU / YubiKey / hardware-TPM), if this change touches
      graphics/GPU, video decode, USBIP/YubiKey, hardware-TPM, or a full
      d2b-microVM boot. Paste results, **or** state
      `N/A: no device/passthrough or full-microVM-boot surface touched`
      with a one-line justification. *(This tier requires physical devices.)*
- [ ] **New/changed tests are wired into a `make` target** and have rows in
      `tests/migration-ledger.toml` (`make check-inventory` green — it fails
      closed on any unclassified test; use `make ledger-regen` to update).
- [ ] **Docs + CI updated in lockstep**: `docs/**`, `AGENTS.md`,
      `tests/README.md`, and `.github/workflows/*` (doc+ci-reference gate
      green).

## Notes

<!-- Migration ledger rows, successor ids touched, release notes, deferrals, etc. -->
