### Added

- `cargo xtask delivery wave merge-target` captures the wave's current
  pull-request stack into the candidate as `merge-target.json`, so
  `merge-eligibility` has a supported, candidate-addressed input it can find
  without a `--target` path. The `MergeTarget` document schema and a capture
  recipe are documented alongside the command.
- `cargo xtask delivery wave help` now prints a per-stage synopsis spelling
  every option's value grammar, including the compound `--repo
  LOGICAL_ID=CHECKOUT_ROOT`, `--edge FROM=TO`, and `NAME=LOGICAL_ID:PATH`
  fingerprint forms that the bare option list could not express.

### Changed

- `cargo xtask delivery wave merge-eligibility` no longer requires `--target`.
  With no `--target` it reads the merge target captured under the candidate;
  `--target PATH` stays supported as an explicit override. The eligibility gate
  is unchanged: it still re-derives every clause from the seal and the captured
  target.
- Delivery structured stdout now reports a bounded non-absolute artifact
  reference instead of the absolute delivery-state path, so a successful run
  no longer prints `HOME`, the local username, or a checkout or store path into
  a CI or operator log.
- The delivery error classes map to four distinct sysexits-flavoured exit codes
  (usage 64, invalid input 65, unimplemented 69, environment 72) instead of
  collapsing several classes onto exit code 1.

### Fixed

- The validator-lane evidence a `cargo xtask delivery wave validate-import` run
  writes can now be consumed by `cargo xtask delivery wave seal`. The two
  stages previously used divergent, incompatible on-disk layouts and lane
  names, so no evidence produced by the public command could ever satisfy the
  seal and no real wave could seal. Both stages now share one evidence ABI and
  one canonical on-disk layout.

### Security

- Delivery evidence reads, writes, and listings reject symlinks across the
  whole resolved path, open the artifact leaf without following a final-
  component symlink, and verify after opening that it is a regular file with
  the expected mode and owner. Writes go through a create-new temporary file in
  the verified parent, an fsync, and an atomic rename. An existing symlink
  planted at an artifact path can no longer redirect a delivery write to
  truncate a file inside a repository checkout.
- The `candidate_id`, `content_id`, and `snapshot_sha256` digest identifiers
  now re-run their format validator when deserialized from JSON, so a delivery
  artifact carrying a malformed, mis-cased, or wrong-length digest is rejected
  at read time instead of being trusted.
