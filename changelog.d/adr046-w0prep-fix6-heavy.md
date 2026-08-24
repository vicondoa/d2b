### Changed

- The delivery workflow now binds the complete expected set of pull requests
  and heads for every repository in the snapshot, and merge eligibility
  requires the supplied pull requests to match that set exactly and to produce
  a deterministic integrated tree covering every slice. A wave of parallel
  same-repository slices can no longer be declared merge-eligible while a slice
  and its required checks are silently absent from the merge target. The
  published merge-target recipe now requires one object per pull request, and
  the persisted delivery artifact schema version is bumped accordingly.
- The delivery status domain and its serialization are now generated from a
  single declaration, and the pinned wire fingerprint serializes an exhaustive
  contract probe with every optional field populated. Adding a status outcome
  or an optional field, or populating a field a stage previously left unset,
  now fails the build unless it travels with a `schema_version` bump and a
  matching golden.
- The delivery workflow success JSON now pins a generated wire fingerprint
  keyed to its `schema_version`. The fingerprint enumerates both value domains
  from the types themselves, the full field set of every emitted object, and
  the serialized shape of a representative output for every wave stage, so
  adding a status outcome, a stage, or an optional field, or changing any
  stage's shape, fails the build unless it travels with a `schema_version`
  bump and a matching golden. A consumer reading this JSON can no longer break
  silently against a producer whose shape drifted without a version change.

### Fixed

- The runtime-ledger gate now fails closed on any test-runner failure. It
  captures the runner exit status, requires a matching successful
  suite-completion signal, and refuses to record measurements from a partial
  or crashed run, so a compile error, a signal, or a runner-level failure can
  no longer produce a stable partial stream that satisfies the gate while the
  underlying suite never passed. Only redacted diagnostics are retained on
  failure.
- A failed git invocation during snapshotting is now mapped to a stable,
  path-free reason code (missing object, not a repository, unsafe ownership,
  permission denied, or corrupt repository), and an unrecognised failure is
  reduced to a bounded sanitised cause with paths and control characters
  redacted. The previous behaviour discarded git's diagnostic entirely, which
  left a real failure undiagnosable because git returns the same exit status
  for all of these classes.

### Security

- Delivery-state candidate directories are now pinned once and every operation
  goes through the retained directory descriptors for the whole invocation.
  Snapshot and seal reads, evidence traversal, and the final write share one
  pinned chain, and the candidate address is derived from the validated
  state-relative reference rather than a supplied path, so a same-uid actor can
  no longer present a forged tree for the reads, restore the legitimate tree
  before the write, and land a seal derived from forged evidence in the
  legitimate candidate.
- Absolute filesystem paths no longer reach the runtime-ledger or
  delivery-evidence diagnostics. Failure messages name the artifact role, or
  the offending file's leaf, instead of the absolute path a caller supplied, so
  the checkout layout and any username-bearing directory no longer leak into
  operator output or continuous-integration logs.
