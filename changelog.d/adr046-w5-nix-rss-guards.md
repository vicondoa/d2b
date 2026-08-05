### Fixed

- Isolate Wave 5 Nix evaluation shards and enforce separate aggregate RSS
  ceilings for the complete Nix-unit and flake evaluation lanes.
- Keep guest-control rejection probes on assertion records instead of forcing
  each VM's `system.build.toplevel` during eval-only tests.
