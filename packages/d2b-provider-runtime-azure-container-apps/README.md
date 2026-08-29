# `d2b-provider-runtime-azure-container-apps`

Canonical implementation of `Provider/runtime-azure-container-apps`.

## Provider identity

The implementation identifier is `azure-container-apps`. It reconciles remote
ACA sandbox Guests while keeping the controller and deployment service inside a
configured gateway Guest.

## Config schema

`AcaProviderConfig` validates the gateway Guest, Credential refs, optional
Network ref, bounded Azure identifiers, and Provider defaults. Guest profiles
validate CPU, memory, image source, auto-suspend, readiness, and operation
ledger bounds.

## Exported resource types

The Provider reconciles `Guest` and owns semantic sandbox-agent Endpoint
observations. A managed ACA sandbox is not a Zone or a ZoneLink.

## Controllers / services / workers / binaries

`AcaController` performs observe, adopt, ensure, start, stop, destroy, and
finalize through `AcaControl` and `AcaCredentialLeaseClient`. The deployment
service dispatches bounded lifecycle requests and never self-spawns a process.

## Placement and dependencies

All cloud control and credential use is gateway-Guest local. No Host process,
Host Credential, ambient SDK credential chain, or Provider-owned persistent
service is used.

The Zone-native controller and effect contracts use the v3 Provider and
Resource contract crates directly. The legacy `gateway` module remains only
for the existing Gateway DAG consumer; its removal is deferred until that
consumer is migrated.

## RBAC requirements

Callers provide an operation-bound opaque ID and a bounded deadline. Effect
ports are the only mutation boundary; ambiguous adoption fails closed.

## Security posture

Credential leases expose only opaque metadata. Status, Debug, audit, and
metrics never contain sandbox IDs, endpoints, tokens, paths, or ZoneLink
authority.

## State and telemetry

Bounded observed identity digests and lifecycle phases are status-first.
Completed operation entries are bounded and expirable. Audit events and metric
labels use closed semantic values.

## Build and test

```text
make test-rust
```

Tests use real typed effect objects and in-process fakes. No Azure account or
network access is required.
