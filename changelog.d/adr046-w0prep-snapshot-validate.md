### Added

- `cargo xtask delivery wave snapshot` binds one wave's stack into an immutable
  candidate: it reads each repository's base commit, head commit, and integrated
  tree out of Git, hashes the named generated, dependency, and contract objects
  as committed, and derives `content_id`, `candidate_id`, and `snapshot_sha256`.
  The snapshot is written only into the external, candidate-ID-addressed
  delivery state directory. A repository with uncommitted tracked changes is
  refused, and a candidate address that already holds a snapshot of different
  integrated content is never overwritten; a history-only rebase reproduces the
  same address and rebinds its new base and head in place.
- `cargo xtask delivery wave validate-import` records one validator lane's
  command and result against one candidate. Records are addressed by
  `candidate_id` under the same external state directory, and are refused when
  the snapshot fails its own digest check, when the repositories no longer
  integrate to the sealed candidate, or when the caller's `--candidate` guard
  names a superseded candidate. Validator output is never stored: `--log` is
  streamed through a hasher and only its digest and byte count are recorded.
