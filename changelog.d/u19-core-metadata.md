### Added

- Core metadata providers declare themselves once and register from the
  declaration. `d2bd::core_declarations` builds the unified provider
  declarations for `zone`, `provider`, `system-core`, `host`, and `user`,
  binds each family's own exported driver descriptors to them, and projects
  the set through the graph projection so the provider identity, the owned
  ResourceTypes, the components, and their execution targets are derived
  rather than authored beside the code. A declaration whose two halves
  disagree is refused by name before anything is registered.
- The foundation seed is admitted instead of trusted. Every seeded row - the
  system Zone, the SeccompProfile and Role rows, the Command rows, the
  provider self-bindings, the materialized Operation rows, and the operator
  bindings - is presented to the one graph evaluator as a create mutation
  whose initiating subject is the verified deployment root. A refused row
  names the stage and the reason and nothing is written. There is no
  bootstrap-operation allowlist to extend.
- The Host family classifies its flattened execution-parent fragment instead
  of copying it. Device and Network attachments become one child
  target-support ceiling that bounds admission and creates no binding;
  Volume defaults become child request defaults that shape only the child
  they name; default domain, allowed domains, default User, and budget stay
  non-authoritative execution-parent facts. An entry that names no child, or
  no Volume source, is refused rather than expanded.
