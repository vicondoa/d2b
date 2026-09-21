### Added

- `cargo xtask check-provider-crate-layout` now fails when a provider
  crate's BUILD file declares a dependency on a target in another package
  that the depended-on package does not grant it visibility to. The toolkit
  and the provider crates gate cross-package links through enumerated
  consumer lists (`package(default_visibility = [...])`), so a depending
  crate whose package is missing from the depended-on package's list used to
  pass every cargo test and fail only when Bazel analyzed the target; the
  check compares the declared deps against the grants and reports the crate,
  the depended-on package, and the exact consumer entry to add
  (`//packages/<crate>:__pkg__`). A dependency on a genuinely public target
  (the shared platform crates) or on a public re-export target (the
  `d2b-contracts` test-support alias) is satisfied without an entry; the
  check does not cover dependencies reached transitively, `data`/`tools`
  edges, or the existence of the depended-on target itself.