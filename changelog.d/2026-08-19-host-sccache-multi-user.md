### Changed

- Nix source builds can opt into a fixed multi-user `/var/cache/d2b-sccache`
  provisioned by the NixOS module as `root:nixbld` mode `2770`, with the Nix
  daemon supplying the global sandbox path.

### Fixed

- Host-tool derivations keep a bounded 10 GiB sccache and visibly reject an
  exposed but unsafe cache while retaining the plain-rustc fallback when the
  configured cache is absent.
