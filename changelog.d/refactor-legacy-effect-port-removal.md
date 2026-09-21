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
- The host family's driver effects ended their daemon-built arm too: the
  bounded capability/platform/proc probe now runs inside
  `d2b-provider-host` behind the family's declared `host.d2bus.org/effects`
  service, hosted per zone by the daemon from the family's registered
  factory. The one daemon-owned read - the minijail platform gate - crosses
  the boundary as the declared `MinijailPlatformGateSource` facet supplied
  by the composition root, and every other probe input is host state the
  crate reads itself with the same bounded seats (moved out of
  `d2bd-runtime`). The daemon's host probe implementation and its
  port-shaped injection at the driver construction site are deleted in the
  same change, with the degraded observation fallback preserved; the
  `HostProbeEffectPort` surface became an async-trait port so the probe can
  ride the hosted service. The probe's family-knowledge rows the layout
  check reported for the daemon module are retired.
- The activation family's driver effects ended their daemon-built arm too:
  the family now serves them from its own crate through its declared
  `activation.d2bus.org/effects` service, hosted per zone by the daemon from
  the family's registered factory over the composition root's facet set. The
  family's `ApplyHostGenerationHandoff` dispatch - caller role `Lifecycle`
  on the typed request, admin-uid daemon caller on the dispatch - runs
  inside the crate over the daemon-supplied broker dispatch facet, and the
  response reduction to the closed handoff result moved with it. The daemon's
  `activation_effects.rs` module, the facet-set injection at the driver
  construction site, and the daemon-side verifier wiring are deleted in the
  same change; the preserved fail-closed application verifier is now built
  by the family's factory itself.
- The interaction family's driver effects ended their daemon-built arm too:
  the family now serves them from its own crate (`d2b-provider-wayland-policy`)
  through its declared `interaction.d2bus.org/effects` service, hosted per
  zone by the daemon from the family's registered factory over the
  composition root's facet set. The display-session admission, the audio
  controller registry, and the shell pool/session reference checks run inside
  the crate; the committed interaction identity, the zone's manager-plane
  reads, and the broker-backed audio mediator cross the boundary as
  daemon-supplied facets, never derived from caller input. The daemon's
  `interaction_effects.rs`, `audio_resource_runtime.rs`, and
  `interaction_child_sources.rs` modules retire in the same change, the
  display child derivation moves into `d2b-provider-display-wayland`, and
  each of the six interaction types registers through the generated
  registration table.
- The guest family's driver effects ended their daemon-built arm: the family
  now serves them from its own crate through its declared
  `guest.d2bus.org/effects` service, hosted per zone by the daemon from the
  family's registered factory over the composition root's facet set. The
  Cloud Hypervisor controller session (target-session establishment and the
  controller-owned reconcile) and the zone's manager view (live rows,
  committed Provider identities, and the controller-session generation)
  cross the boundary as daemon-supplied facets, never derived from caller
  input; the preserved framework state machines for the qemu-media,
  azure-container-apps, and azure-virtual-machine kinds moved into the crate
  read nothing from the daemon. Every refusal and admission check moved
  verbatim: the typed admission, the dependency barrier, the KTD7 identity
  fence, and the gateway-custody validation. The daemon's `guest_effects.rs`
  module and the port-shaped injection at the driver construction site are
  deleted in the same change, and the daemon composes the family only through
  the generated registration table and its registered driver and service
  factories.
