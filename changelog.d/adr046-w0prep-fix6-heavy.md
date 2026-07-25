### Changed

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
- The heavy-gate helper now normalises a relative build target directory the
  same way for both the build and the execution of the freshly built binary,
  so building with a relative target directory and then running the wrapper
  from the repository root no longer looks for the binary under the wrong path.

### Security

- The heavy-entrypoint inventory guard is now closed-world with no free-text
  escape hatch. Every heavy lane, including the nightly distribution-matrix
  lane, is classified by a checked property (whether any script in it performs
  build, container, VM, privilege-elevation, or device work) rather than by an
  assertion in a comment, and the guard also classifies files directly under a
  lane parent and distinguishes each file as a runnable entrypoint or a sourced
  library per file rather than by basename. A genuinely heavy lane can no
  longer be exempted by wording, and a lane cannot escape classification by
  sitting at the top of a lane directory or by being named like a library.
- The heavy-gate semaphore now resolves a single stable per-uid location that
  does not depend on whether a runtime directory exists, and fails closed when
  its shared parent is not in a safe shape. Two lanes started on either side of
  the runtime directory being created or removed can no longer land in two
  independent slot pools, and a same-uid actor can no longer rename the shared
  parent out from under lanes that hold locks to make later invocations create
  a fresh, independent pool.
- Delivery-state candidate directories are now pinned once and every operation
  goes through the retained directory descriptors for the whole invocation.
  Snapshot and seal reads, evidence traversal, and the final write share one
  pinned chain, and the candidate address is derived from the validated
  state-relative reference rather than a supplied path, so a same-uid actor can
  no longer present a forged tree for the reads, restore the legitimate tree
  before the write, and land a seal derived from forged evidence in the
  legitimate candidate.
- The heavy-gate self-guard helper no longer trusts its build environment. It
  builds and runs the wrapper only from this checkout's own target directory,
  ignoring a caller-supplied target directory; it strips build-affecting Cargo
  and Rust environment variables before building; and it reports only a
  bounded, path-free label and an exit status on failure instead of forwarding
  the build tool's output. A hostile continuous-integration environment can no
  longer point the target at a planted binary whose slot check returns success,
  and a build failure no longer discloses the checkout location or a
  username-bearing path.
