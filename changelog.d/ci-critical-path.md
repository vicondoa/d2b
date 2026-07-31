### Changed

- Flake checks whose shard must build rather than evaluate now run in their own
  CI lane instead of taking a slot in the bounded instantiate-only matrix. One
  enumeration produces every dispatch class, and the classifier is shared with
  the driver that decides build-versus-instantiate, so a shard cannot be routed
  to the realized lane and then merely evaluated there.
- The Nix-unit corpus checks are no longer instantiated a second time by the
  flake-eval matrix. The dedicated Nix-unit lane already builds exactly those
  checks, and both lanes now read the same partition, so the names dropped from
  one are provably the names the other runs.
- The Rust-profile jobs reclaim preinstalled runner toolchains only when the
  image arrives with less free space than the gate needs, rather than on every
  run. A fuller image still reclaims exactly as before.
