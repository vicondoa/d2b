# ComponentSession v2 services

This reference defines the internal protobuf/ttrpc services carried by the
reserved ComponentSession control channel. The machine-readable inventory is
[`v2-services.json`](./v2-services.json); its JSON Schema is
[`v2-services-schema.json`](./v2-services-schema.json). The committed `.proto`
sources under `packages/d2b-contracts/proto/v2/` and generated Rust under
`packages/d2b-contracts/src/generated_v2_services/` are the wire authority.

## Service packages

The closed package set is:

- `d2b.daemon.v2`
- `d2b.realm.v2`
- `d2b.guest.v2`
- `d2b.provider.v2`
- `d2b.broker.v2`
- `d2b.user.v2`
- `d2b.runtime.systemd-user.v2`
- `d2b.shell.v2`
- `d2b.clipboard.v2`
- `d2b.clipboard.picker.v2`
- `d2b.notify.v2`
- `d2b.security-key.v2`
- `d2b.wayland.v2`
- `d2b.activation.v2`
- `d2b.tty.v2`

No earlier service package is registered. Service and method names, stable
method IDs, mutation classification, idempotency requirements, message bounds,
and lifetime caps are listed in the inventory JSON.

Protobuf package identifiers cannot contain a hyphen. The descriptor-only
spellings for `runtime.systemd-user` and `security-key` therefore use
`runtime.systemd_user` and `security_key`; checked-in code generation rewrites
the generated ttrpc method and registration paths to the exact ComponentSession
package names above. Tests fail if either dispatch path retains an underscore.

## Authority and payload rules

Caller identity is authenticated by ComponentSession and is not a request
field. Required capability is generated service/method policy and is not a
request field. Requests carry canonical v2 realm/workload/provider/role IDs,
opaque resource or operation IDs, fixed-size digests, closed enums, bounded
stream IDs, and attachment indexes only. They cannot carry credentials, secret
bytes, raw paths, commands, environment, provider-native responses, or
free-form execution authority.

Every admitted request has an exact 16-byte request ID, optional bounded
correlation and W3C trace IDs, authenticated issue and absolute-expiry times,
and a nonzero ComponentSession generation. Mutations additionally require a
bounded idempotency key. Absolute lifetime is at most 15 minutes. A receiver
intersects the authenticated remaining lifetime with its monotonic budget,
method cap, and peer ttrpc relative timeout; `timeout_nano` never carries epoch
time.

Cancellation uses the session-control request ID and generation. Cancellation
cannot create a replacement operation ID or replay an ambiguous durable
mutation. Provider calls carry provider identity/type/generation, policy epoch,
authorization digest, and request digest; credential operations return only
opaque lease handles.

`ProviderRequest` carries a mandatory `ProviderOperationInput` oneof. Its
variants map exactly to the canonical provider input union: no input,
configured item ID, infrastructure power state, transport binding ID, storage
snapshot ID, device selector ID, audio state, observability query, or
observability export. The removed generic `binding_id`, `desired_state`, and
`stream_id` fields and their tags are reserved and rejected. Method-specific
compatibility is validated before provider dispatch.

An `observability-query` success uses the structured
`ProviderResponse.observability_query_result` field. Its bound observation,
closed record labels, records, cursor, byte upper bound, and truncation state
round-trip the canonical Rust result directly; they are never encoded as JSON
or opaque bytes. Query results cannot be mixed with generic observations,
resource or stream handles, result digests, attachments, or errors. Error
responses carry no query result, and every non-query provider method rejects
that field. Provider-service schema fingerprints include this response shape.

## Bounds and strictness

A protobuf message is at most 1 MiB. Strings and opaque IDs are at most 64
bytes, digests are 32 bytes, a page has at most 256 observations or
observability records, and a request references at most 64 unique
ComponentSession attachments. An observability result declares at most 1 MiB
of encoded records. Decode rejects unknown protobuf fields, unknown enum
values, unspecified enum sentinels, invalid canonical IDs, duplicate
attachment indexes, missing metadata, missing or mismatched provider input,
over-limit values, inconsistent result fields, and mutation requests without
idempotency. Strictness applies recursively to every nested operation input
and observability result. JSON inventory and schema fixtures deny unknown
fields.

Generated bindings use only `ttrpc::r#async` client, handler, service, and
server traits from the pinned runtime stack. They define wire dispatch only;
queueing, scheduling, cancellation-token implementation, transport, and
provider execution remain runtime concerns.
