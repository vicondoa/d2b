# ADR 0046 Provider model and packaging

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-model-and-packaging` |
| Parent | ADR 0046 |
| Status | Proposed |
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

Provider spec contains:

- exact package/executable/manifest/config/schema/service digests;
- publisher/signature/trust/conformance/provenance/SBOM identity;
- support channel and compatibility range;
- root configuration validated against signed JSON Schema;
- exported ResourceTypes/schemas;
- controller component descriptors;
- service component descriptors;
- worker Process templates;
- dependency aliases;
- permission claims;
- CLI projection;
- events/telemetry/state contracts;
- component placement templates;
- upgrade/drain/restart policy.

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
| worker | Narrow Process/EphemeralProcess with no controller/bus/dependency/CLI authority |

Every component is a separate Process except the fixed system-core and
system-minijail bootstrap controllers.

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

### Provider/system-minijail

Implements the same ResourceTypes:

- compiled inline Process sandbox;
- broker/Host/Guest supervisor effect;
- clone3(CLONE_PIDFD);
- d2b wait/reap ownership;
- cgroup/namespace/FD/adoption validation.

Future Process Providers pass the same conformance without schema changes.

## Configuration projection

One Provider-owned root JSON Schema is evaluated before launch. The signed
component graph defines deterministic projections:

- fields visible to each component;
- defaults/validation;
- sensitivity;
- ResourceRef/dependency bindings;
- component schema digest.

Components cannot read sibling config. Secrets are Credential refs, not config
values. Root/component digests bind Provider resource, Process resources,
ComponentSessions, state, status, and audit.

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

Nix compiles an offline sorted exact-digest catalog:

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
- failure/upgrade/migration;
- exact v3 source→future destination work items and tests.

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
| `device-usbip` | Device | USB inventory/arbitration/export/attach/firewall and USBIP Process/EphemeralProcess |
| `device-security-key` | Device | Security-key inventory/ceremony/CID/lease/session; unprivileged Host relay and Guest frontend Processes; fixed broker only opens/passes hidraw |
| `device-gpu` | Device | Combined GPU/render/VFIO/video/media arbitration and GPU/video worker Processes |

Azure/ACA-specific network remains inside Guest Providers until an
independently shared Azure Network is required.

### Interaction

| Provider | Implements | Description/processes |
| --- | --- | --- |
| `display-wayland` | Provider-specific display/session types | Wayland/display policy, Host/Guest proxies, window identity/rails and endpoint Processes |
| `audio-pipewire` | Provider-specific audio/session types | PipeWire policy/session, Host/user components, vhost-user-sound Processes |
| `clipboard-wayland` | Provider-specific clipboard types | Selection/bridge/transfer/presentation and Host/user/Guest Processes |
| `notification-desktop` | Provider-specific notification types | Observe/project/action/ack/presentation Processes |
| `shell-terminal` | ShellSession | Persistent terminal session/supervisor, open/attach/detach/kill and named terminal streams |

One-shot exec is EphemeralProcess, not an exec Provider.

### Credentials

| Provider | Implements | Description/processes |
| --- | --- | --- |
| `credential-secret-service` | Credential | Exact-user Secret Service/keyring leases and typed operations |
| `credential-entra` | Credential | Entra-bound credential leases/operations without token export |
| `credential-managed-identity` | Credential | Host/Guest cloud managed-identity leases/operations |

### Transport/observability/activation

| Provider | Implements | Description/processes |
| --- | --- | --- |
| `transport-unix` | ZoneLink/ComponentSession transport | Local Unix/socketpair endpoints, peer evidence, FD-capable local channels |
| `transport-vsock` | ZoneLink/ComponentSession transport | Host/Guest vsock channels, expected CID and no FD transfer |
| `transport-azure-relay` | ZoneLink/ComponentSession transport | Remote Azure Relay reachability; relay identity is carriage only |
| `observability-otel` | Provider-specific telemetry endpoint/export/status types | OTEL endpoint/export/collector integration and health |
| `activation-nixos` | Provider-specific activation types | NixOS generation plan/apply/inspect/adopt/rollback |

Cross-resource composition is ordinary controller behavior. There is no
special orchestrator Provider.

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
| Reuse action | extract and adapt |
| Destination | `packages/d2b-contracts/src/v3/provider.rs`, `packages/d2b-provider/src/lib.rs`, `packages/d2b-provider-toolkit/` |
| Detailed design | Provider resource/manifest/components/dependencies/services/trust/compatibility/toolkit |
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
| Destination | one `packages/d2b-provider-<base>-<implementation>/` per Provider; generic Nix Provider package/catalog emitter |
| Detailed design | Split current combined/composition crates; exact outputs/manifests/conformance |
| Integration | Provider package installed/registered per Zone |
| Data migration | No package compatibility path |
| Validation | Workspace naming/dependency/output/dossier/catalog parity policy |
| Removal proof | Combined crate removed only after every live implementation has a Provider successor |

### ADR046-provider-003

| Field | Value |
| --- | --- |
| Dependency/owner | Process contracts; system Provider owners |
| Current source | `d2bd` DAG/broker spawn; unsafe-local helper; guestd/exec runner; `d2b-host` runtime provider |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-provider-system-core/`, `d2b-provider-system-systemd/`, `d2b-provider-system-minijail/` |
| Detailed design | Bootstrap system-core; common Process/EphemeralProcess providers and pidfd conformance |
| Integration | Host/Guest providerRef/domain/userRef, local supervisors, resource status |
| Data migration | Current roles converted under reset |
| Validation | Shared conformance and host/user/non-Host tests |
| Removal proof | Current role launch paths removed after parity |
