# Realm access resolver contract

**Diataxis category:** reference.

The realm access resolver contract defines how a client turns an operator
supplied realm target into a typed access binding. It is a DTO and resolver
behavior contract only: the current runtime still uses the existing global
`d2bd` public socket and `d2b.envs` VM substrate. Identity enrollment, realm
routing, provider adapters, relay transport, and realm-local lifecycle APIs are
future runtime work.

## Target grammar

A canonical realm target names a workload in a realm path:

```text
<workload>.<realm>[.<ancestor>...].d2b
```

Examples:

```text
builder.dev.d2b
browser.work.d2b
api.payments.work.d2b
```

The first label is the workload. Labels after that, before the reserved
`.d2b` suffix, are the realm path written most-specific first. A target is not
a DNS, SSH, IP, vsock, or route address, and it never encodes a physical node.
The resolver finds placement from the owning realm metadata.

Fully qualified public targets must end in `.d2b`. The labels `all`, `*`, and
non-suffix `d2b` are reserved and are refused as target labels. The optional
`d2b://` scheme may be accepted by compatibility parsers, but canonical output
always renders without a scheme.

## Bare aliases and default realms

Bare workload names are not self-contained targets. Callers that accept them
must provide resolver context:

- an explicit alias table mapping a bare workload name to one canonical target;
- or default-realm metadata that appends a selected realm to an otherwise bare
  workload name.

Alias resolution is fail-closed. If a bare alias maps to multiple candidates,
the resolver returns an `alias-ambiguous` diagnostic with bounded candidate
metadata and asks the caller to use a fully qualified target. If the default
realm was used, the response records the selected realm, whether it was applied,
and the source of the choice (`configuration`, `explicit-request`, or
`local-compatibility`).

## Direct host-local access bindings

Host-local realm access is a direct Unix socket contract. A successful
host-local resolution returns `DirectHostLocalUnix` with:

- the absolute bounded public socket path for the owning realm;
- `peerCredentials.source = connecting-client-process`;
- `peerCredentials.checkedBy = d2bd-public-socket`;
- `peerCredentials.proxy = no-byte-proxy`.

That shape is deliberate: an authorized local client connects to the owning
realm socket directly so the daemon can check the original process with
`SO_PEERCRED`. The global host daemon is not a byte proxy for realm sockets.
Remote realm and provider results are references only; they carry non-secret
transport/provider identifiers, not credentials or endpoint secrets.

See [Realm controller configuration](./realm-controller-config.md) for the
private `realm-controllers.json` rows that reserve the host-local socket,
principal, state, audit, and allocator metadata consumed by this contract.

## Capability preflight

A resolver request carries the caller's required capability set and the binding
kinds the client can consume. A resolver response carries a preflight snapshot:
required capabilities, advertised capabilities, and either `satisfied` or a
typed denial with missing capabilities.

Capability preflight denies before execution. Supported denial reasons are:

| Reason | Meaning |
| --- | --- |
| `missing-capability` | The selected realm/controller/workload does not advertise a required capability. |
| `unsupported-cross-realm-capability` | The requested capability is not exportable through the selected cross-realm placement. |
| `missing-realm-controller` | The selected realm has no controller binding in the resolver input. |
| `stale-realm-controller` | The binding generation does not match the expected controller generation. |

This preflight is not authorization by itself. Future runtime paths still need
realm identity, policy checks, idempotency, and bounded audit before mutating or
streaming operations execute.

## Typed diagnostics

Resolver failures use bounded, client-safe diagnostics rather than free-form
transport errors:

| Diagnostic | When it appears |
| --- | --- |
| `alias-ambiguous` | A bare alias matches more than one canonical target. |
| `old-node-qualified-target` | An input uses an obsolete node-qualified form; the diagnostic includes the suggested realm target with the node label removed. |
| `missing-realm-binding` | The target realm exists as an input concept but has no selected access binding. |
| `unsupported-cross-realm-capability` | A requested capability cannot cross the selected realm boundary. |
| `stale-realm-controller` | The resolver observed a different or missing controller generation than the caller expected. |
| `missing-realm-controller` | No controller metadata was available for the target realm. |

Diagnostics may include bounded related diagnostics and conflict candidates.
They are safe for CLI output and audit because they carry validated ids,
canonical targets, capabilities, generation ids, and non-secret binding
references only.

## Current implementation boundary

The committed contract provides typed DTOs, parser behavior, generated schemas,
and private host-local controller metadata. It does not yet:

- route `d2b` commands through per-realm daemons;
- replace existing VM names or `d2b.vms.<vm>.env` placement;
- implement realm identity enrollment, policy evaluation, or remote relay
  sessions;
- start provider adapters or provider-specific controllers;
- allocate realm-owned networks or migrate current `d2b.envs` networking.

Use the canonical grammar and diagnostics when documenting or testing future
realm-aware CLI behavior, but do not assume these contracts move current core
VM lifecycle operations onto per-realm routing.

## Related references

- [Realm core model reference](./realm-core.md)
- [Realm controller configuration](./realm-controller-config.md)
- [Realm option schema](./realm-options.md)
- [CLI contract](./cli-contract.md)
- [Naming conventions](./naming-conventions.md)
