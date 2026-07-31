# `d2b-credential-service`

Strict Credential service DTOs plus unregistered client and server halves.

## Provider identity

The service package is `d2b.credential.v3` and manages `Credential` operations.
It is provider-neutral: a selected Credential Provider implements the server
trait, and no Provider generation policy is owned by this crate.

## Config schema

This crate has no configuration schema. It consumes the canonical Credential
base spec from `d2b-contracts`, including `allowedOperations`, scope, audience,
rotation, expiry, and revocation policy.

## Exported resource types

The service operates on the standard `Credential` ResourceType. It exports no
new ResourceType and does not alter the closed standard catalog.

## Controllers / services / workers / binaries

`CredentialClient` exposes the exact five typed calls. `CredentialServer`
enforces admission before invoking a `CredentialProvider`. The crate ships no
worker or binary and registers no production listener.

## Placement and dependencies

Placement comes from the Credential base contract (`user-agent`, `host-system`,
or `guest-agent`) and is checked by the selected Provider and trusted admission
layer. The crate depends on `d2b-contracts`; a future authenticated bus adapter
must translate its closed admission vocabulary to the native resource API.

## RBAC requirements

Every operation requires `use-credential` with the method's exact canonical
subresource and the same operation in `spec.allowedOperations`. Credential
lifecycle administration requires both the ordinary CRUD verb and the matching
supplemental `admin-credential` subresource (`create`, `update-spec`, or
`delete`). Empty, wildcard, unknown, and method-name aliases are denied.

## Security posture

Outer DTOs contain only one-way lease/source representations and non-secret
metadata. Token and signature bytes exist only in `SensitiveDeliveryRecord`,
which redacts diagnostics and clears its storage explicitly and on drop. The
delivery binding permits only enrolled KK sessions; production bus wiring is
intentionally absent so this crate cannot create an alternate identity or
authorization path.

## State and telemetry

The crate persists no state and emits no telemetry. Service errors are closed,
field-free codes. Credential identity, request IDs, audience, route bindings,
and plaintext records are redacted from `Debug` and error surfaces.

## Build and test

```bash
cd packages && cargo check -p d2b-contracts -p d2b-credential-service
cd packages && cargo test -p d2b-contracts -p d2b-credential-service
```

Cross-process bus routing and delivery-session cryptography require the later
authenticated production registration path and host/container integration.
