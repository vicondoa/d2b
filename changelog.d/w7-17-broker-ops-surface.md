### Changed

- The broker's operation-handler arms are published only where a
  consumer outside the crate addresses them. `d2b_broker::ops`
  declared all 31 handler modules `pub`, so every item in them was
  published surface even though no crate outside `d2b-broker` imported
  any of them; only `network`, `audit_op`, and `pidfd` stay `pub`,
  because the broker's integration tests are separate crates and
  import those arms by path. The other 28 arms are `pub(crate)`, and a
  new handler arm stays crate-private until something outside the
  crate imports it. No item, type, or behavior inside an arm changed.
