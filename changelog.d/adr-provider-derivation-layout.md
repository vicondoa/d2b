### Added

- Added ADR 0050, a documentation-only decision record fixing the on-disk shape
  of a Provider Nix derivation so the build-time required-outputs check becomes
  writable. An artifact pins one Nix output, and where the derivation has more
  than one that output must carry evidence it was chosen, tested over `outputs`,
  `outputSpecified` and `outputName` rather than over `all`, which both rejects
  a correctly pinned selected output and throws on a store-path-valued package.
  The signed manifest, its detached Ed25519 signature, and the root config JSON
  Schema are regular files at fixed identity-independent paths under
  `share/d2b/provider/`, a closed directory that refuses any fourth entry. The
  executable set is located by enumerating `bin/`, name-checked as read from the
  directory, and required to be ELF with an execute bit, since a valid image at
  mode 0644 otherwise passes every other check and can never be launched.
  Digests have stated preimages, with a new domain tag binding the whole sorted
  name-to-digest map, and admission requires the operator pin, the publisher's
  signed claim, and the compiler's own recomputation to agree pairwise. Path
  resolution is anchored and fd-relative for both the compiler and the launcher,
  in two least-authority handle modes because a path-only descriptor cannot be
  read, behind an injectable boundary so sequencing and error mapping are
  testable without a store or a real exec. Failures are a bounded taxonomy that
  names actionable remedies, including exact toolkit commands, without emitting
  store paths, key material, or unbounded lists. A framework-owned Nix helper
  builds a conforming entry point for interpreted Providers so the ELF rule has
  a supported route. The record adds conformance scenarios for every rule it
  states and corrects five scenario identifiers cited as validation that exist
  nowhere in the specification set. No crates, services, controllers, or
  Providers are created, and no code behaviour changes.
