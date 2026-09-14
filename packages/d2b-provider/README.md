# `d2b-provider`

The Zone-side Provider model surface: descriptors, registry, session identity,
and lifecycle admission.

Neither this crate nor `d2b-provider-toolkit` is a Provider. Both are
Provider-neutral common libraries: they declare no Provider identity, own no
ResourceType, and ship no component binary, so the per-Provider sections below
are answered from that position rather than left as placeholders.

## Provider identity

None. This crate is the Zone's registry of installed Providers, not one of
them. A common library cannot register a second Provider identity or become a
hidden multi-Provider composition binary.

## Config schema

None. A Provider's root configuration is authored in the Provider resource's
two-field spec (`artifactId` and `config`) and validated against the signed
manifest's root JSON Schema. This crate reads neither.

## Exported resource types

None. It consumes the `Provider` ResourceType contract in
`d2b-contracts::v3::provider` and owns no ResourceType of its own.

## Controllers / services / workers / binaries

None. The crate is a library with no binary target.

## Placement and dependencies

It is linked into the Zone runtime rather than placed as a Process. It depends
only on the shared v3 contract catalog.

## RBAC requirements

None of its own. Every type here records an authorization decision that has
already been reached elsewhere: an `AdmittedProvider` is an admission that
happened, and an `InFlightPermit` is a concurrency slot - neither is a
capability that can be presented.

## Security posture

No host mutation, no socket, no path resolution, and no numeric UID or GID,
device node, store path, or socket path in any public type. Registry admission
is a conjunction that fails closed at each clause: the authenticated principal
must own the target row's Zone, the row's generation must match the registry
generation the caller observed, and the Provider's published method surface
must already be registered.

## State and telemetry

No persistent state. The registry generation lives in memory for the lifetime
of the Zone runtime that holds it. Diagnostics are redacted: a descriptor
renders its family, generations, and capability count.

## Build and test

```bash
bazel test //packages/d2b-provider:d2b_provider_test
```
