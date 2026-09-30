### Added

- Provider hosting is generated from the provider declaration and the graph
  projection. One registry holds one entry per declared method, one executable
  per entry, and one lookup that both the committed Operation a forwarded
  invocation names and the declared method the session plane names resolve
  through. There is no second operation namespace, no family switch, and no
  fallback handler route: an identity the table does not carry is refused by
  name instead of being routed somewhere else.
- A hosting site now states what it enforces. A declaration that asks for a
  privilege set, a payload schema, a deadline tier, a state cell, or a
  descriptor carriage the site cannot enforce is refused at admission with the
  unsupported facet and its declared value named, and what reaches a handler is
  the capability the site admitted rather than a copy of the declaration row.
- Every hosting generation carries an identity. A respawn or republish advances
  it, so an invocation binding captured beforehand refuses instead of reaching a
  superseded implementation.
- A method mapped by two declared services, an Operation claimed by two declared
  methods, and a composition-root site answering for two declared services are
  all refused while the table is built, so a duplicate fails before anything is
  hosted rather than at the first call.

### Changed

- Broker handler registration refuses one committed operation two declarations
  claim, and one handler implementation bound to two operations, before the
  handler table is built. Previously the second declaration silently overwrote
  the first.