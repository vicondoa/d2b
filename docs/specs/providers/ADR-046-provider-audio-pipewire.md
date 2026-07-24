# ADR 0046 Provider dossier: audio-pipewire

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-audio-pipewire` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 6 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-audio-pipewire` crate, `AudioState` controller, `AudioMediator` service component |
| Depends on | `ADR-046-resource-object-model`, `ADR-046-primitive-resource-composition`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-provider-model-and-packaging`, `ADR-046-components-processes-and-sandbox`, `ADR-046-resources-volume`, `ADR-046-provider-state`, `ADR-046-componentsession-and-bus`, `ADR-046-nix-configuration`, `ADR-046-telemetry-audit-and-support`, `ADR-046-resources-host-guest-process-user` |
| Supersedes | `nixos-modules/components/audio/host.nix`, `nixos-modules/components/audio/guest.nix`, `packages/d2b-core/src/audio_policy.rs`, `packages/d2bd/src/audio_dispatch.rs`, `packages/d2bd/src/audio_host_controller.rs`, `packages/d2b-host/src/audio_argv.rs` |

## Purpose

This spec exhaustively defines the `audio-pipewire` Provider for d2b 3.0. It
covers:

- Provider identity, crate layout, and package boundary;
- the `audio-pipewire.d2bus.org.AudioState` ResourceType: schema, status, lifecycle,
  validation;
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
  principals from the bounded pool — not runtime-created User resources;
- static components (controller, AudioMediator) created by core
  ProviderDeployment; controller creates only AudioState-owned Process children;
- RBAC, security invariants, and zero-broker-op controller boundary;
- async reconciliation, restart adoption, and status transitions;
- error codes and Degraded-state model;
- authoritative audit events and OTEL telemetry shape;
- Nix authoring, configuration, and resource compilation;
- implementation work items with exact source, destination, and tests;
- required crate layout (`src/`, `tests/`, `integration/`, `README.md`).

Audio is an **interaction Provider**, not a device Provider. It creates and
reconciles one `audio-pipewire.d2bus.org.AudioState` resource and one `Process` (vhost-user-sound
worker) per Guest that opts in.

**Controller boundary**: the `audio-state-controller` creates, updates, and
deletes `Process` resources exclusively through the resource API. It never
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
`Provider/audio-pipewire` is activated. The `audio-state-controller` creates
only AudioState-owned child Process resources (worker, GuestAudioAgent
instances); it does not bootstrap its own companion processes.

## Terminology mapping (baseline → v3)

All evidence citations use baseline symbol names. The v3 target name is
explicitly stated at each design boundary.

| Baseline name / location | v3 ADR 0046 target | Evidence class |
| --- | --- | --- |
| `AudioPolicyState` (`d2b-core/src/audio_policy.rs:130`) | `AudioState` spec (the `mic`/`speaker`/`speakerLevel`/`micGain` fields migrate to `AudioState.spec.grants`) | `implemented-and-reachable` |
| `AudioGrant::On/Off` (`audio_policy.rs:98`) | `AudioState.spec.grants.mic`/`speaker: "on"\|"off"` | `implemented-and-reachable` |
| `LevelPercent` (`audio_policy.rs:26`) | `AudioState.spec.grants.speakerLevel`/`micGain: 0..=100` | `implemented-and-reachable` |
| `parse_audio_state` / `to_v2_bytes` (`audio_policy.rs:282,215`) | Used only for baseline v1/v2 state-file migration on first activation; `AudioState.spec` is the durable desired authority in v3; no state file maintained after migration completes | `implemented-and-reachable` |
| `AudioArgvInput` / `generate_audio_argv` (`d2b-host/src/audio_argv.rs:47,101`) | signed component template for `vhost-user-sound-worker`; argv shape is a template projection, not a live Process spec field | `implemented-and-reachable` |
| per-VM binary copy path `/run/d2b/vms/<vm>/d2b-<vm>` (`audio_argv.rs:97`) | LaunchTicket verifier enforces the path shape against the component template; not exposed in the Process resource spec | `implemented-and-reachable` |
| `RunnerRole::Audio` (`d2b-contracts/src/broker_wire.rs:1524`) | `Process` resource with `spec.template: "vhost-user-sound-worker"`; launch owned by system Process Provider | `implemented-and-reachable` |
| `PipeWireHostController` (`d2bd/src/audio_host_controller.rs:85`) | `AudioMediator` user-session service; exposes `SetGrant`/`SetLevel` ComponentSession service; applies changes via libpipewire API | `implemented-and-reachable` |
| `QemuAudioController` (`audio_host_controller.rs:227`) | removed; audio discovers enforcement capability via the `runtime-audio` dependency alias; no implementation-ID branch in `audio-pipewire.d2bus.org.AudioState.spec` | `implemented-and-reachable` |
| `WPCTL_PATH` / `PW_DUMP_PATH` env keys (`audio_host_controller.rs:103`) | superseded; AudioMediator uses libpipewire registry introspection and direct API calls; no wpctl binary or pw-dump subprocess | `implemented-and-reachable` |
| `PIPEWIRE_RUNTIME_DIR` env key (`audio_host_controller.rs:105`) | not a Process spec field; AudioMediator receives a declared pre-opened PipeWire portal FD from the user supervisor/display portal — it does not open the socket from the ambient runtime environment | `implemented-and-reachable` |
| `access(2)` credential posture check (`audio_host_controller.rs:134`) | replaced by AudioMediator readiness check; AudioMediator reports `PipeWireSessionUnavailable` when the portal FD cannot be acquired from the user supervisor | `implemented-and-reachable` |
| `ofd_lock` / `acquire_audio_state_lock` (`audio_dispatch.rs:73,125`) | superseded; `AudioState.spec` is the durable desired authority; no state file is maintained; OFD lock is removed | `implemented-and-reachable` |
| `write_audio_state_unlocked` atomic rename (`audio_dispatch.rs:221`) | superseded; no state file; grants are authoritative in `AudioState.spec` | `implemented-and-reachable` |
| `AudioHostEnforcementKind` / `AudioGuestEnforcementKind` (`provider_capabilities.rs:21,39`) | superseded; enforcement capability is discovered at runtime via the `runtime-audio` manifest dependency alias; no implementation-ID branch in `AudioState.spec` | `implemented-and-reachable` |
| `AudioProviderCapability` capability row (`provider_capabilities.rs:54`) | inline component descriptor field; not a separate ResourceType | `implemented-and-reachable` |
| `AudioOp` / `AudioOpResponse` (`public_wire.rs:1934,2025`) | v3: `AudioState` spec mutations (`UpdateSpec`) via the resource API; no separate op wire | `implemented-and-reachable` |
| `AudioVmState` / `AudioChannelState` (`public_wire.rs:1955,1943`) | `AudioState.status.channels` inline status | `implemented-and-reachable` |
| `AudioEnforcementPosture` (`public_wire.rs:1848`) | `AudioState.status.enforcementPosture` | `implemented-and-reachable` |
| `AudioSetApplied` (`public_wire.rs:1997`) | `AudioState.status.lastSetApplied` | `implemented-and-reachable` |
| `AudioErrorKind` (`public_wire.rs:1870`) | `AudioState.status.outcome.code` closed enum | `implemented-and-reachable` |
| `AudioProviderKind` (`public_wire.rs:1889`) | `AudioState.status.providerKind` | `implemented-and-reachable` |
| WirePlumber `client.conf.d/90-d2b` stream rules (`nixos-modules/components/audio/host.nix:252`) | retained as host Nix config; not a resource spec field | `implemented-and-reachable` |
| WirePlumber `monitor.alsa.rules` (`nixos-modules/components/audio/guest.nix:197`) | retained as guest Nix config; not a resource spec field | `implemented-and-reachable` |
| `services.pipewire.extraConfig.client."90-d2b"` (`host.nix:252`) | Nix host-side `pipeWireClientConfig` option under `Provider/audio-pipewire.spec.config` | `implemented-and-reachable` |
| `d2b.site.audio.inputTargetNode` (`host.nix:253`) | `Provider/audio-pipewire.spec.config.captureAlias` — bounded named alias (`^[a-z][a-z0-9-]*$`) resolved privately by AudioMediator via libpipewire registry; not a PipeWire node ID or socket path | `generated-or-eval-contract` |
| `vhost-device-sound v0.3.0` (`pkgs/vhost-device-sound/default.nix`) | `spec.artifactId` in `Provider/audio-pipewire` pointing to Nix artifact catalog entry | `implemented-and-reachable` |
| `microvm.extraArgsScript` CH audio injection (`guest.nix:112`) | `Guest.spec.audioExtension` arguments derived from the runtime-audio capability reported by the Guest's Runtime Provider; values such as `virtio_id` and `queue_sizes` are not spec fields | `implemented-and-reachable` |
| `/var/lib/d2b/vms/<vm>/state/audio-state.json` (`host.nix:341`) | superseded; `audio-pipewire.d2bus.org.AudioState.spec` is the durable desired authority; no per-Guest state file in v3 | `implemented-and-reachable` |
| `/run/d2b/locks/audio-<vm>.lock` (`host.nix:366`) | superseded; OFD lock and state file are removed in v3 | `implemented-and-reachable` |
| `d2b-<vm>-snd` system user (`audio_argv.rs:140`) | superseded; dedicated worker principals are core Process principals from the bounded pool allocated by the Process Provider; not runtime-created `User` resources managed by the audio controller | `implemented-and-reachable` |
| `d2b.guestControl.wpctlPath` (`guest.nix:139`) | superseded; AudioMediator uses libpipewire API directly; no operator-visible wpctlPath option in v3 | `generated-or-eval-contract` |
| `d2b.audio.users` guest option (`guest.nix:92`) | `AudioState.spec.guestUsers` list of `User/<name>` ResourceRefs; Nix/compiler sets `spec.groups: ["audio"]` on each referenced guest `User` resource at compile time; runtime API-created AudioState verifies `User.status.groupMembershipVerified` before sidecar start; no runtime `extraGroups` mutation | `generated-or-eval-contract` |
| `minijail-profiles.nix` audio role block / `seccompPolicyRef = "w1-audio"` | `Process.spec.sandbox.seccompClass: audio-pipewire-worker` | `implemented-and-reachable` |
| `minijail_audio_usbip.rs` Layer-1 contract tests | retained and extended in `d2b-provider-audio-pipewire/tests/` | `implemented-and-reachable` |
| `d2b audio status/set-volume/mute` CLI ops (`packages/d2b/src/`) | v3: `d2b resource update audio-pipewire.d2bus.org.AudioState/<name>` or a provider-specific `d2b audio` projection | `implemented-and-reachable` |

## Resolved design decisions

All design decisions are resolved in this revision.

| ID | Question | Resolution |
| --- | --- | --- |
| DRAUDIO-001 | Separate `AudioState` ResourceType or extend `Device`? | `audio-pipewire.d2bus.org.AudioState` is an independent ResourceType. Audio is an interaction Provider; it does not model a Device inventory/arbitration/claim lifecycle. |
| DRAUDIO-002 | Per-Guest `AudioState` or Zone-global? | Per-Guest. Each Guest has independent grants, levels, and enforcement posture. |
| DRAUDIO-003 | Where does the vhost-user-sound socket path live? | Controller-generated private implementation detail. Never appears in `AudioState.spec`, `AudioState.status`, API responses, audit records, OTEL attributes, or any broker configuration. |
| DRAUDIO-004 | How does the controller enforce PipeWire stream routing after a grant change? | The `audio-state-controller` calls the AudioMediator `SetGrant`/`SetLevel` ComponentSession service over d2b-bus. The AudioMediator applies changes directly via libpipewire API (`pw_node_set_param`, `pw_stream_set_control`) on the worker's live PipeWire node. `AudioState.spec` is the durable authority; no prior state-file write is required. No EphemeralProcess, wpctl binary, or node ID is involved. |
| DRAUDIO-005 | How is the `application.name = "d2b-<guest>"` PipeWire stream identity established? | The component template for `vhost-user-sound-worker` is a signed LaunchTicket projection that sets the per-Guest binary copy path as argv[0]. `libpipewire`'s `init_prgname()` reads `/proc/self/exe`. This is a template projection; it does not appear in the live Process resource spec. |
| DRAUDIO-006 | WirePlumber stream rules: resource spec or host Nix config? | Host Nix config. The operator capture target is stored as `Provider/audio-pipewire.spec.config.captureAlias` — a bounded named alias (`^[a-z][a-z0-9-]*$`, ≤64 chars). The AudioMediator resolves it to the actual PipeWire node object via libpipewire registry introspection at runtime, privately. The alias never appears in ResourceSpec/status or public API surfaces. |
| DRAUDIO-007 | Guest PipeWire stack: resource spec or Nix guest config? | Guest Nix config. The in-guest virtio-snd module, PipeWire stack, WirePlumber virtio-snd profile, and diagnostic packages are Nix guest module concerns. |
| DRAUDIO-009 | Mic direction: null-target sentinel vs explicit routing? | WirePlumber stream rules in `client.conf.d/90-d2b` set initial stream-creation properties. Live grant changes are applied by the AudioMediator via libpipewire API calls on the worker's active PipeWire node. No worker restart required. `AudioState.spec` is the durable authority; no state file is written before or after enforcement. |
| DRAUDIO-011 | How does the vhost-user-sound worker access PipeWire without ambient socket exposure? | A same-UID user-session `AudioMediator` receives a declared pre-opened PipeWire portal FD from the user supervisor/display portal (not from the ambient runtime environment). The controller requests an operation-scoped typed attachment transfer; d2b-bus/ProviderSupervisor routes the FD directly mediator→worker LaunchTicket without the Process Provider knowing audio and without the controller handling FDs. No socket path, SetSocketAcl, or `PIPEWIRE_RUNTIME_DIR` env entry appears in any resource spec, status, broker config, or public surface. |
| DRAUDIO-012 | Audio user group membership: resource spec or Nix guest config? | `AudioState.spec.guestUsers` is a list of `User/<name>` ResourceRefs. For Nix/compiler-declared resources, the Nix module sets `spec.groups: ["audio"]` on each referenced guest `User` resource at compile time. For API-created `AudioState`, the operator sets `spec.groups` on the User resources; the controller verifies `User.status.groupMembershipVerified` before starting the sidecar and fails closed if not confirmed. The controller never mutates `User.spec.groups` at runtime. |
| DRAUDIO-013 | Who owns process launch, wait, reap, and pidfd? | `Provider/system-minijail` or `Provider/system-systemd` exclusively. The `audio-state-controller` creates and updates `Process` resource specs only. It never calls `SpawnRunner`, `OpenPidfd`, or adopts pidfds. ProcessEffect audit belongs to the Process Provider. |
| DRAUDIO-014 | Provider root configuration key? | `Provider.spec.config` (not `rootConfig`). The canonical Provider spec shape is `{artifactId; config}`. |
| DRAUDIO-015 | wpctl EphemeralProcess vs AudioMediator service? | wpctl EphemeralProcess is removed entirely. The AudioMediator exposes a `SetGrant`/`SetLevel` ComponentSession service; the controller calls it to apply enforcement. The AudioMediator uses the libpipewire API directly. No EphemeralProcess, no wpctl binary, no node ID in any resource spec or bus message. |
| DRAUDIO-016 | Guest-side enforcement: guestd wpctl path vs typed guest service? | guestd's wpctl dispatch path is superseded. A `GuestAudioAgent` Process running in the Guest under the guest workload user's UID exposes a typed `AudioSet` ComponentSession service over vsock. The `audio-state-controller` calls this service via libpipewire API. No wpctl binary or command path. The `d2b.guestControl.wpctlPath` Nix option is removed from v3. |
| DRAUDIO-017 | How does audio discover the Guest's audio frontend without an `audioFrontend.kind` spec field? | The Provider manifest declares a `runtime-audio` dependency alias bound to the Guest's Runtime Provider. At activation the Runtime Provider advertises typed `AudioCapability` records (e.g., `VhostUserSound { virtio_id, queue_sizes, enforcement_posture }`) via the capability protocol. The controller reads these records via the dependency alias. No implementation-ID branch appears in `AudioState.spec`; `Guest.spec.audioExtension` arguments are derived from the capability. If the runtime advertises no audio capability, sidecar is not deployed. |
| DRAUDIO-018 | Dedicated worker principals: controller-created User resources or core Process principals? | Core Process principals from the bounded pool allocated per provider by the Process Provider. The audio controller does not create `User` resources for worker execution identity. Human guest `User/<name>` references in `guestUsers` are observed from system-core, never created or modified by the audio controller. |
| DRAUDIO-019 | Who creates the static AudioMediator and controller Process resources? | Core ProviderDeployment creates them when `Provider/audio-pipewire` is activated. The `audio-state-controller` creates only AudioState-owned children (worker Processes, GuestAudioAgent Processes). The `allowedResourceVerbs` for the controller does not include `Volume` or `User`. |
| DRAUDIO-020 | How is the PipeWire FD routed from AudioMediator to worker without controller or Process Provider involvement? | The controller declares an operation-scoped typed attachment transfer when creating the worker Process resource. d2b-bus/ProviderSupervisor resolves the AudioMediator's active portal FD and delivers it in the worker's LaunchTicket. The Process Provider (system-minijail) receives and inherits the FD without knowing it is audio-specific. The controller never holds or transfers FDs directly. |

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
  one exported ResourceType (`audio-pipewire.d2bus.org.AudioState`)
- **Depends on**: public neutral contracts/toolkit crates only; no `d2bd`,
  `d2b-priv-broker`, Zone-store, or other Provider internals

### Controller components

| Component | Binary | Class | Domain | Scope |
| --- | --- | --- | --- | --- |
| `audio-state-controller` | `audio-pipewire-controller` | controller | system | Watches `audio-pipewire.d2bus.org.AudioState`; creates/updates/deletes AudioState-owned Process resources only; calls AudioMediator `SetGrant`/`SetLevel` service; never touches pidfds, broker spawn, Volume, or User resources; `Provider/audio-pipewire` declares no Provider state Volume under D087 |
| `audio-mediator` | `audio-pipewire-mediator` | service | user | Same-UID user-session component; receives declared pre-opened PipeWire portal FD from user supervisor; ProviderSupervisor routes FD to worker LaunchTicket; exposes `SetGrant`/`SetLevel` service; applies enforcement via libpipewire API |

The controller runs as a Process in the system domain under the Host. The
mediator runs as a Process in the user domain under the compositor user's UID.
**Both are static components created by core ProviderDeployment** when
`Provider/audio-pipewire` is activated; the controller does not bootstrap
them. Neither component receives a Zone store handle or a broker socket.

**Process lifecycle boundary**: the `audio-state-controller` creates `Process`
resources via the resource API and watches `Process.status` changes delivered
by the Process Provider. It never calls `SpawnRunner`, `OpenPidfd`, `SIGTERM`,
or any pidfd operation. Those belong exclusively to `Provider/system-minijail`
or `Provider/system-systemd`.

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

No audio capability field is stored in `AudioState.spec`. No
implementation-ID branch (cloud-hypervisor vs. qemu) appears in the spec or
the controller's reconcile logic — capability presence/absence and the typed
capability fields are the only dispatch surface.

### Worker process template

| Template name | Class | Domain | Role |
| --- | --- | --- | --- |
| `vhost-user-sound-worker` | worker | system | Per-Guest vhost-device-sound sidecar; long-lived; system-domain under `Provider/system-minijail`; receives PipeWire FD from AudioMediator via component-descriptor attachment; no user session needed |

## `AudioState` ResourceType

### Envelope example

```yaml
apiVersion: resources.d2bus.org/v3
type: audio-pipewire.d2bus.org.AudioState
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
  createdAt: 2026-07-22T00:00:00Z
  updatedAt: 2026-07-22T00:01:00Z
spec:
  providerRef: Provider/audio-pipewire
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
    - type: SidecarRunning
      status: "True"
      reason: vhost-user-sound-healthy
    - type: GuestAttached
      status: "True"
      reason: virtio-snd-attached
    - type: GrantsEnforced
      status: "True"
      reason: mediator-applied
    - type: MediatorReady
      status: "True"
      reason: pipewire-portal-open
  lastReconciledAt: 2026-07-22T00:01:01Z
  channels:
    speaker:
      grant: "on"
      level: 75
      liveEnforced: true
    mic:
      grant: "off"
      gain: null
      liveEnforced: true
  enforcementPosture: HostAndGuest
  lastSetApplied: HostAndGuest
  sidecarRef: Process/corp-vm-audio-sidecar
  outcome:
    code: ok
    exitCode: null
    message: null
    retryable: false
```

### `AudioState.spec` fields

Per D089, `AudioState`'s typed desired spec is the ResourceType base spec
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
| `providerRef` | ResourceRef | yes | — | `Provider/audio-pipewire` | Must resolve to the installed Provider; immutable after creation |
| `grants.mic` | enum | yes | — | `"on"` \| `"off"` | Microphone grant |
| `grants.speaker` | enum | yes | — | `"on"` \| `"off"` | Speaker grant |
| `grants.speakerLevel` | uint \| null | no | `null` | `0..=100` | Speaker volume percent; null = system default |
| `grants.micGain` | uint \| null | no | `null` | `0..=100` | Microphone input gain percent; null = system default |
| `guestUsers` | list[ResourceRef] | no | `[]` | ≤16 entries; each `User/<name>` where name matches `[a-z][a-z0-9_-]*` ≤32 chars | Guest User ResourceRefs; corresponding User resources must exist; group membership verified by controller before sidecar start |
| `suspendOnGuestAbsent` | bool | no | `true` | — | When `true` the sidecar Process is not started/is stopped when the owning Guest is not Running |

Audio frontend parameters (virtio device ID, queue sizes) are not `AudioState`
spec fields. They are derived at runtime from the `runtime-audio` capability
record advertised by the Guest's Runtime Provider (see "runtime-audio
dependency alias" above).

Schema validation is strict: unknown fields are rejected at resource API
admission time. Level and gain values are validated in `[0,100]`; null
sentinels are preserved and serialized as JSON `null`, never as absent fields.
`providerRef` is immutable after creation; a mutation attempt returns
`FieldImmutable`. Each `guestUsers` entry is validated as a syntactically
correct `User/<name>` ResourceRef; referential existence is validated at
runtime.

### `AudioState.status` fields

Per D088, `AudioState.status` is layered: universal `ResourceStatus` fields
(`observedGeneration`, `phase`, `conditions`, timestamps, and `outcome`) remain
at top-level `status.*`, while the typed audio fields below are the
ResourceType-common `status.resource` object for
`audio-pipewire.d2bus.org.AudioState`. Optional `status.provider` carries only
implementation-only observation (`providerRef`, qualified immutable `schemaId`,
semver `schemaVersion`, numeric `observedProviderGeneration`, strict
unknown-field-denied redacted `details` ≤32 KiB registered/signed in the
Provider manifest); shared fields are never duplicated there. The controller
writes all present layers atomically in one status mutation.

D091 currency and upgrade: the audio-pipewire controller implements
`assess_update`, `plan_upgrade`, and `execute_upgrade` for its qualified
ResourceTypes and semantic audio sessions. A `ProviderGenerationChanged`,
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
| `channels.speaker.grant` | `"on"\|"off"` | Last observed speaker grant |
| `channels.speaker.level` | uint \| null | Last observed speaker level |
| `channels.speaker.liveEnforced` | bool | True when confirmed by successful AudioMediator `SetGrant`/`SetLevel` service call this reconcile |
| `channels.mic.grant` | `"on"\|"off"` | Last observed mic grant |
| `channels.mic.gain` | uint \| null | Last observed mic gain |
| `channels.mic.liveEnforced` | bool | True when confirmed via AudioMediator service this reconcile |
| `enforcementPosture` | enum | `HostAndGuest\|HostOnly\|GuestOnly\|None` |
| `lastSetApplied` | enum | `HostAndGuest\|HostOnly\|GuestOnly\|OfflineOnly` |
| `sidecarRef` | ResourceRef \| null | Reference to the owned `Process` resource |
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
| `SidecarRunning` | The owned vhost-user-sound `Process` is Ready per the Process Provider |
| `GuestAttached` | The virtio-snd argument is present in the Guest command-line extension (derived from runtime-audio capability) |
| `GrantsEnforced` | Last `SetGrant`/`SetLevel` service calls completed with `liveEnforced: true` on all active channels |
| `GrantEnforcementFailed` | AudioMediator `SetGrant`/`SetLevel` service returned an error |
| `MediatorReady` | AudioMediator service Process is Ready and holds an open PipeWire portal FD |
| `MicNullTargeted` | Mic stream is routed to `-1` (blocked) via PipeWire node property |
| `SpeakerNullTargeted` | Speaker stream is routed to `-1` (blocked) |
| `PipeWireSessionUnavailable` | AudioMediator cannot acquire a PipeWire portal FD; user supervisor may not have one ready |
| `GuestAbsent` | Guest is not Running and `suspendOnGuestAbsent: true`; sidecar is intentionally stopped |
| `GuestAudioAgentReady` | All GuestAudioAgent Processes for this AudioState are Ready; guest-side AudioSet service available |
| `GuestUserAudioGroupMissing` | One or more `guestUsers` User refs do not have `User.status.groupMembershipVerified: true` for the `audio` group |
| `RuntimeCapabilityUnavailable` | The `runtime-audio` dependency alias returned no audio capability; sidecar not deployed |

### Phase semantics

| Phase | Meaning |
| --- | --- |
| `Pending` | `audio-pipewire.d2bus.org.AudioState` committed; sidecar Process not yet Ready, or Guest not yet Running, or runtime-audio capability not yet advertised |
| `Ready` | Sidecar running, Guest attached, last grants enforced, MediatorReady |
| `Degraded` | Sidecar running but enforcement failed; or Guest temporarily absent; or AudioMediator transiently unavailable |
| `Failed` | Sidecar Process exited unrecoverably; or runtime audio capability permanently absent |
| `Unknown` | Controller cannot currently observe Process or Guest status |

## Host worker process

### Architecture

The host worker is a long-lived `vhost-user-sound` sidecar
(`vhost-device-sound --backend pipewire`). It connects to the host
compositor's PipeWire session using a **pre-opened connected PipeWire FD**
received via the operation-scoped typed attachment transfer routed by
d2b-bus/ProviderSupervisor from the AudioMediator (see AudioMediator section).
It exposes a `vhost-user` server as an owned `Endpoint` resource. The backing
Unix socket locator is a controller-private sealed LaunchTicket value. The owning Guest attaches it via the
`Guest.spec.audioExtension` arguments derived from the `runtime-audio`
capability (e.g., `--generic-vhost-user socket=<sealed_path>,virtio_id=25,...`
for Cloud Hypervisor). No `audioFrontend` spec field is involved.

One worker Process per Guest. The `audio-state-controller` creates and owns
the Process resource. The system Process Provider (`Provider/system-minijail`)
launches, supervises, and reaps the worker and owns all ProcessEffect audit
records. The worker's execution principal is a core Process principal from the
bounded pool — not a controller-created `User` resource.

### Process resource

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: corp-vm-audio-sidecar
  zone: dev
  ownerRef: audio-pipewire.d2bus.org.AudioState/corp-vm-audio
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
    backoffMultiplier: 2.0
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

- **No `executableRef`, `argv`, `env`** — these are signed component-template
  projections in the LaunchTicket. The Process Provider resolves them from the
  compiled `vhost-user-sound-worker` template. The live Process resource spec
  stored in the Zone store contains no executable path, argument list, or
  environment.
- **No `inherited-fd` endpoint in the Process spec** — the PipeWire FD
  attachment from the AudioMediator is declared in the component descriptor
  (the private signed template), not the live resource spec. d2b-bus/
  ProviderSupervisor routes the FD from the AudioMediator to the worker
  LaunchTicket at launch time, without the Process Provider knowing audio and
  without the controller handling FDs.
- **No `mounts` block in the live Process spec** — the worker receives its
  configuration via the sealed component descriptor and the operation-scoped FD
  transfer. `AudioState.spec` is the durable desired authority for grants/levels;
  no application state file is written. The worker declares no Provider state
  Volume; bounded non-secret observations are stored in `AudioState.status`, the
  Provider status subresource where applicable, and the core Operation ledger.
- `domain: system` — the worker runs in the system domain. The execution
  principal is a core Process principal from the bounded pool allocated by the
  Process Provider; it is not a controller-created `User` resource and does not
  appear in the live Process spec. No `userRef` field is set.
- `sandbox.namespaceClasses: [mount, pid, ipc, uts]` — network namespace is
  **not** in this list because the worker receives the PipeWire FD as an
  inherited descriptor from the component descriptor attachment.
- `sandbox.capabilityClasses: []` — zero host capabilities; load-bearing
  invariant; see security section.
- `sandbox.seccompClass: audio-pipewire-worker` — the Process Provider
  resolves the exact seccomp profile from the compiled Process template.
  Maps to the `w1-audio` seccomp policy in the baseline minijail profile table.
- `sandbox.startRoot: false` — the Process Provider must not elevate to root
  before exec.
- `Endpoint/corp-vm-audio-vhost-user` is the owned service identity for the
  vhost-user server. The Process Provider creates the backing Unix socket before
  exec and seals its locator into the LaunchTicket; the locator never appears in
  resource spec or status.
- `budget` uses the canonical nested `cpu`/`memory`/`pids`/`fds` shape.
  `pids` and `fds` use the `{limit: N}` object form (not a bare scalar).
- `restartPolicy.class: on-failure` — canonical class name.
  `backoffBase`/`backoffMax` are duration strings; `backoffMultiplier` is the
  exponential factor; `maxRestarts` is the per-launch-cycle ceiling; `resetAfter`
  resets the counter if the process stays Running for this duration.
- `readiness.class: provider-defined` — the `vhost-user-sound-worker` template
  declares a provider-defined readiness mechanism (vhost-user socket ready).
  Fields: `initialDelay`, `timeout`, `failureThreshold`, `successThreshold`.

**ProcessEffect audit**: all process-launch, signal, and exit audit records
for this Process are owned by `Provider/system-minijail`. The
`audio-state-controller` emits only resource-level `AudioState` audit events.

### Guest command-line extension

The `audio-state-controller` mutates `Guest.spec.audioExtension` to add the
runtime's audio arguments. The exact arguments (socket path, device ID, queue
sizes) are derived from two sealed sources:

- The vhost-user socket path is derived from `AudioState.metadata.uid` by the
  Process Provider and embedded in the LaunchTicket; it is never stored in any
  public field.
- Device parameters (`virtio_id`, `queue_sizes`, runtime-specific flags) are
  read from the `runtime-audio` capability record at reconcile time and applied
  as the extension; they are not `AudioState.spec` fields.

The mutation is conditional: when the runtime advertises no audio capability,
or when both grants are `"off"`, the controller removes `audioExtension` from
the Guest spec.

## AudioMediator user-session service

### Purpose and identity

The `AudioMediator` is a same-UID user-session `Process` resource that runs
under the compositor user's UID. Its responsibilities are:

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
4. Expose a `SetGrant`/`SetLevel` typed ComponentSession service over d2b-bus.
   When the controller calls `SetGrant(channel, value)` or
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
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
  readiness:
    initialDelay: "0s"
    timeout: "10s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined
```

**No `executableRef`, `argv`, `env`** — these are component-template
projections. The live Process resource spec stored in the Zone store contains
only the fields shown above. `budget`, `restartPolicy`, and `readiness` use the
same canonical field names as the worker Process spec. `readiness.class:
provider-defined` — the `audio-mediator-service` template declares its own
readiness mechanism (ComponentSession endpoint accepting connections).

### FD routing via ProviderSupervisor

The controller declares an **operation-scoped typed attachment transfer** on
the worker Process resource at creation time. The transfer descriptor names
the AudioMediator as the FD source and the worker as the FD destination,
without specifying audio details:

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

### `SetGrant` and `SetLevel` service

The AudioMediator exposes a `SetGrant`/`SetLevel` typed ComponentSession service
on `Endpoint/audio-pipewire-service`. The controller calls this service for
every grant or level change on the cloud-hypervisor path.

Service interface (conceptual):

```text
SetGrant(guestRef: ResourceRef, channel: "mic"|"speaker", value: "on"|"off")
  → Ok | Error(code: GrantEnforcementFailed | PipeWireSessionUnavailable | ...)

SetLevel(guestRef: ResourceRef, channel: "mic"|"speaker", valuePercent: u8)
  → Ok | Error(code: GrantEnforcementFailed | ...)
```

The AudioMediator applies the change via:
- `pw_node_set_param` with `SPA_PARAM_Props` to update `mute` or routing on
  the worker's virtual device node;
- `pw_stream_set_control` on any active stream attached to the node;
- WirePlumber session policy enforcement through the node properties already
  set by the component template's initial stream configuration.

No node ID, PipeWire object path, wpctl binary, or any external process is
involved. The service call is synchronous within the AudioMediator; the
controller receives the result before updating `AudioState.status`.

No node ID, node path, or PipeWire runtime directory path appears in any
service request, response, d2b-bus message, audit record, or log entry.

### captureAlias resolution

When `Provider/audio-pipewire.spec.config.captureAlias` is non-null, the
AudioMediator resolves it at grant-change time via libpipewire registry
introspection. It iterates the PipeWire global object list and finds the node
whose `node.nick` or `node.name` matches the alias. The resolution is private
to the AudioMediator process. The resolved node object never leaves the mediator
— it is used in place directly for `pw_node_set_param` routing calls. No node
ID appears in any bus message, resource spec, status, audit record, or OTEL
attribute.

## Guest frontend

### Architecture

The guest frontend is a kernel virtio device driver (`snd_virtio`) and an
in-guest PipeWire stack. These are provisioned by the guest NixOS module
compiled into the Guest's NixOS configuration at Provider activation time.

Activated for any Guest that owns an `AudioState` resource, it installs:

- `boot.kernelModules: ["snd_virtio"]` — in-tree since 5.16;
- `services.pipewire.enable = true` with `alsa.enable`, `alsa.support32Bit`,
  and `pulse.enable` (PulseAudio compat layer);
- `security.rtkit.enable = true` — realtime priority for audio threads;
- WirePlumber `monitor.alsa.rules` override: `device.profile = "pro-audio"`
  and `api.alsa.use-acp = false` for the virtio-snd card; this is required
  because the virtio-snd ALSA driver has no ACP entry and WirePlumber defaults
  to `"Off"`, leaving no Sink or Source;
- `services.pulseaudio.enable = lib.mkForce false` — prevents PulseAudio
  collision;
- diagnostic packages: `pipewire`, `wireplumber`, `alsa-utils`.

### Guest `audio` group membership

**Nix/compiler path** (static configuration): when `AudioState` resources are
declared in the Nix configuration, the compiler sets `spec.groups: ["audio"]`
on the corresponding guest `User` resources for each name in
`AudioState.spec.guestUsers`. system-core verifies group membership
(`User.status.groupMembershipVerified`) and sets `GroupsVerified: True` when
confirmed.

**API-created AudioState** (runtime path): the controller checks
`User.status.conditions.GroupsVerified == True` for every name in
`spec.guestUsers` before starting the sidecar. If any User's audio group is
not confirmed, the controller sets `GuestUserAudioGroupMissing` condition and
phase becomes `Degraded`. The operator must update the User resource's
`spec.groups` to include `"audio"` and rebuild the guest to resolve this. The
controller does **not** mutate `User.spec.groups` at runtime.

### GuestAudioAgent and AudioSet service

Guest-side enforcement is performed by the **`GuestAudioAgent`** — a Process
resource running inside the Guest under the guest workload user's UID. It is
part of the audio-pipewire Provider's guest component set.

The `GuestAudioAgent`:
1. Opens a PipeWire connection in the Guest's compositor session (same-UID
   user domain, natural session access — no socket path or ambient ACL needed).
2. Exposes a typed `AudioSet` ComponentSession service over vsock (Guest→Zone
   d2b-bus transport).
3. When the `audio-state-controller` calls `AudioSet(mic, speaker, speakerLevel,
   micGain)`, the GuestAudioAgent applies changes directly via libpipewire API
   (`pw_node_set_param` with `SPA_PARAM_Props`, `pw_stream_set_control`) on the
   guest virtio-snd PipeWire node.

**No wpctl binary, command path, or external process** is involved on either
the host or the guest side. The baseline `d2b.guestControl.wpctlPath` option is
superseded and removed from v3. The baseline guestd `AudioSet` RPC is superseded
by this typed ComponentSession service.

#### GuestAudioAgent Process resources (one per guestUsers entry)

The controller creates one `GuestAudioAgent` Process per entry in
`AudioState.spec.guestUsers`. Each Process:
- Is named using an opaque UID digest derived from `AudioState.metadata.uid`
  and `User/<name>.metadata.uid`, e.g. `ag-4a7f2c1b` — never by username.
- Has `ownerRef: audio-pipewire.d2bus.org.AudioState/corp-vm-audio` and is located in the controller's
  component identity index for that `AudioState`; never selected by mutable
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
  ownerRef: audio-pipewire.d2bus.org.AudioState/corp-vm-audio
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
    backoffMultiplier: 2.0
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
`env` — these are signed component-template projections in the LaunchTicket.

## Endpoint resources (D092)

`Provider/audio-pipewire` declares standard `Endpoint` base-schema conformance.
Stable vhost-user, AudioMediator, and GuestAudioAgent services are owned
`Endpoint` resources with `producerRef`; they are not inline `Process.spec`
fields. Consumers use `Endpoint/<name>` references. No raw socket path,
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
  ownerRef: audio-pipewire.d2bus.org.AudioState/corp-vm-audio
spec:
  providerRef: Provider/audio-pipewire
  producerRef: Process/corp-vm-audio-sidecar
  endpointClass: data
  transport: unix
  purpose: audio-pipewire.d2bus.org/vhost-user-sound
  serviceFingerprint: audio-pipewire.d2bus.org/vhost-user-sound.v3
  locality: host-local
  visibility: authorized-consumers
  attachmentPolicy: launch-ticket
  consumerPolicy: same-zone-authorized
  lifecyclePolicy: recycle-with-producer
status:
  readiness: Ready
  observedProducerGeneration: 1
  observedResourceGeneration: 1
  endpointGeneration: 1
  connectionAvailability: available
  leaseAvailability: lease-required
```

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: audio-pipewire-service
  zone: dev
  ownerRef: Provider/audio-pipewire
spec:
  providerRef: Provider/audio-pipewire
  producerRef: Process/audio-pipewire-mediator
  endpointClass: service
  transport: unix
  purpose: audio-pipewire.d2bus.org/audio-control
  serviceFingerprint: audio-pipewire.d2bus.org/AudioMediator.v3
  locality: host-local
  visibility: authorized-consumers
  attachmentPolicy: component-session
  consumerPolicy: same-user-authorized
  lifecyclePolicy: recycle-with-producer
status:
  readiness: Ready
  observedProducerGeneration: 1
  observedResourceGeneration: 1
  endpointGeneration: 1
  connectionAvailability: available
  leaseAvailability: lease-required
```

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: ag-4a7f2c1b-audio-set
  zone: dev
  ownerRef: audio-pipewire.d2bus.org.AudioState/corp-vm-audio
spec:
  providerRef: Provider/audio-pipewire
  producerRef: Process/ag-4a7f2c1b
  endpointClass: service
  transport: vsock
  purpose: audio-pipewire.d2bus.org/guest-audio-set
  serviceFingerprint: audio-pipewire.d2bus.org/AudioSet.v3
  locality: guest-local
  visibility: authorized-consumers
  attachmentPolicy: component-session
  consumerPolicy: same-zone-authorized
  lifecyclePolicy: recycle-with-producer
status:
  readiness: Ready
  observedProducerGeneration: 1
  observedResourceGeneration: 1
  endpointGeneration: 1
  connectionAvailability: available
  leaseAvailability: lease-required
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

1. Controller calls AudioMediator `SetGrant`/`SetLevel` service (via d2b-bus,
   Unix transport) for the changed host-side channels. AudioMediator applies
   changes via libpipewire API on the worker's live PipeWire node. Returns
   `Ok` or typed error. (`AudioState.spec` is the durable authority; no prior
   state-file write is required.)
2. Controller calls `AudioSet` service on every active `GuestAudioAgent`
   Process (one per `guestUsers` entry, identified by ownerRef component
   identity index) in parallel, via d2b-bus, vsock transport. Collects all
   results.
3. Both host and guest results are aggregated; `AudioState.status.channels`,
   `enforcementPosture`, and `lastSetApplied` are updated in a single
   `UpdateStatus` batch committed post-reconcile.
4. Audit event `audio-state.grant-changed` is emitted after the status commit
   is durable (post-commit audit).
5. If the `SetGrant`/`SetLevel` service call fails: `GrantEnforcementFailed`
   condition is set; phase becomes `Degraded`.
6. If any `GuestAudioAgent.AudioSet` call fails: `GuestEnforcementFailed`
   condition is set for that agent (keyed by opaque digest); if all fail,
   `enforcementPosture` is set to `HostOnly`; phase becomes `Degraded`.

## PipeWire stream mediation

### No ambient socket exposure

The vhost-user-sound worker **never** opens, connects to, or receives a
PipeWire socket path. No socket path, runtime directory path, or
`PIPEWIRE_RUNTIME_DIR` value appears in:

- `AudioState.spec` or `AudioState.status`;
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
stream via `pw_node_set_param`. The alias label never enters stream rule text,
node property values, spec fields, or any public surface.

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

When `AudioState.spec.grants` changes, the controller:
1. Calls AudioMediator `SetGrant`/`SetLevel` service for the changed channels.
   AudioMediator applies changes via libpipewire API directly — no Process
   restart or UpdateSpec needed for grant changes. `AudioState.spec` is the
   durable authority; no state file is written.
2. Calls `GuestAudioAgent.AudioSet` service (vsock transport) for the guest side.
3. Updates `AudioState.status` in a single post-reconcile commit.

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
`AudioState.spec`, `AudioState.status`, component `Process.status`, the core
Operation ledger, and external PipeWire/guest observation after restart. The
`AudioState` ResourceType remains the Provider-owned resource model; it is not a
Provider state Volume.

No component declares a state namespace, state-layout `User/<name>` principal,
identity marker, migration worker, or Provider state mount. The
`audio-state-controller` reconcile loop does not create, update, or delete
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

**Baseline migration note**: the one-time v1/v2 `audio-state.json` migration (if
a legacy file is found on the host during Provider installation) reads it with
`parse_audio_state`, writes the parsed grants to `AudioState.spec` via
`UpdateSpec`, and removes the legacy file. This runs in the Provider activation
pathway before any component Process enters Ready. It is not a Volume lifecycle
contract and uses no ProviderStateSet Volume.

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

**Nix/compiler path** (static configuration): when `AudioState` resources are
declared in the Nix configuration, the compiler sets `spec.groups: ["audio"]`
on the corresponding guest `User` resources for each ResourceRef in
`AudioState.spec.guestUsers`. system-core verifies group membership
(`User.status.groupMembershipVerified`) and sets `GroupsVerified: True` when
confirmed.

**API-created AudioState** (runtime path): the controller checks
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
| `audio-pipewire:view-status` | `get`, `list`, `watch` | `audio-pipewire.d2bus.org.AudioState` | Read-only status viewer |
| `audio-pipewire:manage-grants` | `get`, `list`, `watch`, `updateSpec` | `audio-pipewire.d2bus.org.AudioState` | May update `spec.grants` and `spec.guestUsers` only; intended CLI role |
| `audio-pipewire:admin` | `get`, `list`, `watch`, `create`, `updateSpec`, `delete` | `audio-pipewire.d2bus.org.AudioState` | Full lifecycle |
| `audio-pipewire:controller` | all verbs including `updateStatus`, `updateFinalizers` | `audio-pipewire.d2bus.org.AudioState`, `Process` (owned by this Provider) | Controller identity only; never held by operators |

### Spec field authorization

| Field | Required role |
| --- | --- |
| `grants.*` | `audio-pipewire:manage-grants` or higher |
| `guestUsers` | `audio-pipewire:admin` |
| `suspendOnGuestAbsent` | `audio-pipewire:admin` |
| `providerRef` | immutable after creation; resource API admission rejects any mutation |

### Broker operations requested

The `audio-state-controller` requests **zero** broker operations. It
communicates exclusively through the resource API (create/update/delete
`Process` resources and read their status). The Process Provider handles all
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
   directory path, or `PIPEWIRE_RUNTIME_DIR` value appears in `AudioState.spec`,
   `AudioState.status`, any Process resource spec, any API response, OTEL
   attribute, audit record, log message, or broker configuration. Tests:
   `AudioState` and `Process` schema round-trip tests assert the absence of any
   socket-path-shaped string in every serialized form.

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

6. **No direct process lifecycle.** The audio-state-controller creates and
   updates Process resource specs only. It never calls `SpawnRunner`,
   `OpenPidfd`, issues a LaunchTicket, or adopts a pidfd. It creates no
   `Volume` or `User` resources. Tests: controller conformance test asserts the
   audio-state-controller's allowed resource API verb set.

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
   `audio_state_controller.rs::no_ephemeral_process_created`.

10. **No path leakage in audit or OTEL.** The audio controller's audit emitter
    never includes socket paths, PipeWire paths, or compositor runtime directory
    paths. Tests: `audio_telemetry.rs` redaction conformance.

11. **Controller creates no Volume or User resources.** The
    `audio-state-controller` is not permitted to issue any verb against `User`
    or `Volume` ResourceTypes; both are absent from `allowedResourceVerbs`.
    Under D087, `Provider/audio-pipewire` declares no Provider state Volume and
    its ProviderStateSet is empty. The semantic controller is not a Volume owner,
    does not export `Volume` as a ResourceType, and does not create prerequisite
    Volumes.
    Tests: controller conformance test verifies the absence of User and Volume
    verbs; ProviderDeployment integration validates that no state Volume or
    state mount is created and that bounded operational state is status-first.

## Lifecycle, restart, and adoption

### Install sequence

1. Operator creates `Provider/audio-pipewire` resource with `spec.artifactId`.
2. Core ProviderDeployment creates the `audio-state-controller` Process (system
   domain) and the `audio-mediator` Process (user domain) as static components;
   no Provider state Volume or state mount is created.
3. Both register watch plans for their respective resources.
4. `audio-pipewire.d2bus.org.AudioState` resources created by Nix or the API become Ready
   through the reconcile loop.

### Per-Guest enable sequence

1. Operator (or Nix compilation) creates `audio-pipewire.d2bus.org.AudioState/corp-vm-audio`.
2. `audio-state-controller` queries the `runtime-audio` dependency alias for
   the Guest's runtime capability. If no audio capability is advertised, sets
   `RuntimeCapabilityUnavailable`; Pending.
3. Controller checks AudioMediator Process status (`MediatorReady` condition).
4. Controller checks each `spec.guestUsers` User ref status for
   `GroupsVerified: True` (audio group membership). If any fails, sets
   `GuestUserAudioGroupMissing`; Degraded.
5. Determine sidecar desired state: if both grants are `"off"` or no runtime
   capability → desired: stopped. Else → desired: running.
6. Controller creates `Process/corp-vm-audio-sidecar` with template
   `vhost-user-sound-worker` and the operation-scoped attachment transfer
   declaration. The `Process` resource enters `Pending`.
7. d2b-bus/ProviderSupervisor resolves the AudioMediator portal FD and routes
   it to the worker's LaunchTicket. The audio controller is not involved.
8. System Process Provider (system-minijail) launches the worker with the FD.
   The vhost-user endpoint becomes ready; Process Provider sets
   `Process.status.phase = Ready`.
9. Controller watches `Process.status`; on `Ready`, sets `SidecarRunning: True`.
10. Controller mutates `Guest.spec.audioExtension` with the runtime-capability-
    derived arguments.
11. On next Guest start, the runtime attaches the virtio-snd device; `GuestAttached`
    becomes `True`; `AudioState.status.phase` transitions to `Ready`.

### Restart and adoption

A Zone runtime restart is a continuation event (ADR 0034). The audio
controller does not hold pidfds; it observes Process status through the
resource API. The Process Provider re-adopts the worker's process identity
after restart. The controller resumes watching `Process.status` and
reconverges `AudioState.status` accordingly.

If the worker exited between restarts (Process.status.phase Failed or
Unknown), the controller detects this on its first reconcile post-restart
and sets `AudioState.status.phase = Degraded`. The audio controller does
not issue any restart signal.

`suspendOnGuestAbsent: true`: when the Guest transitions out of `Running`,
the controller issues a `Process UpdateSpec` setting the desired phase to
`Stopped`. The system Process Provider performs the graceful stop. The
controller sets `GuestAbsent` condition; phase becomes `Degraded`. When
the Guest becomes `Running` again, the full enable sequence from step 6
repeats.

### Deletion sequence

1. `deletionRequestedAt` is set on `AudioState`.
2. `audio-state-controller` finalizer handler:
   a. Issues `Process.spec` mutation setting desired phase to `Stopped`.
   b. Waits for `Process.status.phase` to reach a terminal phase (system
      Process Provider performs graceful stop: SIGTERM → 10s → SIGKILL).
   c. Issues `Process Delete` via resource API.
   d. Removes `--generic-vhost-user` (or equivalent) from `Guest.spec.audioExtension`.
   e. Removes `audio-pipewire.d2bus.org/sidecar-stopped` finalizer.
3. After all finalizers are removed: resource is deleted from the store;
   a single `phase=Deleted` revision event is committed to the revision log.
4. Audit event `audio-state.deleted` is emitted **post-commit** after the
   revision event is durable. No audit event is emitted inline with the
   finalizer steps.

## Errors

### Outcome code enum

| Code | Phase | Retryable | Meaning |
| --- | --- | --- | --- |
| `ok` | `Ready` | false | All conditions satisfied |
| `SidecarNotReady` | `Pending` | true | Process not yet Ready per Process Provider |
| `GuestAbsent` | `Degraded` | true | Guest not Running; sidecar suspended |
| `GrantEnforcementFailed` | `Degraded` | true | AudioMediator `SetGrant`/`SetLevel` service returned error |
| `PipeWireSessionUnavailable` | `Degraded` | true | AudioMediator cannot acquire PipeWire portal FD from user supervisor; retries |
| `MediatorUnavailable` | `Degraded` | true | AudioMediator Process not Ready; retries |
| `GuestAudioAgentUnavailable` | `Degraded` | true | One or more GuestAudioAgent Processes not Ready; retries |
| `GuestEnforcementFailed` | `Degraded` | false | GuestAudioAgent `AudioSet` service returned error |
| `GuestUserAudioGroupMissing` | `Degraded` | false | One or more `guestUsers` lack confirmed `audio` group; operator must set `User.spec.groups` and rebuild guest |
| `RuntimeCapabilityUnavailable` | `Pending` | true | `runtime-audio` alias returned no audio capability; retries when runtime updates |
| `SidecarCrashLoop` | `Failed` | false | Process Provider `restartPolicy.maxRestarts` exceeded |
| `ProviderMisconfigured` | `Failed` | false | Missing artifact or malformed capability record |
| `AudioNotEnabled` | `Failed` | false | `AudioState` exists but owning Guest has no runtime audio capability |

Error messages are bounded to 256 characters. They must not contain socket
paths, lock paths, state-file paths, PipeWire paths, compositor runtime
directory paths, PipeWire node IDs, credential digests, or volume/gain level
values.

## Audit events

All audio controller audit events use the Zone authoritative audit path.
Audit records are JSONL with V3 payload/checksum. Audit events are emitted
**post-commit** after the relevant revision is durable in the Zone store.

No audio-specific audit event includes socket paths, volume levels, gain
values, PipeWire node IDs, or PipeWire runtime directory paths.

| Event kind | Trigger | Redacted |
| --- | --- | --- |
| `audio-state.created` | `AudioState` resource committed to store | `spec.grants.*` direction values included; no paths; no levels |
| `audio-state.grant-changed` | `spec.grants` `UpdateSpec` committed and durable | direction (`"on"`/`"off"`) changes included; `speakerLevel`/`micGain` values **omitted**; no node IDs |
| `audio-state.enforcement-applied` | AudioMediator `SetGrant`/`SetLevel` service call returned `Ok` | result: `Applied\|Degraded` per channel; no node ID, no level value |
| `audio-state.sidecar-started` | `Process.status.phase` transitions to `Ready` (observed by controller) | no socket path, no providerKind |
| `audio-state.sidecar-stopped` | `Process.status.phase` transitions to a terminal phase | includes `outcome.exitCode` relayed from Process status; no socket path |
| `audio-state.deleted` | post-commit after `phase=Deleted` revision event is durable | — |

**Suppressed from all audit records:**
- vhost-user socket path;
- PipeWire runtime directory path;
- AudioMediator FD attachment details;
- initial stream property values;
- volume/gain levels (`speakerLevel`, `micGain`);
- PipeWire node IDs or object paths;
- GuestAudioAgent AudioSet service payload bytes;
- compositor user session details.

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

Guest name, Zone name, socket path, level values, PipeWire node IDs, and
runtime capability implementation IDs are not metric labels.

### Metrics

| Metric name | Type | Description |
| --- | --- | --- |
| `d2b_audio_pipewire_states_total` | gauge | Current count of `AudioState` resources by `outcome` |
| `d2b_audio_pipewire_sidecars_running` | gauge | Count of worker Processes in Ready phase |
| `d2b_audio_pipewire_enforcement_attempts_total` | counter | AudioMediator `SetGrant`/`SetLevel` service call attempts by `channel`, `outcome` |
| `d2b_audio_pipewire_enforcement_latency_seconds` | histogram | AudioMediator service call completion latency by `channel` |
| `d2b_audio_pipewire_sidecar_restarts_total` | counter | Worker Process restart count (observed from Process.status) |
| `d2b_audio_pipewire_grant_changes_total` | counter | `spec.grants` mutations by `channel` |
| `d2b_audio_pipewire_mediator_fd_handoffs_total` | counter | PipeWire FD handoffs completed by AudioMediator by `outcome` |
| `d2b_audio_pipewire_telemetry_drop_total` | counter | Dropped telemetry frames when emitter ring is full |

No level or gain value appears in any metric. `grant_changes_total` counts
direction transitions; it does not record the level value.

### Span attributes

Controller spans carry:
- `d2b.zone`: Zone name
- `d2b.provider`: `audio-pipewire`
- `d2b.component`: `audio-state-controller` or `audio-mediator`
- `d2b.resource.type`: `AudioState` or `Process`
- `d2b.resource.generation`: current `metadata.generation`
- `d2b.outcome`: outcome code

Spans must not carry socket paths, PipeWire paths, PipeWire node IDs, level
values, gain values, or guest workload usernames.

## Async reconciliation

### Controller descriptor

```yaml
watchSelectors:
  - resourceType: audio-pipewire.d2bus.org.AudioState
    verbs: [spec, status, deletion]
  - resourceType: Process
    ownerRefType: audio-pipewire.d2bus.org.AudioState
    verbs: [status]
  - resourceType: Guest
    verbs: [status]
    ownerTrigger: true
  - resourceType: Process
    componentIdentity: audio-mediator
    verbs: [status]
  - resourceType: User
    dependencyRefIn: AudioState.spec.guestUsers
    verbs: [status]
dependencySelectors:
  - resourceType: Guest
    resolveFrom: AudioState.metadata.ownerRef
  - resourceType: User
    resolveFrom: AudioState.spec.guestUsers
allowedResourceVerbs:
  - { type: audio-pipewire.d2bus.org.AudioState, verbs: [get, list, watch, create, updateSpec, updateStatus, updateFinalizers, delete] }
  - { type: Process, verbs: [get, list, watch, create, updateSpec, delete] }
reconcileConcurrency: 8
maxPendingResources: 512
observePolicy: on-status-change
resyncPeriod: "5m"
finalizers:
  - audio-pipewire.d2bus.org/sidecar-stopped
deadlines:
  reconcile: "30s"
  finalize: "120s"
  observe: "10s"
retryClasses:
  - code: SidecarNotReady
    backoff: exponential-bounded-30s
  - code: PipeWireSessionUnavailable
    backoff: exponential-bounded-30s
  - code: MediatorUnavailable
    backoff: exponential-bounded-30s
  - code: GuestAudioAgentUnavailable
    backoff: exponential-bounded-30s
  - code: GrantEnforcementFailed
    backoff: exponential-bounded-30s
  - code: SidecarCrashLoop
    policy: no-retry
  - code: GuestUserAudioGroupMissing
    policy: no-retry
```

`EphemeralProcess` is absent from `allowedResourceVerbs`. The controller
cannot create EphemeralProcess resources. `SpawnRunner`, `OpenPidfd`, and all
broker operations are also absent. `Volume` and `User` are absent: the
audio-state-controller never creates, updates, or deletes Volume or User
resources. The ProviderStateSet is empty under D087, so core ProviderDeployment
creates no Provider state Volume and the component consumes no state view
`dirfd`.

### Reconcile flow (per `AudioState`)

```text
reconcile(AudioState):
  1. Load AudioState spec/status from store (MVCC snapshot).
  2. Resolve ownerRef Guest; check Guest.status.phase.
  3. If suspendOnGuestAbsent && Guest not Running:
       → issue Process.spec update (desired: stopped) if Process exists;
         set GuestAbsent condition; Degraded.
  4. Query runtime-audio dependency alias for the Guest's AudioCapability.
     If no audio capability advertised: set RuntimeCapabilityUnavailable; Pending; retry.
  5. For each ref in spec.guestUsers: verify User.status.groupMembershipVerified == true
     for the "audio" group. If any not verified: set GuestUserAudioGroupMissing;
     Degraded; no-retry (operator action required).
  6. Check AudioMediator Process status via component identity index.
     On not Ready: set MediatorUnavailable; Degraded; retry.
  7. Determine sidecar desired state.
     If both grants "off" → desired: stopped; remove Guest.spec.audioExtension.
     Else → desired: running.
  8. If desired: running:
      a. Create Process/<name>-audio-sidecar if absent (CREATE resource API).
      b. For each ref in spec.guestUsers:
         - Compute opaque digest of AudioState.metadata.uid + User.<name>.metadata.uid.
         - Create Process/ag-<digest> if absent (CREATE resource API), with
          ownerRef: audio-pipewire.d2bus.org.AudioState/<name> and userRef: <guestUser ref>.
         - Locate each existing GuestAudioAgent Process via ownerRef component
          identity index, not label selector.
      c. If sidecar Process.status.phase != Ready: set SidecarNotReady; retry.
      d. For each GuestAudioAgent Process (ownerRef identity index): if phase != Ready,
         set GuestAudioAgentUnavailable (keyed by digest); Degraded; retry.
  9. If desired: stopped:
      a. If sidecar Process exists and not terminal: issue Process UpdateSpec
         (desired phase: stopped). Process Provider performs graceful stop.
      b. For each GuestAudioAgent Process (ownerRef identity index): if not terminal,
         issue UpdateSpec (desired phase: stopped).
      c. Set GuestAbsent/SidecarStopped; wait for terminal phase.
  10. Enforce grants (if sidecar and all GuestAudioAgent Processes are Ready and grants changed):
      a. Call AudioMediator SetGrant/SetLevel service (d2b-bus, Unix transport)
         for each changed channel. Receive Ok or typed error.
      b. Call AudioSet service on ALL GuestAudioAgent Processes in parallel
         (d2b-bus, vsock transport). Collect all results.
      c. Aggregate results: if any agent fails, set GuestEnforcementFailed
         (keyed by agent digest); if all fail, enforcementPosture = HostOnly.
         Update status.channels and enforcementPosture.
  11. Update Guest.spec.audioExtension if needed (derived from AudioCapability record).
  12. Commit UpdateStatus batch (post-reconcile; single commit).
  13. Emit post-commit audit event if grants changed.
```

All steps are asynchronous. Steps 4–11 run concurrently for independent
`AudioState` resources. Each task holds its own optimistic revision
precondition; a conflict causes a retry.

## Cross-Zone audio sharing (D096)

One owner-Zone `audio-pipewire` **authority/mediator** service connects to the
physical PipeWire microphone and speakers; it is the sole holder of that
connection. Child Zones never open the physical device or receive a PipeWire
FD/socket — that FD never crosses a Zone.

- The owner Zone declares a `ResourceExport` referencing the local audio
  authority `Endpoint` and the exported `audio-pipewire.d2bus.org/AudioState`
  type. The **speaker** path is `multiplexed`: the authority mixes all consumer
  Zones with per-Zone volume and quota. The **microphone** path is explicit
  `exclusive` OR approved `multiplexed` capture, arbitrated with consent,
  priority, and a fair queue.
- Each child Zone declares a `ResourceImport` that binds its local `ZoneLink` and
  `exportKey` to a local `AudioState`/proxy **projection** resource; ordinary
  consumers use that local projection Ref. Audio frames stream over the bounded,
  encrypted named stream (per-import session generation, credits/backpressure,
  cancel, deadline) — intermediaries see ciphertext.
- The audio Provider's signed **export/import adapter** performs mix/consent/
  arbitration and builds the projection; core owns `ResourceExport`/
  `ResourceImport` routing and base lifecycle. Export removal or ZoneLink loss
  revokes leases and degrades the local `AudioState` projection; reconnect
  revalidates generation/fingerprint.

## Nix authoring and configuration

### Operator-facing schema

```nix
d2b.zones.dev.resources = {
  corp-vm-audio = {
    type = "audio-pipewire.d2bus.org.AudioState";
    metadata.ownerRef = "Guest/corp-vm";
    spec = {
      providerRef = "Provider/audio-pipewire";
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

### Nix validation

Nix eval-time validation of `audio-pipewire.d2bus.org.AudioState` resources checks:

- `spec.providerRef` resolves to an installed `Provider/audio-pipewire` in the
  same Zone; evaluation fails with a descriptive error if absent;
- `spec.grants.speakerLevel` is `null` or an integer in `[0,100]`;
- `spec.grants.micGain` is `null` or an integer in `[0,100]`;
- `spec.guestUsers` contains ≤16 entries, each a valid `User/<name>` ResourceRef
  where `<name>` matches `[a-z][a-z0-9_-]*` ≤32 chars; at compile time, the
  Nix module sets `spec.groups: ["audio"]` on each referenced guest `User`
  resource; for API-created resources, the controller verifies
  `User.status.groupMembershipVerified` and fails closed;
- `metadata.ownerRef` resolves to an existing `Guest/<name>` in the same Zone;
- no two `AudioState` resources share the same `metadata.ownerRef`.

The Nix module also validates that:
- `Provider/audio-pipewire.spec.config.captureAlias`, when non-null, matches
  `^[a-z][a-z0-9-]*$` and is ≤64 characters; no path separator, whitespace,
  PipeWire syntax, numeric start, or uppercase is permitted;
- `d2b.site.audio.inputTargetNode` (the current legacy option) is absent when
  the v3 Provider is installed; a clear migration error is emitted if both
  are set.

### Generated canonical ResourceSpec

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type": "audio-pipewire.d2bus.org.AudioState",
  "metadata": {
    "name": "corp-vm-audio",
    "zone": "dev",
    "ownerRef": "Guest/corp-vm"
  },
  "spec": {
    "providerRef": "Provider/audio-pipewire",
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

When an `AudioState` resource is removed from the Nix configuration:
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

## Current-code fit

| Item | Value |
| --- | --- |
| Current anchor | `nixos-modules/components/audio/host.nix`, `guest.nix`, `packages/d2b-core/src/audio_policy.rs`, `packages/d2bd/src/audio_dispatch.rs`, `packages/d2bd/src/audio_host_controller.rs`, `packages/d2b-host/src/audio_argv.rs` |
| Evidence class | Mixed; `audio_policy.rs` / `audio_argv.rs` are `implemented-and-reachable`; v3 Provider/`audio-pipewire.d2bus.org.AudioState`/AudioMediator wiring is `ADR-only` |
| Behavior retained | Per-VM mic/speaker grants, `LevelPercent` 0..=100, component-template argv shape, PipeWire `client.conf.d/` stream rule placement, WirePlumber virtio-snd profile, zero host capabilities |
| Required delta | Entire `audio-pipewire.d2bus.org.AudioState` ResourceType, `audio-state-controller`, `AudioMediator` service component with `SetGrant`/`SetLevel` service, `GuestAudioAgent` Process component with libpipewire `AudioSet` service, runtime-audio capability query and alias binding, Guest spec audioExtension mutation derived from AudioCapability record, libpipewire enforcement API on both host and guest sides, OTEL metric/audit shape, RBAC roles, Nix v3 resource authoring with `User/<name>` ResourceRefs, `spec.config` Provider shape |
| Reuse path | `d2b-core/src/audio_policy.rs` → copy-unchanged into `src/audio_state.rs` (one-time activation migration from v1/v2 on-disk format only; no ongoing state file); `audio_argv.rs` → adapt into component template (not live Process spec); WirePlumber stream rule Nix logic → port to Provider Nix module |
| Replacement/deletion | `audio_dispatch.rs`, `audio_host_controller.rs` retired after `audio-state-controller` passes e2e parity; `host.nix`, `guest.nix` retired after v3 Nix module deployed; `d2b-core/src/audio_policy.rs` may remain as re-export shim |
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
| Reuse action | `copy-unchanged` (no daemon imports; pure DTO library) |
| Destination | `packages/d2b-provider-audio-pipewire/src/audio_state.rs`; re-exported from crate root |
| Detailed design | `LevelPercent`, `AudioGrant`, `AudioPolicyState`, `parse_audio_state`, `to_v2_bytes`, `AudioPolicyError` copy unchanged. `AudioPolicyState` is the canonical in-memory representation of `AudioState.spec.grants`. `parse_audio_state`/`to_v2_bytes` are used only once during first-activation migration from a prior v1/v2 on-disk file; there is no ongoing state file in v3. |
| Integration | `audio-state-controller` uses `parse_audio_state` only during first-activation one-time migration of prior on-disk state to `AudioState.spec.grants`. |
| Data migration | v1 and v2 on-disk formats parsed once on first activation; migrated values written to `AudioState.spec.grants` via resource API; prior file removed after successful migration. |
| Validation | `tests/audio_state.rs`: all existing tests from `d2b-core/tests/audio_policy.rs` plus schema-round-trip and resource-spec JSON serialization tests |
| Removal proof | `d2b-core/src/audio_policy.rs` deleted when no `d2bd` caller references it; confirmed by `cargo check --no-default-features`. |

### ADR046-audio-002: Adapt `AudioArgvInput` into vhost-user-sound component template

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-002` |
| Dependency/owner | Depends on `ADR046-audio-001`; Process Provider template schema |
| Current source | `packages/d2b-host/src/audio_argv.rs` (all symbols + tests); `tests/golden/runner-shape/audio-argv-minimal.txt` |
| Reuse source | Same baseline paths |
| Reuse action | `adapt` — argv builder retained; becomes a signed component-template projection, not a live Process spec field |
| Destination | `packages/d2b-provider-audio-pipewire/src/argv.rs` (component template renderer) |
| Detailed design | `generate_audio_argv` remains the canonical argv builder for the `vhost-user-sound-worker` component template. The resulting argv/env/executableRef are sealed into the LaunchTicket. The per-Guest binary copy path enforcement remains via the LaunchTicket verifier. The live Process resource spec contains no argv or executableRef. The `--socket` argument is removed; the vhost-user service identity is `Endpoint/corp-vm-audio-vhost-user`, while the backing locator is resolved into the LaunchTicket under authorization. |
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
| Reuse action | `adapt` — enforcement logic becomes a libpipewire API implementation behind the `SetGrant`/`SetLevel` ComponentSession service |
| Destination | `packages/d2b-provider-audio-pipewire/src/mediator/enforcement.rs` |
| Detailed design | `SetGrant(channel, value)`: maps `"off"` to `pw_node_set_param(SPA_PARAM_Props, mute=true, target.object=-1)` on the worker's node; maps `"on"` to `mute=false` and removes routing override. `SetLevel(channel, valuePercent)`: maps to `pw_node_set_param(SPA_PARAM_Props, volume=valuePercent/100.0)`. `captureAlias` resolution: iterates the PipeWire global registry to find the node with matching `node.nick` or `node.name`. All resolution is private to the mediator; resolved node IDs never appear in any external surface. `QemuAudioController` becomes a no-op: the controller writes the state file only; `enforcementPosture = HostOnly`. `FakeAudioMediator` is a test double behind `#[cfg(test)]`. No wpctl binary, no EphemeralProcess, no node ID in any spec or bus message. |
| Integration | `audio-state-controller` calls `SetGrant`/`SetLevel` service on the AudioMediator via d2b-bus in reconcile step 10. |
| Data migration | No state migration; mediator applies current AudioState grants and levels from resource state during reconcile, replacing host-controller direct writes. |
| Validation | `tests/mediator.rs` and `tests/enforcement.rs`: SetGrant/SetLevel service round-trip; Qemu offline path; no-node-id-in-bus-message assertion; PipeWireSessionUnavailable path; captureAlias registry resolution |
| Removal proof | `d2bd/src/audio_host_controller.rs` retired after `d2bd` audio dispatch path is replaced. |

### ADR046-audio-005: Implement `AudioState` ResourceType schema and resource API admission

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-005` |
| Dependency/owner | Core resource-api foundation; `d2b-provider-audio-pipewire` crate |
| Current source | None (ADR-only); structured after `d2b-contracts/src/public_wire.rs` audio types |
| Reuse source | `public_wire.rs` `AudioChannel`, `AudioEnforcementPosture`, `AudioErrorKind`, `AudioProviderKind`, `AudioSetApplied` — copy-unchanged as closed enum definitions |
| Reuse action | `copy-unchanged` for closed enums; `ADR-only` for schema/admission |
| Destination | `packages/d2b-provider-audio-pipewire/src/resource_type.rs`; `packages/d2b-provider-audio-pipewire/src/admission.rs` |
| Detailed design | `audio-pipewire.d2bus.org.AudioState.spec` struct with serde + schemars; strict `deny_unknown_fields`; `guestUsers` ≤16, each a `User/<name>` ResourceRef where `<name>` matches `[a-z][a-z0-9_-]*` ≤32 chars; `speakerLevel`/`micGain` in `[0,100]` or null; `providerRef` format check and immutability enforcement. Audio frontend configuration is derived at reconcile time from the `runtime-audio` capability record, not stored in the spec. JSON schema exported to `docs/reference/schemas/v3/AudioState.json` (tracked by `make test-drift`). |
| Integration | Provider signs and exports the `audio-pipewire.d2bus.org.AudioState` ResourceTypeSchema during package build. Core validates `AudioState` specs on every `Create`/`UpdateSpec` call. |
| Data migration | Full d2b 3.0 reset; no v2 resource-state import. AudioState specs are authored or generated as new v3 ResourceType data. |
| Validation | `tests/resource_type.rs`: schema round-trip; admission rejection matrix (unknown fields, out-of-range level, excess guestUsers, bad name, invalid ResourceRef format, immutable providerRef mutation, forbidden audioFrontend field); JSON schema drift test |
| Removal proof | None — net-new ResourceType; no prior owner to remove |

### ADR046-audio-006: Implement `audio-state-controller`

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-006` |
| Dependency/owner | Depends on `ADR046-audio-001` through `ADR046-audio-005`; system Process Provider; AudioMediator service (`ADR046-audio-007`); GuestAudioAgent component (`ADR046-audio-011`); core reconciliation framework; no Provider state Volume is declared under D087 |
| Current source | `packages/d2bd/src/audio_dispatch.rs` lines 250–end (dispatch ordering reference) |
| Reuse source | None directly; reconcile flow is new async controller |
| Reuse action | `adapt` — dispatch logic is the reference for step ordering only |
| Destination | `packages/d2b-provider-audio-pipewire/src/controller/audio_state.rs` |
| Detailed design | Single async reconcile loop for `audio-pipewire.d2bus.org.AudioState` resource events. Watch plan for `audio-pipewire.d2bus.org.AudioState`, `Process` (sidecar+GuestAudioAgent owned, via ownerRef component identity index), `Guest` (dependency), AudioMediator `Process` (component identity), `User` (guestUsers dependency, via dependencyRefIn). Never issues SpawnRunner, OpenPidfd, any pidfd, EphemeralProcess, or Volume/User resource operation. ProviderStateSet is empty under D087; the controller consumes no state view `dirfd`. Queries `runtime-audio` capability alias on each reconcile. Calls AudioMediator `SetGrant`/`SetLevel` service and `GuestAudioAgent.AudioSet` service via d2b-bus for grant changes. Verifies `User.status.groupMembershipVerified` for each guestUser before sidecar start; fails closed on missing audio group. Batch `UpdateStatus` as a single post-reconcile commit. Post-commit audit event emission. No direct filesystem access. No runtime `User.spec.groups` mutation. |
| Integration | Registered with Zone core as a controller under `Provider/audio-pipewire`. |
| Data migration | v1/v2 audio policy file migration is handled by ADR046-audio-001 before reconcile; the controller keeps no Provider state Volume and imports no additional runtime state. |
| Validation | `tests/audio_state_controller.rs`: reconcile state machine matrix (enable, grant change, guest absent, mediator unavailable, guest-audio-agent unavailable, guest-user-audio-group-missing, sidecar crash loop, deletion sequence, runtime-capability-unavailable); no-EphemeralProcess-created assertion; no-broker-op assertion; no-pidfd-op assertion; no-runtime-groups-mutation assertion; no-Volume-create assertion; no-User-create assertion. ProviderDeployment integration validates that `Provider/audio-pipewire` creates no Provider state Volume or state mount, returns an empty ProviderStateSet, and keeps bounded operational state in status/core Operation ledger with redaction and size-bound conformance. |
| Removal proof | Supersedes `audio_dispatch.rs`; `d2bd` audio dispatch deleted after e2e parity test confirms |

### ADR046-audio-007: Implement `AudioMediator` user-session service

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-007` |
| Dependency/owner | Depends on `ADR046-audio-004`; ComponentSession service (ADR-046-componentsession-and-bus); libpipewire 1.x |
| Current source | `packages/d2bd/src/audio_host_controller.rs` PipeWire session access patterns (reference only) |
| Reuse source | None; new component |
| Reuse action | `ADR-only` |
| Destination | `packages/d2b-provider-audio-pipewire/src/mediator/mod.rs`; `src/bin/audio_pipewire_mediator.rs` |
| Detailed design | Long-lived user-session Process. Maintains per-Guest map of `{AudioState UID → pw_core* connection, node_handle}`. Opens PipeWire connection naturally (same-UID compositor session). Passes pre-opened `pw_core` FD to worker via component-descriptor attachment on Process Provider launch request. Exposes `SetGrant`/`SetLevel` ComponentSession service through `Endpoint/audio-pipewire-service` (Noise_NN, local purpose, Unix transport, same-UID peer evidence). No EphemeralProcess, no wpctl binary, no node ID in any bus message or resource spec. `captureAlias` resolved via libpipewire registry (`pw_registry_events`). WPCTL_PATH and PW_DUMP_PATH are superseded. |
| Integration | Second binary in the `d2b-provider-audio-pipewire` package. Registered as a user-session service under `Provider/audio-pipewire`. |
| Data migration | No persisted mediator state migration; the service rebuilds its PipeWire node map from the registry on start and consumes current AudioState through controller calls. |
| Validation | `tests/mediator.rs`: FD handoff; SetGrant/SetLevel service calls; captureAlias registry resolution; node-id-sealed-not-in-bus; session-unavailable path; concurrent guest map isolation; teardown on deletion |
| Removal proof | Supersedes `d2bd`'s `PipeWireHostController` direct session access; `d2bd` audio host controller deleted after e2e parity |

### ADR046-audio-008: Nix module for v3 `AudioState` resource authoring

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-008` |
| Dependency/owner | Depends on `ADR046-audio-005`; Nix resource compilation framework; `ADR-046-nix-configuration` |
| Current source | `nixos-modules/components/audio/host.nix` and `guest.nix` |
| Reuse source | Same |
| Reuse action | `replace` |
| Destination | `nixos-modules/components/audio/v3-resource.nix`; `nixos-modules/components/audio/host-config.nix`; `nixos-modules/components/audio/guest-config.nix` |
| Detailed design | Provider root config uses `spec.config` (not `rootConfig`). `captureAlias` is a bounded label option matching `^[a-z][a-z0-9-]*$` (≤64 chars), not an `inputTargetNode` raw PipeWire node name. No `d2b.guestControl.wpctlPath` option in v3. `v3-resource.nix` emits `audio-pipewire.d2bus.org.AudioState` resource specs; `guestUsers` is a list of `User/<name>` ResourceRefs; the Nix module sets `spec.groups: ["audio"]` on each referenced guest `User` resource at compile time (no runtime extraGroups mutation). No `audioFrontend`/`virtioId`/`queueSizes` in the spec; frontend configuration is derived from the `runtime-audio` capability at reconcile time. `host-config.nix` contains WirePlumber stream rules (`client.conf.d/90-d2b`) and vhost-device-sound package; no per-VM tmpfiles or state-file paths. `guest-config.nix` contains the in-guest PipeWire stack, virtio-snd kernel module, and GuestAudioAgent Process resource declaration. |
| Integration | `nixos-modules/default.nix` imports all three modules. |
| Data migration | Full d2b 3.0 reset; legacy Nix audio options emit/deprecate to v3 AudioState authoring during config rebuild, with no runtime state import. |
| Validation | `tests/unit/nix/cases/audio-v3-resource.nix`: resource spec round-trip; `captureAlias` regex validation (`^[a-z][a-z0-9-]*$`); eval-time rejection of duplicate AudioState per Guest; guestUsers as `User/<name>` ResourceRefs; `spec.groups` injection; deprecation warning for legacy options; `spec.config` (not `rootConfig`) shape test; no-wpctlPath-option assertion; no-runtime-extraGroups-mutation assertion; no-audioFrontend-field assertion |
| Removal proof | `host.nix` and `guest.nix` kept as compat shims until v3 module deployed on all Zones |

### ADR046-audio-009: Minijail contract test migration

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-009` |
| Dependency/owner | Depends on `ADR046-audio-005`; `d2b-contract-tests` crate |
| Current source | `packages/d2b-contract-tests/tests/minijail_audio_usbip.rs` audio section |
| Reuse source | Same |
| Reuse action | `adapt` |
| Destination | `packages/d2b-provider-audio-pipewire/tests/minijail_contract.rs` (provider-local); retain cross-bundle source greps in `d2b-contract-tests` |
| Detailed design | Provider-local tests validate: (1) `spec.sandbox.capabilityClasses == []`; (2) `spec.sandbox.seccompClass == "audio-pipewire-worker"`; (3) `spec.sandbox.startRoot == false`; (4) `namespaceClasses` does not include `network`; (5) `spec.domain == "system"` and `spec.userRef` is absent (worker is system-domain; allocator principal is sealed); (6) no `executableRef`/`argv`/`env` fields in the live Process resource spec; (7) no `inherited-fd` endpoint in the live Process resource spec; (8) owned `Endpoint/corp-vm-audio-vhost-user` has `producerRef: Process/corp-vm-audio-sidecar`; (9) no socket path or PipeWire path in any serialized spec or status form; (10) no EphemeralProcess created by the controller. |
| Integration | Provider-local contract tests run in `d2b-provider-audio-pipewire`; retained cross-bundle greps in `d2b-contract-tests` ensure bundle-wide invariants still hold. |
| Data migration | None — test migration only; no runtime state. |
| Validation | `cargo test -p d2b-provider-audio-pipewire -- minijail` must pass; existing cross-bundle tests must continue to pass |
| Removal proof | None — net-new provider-local test file; cross-bundle tests are retained |

### ADR046-audio-010: OTEL telemetry and audit emitters

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-010` |
| Dependency/owner | Depends on `ADR046-audio-006`, `ADR046-audio-007`, `ADR046-audio-011`; `d2b-telemetry` lightweight emitter |
| Current source | `packages/d2bd/src/audio_dispatch.rs` audit call sites (redaction pattern reference) |
| Reuse source | Same; adapt redaction pattern |
| Reuse action | `adapt` |
| Destination | `packages/d2b-provider-audio-pipewire/src/telemetry.rs` |
| Detailed design | Closed metric label set. Post-commit audit emitter. Span attribute allowlist. Compile-time assertion that no audio metric carries a `guest`, `socket`, `path`, `level`, `gain`, or `node_id` label. `enforcement_attempts_total` and `enforcement_latency_seconds` reflect AudioMediator `SetGrant`/`SetLevel` service calls, not EphemeralProcess. ProcessEffect audit events are emitted by the system Process Provider; the audio telemetry module must not duplicate them. |
| Integration | Audio controller and mediator call telemetry/audit emitters after commit or enforcement; d2b-telemetry exporter and policy_observability consume the resulting records. |
| Data migration | No telemetry/audit data migration; v3 emits new closed-label OTEL/audit records after cutover and old audio_dispatch audit sites are removed. |
| Validation | `tests/audio_telemetry.rs`: redaction conformance; post-commit ordering; label cardinality; forbidden-field absence; no-process-effect audit duplication; no-wpctl-ephemeralprocess metrics |
| Removal proof | `audio_dispatch.rs` audit call sites deleted after cutover |

### ADR046-audio-011: Implement `GuestAudioAgent` in-guest service component

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-011` |
| Dependency/owner | Depends on `ADR046-audio-004`; ComponentSession service contract; libpipewire 1.x; system-systemd Process Provider for guest domain |
| Current source | `packages/d2b-guestd/src/audio_set.rs` (guestd wpctl dispatch — reference only) |
| Reuse source | `packages/d2bd/src/audio_host_controller.rs` libpipewire enforcement patterns (reference only) |
| Reuse action | `ADR-only` (new component; supersedes guestd wpctl dispatch path) |
| Destination | `packages/d2b-provider-audio-pipewire/src/guest_agent/mod.rs`; `src/guest_agent/enforcement.rs`; `src/bin/audio_pipewire_guest_agent.rs` |
| Detailed design | Long-lived user-domain Process running in the Guest under the guest workload user's UID. One Process resource per entry in `AudioState.spec.guestUsers`; each named by opaque UID digest (`ag-<digest>`) and carrying label `audio-pipewire.d2bus.org/role: guest-audio-agent`. `userRef` is the corresponding `User/<name>` Zone resource. Opens a PipeWire connection in the Guest's compositor session (same-UID, natural access). Exposes a typed `AudioSet` ComponentSession service through an owned `Endpoint/ag-<digest>-audio-set` (vsock transport, Guest→Zone d2b-bus). `AudioSet(mic, speaker, speakerLevel, micGain)` applies changes via libpipewire API (`pw_node_set_param` with `SPA_PARAM_Props`, `pw_stream_set_control`) on the guest virtio-snd PipeWire node. No wpctl binary, no command path, no EphemeralProcess. Controller calls ALL active GuestAudioAgent instances in parallel for each grant change and aggregates failures. `FakeGuestAudioAgent` is a test double behind `#[cfg(test)]`. |
| Integration | Third binary in the `d2b-provider-audio-pipewire` package. Declared as GuestAudioAgent Process resources by the audio-state-controller (one per guestUser; template: `guest-audio-agent`). System Process Provider (`Provider/system-systemd`) launches each inside the Guest under the respective guest workload user's UID. |
| Data migration | No guest runtime state migration; GuestAudioAgent reconnects to guest PipeWire and applies current AudioState grants and levels on reconcile, replacing guestd wpctl dispatch. |
| Validation | `tests/guest_agent.rs`: AudioSet service call → libpipewire apply; mute/route/level; session-unavailable path; reconnect state restore; no wpctl binary; no command path; N-agent creation (one per guestUser); parallel call and aggregated failure |
| Removal proof | `d2b-guestd` wpctl audio dispatch path deleted after all Guests have GuestAudioAgent deployed and e2e parity test passes |

### ADR046-audio-012: Cross-Zone audio export/import adapter (D096)

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audio-012` |
| Dependency/owner | ADR046-zone-control-019, ADR046-zone-control-020; audio Provider owner |
| Current source | None — net-new ADR 0046 cross-Zone sharing (D096) |
| Reuse source | audio authority/mediator service (this dossier); `packages/d2b-provider/src/share_adapter.rs` `ExportAdapter`/`ImportAdapter` traits |
| Reuse action | net-new (implement the signed audio export/import adapter) |
| Destination | `packages/d2b-provider-audio-pipewire/src/share_adapter.rs` |
| Detailed design | Implement the signed audio `ExportAdapter`/`ImportAdapter`: the owner-Zone authority mediates the physical mic/speakers, mixes speaker output per consumer Zone with per-Zone volume/quota, and arbitrates microphone as `exclusive` or approved `multiplexed` capture with consent/priority/fair queue. The import adapter builds the local `AudioState`/proxy projection and streams audio frames over the bounded encrypted named stream. No PipeWire FD/socket crosses a Zone; intermediaries see ciphertext. Semantic admission/observation only — core owns routing and base lifecycle. |
| Integration | Core export/import controller (ADR046-zone-control-019); local projection lifecycle (ADR046-zone-control-020); ComponentSession bounded encrypted named streams |
| Data migration | None — full d2b 3.0 reset |
| Validation | Speaker mix with per-Zone volume/quota; microphone exclusivity and approved multiplexed capture with consent/priority/fair queue; reconnect revalidation and revocation degrade the projection; no PipeWire FD/socket crosses a Zone (fake-stream hermetic + real-stream integration) |
| Removal proof | Not applicable (new surface) |

## Required crate layout

```text
d2b-provider-audio-pipewire/
  README.md
  src/
    lib.rs
    audio_state.rs          # AudioPolicyState DTOs (ADR046-audio-001)
    argv.rs                 # Component template argv builder (ADR046-audio-002)
    resource_type.rs        # AudioState spec/status schema + serde (ADR046-audio-005)
    admission.rs            # Resource API admission validation (ADR046-audio-005)
    runtime_capability.rs   # runtime-audio capability query client
    telemetry.rs            # Metrics, post-commit audit, span attributes (ADR046-audio-010)
    controller/
      mod.rs
      audio_state.rs        # audio-state-controller AudioState handler (ADR046-audio-006)
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
    audio_state.rs          # AudioPolicyState / schema round-trip (ADR046-audio-001)
    argv.rs                 # Component template argv rejection matrix (ADR046-audio-002)
    resource_type.rs        # Schema admission rejection matrix (ADR046-audio-005)
    audio_state_controller.rs  # AudioState handler state machine (ADR046-audio-006)
    mediator.rs             # FD handoff + SetGrant/SetLevel + captureAlias (ADR046-audio-007)
    enforcement.rs          # host libpipewire enforcement + offline path (ADR046-audio-004)
    guest_agent.rs          # AudioSet service + guest libpipewire (ADR046-audio-011)
    audio_telemetry.rs      # Redaction / label cardinality / post-commit (ADR046-audio-010)
    minijail_contract.rs    # Zero-caps / seccompClass / no-argv/env/executableRef (ADR046-audio-009)
  integration/
    README.md
    audio_e2e.rs            # End-to-end: enable guest, sidecar start, grant change, delete
    grant_enforcement.rs    # AudioMediator SetGrant/SetLevel + libpipewire round-trip
    guest_enforcement.rs    # GuestAudioAgent AudioSet service + libpipewire round-trip
  README.md
```

`src/`, `tests/`, `integration/`, and `README.md` are all required. Workspace
policy rejects a provider crate missing any of these paths.

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has p95 ≤50 ms with no wall-clock
sleep, and `cargo test -p d2b-provider-audio-pipewire --lib --tests` completes
in ≤2 s warm-cache execution time (compilation excluded). They use a
deterministic fake clock/RNG and the toolkit fakes/FakeEffectPort only — no
process spawn, container, network, DBus, systemd, broker daemon, Nix eval/build,
KVM, USB/GPU/TPM hardware, or live cloud, and no filesystem tree beyond tiny
temp fixtures. Any scenario needing those lives only in `integration/`, which
keeps a lane timeout/budget, parallel isolation, and fake external services by
default; such a need is re-placed into `integration/`, never given a sleep,
larger timeout, or `#[ignore]`. Bounded crypto/property tests are the only
classified exception, each named with a capped case count and a declared higher
per-test budget.

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
   `SetGrant`/`SetLevel`/`AudioSet` responses);
6. What `D2B_FIXTURES_FULL` provides for the minijail contract tests.

### `README.md` content requirements

The crate `README.md` must document:

1. Provider identity: `Provider/audio-pipewire`;
2. Exported ResourceType: `AudioState` (link to spec);
3. Controller and service components; one controller binary, one host mediator binary, one guest agent binary;
4. Worker template: `vhost-user-sound-worker`; guest service template: `guest-audio-agent`;
5. Nix authoring schema (verbatim example from this spec);
6. `Provider/audio-pipewire.spec.config` options (`captureAlias`; regex
   `^[a-z][a-z0-9-]*$`);
7. Dependency chain: vhost-device-sound v0.3.0, PipeWire/WirePlumber,
   virtio-snd, libpipewire 1.x `pw_node_set_param`;
8. Security: zero capabilities, no PipeWire socket path in public surfaces,
   AudioMediator receives declared pre-opened portal FD from user supervisor
   (not ambient socket), FD routed via ProviderSupervisor to worker LaunchTicket,
   AudioState reconcile creates no Volume or User resources (`Provider/audio-pipewire`
   declares no Provider state Volume and its ProviderStateSet is empty), no broker
   process lifecycle ops, no EphemeralProcess for enforcement, no wpctl binary
   on host or guest, no runtime User.spec.groups mutation;
9. Build: `cargo build -p d2b-provider-audio-pipewire`;
10. Test: `cargo test -p d2b-provider-audio-pipewire`;
11. Integration: see `integration/README.md`;
12. Standalone-repository consumption path.

## Removal schedule

| Artifact | Condition for removal |
| --- | --- |
| `nixos-modules/components/audio/host.nix` | After `v3-resource.nix` and `host-config.nix` deployed on all Zones and `make test-drift` passes |
| `nixos-modules/components/audio/guest.nix` | After `guest-config.nix` deployed and all Guests rebuilt with v3 module |
| `packages/d2bd/src/audio_dispatch.rs` | After `audio-state-controller` passes e2e parity test and `d2bd` has no callers |
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
entries are deleted once successor coverage and the removal proof pass —
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.
