### Changed

- The design and daemon-lifecycle explanations now describe the shipped
  resource graph instead of the model that preceded the cutover. They state
  that a binding is two-sided: a consumer publishes a typed request, the source
  provider admits it against its own decision and the realization its selected
  backend declares, and the admitted relationship is committed as a row of its
  own served type that a dedicated Provider realizes and releases. A source
  ensures the binding rows it derives and retires the ones it no longer
  derives, and the pages list the binding controllers among the effect
  owners.
- The same pages now record the execution and authority model an operator
  would otherwise have to read the source to learn: an `Operation` row names a
  declared `Provider` method or a provider-owned executable template and never
  a command row, host path, or argv; `Role` is authorization-only, carrying
  bounded rules plus the `Operation` rows a holder may create and no posture,
  mount, or command facet; and `ExecutionPolicy` and `SeccompProfile` state
  confinement and the syscall filter without granting resource access, which
  only an admitted binding row carries.
- The daemon is documented as observing the committed rows a Guest's lifecycle
  depends on, its binding rows among them, rather than the pre-cutover short
  list, so the explanation matches the set the resource plane reconciles.
