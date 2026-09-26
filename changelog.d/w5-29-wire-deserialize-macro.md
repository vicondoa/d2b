### Changed

- The `wire_deserialize!` admission-`Deserialize` macro moved from
  `d2b-contracts-zone-session` to `d2b-contracts`, the contract crate every
  other contract crate already depends on, and is now exported
  (`#[macro_export]`) so all of them can share it. Zone-session imports it from
  its new home; its 28 Wire shapes are unchanged.
- The 72 hand-written Wire-struct admission `Deserialize` impls in
  `d2b-contracts-provider` (17) and `d2b-contracts-resource` (55) now expand the
  shared macro. Wire shapes, `serde` attributes and defaults, and every
  constructor validation gate are unchanged; only the boilerplate is.
