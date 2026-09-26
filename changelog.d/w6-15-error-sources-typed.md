### Changed

- `d2b-provider-clipboard-wayland`: the picker IPC error keeps the typed
  failure behind its frame variants (`Frame(FramingError)`,
  `Read(io::Error)`) and splits the picker-closed-mid-frame case into its own
  variant, instead of flattening every frame failure into a `String`. The
  diagnostic text (`picker frame error: ...`) and the source chain are both
  preserved.
- `d2b-provider-transport-azure-relay`: `GatewayGuestZoneLinkError` keeps the
  typed cause behind `CredentialUnavailable` (the sealed credential or scoped
  credential request failure, carried as `ZoneLinkCredentialRefusal`) and
  `TransportUnavailable` (the relay transport failure) instead of discarding
  it in the `From` impls. The stable `code()` strings and `Display` output are
  unchanged, so no error envelope moves.
