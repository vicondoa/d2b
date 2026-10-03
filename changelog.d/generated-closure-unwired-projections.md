### Fixed

- The new-graph composition no longer states a Provider identity for a crate
  that declares none. A crate carrying neither `registrations.json` nor
  `service-catalog.json` registers no provider identity, which the shared
  absence table already records, but the declared surface composed a
  `Provider/` reference from the crate's directory name and put 33 identities
  into the graph that no declaration made. Resolving one of those references
  would have routed policy to a Provider the graph does not carry. Each such
  crate now states no Provider identity, a crate that declares an identity
  still states exactly the one its declaration names, and the composition
  refuses a crate whose registration and service catalog disagree about which
  Provider it is.
- The composed identity was not only unsourced but wrong. The committed
  Provider matrix gives `d2b-provider-guest-qemu-media` the identity
  `runtime-qemu-media`, `d2b-provider-guest-cloud-hypervisor` the identity
  `runtime-cloud-hypervisor`, `d2b-provider-guest-azure-container-apps` and
  `d2b-provider-guest-azure-virtual-machine` the two `runtime-azure-*`
  identities, and `d2b-provider-process-minijail` the identity
  `system-minijail` - which is one of the two fixed bootstrap Providers. The
  staged surface published `Provider/guest-qemu-media` and
  `Provider/process-minijail` instead. The render no longer infers an identity
  from a crate's directory name at all, and the refusal that compared a
  declaration against that name is replaced by one that compares a
  registration against its own service catalog, so a crate may declare the
  identity it actually owns.
- `d2b-provider-transport-vsock` now declares the Provider identity it
  registers. Four independent production sources already named
  `Provider/transport-vsock` - the `runtimeProviderRefs` assertions in
  `nixos-modules/provider-runtime-contracts.nix` and their same-Zone
  `guestRef`, `portClass` and timeout branches, the committed Provider matrix
  row, the `ZoneLink` spec's transport provider reference, and the gateway
  composition's refusal to accept a non-relay transport - while nothing
  declared it, so the identity existed only in hand-written text. The
  declaration records that identity with no effect service, which is what the
  crate publishes today; the crate's own transport implementation is not yet
  linked into the daemon, so the registered family starts with no drivers until
  that lane wires them.
- The closure manifest no longer claims a consumer that does not exist. It
  recorded the declaration-only operation catalog as installed into
  `docs/reference/policy/broker-operations.json`, but that document is written
  by the retired broker-operations merge, which reads the per-crate
  declarations itself and could not hold these bytes: the staged catalog had
  29 rows and the committed document has 100. The field is now `compiledInto`,
  and the staged catalog has since been removed outright rather than recorded
  as an artifact no production file compiles.

### Changed

- The two JSON projections in `generated/new-graph/` no longer exist. Their
  documentation claimed the canonical graph policy the configuration layer
  reads; the configuration layer is the isolated Nix test surface, which takes
  an in-memory argument of a different shape and reads no generated file. They
  are gone rather than wired into production, because the daemon already
  compiles the three staged Rust tables and no per-provider runtime requirement
  is left for them to serve.
- Fifteen provider crates still state no Provider identity: `audio-pipewire`,
  `credential-entra`, `credential-managed-identity`, `credential-secret-service`,
  `device-gpu`, `device-tpm`, `guest-azure-container-apps`,
  `guest-azure-virtual-machine`, `guest-cloud-hypervisor`, `guest-qemu-media`,
  `observability-otel`, `process-minijail`, `transport-azure-relay`,
  `volume-local` and `volume-virtiofs`. The committed Provider matrix names an
  identity for each, and for five of them it names one that differs from the
  crate's directory name, but that matrix is the inventory the cutover
  replaces. Each of these needs a production-entry-point check establishing
  that the product serves the identity before a declaration can state it;
  `transport-vsock` is the one resolved here, because four production sources
  outside the crate graph already named its identity.
