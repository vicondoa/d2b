### Changed

- `make test-host-integration` now builds the fixed nine host tools with local
  Bazel, injects them into the selected NixOS VM checks, and uploads successful
  output closures to a configured Attic cache. When Attic or its configuration
  is unavailable, the lane explicitly skips the upload; invalid or unusable
  configured state fails the lane.

### Removed

- The host-integration-only sccache switch is retired. The generic
  `d2b.site.hostSccache.enable` option remains available for other Nix source
  builds.
