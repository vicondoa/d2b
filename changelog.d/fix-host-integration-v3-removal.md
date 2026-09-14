### Fixed

- Restored the live TPM device admission chain (`tpm_device_is_admitted` /
  `LegacyTpmMigrationDecision`) that the v2-removal over-deleted, so TPM
  device reconcile and the v3 volume-anchor path for swtpm-backed devices
  complete instead of failing closed. The genuinely unreachable
  `MigrateLegacySwtpmState` broker dispatch stays removed, with its TPM
  effect-port branch now failing closed.
- Converted the host-integration guest bundle fixtures that still emitted
  the legacy v2 bundle shape (`guest-shell-service`,
  `runtime-cloud-hypervisor-guest-preflight`, `state-posture-contract`) to
  the v3 Zone-native contract so the v3-only `BundleResolver` loads them.
