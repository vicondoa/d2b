### Added

- Accepted ADR 0054, which governs the newer workspace shape: one resolver-v2
  product Cargo workspace and root lock for product packages, a separate
  no-bash walker workspace and lock, generated `Cargo.guest.lock` static-guest
  closure input, and exactly the `product` and `walker` Bazel hubs. Selected
  Cargo closure policy remains authoritative for package security, while
  native Bazel context censuses remain authoritative for first-party edges.
  The broker GNU and guest musl policy contexts are generated for
  `x86_64-linux` and `aarch64-linux`; the native arm gate realizes six checks
  and runs the supply-chain gate on one stable head. Release builds use the
  root manifest and `packages/target/release`, with explicit package, binary,
  lock, and feature selectors. Retired `main`, `broker`, and `guest` hub
  identifiers are not authorities.
- Amended the Spec 003 plan to require every governed Rust action to use the
  repository's exact Nix-pinned, Linux-sandbox-patched Bazel 8.6.0. Planned
  implementation must load the fixed seccomp policy before the complete action
  command and prove exact identity, sandbox-only strategy, inherited-authority,
  and pre-action network refusals.
- Amended the Spec 003 plan to retain the workspace-wide Rust unsafe-code
  prohibition. Planned implementation co-locates verified executable ownership
  and its sole safe Rust consumer, maps the consumed descriptor privately, and
  invokes one exact immutable statically linked C supervisor that proves
  pathless exec, remains alive, forwards signals, and reaps exact target
  status. No Rust helper crate or unsafe exception is authorized.
- Amended the Spec 003 plan to require a complete Markdown task census,
  independent exact task-ID census, byte-exact fixed validator diagnostics,
  isolated hybrid-disclosure mismatch fixtures, and one atomic alias-removal
  transition across every renderer, test, doc, evidence field, and semantic
  fragment. These are plan requirements for later implementation, not shipped
  Bazel controls.
