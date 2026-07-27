# ADR 0046 Provider model and packaging

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-model-and-packaging` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | Provider contracts/toolkit, package catalog, Nix integration |
| Depends on | `ADR-046-resource-object-model`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-primitive-resource-composition` |
| Supersedes | Current direct Provider construction/composition |

## Provider resource

A Provider is installed in a Zone as:

```text
Provider/<name>
```

Package presence alone is not installation. providerRef resolves only a Ready
Provider resource in the same Zone.

Provider ResourceSpec is exactly:

```yaml
spec:
  artifactId: <plain bounded ID>   # selects a signed Nix artifact-catalog + manifest entry
  config: {}                        # root configuration validated against the manifest's signed JSON Schema
```

No other field is authored in Provider spec. Every other Provider property is
resolved from the signed manifest/catalog entry the `artifactId` selects, not
authored or independently duplicated in the resource row:

- exact package/executable/manifest/config/schema/service digests;
- publisher/signature/trust/conformance/provenance/SBOM identity;
- support channel and compatibility range;
- exported ResourceTypes/schemas;
- controller component descriptors;
- service component descriptors;
- worker Process templates;
- dependency aliases;
- permission claims;
- CLI projection;
- events/telemetry/state contracts;
- per bound ResourceType, the exact base spec and base status schema version/fingerprint the `ResourceApiBinding` implements, and the signed **standard capability matrix** of supported/unsupported optional base capabilities (D089);
- signed export/import adapter capabilities and one projection factory for each
  capability the Provider marks cross-Zone exportable (D096);
- registered `spec.provider` extension schemas (D089): per owned or bound ResourceType, the qualified `schemaId`, `schemaVersion`, and signed strict JSON Schema for the `spec.provider.settings` object the Provider accepts;
- registered `status.provider` extension schemas (D088): per owned or written ResourceType, the qualified `schemaId`, `schemaVersion`, and signed strict JSON Schema for the `status.provider.details` object the Provider may write;
- component placement templates;
- upgrade/drain/restart policy.

Manifest/package/component/resource/status/generated properties are read-only
derived data, never authored Provider spec fields. Core ProviderDeployment
reads this signed manifest/catalog entry and creates the Provider's static
component graph from it (see `ADR-046-components-processes-and-sandbox`).

**Base-schema conformance and extension registration (D088/D089).** Each
`ResourceApiBinding` declares and MUST implement the exact ResourceType base
spec/status schema version/fingerprint, accept the canonical minimal valid base
Spec, and pass the base lifecycle/status/error/finalizer conformance suite. A
Provider that accepts a Provider-specific `spec.provider` extension or writes a
`status.provider` extension registers both extension schemas in its signed
manifest: the qualified immutable `schemaId`, the `schemaVersion`, and the strict
JSON Schema for `settings`/`details` (unknown-field-denied, size/cardinality
bounded; `details` additionally redacted/non-secret). The resource store
validates every `spec.provider`/`status.provider` write against the installed
Provider's registered schema at Nix build and API admission, rejecting an
unregistered/version-mismatched `schemaId`/`schemaVersion`, an unknown field, or
a `settings`/`details` that shadows a base field. A Provider MAY refuse an
optional base capability only via its signed capability matrix and the
provider-neutral `unsupported-capability` result; it never ignores, reinterprets,
renames, duplicates, or weakens a base field. Fields shared across
implementations of a ResourceType are promoted to the ResourceType base
(`spec.*`/`status.resource`) and are never registered or written under
`spec.provider`/`status.provider` (see `ADR-046-resource-object-model` § Spec /
§ Status). The `spec.provider` and `status.provider` schemas align for the same
Provider.

This rule applies unchanged to D096 Service/Binding types: their qualified
namespaces and base spec/status are semantic and provider-neutral. `providerRef`
selects the implementation, strict `spec.provider` carries only that
implementation's settings, and every implementation MUST accept the canonical
minimal base without `spec.provider`. PipeWire, OTEL, USBIP, CTAPHID, package,
binary, and adapter details never enter the semantic base; `providerRef` remains
the sole opaque implementation selector.

**Common semantic contract catalog (D098).** The canonical base DTOs and schemas
are owned by the shared contract catalog, not by any initial implementation
crate. They are discoverable before a Provider package is selected:

| Family | Service ResourceType | Binding ResourceType | Shared contract module |
| --- | --- | --- | --- |
| audio | `audio.d2bus.org.AudioService` | `audio.d2bus.org.AudioBinding` | `d2b-contracts::v3::semantic_services::audio` |
| security key | `security-key.d2bus.org.SecurityKeyService` | `security-key.d2bus.org.SecurityKeyBinding` | `d2b-contracts::v3::semantic_services::security_key` |
| telemetry | `telemetry.d2bus.org.TelemetryService` | `telemetry.d2bus.org.TelemetryBinding` | `d2b-contracts::v3::semantic_services::telemetry` |
| USB | `usb.d2bus.org.UsbService` | `usb.d2bus.org.UsbBinding` | `d2b-contracts::v3::semantic_services::usb` |

For each catalog ResourceType `<namespace>.<Type>`, its common schema IDs are
`<namespace>/<Type>/spec` and `<namespace>/<Type>/status`. This slash form is a
schema identity only; the API ResourceType remains the exact dot-qualified name
in the table, and a ResourceRef appends `/<name>` to that ResourceType.

Each module exports the strict base spec/status DTOs, schema IDs, versions,
fingerprints, minimal valid fixtures, and projection-factory type binding. Every
Provider implementation binds those exact fingerprints and adds only its own
strict extension schemas. It cannot copy, fork, weaken, or privately redefine
the base. There are no implementation-qualified or former `*State` aliases.

**Export/import adapters and projection factories (D096).** A Provider that
marks a capability cross-Zone exportable MUST advertise signed `ExportAdapter`,
`ImportAdapter`, and `ProjectionFactory` metadata. Each factory is immutable
within a Provider artifact and binds:

| Field | Contract |
| --- | --- |
| `serviceType` | One qualified semantic/provider-neutral `*Service` ResourceType; this is both the owner authority type and consumer projection type |
| `bindingType` | Its qualified semantic/provider-neutral `*Binding` ResourceType expressing local consumer intent |
| `allowedBackingRefTypes` | Closed set of same-Zone `Device`, `Endpoint`, or qualified semantic backend types the owner Service may reference |
| `allowedBindingTargetRefTypes` | Closed subset of `Guest`, `User`, and `Zone` that a Binding may target |
| `projectionSchema` | Signed, strict, deny-unknown semantic base schema for the projection Service; contains only standard `providerRef`, semantic base/import fields, and no `spec.provider`, implementation-specific field, raw locator, path, credential, secret, FD, or bytes |
| `projectionSchemaFingerprint` | SHA-256 of that canonical schema |
| `factoryFingerprint` | SHA-256 binding all fields above plus the semantic projection-protocol version; never Provider/adapter identity or implementation detail |

The owner `*Service` is the one real authority and references its local backing.
`ResourceExport.resourceRef` MUST target that Service, never a `Device`,
`Endpoint`, or `*Binding`. An import creates exactly one same-qualified-type local
projection Service (`ownerRef: ResourceImport/<name>`). It never creates a
Device, Endpoint, or Binding. Operators/Nix author matching local Binding
resources that reference `serviceRef` plus an allowed consuming target. Binding
spec is desired intent only; all observations are written only to `status`. The
Provider's Binding controller creates owned Process/Endpoint children.
High-churn leases, sessions, ceremonies, transfers, and streams remain internal
records.

The adapters perform only semantic admission, arbitration, projection
materialization, and bounded observation. Core owns `ResourceExport`/
`ResourceImport` routing, base lifecycle, projection-Service ownership, and
layered status writes. Provider install, Nix build, and API admission all fail
closed if a required factory is absent, its signature is invalid, the Service/
Binding pair or allowed refs do not match, or either fingerprint differs from
the advertisement/import expectation. `Service` and `Binding` remain qualified
semantic types and do not enlarge the 19-type standard catalog. Export/import
preserves `serviceType` exactly even when owner and consumer select different
conformant implementations through local `providerRef`. It never copies or
requires the remote `spec.provider` extension, and Core does not synthesize a
local one. Routing derives from the signed local Provider descriptor,
`providerRef`, and ResourceImport record; implementation observation is
permitted only in `status.provider`. The strict
descriptor accepts only `bindingType` and `allowedBindingTargetRefTypes`; the
former `stateType`/`allowedStateTargetRefTypes` spellings and qualified `*State`
types are not aliases and are rejected.

**Authority descriptors (D097).** For every scarce or singleton backing it owns
(a physical device, singleton external service, per-Zone/Host/user/seat service,
fixed listener/store, or globally-unique policy), a Provider descriptor MUST
declare a signed `AuthorityDescriptor` on the owning typed Resource:
`authorityScope`, an opaque non-authorizing `authorityKey` class,
`cardinality`, `arbitration`, the single `authorityRef` owner service, a typed
`duplicateConflict`, an adoption/restart `ownerProof`, an update/recycle
strategy, `exportability` (`forbidden`/`explicit-export`, cross-Zone only via
D096), and a quota/fairness policy (see
[`ADR-046-resource-object-model` §Authority and cardinality](ADR-046-resource-object-model.md)).
Backing Device/Endpoint/backend descriptors remain `forbidden`; only an
approved qualified owner `*Service` descriptor may carry `explicit-export`.
The descriptor is not a new opaque public ID and adds no new ResourceType unless
an audit proves no existing type can own the lifecycle. Core rejects a
conflicting authority Resource/Process against its authority index before any
external effect; a shared/multiplexed backing still has exactly one authority
owner.

**Currency, upgrade, and expedited reconcile (D090/D091).** Every controller
component implements, alongside ordinary reconcile, the toolkit methods
`assess_update`, `plan_upgrade`, and `execute_upgrade`, and populates the
universal `status.update` currency object. A controller MUST report
`UpgradeRequired` for a disruptive change rather than apply it in place;
non-disruptive changes reconcile normally. Upgrades preserve the Resource
UID/spec identity and durable/state/secret Volumes and TPM identity where
possible, recycling only realization and owned ephemeral Processes/endpoints;
`Replace` of the resource-row identity is used only when explicitly required and
planned with ownership/state transfer. The signed capability matrix and base
conformance suite (D089) additionally cover currency states, disruption classes,
and the expedited (`waitForReconcile`) path - a controller performs no external
effect, finalizer release, or status mutation on an expedited mutation until it
receives Core's `CommittedRevisionProof`, and it returns a bounded
disposition/projection for the one expedited pass. Upgrade disruption policy is
a provider-neutral base field; a `spec.provider` extension may add
implementation knobs but cannot bypass it.

Provider status contains:

- common resource status;
- package/trust/API/conformance result;
- required/optional component status;
- exported ResourceType/service readiness;
- dependency health;
- controller leases/watch status;
- state schema/migration health;
- disabled/quarantined condition;
- aggregate Provider generation.

Provider status is derived, not self-declared: core computes the aggregate
Provider status from component/dependency/process health. A Provider
controller writes status only for the ResourceTypes it owns and for the
authorized children/status fields its descriptor grants; it never authors its
own aggregate Provider-resource status. When a Provider controller writes an
owned resource's status it writes the three layers (universal base,
`status.resource`, and its own `status.provider`) atomically in one mutation
(D088).

## Crate/package boundary

Every Provider maps to one independently buildable crate and signed package.

One Provider crate:

- declares one Provider identity;
- may build several controller/service/worker binaries;
- may share an internal library among those binaries;
- depends only on public neutral contracts/toolkit/SDK crates and approved
  ecosystem dependencies;
- does not import d2bd, broker, Zone-store, Nix-emitter, or another Provider's
  implementation internals;
- has one Nix package/conformance output;
- has one `ADR-046-provider-<provider-name>.md` dossier.

Every Provider crate contains:

```text
d2b-provider-<base>-<implementation>/
  src/
  tests/
  integration/
  README.md
```

- `src/`: implementation, component binaries, internal modules, and colocated
  unit tests;
- `tests/`: hermetic Cargo integration, ResourceType/controller conformance,
  fault, redaction, schema, and fake-port tests;
- `integration/`: heavier container/Host/Guest/cross-process/provider-system
  fixtures and scenarios invoked by existing repository test orchestration;
- `README.md`: Provider identity/config, ResourceTypes, controllers/services/
  workers/binaries, placement, dependencies/RBAC, security/state/telemetry,
  build/test/integration commands, and standalone-repository consumption.

Workspace policy rejects a Provider crate missing any of these paths.

This boundary must allow moving the crate to its own GitHub repository without
splitting semantics or copying daemon internals.

Common libraries are Provider-neutral. A common library cannot register a
second Provider identity or become a hidden multi-Provider composition binary.

## Provider components

Component types:

| Type | Responsibility |
| --- | --- |
| controller | Owns one or more ResourceTypes and async reconcile loop |
| service | Serves typed runtime/internal ComponentSession methods; no ResourceType ownership |
| worker | Narrow Process/EphemeralProcess with no ResourceClient, d2b-bus/dependency-portal, Credential, CLI, broker, or child-spawn authority; all resources/FDs/config are inherited via LaunchTicket |

Every component is a separate Process except the fixed system-core and
system-minijail bootstrap controllers. Core ProviderDeployment creates every
component's static Process (per the signed manifest's component descriptors)
and, before launching that Process, only the state Volumes the component has
**declared** under the storage-need test, as part of the Provider's optional
**ProviderStateSet** (`ADR-046-provider-state`: the logical, query-time
grouping of the *declared* Volume resources owned by `Provider/<name>` - not a
ResourceType or a stored artifact of its own, and empty for a Provider that
declares no state Volume). Bounded non-secret operational state belongs in the
owning resource's `status` subresource and the core Operation ledger by default
(D087); a component declares a state Volume only when a specific payload is a
secret or sensitive private datum, is large/binary/file content, is private
data unsafe for status readers, or is bounded but revision-unsuitable with a
demonstrated recovery need. A stateless component declares no state Volume,
receives none, and contributes none to the ProviderStateSet; there is no empty
identity-only Volume and no separate "compartment" concept. A Provider
controller never bootstraps its own Process; it may only create authorized
dynamic children (further Process/EphemeralProcess or other primitive/vendor
resources) once it is itself running. Creating a declared state Volume normally
requires a `Provider/volume-local` controller instance to be running on that
same execution target (Host, Guest, or user-domain local-storage owner);
because the fixed bootstrap components (`system-core`, `system-minijail`, and
the first `volume-local` instance on each target) keep their bounded non-secret
operational state in `status`/the core Operation ledger and declare no state
Volume, no component needs a Volume before a `volume-local` instance is Ready,
so there is no bootstrap state-Volume cycle and no bootstrap-storage exception
(D086, superseded by D087). See
`ADR-046-components-processes-and-sandbox` for the full static-deployment and
optional-component-state-Volume contract, and `ADR-046-resources-volume` for
the canonical Volume schema every declared state Volume uses.

Descriptor fields include:

- component ID/type/binary/template;
- exported ResourceTypes/methods;
- supported Host/Guest Provider capabilities;
- allowed `system|user` domains;
- cardinality;
- config projection;
- required/optional dependencies;
- ResourceRefs/templates it may create/use;
- state/Volume views;
- Process Provider selection constraints;
- permission claims;
- readiness/health/drain;
- process/sandbox/budget maximums.

The same ResourceType is declared once. Several controller instances may run
under different Hosts/Guests/domains without duplicate Process schemas.

## system-core bootstrap

The one fixed core-controller process per Zone is also
`Provider/system-core`. It and the fixed Provider/system-minijail controller are
the only Providers not represented by Process resources.

It owns:

- Host reconciliation;
- local User discovery/status.

It does not own:

- Process/EphemeralProcess (`system-systemd`, `system-minijail`);
- Volume;
- Network;
- Device;
- Credential;
- semantic runtime/desktop/cloud resources.

After system-core creates the first Host, system-minijail launches every other
Provider/controller/service/worker as a Process under a Host or Guest.

## Process Provider family

### Provider/system-systemd

Implements Process and EphemeralProcess for systemd-capable Hosts/Guests:

- non-forking transient system service/scope;
- transient user scope through fixed user supervisor;
- InvocationID+cgroup+MainPID/start-time verification;
- mandatory local pidfd;
- systemd wait/reap ownership;
- no per-Provider static PID1 template units.

Neither system-systemd's controller nor the process it launches calls
systemd's D-Bus/socket API or `pidfd_open` directly; it validates the
ExecutionSpec/SandboxSpec and calls the `ProcessLaunchEffectPort`
(ProviderSupervisor), which is the sole caller of the systemd effect owner.

### Provider/system-minijail

Implements the same ResourceTypes:

- compiled inline Process sandbox;
- clone3(CLONE_PIDFD);
- d2b wait/reap ownership;
- cgroup/namespace/FD/adoption validation.

system-minijail's controller never imports or calls the broker directly. It
validates the ExecutionSpec/SandboxSpec and calls the `ProcessLaunchEffectPort`
(ProviderSupervisor) with the resource UID and compiled sandbox digest;
ProviderSupervisor is the sole caller of the broker's `clone3`/spawn effect.

Future Process Providers pass the same conformance without schema changes.

## Configuration projection

One Provider-owned root JSON Schema is evaluated before launch. The signed
component graph defines deterministic projections:

- fields visible to each component;
- defaults/validation;
- sensitivity;
- ResourceRef/dependency bindings;
- component schema digest.

Components cannot read sibling config. Secrets are Credential refs, not config values. A signed Provider component may
be selected as a raw-token consumer only through the Credential spec/RBAC and
the KK end-to-end sensitive ComponentSession contract. Root/component digests
bind Provider resource, Process resources, ComponentSessions, state, status,
and audit.

## Provider dependencies

Manifest declares aliases:

```text
runtime
volume
network
credential
transport
```

Zone config binds each alias to an exact Provider ResourceRef/service
fingerprint. A component asks d2b-bus for an alias; it never receives a global
registry/route table or arbitrary Provider endpoint.

Synchronous dependency cycles fail configuration. Optional dependencies produce
declared degraded behavior only.

## Package catalog

Nix authoring first declares derivations separately:

```nix
d2b.artifacts.provider-wayland = {
  package = inputs.wayland-provider.packages.${system}.default;
  type = "provider";
};
```

The Provider ResourceSpec then uses `artifactId = "provider-wayland"`. Nix
compiles an offline sorted exact-digest catalog:

- Provider/package/publisher/version;
- package/executable/manifest/component/descriptor/config digests;
- systems/platform;
- API/service compatibility;
- signature/root epoch/revocation/deny status;
- provenance/SBOM/license/vulnerability evidence;
- conformance attestation;
- support channel;
- support contact.

Selection is exact digest. No runtime marketplace, download, PATH scan,
directory discovery, latest, or version-range solving.

Artifact is not a ResourceType; `artifactId` is a plain bounded ID, not a
ResourceRef. The private catalog may retain a Nix store path for activation,
but resource spec/status/audit never expose it.

## Trust

Production admission requires:

- exact digest;
- trusted publisher/root epoch;
- valid signature/rotation/revocation;
- no emergency deny;
- accepted provenance/SBOM/license/vulnerability policy;
- exact package/API conformance attestation.

First- and third-party Providers use the same admission and sandbox. Trust does
not bypass runtime restrictions.

## Compatibility

- Provider API major exact;
- minor additive only;
- protobuf numbers never reused;
- exact descriptor fingerprint selected before launch;
- no handshake downgrade/fallback;
- removal after deprecation window or new major;
- state schema compatibility/migration checked independently.

## Distribution bundles

A bundle is a signed package catalog only. It does not:

- merge Providers into one process/sandbox;
- union config/permissions;
- apply last-wins overrides;
- provide runtime discovery.

Duplicate Provider names, command namespaces, ResourceTypes, incompatible
fingerprints, or policy conflicts reject the generation.

## Toolkit

Official Rust toolkit provides:

- async ResourceClient/Reconciler loop;
- ComponentSession/d2b-bus lifecycle;
- generated typed Provider/service clients/servers;
- config/schema projection;
- Volume/pidfd-free Provider state helpers;
- operation/checkpoint/event/telemetry helpers;
- fake core/store/bus/supervisor/effect clients;
- fault injection;
- black-box conformance;
- Provider flake/project templates.

Wire/state-machine golden vectors remain language-neutral.

## Provider dossier requirement

Every Provider dossier specifies:

- exact crate/package/providerRef;
- root config schema/defaults/bounds/secrets;
- ResourceTypes implemented/consumed;
- controller watch/reconcile/finalize;
- services/CLI/events;
- every binary Process template/placement;
- Volume/state/credential use;
- dependencies/permission claims;
- pidfd/wait/reap where Process Provider;
- telemetry/audit/doctor/support;
- for each cross-Zone exportable capability, the exact qualified `*Service` and
  `*Binding` names, signed projection-factory metadata, allowed Service backing
  refs, allowed Binding target refs, schema/fingerprints, adapter behavior,
  arbitration, finalizers, and update/status propagation; or an explicit
  `exportability: forbidden`;
- failure/upgrade/migration;
- exact v3 source→future destination work items and tests, each naming the
  old test selector/file with a keep/adapt/move/delete disposition and removal
  gate (D094);
- test placement and execution budgets (D094): `src/` units and `tests/` are
  fast, in-process, deterministic, parallel-safe hermetic tests within the
  `ADR-046-validation-and-delivery` §10.16 budgets (individual normal test p95
  ≤50 ms, no wall-clock sleep; per-crate `--lib --tests` ≤3 s), and every real
  process/socket/container/Nix/guest/hardware/live scenario lives only in
  `integration/`.

## Frozen initial Provider catalog

Every row requires one Provider crate/package and one
`ADR-046-provider-<name>.md` dossier.

### System, Host, and Guest

| Provider | Implements | Description/processes |
| --- | --- | --- |
| `system-core` | Host, User | Fixed core-controller bootstrap; reconciles one or more Hosts and local User discovery/status only |
| `system-systemd` | Process, EphemeralProcess | Transient non-forking system/user units/scopes, pidfd verification, systemd wait/reap |
| `system-minijail` | Process, EphemeralProcess | Broker/minijail/clone3 sandboxed process, local pidfd and d2b wait/reap |
| `runtime-cloud-hypervisor` | Guest | Local NixOS VM lifecycle; owns VMM and guest-bootstrap child resources/Processes |
| `runtime-qemu-media` | Guest | QEMU media/physical-media lifecycle and QMP-mediated child Processes |
| `runtime-azure-container-apps` | Guest | Azure Container Apps sandbox lifecycle and remote agent integration |
| `runtime-azure-virtual-machine` | Guest | Full-host Azure VM lifecycle, bootstrap, and optional child Zone hosting |

Unsafe-local is not a Provider. It is a user-only Host under
system-core.

### Storage/network/device

| Provider | Implements | Description/processes |
| --- | --- | --- |
| `volume-local` | Volume | Anchored local durable/ephemeral storage, fine-grained layout/ACL/views, bind/tmpfs/local source behavior and store-view mode |
| `volume-virtiofs` | Volume attachment controller | Host source Volume to target Guest virtiofs export/mount; owns virtiofsd Processes and attachment status |
| `network-local` | Network | Local bridge/namespace/address/DHCP/DNS/NAT/firewall/egress and Host/Guest attachment |
| `device-tpm` | Device | TPM allocation, swtpm Process, persistent TPM Volume/state and identity |
| `device-usbip` | Device; `usb.d2bus.org.UsbService`; `usb.d2bus.org.UsbBinding` | USB inventory/arbitration/export/attach/firewall and USBIP Process/EphemeralProcess; implementation detail stays out of semantic USB base |
| `device-security-key` | Device; `security-key.d2bus.org.SecurityKeyService`; `security-key.d2bus.org.SecurityKeyBinding` | Security-key inventory/ceremony/CID/lease/session; unprivileged Host relay and Guest frontend Processes; CTAPHID/hidraw details stay out of semantic security-key base |
| `device-gpu` | Device | Combined GPU/render/VFIO/video/media arbitration and GPU/video worker Processes |

Azure/ACA-specific network remains inside Guest Providers until an
independently shared Azure Network is required.

### Interaction

| Provider | Implements | Description/processes |
| --- | --- | --- |
| `display-wayland` | Provider-specific display/session types | Wayland/display policy, Host/Guest proxies, window identity/rails and endpoint Processes |
| `audio-pipewire` | `audio.d2bus.org.AudioService`; `audio.d2bus.org.AudioBinding` | Audio policy/session, Host/user components, vhost-user-sound Processes; PipeWire stays an implementation detail |
| `clipboard-wayland` | Provider-specific clipboard types | Selection/bridge/transfer/presentation and Host/user/Guest Processes |
| `notification-desktop` | Provider-specific notification types | Observe/project/action/ack/presentation Processes |
| `shell-terminal` | `shell-terminal.d2bus.org.ShellSession` | Persistent terminal session/supervisor, open/attach/detach/kill and named terminal streams |

One-shot exec is EphemeralProcess, not an exec Provider.

### Credentials

| Provider | Implements | Description/processes |
| --- | --- | --- |
| `credential-secret-service` | Credential | Exact-user Secret Service/keyring leases and typed operations |
| `credential-entra` | Credential | Entra-bound credential leases/operations without token export |
| `credential-managed-identity` | Credential | Host/Guest cloud managed-identity leases/operations |

### Transport/observability/activation

Transport Providers are carriage services only; they never own ZoneLink. The
core ZoneLink handler alone reads/writes/finalizes ZoneLink and owns
Noise/session/reconnect/route/idempotency/intent state, calling typed
`OpenTransport`/`CloseTransport`/`ObserveTransport` on the installed Transport
Provider. A Transport Provider returns only an opaque `OwnedTransport`/
byte-stream handle and observations; it holds no ZoneLink state itself.

| Provider | Implements | Description/processes |
| --- | --- | --- |
| `transport-unix` | Transport carriage (`OpenTransport`/`CloseTransport`/`ObserveTransport`) | Local Unix/socketpair endpoints, peer evidence, FD-capable local channels |
| `transport-vsock` | Transport carriage (`OpenTransport`/`CloseTransport`/`ObserveTransport`) | Host/Guest vsock channels, expected CID and no FD transfer |
| `transport-azure-relay` | Transport carriage (`OpenTransport`/`CloseTransport`/`ObserveTransport`) | Remote Azure Relay reachability; relay identity is carriage only |
| `observability-otel` | `telemetry.d2bus.org.TelemetryService`; `telemetry.d2bus.org.TelemetryBinding` | Telemetry endpoint/export/collector integration and health; OTEL stays an implementation detail |
| `activation-nixos` | Provider-specific activation types | NixOS generation plan/apply/inspect/adopt/rollback |

Cross-resource composition is ordinary controller behavior. There is no
special orchestrator Provider.

### D096 exportability classification

Cross-Zone export is deny-by-default. The initial classification is:

| Semantic Service/Binding pair | Initial Provider | D096 classification |
| --- | --- | --- |
| `audio.d2bus.org.AudioService` / `audio.d2bus.org.AudioBinding` | `audio-pipewire` | exportable |
| `security-key.d2bus.org.SecurityKeyService` / `security-key.d2bus.org.SecurityKeyBinding` | `device-security-key` | exportable |
| `telemetry.d2bus.org.TelemetryService` / `telemetry.d2bus.org.TelemetryBinding` | `observability-otel` | exportable |
| `usb.d2bus.org.UsbService` / `usb.d2bus.org.UsbBinding` | `device-usbip` | policy-gated exportable; Provider, Zone, export, and device policy all opt in |
| every other semantic type / frozen initial Provider | - | forbidden unless a later reviewed semantic contract adds a pair and a conformant Provider binds it |

Approval applies to the qualified Service only. A matching Binding, its backing
Device/Endpoint, Credentials, and internal session/stream records are never
export targets. These semantic names are frozen independently of the initial
implementations. Provider dossiers bind implementations and extensions but
cannot alias, rename, or vendor-qualify the base types.

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | `d2b-realm-provider`; live ACA/Relay/gateway; unwired d2b-host-providers; direct d2bd construction; current Nix package outputs |
| Evidence class | Mixed: ACA/Relay/gateway reachable; host adapters/transport/codec/client mostly unwired; generic registry/toolkit absent |
| Behavior retained | Typed Provider traits/errors/capabilities, fail-closed absence, circuit breaker, credential planes, redaction, injected test seams |
| Required delta | Provider resource/catalog/trust, one crate per Provider, process components, toolkit/conformance, exact dependencies |
| Reuse path | Extract current semantic logic with evidence-specific work items; do not copy dead scaffolds as live |
| Replacement/deletion | Direct d2bd constructors/factories removed only after Provider resource/Process/service integration |
| Feasibility proof | Out-of-tree template, multi-binary Provider, signed package, exact process bootstrap and resource controller |
| Future owner | Work items below and Provider dossiers |

## Implementation work items

### ADR046-provider-001

| Field | Value |
| --- | --- |
| Dependency/owner | W0; Provider contract/catalog owner |
| Current source | `packages/d2b-realm-provider/src/{provider,capabilities,error,credential,rate_limit,conformance}.rs` |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/provider.rs`, `packages/d2b-provider/src/lib.rs`, `packages/d2b-provider-toolkit/` |
| Detailed design | Provider resource/manifest/components/dependencies/services/trust/compatibility/toolkit Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Zone config/catalog → Provider resource → Process components → bus/resource routes |
| Data migration | Full reset |
| Validation | Contract vectors, fake/malicious Provider, one-crate/one-identity policy |
| Removal proof | Old trait crate retired only after all Provider dossiers migrate |

### ADR046-provider-002

| Field | Value |
| --- | --- |
| Dependency/owner | Provider contract; package/Nix integrator |
| Current source | `packages/Cargo.toml`; `flake.nix`; `nixos-modules/host-daemon.nix`; current source package derivations |
| Reuse action | adapt |
| Destination | one `packages/d2b-provider-<base>-<implementation>/` per Provider with mandatory src/, tests/, integration/, README.md; generic Nix Provider package/catalog emitter |
| Detailed design | Split current combined/composition crates; exact outputs/manifests/conformance/layout/documentation |
| Integration | Provider package installed/registered per Zone |
| Data migration | No package compatibility path |
| Validation | Workspace naming/dependency/output/dossier/catalog parity policy |
| Removal proof | Combined crate removed only after every live implementation has a Provider successor |

### ADR046-provider-003

| Field | Value |
| --- | --- |
| Dependency/owner | Process contracts; system Provider owners |
| Current source | `d2bd` DAG/broker spawn; unsafe-local helper; guestd/exec runner; `d2b-host` runtime provider |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-system-core/`, `d2b-provider-system-systemd/`, `d2b-provider-system-minijail/` |
| Detailed design | Bootstrap system-core; common Process/EphemeralProcess providers and pidfd conformance Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Host/Guest providerRef/domain/userRef, local supervisors, resource status |
| Data migration | Current roles converted under reset |
| Validation | Shared conformance and host/user/non-Host tests |
| Removal proof | Current role launch paths removed after parity |

### ADR046-provider-004

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-provider-001; shared semantic Service/Binding contract owner |
| Current source | None - D098 common semantic Service/Binding bases are net-new ADR 0046 contracts |
| Reuse action | create |
| Destination | `packages/d2b-contracts/src/v3/semantic_services/{mod,audio,security_key,telemetry,usb}.rs`; generated schema artifacts for the eight exact qualified ResourceTypes |
| Detailed design | Define one shared strict base spec/status DTO and schema contract for each frozen D098 Service/Binding pair, including exact semantic type/schema IDs, versions, fingerprints, minimal valid base fixtures without `spec.provider`, authority/projection Service union, same-Zone Binding `serviceRef`/target rules, D088 `status.resource` layering, status-only observations, and projection-factory type binding. A Core-generated projection permits only `providerRef`, semantic base/import fields, and ResourceImport ownership; it rejects `spec.provider`. Register no implementation-qualified or former `*State` alias. |
| Integration | Provider manifests and ResourceApiBindings consume the common catalog fingerprint; ADR046-zone-control-019/020 use the same factory metadata to admit an owner Service and core-create one same-type projection Service; the four initial Provider dossiers supply only strict implementation extensions/controllers. |
| Data migration | Full d2b 3.0 reset; no prior public Service/Binding names or aliases are imported |
| Validation | Shared contract tests cover exact names, strict serde/schema round trips, common base discoverability without any Provider package, canonical minimal base acceptance without `spec.provider`, same-Zone refs/targets, owner/projection discrimination, Core projection rejection of `spec.provider`, common fields only under `status.resource`, implementation observation only under `status.provider`, status-only observations, no Device/Endpoint/Binding projection, implementation-detail rejection, semantic factory-fingerprint stability under Provider/adapter identity changes, and rejection of every implementation-qualified/former `*State` alias. Each initial and fake alternate Provider must pass the identical base conformance fixture. |
| Removal proof | Any Provider-local duplicate base DTO/schema is removed before that Provider is registered; only strict Provider extension DTOs remain implementation-owned. |
