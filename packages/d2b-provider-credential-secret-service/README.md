# `d2b-provider-credential-secret-service`

## Provider identity

`Provider/credential-secret-service` manages `Credential` resources. Its
generation changes with its binary, descriptor, or configuration. One
controller runs per `(Zone, User, executionRef)` in a user-domain process.

## Config schema

`collectionAlias` is a non-empty printable-ASCII Secret Service collection
alias of at most 128 bytes; spaces are allowed while quotes, backslashes, and
controls are rejected. `maxLeases` defaults to 64 and is bounded to 1-256.
`lockPolicy` is `fail-closed` or `fail-degraded`.

```nix
d2b.zones.dev.resources.credential-secret-service = {
  type = "Provider";
  spec = {
    artifactId = "credential-secret-service-bin";
    config = {
      collectionAlias = "login";
      maxLeases = 64;
      lockPolicy = "fail-closed";
    };
  };
};
```

## Exported resource types

The Provider manages `Credential` through Pending, Ready, Degraded, Failed,
and deletion cleanup. It projects `CredentialReady`, `RotationDue`,
`ProviderUnavailable`, and `LeaseRevoked`, and owns the
`credential.d2bus.org/provider-revoke` finalizer.

## Controllers / services / workers / binaries

The `d2b-provider-credential-secret-service` binary hosts the user-domain
`secret-service-controller` and `d2b.credential.v3` handler. The injected
`Oo7SecretServicePort` is the only Secret Service boundary.

## Placement and dependencies

Only `user-agent` is accepted, on a same-Zone `Host` or `Guest` with an exact
`User` reference. `host-system` and system-domain `guest-agent` fail with
`credential placement mismatch`. The execution context and user must be Ready
before acquisition. An optional `consumerRef` must name a Provider.

## RBAC requirements

Consumers require `use-credential` with the exact `acquire-token`,
`refresh-token`, `revoke-token`, or `inspect-metadata` subresource also present
in `Credential.spec.allowedOperations`. Wildcards and aliases deny.
Administrative create, update, and delete require ordinary CRUD plus the
matching `admin-credential` subresource.

## Security posture

The port retains credential bytes and Secret Service object paths. Outer
responses contain only opaque digests and metadata; raw values use a dedicated
adapter-authorized Noise KK delivery session. The Provider receives that
binding read-only and cannot select its consumer, audience, route, or limits.
There is no ambient D-Bus or path fallback.

## State and telemetry

There is no Provider state Volume. Bounded observations live in Credential
status and the core operation ledger. Audit permits only authorized bounded
identity digests. Logs, errors, status, OTEL attributes, metric labels, and
Debug output exclude credential and object-path canaries. The test
`process_unique_secret_service_canaries_are_absent_from_every_rendered_surface`
enforces this.

## Build and test

```bash
cd packages && cargo check -p d2b-provider-credential-secret-service
cd packages && cargo test -p d2b-provider-credential-secret-service
make test-integration
make test-host-integration
```

Container service lifecycle and real D-Bus behavior belong in the container
tier. Host and Guest user-session placement, plus generation cleanup and
rollback, require host integration. These scenarios are recorded in
`integration/README.md`; hermetic tests use an in-process fake port.

The crate remains a workspace package until a sibling release defines a public
flake and toolkit compatibility contract. A future standalone consumer must
follow d2b's `nixpkgs` input and use the same Credential service major version.
