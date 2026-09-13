### Changed

- Converted the U8 shared host-provider family to per-kind v3 drivers: the
  `Network` Provider and the `Device` Providers (tpm, usbip, security-key,
  gpu) now run through `SharedProviderDriverFactory` on the resource plane
  instead of shared Runner rows. Each driver validates the spec against the
  owning Provider row, ensures the kind's desired children through the
  manager child API (commit before the child actor exists), retires owned
  children the desired set no longer names in the family's preserved
  endpoint-first/process-last order, runs the typed Provider effect behind
  the `SharedProviderDriverEffects` port, publishes in-memory status (R11)
  with a resync requeue while not converged, and deletes through the
  family's preserved teardown ordering. The six family ResourceTypes
  (`Network`, `Device`, `usb.d2bus.org.UsbService`,
  `usb.d2bus.org.UsbBinding`, `security-key.d2bus.org.SecurityKeyService`,
  `security-key.d2bus.org.SecurityKeyBinding`) are part of
  `V3_CONVERTED_RESOURCE_TYPES` (8 -> 14) and route to the new plane.

### Removed

- Deleted the old U8 shared Runner machinery:
  `packages/d2bd/src/resource_runtime/shared_provider_runtime.rs` shrinks
  from 8118 to 4218 lines (the U8 runner table, kind/effect/reconciler
  types, and their previews are gone), and the U8 provider-runner
  catalogue, runner preparation, capacity term, and readiness wiring are
  removed from `resource_runtime.rs`, `composition.rs`, and the
  U8-only test pins in `tests/core_composition.rs`.
