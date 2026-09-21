### Changed

- Broker operation rows a provider crate serves are now declared in that
  crate's `operations.json` and generated into the committed
  `docs/reference/policy/broker-operations.json` and its derived views, so
  adding or renaming a family operation needs no edit outside the declaring
  crate. The network and process families' 24 declared rows moved onto the
  new declaration surface; the drift and parity gates pin the generated
  artifacts and the declaration-to-descriptor agreement.
- A provider-owned service now carries the envelope's real request and
  response contract instead of the hosting fixture's placeholder payload, and
  reaches resource state through the generic driver context and its declared
  state cells. The composition root hosts a declared service through its
  registered factory and still refuses a declared service with no
  implementation.
- The process family's driver effects ended their daemon-built arm: the
  family now serves them from its own crate through its declared
  `process.d2bus.org/effects` service, hosted per zone by the daemon from the
  family's registered factory over the composition root's facet set. The
  daemon's `process_effects.rs` module and its port-shaped injection at the
  driver construction site are deleted in the same change; the moved
  implementation binds daemon-structural state only through the declared
  facets and reaches resource state through the generic driver context.
- The network family's driver effects ended their daemon-built arm too: the
  family now serves them from its own crate through its declared
  `network.d2bus.org/effects` service, hosted per zone by the daemon from the
  family's registered factory over the composition root's facet set, and the
  kernel-invoking adapter moved into the crate as the `KernelNetworkBroker`
  over the same broker-generic network kernels and the same `AdminUid`
  authority. The resolved bundle intents and the installed generation
  identity cross the boundary as daemon-supplied facets, never derived from
  caller input. The daemon's `network_effect_port.rs` module, the network
  field of the shared provider effects, and the port-shaped injection at the
  driver construction site are deleted in the same change.
- The credential family's driver effects ended their daemon-built arm too:
  the family now serves them from its own crate through its declared
  `credential.d2bus.org/effects` service, hosted per zone by the daemon from
  the family's registered factory over the composition root's facet set. The
  preserved Provider and execution-target reads, the lease-facts read, the
  managed-identity agent probe, and the authenticated Provider session
  handoff registry cross the boundary as daemon-supplied facets, never
  derived from caller input, and credential material never crosses the new
  boundary: the runtime facet answers typed facts and hands back the same
  session objects the daemon's ProviderSupervisor registry holds. The
  daemon's `credential_effects.rs` module, the port-shaped injection at the
  driver construction site, and the dead Guest-local backend supervisor
  surface (`credential_backend_runtime.rs`, retained only by its module-level
  dead-code allowance) are deleted in the same change.