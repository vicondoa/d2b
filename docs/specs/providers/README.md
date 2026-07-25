# ADR 0046 Provider dossiers

This directory holds the normative Provider dossiers for the d2b 3.0 Provider
control plane. Each dossier is a member of the atomic ADR 0046 specification set
indexed by [`../README.md`](../README.md); the parent decision is
[`docs/adr/0046-d2b-3-provider-control-plane.md`](../../adr/0046-d2b-3-provider-control-plane.md).

This is an accepted, documentation-only set. All dossiers are
`Accepted`; nothing here creates crates, packages, controllers, services, or
Provider resources.

## What a Provider is

A Provider is installed in a Zone as a `Provider/<name>` resource and can be
selected by `providerRef` only once that resource is `Ready`. Package presence
alone is not installation. Provider ResourceSpec is exactly `{ artifactId,
config }`: `artifactId` selects a signed Nix artifact-catalog and manifest
entry, and `config` is validated against the Provider's settings schema. See
[`../ADR-046-provider-model-and-packaging.md`](../ADR-046-provider-model-and-packaging.md).

## Packaging and crate layout

Every Provider is **one independently buildable crate and signed,
separately-built multi-process package**. A Provider may contain several
separately sandboxed controller, service, and worker binaries, but it declares
exactly one Provider identity. Each Provider crate root MUST provide the four
workspace-policy paths:

- `src/` — controller, service, and worker binaries and shared library code;
- `tests/` — fast hermetic unit and binary integration tests with runtime
  budgets (D094);
- `integration/` — slower cross-process/container integration fixtures (D094);
- `README.md` — crate overview covering its required topics.

Each dossier's **Implementation work items** assign exact work items and files
to each of those paths.

## Process placement, controllers, and state

With the two bootstrap exceptions `Provider/system-core` and
`Provider/system-minijail` (whose handlers run in the fixed core-controller and
the fixed minijail bootstrap controller), every Provider controller, service,
and worker is an ordinary `Process`/`EphemeralProcess` resource placed on a
`Host/<name>` or `Guest/<name>` through an installed Process Provider. Semantic
Provider controllers compose behavior by creating owned primitive resources and
typed `EffectPort` calls; they never call spawn, systemd, minijail, broker,
filesystem, network, or device effects directly.

Component state is **status-first** (D086/D087): bounded, non-secret state lives
in the owning resource's `status` by default. A component declares a
per-component `Volume` — created and deleted by **core ProviderDeployment**, not
by the semantic controller — only when its state is secret/sensitive, large or
binary/file content, or unsuitable for revisioned API/status churn. Stateless
and status-sufficient components declare none. When declared, those state
Volumes are `Provider/volume-local`-backed, `persistent`, with a nonzero quota
and identity marker; a component only mounts its own required Volume view.

## Endpoints

Stable managed endpoints are `Endpoint` resources (D092), not inline
`ProcessSpec` fields: producers reference them by `Endpoint/<name>` and consumers
resolve them through EffectPort/LaunchTicket only. Per-session/high-churn handles
stay internal. Vendor ResourceTypes and Provider spec/status extensions are
qualified on `d2bus.org` (D080). The frozen semantic Service/Binding families use
provider-neutral namespaces; implementation namespaces identify only strict
Provider extensions.

## Provider catalog (27)

"Service-only" Providers own no exported ResourceType and act through
ComponentSession services and owned primitive resources; "transport-only"
Providers carry ZoneLink sessions and own no Zone ResourceType.

| Provider resource | Owned / exported ResourceTypes | Main components (role / placement) | Dossier |
| --- | --- | --- | --- |
| `Provider/activation-nixos` | qualified `activation-nixos.d2bus.org.NixosGeneration` | controller + activation EphemeralProcess dispatch (Host, system) | [activation-nixos](ADR-046-provider-activation-nixos.md) |
| `Provider/audio-pipewire` | `audio.d2bus.org.AudioService`, `audio.d2bus.org.AudioBinding` | controller + dedicated audio worker Processes (Host/Guest) | [audio-pipewire](ADR-046-provider-audio-pipewire.md) |
| `Provider/clipboard-wayland` | service-only (qualified `clipboard-wayland.*`; no standard type) | `clipboard-controller` (system-minijail, system), `clipd-host` (system-systemd, user), picker `EphemeralProcess` | [clipboard-wayland](ADR-046-provider-clipboard-wayland.md) |
| `Provider/credential-entra` | `Credential` | Guest-resident adapter to an Entrablau identity Guest `Endpoint`; controller + agent (login/token/TPM state in Guest, CLI login, no Host ambient auth) | [credential-entra](ADR-046-provider-credential-entra.md) |
| `Provider/credential-managed-identity` | `Credential` | controller + managed-identity agent Process | [credential-managed-identity](ADR-046-provider-credential-managed-identity.md) |
| `Provider/credential-secret-service` | `Credential` | controller + Secret Service agent Process (user domain) | [credential-secret-service](ADR-046-provider-credential-secret-service.md) |
| `Provider/device-gpu` | `Device` (GPU) + owned cross-domain `Endpoint` | controller/arbitration (Host, system) | [device-gpu](ADR-046-provider-device-gpu.md) |
| `Provider/device-security-key` | `Device` (hidraw), `security-key.d2bus.org.SecurityKeyService`, `security-key.d2bus.org.SecurityKeyBinding` | controller + CTAPHID relay Process + guest frontend Process (Guest, user) | [device-security-key](ADR-046-provider-device-security-key.md) |
| `Provider/device-tpm` | `Device` + owned TPM `Endpoint` | controller + swtpm Process + mandatory flush `EphemeralProcess` | [device-tpm](ADR-046-provider-device-tpm.md) |
| `Provider/device-usbip` | `Device`, `usb.d2bus.org.UsbService`, `usb.d2bus.org.UsbBinding` | initial generic USB implementation; controller + per-Binding attachment Process | [device-usbip](ADR-046-provider-device-usbip.md) |
| `Provider/display-wayland` | qualified `display-wayland.d2bus.org.WaylandPolicy`, `WaylandSession` | controller + Wayland filter-proxy Process | [display-wayland](ADR-046-provider-display-wayland.md) |
| `Provider/network-local` | `Network` | controller + agent/dnsmasq/mDNS Processes (net-VM Guest) | [network-local](ADR-046-provider-network-local.md) |
| `Provider/notification-desktop` | service-only (exports no ResourceType) | host desktop notification sink Processes | [notification-desktop](ADR-046-provider-notification-desktop.md) |
| `Provider/observability-otel` | `telemetry.d2bus.org.TelemetryService`, `telemetry.d2bus.org.TelemetryBinding` | OTEL collector Processes | [observability-otel](ADR-046-provider-observability-otel.md) |
| `Provider/runtime-azure-container-apps` | `Guest` | controller + gateway Guest guest-control | [runtime-azure-container-apps](ADR-046-provider-runtime-azure-container-apps.md) |
| `Provider/runtime-azure-virtual-machine` | `Guest` | controller (cloud full-host) | [runtime-azure-virtual-machine](ADR-046-provider-runtime-azure-virtual-machine.md) |
| `Provider/runtime-cloud-hypervisor` | `Guest` | controller + Cloud Hypervisor VMM Process (Host) | [runtime-cloud-hypervisor](ADR-046-provider-runtime-cloud-hypervisor.md) |
| `Provider/runtime-qemu-media` | `Guest` | controller + QEMU media VMM Process (Host) | [runtime-qemu-media](ADR-046-provider-runtime-qemu-media.md) |
| `Provider/shell-terminal` | qualified `shell-terminal.d2bus.org.ShellPool`, `ShellSession` | controller + per-session PTY supervisor (Host/Guest, user) | [shell-terminal](ADR-046-provider-shell-terminal.md) |
| `Provider/system-core` | `Host`, `User` | fixed core-controller (bootstrap; not a Process) | [system-core](ADR-046-provider-system-core.md) |
| `Provider/system-minijail` | `Process`, `EphemeralProcess` | fixed bootstrap Process controller (not a Process) | [system-minijail](ADR-046-provider-system-minijail.md) |
| `Provider/system-systemd` | `Process`, `EphemeralProcess` | systemd-backed Process/scope controller | [system-systemd](ADR-046-provider-system-systemd.md) |
| `Provider/transport-azure-relay` | transport-only (none) | ZoneLink Azure Relay transport | [transport-azure-relay](ADR-046-provider-transport-azure-relay.md) |
| `Provider/transport-unix` | transport-only (none) | ZoneLink Unix transport | [transport-unix](ADR-046-provider-transport-unix.md) |
| `Provider/transport-vsock` | transport-only (none) | ZoneLink/delegation vsock controller | [transport-vsock](ADR-046-provider-transport-vsock.md) |
| `Provider/volume-local` | `Volume` | controller (Host source-side storage, ACL/quota/marker) | [volume-local](ADR-046-provider-volume-local.md) |
| `Provider/volume-virtiofs` | qualified `virtiofs.d2bus.org.Export` (does not own `Volume`) | controller + virtiofsd Process (Guest-side mount) | [volume-virtiofs](ADR-046-provider-volume-virtiofs.md) |

Volume ownership stays split: `Provider/volume-local` is the sole `Volume`
reconciler and owns Host source-side storage; `Provider/volume-virtiofs` owns
the virtiofsd Process and the qualified `virtiofs.d2bus.org.Export` attachment,
and never adds `Volume` to its exported ResourceTypes.
