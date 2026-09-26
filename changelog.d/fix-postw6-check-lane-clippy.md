### Fixed

- The volume-local binding derivation forwards its borrowed volume reference
  to the binding-name hasher instead of re-borrowing it, so the crate builds
  clean under the deny-warnings lint set. The emitted binding names, their
  inputs, and every caller are unchanged.
