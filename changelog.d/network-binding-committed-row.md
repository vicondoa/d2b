---
type: added
area: providers
---

### Added

- **A committed `Network` row produces and serves its own `NetworkBinding`
  relationships.** The Network driver now derives one committed
  `NetworkBinding` row per execution target the row's own attachments declare,
  commits it through the manager's child surface beside the family's existing
  children, and retires the rows a shrunken attachment set no longer derives.
  Each consumer's exact request - its slot, its presentation, and its traffic
  policy - is read off the committed `Network` row rather than supplied by a
  caller or a consumer, so a relationship cannot widen itself by asking for a
  different slot, a different presentation, or another Network.

- **The serving half reads those rows back.** `NetworkBinding` has its own
  registered driver: it re-derives the committed row name from the committed
  identities, compares the committed decision against the decision this family
  admits, checks the owning `Network` row behind its owner fence and its
  current attachments, and publishes the interface the consumer holds on the
  shared fabric. A row the source no longer attaches is refused rather than
  served, and the host admission consumes the served rows through one public
  entry point that both halves derive from.

- **The rendered configuration carries the committed membership.** The
  per-consumer policy the live config writes comes from the committed
  `NetworkBinding` rows rather than from an empty relationship list, so a
  consumer policy in the rendered config is now a statement the source admitted
  and the graph can read back.