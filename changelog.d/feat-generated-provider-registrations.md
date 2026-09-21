### Added

- The daemon's composition root now composes a generated provider/service
  registration table instead of naming families. Each provider crate that
  wants the daemon to register it declares its provider identity and its
  effect-service ids in a `registrations.json` beside its
  `resource-types.json`; `cargo xtask check-provider-crate-layout --fix`
  renders `packages/d2bd/src/generated/provider_registrations.rs` from those
  declarations, and the layout check's registration authority pins the
  committed table with a parity gate (a declared provider that is not the
  crate's own family, a declared service the crate's sources do not spell, a
  service the crate spells or registers that the declaration omits, and a
  service or provider declared by two crates all fail naming both) and a
  drift gate (a hand edit fails; regeneration is idempotent). A new family
  is therefore registered with no edit outside its declaring crate and no
  layout-ratchet row, which is what the process-systemd lane needs.

### Changed

- The Process and Network families' provider and service-factory
  registrations moved onto the generated registration table: the daemon's
  `provider_set` registers the table's rows through
  `family_declaration(registration.provider_ref)`, and the composition
  root's `effect_service_factories` map is built from the table's declared
  services. The two families start and host their declared effects services
  exactly as before; the registered families now start first, in the
  generated table's declaration order.