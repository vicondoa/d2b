---
type: fixed
area: providers,daemon
---

### Fixed

- **A Network membership now reaches the rendered config.** The Network
  reconciler wrote the fabric's four config files from `render_config_inner(spec,
  None, None)`, so the consumer reservation a committed membership declared
  never reached the Volume the net VM reads; the renderer that can carry it was
  reachable only from a unit test. The live render now reads the committed
  `NetworkBinding` relationships off the host admission proof instead of taking
  a policy from a caller argument, and writes each admitted consumer's
  interface, reservation, and egress decision into the config and its digest. A
  Network row that declares no membership renders byte for byte what it
  rendered before.

- **The host admission carries the committed relationship, not a bare
  identity.** `NetworkAdmissionIntent::new` takes the admitted consumers and,
  beside them, the committed relationships the accepted graph carries. Each
  relationship is one `NetworkAdmittedConsumer`, which now holds the consumer's
  exact typed request rather than only its presentation, so the row the graph
  reads back and the policy the render writes are two views of one derivation.
  A relationship naming a consumer the host admission never admitted is refused
  before any effect runs, and one whose execution target the committed `Network`
  row does not attach renders no policy: a source row implies no relationship
  for a consumer it does not declare.
