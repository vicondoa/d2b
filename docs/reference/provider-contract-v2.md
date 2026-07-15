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
audit/trace correlation. `ProviderOperationRequest.input` is mandatory and
closed; plans and handles retain the operation binding.
Operation scopes are realm, workload, or workload-role scopes. Controller
authority comes from authenticated ComponentSession state rather than a
payload-supplied role ID; realm-controller handle ownership is an explicit
realm-scoped owner kind.

### Operation input

Each method accepts exactly one input shape:

| Method | Input |
| --- | --- |
| `runtime-execute` | configured runtime item ID |
| `infrastructure-set-power-state` | `running` or `stopped` |
| `infrastructure-bootstrap-binding`, `transport-revoke-binding` | transport binding ID |
| `storage-snapshot` | snapshot ID |
| `device-plan-attach` | device selector ID |
| `audio-set-state` | speaker/output or microphone/input plus mute and/or volume |
| `observability-query` | closed view, optional cursor, and limit |
| `observability-export` | closed format and bounded time range |
| every other method | explicit `no-input` |

Plan-consuming methods still receive the bound `ProviderPlan` and accept
`no-input`; plan authority is never duplicated into operation input. Configured
runtime execution carries only a bounded configured item ID, never argv,
environment, working directory, or path. Runtime execution remains
non-dispatchable and may not be advertised by a registered provider until the
typed execution path exists end to end.

All operation identifiers are bounded canonical opaque identifiers. The query
cursor is at most 64 bytes, query limit is 1 through 256, volume is 0 through
100, and an export range is ordered, safe for JSON integers, and at most 31
days. The input union has no JSON escape hatch, endpoint, credential, command,
filesystem path, raw query, or arbitrary label field. Identifier-bearing
inputs redact their values from `Debug`.

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

## Provider toolkit

`d2b-provider-toolkit::ProviderValues` constructs descriptor-bound health,
plans, request- or plan-derived handles, observations, mutation receipts, and
closed failures. Its constructors validate and preserve provider, operation,
owner, generation, fingerprint, correlation, and timestamp bindings, while its
`Debug` output omits identifiers.

`Fixture::from_descriptor` runs conformance against the caller's exact
descriptor, placement, and target. `Fixture::new` is only the deterministic
fake-provider convenience. Registration, provider-agent serving, and toolkit
admission share the dispatchability policy from `d2b-provider`; they do not
maintain independent method allowlists.

## Artifacts

- [`provider-contract-v2.schema.json`](./provider-contract-v2.schema.json) is
  the exact generated JSON Schema for `ProviderContractDocument`.
- [`provider-contract-v2-fixture.json`](./provider-contract-v2-fixture.json)
  is the canonical complete fixture.
- Fingerprint
  `f95fd0dbf69959090cb9ccead1b80b395597b7f0aeab5b4cf91ab603ec2773bf`
  binds the canonical compact schema and fixture with its fingerprint field
  zeroed. The focused Rust contract test rejects any drift.
