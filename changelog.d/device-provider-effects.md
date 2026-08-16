### Added

- Added opaque Volume effect-port contracts and dependency-safe local Volume
  finalization.
- Added strict virtiofs Export contracts and host-side effect-port composition.
- Added exact Device and holder-bound security-key admission with fail-closed
  hidraw selection.
- Added controller-created TPM child-resource contracts and a broker-backed
  production reconcile path that preserves TPM state.
- Hardened TPM state before the first flush, routed reconcile through the
  broker-owned legacy migration journal, and bound launch tickets to the
  validated state intent.
- Refused the unbound legacy security-key broker operation and raw hidraw
  selectors until a bundle-backed stable-selector Provider path is present.
- Enforced canonical virtiofs Provider identity and mount-path validation.
