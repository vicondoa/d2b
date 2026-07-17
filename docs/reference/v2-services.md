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

`BrokerService.Allocate` uses typed bounded allocator messages rather than the
generic service envelope. A request names one owner generation and at most 32
opaque resources with closed kind/share and acquisition-order fields. A grant
returns an opaque lease and closed delegations; only file-descriptor
delegations may name attachment indexes. Denials carry one closed reason and at
most 16 opaque conflicts.

`BrokerService.Spawn` uses the typed realm-child pair contract. Its request
names opaque controller/broker process records and binds the exact public
listener, broker listener, bootstrap-session, namespace, cgroup, state, audit,
resource, and lease attachment roles. It carries no executable argv, host path,
credential, UID map, or free-form authority. Success returns exactly one
controller and one broker record with distinct nonzero numeric PIDs, distinct
opaque process IDs, executable digests, and role-bound pidfd attachments:
controller is response attachment zero and broker is response attachment one.

Listener, bootstrap-session, namespace, cgroup, state-root, and audit-root
bindings are singleton authority: each request has at most one binding for a
`(role, kind)` pair, and each such binding must omit `resource_id`. `Resource`
and `lease` bindings instead carry a mandatory opaque `resource_id` and are
unique by `(role, kind, resource_id)`, so one child may receive multiple
distinct delegated resources or leases without duplicating one authority.

Before accepting a response, the receiver validates it against the originating
request. The operation ID and launch-record digest must match, and each child
role must retain the request's exact controller or broker process ID. A swapped
role, pidfd attachment, process ID, duplicate PID, or duplicate pidfd attachment
is rejected. `decode_spawn_response_for_request` performs strict protobuf
decoding and this correlation as one acceptance step. The service schema
fingerprint includes these allocator and child-spawn message shapes.

### Daemon result projections

`DaemonService.ListRealms`, `ListWorkloads`, and `Inspect` return generated
protobuf projections rather than digests, generic observations, private bundle
objects, or JSON. Realm rows carry canonical realm identity, placement mode,
gateway identity when applicable, lifecycle state, cross-realm policy,
credential boundary, and generation. Workload rows carry canonical
realm/workload identity, environment, lifecycle and pending-restart state,
runtime kind and capabilities, graphics/TPM/USB posture, service-role states,
network address bytes, deployment references, readiness, runner parity,
console-media details, USB summary, and degraded details used by list and
status renderers.

Every repeated field has a closed maximum and every string has a byte limit.
Runtime, lifecycle, service, capability, autostart, readiness, and media state
use closed enums. Responses contain at most 64 realms or 256 workloads.
`PageInfo.returned_items` must equal the encoded row count; truncation requires
a bounded next cursor, and a supplied total cannot be smaller than the returned
count. Error outcomes cannot carry rows or pagination. The daemon service
fingerprint hashes the canonical daemon and shared terminal protobuf sources,
so a projection or stream shape change changes the advertised schema identity.

### Shared terminal streams

The transport-neutral `d2b.terminal.v2` package is shared by
`DaemonService.Exec`/`Shell`/`OpenConsole` and
`GuestService.Exec`/`OpenShell`. Guest methods wrap `TerminalOpenRequest` in
method-specific requests whose validation requires an exact workload scope:
realm and workload IDs with no provider or role. `TerminalOpenRequest` has no
stream-id field. After admission, the server reserves a ComponentSession named channel in the
range `0x0100..=0xffff` and returns its canonical `stream-N` spelling plus a
bounded opaque resource handle. The client opens exactly that returned stream,
once. A client cannot name, preselect, reuse, or cause the server to open a
caller-selected channel.

The first logical message is a client-to-server `TerminalSelection` matching
the opening method. Exec selection is either bounded arbitrary argv with the
closed `admin-arbitrary` authority or one opaque configured-item ID with the
closed `configured-launch` authority. The latter is resolved from the
integrity-checked bundle; it carries no argv. Shell selection uses closed
attach-default, attach-configured, list, detach, and kill actions with opaque
configured IDs or server-issued handles. Console selection carries only
terminal shape. Retained-log selection carries an exec handle, closed output
stream, requested offset, and maximum byte count. No selection carries environment variables, a working
directory, a host path, credentials, or provider-native data.

Each frame binds the exact authenticated session generation and 16-byte opening
request ID, operation ID, and resource handle. Client and server frame
sequences are independent, start at zero, increase by one, and are capped.
Client frames are selection, bounded stdin, PTY resize, a closed signal, stdin
close, detach, close, or cancellation. Server frames are start acknowledgement,
bounded stdout/stderr, closed status, shell-management result, or one terminal
outcome. Resize is valid only for a selected PTY. Detached exec rejects PTY and
streamed I/O and terminates the stream with the detached outcome while the
guest-owned job continues. Once a client requests detach, close, or
cancellation, only in-flight server output/status and the terminal outcome
remain valid. No frame is valid after the first terminal outcome.

ComponentSession credits, close, and reset remain transport controls rather
than application frames. Credit is nonzero and bounded; close or reset is
accepted by the application validator only after the typed terminal outcome.

Arbitrary argv is limited to 256 UTF-8 arguments, 4 KiB per argument and
64 KiB total. Terminal chunks are limited to 64 KiB. Generated Debug
implementations for stream/opening messages and oneofs are redacted; validation
errors are closed slugs. Runtime logging, audit, traces, and metrics must never
format raw generated messages or record argv, terminal bytes, configured item
IDs, shell names, request IDs, stream IDs, environment, working directories, or
paths.

### Guest operations

`GuestService.Bootstrap` and `Reconnect` use typed requests and responses bound
to the exact ComponentSession generation and request/operation IDs. They
confirm the guest and parent static public-key digests, an opaque guest identity
handle, and at most 32 closed capabilities. Bootstrap PSKs and static private
keys remain handshake-only and are not representable in the service payload.

`CancelExec` names the exact exec resource handle, generation, request, control
sequence, and closed cancellation reason/outcome. Signalled cancellation is
`accepted`, already-terminal is `not-applicable`, and unknown-resource or
generation-mismatch are typed failures; other combinations are rejected.
`InspectExec` uses one closed read-only query: status, bounded wait, or
detached-exec list page.
Status and list results contain only closed lifecycle/stdin state, offsets,
retention counters, opaque handles, and argv digests. Exec state, terminal
outcome, and stdin state must agree. Wait results cannot regress the caller's
known state generation; timeout is valid only when no newer nonterminal state
exists.

`OpenExecRetainedLog` is a separate mutating operation requiring an idempotency
key, so ambiguous retries cannot allocate multiple streams. Its terminal
response binds output stream, requested offset, selected start/end range,
maximum byte count, and EOF posture. The first retained-log selection repeats
the exact resource/output/offset/limit, and output frames must remain contiguous
within that range.

`FileTransfer` names one closed artifact ID and one opaque configured-intent ID;
paths are not representable. Its stream binds generation/request/operation/
resource, direction, offset, declared size, optional expected digest, 64 KiB
chunks, EOF/final digest, bounded credit, cancellation, and one completion or
error outcome. Chunks require previously granted application credit, debit it
exactly, and preserve the accepted EOF total/digest through completion.
Total transfer size is capped at 16 MiB.

`SecurityKey` names only opaque device and ceremony handles and a closed
ceremony kind. Its stream carries exactly 64-byte CTAPHID reports in explicit
guest/device directions, closed approval request/decision, cancellation, and
one completion or bounded error. A ceremony requiring approval cannot complete
successfully before an explicit grant; denial permits only denied, cancelled,
or failed closure. It cannot carry credentials, relying-party
secrets, device paths, or arbitrary diagnostics.

`Shutdown` carries one closed power action and an absolute deadline within the
request lifetime. Its response is either accepted with no final result or a
strict final completed/already-applied/cancelled/failed outcome. Console remains
host-owned; `GuestService` has no `OpenConsole` method.

Every guest request, response, nested oneof, and stream frame rejects unknown
fields, unknown or unspecified enums, over-limit data, generation/request/
operation/resource mismatches, invalid direction or sequence, duplicate
terminal outcomes, and mixed success/error fields. Generated Debug output is
redacted for all guest and shared-terminal messages.

## Bounds and strictness

A protobuf message is at most 1 MiB. Strings and opaque IDs are at most 64
bytes unless a field declares the 128-byte cursor, 256-byte detail, or 512-byte
reference bound. Digests are 32 bytes, a page has at most 256 observations or
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
