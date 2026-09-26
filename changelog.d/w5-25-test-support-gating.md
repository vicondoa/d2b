### Fixed

- The `d2b-provider-volume-local` `testing` doubles module and the
  `d2b-resource-types` `assert_metadata_registration` assertion now compile
  only for the owning crate's own tests or for consumers that enable the
  `test-support` feature, so neither test-only helper is part of the
  production library or the crate root surface any more.
