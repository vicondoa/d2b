### Changed

- Every published `delivery wave` synopsis, the `merge-target` schema recipe,
  and the `xtask` top-level usage now show a repository-root-runnable
  invocation (`cargo run --manifest-path packages/Cargo.toml -p xtask -- ...`)
  instead of the `cargo xtask ...` alias, which only resolves under
  `packages/`. The top-level usage also lists the `merge-target` stage.
- The `merge-target` document schema and the `merge-eligibility` help now use a
  positive example pull-request number and state the rule that a pull-request
  number is a positive integer; `0` is rejected.

### Fixed

- Delivery directory creation is now durable. The state root, wave, and
  candidate directories are created relative to a trusted directory descriptor,
  and each new directory's immediate parent is fsynced before success is
  reported, so a write reported as successful is not lost after power loss.
- Concurrent delivery evidence imports no longer fail spuriously. When a valid
  concurrent writer creates an absent nested directory first, the losing writer
  treats the resulting `EEXIST` as concurrent creation, reopens the verified
  directory, and continues instead of rejecting an otherwise valid candidate.

### Security

- The delivery state anchor is opened by walking its absolute path from the
  filesystem root one component at a time with `O_DIRECTORY | O_NOFOLLOW`,
  verifying the final directory's type, mode, and owner and cross-checking its
  device and inode against the name in its parent. A symlink swapped into any
  intermediate state-root or wave component is now refused rather than
  traversed, closing a window where a descriptor-relative write could be
  anchored on an attacker-chosen directory.
- The delivery wave component is validated against the closed ADR 0046 wave
  namespace (`ADR046-W0` through `ADR046-W8`) before any state is created or
  emitted, and the program component is fixed to `ADR046`. A free-form or
  name-like value such as a username can no longer become a state-directory
  name or appear in a structured artifact reference.
- Delivery failure diagnostics on the prepare, read, list, and
  root-verification paths now use logical labels and candidate-relative keys,
  matching the write path. A delivery failure written to stderr no longer
  interpolates an absolute state, repository, `XDG_RUNTIME_DIR`, or `TMPDIR`
  path.
