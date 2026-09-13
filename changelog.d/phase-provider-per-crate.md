### Changed

- The Process family now lives in its own `d2b-provider-process` crate: the
  driver for `Process` and `EphemeralProcess`, the family's declarations, the
  spec decoder and driver factory the registry serves, the one canonical
  launch-identity resolver, and the process launch and supervision primitives
  the family exchanges. The daemon registers the family through its
  descriptors and keeps only the production effect implementation behind the
  port the crate declares. Its `process_driver` module is gone.
- `d2b-provider-system-minijail` and `d2b-provider-system-systemd` are renamed
  to `d2b-provider-process-minijail` and `d2b-provider-process-systemd`, beside
  the family crate they realize. Both keep their provider identities
  (`Provider/system-minijail`, `Provider/system-systemd`) and their exported
  names, and every Cargo, Bazel, Nix, copied-Guest, and packaging-matrix
  reference follows the new names.
- `check-provider-crate-layout` now refuses a resource driver declared in a
  shared crate: the still-un-migrated families are an explicit, shrinking
  exemption list in the policy crate, and a new driver outside a provider
  crate fails with its module path. The integration-scenario ratchet records
  the crates whose integration surface is still a scaffold.
- The Endpoint resource driver now lives in its own `d2b-provider-endpoint`
  crate together with its spec decoder, its effect port, and the driver
  declaration the resource plane registers the type by. The daemon keeps the
  production effect implementation behind the port, so the driver depends on
  no provider crate, and the registry serves the type's decoder and factory
  from the declaration instead of a daemon-side table. Operator-visible
  behavior is unchanged: the same endpoint shapes are admitted, and the same
  validate, recover, reconcile, finalize, and delete verbs run.
- The Volume and VolumeBinding drivers now live in their own
  `d2b-provider-volume` and `d2b-provider-volume-binding` crates, together
  with their spec decoders, their effect ports, and the driver declarations
  the resource plane registers the types by. The daemon keeps the production
  effect implementations behind those ports - the volume-local layout effect
  and its durable probe, and the binding serving socket, its removal, and the
  guest-mount observation - so the family crates carry no host state. The
  registry serves each type's decoder and factory from its declaration
  instead of a hand-built provider and decoder table, and the
  `volume_driver`, `binding_driver`, and `binding_child_resource_runtime`
  daemon modules are gone.
- The VolumeBinding declaration licenses the two children the driver mints:
  the worker `Process` served by `Provider/system-minijail` and the `Endpoint`
  served by `Provider/volume-virtiofs`, each with its creation rank. The
  Volume declaration licenses the `VolumeBinding` child its admitted
  attachments derive. Both family crates take those provider references from
  the provider crates that own them, and the binding driver's own child
  retirement order is derived from the declaration's ranks rather than from a
  second table.
- The two binding row readers (`binding_readiness_current`,
  `parsed_binding_spec`) move to the binding crate, and the daemon reads
  stored binding rows through them. `d2b-provider-volume-virtiofs` exports
  its canonical `PROVIDER_REF` so the declaring crates stop respelling it.
  Operator-visible behavior is unchanged: the same volume and binding shapes
  are admitted, the same validate, recover, reconcile, finalize, and delete
  verbs run, and the worker/endpoint teardown order is preserved.
- The Credential resource driver now lives in its own `d2b-provider-credential`
  crate together with its spec decoder, its effect port, the session and
  revocation vocabulary its teardown binds, and the driver declaration the
  resource plane registers the type by. The three Credential Providers stay
  the separate realizer crates they already are; the daemon keeps the
  production effect implementation - the Provider reads, the live session
  adapter, and the handoff registry - behind the port. Operator-visible
  behavior is unchanged: the same three Providers are admitted, the same
  per-Provider scope checks apply, the managed-identity agent Process child is
  still minted through the manager before it is spawned, and a delete still
  revokes the lease before anything owned is marked deleting.
- The managed-identity agent's Process child is now a declared `ChildCreation`
  on the Credential declaration, pinned to the minijail Process Provider's own
  exported reference rather than a daemon-side literal, and the Credential
  family's `credential_driver` module is gone from the daemon.
- The interaction family is now six per-type crates -
  `d2b-provider-wayland-policy`, `d2b-provider-wayland-session`,
  `d2b-provider-audio-service`, `d2b-provider-audio-binding`,
  `d2b-provider-shell-pool`, and `d2b-provider-shell-session` - one per
  resource type. Each owns its type's driver, spec decoder, factory, row
  vocabulary, and driver declaration; the six descriptors register through the
  registry, so the daemon's `interaction_driver` module, its decoder loop, and
  its type and provider literals are gone. The shared driver engine (the
  reconcile, recover, finalize, and delete verbs, the spec-envelope decode, the
  manager-child plumbing, and the effect port) lives in the family's root
  crate, the daemon keeps the production effects, and the display supervisor's
  and audio Provider's child intents reach the session and binding crates
  through ports the daemon implements. Operator-visible behavior is unchanged:
  the same six types are served, the same children are ensured in the same
  order, and the same teardown ordering runs.
- The Network, USB, and security-key families left the daemon's shared
  provider driver. Network now lives in `d2b-provider-network-local` beside
  the reconciler it drives, the USB Service/Binding types in
  `d2b-provider-device-usbip`, the security-key Service/Binding types in
  `d2b-provider-device-security-key`, and the `Device` type - one
  ResourceType served by four hardware Providers - in the new
  `d2b-provider-device` crate. Each crate declares its own rows, children,
  dependency references, and effect port; the daemon keeps the production
  effects behind those ports and registers every type through its declaration.
  The shared driver flow (row resolution, child ensures, owned-child
  retirement, status projection) lives in `d2b-provider-toolkit`, so no two
  families can diverge on it. Operator-visible behavior is unchanged: the same
  rows, Provider identities, controller references, repair cadences, and
  teardown ordering run, and `packages/d2bd/src/shared_provider_driver.rs` is
  gone.
- The controller family's nine resource types now have one driver crate each:
  `d2b-provider-zone`, `d2b-provider-zone-link`, `d2b-provider-provider`,
  `d2b-provider-role`, `d2b-provider-role-binding`, `d2b-provider-quota`,
  `d2b-provider-emergency-policy`, `d2b-provider-resource-export`, and
  `d2b-provider-resource-import`, plus the policy-type crates
  `d2b-provider-command`, `d2b-provider-operation`, and
  `d2b-provider-seccomp-profile`. Each crate owns its type's driver, spec
  decoder, and driver declaration; the daemon registers all twelve through the
  registry and no longer holds a core-family driver module. The twelve crates
  keep exactly the behavior the fixed Core process had: eight types converge as
  metadata, `Provider` re-observes its owned controller `Process` and state
  `Volume` rows and republishes its phase, and every type drains owned children
  before it retires. Failure kinds, phases, and teardown ordering are
  unchanged.
- The three policy types (`Command`, `Operation`, `SeccompProfile`) are
  declared as standard, converted resource types with their drivers registered,
  so their descriptors are reachable and the plane's driver-registry coverage
  fence closes over them. Their rows still commit with the committed policy
  rows; this change adds no seed work.
- The resource-domain modules left the controller-session library with the
  types they belong to: the Zone status projection moved to
  `d2b-provider-zone`, the zone-link enrollment state machine and its cursor
  ownership to `d2b-provider-zone-link`, the positive authorization decision
  cache to `d2b-provider-role`, and the provider lifecycle policy to
  `d2b-provider-provider`. `d2b-core-controller` keeps only resource-agnostic
  machinery - the assignment transport, the coordinator, the migration
  receipts, the fixed handler catalog, the owner-child reconciler, and the
  Host-global authority index with its durable operation adapter.
- The fixed provider identities (`Provider/system-core`,
  `Provider/system-minijail`) are taken from the crates that declare them
  (`d2b-provider-system-core`, `d2b-provider-process-minijail`) instead of
  being restated by the driver.
- The Host and User bootstrap drivers now live in their own
  `d2b-provider-host` and `d2b-provider-user` crates, each with the type's
  spec decoder, its effect port, and the driver declaration the resource plane
  registers the type by; `d2b-provider-system-core` keeps the Host/User
  reconciler realizers the daemon's effect implementations drive. The daemon
  registers both types through their declarations, so the daemon-side
  `system_core_driver` module and the per-type decoder table entries that only
  existed for these two types are gone. Operator-visible behavior is
  unchanged: the same Host Provider fence, the same bounded probe with its
  degraded fallback, the same local User discovery, and the same failure
  kinds.
- The Activation driver and the telemetry pair now live in their own provider
  crates: `d2b-provider-activation-nixos` owns the `NixosGeneration` driver,
  its spec decoder, its factory, and the declaration the plane registers the
  type by, and `d2b-provider-telemetry-service` and
  `d2b-provider-telemetry-binding` own the two telemetry types with their
  decoders and declarations. The daemon keeps the production activation
  effects behind the port the driver declares and registers all three types
  through their declarations, so the `activation_driver` and
  `semantic_binding_resource_runtime` modules and their decoder table entries
  are gone. Operator-visible behavior is unchanged: the same three types are
  admitted, the preserved `ApplyHostGenerationHandoff` dispatch and its closed
  result mapping run unchanged, and the telemetry pair keeps its verbs,
  execution domains, and provider-declared child creations.
- The Guest family now lives in its own `d2b-provider-guest` crate: the
  `Guest` driver over the four runtime Providers, its spec decoder and driver
  factory, the family's registration and child-creation declarations, and the
  Guest-side target-control service and host-side channel. The daemon
  registers the family through its descriptor and keeps only the production
  effect implementation behind the port the crate declares, so
  `guest_driver.rs` is gone. Operator-visible behavior is unchanged: the same
  four Providers are admitted, the Cloud Hypervisor children stay
  controller-owned, and validate, recover, reconcile, finalize, and delete run
  the same per-kind verbs.
- `d2b-provider-runtime-cloud-hypervisor`, `d2b-provider-runtime-qemu-media`,
  `d2b-provider-runtime-azure-container-apps`, and
  `d2b-provider-runtime-azure-virtual-machine` are renamed to
  `d2b-provider-guest-cloud-hypervisor`, `d2b-provider-guest-qemu-media`,
  `d2b-provider-guest-azure-container-apps`, and
  `d2b-provider-guest-azure-virtual-machine`, beside the family crate they
  realize. All four keep their provider identities
  (`Provider/runtime-cloud-hypervisor`, `Provider/runtime-qemu-media`,
  `Provider/runtime-azure-container-apps`,
  `Provider/runtime-azure-virtual-machine`), their packaging dossiers, and
  their exported names, and every Cargo, Bazel, Nix, copied-Guest, and
  packaging-matrix reference follows the new names.
- The shared-driver exemption ratchet in `check-provider-crate-layout` is
  empty: no module in a shared crate declares a resource driver, so the list
  retires its last entry with the Guest move and a driver outside a provider
  crate fails with its module path instead of an exemption.

- The controller family's policy types now have their authored spec schemas:
  `Command` (executable, argv placeholder slots validated against the declared
  parameters, JSON Schema parameters, worker `roleRef`, and the intent facet),
  `Operation` (payload schema, destructive flag, secret-access ceiling, audit
  facet with retained fields, redaction keys and target label, audit-join
  facet, authority facet, fd contract, bounds, payload provenance, and the
  inherited wire tag), and `SeccompProfile` (syscall allowlist, namespace and
  cgroup sets, and inline device-node binds). A payload schema is always a
  closed object, and a `writeOnly` property never carries a
  default/enum/const/examples value. The compiler's schema farm now resolves
  every standard ResourceType.
- The foundation seed commits the committed policy rows in the order the
  resource plane depends on them: the zone itself, the declared postures,
  roles, and commands, the provider self-bindings, the spawn operations the
  process controller materializes from those commands, and the operator
  bindings. Resolution is declare-then-validate over the committed set, so the
  command-to-role-to-operation cycle resolves without an ordering hack and a
  refused seed writes nothing. An uncommitted profile, command, role,
  operation, or principal is a named refusal.
- Operation materialization is authorized by the process controller's own
  committed self-binding alone: a role that lacks the command scope cannot
  publish the operation, and a controller with no binding materializes
  nothing.
- Principal uid/gid come from a committed allocation document; manifests
  declare principals by name only, an unseen name takes the next free id in
  the reserved range, and an id a host account already owns is refused. The
  ids a name was allocated are stable across restarts and hosts, and a role
  posture naming an unallocated principal refuses.
- A zone-local resource plane now refuses a write to a system-homed type
  (`Command`, `Operation`, `SeccompProfile`) with the terminal named refusal
  shape the plane partition uses; only the foundation plane the seed runs on
  carries those rows.
- Role rows carry the authority facet (`operationRefs`, `commandRefs`) and the
  optional posture facet (`seccompRef`, `principalRef`, capabilities,
  namespaces, mounts, umask, and the user-namespace flag), and RoleBinding
  rows carry the typed scope lists (`resourceRefs`, `zoneRefs`,
  `executionRefs`) beside the existing subjects and scope narrowing. Both
  facets are additive: existing rules, subjects, and committed rows keep their
  exact wire shape.

- Broker operations are now one committed catalog. A committed row per
  operation carries the wire discriminant it inherits, its owner in the
  three-way triage (a family driver, the broker itself, or a transport concern
  the envelope does not carry), the declaring provider crate, the profiles that
  admit it, the authorization facet, and every typed audit record it can emit.
  The wire's profile catalogs, the closed broker-operation inventory, the
  private broker authorization rows, and the typed audit fields are generated
  views of those rows (`xtask gen-broker-operations`, drift-checked by
  `//packages/xtask:gen_broker_operations_drift`), and a completeness gate that
  runs with the broker's tests compares every view against the rows and names
  any variant, row, profile, authorization, or audit mismatch.
- The broker gained the generic operation envelope. One invocation resolves the
  committed row, validates the payload against the row's declared shape,
  authorizes the caller against the row's committed grants, audits the attempt
  with an invocation identifier, and dispatches to the handler of the declaring
  crate. The wire gained one generic `Invoke` request carrying the operation
  name, the Zone, and the payload, so a new operation is reached by committing a
  row and registering a handler rather than by adding a wire variant. Deny by
  default holds at every step: an operation no row declares, a declared
  operation this broker holds no committed row for, and a caller no committed
  grant covers each refuse with a named code and exactly one audit record, and
  the refusal response names the operation and the code.
- `docs/reference/broker-operation-triage.md` is the operator view of the
  triage, generated from the committed rows. The historical dispositions table
  it was seeded from is kept as the triage input.
- A reserved broker operation now refuses with a closed deferral marker: each
  still-stubbed row carries one of `future-work`, `reserved`, or
  `bootstrap-only`, and that value is what the refusal puts in `targetWave` and
  in its `operation_fields` audit join, in place of the delivery-wave label the
  dispatcher used to name.

- The Nix closed inventories are generated from the declarations instead of
  being restated beside them. `xtask gen-nix-inventories` emits the standard
  ResourceType registry (projected from the resource contract's own list, the
  one declaration the resource plane already serves), the Provider projection
  ownership table and the compiler option key each projection lands on (one
  table consumed by the three bundle/compiler folds instead of three copies),
  and the resource vocabularies: the zone-control type set, the RoleBinding
  subject vocabulary and the relay-bound types, the Role resource and session
  verb sets, the shared envelope field lists, the committed schema pointers,
  and the qualified types the compiler schema farm carries. The hand copies in
  `resources.nix`, `resources-bundle.nix`, `resources-zone-control.nix`,
  `options-zones-resources.nix`, `bundle-zones.nix`, `zone-resources.nix`,
  `resources-zones-processes.nix`, `resource-compiler.nix`, and
  `provider-projection-validate.nix` are gone, and
  `//packages/xtask:gen_nix_inventories_drift` compares every generated file
  byte-for-byte against the generator.
- `nixos-modules/host-users.nix` is generated from the committed principal
  allocation: `d2b-zonert`'s uid/gid come from the allocation instead of a
  hash derivation, and the per-Device TPM accounts stay derived from the
  trusted bundle rows. The daemon principal stays the module that runs the
  daemon's account, with its allocated id pinned for the layout owner and the
  broker.
- The host contract aggregates the host-side rows into one document with the
  bundle's framing: the operator RoleBinding rows authored in Nix (each with
  the Zone and owner reference it was authored with), the committed principal
  allocation, and the declared storage trees with their ACL rows and posture
  owners. One preimage, one `d2b-digest/v1` frame, one digest beside it, and a
  golden digest pinned by the `host-contract` unit surface, so a row that
  moves without regenerating the document fails the gate.

- The client-facing layers read generated catalogs instead of hand tables.
  `xtask gen-layer-catalogs` emits three crate-local views, drift-checked by
  `//packages/xtask:gen_layer_catalogs_drift`: the CLI's surface catalog (the
  resource type registry, the typed nouns and their types, the typed-verb
  gates, the execution targets, the controller-owned types, the no-isolation
  vocabulary, the mutation verbs, and the process providers), the audit
  crate's record catalog (the converted registry in full plus the vendor
  pseudo type, the mutation verbs projected from the Role contract, and the
  process providers), and the provider contracts crate's telemetry catalog
  (the standard registry as the resource type label domain, the API verbs
  projected from the Role contract, and the process provider label domain).
  The CLI's typed-noun dispatch, per-type gating literals, execution-target
  gates, no-isolation literals, audit type/verb/provider lists, and share
  exportability heuristic are deleted; the share admission now asks the
  declared semantic projection contract. The audit type vocabulary is the
  registry in full, so `VolumeBinding`, `EmergencyPolicy`, `Command`,
  `Operation`, and `SeccompProfile` are record subjects instead of being
  missed by a stale copy, and the metric label resource-type domain is the
  complete standard registry for the same reason.
- `d2b-resource-compiler` reads the declared registry instead of its own
  type list: `ADDITIONAL_RESOURCE_TYPES` is deleted and a resource type is
  recognized when the converted-resource registry carries it or a declared
  semantic projection pair names it. The bootstrap external-reference list is
  deleted too: the compiler input carries the declared system Providers
  (`systemProviderNames`), which `bundle-zones.nix` feeds from the generated
  Provider catalog's fixed-bootstrap rows.
- The converted-resource-type list has one declaration again.
  `d2b-contracts::identity::V3_CONVERTED_RESOURCE_TYPES` is the only list:
  the generated crate-local catalog, its generator, its `--check` drift gate,
  the generated-module tree it lived in, and the two fence tests that compared
  the copies are deleted, and the resource-type vocabulary derives
  `WellKnownType::ALL` from the authority const instead of restating it. The
  plane's two startup cross-checks read the authority const directly, so a
  type added to the list reaches the vocabulary and the plane with no second
  edit.
- `d2b-core::runtime` re-exports the eight runtime capability and service DTOs
  that `d2b-contracts::runtime` already owns, instead of declaring a second
  field-for-field copy: `RuntimeOperationCapabilities` (with its
  `local_nixos`/`local_qemu_media` presets, which move to the owning crate),
  `RuntimeLifecycleCapabilities`, `RuntimeMediaCapabilities`,
  `RuntimeDisplayCapabilities`, `RuntimeGuestCapabilities`,
  `RuntimeStorageCapabilities`, `RuntimeServiceRole`, and
  `RuntimeServiceSummary`. The `ProcessRole`-derived helpers stay in
  `d2b-core`: the `From<&ProcessRole>` conversion and a `service_summary`
  constructor replace the inherent summary constructor. Wire shape, serde
  attributes, and every caller-visible behavior are unchanged.
- The dead contract surface is deleted: `d2b-contracts-zone-session`'s
  second Zone-bundle DTO module and its fixture test (the live bundle DTOs
  live in `resource_bundle`), four zero-caller `d2b-resource-api` modules
  (the API metrics inventory, the Zone service dispatch seam, the quota gate,
  and the emergency gate), `d2b-provider`'s installation, share-adapter, and
  forwarding admission modules, and `d2b-contracts`' unconsumed usbip effect
  port, its one-line provider-effects re-export, and its one-constant
  auth-wire module. No production path referenced any of them. The API watch
  sink stays: its impl is the writer end of the bus watch-delivery credit
  path, which is built and tested but not yet wired to a producer.
- `d2b-resource-api` no longer depends on `d2b-telemetry`, and
  `d2b-provider` no longer depends on `d2b-bus` or `d2b-zone-routing`; the
  removed modules were their only users.

- The eleven declaration-only metadata resource types (`Role`, `RoleBinding`,
  `Command`, `Operation`, `Quota`, `EmergencyPolicy`, `ResourceImport`,
  `ResourceExport`, `SeccompProfile`, `Zone`, `ZoneLink`) no longer each ship a
  copy of the same driver: one shared implementation in `d2b-resource-runtime`
  and one shared declaration in `d2b-resource-types` serve all of them, and
  each crate keeps the type's identity. Their registration suites assert the
  same contract through one shared assertion. Operator-visible behavior is
  unchanged: the same stored-spec fence, the same adopted recovery, the same
  converged reconcile, the same child-first drain, and the same declared
  verbs, execution domains, reads, and built-in mask.
- The `system-core` Provider library no longer carries the unused bootstrap
  sequence, raw NSS reconciler, manifest, Host process-effect audit, the
  duplicate Host posture and status modules, the duplicate handler-status
  emitter, or the Host budget check nothing computed. Its Host reconciliation,
  its ownership allowlist, and its User discovery are unchanged.
- The `Provider` driver's in-memory status is the fields it always carried
  rather than a single-variant enum with accessors, and `ZoneLink` no longer
  restates the bootstrap-PSK and session cryptoperiod bounds that the bus
  enrollment machine owns.
