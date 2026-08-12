### Added

- Added strict resource-envelope compilation with canonical schema, version,
  unknown-field, type, required-field, and reference rejection.
- Added a single Host-global authority startup barrier and an async
  reservation lifecycle persisted through effect closure and restart recovery.
- Added the Core-issued TPM migration decision, typed production effect-port
  adapter, and broker-owned fd-relative journal replay; unbound effects remain
  fail-closed.
- Host-install no longer performs an unsealed legacy migration; absence-only
  state is quarantined until Core supplies the never-provisioned decision.
- Recovery exposes a sealed external-inventory provenance port; active
  physical-NIC rows quarantine when that port is not installed.
