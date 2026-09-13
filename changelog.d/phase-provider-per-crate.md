### Changed

- The broker's operation envelope now forwards a validated, authorized
  invocation to the process that declares the operation's handler. The
  broker links no provider crate, so the dispatch step crosses the
  invocation over a dialed Unix seqpacket socket as the committed
  `ForwardOperationRequest` shape - the operation name, the Zone, the
  invocation identifier, and the payload the envelope already validated -
  and returns the peer's canonical result. The live envelope commits every
  row a declaring crate owns as well as the broker's own, so a family row
  is reachable; a peer that has not registered the operation, a peer that
  is absent, and a broker started with no peer all refuse named as the
  missing handler rather than serving the wrong process. The peer's socket
  is `--forward-socket`, else `D2B_BROKER_FORWARD_SOCKET`, else no peer at
  all.
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
- The dormant guest shell helper is gone. `packages/d2b-guest-shell-runner` was
  a single-shot `libshpool` CLI with no d2b dependencies whose one declared
  caller was a guest systemd unit whose socket had no in-tree client, so the
  crate goes with the image wiring it needed: the static musl package output and
  its dependency-policy check, the shpool config, the service's PAM and linger
  configuration, the guest-side shell policy options, the Rust suite with its
  Bazel suites, Make targets, and CI job, and the pins that named the crate.
  Persistent guest sessions are served by the shell family: a
  `ShellPool`/`ShellSession` carries a `Host` or `Guest` execution reference and
  a login-shell artifact, guest placement is validated against the user domain,
  and the per-session supervisor service is served over the Guest
  ComponentSession route with the session's supervisor owned as a target-local
  Process child. The `d2b-sk-waybar-helper` package output, whose sources were
  already deleted, is dropped in the same pass.
- The resource compiler reads the declared bootstrap boundary instead of
  matching two Provider IDs: the compiler input's declared system Provider
  names (the Provider catalog's fixed-bootstrap rows) are what keeps an
  artifact's components in-process, and what admits an in-process bootstrap
  component in the first place. A boundary that drops a declared bootstrap row
  no longer compiles that row's artifact or projects its Provider, so the
  declaration, not a list in the compiler, is the authority.
- The static controller projection binds Device-owned worker templates from
  the rows the Device Providers declare - the row's `template` plus its owning
  Device's declared `providerRef` - instead of one arm per Device family, so a
  new Device Provider that declares the same row shape is bound with no
  compiler edit, and the serving worker's Provider reference and template are
  read from the one core declaration the resolver's mint classifies with
  rather than respelled here. The declared row, its digest-pinned executable,
  and the closed posture its template pins are unchanged.
- The CLI's built-in top-level command registry is derived from its own
  parser: the hand-maintained 33-name table is gone, a command the parser does
  not declare is not a built-in, and the three Provider projection carriers
  (`audio`, `clipboard`, `display`) stay outside the registry because the
  declaring Provider names them through its own `cliProjection`. A typed noun
  the generated CLI catalog declares is asserted to be a parser command, so
  the catalog and the parser cannot drift apart.
- The telemetry `op` label domain is generated from the committed broker
  operation rows - `gen-layer-catalogs` projects their wire variants into the
  provider contracts catalog - instead of a hand list of 21 names: every
  operation a broker request can name labels a data point, a row the catalog
  retires leaves the domain with it, and the two redaction fixtures that
  pinned `vmStart` as an admitted value now use a committed family operation.
  The compiler's inline-secret check stays key and value keyed for now: no
  resource schema marks a property `writeOnly`, so the declared secret surface
  it would read does not exist yet, and the check moves onto the marker when
  the schemas grow it.
- Every broker row no family owns now records why it is not provider-owned, and
  the completeness gate fails a row that does not: `Hello` and
  `ExportBrokerAudit` (the broker's own handshake and self-audit),
  `ApplyHostGenerationHandoff` (the broker executes the handoff against its own
  helper path and state dir, and the activation family requests it through its
  effect port), and `PrepareSwtpmDir` (the broker's pre-spawn step provisions
  and hardens the per-VM swtpm state dir before the child starts). The reason is
  a committed row facet, it reaches the generated catalog and the operator
  triage table, and a row that drops it fails both the generator and the
  broker's gate.
- `check-provider-crate-layout` now watches `d2b-resource-runtime` and
  `d2b-resource-types`, so the framework's shared declaration-only metadata
  driver is policed where it lives instead of sitting outside the monitored
  roots. The framework driver and its factory are named as the one allowed
  declaration in those crates; a per-resource driver parked in either crate
  still fails, and an allowance that stops matching the tree fails with it.
  The driver signal is the `ResourceDriver for` shape rather than a line
  prefix, so a declaration behind a visibility qualifier, an attribute, or
  another item on the line is still seen, commented-out examples are not
  declarations, and test modules are not production placement.
- The check now fails on a comment inside a monitored root that cites a
  repository path or a crate-qualified module path the tree no longer has,
  and `check-provider-crate-layout --fix` removes the mechanical shapes (a
  citation-only line, a parenthetical holding only the citation, and a
  trailing clause after a comma or dash) while reporting the citations woven
  into a sentence for a human to restate. Every rewrite is comments-only and
  check mode verifies it afterwards. The three citations in the monitored
  roots that had drifted now point at the module that holds the behavior, the
  policy matrix's system-core row cites `src/host.rs` instead of the deleted
  reconciler module, and the generated provider catalog shape is regenerated
  from that row.
- `gen-package-policy-inputs` prunes context directories the current context
  set does not name: `--write` deletes them in deterministic order and
  reports each removal, and `--check` fails naming them, so a context retired
  with its crate cannot strand protected files that nothing regenerates or
  verifies.

- The CLI runs its transport on a process runtime: `CliSocket` is a
  non-blocking seqpacket socket driven by descriptor readiness over the same
  4-byte-length-prefixed envelope, and the hand-rolled `ThreadWaker`/`block_on`
  pair is gone. Connect, send, and receive carry the command's deadline, so
  `d2b audit` against a daemon that accepts and then goes silent exits with
  `deadline-exceeded` instead of parking in a receive forever, an unreachable
  daemon still reports `zone-unavailable`, and the interactive shell bounds
  each named-stream round trip at five seconds, so a wedged peer ends as a
  named transport failure rather than a hung terminal.

- The rule that nothing blocks an executor worker is enforced by tooling
  instead of memory. `clippy.toml` at the workspace root carries the deny list
  of blocking APIs - standard mutex and read-write lock acquisition,
  condition-variable waits, blocking channel receives, thread sleep, blocking
  filesystem and subprocess calls, and blocking socket connect, accept, read
  and write including the `nix` socket entry points this tree actually uses -
  each entry naming why the call blocks and the replacement this workspace
  already ships (`tokio::time`/`sync`/`fs`/`process`/`net`, `tokio::io::unix::
  AsyncFd` as `d2b-session-unix` wraps it for sequence-packet sockets, the
  `d2b-core` bounded loader worker, the toolkit's notify-plus-timeout drain,
  the daemon's bounded-admission semaphore). The workspace lint table denies
  `clippy::disallowed_methods` beside `clippy::await_holding_lock` and its
  interior-mutability sibling: the first catches a blocking call that awaits
  nothing, the other two catch a synchronous guard held across a suspension
  point, and neither sees what the other does, so both are on. The exception
  for a genuinely synchronous path - a command-line-only path, a dedicated
  blocking worker, test scaffolding - is an inline `#[allow(..., reason = "..")]`
  that the crate policy check must find on its tracked list, so a reasonless or
  untracked allow fails rather than decorates. Three facts from landing it:
  the configuration alone arms the lint at its default warn level in every
  crate clippy checks, so the manifest levels decide enforcement and not the
  config; the workspace rustflags set `-D warnings`, which makes `warn` fail
  exactly as `deny` does; and `clippy::await_holding_invalid_type`, the nightly
  lint for values that must not be suspended across, does not fire on the
  pinned stable toolchain, which is recorded in `clippy.toml` rather than
  assumed. The census at 38d1b7407 - 4948 distinct blocking-API call sites
  (2196 production, 2752 test) and three lock-across-await sites - is still
  being worked off, so `disallowed_methods` and `await_holding_lock` are
  temporarily allowed at the workspace level and in the six members that carry
  their own lint posture instead of inheriting the table;
  `await_holding_refcell_ref` is denied outright at zero hits. The numbers,
  the census command, and the removal condition sit in the root manifest next
  to the allowance.

### Removed

- Seven broker operations nothing in tree constructed are gone with their rows,
  dispatch arms, and wire variants: `ValidateBundle`, `ResourceActivationAudit`,
  `Invoke`, `PauseBroker`, `ResumeBroker`, `BindUnixSocket`, and `SetSocketAcl`.
  No daemon or CLI caller existed for any of them - the generic `Invoke` variant
  was superseded by the committed `ForwardOperationRequest` carrier the broker
  dials, and the other six were refusals, a read-only probe, or a dispatcher
  entry the daemon stopped naming. The removal takes the whole surface with it:
  the committed rows and every generated view of them (the broker catalog, the
  profile catalogs, the `W3BrokerOperation` inventory, the private authorization
  rows, the triage table, and the telemetry label domain), the typed wire
  request and response shapes, the audit record shapes, the bootstrap probe
  wire's matching call shapes (including its looser bundle-exists check), the
  broker's capability advertisement, and `d2b_core::manifest`, whose strict
  v0.4 manifest parse existed only for the retired dispatch. A previous-protocol
  client that still sends a retired request is refused as an unknown variant,
  and the broker round-trip budget now measures a live read-only operation
  instead of the removed probe.

- Guest enrollment is served. The landed `zone-bootstrap` and `zone-enroll`
  handlers now have a serving runtime: it reads one accepted transport with a
  bounded frame and a bounded call allowance, dispatches whichever of the two
  closed calls decoded rather than trusting a frame position, mints the
  runtime-issued single-use admission from the allocator's own placement
  lookup, and answers with the contract's own named refusal - an absent
  placement, a revoked authority, an expired issuance, a replayed bootstrap,
  and a frame that is not a call each refuse by name. The daemon binds the
  endpoint per guest on the host side of the guest's own vsock socket family,
  but only where the committed facts compose a placement: a committed
  `ZoneLink` row in the guest's Zone whose transport settings name that Guest,
  plus the guest identity the host published under the VM state root. A
  deployment that declares no such guest enrollment binds nothing.
- Two declarations that nothing referenced are gone: the duplicated
  `ZONE_SERVICE_NAME` in the bus routing module, which the Zone routing
  service's own frozen wire name already defines, and the duplicate
  `PROVIDER_REF` inside the qemu-media Guest type module, whose reader now
  resolves the crate root's public declaration.

### Fixed

- A provider that declares one storage root twice is refused by name instead
  of wedging the plane: the duplicate-root refusal recorded itself through
  the same non-reentrant ledger lock the claim already held, so the claim
  hung. The refusal now goes through the seat that writes the ledger the
  caller holds, and every other refusal path in the port was audited for the
  same shape.
- The daemon's audit chain has one appender thread: callers admit a record
  into a bounded queue and await its append, so no request or startup path
  holds a lock across the append, the fsync, or retention pruning, and the
  record order on disk is the order the records were admitted.
- VM start no longer parks an async worker on readiness: the wait is awaited
  (its interval, and the liveness probe's own async seat), the synchronous
  predicates run on the blocking pool, and the provider-managed probe
  observes through the async seat instead of driving a runtime per poll. The
  same treatment reached resource-plane construction, which now awaits its
  store open, foundation seed, provider start, and manager spawn instead of
  driving them from blocking sections.
- The daemon's provider effect seats run on the blocking pool behind bounded
  admission rather than on a thread and a runtime built for each call, and a
  call past the cap is refused rather than adding another thread. The
  interaction accept loops park in the kernel on their listener instead of
  sleeping per iteration, and the synchronous seat that drives async work
  reuses one process-wide runtime instead of building one per call.
- The daemon's async paths await the core loaders' bounded worker seat
  instead of resolving the bundle and running the host check inline: `serve`
  (both startup loads), `run_startup_autostart`, and the two Guest
  component-session connects await `load_bundle_resolver_on_worker`,
  `serve_guest` awaits `BundleResolver::load_on_loader_worker`, and
  `dispatch_host_check` runs its bundle, host, and closure reads plus the
  check as one job on the worker. A saturated queue or a dead worker
  surfaces as the loader's named refusal rather than parking an executor
  worker for the seconds the reads, hash verification, and
  `nft`/`systemctl` probes take. The remaining `load_bundle_resolver`
  callers are synchronous dispatch handlers on the connection thread and
  are unchanged.

- The broker serves requests concurrently. Its accept loop is an async loop on
  the tokio reactor that hands each accepted connection to its own task behind
  an in-flight gate, and the frames cross a nonblocking `SOCK_SEQPACKET`
  descriptor through `tokio::io::unix::AsyncFd` - the pattern the session crate
  already uses - instead of a blocking read and write inline on the accept
  thread. A caller that connects and then sends nothing no longer holds the
  next caller's request behind it.
- The broker's request body runs on a bounded dispatch pool rather than on the
  accept thread or a reactor worker. The steps with no async form - the
  per-request bundle reload, the handlers' subprocess and filesystem work, the
  audit append - run on a fixed worker set with one bounded queue each, and the
  connection task awaits its own job, so a queued request waits in async time
  instead of in a thread and the pool's size is a bound rather than a per-call
  resource.
- The broker's forwarding dial is async and deadline-bounded. The seqpacket
  connection is nonblocking, the whole round trip - dial, request frame, reply
  frame - sits under one async budget, and the kernel-level receive and send
  timeouts are gone. A peer that accepts and then never answers returns inside
  its budget instead of holding a broker thread, and a dial the kernel cannot
  complete waits out its own deadline rather than blocking unbounded.
- The broker's operation envelope and its forwarder seam are async:
  `OperationDispatcher::dispatch` and `OperationForwarder::forward` return
  boxed futures so `dyn` dispatch survives, `BrokerEnvelope::call` is async,
  and a local handler still answers from a ready future.
- The obs-vsock socket ACL refresh is an async retry with a deadline instead of
  a thread per call: one pending refresh per socket, each attempt on the
  dispatch pool, and no thread is spawned for a socket that has not appeared
  yet.
- The broker's SIGCHLD reap loop shares the reactor the accept loop runs on
  instead of owning a second runtime.

### Security

- The broker-forwarding rendezvous admits a peer only after verifying its
  kernel credentials. The socket is chgrp'd to the public socket group, which
  carries every launcher and admin, so group access alone let any of them
  drive a provider operation the broker had never authorized and that carried
  no broker audit record. The accepted peer is the privileged broker (uid 0,
  host and realm alike) or the daemon's own uid where both run under one
  unprivileged user; anything else is refused by name as `ungranted-caller`.
