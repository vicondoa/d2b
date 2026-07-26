### Fixed

- NixOS activation no longer fails when a configured lifecycle user is
  temporarily unavailable through a network-backed identity provider. Heavy
  validation remains fail-closed until that user provisions protected runtime
  slots after login.
