<!-- nixling PR template. The checklist below is MANDATORY (plan §7.5 of the
     test rearchitecture). The check-inventory/pr-checklist gate fails closed
     if it is missing. -->

## Summary

<!-- What changed and why. -->

## Testing checklist (mandatory)

- [ ] **`make check` passes locally** (paste the summary). CI also runs
      `make check-ci` = `check` + `test-integration` (the device-free
      runNixOSTest VM tier, on a KVM Ubuntu job) — confirm green.
- [ ] **Manual `make test-hardware` run** on a NixOS host **with the real
      devices** (GPU / YubiKey / hardware-TPM), if this change touches
      graphics/GPU, video decode, USBIP/YubiKey, hardware-TPM, or a full
      nixling-microVM boot. Paste results, **or** state
      `N/A: no device/passthrough or full-microVM-boot surface touched`
      with a one-line justification. *(This is the only tier CI cannot run —
      hosted runners have KVM but no physical devices.)*
- [ ] **New/changed tests are wired into a `make` target** and have rows in
      `tests/migration-ledger.toml` (`make check-inventory` green — it fails
      closed on any unclassified test).
- [ ] **Docs + CI updated in lockstep**: `docs/**`, `AGENTS.md`,
      `tests/README.md`, and `.github/workflows/*` (doc+ci-reference gate
      green).

## Notes

<!-- Migration ledger rows / successor ids touched, panel sign-off refs, etc. -->
