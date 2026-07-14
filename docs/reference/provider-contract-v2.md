# Provider contract v2

The canonical provider DTOs live in
`d2b_contracts::v2_provider`. The same object-safe traits and request/result
types are defined for trusted first-party in-process adapters and authenticated
provider-agent proxies over ComponentSession service `d2b.provider.v2` in later
implementation waves.

## Authority

Every provider has exactly one primary authority:

`runtime`, `infrastructure`, `transport`, `substrate`, `credential`, `display`,
`network`, `storage`, `device`, `audio`, or `observability`.

Capabilities are positive, closed method claims. Unknown claims fail
deserialization, missing required claims reject the descriptor, and a claim
from another authority rejects the complete registry generation. Runtime
descriptors additionally declare process, cgroup, network namespace, user
namespace, persistent identity, and device-mediation posture.

## Placement and credentials

Placement is either an audited trusted adapter in its owning controller or a
provider agent identified by canonical realm, workload, and role IDs. Agent
placement must use ComponentSession role `provider-agent` and service
`d2b.provider.v2`.

Credential operations return only `CredentialLease`. A lease contains an
opaque ID, closed SDK operation classes, generations, expiry, rotation, and
revocation state. It is usable only when the credential and consumer providers
have the same exact agent placement binding. Lease transfer is forbidden.
There is no serialized secret, byte stream, environment, file, or descriptor
return path.

## Lifecycle

Every serialized operation is bound to an already-authorized scope, provider
ID and authority, registry generation, method/capability, operation ID,
idempotency key, request digest, policy epoch, decision digest, expiry, and
audit/trace correlation. Plans and handles retain that binding.
Operation scopes are realm, workload, or workload-role scopes. Controller
authority comes from authenticated ComponentSession state rather than a
payload-supplied role ID; realm-controller handle ownership is an explicit
realm-scoped owner kind.

Handles bind realm, optional workload, owner, provider and resource
generations, and configuration fingerprint. Ownership transfer is explicit,
single-use, expiring, and realm-local. Adoption verifies all bindings.
Multiple candidates produce a failed, quarantined observation and never admit
mutation.

Registry snapshots contain all eleven axes in canonical order, globally unique
provider IDs, unique factory keys, immutable generations, and bounded
descriptors. Several configured instances may use one factory key. Updates
replace the complete generation transactionally and carry a fail-closed drain
policy. Selection never probes an unconfigured fallback.

## Artifacts

- [`provider-contract-v2.schema.json`](./provider-contract-v2.schema.json) is
  the exact generated JSON Schema for `ProviderContractDocument`.
- [`provider-contract-v2-fixture.json`](./provider-contract-v2-fixture.json)
  is the canonical complete fixture.
- Fingerprint
  `025a883c7e6975a797bae9fe74483a5f96a16adfd27d1e2d31a63f5a0fcd2312`
  binds the canonical compact schema and fixture with its fingerprint field
  zeroed. The focused Rust contract test rejects any drift.
