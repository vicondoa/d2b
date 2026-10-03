### Added

- The virtiofs binding family gained a production effect port. The
  `volume-virtiofs` crate now declares the privileged serving facet one
  committed `VolumeBinding` row's delivery rides - a closed verb set of
  launch, observe, remove, and publish, with the committed row's KTD3 fence,
  its derived socket identity, and its derived worker reference on every
  request - together with `VirtiofsBindingPort`, the effect port built over
  that facet. The port reconciles each answer against the facts the serving
  side derived from the committed row: a worker that came back under another
  reference, or an observation that answered about another socket, is
  refused by name instead of being folded into a verdict, and a privileged
  leg that refuses or never answers fails the verb closed.
- The `Volume` source driver now delivers what it commits. After the
  canonical pass commits each `VolumeBinding` row through the manager, the
  same pass hands the committed row to the virtiofs family's serving pass
  over the production port, and withdraws the delivery of every canonical
  row the source no longer admits. The verdict is the consumer's own mount
  reaching it as the privileged leg reports it, so a consumer that reports
  the source serving while its own mount is absent is degraded rather than
  delivered. The daemon composes the privileged leg through the
  `VolumeRuntime::virtiofs_serving` facet, which supplies the privileged
  dispatch, the broker-owned runtime root, and the Zone's vcpu authority
  together.

### Changed

- The virtiofs effect port's observation and teardown verbs now carry the
  committed row they are about, so the privileged leg receives the KTD3
  fence on every request rather than only on the one that launched the
  worker.
- The virtiofs controller exposes its serving pass without the status
  publication: `observe` runs the whole pass - the derived worker plan, the
  private socket path, the closure store-view marker gate, the socket probe,
  and the consumer's three-state mount observation - while the fenced status
  projection stays with the `VolumeBinding` row's own actor. A fenced
  projection has exactly one writer.
- The closed virtiofs error set gained `virtiofs-serving-refused`,
  `virtiofs-serving-unavailable`, and `virtiofs-worker-identity-mismatch`.