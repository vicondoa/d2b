# `d2b-provider-guest-azure-virtual-machine`

Canonical implementation of `Provider/runtime-azure-virtual-machine`.

## Provider identity

The implementation identifier is `azure-vm`. It reconciles cloud-backed Guest
resources through an opaque Azure effect port.

## Config schema

`AzureVmConfig` requires a gateway Guest execution boundary and an ARM
Credential ref. `AzureVmGuestSettings` validates placement, VM shape, bounded
data disks, bootstrap delivery, and operator tag rules.

## Exported resource types

The Provider reconciles `Guest`. Azure VM resources and operation handles stay
inside the effect adapter and are represented externally only by digests.

## Controllers / services / workers / binaries

`AzureVmController` implements non-blocking LRO provisioning, bootstrap
delivery, restart adoption on reconcile, and finalization. `BootstrapService`
performs the one-time PSK admission transition to enrolled KK.

## Placement and dependencies

ARM credentials and bootstrap state are gateway-Guest local. The Host does not
hold realm credentials, ARM URLs, PSKs, or remote node registries.

## RBAC requirements

Every operation has a deterministic idempotency key and uses injected typed
effect and credential ports. Finalizers remain installed through ambiguous or
incomplete deletion.

## Security posture

Operation handles, VM handles, tags, PSKs, and tokens have redacted Debug
implementations. Bootstrap PSKs are zeroized and single-use.

## State and telemetry

The controller keeps bounded non-secret lifecycle state for restart recovery;
the framework Guest adapter owns the published Guest status. ARM LRO polling is
requeue-driven.

## Build and test

```text
make test-rust
```

Tests use scripted effect ports and do not contact Azure or a host daemon.
