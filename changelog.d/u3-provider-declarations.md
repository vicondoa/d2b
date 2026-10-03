### Added

- `d2b-provider-toolkit::declaration::provider`: the unified provider declaration (KTD1). One
  provider now exports a single typed source instead of separately authored metadata: the
  serializable semantic facets (`ProviderDeclarationSpec`: supported resource types, methods, code
  identity, configuration, placement, child creation, required resource capabilities, the closed
  `filesystem-presentation` / `namespace-first-service-source` presentation capability, and the
  setup restrictions that capability requires) are held separately from the local constructor and
  function bindings that realize them (`d2b_resource_types::ProviderImplementationBindings`), and
  every identity is declared once and referenced by both halves. The declaration refuses a method
  with no implementation, an implementation with no declaration, a duplicate identity, an
  unsupported placement, and a presentation a component cannot realize. A mutable `Provider` row
  selects a deployment-admitted artifact and nothing else, so it cannot create a compiled
  privileged handler.
- `emit_declaration_canonical`: the deterministic `d2b-cjson/v1` data projection generators consume
  from a declaration. It carries no signing key, no handler, and no runtime state.
