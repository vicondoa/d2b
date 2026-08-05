### Added

- Recorded the decision that resolves the last blocker in the Bazel Rust build
  and test migration: the privileged broker's dependency hub. The broker keeps
  its own Cargo workspace and its own independently pinned and independently
  audited lock file, which is the dependency closure of the only binary the
  framework runs as root, and neither that manifest nor that lock is edited,
  generated, or rewritten by any build command. The build tool instead reads a
  generated, drift-checked stand-in workspace derived from what the broker's
  own dependency resolution actually contains, so an optional dependency the
  broker never turns on stays out of the privileged closure along with
  everything it would pull in. Three independent checks prove the generated
  tree still describes the authoritative lock, and each catches drift the
  others cannot see.
- Recorded that the shared first-party libraries the broker uses are built
  twice, once for each dependency set, while their tests continue to run once
  in the main workspace exactly as they do today, because the broker's lock
  does not contain those crates' test-only dependencies and its own test run
  never built them. An explicit check refuses any build graph in which the
  privileged binary reaches a library built against the other dependency set,
  or the main build reaches one built against the broker's, so the audited
  closure cannot silently become a mixture of the two.
- Recorded that regenerating the broker's build-side dependency lock refuses
  outright when the generated stand-in workspace is out of date, before any
  build-tool process starts, and names the command that brings it up to date.
  Without that refusal the regeneration can record a dependency set the
  repository no longer has, and every other check stays green afterwards.
