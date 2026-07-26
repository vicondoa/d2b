# ADR 0046 Provider dossier: audio-pipewire

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-audio-pipewire` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 10 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-audio-pipewire` crate, `AudioService` and `AudioBinding` controllers, `AudioMediator` service component |
| Depends on | `ADR-046-resource-object-model`, `ADR-046-primitive-resource-composition`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-provider-model-and-packaging`, `ADR-046-components-processes-and-sandbox`, `ADR-046-resources-volume`, `ADR-046-provider-state`, `ADR-046-componentsession-and-bus`, `ADR-046-nix-configuration`, `ADR-046-telemetry-audit-and-support`, `ADR-046-resources-host-guest-process-user` |
| Supersedes | `nixos-modules/components/audio/host.nix`, `nixos-modules/components/audio/guest.nix`, `packages/d2b-core/src/audio_policy.rs`, `packages/d2bd/src/audio_dispatch.rs`, `packages/d2bd/src/audio_host_controller.rs`, `packages/d2b-host/src/audio_argv.rs` |

## Purpose

This spec exhaustively defines the `audio-pipewire` Provider for d2b 3.0. It
covers:

- Provider identity, crate layout, and package boundary;
- the two provider-neutral qualified ResourceTypes:
  `audio.d2bus.org.AudioService` for owner authority and imported local
  service projections, and `audio.d2bus.org.AudioBinding` for
  per-Guest consumer bindings;
- both ResourceType schemas, three-layer status, lifecycle, and validation;
- the `runtime-audio` manifest dependency alias: runtime capability discovery
  without implementation-ID branches in the spec;
- the host worker Process (vhost-user-sound) and its exact execution schema;
- the `AudioMediator` same-UID user-session service: receives a declared
  pre-opened PipeWire portal FD from the user supervisor; exposes
  `SetGrant`/`SetLevel` ComponentSession service;
- FD routing: controller requests an operation-scoped typed attachment transfer;
  d2b-bus/ProviderSupervisor routes the FD directly mediator→worker LaunchTicket
  without the Process Provider knowing audio and without the controller handling
  FDs;
- the guest frontend provisioning model (virtio-snd + in-guest PipeWire stack);
- per-guestUser `GuestAudioAgent` Processes (libpipewire `AudioSet` service);
- process principal model: dedicated worker principals are core Process
  principals from the bounded pool - not runtime-created User resources;
- static components (controller, AudioMediator) created by core
  ProviderDeployment; the Provider controller reconciles AudioService and
  AudioBinding, while the AudioBinding handler creates only its owned Process and
  private Endpoint children;
- D096 export/import composition in which only an owner AudioService is
  exported and core creates exactly one projection AudioService per
  ResourceImport; ResourceExport and ResourceImport use the canonical signed
  projection-factory type/fingerprint fields, the Service retains every
  implementation Endpoint, and AudioBinding is never exported or generated as
  a projection;
- speaker mixing plus initial-v3 exclusive microphone arbitration with a
  bounded fair queue across authenticated consumer Zones; concurrent microphone
  capture is deferred until a future spec defines a concrete consent
  authorization resource and verb;
- RBAC, security invariants, and zero-broker-op controller boundary;
- async reconciliation, restart adoption, and status transitions;
- error codes and Degraded-state model;
- authoritative audit events and OTEL telemetry shape;
- Nix authoring, configuration, and resource compilation;
- implementation work items with exact source, destination, and tests;
- required crate layout (`src/`, `tests/`, `integration/`, `README.md`).

Audio is an **interaction Provider**, not a device Provider. An owner Zone has
one real `audio.d2bus.org.AudioService` authority for its PipeWire
backing. A consumer Zone has one core-owned projection AudioService per
`ResourceImport`. Every opted-in Guest has its own
`audio.d2bus.org.AudioBinding` and vhost-user-sound worker;
that AudioBinding references a same-Zone AudioService through `spec.serviceRef`.

The two ResourceType identities are audio-domain contracts, not PipeWire
implementation identities. `Provider/audio-pipewire` is their initial
implementation. PipeWire-specific desired fields are permitted only in a
strict signed `spec.provider` envelope or the Provider's own `spec.config`;
PipeWire-specific observations are permitted only in strict signed
`status.provider`. They never become fields in the provider-neutral
`spec.*`/`status.resource` schemas.

**Controller boundary**: the `audio-binding-controller` creates, updates, and
deletes owned `Process` and private `Endpoint` resources exclusively through
the resource API. It never
calls `SpawnRunner`, `OpenPidfd`, adopts pidfds, creates `Volume` resources,
or creates `User` resources. `Provider/system-minijail` or
`Provider/system-systemd` owns process launch, wait, reap, and Process status
updates. ProcessEffect audit belongs to the Process Provider.

**Execution schema boundary**: the live Process resource spec contains no
`executableRef`, `argv`, `env`, or inherited-FD endpoint records. Those are
signed component-template/LaunchTicket projections owned by the Process
Provider and never appear in the Zone store's Process ResourceSpec.

**Enforcement model**: the `AudioMediator` owns the compositor-user PipeWire
connection (received as a declared portal FD from the user supervisor/display
portal) and exposes a typed `SetGrant`/`SetLevel` ComponentSession service.
Grant and level changes are applied directly via libpipewire API calls inside
the AudioMediator. No `wpctl` EphemeralProcess, node ID, argv, binary path,
or sealed command handle is required.

**Static components**: the controller binary and the AudioMediator service
binary are created as Process resources by core ProviderDeployment when
`Provider/audio-pipewire` is activated. The controller binary registers
deterministic AudioService and AudioBinding handlers. The AudioBinding handler
creates only AudioBinding-owned child Process resources (worker,
GuestAudioAgent instances) and private Endpoints; it does not bootstrap its
own companion processes.

## Terminology mapping (baseline → v3)

All evidence citations use baseline symbol names. The v3 target name is
explicitly stated at each design boundary.

| Baseline name / location | v3 ADR 0046 target | Evidence class |
| --- | --- | --- |
| `AudioPolicyState` (`d2b-core/src/audio_policy.rs:130`) | Per-Guest `AudioBinding` spec (the `mic`/`speaker`/`speakerLevel`/`micGain` fields migrate to `AudioBinding.spec.grants`; service ownership is separate in `AudioService`) | `implemented-and-reachable` |
| `AudioGrant::On/Off` (`audio_policy.rs:98`) | `AudioBinding.spec.grants.mic`/`speaker: "on"\|"off"` | `implemented-and-reachable` |
| `LevelPercent` (`audio_policy.rs:26`) | `AudioBinding.spec.grants.speakerLevel`/`micGain: 0..=100` | `implemented-and-reachable` |
| `parse_audio_state` / `to_v2_bytes` (`audio_policy.rs:282,215`) | Used only for baseline v1/v2 state-file migration on first activation; `AudioBinding.spec.grants` is durable per-Guest intent in v3 and `serviceRef` selects the backing Service; no state file maintained after migration completes | `implemented-and-reachable` |
| `AudioArgvInput` / `generate_audio_argv` (`d2b-host/src/audio_argv.rs:47,101`) | signed component template for `vhost-user-sound-worker`; argv shape is a template projection, not a live Process spec field | `implemented-and-reachable` |
| per-VM binary copy path `/run/d2b/vms/<vm>/d2b-<vm>` (`audio_argv.rs:97`) | LaunchTicket verifier enforces the path shape against the component template; not exposed in the Process resource spec | `implemented-and-reachable` |
| `RunnerRole::Audio` (`d2b-contracts/src/broker_wire.rs:1524`) | `Process` resource with `spec.template: "vhost-user-sound-worker"`; launch owned by system Process Provider | `implemented-and-reachable` |
| `PipeWireHostController` (`d2bd/src/audio_host_controller.rs:85`) | `AudioMediator` user-session service; exposes `SetGrant`/`SetLevel` ComponentSession service; applies changes via libpipewire API | `implemented-and-reachable` |
| `QemuAudioController` (`audio_host_controller.rs:227`) | removed; audio discovers enforcement capability via the `runtime-audio` dependency alias; no implementation-ID branch in `audio.d2bus.org.AudioBinding.spec` | `implemented-and-reachable` |
| `WPCTL_PATH` / `PW_DUMP_PATH` env keys (`audio_host_controller.rs:103`) | superseded; AudioMediator uses libpipewire registry introspection and direct API calls; no wpctl binary or pw-dump subprocess | `implemented-and-reachable` |
| `PIPEWIRE_RUNTIME_DIR` env key (`audio_host_controller.rs:105`) | not a Process spec field; AudioMediator receives a declared pre-opened PipeWire portal FD from the user supervisor/display portal - it does not open the socket from the ambient runtime environment | `implemented-and-reachable` |
| `access(2)` credential posture check (`audio_host_controller.rs:134`) | replaced by AudioMediator readiness check; AudioMediator reports `ProviderSessionUnavailable` when the portal FD cannot be acquired from the user supervisor | `implemented-and-reachable` |
| `ofd_lock` / `acquire_audio_state_lock` (`audio_dispatch.rs:73,125`) | superseded; `AudioBinding.spec` is durable per-Guest intent and AudioService owns backing authority; no state file is maintained; OFD lock is removed | `implemented-and-reachable` |
| `write_audio_state_unlocked` atomic rename (`audio_dispatch.rs:221`) | superseded; no state file; grants are authoritative in `AudioBinding.spec` | `implemented-and-reachable` |
| `AudioHostEnforcementKind` / `AudioGuestEnforcementKind` (`provider_capabilities.rs:21,39`) | superseded; enforcement capability is discovered at runtime via the `runtime-audio` manifest dependency alias; no implementation-ID branch in `AudioBinding.spec` | `implemented-and-reachable` |
| `AudioProviderCapability` capability row (`provider_capabilities.rs:54`) | inline component descriptor field behind the `AudioService` implementation; not a third ResourceType | `implemented-and-reachable` |
| `AudioOp` / `AudioOpResponse` (`public_wire.rs:1934,2025`) | v3: `AudioBinding` spec mutations (`UpdateSpec`) via the resource API; no separate op wire | `implemented-and-reachable` |
| `AudioVmState` / `AudioChannelState` (`public_wire.rs:1955,1943`) | `AudioBinding.status.channels` inline status | `implemented-and-reachable` |
| `AudioEnforcementPosture` (`public_wire.rs:1848`) | `AudioBinding.status.enforcementPosture` | `implemented-and-reachable` |
| `AudioSetApplied` (`public_wire.rs:1997`) | `AudioBinding.status.lastSetApplied` | `implemented-and-reachable` |
| `AudioErrorKind` (`public_wire.rs:1870`) | `AudioBinding.status.outcome.code` closed enum | `implemented-and-reachable` |
| `AudioProviderKind` (`public_wire.rs:1889`) | removed from provider-neutral `AudioBinding.status.resource`; implementation identity is `spec.providerRef`, and bounded PipeWire-only observation belongs in `status.provider.details` | `implemented-and-reachable` |
| WirePlumber `client.conf.d/90-d2b` stream rules (`nixos-modules/components/audio/host.nix:252`) | retained as host Nix config; not a resource spec field | `implemented-and-reachable` |
| WirePlumber `monitor.alsa.rules` (`nixos-modules/components/audio/guest.nix:197`) | retained as guest Nix config; not a resource spec field | `implemented-and-reachable` |
| `services.pipewire.extraConfig.client."90-d2b"` (`host.nix:252`) | compiled host configuration owned by the `Provider/audio-pipewire` Nix module; not an AudioService/AudioBinding base field | `implemented-and-reachable` |
| `d2b.site.audio.inputTargetNode` (`host.nix:253`) | `Provider/audio-pipewire.spec.config.captureAlias` - bounded named alias (`^[a-z][a-z0-9-]*$`) resolved privately by AudioMediator via libpipewire registry; not a PipeWire node ID or socket path | `generated-or-eval-contract` |
| `vhost-device-sound v0.3.0` (`pkgs/vhost-device-sound/default.nix`) | `spec.artifactId` in `Provider/audio-pipewire` pointing to Nix artifact catalog entry | `implemented-and-reachable` |
| `microvm.extraArgsScript` CH audio injection (`guest.nix:112`) | `Guest.spec.audioExtension` arguments derived from the runtime-audio capability reported by the Guest's Runtime Provider; values such as `virtio_id` and `queue_sizes` are not spec fields | `implemented-and-reachable` |
| `/var/lib/d2b/vms/<vm>/state/audio-state.json` (`host.nix:341`) | superseded legacy file; per-Guest grants live in `audio.d2bus.org.AudioBinding.spec`, while physical authority lives in `AudioService`; no per-Guest state file in v3 | `implemented-and-reachable` |
| `/run/d2b/locks/audio-<vm>.lock` (`host.nix:366`) | superseded; OFD lock and state file are removed in v3 | `implemented-and-reachable` |
| `d2b-<vm>-snd` system user (`audio_argv.rs:140`) | superseded; dedicated worker principals are core Process principals from the bounded pool allocated by the Process Provider; not runtime-created `User` resources managed by the audio controller | `implemented-and-reachable` |
| `d2b.guestControl.wpctlPath` (`guest.nix:139`) | superseded; AudioMediator uses libpipewire API directly; no operator-visible wpctlPath option in v3 | `generated-or-eval-contract` |
| `d2b.audio.users` guest option (`guest.nix:92`) | `AudioBinding.spec.guestUsers` list of `User/<name>` ResourceRefs; Nix/compiler sets `spec.groups: ["audio"]` on each referenced guest `User` resource at compile time; runtime API-created AudioBinding verifies `User.status.groupMembershipVerified` before sidecar start; no runtime `extraGroups` mutation | `generated-or-eval-contract` |
| `minijail-profiles.nix` audio role block / `seccompPolicyRef = "w1-audio"` | `Process.spec.sandbox.seccompClass: audio-pipewire-worker` | `implemented-and-reachable` |
| `minijail_audio_usbip.rs` Layer-1 contract tests | retained and extended in `d2b-provider-audio-pipewire/tests/` | `implemented-and-reachable` |
| `d2b audio status/set-volume/mute` CLI ops (`packages/d2b/src/`) | v3: `d2b resource update audio.d2bus.org.AudioBinding/<name>` or a provider-specific `d2b audio` view | `implemented-and-reachable` |

## Resolved design decisions

All design decisions are resolved in this revision.

| ID | Question | Resolution |
| --- | --- | --- |
| DRAUDIO-001 | Separate `AudioBinding` ResourceType or extend `Device`? | `audio.d2bus.org.AudioBinding` is an independent ResourceType. Audio is an interaction Provider; it does not model a Device inventory/arbitration/claim lifecycle. |
| DRAUDIO-002 | Per-Guest `AudioBinding` or Zone-global? | Per-Guest. Each Guest has independent grants, levels, and enforcement posture. |
| DRAUDIO-003 | Where does the vhost-user-sound socket path live? | Controller-generated private implementation detail. Never appears in `AudioBinding.spec`, `AudioBinding.status`, API responses, audit records, OTEL attributes, or any broker configuration. |
| DRAUDIO-004 | How does the controller enforce PipeWire stream routing after a grant change? | The AudioBinding controller calls the same-Zone AudioService selected by `serviceRef`. An owner Service dispatches to its local AudioMediator; a projection Service routes over its ResourceImport encrypted stream to the remote owner. Only the owner AudioMediator applies libpipewire changes. `AudioBinding.spec` is durable per-Guest intent; no state file is required. |
| DRAUDIO-005 | How is the `application.name = "d2b-<guest>"` PipeWire stream identity established? | The component template for `vhost-user-sound-worker` is a signed LaunchTicket projection that sets the per-Guest binary copy path as argv[0]. `libpipewire`'s `init_prgname()` reads `/proc/self/exe`. This is a template projection; it does not appear in the live Process resource spec. |
| DRAUDIO-006 | WirePlumber stream rules: resource spec or host Nix config? | Host Nix config. The operator capture target is stored only as `Provider/audio-pipewire.spec.config.captureAlias` - a bounded named alias (`^[a-z][a-z0-9-]*$`, ≤64 chars). The AudioMediator resolves it to the actual PipeWire node object via libpipewire registry introspection at runtime, privately. The alias never appears in AudioService/AudioBinding base spec/status, stream rules, audit, or telemetry. |
| DRAUDIO-007 | Guest PipeWire stack: resource spec or Nix guest config? | Guest Nix config. The in-guest virtio-snd module, PipeWire stack, WirePlumber virtio-snd profile, and diagnostic packages are Nix guest module concerns. |
| DRAUDIO-009 | Mic direction: null-target sentinel vs explicit routing? | WirePlumber stream rules in `client.conf.d/90-d2b` set initial stream-creation properties. Live changes reach the owner AudioMediator through the referenced AudioService (local or projection route) and use libpipewire. No worker restart or state file is required. |
| DRAUDIO-011 | How does the vhost-user-sound worker access PipeWire without ambient socket exposure? | A same-UID user-session `AudioMediator` receives a declared pre-opened PipeWire portal FD from the user supervisor/display portal (not from the ambient runtime environment). The controller requests an operation-scoped typed attachment transfer; d2b-bus/ProviderSupervisor routes the FD directly mediator→worker LaunchTicket without the Process Provider knowing audio and without the controller handling FDs. No socket path, SetSocketAcl, or `PIPEWIRE_RUNTIME_DIR` env entry appears in any resource spec, status, broker config, or public surface. |
| DRAUDIO-012 | Audio user group membership: resource spec or Nix guest config? | `AudioBinding.spec.guestUsers` is a list of `User/<name>` ResourceRefs. For Nix/compiler-declared resources, the Nix module sets `spec.groups: ["audio"]` on each referenced guest `User` resource at compile time. For API-created `AudioBinding`, the operator sets `spec.groups` on the User resources; the controller verifies `User.status.groupMembershipVerified` before starting the sidecar and fails closed if not confirmed. The controller never mutates `User.spec.groups` at runtime. |
| DRAUDIO-013 | Who owns process launch, wait, reap, and pidfd? | `Provider/system-minijail` or `Provider/system-systemd` exclusively. The AudioBinding controller creates Process and private Endpoint resource specs only. It never calls `SpawnRunner`, `OpenPidfd`, or adopts pidfds. ProcessEffect audit belongs to the Process Provider. |
| DRAUDIO-014 | Provider root configuration key? | `Provider.spec.config` (not `rootConfig`). The canonical Provider spec shape is `{artifactId; config}`. |
| DRAUDIO-015 | wpctl EphemeralProcess vs AudioMediator service? | wpctl EphemeralProcess is removed. AudioBinding calls its AudioService. Owner Service dispatches to the local AudioMediator; projection Service routes to the remote owner. Only the owner mediator uses libpipewire. No EphemeralProcess, wpctl, or node ID enters any external surface. |
| DRAUDIO-016 | Guest-side enforcement: guestd wpctl path vs typed guest service? | guestd's wpctl dispatch path is superseded. A `GuestAudioAgent` Process running in the Guest under the guest workload user's UID exposes a typed `AudioSet` ComponentSession service over vsock. The `audio-binding-controller` calls this service via libpipewire API. No wpctl binary or command path. The `d2b.guestControl.wpctlPath` Nix option is removed from v3. |
| DRAUDIO-017 | How does audio discover the Guest's audio frontend without an `audioFrontend.kind` spec field? | The Provider manifest declares a `runtime-audio` dependency alias bound to the Guest's Runtime Provider. At activation the Runtime Provider advertises typed `AudioCapability` records (e.g., `VhostUserSound { virtio_id, queue_sizes, enforcement_posture }`) via the capability protocol. The controller reads these records via the dependency alias. No implementation-ID branch appears in `AudioBinding.spec`; `Guest.spec.audioExtension` arguments are derived from the capability. If the runtime advertises no audio capability, sidecar is not deployed. |
| DRAUDIO-018 | Dedicated worker principals: controller-created User resources or core Process principals? | Core Process principals from the bounded pool allocated per provider by the Process Provider. The audio controller does not create `User` resources for worker execution identity. Human guest `User/<name>` references in `guestUsers` are observed from system-core, never created or modified by the audio controller. |
| DRAUDIO-019 | Who creates the static AudioMediator and controller Process resources? | Core ProviderDeployment creates them when `Provider/audio-pipewire` is activated. The AudioBinding handler creates only Binding-owned workers, GuestAudioAgents, and private Endpoints. The controller has no `Volume` or `User` verbs. |
| DRAUDIO-020 | How is the PipeWire FD routed from AudioMediator to worker without controller or Process Provider involvement? | The controller declares an operation-scoped typed attachment transfer when creating the worker Process resource. d2b-bus/ProviderSupervisor resolves the AudioMediator's active portal FD and delivers it in the worker's LaunchTicket. The Process Provider (system-minijail) receives and inherits the FD without knowing it is audio-specific. The controller never holds or transfers FDs directly. |
| DRAUDIO-021 | Which resource owns the physical audio authority and which resource is imported? | `audio.d2bus.org.AudioService`. The owner-Zone Service holds the D097 AuthorityDescriptor and references only same-Zone implementation Endpoints. `ResourceExport.resourceRef` and `serviceType` identify that Service; `projectionSchemaFingerprint` and `factoryFingerprint` bind its signed projection factory. The Endpoint remains a Service-owned implementation detail and is never an Export field. Core creates exactly one local projection AudioService per ResourceImport with `metadata.ownerRef: ResourceImport/<name>`. AudioBinding never holds the AuthorityDescriptor, is never exported, and is never an import projection. |
| DRAUDIO-022 | How does a Guest select local or imported audio backing? | Every per-Guest AudioBinding has a required same-Zone `spec.serviceRef` to an AudioService plus its existing Guest ownership, grants, levels, and users. Owner Services use the local AudioMediator/PipeWire backing. Projection Services route encrypted named streams to the remote owner Service and are forbidden from opening PipeWire. |
| DRAUDIO-023 | What is the per-consumer ResourceType name? | `audio.d2bus.org.AudioBinding`. `State` is reserved for resource `status`; no former ResourceType name, serde rename, API alias, schema alias, or Nix compatibility alias is accepted. |
| DRAUDIO-024 | Are the audio ResourceTypes tied to PipeWire? | No. `audio.d2bus.org.AudioService` and `audio.d2bus.org.AudioBinding` are provider-neutral audio contracts initially implemented by `Provider/audio-pipewire`. The former `audio-pipewire.d2bus.org.*` ResourceType namespace and every AudioState spelling are unknown, with no alias. PipeWire-specific desired configuration is confined to strict `spec.provider` or `Provider/audio-pipewire.spec.config`; implementation observation is confined to strict `status.provider`. |
| DRAUDIO-025 | Is concurrent microphone capture supported in initial v3? | No. The owner AudioService exposes exactly one microphone capture slot across local and imported consumers and schedules pending requests with the fixed bounded per-Zone round-robin/FIFO policy in this dossier. Speaker playback remains multiplexed and mixed. `microphone: multiplexed`, consent/approval fields, priority overrides, and concurrent-capture verbs are rejected. Concurrent microphone capture is deferred until a future normative spec defines a concrete consent authorization ResourceType and resource-API verb, including revocation and audit semantics. |

## Provider identity

```text
Provider/audio-pipewire
```

- **Crate**: `d2b-provider-audio-pipewire`
- **Package name**: `d2b-provider-audio-pipewire`
- **Declares**: one Provider identity, one controller binary (`audio-pipewire-controller`),
  one user-session service binary (`audio-pipewire-mediator`), one worker
  template (`vhost-user-sound-worker`), one guest agent template
  (`guest-audio-agent`), one manifest dependency alias (`runtime-audio`), and
  initial implementation support for two provider-neutral qualified
  ResourceTypes (`audio.d2bus.org.AudioService` and
  `audio.d2bus.org.AudioBinding`)
- **Depends on**: public neutral contracts/toolkit crates only; no `d2bd`,
  `d2b-priv-broker`, Zone-store, or other Provider internals

### Provider resource catalog

| Qualified ResourceType | Cardinality/scope | Authority/export/import semantics | Controller |
| --- | --- | --- | --- |
| `audio.d2bus.org.AudioService` | One real owner authority Service per owner Zone/physical backing; one projection Service per ResourceImport | Owner alone carries D097 AuthorityDescriptor and is the ResourceExport target. Core alone creates/deletes the projection with `ownerRef: ResourceImport/<name>`. Projection routes to the remote owner and never opens PipeWire. | AudioService handler reconciles owner/projection semantics and Service-owned local Endpoints |
| `audio.d2bus.org.AudioBinding` | Exactly one per opted-in Guest | Per-Guest grants/levels/users with required same-Zone `serviceRef`; never carries authority, is never exported, and is never an import projection | AudioBinding handler creates the worker, GuestAudioAgents, and private Endpoints |

These catalog entries register provider-neutral base schema identities.
`Provider/audio-pipewire` advertises matching schema fingerprints and is the
initial implementation selected by `spec.providerRef`. Registering the former
provider-qualified names as additional ResourceTypes is forbidden.

Standard `Process`, `Endpoint`, `ResourceExport`, and `ResourceImport`
resources are composed as children/dependencies; they are not additional
audio ResourceTypes.

### Controller components

| Component | Binary | Class | Domain | Scope |
| --- | --- | --- | --- | --- |
| `audio-service-controller` | `audio-pipewire-controller` | controller handler | system | Watches `audio.d2bus.org.AudioService`; claims/observes an owner Service's D097 authority and local implementation Endpoints, or binds a projection Service to its ResourceImport encrypted-stream route; a projection never opens PipeWire |
| `audio-binding-controller` | `audio-pipewire-controller` | controller handler | system | Watches `audio.d2bus.org.AudioBinding`; resolves its same-Zone `serviceRef`; creates/updates/deletes AudioBinding-owned worker, GuestAudioAgent, and private Endpoint resources; calls the selected AudioService; never touches pidfds, broker spawn, Volume, or User resources; `Provider/audio-pipewire` declares no Provider state Volume under D087 |
| `audio-mediator` | `audio-pipewire-mediator` | service | user | Same-UID user-session component; receives declared pre-opened PipeWire portal FD from user supervisor; ProviderSupervisor routes FD to worker LaunchTicket; exposes `SetGrant`/`SetLevel` service; applies enforcement via libpipewire API |

The controller runs as a Process in the system domain under the Host. The
mediator runs as a Process in the user domain under the compositor user's UID.
**The controller Process and mediator are static components created by core ProviderDeployment** when
`Provider/audio-pipewire` is activated; the controller does not bootstrap
them. Neither component receives a Zone store handle or a broker socket.

**Process lifecycle boundary**: the AudioBinding handler creates `Process`
resources and private `Endpoint` resources through the resource API and watches
their status. It never calls `SpawnRunner`, `OpenPidfd`, `SIGTERM`, or any pidfd
operation. Those effects belong exclusively to `Provider/system-minijail` or
`Provider/system-systemd`.

**Execution schema boundary**: the live Process resource spec contains no
`executableRef`, `argv`, `env`, or inherited-FD endpoint records. Those are
signed component-template/LaunchTicket projections.

### `runtime-audio` dependency alias

The Provider manifest declares:

```text
dependencyAlias: runtime-audio
boundTo: Guest.runtimeProvider
purpose: audio-capability-query
```

At activation the Runtime Provider for the owning Guest is queried via the
`runtime-audio` alias for its typed `AudioCapability` set. The capability
record (if present) contains implementation details such as `virtio_id`,
`queue_sizes`, and `enforcement_posture`. The controller derives
`Guest.spec.audioExtension` arguments from these values. If the runtime
advertises no audio capability, the sidecar is not deployed.

No audio capability field is stored in `AudioBinding.spec`. No
implementation-ID branch (cloud-hypervisor vs. qemu) appears in the spec or
the controller's reconcile logic - capability presence/absence and the typed
capability fields are the only dispatch surface.

### Worker process template

| Template name | Class | Domain | Role |
| --- | --- | --- | --- |
| `vhost-user-sound-worker` | worker | system | Per-Guest vhost-device-sound sidecar; long-lived; system-domain under `Provider/system-minijail`; uses its AudioBinding's resolved AudioService backend: an owner Service supplies the local AudioMediator attachment, while a projection Service supplies a local encrypted-stream route and never a PipeWire FD |

## `AudioService` ResourceType

`audio.d2bus.org.AudioService` is the provider-neutral audio
service/authority boundary, initially implemented by
`Provider/audio-pipewire`. It has exactly two roles:

- **owner**: one real Service in the owner Zone holds the D097
  `AuthorityDescriptor`, arbitrates the physical microphone/speaker backing,
  and references only same-Zone implementation Endpoints;
- **projection**: core creates exactly one local Service per `ResourceImport`
  in a consumer Zone with `metadata.ownerRef: ResourceImport/<name>`. It routes
  through that import's lease and bounded encrypted named streams to the remote
  owner Service. It never opens PipeWire and contains no remote ResourceRef,
  FD, socket, or path.

An AudioService is not per Guest. Per-Guest policy is always an AudioBinding that
references a same-Zone AudioService.

### Owner Service example

```yaml
apiVersion: resources.d2bus.org/v3
type: audio.d2bus.org.AudioService
metadata:
  name: host-audio
  zone: host
  ownerRef: Provider/audio-pipewire
  uid: <store-generated>
  generation: 1
  revision: <opaque>
  finalizers:
    - audio-pipewire.d2bus.org/service-released
spec:
  providerRef: Provider/audio-pipewire
  serviceRole: owner
  implementationEndpointRefs:
    - Endpoint/audio-pipewire-authority
  operations: [playback, capture]
  authority:
    authorityScope: physical-device
    authorityClass: audio
    authorityKey: host-default-audio
    cardinality: zero-or-one
    arbitration:
      speaker: multiplexed
      microphone: exclusive
    exportability: explicit-export
status:
  observedGeneration: 1
  phase: Ready
  conditions:
    - type: AuthorityClaimed
      status: "True"
      reason: owner-proof-verified
    - type: ServiceReady
      status: "True"
      reason: local-endpoint-ready
    - type: MicArbiterReady
      status: "True"
      reason: exclusive-slot-ready
  resource:
    serviceRole: owner
    availability: ready
    routeState: local
    implementationEndpointRefs:
      - Endpoint/audio-pipewire-authority
    activeConsumerCount: 1
    activeMicCaptureCount: 0
    pendingMicRequestCount: 0
    pendingMicZoneCount: 0
  provider:
    providerRef: Provider/audio-pipewire
    schemaId: audio-pipewire.d2bus.org/status/audio-service
    schemaVersion: 1.0.0
    observedProviderGeneration: 1
    details:
      pipeWireSession: ready
      portalAttachment: ready
  outcome:
    code: ok
    message: null
    retryable: false
  update:
    state: Current
    reasons: []
    observedGeneration: 1
    targetGeneration: 1
    disruption: None
    preserveState: true
    operationId: null
    lastAssessedAt: null
    owned: { count: 0, refs: [] }
    dependencies: { count: 0, refs: [] }
```

`authority.authorityKey` is a bounded opaque key, not a PipeWire node name,
path, serial, address, or credential. Every
`implementationEndpointRefs` entry must resolve in the Service's Zone and be
owned by this Service or by its static Provider component. The owner Service
cannot reference a ResourceImport, projection Service, or cross-Zone resource.

### Core-generated projection Service example

```yaml
apiVersion: resources.d2bus.org/v3
type: audio.d2bus.org.AudioService
metadata:
  name: host-audio-projection
  zone: work
  ownerRef: ResourceImport/host-audio
  uid: <store-generated>
  generation: 1
  revision: <opaque>
  finalizers:
    - audio-pipewire.d2bus.org/service-released
spec:
  providerRef: Provider/audio-pipewire
  serviceRole: projection
  implementationEndpointRefs:
    - Endpoint/host-audio-import-route
  operations: [playback, capture]
status:
  observedGeneration: 1
  phase: Ready
  conditions:
    - type: ImportBound
      status: "True"
      reason: encrypted-stream-session-ready
    - type: ServiceReady
      status: "True"
      reason: projection-route-ready
  resource:
    serviceRole: projection
    availability: ready
    routeState: bound
    implementationEndpointRefs:
      - Endpoint/host-audio-import-route
    activeConsumerCount: 1
  outcome:
    code: ok
    message: null
    retryable: false
  update:
    state: Current
    reasons: []
    observedGeneration: 1
    targetGeneration: 1
    disruption: None
    preserveState: true
    operationId: null
    lastAssessedAt: null
    owned: { count: 0, refs: [] }
    dependencies: { count: 0, refs: [] }
```

The projection carries no `authority` field. Its routing authority is the
local ownerRef chain
`AudioService -> ResourceImport -> ZoneLink/exportKey/lease`; no field names a
remote Service. Core is the only creator/deleter of projection AudioServices.
The audio Provider reconciles their semantic status and local route Endpoint.

### `AudioService.spec` fields

Per D089, the typed desired spec is Layer 2 and any implementation extension
uses only `spec.provider = { schemaId, schemaVersion, settings }`. The strict
base schema is:

| Field | Type | Required | Default | Bounds | Notes |
| --- | --- | --- | --- | --- | --- |
| `providerRef` | ResourceRef | yes | - | same-Zone Provider advertising the AudioService base fingerprint | Immutable; this dossier's initial implementation is `Provider/audio-pipewire` |
| `serviceRole` | enum | yes | - | `owner\|projection` | Immutable; must agree with ownerRef rules |
| `implementationEndpointRefs` | list[ResourceRef] | yes | - | 1..=4 local `Endpoint/<name>` refs | Owner: only local backing implementation Endpoints; projection: only local encrypted-stream route Endpoints |
| `operations` | list[enum] | yes | - | unique subset of `playback\|capture` | Closed service capability set |
| `authority` | AuthorityDescriptor | role-dependent | - | D097 schema | Required only for `owner`; forbidden for `projection` |

The initial PipeWire implementation accepts an absent `spec.provider` or the
exact signed envelope
`{schemaId:"audio-pipewire.d2bus.org/spec/audio-service",
schemaVersion:"1.0.0",settings:{}}`; the v1 `settings` object has no fields.
Provider-wide PipeWire desired configuration such as `captureAlias` belongs
under `Provider/audio-pipewire.spec.config`, not in this base spec. A future
per-resource PipeWire setting requires a versioned, deny-unknown
`spec.provider.settings` schema and may not shadow any base field.

Admission requires an owner Service to have `metadata.ownerRef` equal to its
selected `spec.providerRef` (initially `Provider/audio-pipewire`) and a valid
D097 descriptor with
`authorityScope: physical-device`, `authorityClass: audio`,
`cardinality: zero-or-one`, `arbitration.speaker: multiplexed`,
`arbitration.microphone: exclusive`, and
`exportability: explicit-export|forbidden`. The initial schema rejects every
other microphone arbitration value and rejects consent, approval, priority, or
concurrent-capture fields as unknown.
Admission requires a projection Service to have
`metadata.ownerRef: ResourceImport/<name>` and forbids `authority`. Core
supplies the projection spec from the matched import; operator/API creation or
mutation of a projection is rejected with `ProjectionCoreOwned`.

Every Endpoint ref is same-Zone. Unknown base fields, a cross-Zone or non-Endpoint
ref, an owner descriptor on a projection, a missing owner descriptor, or a
role/ownerRef mismatch fail admission. No schema layer permits a PipeWire
locator, FD, remote Ref, audio byte, lease handle, or named-stream key.

### `AudioService.status` fields and conditions

Per D088, universal fields remain at `status.*`; the following typed fields
are `status.resource`. Optional `status.provider` follows the same strict,
signed, bounded Layer 3 envelope as AudioBinding. All layers are written
atomically.

| Field | Type | Notes |
| --- | --- | --- |
| `serviceRole` | enum | Observed `owner\|projection` |
| `availability` | enum | `pending\|ready\|degraded\|revoked` |
| `routeState` | enum | Owner: `local`; projection: `pending\|bound\|degraded\|revoked` |
| `implementationEndpointRefs` | list[ResourceRef] | Ready same-Zone Endpoints, bounded to the spec set |
| `activeConsumerCount` | uint | Bounded aggregate count; no consumer names |
| `activeMicCaptureCount` | uint | Owner only; always `0` or `1` |
| `pendingMicRequestCount` | uint | Owner only; aggregate `0..=64`; no consumer identity or queue position |
| `pendingMicZoneCount` | uint | Owner only; aggregate `0..=64`; no Zone names |

Owner Services report all three microphone arbitration aggregates on each
material change. Projection Services omit them because the remote owner alone
arbitrates capture; a projection never mirrors queue membership or consumer
identity into local status.

For `Provider/audio-pipewire`, `status.provider.schemaId` is
`audio-pipewire.d2bus.org/status/audio-service` at `1.0.0`. Its strict
role-sensitive `details` object permits only `pipeWireSession` and
`portalAttachment` (`ready|unavailable`) for an owner Service; a projection
must omit those fields because it never opens PipeWire or receives a portal
attachment. No PipeWire field is admitted in `status.resource`.

Universal `status.outcome.code` is a closed enum: `ok`,
`AuthorityConflict`, `EndpointNotReady`, `ImportNotBound`, `ImportRevoked`,
`ProviderSessionUnavailable`, or `ProviderMisconfigured`. Message and
retryability remain universal, bounded, and redacted.

Closed conditions:

| Type | Applies to | Meaning |
| --- | --- | --- |
| `AuthorityClaimed` | owner | D097 authority index accepted and ownerProof verified |
| `BackingReady` | owner | The selected Provider reports the local audio backing Ready |
| `ImportBound` | projection | ResourceImport lease and encrypted named streams are current |
| `ProjectionRouteReady` | projection | Same-Zone route Endpoint is Ready; no PipeWire open occurred |
| `MicArbiterReady` | owner | Exclusive microphone slot and bounded fair queue are available |
| `MicQueueSaturated` | owner | Fixed total or per-Zone pending-request bound was reached |
| `ServiceReady` | both | Required local implementation Endpoints and role semantics are Ready |
| `ServiceDegraded` | both | Authority, endpoint, import, or route observation is degraded |
| `Revoked` | projection | Import/export lease was revoked; consumers must degrade |

An owner is `Ready` only when authority and local Endpoint observations are
ready. A projection is `Ready` only when its import is bound and its local
encrypted-stream route is ready. Link loss or export revocation marks the
projection `Degraded`/`revoked`; it never falls back to opening local PipeWire.
D091 currency propagates remote Service -> ResourceImport -> projection
AudioService -> referencing AudioBindings. Disruptive Service upgrades drain
AudioBindings and named streams before recycle; non-disruptive updates preserve
the service role and ownerRef chain.

## `AudioBinding` ResourceType

`audio.d2bus.org.AudioBinding` is the provider-neutral per-Guest consumer
binding, initially implemented by `Provider/audio-pipewire`. Its
`metadata.ownerRef` is the Guest, and its required `spec.serviceRef` selects
one same-Zone AudioService. It carries grants, levels, guest users, and the
observed realization for that Guest. It never holds a D097
AuthorityDescriptor, is never the `resourceRef`/`serviceType` of a
ResourceExport, and is never generated as a ResourceImport projection.

This is a clean-break ResourceType name. Every former provider-qualified
Binding spelling, every qualified or unqualified `AudioState` spelling, and
every other former per-consumer identifier are unknown. None is registered,
admitted, decoded, or served as an alias, and no schema or serde rename accepts
one. `State` is reserved for resource `status`. References below to baseline symbols such as
`AudioPolicyState`, `parse_audio_state`, or the legacy `audio-state.json` file
are migration evidence only; none defines a v3 ResourceType or compatibility
alias.

### Envelope example

```yaml
apiVersion: resources.d2bus.org/v3
type: audio.d2bus.org.AudioBinding
metadata:
  name: corp-vm-audio
  zone: dev
  uid: <store-generated>
  generation: 3
  revision: <opaque>
  ownerRef: Guest/corp-vm
  finalizers:
    - audio-pipewire.d2bus.org/sidecar-stopped
  deletionRequestedAt: null
  createdAt: 2026-07-22T00:00:00.000Z
  updatedAt: 2026-07-22T00:01:00.000Z
spec:
  providerRef: Provider/audio-pipewire
  serviceRef: audio.d2bus.org.AudioService/host-audio
  grants:
    mic: "off"
    speaker: "on"
    speakerLevel: 75
    micGain: null
  guestUsers:
    - User/alice
  suspendOnGuestAbsent: true
status:
  observedGeneration: 3
  phase: Ready
  conditions:
    - type: RealizationReady
      status: "True"
      reason: provider-realization-ready
    - type: ConsumerAttached
      status: "True"
      reason: runtime-audio-attached
    - type: GrantsEnforced
      status: "True"
      reason: service-applied
    - type: ServiceReady
      status: "True"
      reason: referenced-audio-service-ready
  lastReconciledAt: 2026-07-22T00:01:01.000Z
  resource:
    channels:
      speaker:
        grant: "on"
        level: 75
        liveEnforced: true
      mic:
        grant: "off"
        gain: null
        liveEnforced: true
        arbitrationState: inactive
    enforcementPosture: HostAndGuest
    lastSetApplied: HostAndGuest
    observedServiceRef: audio.d2bus.org.AudioService/host-audio
    realizationRefs:
      - Process/corp-vm-audio-sidecar
  provider:
    providerRef: Provider/audio-pipewire
    schemaId: audio-pipewire.d2bus.org/status/audio-binding
    schemaVersion: 1.0.0
    observedProviderGeneration: 1
    details:
      pipeWireHostSession: ready
      pipeWireGuestSessions: ready
  outcome:
    code: ok
    exitCode: null
    message: null
    retryable: false
  update:
    state: Current
    reasons: []
    observedGeneration: 1
    targetGeneration: 1
    disruption: None
    preserveState: true
    operationId: null
    lastAssessedAt: null
    owned: { count: 0, refs: [] }
    dependencies: { count: 0, refs: [] }
```

### `AudioBinding.spec` fields

Per D089, `AudioBinding`'s typed desired spec is the ResourceType base spec
(Layer 2): top-level `spec.*`, including `spec.providerRef` where applicable.
Any implementation-variant desired settings use only the canonical Layer 3
`spec.provider = { schemaId, schemaVersion, settings }` envelope, whose
`settings` are manifest-registered/signed, deny-unknown, bounded,
versioned/digested, validated against `spec.providerRef`, and forbidden to
shadow base fields; shared fields are promoted into the base spec.
`Provider/audio-pipewire` implements the exact base spec/status schema
version/fingerprint, accepts the canonical minimal base Spec, and rejects an
unsupported optional base capability only through its signed capability matrix
plus typed provider-neutral `unsupported-capability`. `spec.provider` aligns
with `status.provider`.

| Field | Type | Required | Default | Bounds | Notes |
| --- | --- | --- | --- | --- | --- |
| `providerRef` | ResourceRef | yes | - | same-Zone Provider advertising the AudioBinding base fingerprint | Immutable after creation; this dossier's initial implementation is `Provider/audio-pipewire` |
| `serviceRef` | ResourceRef | yes | - | same-Zone `audio.d2bus.org.AudioService/<name>` | Immutable; owner or projection Service; must be Ready before realization |
| `grants.mic` | enum | yes | - | `"on"` \| `"off"` | Microphone grant |
| `grants.speaker` | enum | yes | - | `"on"` \| `"off"` | Speaker grant |
| `grants.speakerLevel` | uint \| null | no | `null` | `0..=100` | Speaker volume percent; null = system default |
| `grants.micGain` | uint \| null | no | `null` | `0..=100` | Microphone input gain percent; null = system default |
| `guestUsers` | list[ResourceRef] | no | `[]` | ≤16 entries; each `User/<name>` where name matches `[a-z][a-z0-9_-]*` ≤32 chars | Guest User ResourceRefs; corresponding User resources must exist; group membership verified by controller before sidecar start |
| `suspendOnGuestAbsent` | bool | no | `true` | - | When `true` the sidecar Process is not started/is stopped when the owning Guest is not Running |

The initial PipeWire implementation accepts an absent `spec.provider` or the
exact signed envelope
`{schemaId:"audio-pipewire.d2bus.org/spec/audio-binding",
schemaVersion:"1.0.0",settings:{}}`; the v1 `settings` object has no fields.
PipeWire aliases, node selectors, portal settings, frontend parameters, and
other implementation details are rejected as top-level AudioBinding fields.
Provider-wide `captureAlias` remains solely in
`Provider/audio-pipewire.spec.config`.

Audio frontend parameters (virtio device ID, queue sizes) are not `AudioBinding`
spec fields. They are derived at runtime from the `runtime-audio` capability
record advertised by the Guest's Runtime Provider (see "runtime-audio
dependency alias" above).

Schema validation is strict: unknown fields are rejected at resource API
admission time. Level and gain values are validated in `[0,100]`; null
sentinels are preserved and serialized as JSON `null`, never as absent fields.
`providerRef` and `serviceRef` are immutable after creation; a mutation attempt
returns `FieldImmutable`. `serviceRef` must be a syntactically valid qualified
AudioService Ref in the same Zone and may not point to ResourceImport directly.
Each `guestUsers` entry is validated as a syntactically correct
`User/<name>` ResourceRef; referential existence is validated at runtime.

### `AudioBinding.status` fields

Per D088, `AudioBinding.status` is layered: universal `ResourceStatus` fields
(`observedGeneration`, `phase`, `conditions`, timestamps, and `outcome`) remain
at top-level `status.*`, while the typed audio fields below are the
ResourceType-common `status.resource` object for
`audio.d2bus.org.AudioBinding`. Optional `status.provider` carries only
implementation-only observation (`providerRef`, qualified immutable `schemaId`,
semver `schemaVersion`, numeric `observedProviderGeneration`, strict
unknown-field-denied redacted `details` ≤32 KiB registered/signed in the
Provider manifest); shared fields are never duplicated there. The controller
writes all present layers atomically in one status mutation.

For `Provider/audio-pipewire`, the strict provider-status schema is
`audio-pipewire.d2bus.org/status/audio-binding` at `1.0.0`; `details` permits
only bounded `pipeWireHostSession` and `pipeWireGuestSessions` values
(`ready|unavailable|not-applicable`). These implementation observations are
optional and redacted. They are forbidden in `status.resource`, and the
provider envelope may not repeat grants, levels, service refs, child refs,
currency, or universal outcome data.

D091 currency and upgrade: the audio-pipewire controller implements
`assess_update`, `plan_upgrade`, and `execute_upgrade` for the provider-neutral
ResourceTypes it implements and their semantic audio sessions. AudioService
currency propagates to every AudioBinding that references it. A
`ProviderGenerationChanged`,
`ArtifactChanged`, `DependencyChanged`, or `SpecChanged` reason populates
universal `status.update` with
`UpdateAvailable` or `UpgradeRequired`; disruptive changes MUST return
`UpgradeRequired` rather than being applied in place, while non-disruptive
changes reconcile normally. These currency fields are universal/ResourceType
base fields, never `status.provider`. Upgrades recycle only the audio
realization (owned `Process` resources, endpoints, and sessions) with
`disruption` set to `Reload`, `Restart`, or `Recycle`; durable config is
preserved, dependent sessions and attachments are drained and restarted by the
dependency-aware planner, and owned ephemeral session state remains process
memory. No audio samples, clipboard bytes, terminal bytes, notification content
bytes, secrets, paths, session bytes, or handles may appear in `status.update`.

D090 expedited reconcile: Create, UpdateSpec, and Delete requests that set
`waitForReconcile` perform no external effect, finalizer mutation, or status
mutation until Core supplies a typed `CommittedRevisionProof`
`{resourceUid,generation,revision,operationId}`. Abort produces no effect; a
durable commit is never rolled back on later reconcile timeout. The response is
the committed object plus one-pass projected layered status, a disposition
(`Converged`, `Progressing`, `Blocked`, `UpgradeRequired`, or `Failed`), and
`statusPersistence` (`pending` or `committed`); effect idempotency keys derive
from `(UID,generation,revision,operationId)` in the same per-resource
single-flight priority lane.

| Field | Type | Notes |
| --- | --- | --- |
| `phase` | enum | Common framework phase: `Pending\|Ready\|Degraded\|Failed\|Unknown`. `Deleted` exists only as a revision-log event, not a live resource phase. Audio-specific detail is in `conditions` and `outcome.code`. |
| `conditions` | list | See condition types below |
| `resource.channels.speaker.grant` | `"on"\|"off"` | Last observed speaker grant |
| `resource.channels.speaker.level` | uint \| null | Last observed speaker level |
| `resource.channels.speaker.liveEnforced` | bool | True when confirmed by successful call through the referenced AudioService this reconcile |
| `resource.channels.mic.grant` | `"on"\|"off"` | Last observed mic grant |
| `resource.channels.mic.gain` | uint \| null | Last observed mic gain |
| `resource.channels.mic.liveEnforced` | bool | True when confirmed through the referenced AudioService this reconcile |
| `resource.channels.mic.arbitrationState` | enum | `inactive\|queued\|active\|blocked`; `active` is possible for at most one consumer of an owner Service |
| `resource.enforcementPosture` | enum | `HostAndGuest\|HostOnly\|GuestOnly\|None` |
| `resource.lastSetApplied` | enum | `HostAndGuest\|HostOnly\|GuestOnly\|OfflineOnly` |
| `resource.observedServiceRef` | ResourceRef | Last resolved same-Zone AudioService; must equal `spec.serviceRef` |
| `resource.realizationRefs` | list[ResourceRef] | At most 32 same-Zone owned `Process`/`Endpoint` refs; no implementation locator or identity |
| `outcome.code` | string | Closed enum; see error codes |
| `outcome.exitCode` | int \| null | Worker exit code when phase is Failed |
| `outcome.message` | string \| null | Bounded ≤256 chars; redacted: no paths, credentials, or VM-identifying details |
| `outcome.retryable` | bool | Whether the controller will retry |

**`Deleted` phase**: after all finalizers complete, the resource is removed
from the store immediately. A single `phase=Deleted` revision event is emitted
to the revision log before removal. The audit record for the deletion is emitted
post-commit, after the revision event is durable.

### Condition types

| Type | Meaning |
| --- | --- |
| `RealizationReady` | The selected Provider's owned realization resources are Ready |
| `ConsumerAttached` | The Guest runtime reports its selected audio capability attached |
| `GrantsEnforced` | Last `SetGrant`/`SetLevel` service calls completed with `liveEnforced: true` on every requested channel; false while requested microphone capture is queued or blocked |
| `GrantEnforcementFailed` | AudioMediator `SetGrant`/`SetLevel` service returned an error |
| `ServiceReady` | Referenced owner/projection AudioService is Ready and current |
| `ServiceUnavailable` | Referenced AudioService is absent, degraded, revoked, cross-Zone, or stale |
| `MicBlocked` | The selected Provider confirms that microphone capture is blocked |
| `MicQueued` | A requested microphone grant is waiting in the owner Service's bounded fair queue |
| `MicCaptureActive` | The owner Service granted this Binding the single microphone capture slot |
| `MicQueueFull` | The owner Service rejected this Binding's request because the fixed total or authenticated per-Zone bound is full |
| `SpeakerBlocked` | The selected Provider confirms that speaker playback is blocked |
| `ProviderSessionUnavailable` | The selected Provider cannot currently access its local audio session; implementation detail is confined to `status.provider` |
| `GuestAbsent` | Guest is not Running and `suspendOnGuestAbsent: true`; owned realization is intentionally stopped |
| `ConsumerEnforcementReady` | The selected Provider reports all required consumer-side enforcement ready |
| `GuestUserAudioGroupMissing` | One or more `guestUsers` User refs do not have `User.status.groupMembershipVerified: true` for the `audio` group |
| `RuntimeCapabilityUnavailable` | The `runtime-audio` dependency alias returned no audio capability; sidecar not deployed |

### Phase semantics

| Phase | Meaning |
| --- | --- |
| `Pending` | `audio.d2bus.org.AudioBinding` committed; referenced AudioService or owned realization not yet Ready, Guest not yet Running, runtime-audio capability not yet advertised, or a requested microphone grant is fairly queued |
| `Ready` | Referenced AudioService Ready, realization ready, consumer attached, speaker grant enforced, and microphone either off or holding the exclusive capture slot |
| `Degraded` | Referenced AudioService degraded/revoked, realization present but enforcement failed, microphone queue admission is temporarily full, Guest temporarily absent, or required provider session/route unavailable |
| `Failed` | Owned realization failed unrecoverably, or runtime audio capability is permanently absent |
| `Unknown` | Controller cannot currently observe Process or Guest status |

## Host worker process

### Architecture

The host worker is a long-lived `vhost-user-sound` sidecar
(`vhost-device-sound` with the backend selected by the referenced
AudioService). For an owner AudioService it connects to the owner Zone's
compositor PipeWire session using a **pre-opened connected PipeWire FD**
received via the operation-scoped typed attachment transfer routed by
d2b-bus/ProviderSupervisor from the AudioMediator. For a projection
AudioService it connects only to the projection's same-Zone route Endpoint;
audio frames then use the ResourceImport's bounded encrypted named streams.
The projection worker never opens PipeWire and never receives a PipeWire FD.
It exposes a `vhost-user` server as an owned `Endpoint` resource. The backing
Unix socket locator is a controller-private sealed LaunchTicket value. The owning Guest attaches it via the
`Guest.spec.audioExtension` arguments derived from the `runtime-audio`
capability (e.g., `--generic-vhost-user socket=<sealed_path>,virtio_id=25,...`
for Cloud Hypervisor). No `audioFrontend` spec field is involved.

One worker Process per AudioBinding/Guest. The `audio-binding-controller` creates
and owns the Process and its private Endpoint resources after resolving
`spec.serviceRef`. The system Process Provider (`Provider/system-minijail`)
launches, supervises, and reaps the worker and owns all ProcessEffect audit
records. The worker's execution principal is a core Process principal from the
bounded pool - not a controller-created `User` resource.

### Process resource

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: corp-vm-audio-sidecar
  zone: dev
  ownerRef: audio.d2bus.org.AudioBinding/corp-vm-audio
  finalizers: [audio-pipewire.d2bus.org/sidecar-stopped]
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  processClass: worker
  template: vhost-user-sound-worker
  sandbox:
    namespaceClasses: [mount, pid, ipc, uts]
    capabilityClasses: []
    seccompClass: audio-pipewire-worker
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: minimal
  budget:
    cpu:
      limit: "500m"
      request: null
    memory:
      limit: "64Mi"
      request: null
    pids:
      limit: 32
    fds:
      limit: 64
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "30s"
    backoffMultiplierMilli: 2000
    maxRestarts: 5
    resetAfter: "300s"
  readiness:
    initialDelay: "0s"
    timeout: "10s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined
```

### Execution schema notes

- **No `executableRef`, `argv`, `env`** - these are signed component-template
  projections in the LaunchTicket. The Process Provider resolves them from the
  compiled `vhost-user-sound-worker` template. The live Process resource spec
  stored in the Zone store contains no executable path, argument list, or
  environment.
- **No `inherited-fd` endpoint in the Process spec** - for an owner Service,
  the PipeWire FD attachment from the AudioMediator is declared in the
  component descriptor (the private signed template), not the live resource
  spec. For a projection Service, the template instead resolves the local
  projection route Endpoint and no PipeWire FD exists. d2b-bus/
  ProviderSupervisor performs either authorized local attachment without the
  Process Provider knowing audio and without the controller handling FDs.
- **No `mounts` block in the live Process spec** - the worker receives its
  configuration via the sealed component descriptor and the operation-scoped FD
  transfer. `AudioBinding.spec` is the durable desired intent for grants/levels,
  and its AudioService is the backing authority/route; no application state
  file is written. The worker declares no Provider state
  Volume; bounded non-secret observations are stored in `AudioBinding.status`, the
  Provider status subresource where applicable, and the core Operation ledger.
- `domain: system` - the worker runs in the system domain. The execution
  principal is a core Process principal from the bounded pool allocated by the
  Process Provider; it is not a controller-created `User` resource and does not
  appear in the live Process spec. No `userRef` field is set.
- `sandbox.namespaceClasses: [mount, pid, ipc, uts]` - network namespace is
  **not** in this list. An owner worker receives the PipeWire FD as an inherited
  descriptor; a projection worker receives only an authorized same-Zone route
  attachment and the Provider's stream component owns encrypted carriage.
- `sandbox.capabilityClasses: []` - zero host capabilities; load-bearing
  invariant; see security section.
- `sandbox.seccompClass: audio-pipewire-worker` - the Process Provider
  resolves the exact seccomp profile from the compiled Process template.
  Maps to the `w1-audio` seccomp policy in the baseline minijail profile table.
- `sandbox.startRoot: false` - the Process Provider must not elevate to root
  before exec.
- `Endpoint/corp-vm-audio-vhost-user` is the AudioBinding-owned private service
  identity for the vhost-user server. The AudioBinding controller creates the
  Endpoint resource; the Process Provider creates the backing Unix socket
  before exec and seals its locator into the LaunchTicket. The locator never
  appears in resource spec or status.
- `budget` uses the canonical nested `cpu`/`memory`/`pids`/`fds` shape.
  `pids` and `fds` use the `{limit: N}` object form (not a bare scalar).
- `restartPolicy.class: on-failure` - canonical class name.
  `backoffBase`/`backoffMax` are duration strings; `backoffMultiplierMilli` is
  the exponential factor multiplied by 1000; `maxRestarts` is the
  per-launch-cycle ceiling; `resetAfter` resets the counter if the process stays
  Running for this duration.
- `readiness.class: provider-defined` - the `vhost-user-sound-worker` template
  declares a provider-defined readiness mechanism (vhost-user socket ready).
  Fields: `initialDelay`, `timeout`, `failureThreshold`, `successThreshold`.

**ProcessEffect audit**: all process-launch, signal, and exit audit records
for this Process are owned by `Provider/system-minijail`. The
`audio-binding-controller` emits only resource-level `AudioBinding` audit events.

### Guest command-line extension

The `audio-binding-controller` mutates `Guest.spec.audioExtension` to add the
runtime's audio arguments. The exact arguments (socket path, device ID, queue
sizes) are derived from two sealed sources:

- The vhost-user socket path is derived from `AudioBinding.metadata.uid` by the
  Process Provider and embedded in the LaunchTicket; it is never stored in any
  public field.
- Device parameters (`virtio_id`, `queue_sizes`, runtime-specific flags) are
  read from the `runtime-audio` capability record at reconcile time and applied
  as the extension; they are not `AudioBinding.spec` fields.

The mutation is conditional: when the runtime advertises no audio capability,
or when both grants are `"off"`, the controller removes `audioExtension` from
the Guest spec.

## AudioMediator user-session service

### Purpose and identity

The `AudioMediator` is the owner AudioService's same-UID user-session
implementation `Process`; it runs under the compositor user's UID. A
projection AudioService never starts or calls a local AudioMediator. The
mediator's responsibilities are:

1. **Receive a declared pre-opened PipeWire portal FD** from the user
   supervisor/display portal. The AudioMediator does not open the PipeWire
   socket from the ambient runtime environment; it receives the FD as a
   declared pre-opened attachment from the user supervisor, which owns the
   compositor session.
2. **Participate in the operation-scoped typed attachment transfer**:
   d2b-bus/ProviderSupervisor routes the portal FD directly from the
   AudioMediator to the worker LaunchTicket at launch time (see "FD routing
   via ProviderSupervisor" below). The Process Provider (system-minijail) and
   the controller are both audio-agnostic in this path.
3. Resolve a bounded `captureAlias` label to the actual PipeWire node object
   via libpipewire registry introspection at runtime (privately, never
   exported).
4. Expose the owner AudioService's `SetGrant`/`SetLevel` typed ComponentSession
   service over d2b-bus. When a same-Zone AudioBinding or an authorized remote
   projection route calls `SetGrant(channel, value)` or
   `SetLevel(channel, value)`, the AudioMediator applies the change directly via
   libpipewire API (`pw_node_set_param`, `pw_stream_set_control`) on the
   worker's live PipeWire node.

### AudioMediator Process resource

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: audio-pipewire-mediator
  zone: dev
  ownerRef: Provider/audio-pipewire
spec:
  providerRef: Provider/system-systemd
  executionRef: Host/host-system
  domain: user
  userRef: User/compositor-user
  processClass: service
  template: audio-mediator-service
  sandbox:
    namespaceClasses: []
    capabilityClasses: []
    seccompClass: audio-pipewire-mediator
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: provider-defined
  budget:
    cpu:
      limit: "250m"
      request: null
    memory:
      limit: "32Mi"
      request: null
    pids:
      limit: 16
    fds:
      limit: 128
  restartPolicy:
    class: on-failure
    backoffBase: "2s"
    backoffMax: "30s"
    backoffMultiplierMilli: 2000
    maxRestarts: null
    resetAfter: "300s"
  readiness:
    initialDelay: "0s"
    timeout: "10s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined
```

**No `executableRef`, `argv`, `env`** - these are component-template
projections. The live Process resource spec stored in the Zone store contains
only the fields shown above. `budget`, `restartPolicy`, and `readiness` use the
same canonical field names as the worker Process spec. `readiness.class:
provider-defined` - the `audio-mediator-service` template declares its own
readiness mechanism (ComponentSession endpoint accepting connections).

### FD routing via ProviderSupervisor

For an AudioBinding that references an owner Service, the controller declares an
**operation-scoped typed attachment transfer** on the worker Process resource
at creation time. The transfer descriptor names the AudioMediator as the FD
source and the worker as the FD destination, without specifying audio details:

```text
attachmentTransfer:
  source: Process/audio-pipewire-mediator
  sourceHandle: pipewire-portal-fd
  destinationRole: inherited-fd
```

When the Process Provider receives the new worker Process resource, it does
**not** resolve this transfer itself. Instead, d2b-bus/ProviderSupervisor
orchestrates:

1. ProviderSupervisor resolves the AudioMediator's active `pipewire-portal-fd`
   handle from its component-descriptor declaration.
2. Validates the requesting subject (worker Process generation/UID) against the
   AudioMediator's ACL.
3. Delivers the FD via an atomic seqpacket SCM_RIGHTS transfer bound to the
   transfer descriptor.
4. Embeds the received FD into the worker's LaunchTicket as an inherited
   descriptor (CLOEXEC cleared).

The Process Provider (system-minijail) launches the worker with the FD already
present in the LaunchTicket. It does not know the FD is a PipeWire connection.
The controller does not hold or transfer FDs at any point.

This transfer is a component-descriptor declaration, not a Process resource
spec field. No socket path, runtime directory path, or user identifier appears
in the Process resource spec or any API response.

For an AudioBinding that references a projection Service, this transfer is
absent. The worker resolves only the projection's local route Endpoint, and the
AudioService controller binds that Endpoint to the ResourceImport's encrypted
named streams. No FD is forwarded across a Zone.

### `SetGrant` and `SetLevel` service

The AudioMediator exposes a `SetGrant`/`SetLevel` typed ComponentSession service
on the owner Service's `Endpoint/audio-pipewire-authority`. A same-Zone
AudioBinding controller reaches it through its owner `serviceRef`; a consumer
Zone reaches it through the projection Service's encrypted route. No
AudioBinding references this Endpoint directly.

Service interface (conceptual):

```text
SetGrant(consumer: OpaqueConsumerHandle, channel: "mic"|"speaker", value: "on"|"off")
  → Applied | Queued
  | Error(code: GrantEnforcementFailed | MicQueueFull
                | ProviderSessionUnavailable | ...)

SetLevel(consumer: OpaqueConsumerHandle, channel: "mic"|"speaker", valuePercent: u8)
  → Ok | Error(code: GrantEnforcementFailed | ...)
```

The authenticated ComponentSession or import route supplies the consumer Zone;
it is never accepted from request data. For a local caller, the AudioService
maps the opaque handle to the same-Zone AudioBinding. A projection maps its
local Binding to a route-scoped opaque handle before forwarding, so no
ResourceRef crosses a Zone.

The AudioMediator applies the change via:
- `pw_node_set_param` with `SPA_PARAM_Props` to update `mute` or routing on
  the worker's virtual device node;
- `pw_stream_set_control` on any active stream attached to the node;
- WirePlumber session policy enforcement through the node properties already
  set by the component template's initial stream configuration.

No node ID, PipeWire object path, wpctl binary, or any external process is
involved. The service call is synchronous within the AudioMediator; the caller
receives the result before updating `AudioBinding.status`.

No node ID, node path, or PipeWire runtime directory path appears in any
service request, response, d2b-bus message, audit record, or log entry.

### Initial-v3 microphone arbitration

Speaker `SetGrant` calls are applied immediately and all admitted speaker
streams remain multiplexed through the owner Service's mixer. Microphone
capture has one global slot per owner AudioService across the owner Zone and
all importing Zones. The owner admits or queues `mic: "on"` as follows:

1. A request is keyed by the authenticated consumer Zone and its
   route-scoped opaque consumer handle. Duplicate requests are idempotent, and
   at most one pending entry exists per handle.
2. Pending requests are bounded to 16 per Zone and 64 total. A request that
   would exceed either bound returns `MicQueueFull` and creates no entry.
3. Each Zone has an owner-sequenced FIFO subqueue. Non-empty Zones form a
   round-robin ring. On release or lease expiry, the arbiter selects the head
   request from the Zone after the last-served Zone; a Zone that still has
   pending requests moves to the ring tail. Client timestamps and requested
   priority are ignored.
4. The active capture lease is 30 seconds. It may renew while no other Zone is
   waiting. Once another Zone waits, renewal is denied; at the deadline the
   mediator mutes and disconnects the old capture stream, dequeues the next
   Zone, and, if the old consumer still requests `mic: "on"`, atomically places
   it at its Zone FIFO tail. Thus no two microphone streams overlap and a
   continuously requesting holder cannot bypass waiting Zones.
5. `mic: "off"`, Binding deletion, import revocation, ZoneLink loss, session
   cancellation, or ResourceImport lease expiry removes that consumer's
   active/pending entry. Release is idempotent. Queue membership and ordering
   contain opaque handles only and are never persisted, logged, audited, or
   exported; only the bounded aggregate counts defined below may leave the
   arbiter.

`AudioService.status.resource` exposes only the bounded aggregate active,
pending-request, and pending-Zone counts. `AudioBinding.status.resource`
exposes only that Binding's `inactive|queued|active|blocked` state; it never
exposes queue position, another consumer, or a Zone name.

Initial v3 has no concurrent microphone capture. `microphone: multiplexed`,
consent or approval fields, priority overrides, and concurrent-capture verbs
are unknown and rejected by schema/admission. A future version may add
multiplexed capture only after a separate normative spec defines a concrete
consent authorization ResourceType and resource-API verb, including grant,
revocation, expiry, status, and audit semantics. This dossier reserves no
placeholder consent field or verb.

### captureAlias resolution

When `Provider/audio-pipewire.spec.config.captureAlias` is non-null, the
AudioMediator resolves it at grant-change time via libpipewire registry
introspection. It iterates the PipeWire global object list and finds the node
whose `node.nick` or `node.name` matches the alias. The resolution is private
to the AudioMediator process. The resolved node object never leaves the mediator -
it is used in place directly for `pw_node_set_param` routing calls. No node
ID appears in any bus message, resource spec, status, audit record, or OTEL
attribute.

## Guest frontend

### Architecture

The guest frontend is a kernel virtio device driver (`snd_virtio`) and an
in-guest PipeWire stack. These are provisioned by the guest NixOS module
compiled into the Guest's NixOS configuration at Provider activation time.

Activated for any Guest that owns an `AudioBinding` resource, it installs:

- `boot.kernelModules: ["snd_virtio"]` - in-tree since 5.16;
- `services.pipewire.enable = true` with `alsa.enable`, `alsa.support32Bit`,
  and `pulse.enable` (PulseAudio compat layer);
- `security.rtkit.enable = true` - realtime priority for audio threads;
- WirePlumber `monitor.alsa.rules` override: `device.profile = "pro-audio"`
  and `api.alsa.use-acp = false` for the virtio-snd card; this is required
  because the virtio-snd ALSA driver has no ACP entry and WirePlumber defaults
  to `"Off"`, leaving no Sink or Source;
- `services.pulseaudio.enable = lib.mkForce false` - prevents PulseAudio
  collision;
- diagnostic packages: `pipewire`, `wireplumber`, `alsa-utils`.

### Guest `audio` group membership

**Nix/compiler path** (static configuration): when `AudioBinding` resources are
declared in the Nix configuration, the compiler sets `spec.groups: ["audio"]`
on the corresponding guest `User` resources for each name in
`AudioBinding.spec.guestUsers`. system-core verifies group membership
(`User.status.groupMembershipVerified`) and sets `GroupsVerified: True` when
confirmed.

**API-created AudioBinding** (runtime path): the controller checks
`User.status.conditions.GroupsVerified == True` for every name in
`spec.guestUsers` before starting the sidecar. If any User's audio group is
not confirmed, the controller sets `GuestUserAudioGroupMissing` condition and
phase becomes `Degraded`. The operator must update the User resource's
`spec.groups` to include `"audio"` and rebuild the guest to resolve this. The
controller does **not** mutate `User.spec.groups` at runtime.

### GuestAudioAgent and AudioSet service

Guest-side enforcement is performed by the **`GuestAudioAgent`** - a Process
resource running inside the Guest under the guest workload user's UID. It is
part of the audio-pipewire Provider's guest component set.

The `GuestAudioAgent`:
1. Opens a PipeWire connection in the Guest's compositor session (same-UID
   user domain, natural session access - no socket path or ambient ACL needed).
2. Exposes a typed `AudioSet` ComponentSession service over vsock (Guest→Zone
   d2b-bus transport).
3. When the `audio-binding-controller` calls `AudioSet(mic, speaker, speakerLevel,
   micGain)`, the GuestAudioAgent applies changes directly via libpipewire API
   (`pw_node_set_param` with `SPA_PARAM_Props`, `pw_stream_set_control`) on the
   guest virtio-snd PipeWire node.

**No wpctl binary, command path, or external process** is involved on either
the host or the guest side. The baseline `d2b.guestControl.wpctlPath` option is
superseded and removed from v3. The baseline guestd `AudioSet` RPC is superseded
by this typed ComponentSession service.

#### GuestAudioAgent Process resources (one per guestUsers entry)

The controller creates one `GuestAudioAgent` Process per entry in
`AudioBinding.spec.guestUsers`. Each Process:
- Is named using an opaque UID digest derived from `AudioBinding.metadata.uid`
  and `User/<name>.metadata.uid`, e.g. `ag-4a7f2c1b` - never by username.
- Has `ownerRef: audio.d2bus.org.AudioBinding/corp-vm-audio` and is located in the controller's
  component identity index for that `AudioBinding`; never selected by mutable
  label selector.
- Has `userRef` pointing to the corresponding `User/<name>` resource in the
  Zone (the guest workload user's User resource).
- Opens a PipeWire connection in that user's guest session (same-UID).
- Serves the `AudioSet` ComponentSession service for that user's audio context.

The controller calls all active GuestAudioAgent instances for each grant change
and aggregates failures: if all agents succeed, `liveEnforced: true`; if any
fail, `GuestEnforcementFailed` condition is set per-agent; if all fail,
`enforcementPosture: HostOnly`.

Representative Process resource (for one guestUser, `User/alice`):

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: ag-4a7f2c1b
  zone: dev
  ownerRef: audio.d2bus.org.AudioBinding/corp-vm-audio
spec:
  providerRef: Provider/system-systemd
  executionRef: Guest/corp-vm
  domain: user
  userRef: User/alice
  processClass: service
  template: guest-audio-agent
  sandbox:
    namespaceClasses: []
    capabilityClasses: []
    seccompClass: guest-audio-agent
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: provider-defined
  budget:
    cpu:
      limit: "250m"
      request: null
    memory:
      limit: "32Mi"
      request: null
    pids:
      limit: 16
    fds:
      limit: 64
  restartPolicy:
    class: on-failure
    backoffBase: "2s"
    backoffMax: "30s"
    backoffMultiplierMilli: 2000
    maxRestarts: null
    resetAfter: "300s"
  readiness:
    initialDelay: "0s"
    timeout: "10s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined
```

Like all Process resources, this spec contains no `executableRef`, `argv`, or
`env` - these are signed component-template projections in the LaunchTicket.

## Endpoint resources (D092)

`Provider/audio-pipewire` declares standard `Endpoint` base-schema conformance.
Owner/projection AudioService implementation routes and every AudioBinding's
vhost-user and GuestAudioAgent services are owned `Endpoint` resources with
`producerRef`; they are not inline `Process.spec` fields. The AudioService
controller creates Service-owned implementation/route Endpoints; the
AudioBinding controller creates Binding-owned private vhost-user and guest-agent
Endpoints. Consumers use `Endpoint/<name>` references. No raw socket path,
PipeWire node path, CID, port, fd number, credential, level value, or content
byte appears in Endpoint spec, Endpoint status, CLI output, audit, or telemetry.
Resolution occurs only through an authorized EffectPort/LaunchTicket;
unauthorized resolution returns `endpoint-resolve-denied`. Producer restarts
bump `Endpoint.status.endpointGeneration`, causing consumers to receive a
`dependency-changed` trigger.

Representative owned Endpoint resources:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: corp-vm-audio-vhost-user
  zone: dev
  ownerRef: audio.d2bus.org.AudioBinding/corp-vm-audio
spec:
  providerRef: Provider/audio-pipewire
  producerRef: Process/corp-vm-audio-sidecar
  endpointClass: data
  transport: unix
  purpose: audio-pipewire.d2bus.org/vhost-user-sound
  serviceFingerprint: audio-pipewire.d2bus.org/vhost-user-sound.v3
  locality: host-local
  visibility: zone
  attachmentPolicy: launch-ticket
  consumerPolicy:
    allowedOperations: [resolve]
  lifecyclePolicy: recycle-with-producer
status:
  resource:
    readiness: Ready
    observedProducerGeneration: 1
    observedResourceGeneration: 1
    endpointGeneration: 1
    connectionAvailability: available
    leaseAvailability: lease-required
  update:
    state: Current
    reasons: []
    observedGeneration: 1
    targetGeneration: 1
    disruption: None
    preserveState: true
    operationId: null
    lastAssessedAt: null
    owned: { count: 0, refs: [] }
    dependencies: { count: 0, refs: [] }
```

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: audio-pipewire-authority
  zone: host
  ownerRef: audio.d2bus.org.AudioService/host-audio
spec:
  providerRef: Provider/audio-pipewire
  producerRef: Process/audio-pipewire-mediator
  endpointClass: service
  transport: unix
  purpose: audio-pipewire.d2bus.org/audio-control
  serviceFingerprint: audio-pipewire.d2bus.org/AudioMediator.v3
  locality: host-local
  visibility: zone
  attachmentPolicy: component-session
  consumerPolicy:
    allowedSubjects: [User/alice]
    allowedOperations: [resolve]
  lifecyclePolicy: recycle-with-producer
status:
  resource:
    readiness: Ready
    observedProducerGeneration: 1
    observedResourceGeneration: 1
    endpointGeneration: 1
    connectionAvailability: available
    leaseAvailability: lease-required
  update:
    state: Current
    reasons: []
    observedGeneration: 1
    targetGeneration: 1
    disruption: None
    preserveState: true
    operationId: null
    lastAssessedAt: null
    owned: { count: 0, refs: [] }
    dependencies: { count: 0, refs: [] }
```

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: host-audio-import-route
  zone: work
  ownerRef: audio.d2bus.org.AudioService/host-audio-projection
spec:
  providerRef: Provider/audio-pipewire
  producerRef: Process/audio-pipewire-controller
  endpointClass: service
  transport: unix
  purpose: audio-pipewire.d2bus.org/import-route
  serviceFingerprint: audio-pipewire.d2bus.org/AudioServiceRoute.v3
  locality: zone-local
  visibility: zone
  attachmentPolicy: component-session
  consumerPolicy:
    allowedOperations: [resolve]
  lifecyclePolicy: recycle-with-producer
status:
  resource:
    readiness: Ready
    observedProducerGeneration: 1
    observedResourceGeneration: 1
    endpointGeneration: 1
    connectionAvailability: available
    leaseAvailability: lease-required
  update:
    state: Current
    reasons: []
    observedGeneration: 1
    targetGeneration: 1
    disruption: None
    preserveState: true
    operationId: null
    lastAssessedAt: null
    owned: { count: 0, refs: [] }
    dependencies: { count: 0, refs: [] }
```

This projection-Service-owned local Endpoint is only the front door to the
import adapter; it is an ordinary Endpoint child, never an import projection.
Its producer binds a ComponentSession named stream internally; it contains no
remote Ref or locator and never opens PipeWire.

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: ag-4a7f2c1b-audio-set
  zone: dev
  ownerRef: audio.d2bus.org.AudioBinding/corp-vm-audio
spec:
  providerRef: Provider/audio-pipewire
  producerRef: Process/ag-4a7f2c1b
  endpointClass: service
  transport: vsock
  purpose: audio-pipewire.d2bus.org/guest-audio-set
  serviceFingerprint: audio-pipewire.d2bus.org/AudioSet.v3
  locality: guest-local
  visibility: zone
  attachmentPolicy: component-session
  consumerPolicy:
    allowedOperations: [resolve]
  lifecyclePolicy: recycle-with-producer
status:
  resource:
    readiness: Ready
    observedProducerGeneration: 1
    observedResourceGeneration: 1
    endpointGeneration: 1
    connectionAvailability: available
    leaseAvailability: lease-required
  update:
    state: Current
    reasons: []
    observedGeneration: 1
    targetGeneration: 1
    disruption: None
    preserveState: true
    operationId: null
    lastAssessedAt: null
    owned: { count: 0, refs: [] }
    dependencies: { count: 0, refs: [] }
```

## Retained opaque handles

- pidfds: kernel Process supervision handles; they are authority-bearing and not
  durable resource identities.
- Per-connection/session handles: PipeWire core/node handles and
  ComponentSession IDs are high-churn per session.
- Named streams: audio control streams, when present, carry operation payloads
  and do not identify a stable endpoint.
- `OwnedTransport`: authenticated transport ownership remains an in-memory
  ComponentSession capability.
- fd indexes: the PipeWire portal FD and inherited vhost-user descriptors are
  LaunchTicket-local slots and stay opaque under D092.

### Enforcement sequence

For Guests with an active audio capability (advertised via `runtime-audio`),
the enforcement sequence for a grant/level change is:

1. Controller resolves the AudioBinding's same-Zone `serviceRef` and requires
   `ServiceReady`.
2. For an owner Service, it calls the Service's AudioMediator
   `SetGrant`/`SetLevel` Endpoint via local d2b-bus. For a projection Service,
   it calls the local projection route, which carries the operation over the
   ResourceImport's bounded encrypted named stream to the remote owner
   Service. Only the remote owner AudioMediator applies libpipewire changes;
   the projection never opens PipeWire. The call returns `Ok` or a typed error.
   (`AudioBinding.spec` is durable per-Guest intent; no state-file write is
   required.)
3. Controller calls `AudioSet` service on every active `GuestAudioAgent`
   Process (one per `guestUsers` entry, identified by ownerRef component
   identity index) in parallel, via d2b-bus, vsock transport. Collects all
   results.
4. Service and guest results are aggregated; `AudioBinding.status.channels`,
   `enforcementPosture`, and `lastSetApplied` are updated in a single
   `UpdateStatus` batch committed post-reconcile.
5. Audit event `audio-binding.grant-changed` is emitted after the status commit
   is durable (post-commit audit).
6. If the Service call fails: `GrantEnforcementFailed`
   condition is set; phase becomes `Degraded`.
7. If any `GuestAudioAgent.AudioSet` call fails: `GuestEnforcementFailed`
   condition is set for that agent (keyed by opaque digest); if all fail,
   `enforcementPosture` is set to `HostOnly`; phase becomes `Degraded`.

## PipeWire stream mediation

### No ambient socket exposure

The vhost-user-sound worker **never** opens, connects to, or receives a
PipeWire socket path. No socket path, runtime directory path, or
`PIPEWIRE_RUNTIME_DIR` value appears in:

- `AudioBinding.spec` or `AudioBinding.status`;
- `AudioService.spec` or `AudioService.status`;
- any resource API response;
- any broker configuration or operation;
- any OTEL attribute or audit record;
- any log message or error message.

PipeWire session access is exclusively through the pre-opened connected FD
declared in the component descriptor and passed at launch time by the
AudioMediator (see AudioMediator section). No `SetSocketAcl`, `ChownSocket`,
or any broker socket-ACL operation is issued by the audio controller or by the
Process Provider on behalf of the audio worker.

### WirePlumber stream rules (host Nix config)

The following configuration is compiled into the host NixOS system by the
audio-pipewire Provider Nix module. It is not a resource spec field.

**`services.pipewire.extraConfig.client."90-d2b"` (`client.conf.d/`):**

The component template for `vhost-user-sound-worker` sets initial stream
properties (including `d2b.mic` and `d2b.speaker`) at connection time. The
WirePlumber stream rules in `client.conf.d/90-d2b` match on these initial
properties and apply `target.object = "-1"`, `node.dont-reconnect = true`,
`node.dont-fallback = true`, and `node.linger = true` when a direction is
`"off"` at stream creation. Live grant changes after connection are applied
by the AudioMediator via libpipewire API calls (not stream rule re-evaluation).

When `grants.mic == "on"` and `captureAlias` is non-null, the AudioMediator
resolves the alias privately via libpipewire registry and routes the capture
stream via `pw_node_set_param`. The alias label remains only in
`Provider/audio-pipewire.spec.config`; it never enters stream rule text, node
property values, AudioService/AudioBinding base spec/status, audit, or
telemetry.

**Placement invariant**: stream rules belong in `client.conf.d/` (PipeWire
client configuration), not in `wireplumber.conf.d/`. WirePlumber's
`stream.rules` section governs state restoration only and does not update
live node properties at creation time. Placing the rule in the wrong file
causes WirePlumber to incorrectly match ALSA hardware devices and removes host
audio sinks. This invariant is load-bearing; see baseline `host.nix` comments
for the full failure analysis.

**`services.pipewire.wireplumber.extraConfig."91-d2b-virtio-snd"` (guest Nix config):**

In-guest WirePlumber forces `pro-audio` profile and disables ACP for the
virtio-snd ALSA card. Guest module concern; not a host rule.

### Stream rule update on grant change

When `AudioBinding.spec.grants` changes, the controller:
1. Calls the referenced AudioService for the changed channels. An owner
   Service dispatches to its local AudioMediator; a projection Service routes
   the request to the remote owner over its encrypted named stream. Only the
   owner AudioMediator applies libpipewire changes. No Process restart or
   UpdateSpec is needed for grant changes. `AudioBinding.spec` is durable
   per-Guest intent; no state file is written. Speaker changes return
   `Applied`; `mic: "on"` returns `Applied`, `Queued`, or `MicQueueFull` under
   the exclusive arbitration contract, and `mic: "off"` idempotently releases
   any active or pending request.
2. Calls `GuestAudioAgent.AudioSet` service (vsock transport) for the guest side.
   For microphone, `Queued` or `MicQueueFull` keeps the guest input muted;
   only `Applied` permits the guest-side `mic: "on"` call.
3. Updates `AudioBinding.status` in a single post-reconcile commit.

## ProviderStateSet

A **ProviderStateSet** is the set of all `Volume` resources in a Zone whose
`metadata.ownerRef` resolves to `Provider/audio-pipewire`. It is a query-time
grouping of ordinary Volume resources, not a ResourceType or stored artifact:

```text
ProviderStateSet(zone, "audio-pipewire") =
  { v : Volume | v.metadata.zone == zone
              && v.metadata.ownerRef == "Provider/audio-pipewire" }
```

Under D087, `Provider/audio-pipewire` declares **no Provider state Volume**. Its
ProviderStateSet is therefore empty:

```text
ProviderStateSet(zone, "audio-pipewire") = {}
```

The audio components fail the storage-need test for a durable Provider state
Volume: their operational state is bounded, non-secret, and derivable from
`AudioService`/`AudioBinding` spec and status, component `Process.status`, the
core Operation ledger, ResourceImport lease state, and external
PipeWire/guest observation after restart. The two provider-neutral qualified
ResourceTypes are the audio-domain resource model implemented by this
Provider; neither is a Provider state Volume.

No component declares a state namespace, state-layout `User/<name>` principal,
identity marker, migration worker, or Provider state mount. The
`audio-binding-controller` reconcile loop does not create, update, or delete
Volumes of any kind. The mediator's PipeWire portal FD, worker LaunchTicket
configuration, and GuestAudioAgent interactions are runtime operational
carriage, not Provider state Volumes.

Status is observation only. It is revisioned, optimistic-status-writer
controlled, RBAC-readable, redacted, bounded to the global/provider-detail
limits, written only on material change, and re-verified against external
PipeWire and guest-agent reality after restart. It never contains secrets,
tokens, socket paths, argv/env, paths, PIDs, unit names, private PipeWire object
dumps, terminal/clipboard/notification/audio bytes, authority-conferring
handles, large blobs, or unbounded collections; oversize status is rejected
with `status-oversize`.

There is no bootstrap state-Volume mechanism; the previous bootstrap exception
(D086, superseded by D087) does not apply.

**Baseline migration note**: the one-time v1/v2 legacy `audio-state.json` migration (if
a legacy file is found on the host during Provider installation) reads it with
`parse_audio_state`, requires the explicitly configured same-Zone owner
AudioService, writes the parsed grants plus that `serviceRef` to AudioBinding via
the resource API, and removes the legacy file. Missing or ambiguous Service
selection fails closed. This runs before any component Process enters Ready; it
is not a Volume lifecycle contract and uses no ProviderStateSet Volume.

## Identities, principals, and OS accounts

### Process principals

| Principal kind | Source | Used for |
| --- | --- | --- |
| Worker core Process principal | Bounded pool allocated by `Provider/system-minijail` per Provider | vhost-user-sound worker Process; sealed in LaunchTicket; not a controller-created `User` resource |
| AudioMediator compositor user | `User/<compositor-name>` pre-existing host user | AudioMediator user-session Process; observed from system-core |
| GuestAudioAgent guest user | `User/<name>` per declared guestUser ResourceRef | GuestAudioAgent user-session Process inside the Guest; observed from system-core |

**Worker principal model**: the audio controller creates no `User` resources
for worker execution identity. The dedicated worker principal is a core Process
principal from the bounded pool allocated by `Provider/system-minijail` for the
`audio-pipewire` Provider. This principal is sealed in the LaunchTicket and
never appears in the live Process resource spec or any public spec field.

**Guest `User` refs are system-core observed**: the `guestUsers` list contains
`User/<name>` ResourceRefs pointing to guest workload users that already exist
as system-core resources. The audio controller never creates, updates, or
deletes guest `User` resources.

**AudioMediator compositor user**: the AudioMediator's `userRef` points to the
pre-existing `User/<compositor-name>` resource managed by system-core. The
audio controller observes its status but does not own it.

### Guest `audio` group membership

**Nix/compiler path** (static configuration): when `AudioBinding` resources are
declared in the Nix configuration, the compiler sets `spec.groups: ["audio"]`
on the corresponding guest `User` resources for each ResourceRef in
`AudioBinding.spec.guestUsers`. system-core verifies group membership
(`User.status.groupMembershipVerified`) and sets `GroupsVerified: True` when
confirmed.

**API-created AudioBinding** (runtime path): the controller checks
`User.status.conditions.GroupsVerified == True` for every ref in
`spec.guestUsers` before starting the sidecar. If any User's audio group is
not confirmed, the controller sets `GuestUserAudioGroupMissing` condition and
phase becomes `Degraded`. The operator must update the User resource's
`spec.groups` to include `"audio"` and rebuild the guest to resolve this. The
controller does **not** mutate `User.spec.groups` at runtime.

The worker core Process principal:
- has no login shell, no home directory, and no supplementary groups;
- is never a member of the `kvm`, `render`, `video`, or `audio` host groups;
- has no ACL entry on any PipeWire socket, runtime directory, or compositor
  socket path; PipeWire access is exclusively through the FD from the
  ProviderSupervisor-routed attachment transfer.

## RBAC and authorization

### Roles

| Role name | Verbs | ResourceTypes | Notes |
| --- | --- | --- | --- |
| `audio-pipewire:view-status` | `get`, `list`, `watch` | `audio.d2bus.org.AudioService`, `audio.d2bus.org.AudioBinding` selected by `Provider/audio-pipewire` | Read-only Service and Binding status viewer |
| `audio-pipewire:manage-grants` | `get`, `list`, `watch`, `updateSpec` | `audio.d2bus.org.AudioBinding` selected by `Provider/audio-pipewire` | May update `spec.grants` and `spec.guestUsers` only; intended CLI role |
| `audio-pipewire:admin` | `get`, `list`, `watch`, `create`, `updateSpec`, `delete` | owner `audio.d2bus.org.AudioService`; `audio.d2bus.org.AudioBinding`; both selected by `Provider/audio-pipewire` | Full owner-Service and Binding lifecycle; cannot create/mutate projection Services |
| `audio-pipewire:controller` | `get`, `list`, `watch`, `updateStatus`, `updateFinalizers`; child lifecycle verbs | `audio.d2bus.org.AudioService`, `audio.d2bus.org.AudioBinding` with `spec.providerRef: Provider/audio-pipewire`, plus owned `Process` and `Endpoint` children | Provider controller identity only; cannot create/delete projection AudioService or mutate a resource selected for another Provider |
| `system-core:resource-import-controller` | `get`, `create`, `delete` | projection `audio.d2bus.org.AudioService` only | Creates/deletes the exact local projection named by ResourceImport and preserves its selected Provider; cannot create AudioBinding or owner Service, update grants, or claim backing authority |

### Spec field authorization

| Field | Required role |
| --- | --- |
| `AudioService.serviceRole=owner`, `implementationEndpointRefs`, `operations`, `authority` | `audio-pipewire:admin`; immutable role/authority identity after creation |
| `AudioService.serviceRole=projection` and projection fields | core ResourceImport controller only; operator and Provider controller mutation denied |
| `AudioBinding.serviceRef` | `audio-pipewire:admin`; immutable after creation and same-Zone only |
| `grants.*` | `audio-pipewire:manage-grants` or higher |
| `guestUsers` | `audio-pipewire:admin` |
| `suspendOnGuestAbsent` | `audio-pipewire:admin` |
| `providerRef` | immutable after creation; must name a Provider advertising the neutral type's signed base fingerprint; resource API admission rejects any mutation |

### Broker operations requested

The `audio-binding-controller` requests **zero** broker operations. It
communicates exclusively through the resource API (create/update/delete
owned `Process`/private `Endpoint` resources and read their status). The
AudioService handler likewise uses resource/ComponentSession APIs for its
local Endpoints and authority/route observations. The Process Provider handles all
broker-mediated process lifecycle effects. No `SetSocketAcl`, `ChownSocket`,
`SpawnRunner`, `OpenPidfd`, `StoreViewFarm`, `SwtpmDir`, or
`UsbipBindFirewallRule` operation is requested by the audio-pipewire Provider.

## Security invariants

The following invariants are load-bearing. Any change requires corresponding
test coverage and is subject to panel review.

1. **Zero host capabilities.** The vhost-user-sound worker Process
   `spec.sandbox.capabilityClasses` is always `[]` and `startRoot` is always
   `false`. The worker runs in the system domain (`domain: system`) under a
   core Process principal sealed in the LaunchTicket; no `userRef` is set in
   the live Process resource spec and no controller-created `User` resource is
   needed. The Process Provider rejects any LaunchTicket derived from this
   template that carries a non-empty capability class or `startRoot: true`.
   Tests: `minijail_contract.rs::audio_rendered_capabilities_empty`,
   `audio_source_startRoot_false`, `audio_worker_domain_system_no_userref`,
   `audio_worker_no_controller_created_user`.

2. **No ambient PipeWire socket exposure.** No PipeWire socket path, runtime
   directory path, or `PIPEWIRE_RUNTIME_DIR` value appears in
   `AudioService.spec`, `AudioService.status`, `AudioBinding.spec`,
   `AudioBinding.status`, any Process/Endpoint resource spec, any API response,
   OTEL attribute, audit record, log message, or broker configuration. Tests:
   Service, Binding, Process, and Endpoint schema round-trip tests assert the
   absence of any socket-path-shaped string in every serialized form.

3. **No SetSocketAcl or ambient ACL grant.** No broker `SetSocketAcl` or
   equivalent socket-ACL operation is issued. The AudioMediator's PipeWire
   access is through a declared pre-opened portal FD from the user supervisor,
   not an ambient socket ACL. Tests: broker-op policy test asserts the
   audio-pipewire Provider's allowed broker op set is empty.

4. **No executableRef/argv/env in live Process spec.** The live Process
   resource spec (stored in the Zone store) contains no `executableRef`, `argv`,
   or `env`. These are signed component-template projections. Tests:
   `minijail_contract.rs::audio_process_spec_no_argv_env_executableref`.

5. **Per-Guest binary copy argv[0] enforcement in template.** The component
   template for `vhost-user-sound-worker` enforces the per-Guest binary copy
   path as argv[0]. The LaunchTicket verifier enforces the path shape; no Nix
   store path, symlink, or cross-guest copy is accepted. Tests: `argv.rs`
   rejection matrix (Nix store path, current-system symlink, wrong-guest copy,
   empty binary, empty VM name).

6. **No direct process lifecycle.** The AudioBinding controller creates and
   updates owned Process and private Endpoint resource specs only. It never calls `SpawnRunner`,
   `OpenPidfd`, issues a LaunchTicket, or adopts a pidfd. It creates no
   `Volume` or `User` resources. Tests: controller conformance test asserts the
   audio-binding-controller's allowed resource API verb set.

7. **No per-workload systemd unit.** There is no `d2b-<vm>-snd.service`
   systemd unit, no `systemctl start` call in the controller, and no direct
   `spawn()`. Tests: workspace policy gate `no-systemd-unit-for-audio.sh`.

8. **AudioMediator does not export PipeWire node IDs.** The AudioMediator's
   internal node registry never appears in any resource spec, status, d2b-bus
   message, audit record, or OTEL attribute. Tests:
   `mediator.rs::node_id_not_in_any_bus_message_or_spec`.

9. **No wpctl EphemeralProcess.** No `EphemeralProcess` with `purpose`
   containing `wpctl` or any external command binary is created by the audio
   Provider. Grant enforcement is exclusively through the AudioMediator's
   `SetGrant`/`SetLevel` service using libpipewire. Tests:
   `audio_binding_controller.rs::no_ephemeral_process_created`.

10. **No path leakage in audit or OTEL.** The audio controller's audit emitter
    never includes socket paths, PipeWire paths, or compositor runtime directory
    paths. Tests: `audio_telemetry.rs` redaction conformance.

11. **Controller creates no Volume or User resources.** The
    `audio-binding-controller` is not permitted to issue any verb against `User`
    or `Volume` ResourceTypes; both are absent from `allowedResourceVerbs`.
    Under D087, `Provider/audio-pipewire` declares no Provider state Volume and
    its ProviderStateSet is empty. The semantic controller is not a Volume owner,
    does not export `Volume` as a ResourceType, and does not create prerequisite
    Volumes.
    Tests: controller conformance test verifies the absence of User and Volume
    verbs; ProviderDeployment integration validates that no state Volume or
    state mount is created and that bounded operational state is status-first.

12. **Service/Binding separation and canonical core-owned projections.** Only
    `audio.d2bus.org.AudioService` may carry the audio D097 authority or be
    named by ResourceExport `resourceRef`/`serviceType` and ResourceImport
    `expectedServiceType`. Export and Import projection-schema/factory
    fingerprints must exactly match the signed factory. Every Endpoint remains
    Service-owned and no Export field names one.
    AudioBinding always has `ownerRef: Guest/<name>` plus a same-Zone
    `serviceRef`; it is never exported or generated by the import controller.
    Core creates/deletes only projection AudioService resources, and the
    Provider rejects operator-created projections. Tests:
    `service_binding_separation`, `core_projection_only_audio_service`, and
    `audio_binding_never_exportable`, plus
    `canonical_export_import_fields`.

13. **Projection ownerRef chain and no local PipeWire open.** Every projection
    AudioService has `ownerRef: ResourceImport/<name>`, no AuthorityDescriptor,
    and only same-Zone route Endpoint refs. Its implementation can bind only
    the import's encrypted named streams and is denied the AudioMediator portal
    attachment/PipeWire-open capability. Tests:
    `projection_ownerref_chain`, `projection_forbids_authority`, and
    `projection_never_opens_pipewire`.

14. **Provider-neutral type identity and field isolation.** Audio resources use
    only `audio.d2bus.org.AudioService` and
    `audio.d2bus.org.AudioBinding`. Provider-qualified Service/Binding names and
    every AudioState name are unknown with no API/schema/serde/Nix alias.
    `Provider/audio-pipewire` reconciles only resources whose immutable
    `spec.providerRef` selects it. PipeWire-specific desired fields occur only
    in its strict `spec.provider` or Provider config, and PipeWire-specific
    observations occur only in strict `status.provider`; neutral base fields
    reject them. Tests: `provider_neutral_type_registration`,
    `resource_type_name_clean_break`, `foreign_provider_ignored`, and
    `provider_field_isolation`.

15. **Exclusive microphone capture and defined fair queue.** Initial v3 permits
    exactly one active microphone capture stream per owner AudioService across
    local and imported consumers. The owner-authenticated Zone is the fairness
    principal; requests use owner-sequenced per-Zone FIFO subqueues and
    round-robin Zone selection, bounded to 16 pending requests per Zone and 64
    total, with one entry per opaque consumer handle. A contended 30-second
    lease cannot renew; the old stream is muted/disconnected and re-enters its
    Zone's FIFO tail if intent remains on before the next Zone activates.
    Speaker streams remain multiplexed/mixed.
    `microphone: multiplexed`, consent/approval fields, priority overrides, and
    concurrent-capture verbs fail closed. No concurrent capture may be added
    until a future normative spec defines its concrete consent authorization
    ResourceType and resource-API verb. Tests:
    `mic_exclusive_across_zones`, `mic_zone_round_robin_fifo`,
    `mic_queue_bounds`, `mic_contended_lease_handoff`,
    `mic_no_overlap`, and `mic_consent_surface_rejected`.

## Lifecycle, restart, and adoption

### Install sequence

1. Operator creates `Provider/audio-pipewire` resource with `spec.artifactId`.
2. Core ProviderDeployment creates the `audio-binding-controller` Process (system
   domain) and the `audio-mediator` Process (user domain) as static components;
   no Provider state Volume or state mount is created.
3. The controller Process registers the provider-neutral AudioService and
   AudioBinding watch plans constrained to
   `spec.providerRef: Provider/audio-pipewire`. An owner Zone creates its one
   real owner AudioService; consumer Zones receive projection AudioServices
   only from the core ResourceImport controller.
4. `audio.d2bus.org.AudioBinding` resources created by Nix or the API
   resolve their required same-Zone AudioService and become Ready through the
   reconcile loop.

### Service lifecycle and import projection

1. An operator/Nix creates the owner AudioService with its D097 descriptor and
   local implementation Endpoint refs. The AudioService controller claims the
   authority index entry, verifies ownerProof, and reconciles only local
   AudioMediator/PipeWire semantics.
2. A ResourceExport in that owner Zone references only the owner AudioService
   and carries the canonical Service type plus signed projection-schema/factory
   fingerprints. Its local authority Endpoint remains owned by the Service and
   is never an Export field. AudioBinding is not exported.
3. In a consumer Zone, a ResourceImport matches that exported AudioService.
   The **core import controller** creates exactly one projection AudioService
   named by `projectionName`, with
   `ownerRef: ResourceImport/<name>`. It does not create AudioBinding, Process,
   Endpoint, worker, GuestAudioAgent, or PipeWire state.
4. The AudioService controller observes the ResourceImport and reconciles the
   projection's local route Endpoint and encrypted named-stream binding. It
   never opens PipeWire in the consumer Zone.
5. On import deletion/revocation, core first drives referencing AudioBindings
   degraded, cancels every active or queued microphone request from that
   consumer Zone, and waits for child cleanup; it then releases the remote
   lease, deletes only the projection AudioService, and clears the import
   finalizer.

### Per-Guest enable sequence

1. Operator (or Nix compilation) creates
   `audio.d2bus.org.AudioBinding/corp-vm-audio` with
   `ownerRef: Guest/corp-vm` and a same-Zone `spec.serviceRef`.
2. `audio-binding-controller` resolves that AudioService and requires
   `ServiceReady`. An absent/degraded/revoked Service sets
   `ServiceUnavailable`; no worker opens PipeWire as fallback.
3. `audio-binding-controller` queries the `runtime-audio` dependency alias for
   the Guest's runtime capability. If no audio capability is advertised, sets
   `RuntimeCapabilityUnavailable`; Pending.
4. For an owner Service, controller checks its AudioMediator Endpoint; for a
   projection Service, it checks the local import-route Endpoint. The
   projection path has no PipeWire portal FD.
5. Controller checks each `spec.guestUsers` User ref status for
   `GroupsVerified: True` (audio group membership). If any fails, sets
   `GuestUserAudioGroupMissing`; Degraded.
6. Determine sidecar desired state: if both grants are `"off"` or no runtime
   capability → desired: stopped. Else → desired: running.
7. Controller creates `Process/corp-vm-audio-sidecar`, its private vhost-user
   Endpoint, each GuestAudioAgent Process, and each private guest-agent
   Endpoint. For an owner Service the template includes the operation-scoped
   AudioMediator attachment; for a projection it includes only the same-Zone
   route attachment.
8. On an owner path, d2b-bus/ProviderSupervisor resolves the AudioMediator
   portal FD and routes it to the worker's LaunchTicket. On a projection path,
   no PipeWire FD exists or is transferred.
9. System Process Provider (system-minijail) launches the worker.
   The vhost-user endpoint becomes ready; Process Provider sets
   `Process.status.phase = Ready`.
10. Controller watches `Process.status`; on `Ready`, sets
    `RealizationReady: True`.
11. Controller mutates `Guest.spec.audioExtension` with the runtime-capability-
    derived arguments.
12. On next Guest start, the runtime attaches the selected audio capability;
    `ConsumerAttached` becomes `True`.
13. Controller applies speaker intent immediately and submits microphone
    intent to the owner Service. `mic: "off"` is inactive; an admitted
    `mic: "on"` becomes `MicCaptureActive`; a fair-queue admission becomes
    `MicQueued`/`Pending`; a saturated queue becomes
    `MicQueueFull`/`Degraded`.
14. `AudioBinding.status.phase` transitions to `Ready` only when speaker intent
    is enforced and microphone intent is either off or owns the exclusive slot.

### Restart and adoption

A Zone runtime restart is a continuation event (ADR 0034). The audio
controller does not hold pidfds; it observes Process/Endpoint status through
the resource API. The Process Provider re-adopts worker identity. The
AudioService handler revalidates the D097 ownerProof or the projection's import
lease/generation without opening a second backing; the AudioBinding handler then
reconverges status from its serviceRef and children.

The owner mediator fails closed across restart: every reconstructed capture
path starts muted, no prior active slot is assumed, and it never adopts two
active captures. Local and projection controllers resubmit committed
`mic: "on"` intent; the owner assigns fresh owner-observed enqueue sequence
numbers and resumes the same bounded per-Zone round-robin/FIFO policy. Queue
state is not a durable authority record, and no requester bypasses the queue
because of restart.

If the worker exited between restarts (Process.status.phase Failed or
Unknown), the controller detects this on its first reconcile post-restart
and sets `AudioBinding.status.phase = Degraded`. The audio controller does
not issue any restart signal.

`suspendOnGuestAbsent: true`: when the Guest transitions out of `Running`,
the controller issues a `Process UpdateSpec` setting the desired phase to
`Stopped`. The system Process Provider performs the graceful stop. The
controller sets `GuestAbsent` condition; phase becomes `Degraded`. When
the Guest becomes `Running` again, the full enable sequence from step 6
repeats.

### Deletion sequence

1. `deletionRequestedAt` is set on `AudioBinding`.
2. `audio-binding-controller` finalizer handler:
   a. Idempotently releases any active or pending microphone request through
      the referenced AudioService.
   b. Issues `Process.spec` mutation setting desired phase to `Stopped`.
   c. Waits for `Process.status.phase` to reach a terminal phase (system
      Process Provider performs graceful stop: SIGTERM → 10s → SIGKILL).
   d. Issues owned private `Endpoint` and `Process` Deletes via resource API.
   e. Removes `--generic-vhost-user` (or equivalent) from `Guest.spec.audioExtension`.
   f. Removes `audio-pipewire.d2bus.org/sidecar-stopped` finalizer.
3. After all finalizers are removed: resource is deleted from the store;
   a single `phase=Deleted` revision event is committed to the revision log.
4. Audit event `audio-binding.deleted` is emitted **post-commit** after the
   revision event is durable. No audit event is emitted inline with the
   finalizer steps.

## Errors

### Outcome code enum

| Code | Phase | Retryable | Meaning |
| --- | --- | --- | --- |
| `ok` | `Ready` | false | All conditions satisfied |
| `ServiceNotReady` | `Pending` | true | Referenced owner/projection AudioService not yet Ready |
| `ServiceUnavailable` | `Degraded` | true | Referenced AudioService absent, degraded, revoked, stale, or cross-Zone |
| `AuthorityConflict` | `Failed` | false | Owner Service D097 authority claim conflicts with an incumbent |
| `EndpointNotReady` | `Pending` | true | Required same-Zone Service or Binding-owned private Endpoint not Ready |
| `ImportNotBound` | `Pending` | true | Projection Service's owning ResourceImport has no current lease |
| `ImportRevoked` | `Degraded` | true | Projection Service's import/export lease was revoked |
| `ProjectionLocalBackingDenied` | `Failed` | false | Projection attempted to open a forbidden local backing; PipeWire detail, if any, is provider status only |
| `RealizationNotReady` | `Pending` | true | Selected Provider's owned realization is not yet Ready |
| `GuestAbsent` | `Degraded` | true | Guest not Running; owned realization suspended |
| `GrantEnforcementFailed` | `Degraded` | true | Selected AudioService implementation returned an enforcement error |
| `MicCaptureQueued` | `Pending` | true | Microphone intent is admitted to the owner Service's bounded fair queue |
| `MicQueueFull` | `Degraded` | true | The fixed total or authenticated per-Zone pending-request bound is full; no queue entry was created |
| `ProviderSessionUnavailable` | `Degraded` | true | Selected Provider cannot access its local audio session; implementation detail is in `status.provider`; retries |
| `ConsumerEnforcementUnavailable` | `Degraded` | true | Selected Provider's consumer-side enforcement is not Ready; retries |
| `GuestEnforcementFailed` | `Degraded` | false | Selected Provider's consumer-side grant enforcement returned an error |
| `GuestUserAudioGroupMissing` | `Degraded` | false | One or more `guestUsers` lack confirmed `audio` group; operator must set `User.spec.groups` and rebuild guest |
| `RuntimeCapabilityUnavailable` | `Pending` | true | `runtime-audio` alias returned no audio capability; retries when runtime updates |
| `RealizationCrashLoop` | `Failed` | false | An owned realization Process exceeded its restart ceiling |
| `ProviderMisconfigured` | `Failed` | false | Missing artifact or malformed capability record |
| `AudioNotEnabled` | `Failed` | false | `AudioBinding` exists but owning Guest has no runtime audio capability |

Error messages are bounded to 256 characters. They must not contain socket
paths, lock paths, state-file paths, PipeWire paths, compositor runtime
directory paths, PipeWire node IDs, credential digests, or volume/gain level
values.

## Audit events

All audio controller audit events use the Zone authoritative audit path.
Audit records are JSONL with V3 payload/checksum. Audit events are emitted
**post-commit** after the relevant revision is durable in the Zone store.

No audio-specific audit event includes socket paths, volume levels, gain
values, PipeWire node IDs, PipeWire runtime directory paths, microphone queue
positions, opaque consumer handles, or consumer Zone identities.

| Event kind | Trigger | Redacted |
| --- | --- | --- |
| `audio-service.created` | Owner or core-generated projection AudioService committed | role and operation set only; no authority key, endpoint locator, import key, or remote identity |
| `audio-service.authority-claimed` | Owner Service D097 claim committed | authority class and outcome only; no opaque key value or PipeWire identity |
| `audio-service.projection-bound` | Projection Service observes a current ResourceImport lease | outcome only; no ZoneLink/export key, session generation, or stream identifier |
| `audio-service.degraded` | Owner authority or projection route becomes unavailable/revoked | closed reason only |
| `audio-service.mic-arbitration` | Microphone request queued, activated, released, expired, or rejected at a bound | closed action/outcome and aggregate queue counts only; no Binding, Guest, Zone, handle, position, or client timestamp |
| `audio-binding.created` | `AudioBinding` resource committed to store | `spec.grants.*` direction values included; no paths; no levels |
| `audio-binding.grant-changed` | `spec.grants` `UpdateSpec` committed and durable | direction (`"on"`/`"off"`) changes included; `speakerLevel`/`micGain` values **omitted**; no node IDs |
| `audio-binding.enforcement-applied` | Referenced AudioService call returned `Applied` | result: `Applied\|Degraded` per channel; no route, node ID, or level value |
| `audio-binding.realization-ready` | All required owned realization resources become Ready | no socket path or provider-only status detail |
| `audio-binding.realization-stopped` | Owned realization resources reach terminal phases | includes a bounded aggregate outcome only; no socket path or implementation identity |
| `audio-binding.deleted` | post-commit after `phase=Deleted` revision event is durable | - |

**Suppressed from all audit records:**
- vhost-user socket path;
- PipeWire runtime directory path;
- AudioMediator FD attachment details;
- initial stream property values;
- volume/gain levels (`speakerLevel`, `micGain`);
- PipeWire node IDs or object paths;
- GuestAudioAgent AudioSet service payload bytes;
- compositor user session details;
- microphone queue entries, positions, client timestamps, opaque handles, and
  consumer Zone identities.

`subject_digest` is always the authenticated Zone subject digest (`sha256:<hex>`).
ProcessEffect audit records (process launch, signal, exit) are emitted by
the Process Provider, not by this Provider.

## OTEL telemetry

### Metric labels

All audio-pipewire metrics use only the following closed label set.

| Label key | Allowed values | Notes |
| --- | --- | --- |
| `provider` | `audio-pipewire` | Always present |
| `channel` | `speaker\|mic` | For per-channel metrics |
| `outcome` | `ok\|degraded\|failed\|unknown` | Terminal state |
| `enforcement_posture` | `host-and-guest\|host-only\|guest-only\|none` | |
| `arbitration_state` | `inactive\|queued\|active\|blocked` | Microphone arbitration metrics only |

Guest name, Zone name, socket path, level values, PipeWire node IDs, and
runtime capability implementation IDs, opaque consumer handles, and queue
positions are not metric labels.

### Metrics

| Metric name | Type | Description |
| --- | --- | --- |
| `d2b_audio_pipewire_services_total` | gauge | Current count of owner/projection AudioServices by `outcome` |
| `d2b_audio_pipewire_bindings_total` | gauge | Current count of per-Guest AudioBinding resources by `outcome` |
| `d2b_audio_pipewire_sidecars_running` | gauge | Count of worker Processes in Ready phase |
| `d2b_audio_pipewire_enforcement_attempts_total` | counter | AudioService enforcement attempts by `channel`, `outcome` |
| `d2b_audio_pipewire_enforcement_latency_seconds` | histogram | AudioService call completion latency by `channel` |
| `d2b_audio_pipewire_sidecar_restarts_total` | counter | Worker Process restart count (observed from Process.status) |
| `d2b_audio_pipewire_grant_changes_total` | counter | `spec.grants` mutations by `channel` |
| `d2b_audio_pipewire_mediator_fd_handoffs_total` | counter | PipeWire FD handoffs completed by AudioMediator by `outcome` |
| `d2b_audio_pipewire_mic_active` | gauge | Owner-Service active microphone capture count; always `0` or `1` |
| `d2b_audio_pipewire_mic_queue_depth` | gauge | Aggregate bounded pending microphone request count; no Zone or consumer label |
| `d2b_audio_pipewire_mic_arbitration_total` | counter | Microphone arbitration transitions by `arbitration_state` and `outcome` |
| `d2b_audio_pipewire_telemetry_drop_total` | counter | Dropped telemetry frames when emitter ring is full |

No level or gain value appears in any metric. `grant_changes_total` counts
direction transitions; it does not record the level value.

### Span attributes

Controller spans carry:
- `d2b.provider`: `audio-pipewire`
- `d2b.component`: `audio-binding-controller` or `audio-mediator`
- `d2b.resource.type`: `AudioService`, `AudioBinding`, `Process`, or `Endpoint`
- `d2b.resource.generation`: current `metadata.generation`
- `d2b.outcome`: outcome code

Zone/resource identity is available only in bounded OTEL resource attributes
and permitted audit fields, never as a span attribute.
Spans must not carry socket paths, PipeWire paths, PipeWire node IDs, level
values, gain values, or guest workload usernames.

## Async reconciliation

### Controller descriptor

```yaml
watchSelectors:
  - resourceType: audio.d2bus.org.AudioService
    specProviderRef: Provider/audio-pipewire
    verbs: [spec, status, deletion]
  - resourceType: ResourceImport
    ownerOfType: audio.d2bus.org.AudioService
    specProviderRef: Provider/audio-pipewire
    verbs: [spec, status, deletion]
  - resourceType: Endpoint
    ownerRefType: audio.d2bus.org.AudioService
    verbs: [status]
  - resourceType: audio.d2bus.org.AudioBinding
    specProviderRef: Provider/audio-pipewire
    verbs: [spec, status, deletion]
  - resourceType: Process
    ownerRefType: audio.d2bus.org.AudioBinding
    verbs: [status]
  - resourceType: Endpoint
    ownerRefType: audio.d2bus.org.AudioBinding
    verbs: [status]
  - resourceType: audio.d2bus.org.AudioService
    specProviderRef: Provider/audio-pipewire
    dependencyRefIn: AudioBinding.spec.serviceRef
    verbs: [spec, status, deletion]
  - resourceType: Guest
    verbs: [status]
    ownerTrigger: true
  - resourceType: Process
    componentIdentity: audio-mediator
    verbs: [status]
  - resourceType: User
    dependencyRefIn: AudioBinding.spec.guestUsers
    verbs: [status]
dependencySelectors:
  - resourceType: ResourceImport
    resolveFrom: AudioService.metadata.ownerRef
  - resourceType: Endpoint
    resolveFrom: AudioService.spec.implementationEndpointRefs
  - resourceType: audio.d2bus.org.AudioService
    specProviderRef: Provider/audio-pipewire
    resolveFrom: AudioBinding.spec.serviceRef
  - resourceType: Guest
    resolveFrom: AudioBinding.metadata.ownerRef
  - resourceType: User
    resolveFrom: AudioBinding.spec.guestUsers
allowedResourceVerbs:
  - { type: audio.d2bus.org.AudioService, specProviderRef: Provider/audio-pipewire, verbs: [get, list, watch, update-status, update-finalizers] }
  - { type: audio.d2bus.org.AudioBinding, specProviderRef: Provider/audio-pipewire, verbs: [get, list, watch, update-status, update-finalizers] }
  - { type: ResourceImport, verbs: [get, watch] }
  - { type: Process, verbs: [get, list, watch, create, update-spec, delete] }
  - { type: Endpoint, verbs: [get, list, watch, create, update-spec, delete] }
  - { type: Guest, verbs: [get, watch, update-spec] }
reconcileConcurrency: 8
maxPendingResources: 512
observePolicy: on-status-change
resyncPeriod: "5m"
finalizers:
  - audio-pipewire.d2bus.org/service-released
  - audio-pipewire.d2bus.org/sidecar-stopped
deadlines:
  reconcile: "30s"
  finalize: "120s"
  observe: "10s"
retryClasses:
  - code: ServiceNotReady
    backoff: exponential-bounded-30s
  - code: ImportNotBound
    backoff: exponential-bounded-30s
  - code: EndpointNotReady
    backoff: exponential-bounded-30s
  - code: AuthorityConflict
    policy: no-retry
  - code: ProjectionLocalBackingDenied
    policy: no-retry
  - code: RealizationNotReady
    backoff: exponential-bounded-30s
  - code: ProviderSessionUnavailable
    backoff: exponential-bounded-30s
  - code: ConsumerEnforcementUnavailable
    backoff: exponential-bounded-30s
  - code: GrantEnforcementFailed
    backoff: exponential-bounded-30s
  - code: RealizationCrashLoop
    policy: no-retry
  - code: GuestUserAudioGroupMissing
    policy: no-retry
```

`EphemeralProcess` is absent from `allowedResourceVerbs`. The controller
cannot create EphemeralProcess resources. `SpawnRunner`, `OpenPidfd`, and all
broker operations are also absent. `Volume` and `User` are absent. `Endpoint`
verbs are limited to Service-owned local implementation/route Endpoints and
AudioBinding-owned private endpoints. The Provider has no `create`/`delete` verb
for AudioService: operators/Nix own owner-Service lifecycle and the **core
ResourceImport controller** owns projection-Service creation/deletion. Core's
projection authority is constrained to
`serviceRole: projection`, `ownerRef: ResourceImport/<name>` and cannot create
AudioBinding. The ProviderStateSet is empty under D087, so core
ProviderDeployment creates no Provider state Volume and the component consumes
no state view `dirfd`.

The ResourceType names in this descriptor are neutral. The
`specProviderRef: Provider/audio-pipewire` selector and matching authorization
constraint are mandatory: this controller ignores resources selected for a
different conforming audio Provider and cannot update their status or
finalizers.

### Reconcile flow (per `AudioService`)

```text
reconcile(AudioService):
  1. Load Service spec/status; require spec.providerRef Provider/audio-pipewire
     and classify immutable serviceRole. Ignore a resource selected for any
     other conforming audio Provider.
  2. If owner:
       a. Require ownerRef Provider/audio-pipewire and a valid D097
          AuthorityDescriptor.
       b. Resolve only same-Zone implementationEndpointRefs.
       c. Claim/revalidate the authority index by ownerProof; on duplicate,
          set AuthorityConflict and perform no PipeWire open.
       d. Observe the local AudioMediator/PipeWire backing and Endpoint status.
       e. Reconcile the exclusive microphone slot and bounded aggregate
          per-Zone fair-queue status; never persist queue entries.
  3. If projection:
       a. Require ownerRef ResourceImport/<name>; reject any authority field.
       b. Resolve that same-Zone import and require bound/current generation,
          expected AudioService type, and both signed projection-schema/factory
          fingerprints.
       c. Create/update only the Service-owned local route Endpoint and bind it
          to the import's bounded encrypted named streams.
       d. Deny any AudioMediator portal attachment or PipeWire open.
  4. Atomically write layered Service status and D091 currency.
  5. On delete, drain referencing AudioBindings and Service Endpoints, release
     owner authority or projection route, then clear service-released.
```

### Reconcile flow (per `AudioBinding`)

```text
reconcile(AudioBinding):
  1. Load AudioBinding spec/status from store (MVCC snapshot); require
     spec.providerRef Provider/audio-pipewire and ignore another Provider's
     conforming Binding.
  2. Resolve ownerRef Guest and same-Zone spec.serviceRef AudioService.
     Reject ResourceImport refs and cross-Zone Services. If Service is not
     Ready/current, set ServiceUnavailable and create no worker fallback.
  3. Check Guest.status.phase.
  4. If suspendOnGuestAbsent && Guest not Running:
      → idempotently release any active/pending microphone request;
      → issue Process.spec update (desired: stopped) if Process exists;
        set GuestAbsent condition; Degraded.
  5. Query runtime-audio dependency alias for the Guest's AudioCapability.
     If no audio capability advertised: set RuntimeCapabilityUnavailable; Pending; retry.
  6. For each ref in spec.guestUsers: verify User.status.groupMembershipVerified == true
     for the "audio" group. If any not verified: set GuestUserAudioGroupMissing;
     Degraded; no-retry (operator action required).
  7. Resolve Service backend:
       owner → require local AudioMediator Endpoint;
       projection → require local import-route Endpoint and no PipeWire FD.
  8. Determine sidecar desired state.
     If mic grant is "off", idempotently release any active/pending microphone
     request before evaluating Process state.
     If both grants "off" → desired: stopped; remove Guest.spec.audioExtension.
     Else → desired: running.
  9. If desired: running:
      a. Create Process/<name>-audio-sidecar if absent (CREATE resource API).
      b. Create its private vhost-user Endpoint if absent.
      c. For each ref in spec.guestUsers:
         - Compute opaque digest of AudioBinding.metadata.uid + User.<name>.metadata.uid.
         - Create Process/ag-<digest> if absent (CREATE resource API), with
          ownerRef: audio.d2bus.org.AudioBinding/<name> and userRef: <guestUser ref>.
         - Create its private AudioSet Endpoint if absent.
         - Locate each existing GuestAudioAgent Process via ownerRef component
          identity index, not label selector.
      d. If sidecar Process/status or private Endpoint is not Ready: retry.
      e. For each GuestAudioAgent Process/Endpoint: if not Ready,
         set ConsumerEnforcementUnavailable (keyed by digest); Degraded; retry.
  10. If desired: stopped:
      a. If sidecar Process exists and not terminal: issue Process UpdateSpec
         (desired phase: stopped). Process Provider performs graceful stop.
      b. Delete/stop private Endpoints and, for each GuestAudioAgent Process
         (ownerRef identity index), if not terminal,
         issue UpdateSpec (desired phase: stopped).
      c. Set GuestAbsent/SidecarStopped; wait for terminal phase.
  11. Enforce grants (if Service, sidecar, and all agents/Endpoints are Ready):
      a. Call the resolved AudioService for each changed channel. Owner
         dispatches locally to AudioMediator; projection routes over the
         import's encrypted named stream. Speaker receives Applied or a typed
         error. Microphone receives Applied, Queued, MicQueueFull, or another
         typed error.
      b. Call AudioSet service on ALL GuestAudioAgent Processes in parallel
         (d2b-bus, vsock transport). Collect all results. A microphone
         Applied result permits guest `mic: "on"`; Queued or MicQueueFull
         requires guest `mic: "off"` so a waiting consumer stays muted.
      c. Aggregate results: if any agent fails, set GuestEnforcementFailed
         (keyed by agent digest); if all fail, enforcementPosture = HostOnly.
         For microphone, map Applied to MicCaptureActive/active, Queued to
         MicQueued/queued/Pending, and MicQueueFull to
         MicQueueFull/blocked/Degraded. Update status.channels and
         enforcementPosture.
  12. Update Guest.spec.audioExtension if needed (derived from AudioCapability record).
  13. Commit UpdateStatus batch (post-reconcile; single commit).
  14. Emit post-commit audit event if grants changed.
```

All steps are asynchronous. The controller's reconcile work queues use stable
UID ordering and are distinct from the owner Service's microphone arbitration
queue. Steps for independent resources run concurrently, and an AudioService event
enqueues every AudioBinding indexed by `spec.serviceRef`.
Each task holds its own optimistic revision precondition; a conflict causes a
retry.

## Audio authority and cross-Zone sharing (D096/D097)

### Resource graphs and ownership

The two provider-neutral qualified ResourceTypes have non-overlapping jobs.
The owner-Zone graph is:

```text
Provider/audio-pipewire
├── Process/audio-pipewire-mediator                 (static implementation)
└── audio.d2bus.org.AudioService/host-audio (real authority)
    ├── AuthorityDescriptor                         (D097 physical backing)
    └── Endpoint/audio-pipewire-authority            (same-Zone only)

ResourceExport/host-audio
├── resourceRef                  -> audio.d2bus.org.AudioService/host-audio
├── serviceType                  -> audio.d2bus.org.AudioService
├── projectionSchemaFingerprint  -> signed projection schema
└── factoryFingerprint           -> signed semantic factory

Guest/corp-vm
└── AudioBinding/corp-vm-audio
    ├── serviceRef -> AudioService/host-audio
    ├── Process/corp-vm-audio-sidecar
    ├── Endpoint/corp-vm-audio-vhost-user            (private)
    ├── Process/ag-<digest>
    └── Endpoint/ag-<digest>-audio-set                (private)
```

The consumer-Zone graph is:

```text
ResourceImport/host-audio
└── AudioService/host-audio-projection                (core-created projection)
    ├── ownerRef -> ResourceImport/host-audio
    └── Endpoint/host-audio-import-route              (same-Zone route)

Guest/work-vm
└── AudioBinding/work-vm-audio
    ├── serviceRef -> AudioService/host-audio-projection
    ├── Process/work-vm-audio-sidecar
    └── private vhost-user/GuestAudioAgent Endpoints

projection route == bounded encrypted named streams == owner Service
```

AudioBinding remains per Guest in both graphs. It is not exported, imported, or
core-generated. The only core-generated semantic projection is AudioService.

### Owner authority Service (D097)

Exactly one owner `audio.d2bus.org.AudioService` per owner Zone holds
the real PipeWire connection to the physical microphone and speakers and is the
single arbiter of every local or imported consumer. It is grounded in the
current host controller (`packages/d2bd/src/audio_host_controller.rs`: the
dyn-safe `HostAudioController` trait,
`PipeWireHostController::{from_audio_node,find_audio_node}`, with
`QemuAudioController`/`FakeHostController` as test doubles) and argv generator
(`packages/d2b-host/src/audio_argv.rs`: `AudioBackend`, `AudioArgvInput`, and
`generate_audio_argv`).

The owner Service alone declares the D097 `AuthorityDescriptor` with
`authorityScope: physical-device`, opaque `authorityKey`, `cardinality:
zero-or-one`, and split arbitration:

- **Speaker: `multiplexed`.** The authority runs one mixer. Every consumer
  AudioBinding receives a mixed stream with its admitted per-Zone quota and
  per-Guest level. No consumer opens the physical sink.
- **Microphone: `exclusive` only.** Exactly one local or imported consumer may
  capture. Pending requests enter the fixed bounded queue: owner-sequenced FIFO
  within each authenticated Zone, round-robin across non-empty Zones, at most
  16 requests per Zone and 64 total, one request per opaque consumer handle.
  A contended 30-second lease cannot renew, and the old stream is muted and
  disconnected before the next activates; if its intent remains on, it
  re-enters its Zone FIFO tail. No priority or client timestamp affects order.
  Concurrent/multiplexed capture and every consent/approval placeholder are
  rejected until a future normative spec defines a concrete consent
  authorization ResourceType and resource-API verb.

Core's authority index rejects a second owner AudioService for the same
`(Zone, physical-device, opaqueKeyDigest)` with `duplicateConflict` before any
open. Restart adopts the exact Service by ownerProof; ambiguity quarantines.
AudioBinding carries no AuthorityDescriptor and cannot claim this index.

### ResourceExport and ResourceImport (D096)

The owner Zone exports the **owner AudioService**, not AudioBinding:

```yaml
apiVersion: resources.d2bus.org/v3
type: ResourceExport
metadata:
  name: host-audio
  zone: host
spec:
  providerRef: Provider/audio-pipewire
  resourceRef: audio.d2bus.org.AudioService/host-audio
  serviceType: audio.d2bus.org.AudioService
  projectionSchemaFingerprint: sha256:<audio-service-projection-schema>
  factoryFingerprint: sha256:<audio-projection-factory>
  operations: [playback, capture]
  arbitration: multiplexed
  quota:
    maxConsumers: 16
    fairness: fifo
    leaseDeadlineMs: 30000
  consumerZonePolicy:
    zones: [Zone/work]
    capabilityCeiling: [playback, capture]
  visibility: named-zones
```

The consumer Zone imports a local projection AudioService:

```yaml
apiVersion: resources.d2bus.org/v3
type: ResourceImport
metadata:
  name: host-audio
  zone: work
spec:
  providerRef: Provider/audio-pipewire
  zoneLinkRef: ZoneLink/work-uplink
  exportKey: host/host-audio
  expectedServiceType: audio.d2bus.org.AudioService
  expectedProjectionSchemaFingerprint: sha256:<audio-service-projection-schema>
  expectedFactoryFingerprint: sha256:<audio-projection-factory>
  projectionName: host-audio-projection
  requestedCapabilities: [playback, capture]
  requestedQuota:
    leaseDeadlineMs: 30000
  disconnectPolicy:
    mode: degrade
```

The Export's `arbitration: multiplexed` describes concurrent Service sessions
and speaker mixing; it does not relax the AudioService schema's
`authority.arbitration.microphone: exclusive`. Capture operations from all
imports still enter the single owner-side fair queue defined above. The audio
adapter admits exactly `quota.fairness: fifo` and
`quota.leaseDeadlineMs: 30000`; these generic Export fields cannot introduce a
priority or extend a contended microphone lease.

The signed audio projection factory and the live Export/Import fields agree
exactly:

| Factory/authoring field | Audio value |
| --- | --- |
| `serviceType` / `expectedServiceType` | `audio.d2bus.org.AudioService` |
| `bindingType` | `audio.d2bus.org.AudioBinding` |
| `allowedBackingRefTypes` | `Endpoint` |
| `allowedBindingTargetRefTypes` | `Guest` |
| `projectionSchema` | Strict projection AudioService schema: `providerRef`, role/import semantic fields, and no `spec.provider`, authority, Endpoint locator, FD, path, credential, payload, or consent field |
| `projectionSchemaFingerprint` / `expectedProjectionSchemaFingerprint` | SHA-256 of that canonical committed projection schema |
| `factoryFingerprint` / `expectedFactoryFingerprint` | SHA-256 binding the semantic factory fields and projection-protocol version; never Provider/adapter identity |

Provider install, Nix build, API admission, export, import, and reconnect fail
closed unless the signed factory, Service type, projection-schema fingerprint,
and factory fingerprint match exactly. The Export has no Endpoint field: its
`resourceRef` selects the owner Service, and that Service alone owns its local
authority Endpoint.

ResourceExport status reports the verified projection-schema/factory
fingerprints, owner Service generation/readiness, active/pending consumer
counts, and no Endpoint locator. Its closed conditions are
`ExportAdvertised`, `AuthorityReady`, `ConsumersAdmitted`, and `Revoking`.
ResourceImport status reports the same verified fingerprints, local projection
Service Ref, lease state, and bounded session-generation digest. Its closed
conditions include `ExportReachable`, `FactoryMatched`, `SchemaMatched`,
`Bound`, `ProjectionReady`, `BindingReferencesRemain`, and `Degraded`.

Core owns ResourceExport/ResourceImport routing and base lifecycle. On a bound
import, core creates only
`audio.d2bus.org.AudioService/host-audio-projection` with
`metadata.ownerRef: ResourceImport/host-audio`; it never creates AudioBinding,
workers, guest agents, or private Endpoints. The Provider import adapter
reconciles the projection's local route semantics. An operator then declares a
normal per-Guest AudioBinding whose `serviceRef` names that projection.

No PipeWire FD, socket, path, authority handle, or ResourceRef crosses a Zone.
Audio frames and service operations flow only over per-import bounded encrypted
named streams, grounded in `packages/d2b-realm-core/src/stream.rs`
(`StreamKind::{AudioPlayback,AudioCapture}` and split-direction
`StreamChannel`) and `packages/d2b-realm-core/src/mux.rs` credit-based flow.
Each import has its own session generation, authorization, credits/backpressure,
cancel, deadline, and idempotency. Intermediaries see ciphertext.

### Deterministic controller ownership

| Controller | Creates/deletes | Reconciles | Must never do |
| --- | --- | --- | --- |
| Core ResourceImport controller | Exactly one projection AudioService per ResourceImport | Import lease, projection ownerRef/lifecycle, D091 propagation | Create AudioBinding, Process, Endpoint, GuestAudioAgent, or open PipeWire |
| AudioService controller | Service-owned local implementation/route Endpoints only | Owner AuthorityDescriptor, speaker mixer, exclusive bounded fair mic arbiter, owner endpoint; or projection import/stream route | Create/delete projection Service, put authority on a projection, open PipeWire for a projection |
| AudioBinding controller | Per-Binding vhost-user worker, GuestAudioAgent Processes, and private Endpoints | Guest ownership, same-Zone serviceRef, grants/levels, guest frontend and status | Export/project AudioBinding, claim physical authority, follow a cross-Zone Ref, handle FDs directly |

Export removal or ZoneLink loss revokes leases and marks the projection Service
`Degraded`/`revoked`; its referencing AudioBindings degrade through the serviceRef
index. Reconnect revalidates generation, Service type, and both fingerprints
before binding. D091
currency is ordered remote owner Service -> ResourceImport -> projection
Service -> AudioBinding -> owned realization. A disruptive owner upgrade drains
consumers and named streams before recycling authority; no stale projection
continues.

Guest AudioSet calls remain concurrent across all active GuestAudioAgent
Processes. The authority aggregates per-guest results and never serializes one
slow Guest behind another.

### Selected invariants

- **vhost-device-sound v0.3.0** is required; nixpkgs v0.2.0 has the known
  PipeWire-backend format-negotiation bug and MUST NOT be used.
- **No `monitor.rules`.** WirePlumber split-direction enforcement uses
  `client.conf.d` stream rules or scripts, never the section that broke host
  audio output.
- **Volume/gain redaction.** Speaker level, mic gain, authority key, raw node
  identity, import keys, and stream/session identifiers are redacted from audit,
  OTEL labels, and logs; status remains bounded and provider-neutral.
- **Encrypted credit streams.** All cross-Zone audio bytes use the bounded
  encrypted credit streams above; no plaintext, FD, socket, path, or authority
  grant crosses a Zone.
- **Exclusive microphone, mixed speaker.** Speaker streams remain
  multiplexed/mixed. Initial-v3 capture has one owner-side slot and the fixed
  bounded per-Zone fair queue; no schema, Provider extension, Export policy, or
  route admits concurrent capture or an undefined consent override.
- **No legacy shortcuts.** The two-ResourceType model adds no new ProcessRole,
  direct broker path, per-VM state file, or ambient PipeWire access. Effects use
  D077 EffectPort/LaunchTicket, observations are D087 status-first, schemas and
  status retain D088/D089 three-layer shape, and cross-Zone sharing is only
  D096 ResourceExport/ResourceImport plus Service-owned Endpoints.

## Nix authoring and configuration

### Operator-facing provider-neutral resource schema

```nix
d2b.zones.dev.resources = {
  host-audio = {
    type = "audio.d2bus.org.AudioService";
    metadata.ownerRef = "Provider/audio-pipewire";
    spec = {
      providerRef = "Provider/audio-pipewire";
      serviceRole = "owner";
      implementationEndpointRefs = [ "Endpoint/audio-pipewire-authority" ];
      operations = [ "playback" "capture" ];
      authority = {
        authorityScope = "physical-device";
        authorityClass = "audio";
        authorityKey = "host-default-audio";
        cardinality = "zero-or-one";
        arbitration = {
          speaker = "multiplexed";
          microphone = "exclusive";
        };
        exportability = "explicit-export";
      };
    };
  };

  host-audio-export = {
    type = "ResourceExport";
    spec = {
      providerRef = "Provider/audio-pipewire";
      resourceRef = "audio.d2bus.org.AudioService/host-audio";
      serviceType = "audio.d2bus.org.AudioService";
      projectionSchemaFingerprint =
        "sha256:<audio-service-projection-schema>";
      factoryFingerprint = "sha256:<audio-projection-factory>";
      operations = [ "playback" "capture" ];
      arbitration = "multiplexed"; # speaker sessions; mic remains exclusive
      quota = {
        maxConsumers = 16;
        fairness = "fifo";
        leaseDeadlineMs = 30000;
      };
      consumerZonePolicy = {
        zones = [ "Zone/work" ];
        capabilityCeiling = [ "playback" "capture" ];
      };
      visibility = "named-zones";
    };
  };

  corp-vm-audio = {
    type = "audio.d2bus.org.AudioBinding";
    metadata.ownerRef = "Guest/corp-vm";
    spec = {
      providerRef = "Provider/audio-pipewire";
      serviceRef = "audio.d2bus.org.AudioService/host-audio";
      grants = {
        mic = "off";
        speaker = "on";
        speakerLevel = 75;   # null = system default
        micGain = null;
      };
      guestUsers = [ "User/alice" ];  # Nix/compiler sets spec.groups=["audio"] on User/alice
      suspendOnGuestAbsent = true;
    };
  };
};
```

A consumer Zone declares ResourceImport plus a normal per-Guest AudioBinding. It
does not author the projection Service; core generates it:

```nix
d2b.zones.work.resources = {
  host-audio-import = {
    type = "ResourceImport";
    spec = {
      providerRef = "Provider/audio-pipewire";
      zoneLinkRef = "ZoneLink/work-uplink";
      exportKey = "dev/host-audio-export";
      expectedServiceType = "audio.d2bus.org.AudioService";
      expectedProjectionSchemaFingerprint =
        "sha256:<audio-service-projection-schema>";
      expectedFactoryFingerprint = "sha256:<audio-projection-factory>";
      projectionName = "host-audio-projection";
      requestedCapabilities = [ "playback" "capture" ];
      requestedQuota.leaseDeadlineMs = 30000;
    };
  };

  work-vm-audio = {
    type = "audio.d2bus.org.AudioBinding";
    metadata.ownerRef = "Guest/work-vm";
    spec = {
      providerRef = "Provider/audio-pipewire";
      serviceRef =
        "audio.d2bus.org.AudioService/host-audio-projection";
      grants = {
        mic = "off";
        speaker = "on";
        speakerLevel = 75;
        micGain = null;
      };
      guestUsers = [ "User/alice" ];
      suspendOnGuestAbsent = true;
    };
  };
};
```

### Provider root configuration (`spec.config`)

```nix
d2b.zones.dev.resources.audio-pipewire = {
  type = "Provider";
  spec = {
    artifactId = "audio-pipewire";
    config = {
      captureAlias = null;
      # When non-null: a bounded named alias matching ^[a-z][a-z0-9-]*$,
      # max 64 chars. The AudioMediator resolves it to the actual PipeWire
      # node via libpipewire registry introspection at runtime, privately.
      # This is NOT a PipeWire node ID, object path, or socket path.
      # Example: captureAlias = "desk-headset";
    };
  };
};
```

This Provider config is strict and deny-unknown. In its initial schema,
`captureAlias` is its only operator-set PipeWire field; compiled
`client.conf.d`/WirePlumber policy remains Provider package configuration, not
an AudioService or AudioBinding field. Provider config cannot shadow grants,
levels, service roles, authority, or references from the neutral base schemas.
Microphone queue bounds, ordering, and lease behavior are fixed initial-v3
contract values, not authorable config. Consent, approval, priority, and
concurrent-capture settings are unknown.

### Nix validation

Nix eval-time validation checks the two provider-neutral qualified
ResourceTypes:

- owner `AudioService` has `metadata.ownerRef: Provider/audio-pipewire`, a
  D097 AuthorityDescriptor, and only same-Zone Endpoint refs;
- projection `AudioService` has
  `metadata.ownerRef: ResourceImport/<name>`, no AuthorityDescriptor, and only
  same-Zone route Endpoint refs; a projection may only be compiler materialized
  by core from a matching ResourceImport;
- ResourceExport `serviceType`, `projectionSchemaFingerprint`, and
  `factoryFingerprint` exactly match the installed signed AudioService
  projection factory; ResourceImport uses matching `expectedServiceType`,
  `expectedProjectionSchemaFingerprint`, and `expectedFactoryFingerprint`;
  only `audio.d2bus.org.AudioService`, never AudioBinding, is selected;
- the obsolete ResourceExport fields `endpointRef`, `exportedType`, and
  `baseSchemaFingerprint`, and obsolete ResourceImport fields `expectedType`,
  `expectedBaseSchemaFingerprint`, and `projectionType`, each fail as unknown
  in explicit rejection tests; no compatibility alias is emitted;
- a ResourceExport never contains an Endpoint field; the owner AudioService
  retains all `implementationEndpointRefs`;
- audio ResourceExport requires `arbitration = "multiplexed"` for Service
  sessions/speaker mixing, `quota.fairness = "fifo"`, and
  `quota.leaseDeadlineMs = 30000`, and an audio ResourceImport requests that
  same deadline; none overrides exclusive microphone arbitration or introduces
  priority;
- `spec.providerRef` resolves to an installed `Provider/audio-pipewire` in the
  same Zone; evaluation fails with a descriptive error if absent;
- `AudioBinding.spec.serviceRef` resolves to an owner or projection AudioService
  in the same Zone and never to ResourceImport directly;
- `spec.grants.speakerLevel` is `null` or an integer in `[0,100]`;
- `spec.grants.micGain` is `null` or an integer in `[0,100]`;
- `spec.guestUsers` contains ≤16 entries, each a valid `User/<name>` ResourceRef
  where `<name>` matches `[a-z][a-z0-9_-]*` ≤32 chars; at compile time, the
  Nix module sets `spec.groups: ["audio"]` on each referenced guest `User`
  resource; for API-created resources, the controller verifies
  `User.status.groupMembershipVerified` and fails closed;
- `metadata.ownerRef` resolves to an existing `Guest/<name>` in the same Zone;
- no two `AudioBinding` resources share the same Guest `metadata.ownerRef`;
- no AudioBinding contains `authority`, has `ownerRef: ResourceImport/<name>`, or
  is selected as an exported/projection type;
- owner AudioService requires speaker `multiplexed` and microphone `exclusive`;
  every other microphone arbitration value and every consent, approval,
  priority, or concurrent-capture field fails as unknown;
- every provider-qualified Service/Binding type and every former AudioState
  spelling fails as unknown; the Nix compiler emits no alias or deprecation
  shim;
- PipeWire-specific fields at AudioService/AudioBinding base `spec.*` fail as
  unknown. The initial provider accepts only absent or strict versioned
  `spec.provider` with empty `settings`; `captureAlias` is accepted only at
  `Provider/audio-pipewire.spec.config`.

The Nix module also validates that:
- `Provider/audio-pipewire.spec.config.captureAlias`, when non-null, matches
  `^[a-z][a-z0-9-]*$` and is ≤64 characters; no path separator, whitespace,
  PipeWire syntax, numeric start, or uppercase is permitted;
- `d2b.site.audio.inputTargetNode` (the current legacy option) is absent when
  the v3 Provider is installed; a clear migration error is emitted if both
  are set.

The generated base schemas are owned by the `audio.d2bus.org` contract
namespace. The selected Provider supplies signed implementation capability and
provider-envelope schemas; it does not mint a second PipeWire-qualified copy of
either ResourceType.

### Generated canonical ResourceSpec

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type": "audio.d2bus.org.AudioBinding",
  "metadata": {
    "name": "corp-vm-audio",
    "zone": "dev",
    "ownerRef": "Guest/corp-vm"
  },
  "spec": {
    "providerRef": "Provider/audio-pipewire",
    "serviceRef": "audio.d2bus.org.AudioService/host-audio",
    "grants": {
      "mic": "off",
      "speaker": "on",
      "speakerLevel": 75,
      "micGain": null
    },
    "guestUsers": ["User/alice"],
    "suspendOnGuestAbsent": true
  }
}
```

### Resource cleanup contract

When an `AudioBinding` resource is removed from the Nix configuration:
1. The new generation activates immediately; the resource enters
   `deletionRequestedAt` asynchronous deletion.
2. The generation reports `Degraded/pending-cleanup` until deletion completes.
3. Activation does not block; the rest of the Zone configuration activates
   normally.
4. The owned `Process` resources are deleted finalizer-safely per the deletion
   sequence above.
5. The `GuestAudioAgent` Process for the deleted Guest is stopped and deleted
   by the finalizer handler. On next guest NixOS config generation rebuild,
   the guest user's `spec.groups` audio entry is removed (Nix/compiler removes
   the declaration; system-core reconciles).

When an owner AudioService is removed, its finalizer first drains referencing
AudioBindings and exports, releases the authority and local implementation
Endpoints, then removes the Service. A projection AudioService is never removed
directly from Nix: deleting/revoking its ResourceImport drives the core
child-first sequence and core deletes that projection after consumers and the
remote lease are released.

## Current-code fit

| Item | Value |
| --- | --- |
| Current anchor | `nixos-modules/components/audio/host.nix`, `guest.nix`, `packages/d2b-core/src/audio_policy.rs`, `packages/d2bd/src/audio_dispatch.rs`, `packages/d2bd/src/audio_host_controller.rs`, `packages/d2b-host/src/audio_argv.rs` |
| Evidence class | Mixed; `audio_policy.rs` / `audio_argv.rs` are `implemented-and-reachable`; v3 Provider/two-ResourceType AudioService+AudioBinding/AudioMediator wiring is `ADR-only` |
| Behavior retained | Per-VM mic/speaker grants, `LevelPercent` 0..=100, component-template argv shape, PipeWire `client.conf.d/` stream rule placement, WirePlumber virtio-snd profile, zero host capabilities |
| Required delta | Provider-neutral qualified `audio.d2bus.org.AudioService` owner/projection ResourceType plus per-Guest `audio.d2bus.org.AudioBinding` ResourceType with required same-Zone `serviceRef`, initially implemented by `Provider/audio-pipewire`; strict provider-field isolation; deterministic Service and Binding handlers; core-created projection-Service contract; owner `AudioMediator` with `SetGrant`/`SetLevel`; GuestAudioAgent and private Endpoints; runtime-audio alias; encrypted import streams; D097 authority/mixer/mic arbitration; three-layer status and D091 propagation; RBAC/watch plans; Nix authoring and tests |
| Reuse path | `d2b-core/src/audio_policy.rs` → adapt into `src/audio_policy.rs` (one-time activation migration from v1/v2 on-disk format only; no ongoing state file); `audio_argv.rs` → adapt into component template (not live Process spec); WirePlumber stream rule Nix logic → port to Provider Nix module |
| Replacement/deletion | `audio_dispatch.rs`, `audio_host_controller.rs` retired after `audio-binding-controller` passes e2e parity; `host.nix`, `guest.nix` retired after v3 Nix module deployed; `d2b-core/src/audio_policy.rs` may remain as re-export shim |
| Feasibility proof | Minijail contract tests in `d2b-contract-tests/tests/minijail_audio_usbip.rs`; ComponentSession FD-attachment protocol from ADR-046-componentsession-and-bus; libpipewire `pw_node_set_param` API confirmed in upstream PipeWire 1.x |
| Future owner | `d2b-provider-audio-pipewire` crate; exact work items listed below |

## Implementation work items

### ADR046-audio-001: Extract `AudioPolicyState` into Provider crate

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-001` |
| Dependency/owner | No prerequisites; `d2b-provider-audio-pipewire` crate |
| Current source | `packages/d2b-core/src/audio_policy.rs` (all symbols); `packages/d2b-core/tests/audio_policy.rs` |
| Reuse source | Same baseline paths |
| Reuse action | copy-unchanged |
| Destination | `packages/d2b-provider-audio-pipewire/src/audio_policy.rs`; re-exported from crate root |
| Detailed design | `LevelPercent`, `AudioGrant`, `AudioPolicyState`, `parse_audio_state`, `to_v2_bytes`, `AudioPolicyError` copy unchanged. `AudioPolicyState` is the canonical in-memory representation of `AudioBinding.spec.grants`. `parse_audio_state`/`to_v2_bytes` are used only once during first-activation migration from a prior v1/v2 on-disk file; there is no ongoing state file in v3. Primary reuse disposition: `copy-unchanged`. Preserved source-plan detail: `copy-unchanged` (no daemon imports; pure DTO library). |
| Integration | First-activation migration uses `parse_audio_state` and writes grants only into a per-Guest AudioBinding that also names the explicitly configured same-Zone owner AudioService. |
| Data migration | Parse v1/v2 once; require exactly one configured owner Service; write grants plus `serviceRef`; fail closed on missing/ambiguous Service; remove prior file only after successful commit. |
| Validation | `tests/audio_policy.rs`: all existing tests from `d2b-core/tests/audio_policy.rs` plus AudioBinding spec serialization tests |
| Removal proof | `d2b-core/src/audio_policy.rs` deleted when no `d2bd` caller references it; confirmed by `cargo check --no-default-features`. |

### ADR046-audio-002: Adapt `AudioArgvInput` into vhost-user-sound component template

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-002` |
| Dependency/owner | Depends on `ADR046-audio-001`; Process Provider template schema |
| Current source | `packages/d2b-host/src/audio_argv.rs` (all symbols + tests); `tests/golden/runner-shape/audio-argv-minimal.txt` |
| Reuse source | Same baseline paths |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-audio-pipewire/src/argv.rs` (component template renderer) |
| Detailed design | `generate_audio_argv` remains the canonical argv builder for the `vhost-user-sound-worker` component template. The resulting argv/env/executableRef are sealed into the LaunchTicket. The per-Guest binary copy path enforcement remains via the LaunchTicket verifier. The live Process resource spec contains no argv or executableRef. The `--socket` argument is removed; the vhost-user service identity is `Endpoint/corp-vm-audio-vhost-user`, while the backing locator is resolved into the LaunchTicket under authorization. Primary reuse disposition: `adapt`. Preserved source-plan detail: `adapt` - argv builder retained; becomes a signed component-template projection, not a live Process spec field. |
| Integration | The component template for `vhost-user-sound-worker` embeds the output of `generate_audio_argv`; the Process Provider resolves arg0 from the artifact catalog. |
| Data migration | No runtime state migration; argv template output is regenerated from the v3 component template, and live Process specs never store argv. |
| Validation | `tests/argv.rs`: rejection matrix (Nix store path, symlink, cross-guest copy, empty name); no-socket-in-argv assertion; no-argv-in-process-spec assertion |
| Removal proof | `d2b-host/src/audio_argv.rs` deleted after `d2bd` has no callers; confirmed by `cargo check -p d2b-host`. |

### ADR046-audio-004: Implement AudioMediator SetGrant/SetLevel service and libpipewire enforcement

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-004` |
| Dependency/owner | Depends on `ADR046-audio-001`; ComponentSession service contract; libpipewire 1.x API |
| Current source | `packages/d2bd/src/audio_host_controller.rs` `PipeWireHostController` enforcement logic; `QemuAudioController` |
| Reuse source | Same baseline paths |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-audio-pipewire/src/mediator/enforcement.rs` |
| Detailed design | Owner AudioService only: speaker `SetGrant` maps `"off"` to `pw_node_set_param(SPA_PARAM_Props, mute=true, target.object=-1)` on the worker's node and `"on"` to `mute=false`; `SetLevel` maps to a bounded volume. Microphone `"on"` is applied only after the owner authority grants its single capture slot; queued/blocked consumers remain muted, and release mutes/disconnects before handoff. `captureAlias` resolves privately through the registry. A projection AudioService routes the operation to the remote owner over its import stream and is denied any local mediator/PipeWire open. `FakeAudioMediator` is the hermetic test double. No state file, wpctl, EphemeralProcess, or node ID in any external surface. Primary reuse disposition: `adapt`. Preserved source-plan detail: `adapt` - enforcement logic becomes a libpipewire API implementation behind the `SetGrant`/`SetLevel` ComponentSession service. |
| Integration | AudioBinding controller calls its resolved AudioService. Owner Service dispatches locally to AudioMediator; projection Service dispatches over the encrypted import route to the remote owner. |
| Data migration | No state migration; mediator applies current AudioBinding grants and levels from resource state during reconcile, replacing host-controller direct writes. |
| Validation | `tests/mediator.rs` and `tests/enforcement.rs`: owner-Service SetGrant/SetLevel round-trip; speaker mixing; microphone queued consumers remain muted; release/lease-expiry mute-before-handoff and no-overlap proof; projection routing with fake streams; projection-PipeWire-open denial; no-node-id-in-bus-message; ProviderSessionUnavailable; captureAlias registry resolution |
| Removal proof | `d2bd/src/audio_host_controller.rs` retired after `d2bd` audio dispatch path is replaced. |

### ADR046-audio-005: Implement `AudioService` and `AudioBinding` schemas and admission

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-005` |
| Dependency/owner | ADR046-provider-004 common audio Service/Binding base; Core resource-api foundation; `d2b-provider-audio-pipewire` crate |
| Current source | None (ADR-only); structured after `d2b-contracts/src/public_wire.rs` audio types |
| Reuse source | `public_wire.rs` `AudioChannel`, `AudioEnforcementPosture`, `AudioErrorKind`, `AudioProviderKind`, `AudioSetApplied`; `AudioProviderKind` is removal evidence, not a base-status field |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-audio-pipewire/src/{resource_type,admission,provider_extension}.rs` (strict implementation extensions and binding only; common base lives under ADR046-provider-004) |
| Detailed design | Bind the shared D098 `audio.d2bus.org.AudioService` and `audio.d2bus.org.AudioBinding` base schema versions/fingerprints from ADR046-provider-004 and define only strict audio-pipewire Provider extensions/admission. AudioService validates immutable `serviceRole`, same-Zone local Endpoint refs, owner-only D097 AuthorityDescriptor, projection-only `ownerRef: ResourceImport/<name>`, Core-only projection creation, and projection `spec.provider` rejection. Initial-v3 owner admission requires speaker `multiplexed` and microphone `exclusive` and rejects multiplexed capture plus every consent/approval/priority placeholder. Owner Service status carries only bounded aggregate microphone active/request/Zone counts; Binding status carries only its own arbitration state. AudioBinding validates Guest ownership, required immutable same-Zone `serviceRef`, grants/levels/users, and forbids authority/export/projection semantics. Semantic authority/import/attachment observations stay under `status.resource`; implementation observations stay under `status.provider`. PipeWire fields are rejected from base spec/status. Register no provider-qualified or AudioState identifier and no serde/schema alias. Primary reuse disposition: `adapt`. Preserved source-plan detail: `adapt` provider-neutral closed enums; remove `AudioProviderKind` from base status; `ADR-only` for schema/admission. |
| Integration | `Provider/audio-pipewire` signs and publishes implementation support for both neutral qualified ResourceTypeSchemas and strict provider-envelope schemas. Core import controller may create/delete only projection AudioService; ordinary resource API admission handles owner Services and AudioBindings. |
| Data migration | Full d2b 3.0 reset; owner Services and per-Guest AudioBindings are authored as new v3 resources; projection Services are core-generated from ResourceImport. |
| Validation | `tests/resource_type.rs`: consume the ADR046-provider-004 common fixtures/fingerprints; canonical minimal base without `spec.provider`; neutral qualified-name registration; both schema/status round-trips including bounded aggregate owner queue status and per-Binding arbitration state; clean-break rejection of provider-qualified names, every AudioState spelling, and all aliases; fake alternate-provider base conformance; strict base/provider unknown-field matrices; projection `spec.provider` rejection; D088 `status.resource`/`status.provider` placement; PipeWire fields only in strict provider envelopes/config; Service role/AuthorityDescriptor/ownerRef/Endpoint-locality rules; initial-v3 exclusive-mic/mixed-speaker schema and consent/approval/priority/concurrent-capture rejection; Core-only projection admission; AudioBinding required same-Zone serviceRef and Guest owner; immutable refs; out-of-range levels/users; explicit tests that AudioBinding cannot be exported or projected |
| Removal proof | None - both ResourceTypes are net-new |

### ADR046-audio-006: Implement deterministic AudioService and AudioBinding handlers

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-006` |
| Dependency/owner | Depends on `ADR046-audio-001` through `ADR046-audio-005`; core ResourceImport controller; system Process Provider; AudioMediator (`ADR046-audio-007`); GuestAudioAgent (`ADR046-audio-011`); no Provider state Volume under D087 |
| Current source | `packages/d2bd/src/audio_dispatch.rs` lines 250-end (dispatch ordering reference) |
| Reuse source | None directly; reconcile flow is new async controller |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-audio-pipewire/src/controller/audio_service.rs`; `src/controller/audio_binding.rs` |
| Detailed design | One controller binary registers deterministic handlers for the two neutral ResourceTypes, constrained to immutable `spec.providerRef: Provider/audio-pipewire`. Service handler watches AudioService, its ResourceImport owner and local Endpoints; owner semantics claim/revalidate D097, local mediator, and aggregate exclusive-mic queue state, while projection semantics bind only encrypted import streams and deny PipeWire. It cannot create/delete projection Service. Binding handler watches AudioBinding, same-Zone serviceRef, Guest/User, owned Process, and private Endpoints; creates the vhost-user worker, GuestAudioAgents, and private Endpoints, then calls the resolved Service and guest agents. It maps `Applied|Queued|MicQueueFull` into the closed conditions/status/phase, and release/delete/revocation cancels queue state before child teardown. A Service event enqueues serviceRef-indexed Bindings. A resource selecting another conforming Provider is ignored and cannot be status/finalizer-mutated. Neither handler uses broker/pidfd/EphemeralProcess/Volume/User operations or direct filesystem access. Primary reuse disposition: `adapt`. Preserved source-plan detail: `adapt` - dispatch logic is the reference for step ordering only. |
| Integration | Registered with Zone core as a controller under `Provider/audio-pipewire`. |
| Data migration | v1/v2 audio policy file migration is handled by ADR046-audio-001 before reconcile; the controller keeps no Provider state Volume and imports no additional runtime state. |
| Validation | Fast hermetic `tests/audio_service_controller.rs`: neutral type/provider selection, foreign-provider ignore/deny, owner authority, bounded aggregate microphone status, projection ownerRef/import chain, core-only create/delete, projection no-PipeWire-open, revocation queue cancellation, and D091 propagation. `tests/audio_binding_controller.rs`: neutral type/provider selection, required same-Zone serviceRef, owner/projection dispatch, child Process/private Endpoint state machine, `Applied|Queued|MicQueueFull` status mapping, off/delete/revocation cancellation, grant changes, absence/failures/deletion. Conformance asserts no AudioBinding export/projection, no broker/pidfd/EphemeralProcess/Volume/User ops. ProviderDeployment integration remains fake-only and validates empty ProviderStateSet. |
| Removal proof | Supersedes `audio_dispatch.rs`; `d2bd` audio dispatch deleted after e2e parity test confirms |

### ADR046-audio-007: Implement `AudioMediator` user-session service

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-007` |
| Dependency/owner | Depends on `ADR046-audio-004`; ComponentSession service (ADR-046-componentsession-and-bus); libpipewire 1.x |
| Current source | `packages/d2bd/src/audio_host_controller.rs` PipeWire session access patterns (reference only) |
| Reuse source | None |
| Reuse action | create |
| Destination | `packages/d2b-provider-audio-pipewire/src/mediator/mod.rs`; `src/bin/audio_pipewire_mediator.rs` |
| Detailed design | Owner AudioService implementation only. Long-lived user-session Process maintains per-AudioBinding nodes under the single owner backing, receives the pre-opened local PipeWire portal FD, and exposes `SetGrant`/`SetLevel` through `Endpoint/audio-pipewire-authority`. It enforces the authority arbiter's single microphone slot and mute-before-handoff result while speaker nodes remain mixed. Projection Services never start/call a local mediator and cannot receive its FD. No EphemeralProcess, wpctl, remote Ref, or node identity in external surfaces. |
| Integration | Second binary in the `d2b-provider-audio-pipewire` package. Registered as a user-session service under `Provider/audio-pipewire`. |
| Data migration | No persisted mediator state migration; the service rebuilds its PipeWire node map from the registry on start and consumes current AudioBinding through controller calls. |
| Validation | `tests/mediator.rs`: owner-Service FD handoff and calls; captureAlias; node-id sealing; session-unavailable; concurrent speaker Guest isolation; exclusive microphone no-overlap and mute-before-handoff; teardown; projection Service cannot resolve mediator Endpoint or portal attachment |
| Removal proof | Supersedes `d2bd`'s `PipeWireHostController` direct session access; `d2bd` audio host controller deleted after e2e parity |

### ADR046-audio-008: Nix module for v3 AudioService/AudioBinding authoring

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-008` |
| Dependency/owner | Depends on `ADR046-audio-005`; Nix resource compilation framework; `ADR-046-nix-configuration` |
| Current source | `nixos-modules/components/audio/host.nix` and `guest.nix` |
| Reuse source | Same |
| Reuse action | replace |
| Destination | `nixos-modules/components/audio/v3-resource.nix`; `nixos-modules/components/audio/host-config.nix`; `nixos-modules/components/audio/guest-config.nix` |
| Detailed design | `v3-resource.nix` emits only `audio.d2bus.org.AudioService` and `audio.d2bus.org.AudioBinding`, selected by `Provider/audio-pipewire`, with required same-Zone serviceRef. For projection identity, ResourceExport emits `resourceRef` plus canonical `serviceType`, `projectionSchemaFingerprint`, and `factoryFingerprint`; its Service retains the Endpoint. ResourceImport emits the matching three `expected*` fields. Projection Services are never authored: core materializes them from ResourceImport. Eval rejects every obsolete Export/Import field in the explicit rejection matrix, provider-qualified ResourceTypes, every AudioState spelling, aliases, PipeWire fields in neutral base spec/status, AudioBinding export/projection, non-exclusive microphone or consent/approval/priority/concurrent-capture fields, Service role/ownerRef/authority mismatches, cross-Zone Endpoint/service refs, and duplicate owner authority. Existing captureAlias stays in Provider config; guestUsers/group injection, runtime-audio derivation, host stream rules, and guest stack remain. |
| Integration | `nixos-modules/default.nix` imports all three modules. |
| Data migration | Full d2b 3.0 reset; legacy Nix audio options emit/deprecate to one owner AudioService plus per-Guest AudioBindings that reference it; no projection Service is authored. |
| Validation | `tests/unit/nix/cases/audio-v3-resource.nix`: exact neutral type names; provider-qualified/AudioState/alias rejection; strict provider-field placement; owner Service and Binding round-trip; same-Zone serviceRef; exact canonical Export/Import Service type and both fingerprints; obsolete Export/Import field rejection; Export Endpoint-field rejection; projection core-only ownerRef chain; AudioBinding export/projection rejection; exclusive-mic/mixed-speaker and consent/approval/priority/concurrent-capture rejection; authority uniqueness; Endpoint locality; plus existing grants/users/captureAlias/deprecation/no-wpctl/no-audioFrontend assertions |
| Removal proof | `host.nix` and `guest.nix` kept as compat shims until v3 module deployed on all Zones |

### ADR046-audio-009: Minijail contract test migration

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-009` |
| Dependency/owner | Depends on `ADR046-audio-005`; `d2b-contract-tests` crate |
| Current source | `packages/d2b-contract-tests/tests/minijail_audio_usbip.rs` audio section |
| Reuse source | Same |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-audio-pipewire/tests/minijail_contract.rs` (provider-local); retain cross-bundle source greps in `d2b-contract-tests` |
| Detailed design | Retain worker sandbox assertions and add role-sensitive attachment checks: an owner-Service worker may receive only the local AudioMediator attachment; a projection-Service worker receives only its same-Zone route Endpoint and can never receive a PipeWire FD. Binding-owned private Endpoint ownership/producerRef is explicit. All Service/Binding/Process/Endpoint serialized forms remain locator-free. |
| Integration | Provider-local contract tests run in `d2b-provider-audio-pipewire`; retained cross-bundle greps in `d2b-contract-tests` ensure bundle-wide invariants still hold. |
| Data migration | None - test migration only; no runtime state. |
| Validation | `cargo test -p d2b-provider-audio-pipewire -- minijail` must pass; existing cross-bundle tests must continue to pass |
| Removal proof | The superseded duplicate shell validator `tests/minijail-validator-audio.sh` is deleted only after the successor Rust gate (`minijail_audio_usbip.rs` cross-bundle + provider-local `minijail_contract.rs`) is green and its removal-proof check passes; the `seccomp_policy_ref == "w1-audio"` assertion migrates to `spec.sandbox.seccompClass == "audio-pipewire-worker"` before removal. Cross-bundle Rust tests are retained. |

### ADR046-audio-010: OTEL telemetry and audit emitters

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-010` |
| Dependency/owner | Depends on `ADR046-audio-006`, `ADR046-audio-007`, `ADR046-audio-011`; `d2b-telemetry` lightweight emitter |
| Current source | `packages/d2bd/src/audio_dispatch.rs` audit call sites (redaction pattern reference) |
| Reuse source | Same; adapt redaction pattern |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-audio-pipewire/src/telemetry.rs` |
| Detailed design | Emit closed-label Service and Binding metrics plus post-commit audit. Service events distinguish only `owner\|projection` and closed outcomes; microphone arbitration emits closed transition/outcome plus bounded aggregate counts only. Metrics expose active count `0|1`, aggregate queue depth, and closed arbitration state without Zone/Binding/handle/position labels. Events omit authority keys, import/export keys, remote identity, stream/session ids, endpoints, client timestamps, and queue entries. Enforcement metrics cover owner-local and projection-routed calls without exposing route identity. ProcessEffect audit remains Process Provider-owned. |
| Integration | Audio controller and mediator call telemetry/audit emitters after commit or enforcement; d2b-telemetry exporter and policy_observability consume the resulting records. |
| Data migration | No telemetry/audit data migration; v3 emits new closed-label OTEL/audit records after cutover and old audio_dispatch audit sites are removed. |
| Validation | `tests/audio_telemetry.rs`: Service/Binding event separation, microphone aggregate metrics and transition audit, redaction of Zone/Binding/handle/position/timestamp, post-commit ordering, label cardinality, forbidden authority/import/stream/path fields, no ProcessEffect duplication |
| Removal proof | `audio_dispatch.rs` audit call sites deleted after cutover |

### ADR046-audio-011: Implement `GuestAudioAgent` in-guest service component

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-011` |
| Dependency/owner | Depends on `ADR046-audio-004`; ComponentSession service contract; libpipewire 1.x; system-systemd Process Provider for guest domain |
| Current source | `packages/d2b-guestd/src/audio_set.rs` (guestd wpctl dispatch - reference only) |
| Reuse source | `packages/d2bd/src/audio_host_controller.rs` libpipewire enforcement patterns (reference only) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-audio-pipewire/src/guest_agent/mod.rs`; `src/guest_agent/enforcement.rs`; `src/bin/audio_pipewire_guest_agent.rs` |
| Detailed design | Long-lived user-domain Process running in the Guest under the guest workload user's UID. One Process resource per entry in `AudioBinding.spec.guestUsers`; each named by opaque UID digest (`ag-<digest>`) and carrying label `audio-pipewire.d2bus.org/role: guest-audio-agent`. `userRef` is the corresponding `User/<name>` Zone resource. Opens a PipeWire connection in the Guest's compositor session (same-UID, natural access). Exposes a typed `AudioSet` ComponentSession service through an owned `Endpoint/ag-<digest>-audio-set` (vsock transport, Guest→Zone d2b-bus). `AudioSet(mic, speaker, speakerLevel, micGain)` applies changes via libpipewire API (`pw_node_set_param` with `SPA_PARAM_Props`, `pw_stream_set_control`) on the guest virtio-snd PipeWire node. No wpctl binary, no command path, no EphemeralProcess. Controller calls ALL active GuestAudioAgent instances in parallel for each grant change and aggregates failures. `FakeGuestAudioAgent` is a test double behind `#[cfg(test)]`. Primary reuse disposition: `adapt`. Preserved source-plan detail: `ADR-only` (new component; supersedes guestd wpctl dispatch path). |
| Integration | Third binary in the `d2b-provider-audio-pipewire` package. Declared as GuestAudioAgent Process resources by the audio-binding-controller (one per guestUser; template: `guest-audio-agent`). System Process Provider (`Provider/system-systemd`) launches each inside the Guest under the respective guest workload user's UID. |
| Data migration | No guest runtime state migration; GuestAudioAgent reconnects to guest PipeWire and applies current AudioBinding grants and levels on reconcile, replacing guestd wpctl dispatch. |
| Validation | `tests/guest_agent.rs`: AudioSet service call → libpipewire apply; mute/route/level; session-unavailable path; reconnect state restore; no wpctl binary; no command path; N-agent creation (one per guestUser); parallel call and aggregated failure |
| Removal proof | `d2b-guestd` wpctl audio dispatch path deleted after all Guests have GuestAudioAgent deployed and e2e parity test passes |

### ADR046-audio-012: Cross-Zone audio export/import adapter (D096)

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-012` |
| Dependency/owner | ADR046-zone-control-019, ADR046-zone-control-020; audio Provider owner |
| Current source | None - net-new ADR 0046 cross-Zone sharing (D096) |
| Reuse source | audio authority/mediator service (this dossier); `packages/d2b-provider/src/share_adapter.rs` `ExportAdapter`/`ImportAdapter` traits |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-audio-pipewire/src/share_adapter.rs` |
| Detailed design | Implement signed `Provider/audio-pipewire` adapters only for canonical `serviceType: audio.d2bus.org.AudioService` when `spec.providerRef` selects this Provider. ResourceExport carries that `serviceType` plus the signed `projectionSchemaFingerprint` and semantic `factoryFingerprint`; ResourceImport carries the matching `expectedServiceType`, `expectedProjectionSchemaFingerprint`, and `expectedFactoryFingerprint`. Export adapter admits only the owner Service. Its local authority Endpoint remains Service-owned and is never an Export field. Core creates/deletes the projection AudioService with `ownerRef: ResourceImport/<name>`, `providerRef`, and semantic base/import fields but no `spec.provider`; routing derives from the signed local descriptor and ResourceImport record. The semantic factory fingerprint binds factory metadata plus projection-protocol version, never Provider/adapter identity, which the signed descriptor authenticates separately. The import adapter reconciles its semantic route and never creates AudioBinding or opens PipeWire. Per-Guest AudioBindings are ordinary consumer resources with same-Zone serviceRef. No provider-qualified type alias, FD/path/socket/remote Ref crosses a Zone. Primary reuse disposition: `adapt`. Preserved source-plan detail: net-new (implement the signed audio export/import adapter). |
| Integration | Core export/import controller (ADR046-zone-control-019); local projection lifecycle (ADR046-zone-control-020); ComponentSession bounded encrypted named streams |
| Data migration | None - full d2b 3.0 reset |
| Validation | Fast hermetic `tests/share_adapter.rs`: exact neutral AudioService `serviceType`; exact projection-schema/factory fingerprint match; explicit rejection of obsolete `endpointRef`, `exportedType`, `baseSchemaFingerprint`, `expectedType`, `expectedBaseSchemaFingerprint`, and `projectionType`; reject every Export Endpoint field, provider-qualified alias, and AudioBinding export/projection; accept owner AudioService export; Core-only projection creation/deletion; exact ResourceImport -> projection AudioService ownerRef chain with no `spec.provider`; semantic factory fingerprint unchanged by Provider/adapter identity mutation while signed identity authentication remains exact; projection never opens PipeWire; reconnect/revocation/D091 propagation with fake streams. Only `integration/real_stream.rs` exercises a real encrypted named stream. |
| Removal proof | Not applicable (new surface) |

### ADR046-audio-013: Audio authority service - speaker mixer and mic arbiter (D096/D097)

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-013` |
| Dependency/owner | Depends on `ADR046-audio-001`, `ADR046-audio-004`, `ADR046-zone-control-019`; audio Provider owner |
| Current source | `packages/d2bd/src/audio_host_controller.rs` (`HostAudioController` trait, `PipeWireHostController::{from_audio_node,find_audio_node}`, `QemuAudioController`, `FakeHostController`); `packages/d2b-core/src/audio_policy.rs` (`LevelPercent`, `AudioGrant`, `AudioPolicyState`) |
| Reuse source | Same baseline controller/policy symbols |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-audio-pipewire/src/authority.rs` (speaker mixer + mic arbiter); `AuthorityDescriptor` on owner `AudioService` |
| Detailed design | Exactly one owner AudioService holds the real PipeWire connection and D097 AuthorityDescriptor. Projection Services and AudioBindings cannot carry it. Speaker streams remain multiplexed/mixed. Microphone capture has exactly one slot across owner and importing Zones. The arbiter keys requests by authenticated Zone plus route-scoped opaque consumer handle, permits one pending entry per handle, bounds pending entries to 16 per Zone and 64 total, uses owner-sequenced FIFO per Zone and round-robin across non-empty Zones, and ignores client timestamps/priority. Its 30-second active lease renews only while no other Zone waits; contended expiry mutes/disconnects, dequeues the next Zone, and atomically requeues a still-requesting old holder at its Zone FIFO tail. Off/delete/revoke/disconnect cancel idempotently. Queue entries are memory-only and restart rebuild fails closed with capture muted. Multiplexed capture and consent/approval/priority/concurrent-capture surfaces are rejected; a future spec must define a concrete consent authorization ResourceType and resource-API verb before concurrent capture exists. Core rejects duplicate owner Services before open and adopts by ownerProof. No new ProcessRole, broker path, or state file. Primary reuse disposition: `adapt`. Preserved source-plan detail: `adapt` - the host controller becomes the single authority service; no daemon `Mutex`/state-file wrapping. |
| Integration | Owner AudioService references `Endpoint/audio-pipewire-authority`; same-Zone AudioBindings and remote projection Services call it. Core authority index admits exactly one owner Service. |
| Data migration | None - full d2b 3.0 reset; grants are authoritative in `AudioBinding.spec` (no state file). |
| Validation | Fast hermetic `tests/authority.rs`: AuthorityDescriptor accepted only on owner AudioService; Binding/projection rejection; duplicate conflict; multiplexed speaker mix/quota; one active mic across local/imported Zones; per-Zone FIFO and cross-Zone round-robin; one-entry-per-handle, per-Zone/total bounds and `MicQueueFull`; idempotent cancellation; contended 30-second lease; mute-before-handoff/no overlap; restart-muted rebuild; multiplexed-capture and consent/approval/priority/concurrent-verb rejection; ownerProof adoption; D091 drain/recycle with fake clock/FakeHostController |
| Removal proof | `audio_host_controller.rs` daemon-side controller deleted after the authority service reaches parity; confirmed by `cargo check`. |

### ADR046-audio-014: Per-import encrypted audio credit streams (D096)

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-014` |
| Dependency/owner | Depends on `ADR046-audio-013`, `ADR046-zone-control-019`; audio Provider owner |
| Current source | `packages/d2b-realm-core/src/stream.rs` (`StreamKind::{AudioPlayback,AudioCapture}` → `Capability::{AudioPlayback,AudioCapture}`, `StreamAuthz`, `StreamChannel` split-direction); `packages/d2b-realm-core/src/mux.rs` (credit-based flow); `packages/d2bd/src/guest_control_bridge.rs` (`audio_set_authenticated`/`audio_status_authenticated`, `GuestAudioSetRequest`/`GuestAudioStatus`) |
| Reuse source | Same baseline stream/mux/bridge symbols |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-audio-pipewire/src/streams.rs` |
| Detailed design | Per-import audio frames flow only over bounded encrypted named streams: one stream with two `StreamChannel`s for playback/capture split direction and a single `StreamAuthz` (a consumer never opens two authz contexts to split direction), credit-based backpressure (a sender spends only receiver-granted credit), per-import session generation, cancel, and deadline. `StreamKind::AudioPlayback`/`AudioCapture` require `Capability::AudioPlayback`/`AudioCapture`. Playback streams may run concurrently; the owner activates capture frames for only its single granted opaque consumer handle, and revocation/disconnect cancels that Zone's active/pending requests before route teardown. No PipeWire FD/socket crosses a Zone; intermediaries see ciphertext. Guest audio calls (`audio_set`/`audio_status`) are issued to all active guests concurrently and results aggregated. Volume/gain (`LevelPercent`), queue identity/position, and node identity are redacted from audit/OTEL/logs. Primary reuse disposition: `adapt`. Preserved source-plan detail: `adapt` - audio frames ride the existing ComponentSession named-stream credit machinery. |
| Integration | Owner and projection AudioService adapters allocate per-import streams over ComponentSession; AudioBinding only consumes the same-Zone Service Ref; core routes encrypted records only. |
| Data migration | None - full d2b 3.0 reset |
| Validation | Fast hermetic `tests/streams.rs`: projection-Service ownerRef/import binding, split-direction single-authz stream, credits, generation isolation, cancel/deadline, concurrent playback, one active capture across imports, route loss cancels that Zone's active/pending capture, ciphertext-only intermediary, redaction. Only `integration/real_stream.rs` runs the slower real encrypted stream. |
| Removal proof | Not applicable (new surface) |

## Required crate layout

```text
d2b-provider-audio-pipewire/
  README.md
  src/
    lib.rs
    audio_policy.rs         # AudioPolicyState DTOs (ADR046-audio-001)
    argv.rs                 # Component template argv builder (ADR046-audio-002)
    resource_type/
      mod.rs
      audio_service.rs      # AudioService owner/projection schema + status
      audio_binding.rs      # per-Guest AudioBinding schema + status/serviceRef
    admission.rs            # Resource API admission validation (ADR046-audio-005)
    authority.rs            # owner D097 speaker mixer + exclusive fair mic arbiter
    share_adapter.rs        # AudioService export/import semantics
    streams.rs              # projection encrypted named-stream route
    runtime_capability.rs   # runtime-audio capability query client
    telemetry.rs            # Metrics, post-commit audit, span attributes (ADR046-audio-010)
    controller/
      mod.rs
      audio_service.rs      # owner/projection Service handler
      audio_binding.rs      # audio-binding-controller handler (ADR046-audio-006)
    mediator/
      mod.rs                # AudioMediator service + SetGrant/SetLevel (ADR046-audio-007)
      enforcement.rs        # host libpipewire enforcement API (ADR046-audio-004)
    guest_agent/
      mod.rs                # GuestAudioAgent service + AudioSet (ADR046-audio-011)
      enforcement.rs        # guest libpipewire enforcement API (ADR046-audio-011)
    bin/
      audio_pipewire_controller.rs
      audio_pipewire_mediator.rs
      audio_pipewire_guest_agent.rs
  tests/
    audio_policy.rs         # AudioPolicyState / policy migration (ADR046-audio-001)
    argv.rs                 # Component template argv rejection matrix (ADR046-audio-002)
    resource_type.rs        # Both schemas/status + Service/Binding separation
    audio_service_controller.rs # owner/projection state machine
    audio_binding_controller.rs  # AudioBinding handler state machine (ADR046-audio-006)
    share_adapter.rs        # ownerRef chain/core-only projection/fake streams
    authority.rs            # D097 mixed speaker/exclusive bounded mic arbitration
    streams.rs              # fake encrypted-stream credit semantics
    mediator.rs             # FD handoff + SetGrant/SetLevel + captureAlias (ADR046-audio-007)
    enforcement.rs          # host libpipewire enforcement + offline path (ADR046-audio-004)
    guest_agent.rs          # AudioSet service + guest libpipewire (ADR046-audio-011)
    audio_telemetry.rs      # Redaction / label cardinality / post-commit (ADR046-audio-010)
    minijail_contract.rs    # Zero-caps / seccompClass / no-argv/env/executableRef (ADR046-audio-009)
  integration/
    README.md
    audio_e2e.rs            # End-to-end: enable guest, sidecar start, grant change, delete
    grant_enforcement.rs    # SetGrant/SetLevel + exclusive mic handoff round-trip
    guest_enforcement.rs    # GuestAudioAgent AudioSet service + libpipewire round-trip
    real_stream.rs          # only real encrypted named-stream cross-Zone test
  README.md
```

`src/`, `tests/`, `integration/`, and `README.md` are all required. Workspace
policy rejects a provider crate missing any of these paths.

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has an advisory wall-clock p95
diagnostic threshold of <=50 ms; gate enforcement is aggregate per-crate
process CPU only. There is no wall-clock
sleep, and `cargo test -p d2b-provider-audio-pipewire --lib --tests` completes
in ≤3 s warm-cache execution time (compilation excluded). They use a
deterministic fake clock/RNG and the toolkit fakes/FakeEffectPort only - no
process spawn, container, network, DBus, systemd, broker daemon, Nix eval/build,
KVM, USB/GPU/TPM hardware, or live cloud, and no filesystem tree beyond tiny
temp fixtures. Any scenario needing those lives only in `integration/`, which
keeps a lane timeout/budget, parallel isolation, and fake external services by
default; such a need is re-placed into `integration/`, never given a sleep,
larger timeout, or `#[ignore]`. Bounded crypto/property tests are the only
classified exception, each named with a capped case count and a declared higher
per-test advisory threshold.

The Service/Binding split is a mandatory fast hermetic matrix:

- `provider_neutral_type_registration`: only
  `audio.d2bus.org.AudioService` and `audio.d2bus.org.AudioBinding` register,
  with `Provider/audio-pipewire` as an implementation selected by
  `spec.providerRef`;
- `service_binding_separation`: only owner AudioService carries authority or is
  exportable; AudioBinding always has Guest owner + same-Zone serviceRef;
- `resource_type_name_clean_break`: provider-qualified Service/Binding and
  every AudioState ResourceType/serde/schema spelling are rejected and no alias
  is emitted;
- `provider_field_isolation`: PipeWire fields are rejected in neutral
  `spec.*` and `status.resource`, accepted only by the exact signed
  `spec.provider`/`status.provider` schema or Provider config, and unknown
  provider-envelope fields fail closed;
- `foreign_provider_ignored`: the PipeWire controller cannot reconcile or
  status/finalizer-mutate a neutral audio resource selected for another
  conforming Provider;
- `projection_ownerref_chain`: ResourceImport owns exactly one projection
  AudioService, and AudioBinding references that Service rather than the import;
- `canonical_export_import_fields`: Export and Import carry the exact
  Service/projection-schema/factory fields and matching fingerprints, an Export
  never carries an Endpoint, and the obsolete field matrix is rejected;
- `core_projection_scope`: core creates/deletes only projection AudioService;
- `projection_no_pipewire`: fake EffectPort proves no portal/PipeWire-open
  request can be emitted by a projection;
- `service_currency_propagation`: fake generations prove D091 ordering from
  remote owner to import, projection Service, and Binding;
- `exclusive_mic_fair_queue`: fake clock and authenticated local/imported Zones
  prove one active capture, fixed per-Zone/total bounds, per-Zone FIFO,
  cross-Zone round-robin, contended lease expiry, mute-before-handoff, restart
  fail-closed, and rejection of multiplexed/consent/priority surfaces while
  speaker streams remain concurrent.

These use fake stores, adapters, clocks, and streams in `tests/*.rs`. No
Service/Binding/ownerRef test is moved to integration. The only cross-Zone
transport case in `integration/` is `real_stream.rs`, which verifies actual
encrypted named-stream framing/credit behavior; all other stream semantics are
fast fake-stream tests.

### `integration/README.md` content requirements

The `integration/README.md` must document:

1. Required host environment: Linux x86_64, PipeWire user session running
   under the compositor user's UID, `vhost-device-sound` binary present,
   libpipewire 1.x with `pw_node_set_param` support;
2. Required guest environment for guest-enforcement tests: running Guest with
   virtio-snd device, PipeWire session, and GuestAudioAgent Process Ready;
3. How to run: `cargo test -p d2b-provider-audio-pipewire --test audio_e2e`;
4. How to skip PipeWire-dependent tests in CI without a live compositor
   session: `D2B_SKIP_PIPEWIRE_LIVE=1 cargo test ...`;
5. Fixture setup for the fake-bus hermetic path (fake AudioMediator service,
   fake GuestAudioAgent service, fake ComponentSession with fake
   `SetGrant` `Applied|Queued|MicQueueFull`, `SetLevel`, and `AudioSet`
   responses);
6. What `D2B_FIXTURES_FULL` provides for the minijail contract tests.
7. How to run `integration/real_stream.rs`, the only real cross-Zone stream
   test; Service/Binding separation and ownerRef-chain coverage remain hermetic.

### `README.md` content requirements

The crate `README.md` must document:

1. Provider identity: `Provider/audio-pipewire`;
2. Provider-neutral qualified ResourceTypes:
   `audio.d2bus.org.AudioService` and `audio.d2bus.org.AudioBinding`, initially
   implemented by `Provider/audio-pipewire`; no provider-qualified or
   AudioState aliases; only owner AudioService is exportable and only
   projection AudioService is core-generated for ResourceImport;
3. Controller and service components; one controller binary with deterministic
   Service/Binding handlers, one owner host mediator binary, one guest agent binary;
4. Worker template: `vhost-user-sound-worker`; guest service template: `guest-audio-agent`;
5. Nix authoring schema (verbatim example from this spec);
6. `Provider/audio-pipewire.spec.config` options (`captureAlias`; regex
   `^[a-z][a-z0-9-]*$`) and strict PipeWire field placement in
   `spec.provider`/`status.provider`;
7. Dependency chain: vhost-device-sound v0.3.0, PipeWire/WirePlumber,
   virtio-snd, libpipewire 1.x `pw_node_set_param`;
8. Security: zero capabilities, no PipeWire socket path in public surfaces,
   AudioMediator receives declared pre-opened portal FD from user supervisor
   (not ambient socket), FD routed via ProviderSupervisor to worker LaunchTicket,
   projection Service never opens PipeWire, AudioBinding always references a
   same-Zone Service and is never exported/projected, AudioBinding reconcile
   creates no Volume or User resources (`Provider/audio-pipewire`
   declares no Provider state Volume and its ProviderStateSet is empty), no broker
   process lifecycle ops, no EphemeralProcess for enforcement, no wpctl binary
   on host or guest, no runtime User.spec.groups mutation; ResourceExport uses
   only canonical Service/projection-factory fields and never carries the
   Service Endpoint; speakers remain multiplexed/mixed while microphone
   capture is exclusive with the fixed bounded fair queue across authenticated
   Zones; there is no consent/approval/priority override or concurrent capture
   until a future spec defines a concrete consent authorization ResourceType
   and verb;
9. Build: `cargo build -p d2b-provider-audio-pipewire`;
10. Test: `cargo test -p d2b-provider-audio-pipewire`;
11. Integration: see `integration/README.md`;
12. Standalone-repository consumption path.

## Removal schedule

| Artifact | Condition for removal |
| --- | --- |
| `nixos-modules/components/audio/host.nix` | After `v3-resource.nix` and `host-config.nix` deployed on all Zones and `make test-drift` passes |
| `nixos-modules/components/audio/guest.nix` | After `guest-config.nix` deployed and all Guests rebuilt with v3 module |
| `packages/d2bd/src/audio_dispatch.rs` | After `audio-binding-controller` passes e2e parity test and `d2bd` has no callers |
| `packages/d2bd/src/audio_host_controller.rs` | Same as `audio_dispatch.rs` |
| `packages/d2b-host/src/audio_argv.rs` | After `d2b-host` has no callers; confirmed by `cargo check -p d2b-host` |
| `packages/d2b-core/src/audio_policy.rs` | After no caller outside `d2b-provider-audio-pipewire` imports it; may remain as re-export shim |
| `d2b.site.audio.inputTargetNode` legacy option | After all Zones deployed v3 Provider; replaced by `Provider/audio-pipewire.spec.config.captureAlias` |
| `d2b.guestControl.wpctlPath` Nix option | Superseded by GuestAudioAgent libpipewire implementation; removed with guest.nix |
| guestd wpctl audio dispatch path | After all Guests have GuestAudioAgent deployed and e2e parity passes (ADR046-audio-011 removal proof) |
| `d2b.vms.<vm>.audio.*` legacy Nix options | After all configured VMs use `d2b.zones.<zone>.resources.<name>` authoring |

No removal is performed until the live successor is integrated, tested, and
confirmed by the removal proof listed in each work item. A current path is
never deleted speculatively.

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass -
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.
