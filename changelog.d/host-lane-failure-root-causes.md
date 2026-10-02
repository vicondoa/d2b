### Fixed

- A shared host-provider Driver row now carries the Provider's own closed cause
  on its failure instead of losing it at the erased driver boundary.
  `SharedProviderEffectError` gains `UnavailableWithCause(&'static str)`;
  `SharedProviderDriverError` carries the code it names; `classify_error` puts
  it on the `DriverFailure` note. The TPM Device adapter uses it on both the
  reconcile and finalize arms, so a `Device` that will not reconcile reports
  `device-tpm-state-integrity-failure`, `device-tpm-effect-transient` or
  `device-tpm-admission-unavailable` on the row rather than the flat
  `shared-provider-unavailable` it reported for every pass of the retry. The
  cause used to exist only in the single `tracing::warn!` the adapter wrote
  before flattening it; a journal tail bounded to the last few hundred lines
  does not have to contain that line, so the row - which is what an operator
  and the lane both read - named nothing at all.

### Known issues

- The `device-worker-launch` and `virtiofsd-volume-runtime` lane fixtures
  declare their `acceptance-guest` `Guest` row with
  `providerRef = "Provider/volume-virtiofs"`, which the Guest family does not
  own, so the family refuses the row at Validate with the closed
  `guest-spec-invalid` refusal and `Guest/acceptance-guest` is terminally
  `Failed` for the whole run. This is left in place deliberately: naming a
  Provider the family does own makes the row demand
  `spec.systemArtifactId` resolving to a nixos-system artifact plus a matching
  private Guest setup descriptor (the Nix module asserts all four), and neither
  fixture declares a guest system on purpose. The row is inert on the paths
  those checks measure - `tpm_device_is_admitted` reads the committed `Device`
  row and the broker derives the VM identity from `Device.metadata.ownerRef`, so
  neither consults the `Guest` row's phase - but the fixture is carrying a row
  the daemon and the module both refuse. Dropping it means re-parenting the
  `Device` rows (and the volume attachment's execution target) onto
  `Host/host-system`, which moves the per-Guest runtime and `path:swtpm-state`
  storage rows with them.