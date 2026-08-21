### Changed

- Host-integration Rust builds can opt into a fixed multi-user
  `/var/cache/d2b-sccache` provisioned by the NixOS module as
  `root:nixbld` mode `2770`, with the Nix daemon supplying the global
  sandbox path. The lane now preflights that host contract and supports a
  fail-closed focused `vmChecks` selector without accepting caller-home cache
  mounts or restricted per-command sandbox options.

### Fixed

- Host-tool derivations keep a bounded 10 GiB sccache and visibly reject an
  exposed but unsafe cache while retaining the plain-rustc fallback when the
  opt-in cache is absent.
