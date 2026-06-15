<!-- nixling PR template. The checklist below is MANDATORY (plan §7.5 of the
     test rearchitecture). The pr-checklist gate validates these items; live PR
     body wiring lands in a later CI wave. -->

## Summary

<!-- What changed and why. -->

## Testing checklist (mandatory)

- [ ] **`make check` passes locally** (paste the summary). This remains the
      real CI gate during W0; `make check-ci` adds only a safe
      `test-integration` placeholder until the runNixOSTest CI job lands W4.
- [ ] **Manual `make test-hardware` run** on a NixOS host **with the real
      devices** (GPU / YubiKey / hardware-TPM), if this change touches
      graphics/GPU, video decode, USBIP/YubiKey, hardware-TPM, or a full
      nixling-microVM boot. Paste results, **or** state
      `N/A: no device/passthrough or full-microVM-boot surface touched`
      with a one-line justification. *(This is the only tier CI cannot run —
      hosted runners have KVM but no physical devices.)*
- [ ] **New/changed tests are wired into a `make` target** and have rows in
      `tests/migration-ledger.toml` (`make check-inventory` green — it fails
      closed on any unclassified test; use `make ledger-regen` to update).
- [ ] **Docs + CI updated in lockstep**: `docs/**`, `AGENTS.md`,
      `tests/README.md`, and `.github/workflows/*` (doc+ci-reference gate
      green).

## Notes

<!-- Migration ledger rows / successor ids touched, panel sign-off refs, etc. -->
