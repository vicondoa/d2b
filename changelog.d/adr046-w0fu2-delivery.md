### Changed

- Delivery structured stdout now reports each stage's artifact as a
  state-root-relative reference (`<wave>/<candidate>/<artifact>`, for example
  `w0/<candidate-id>/snapshot.json`) instead of a bare candidate-relative key.
  The reference still carries no absolute path, `HOME`, username, or checkout
  path, but it now names the wave and candidate, so it can be passed straight
  back as a later stage's `--snapshot` or `--seal`. Those options resolve a
  relative value under the delivery state root, so the workflow chains without
  a contributor reconstructing the on-disk path.
- `cargo xtask delivery wave help` now documents where delivery state lives -
  the default state root (`$XDG_STATE_HOME/d2b/delivery`, else
  `$HOME/.local/state/d2b/delivery`), the `--state-dir` override, and the
  `<state-root>/<wave>/<candidate>/<artifact>` layout - and how one stage's
  reported artifact chains into the next.
- `cargo xtask delivery wave help` now publishes the complete `merge-target`
  document schema and a precise, offline recipe for producing it, including
  that the target's `material` is copied verbatim from the candidate's
  `seal.json`. The schema is no longer buried in source comments with an
  ellipsis for the material shape.

### Fixed

- Delivery state writes are now anchored on verified directory descriptors:
  every intermediate directory is opened, verified, and (when absent) created
  relative to its parent's descriptor, and the temporary file create, the
  rename, and the directory fsync all run against those pinned descriptors. A
  symlink swapped into any path component after validation is rejected rather
  than traversed, so a write can no longer be redirected into another
  directory - including a repository checkout - between validation and use.
  Newly created directories are fsynced so the layout survives a crash.
- A failed delivery write no longer leaves a temporary file behind. The temp
  file is unlinked on every failure path after it is created, and a cleanup
  failure is surfaced alongside the original error instead of being swallowed,
  so stale temporaries can no longer accumulate or collide.
- Delivery storage failures no longer leak absolute paths, `HOME`, the local
  username, or a checkout or store path into the error written to stderr; the
  public diagnostics carry stable, candidate-relative descriptions instead.
- A history-only rebase now correctly invalidates validator-lane evidence while
  preserving panel evidence, enforced end to end. A rebase moves the snapshot
  digest, so `validate-import`, `seal`, and `merge-eligibility` all reject a
  seal or evidence bound to a superseded snapshot until every lane reruns and
  re-imports against the current history; the ten-role panel, which binds only
  the content-addressed candidate, is unaffected and does not have to rerun.
