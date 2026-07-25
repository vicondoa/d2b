### Added

- `cargo xtask delivery wave panel-request` writes the candidate-bound panel
  request into external delivery state: the wave's `candidate_id`,
  `content_id`, and `snapshot_sha256`, the exact ten-role roster, and the
  required provider, model, and reasoning-effort binding.
- `cargo xtask delivery wave panel-attest` validates a directory holding
  exactly one strict panel record per role and imports the records
  byte-verbatim into the candidate's state directory. Anything short of ten
  correctly bound, unanimous, zero-recommendation records fails closed; there
  is no override, force flag, or warn-and-continue path.
- `cargo xtask delivery wave seal` binds a wave once the ten panel records are
  present, unanimous, and bound to the same candidate triple, and every
  required validator lane reports success against that exact snapshot.
- `cargo xtask delivery wave merge-eligibility` confirms, per pull request in
  the wave's stack, that the seal exists, that the current base and head still
  match the sealed snapshot's recorded object IDs or a history-only rebase
  passes the byte-identical integrated-content proof, and that every required
  check is green.
- A byte-identical integrated-content proof module that lets a history-only
  rebase reuse a wave's validation and panel evidence without re-running it,
  consumed by `merge-eligibility`.

### Security

- Panel requests, panel records, attestations, seals, and eligibility verdicts
  are written only to the candidate-ID-addressed external delivery state
  directory. The path arguments must resolve to the candidate's own artifacts,
  so no delivery artifact can be read from or written into a repository
  checkout, a pull-request attachment, or a release archive.
