### Added

- Accepted ADR 0053, which defines an optional Gas City contributor workflow
  for agent-assisted work on this repository. Gas City remains external to the
  d2b product and runtime, while a generic `gascity.nix` repository owns
  reusable NixOS lifecycle and security machinery and
  `d2b-gascity-configs` owns d2b-specific workflow policy.
