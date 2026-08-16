# `d2b-provider-runtime-cloud-hypervisor`

Canonical implementation of `Provider/runtime-cloud-hypervisor`.

## Provider identity

The implementation identifier is `cloud-hypervisor`. It reconciles local
`Guest` resources and owns the VMM Process lifecycle through a typed effect
port.

## Config schema

`CloudHypervisorConfig` requires an explicit Host execution reference and
validates bounded VCPU, memory, health, adoption, and startup settings.
Guest settings require a top-level system artifact and reject raw locators.

## Exported resource types

The Provider reconciles `Guest` and creates the semantic VMM Process through
Core. Device, Network, and Volume resources remain owned by their Providers.

## Controllers / services / workers / binaries

`CloudHypervisorController` gates launch on Device, Network, and Volume
readiness, adopts exact process identity, probes authenticated guest-control,
and finalizes guest-control before the VMM process.

## Placement and dependencies

The controller runs on an explicit Host, while the VMM is broker-spawned and
daemon-supervised. No per-VM systemd unit or direct broker socket is used.

## RBAC requirements

Only opaque attachment refs and typed launch effects cross the Provider
boundary. Pidfds are opened after PID, start-time, cgroup, executable,
template, and generation evidence is verified.

## Security posture

Guest readiness requires authenticated guest-control health, not only process
existence. Ambiguous adoption is quarantined and never broadly killed.

## State and telemetry

Status contains only bounded readiness and lifecycle fields. VMM paths, argv,
PIDs, store paths, and guest identity bytes are absent from public projections.

## Build and test

```text
cargo test -p d2b-provider-runtime-cloud-hypervisor
```

Host/KVM acceptance remains a separate manual host-integration lane.
