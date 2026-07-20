# ComponentSession v2 services

**Diataxis category:** reference.

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

## Fixed per-user endpoints

The systemd user manager owns the two fixed local `SOCK_SEQPACKET` listeners:

| Unit | Path | Named descriptor | Service package |
| --- | --- | --- | --- |
| `d2b-userd.socket` | `/run/d2b/u/%U/userd.sock` | `user-agent` | `d2b.user.v2` |
| `d2b-runtime-systemd-user.socket` | `/run/d2b/u/%U/runtime-agent.sock` | `runtime-systemd-user` | `d2b.runtime.systemd-user.v2` |

Both sockets are mode `0600` below a mode `0700` per-user directory. The shared
activation adapter adopts only the exact named descriptor after validating
`LISTEN_PID`, `LISTEN_FDS`, `LISTEN_FDNAMES`, Unix address family,
`SOCK_SEQPACKET`, listening state, and `CLOEXEC`. The pathname locates the fixed
endpoint but supplies no request or identity authority. Ttrpc services run over
the established ComponentSession driver.

Parent composition must connect each service implementation to those adapters.
The current direct `d2b-userd` and `d2b-unsafe-local-helper` entrypoints remain
unavailable and exit with configuration status rather than exposing another
local protocol.

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

`BrokerService.Apply` method ID `2253834528` materializes guest runtime
credentials when `resource_id` is the exact private-bundle storage ID
`path:workload-guest-session-credential:<workload>`. The authenticated request
also carries the exact realm/workload scope, operation ID, generation, and a
SHA-256 digest over the closed authority inputs. It carries no path, key, PSK,
argv, inventory, or byte payload.

The owning realm broker resolves both storage rows and the configured-launch
inventory from its integrity-verified private bundle. A realm-session authority
connector supplies the exact generation, parent X25519 public key, channel
binding, enrolled guest identity digest/static public key, and optional
operation-bound bootstrap secret. The connector has no ambient file, token, or
environment fallback. The broker verifies the complete request digest before
encoding with the shared `GuestSessionCredentialV1` implementation.

The guest-material dispatcher is installed only in a realm-bound production
handler whose authenticated endpoint roles are `realm-controller` and
`realm-broker` and whose request realm must equal the child broker's launch
realm. The local-root broker has no guest-material authority connector and
denies this resource class.

Success is an atomic pair:

1. response attachment zero is `d2b-guest-session-v2`;
2. response attachment one is `d2b-configured-launch-v2`.

Both are read-only, close-on-exec, fully sealed regular-file memfds. The broker
also atomically replaces the pair at their declared broker-owned runtime
storage rows using the bundle-selected root/private ownership and modes.
Failure while creating the second member restores the prior pair or removes
both new members. The response result digest binds both credential digests.
The opened parent directory itself must be owned by uid 0 and have neither
group nor world write permission. Both prior members are snapshotted before
either replacement. First-member, second-member, rename, parent-fsync, handler
drop, and mandatory-audit failures restore both snapshots. The replacement
transaction remains rollback-armed until the mandatory path-free audit append
succeeds.

The configured-launch payload is encoded exclusively by the shared
`GuestConfiguredLaunchesV1` codec documented in
[`guest-configured-launches-v2.md`](guest-configured-launches-v2.md). The broker
derives canonical realm/workload short IDs, binds the integrity digest of the
exact private workload definition, and includes only its configured exec items.
Argv remains confined to this credential; it is absent from request/response
messages, audit, Debug, errors, and public launcher metadata.

Bootstrap lifetime is at most five minutes and its PSK is single-use.
Consumption is authority-owned and keyed by the exact realm, workload,
bootstrap binding operation ID, and replay nonce—not the Apply request
operation ID. The append-and-sync replay ledger survives connection and
connector recreation. A replay, scope/generation/identity mismatch, stale or
unregistered storage ID, wrong descriptor count, cancellation, or deadline
expiry fails closed. Drop-armed request reservations terminally record
in-progress operations and attempt the same failure audit. Audit records contain
only realm/workload/operation/storage IDs, generations, digests, closed outcomes,
and closed error kinds.

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
count. Error outcomes cannot carry rows or pagination.

The public `DaemonV2` endpoint registers `DaemonService` and a workload-scoped
`GuestService` proxy on one authenticated session. Its schema fingerprint hashes
an ordered `daemon-service`, then `guest-proxy`, package descriptor list. Every
descriptor is domain-separated and length-prefixed and covers package/service
identity, ordered methods, canonical package protobuf, and ordered common and
terminal protobuf dependencies. Daemon-only, guest-only, method, message, or
dependency drift therefore changes the public handshake identity without
ambiguous byte concatenation. That public endpoint does not proxy
`ActivationService`, so activation-only drift does not change its fingerprint.

The authenticated direct daemon-to-guest `GuestV2` endpoint instead hashes an
ordered `guest-service`, then `activation-service`, descriptor list. The guest
descriptor binds common and terminal dependencies; the activation descriptor
binds its common dependency. Activation-only method or message drift therefore
fails the direct guest handshake without widening the public daemon surface.

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

The production guest implementation, identity bootstrap, runtime credential,
backend readiness, and stream ownership rules are specified in
[`guest-service-v2.md`](./guest-service-v2.md).

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
Client frames are selection, bounded stdin, PTY resize, a closed interrupt,
quit, terminate, suspend, or hangup signal, stdin
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
within that range. Denied, cancelled, and failed open responses remain valid
closed results and carry no stream, resource handle, or range; only an accepted
response can construct a live stream validator.

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
or failed closure and rejects all subsequent report or approval traffic.
Conversely, denied completion is invalid unless approval was explicitly denied.
It cannot carry credentials, relying-party
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
