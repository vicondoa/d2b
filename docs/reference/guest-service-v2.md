# Guest service v2

**Diataxis category:** reference.

`d2b-guestd` serves typed `d2b.guest.v2.GuestService` and
`d2b.activation.v2.ActivationService` over one authenticated direct
ComponentSession. The frozen daemon and direct guest endpoint schema
fingerprints are:

- daemon combined: `4b2834c89162e5a2c17ea879052c066fd546cdc440d1473955a99e2d9521a54a`
- direct guest: `e6d2fd47db903deff84b5b9cb58a0aed17e2f6ef43010182925890878a15dd3d`

The guest accepts the workload ID supplied by its trusted unit configuration.
Every operation must carry that exact workload scope and the current
ComponentSession generation. Bootstrap checks the parent static-public-key
digest and requested capabilities before sealing the generated guest identity.
Reconnect additionally checks the sealed guest identity handle and both static
public-key digests. Capabilities describe only backends that were ready when
guestd started. A new ComponentSession begins unauthorized: a bootstrap session
cannot run an operation until `Bootstrap` has sealed and confirmed its identity,
and an enrolled session cannot run one until `Reconnect` succeeds.
Immediately before sealing or confirming authorization, the request atomically
crosses a non-cancellable point of no return. Cancellation that wins that race
prevents both sealing and authorization; after the transition, the remaining
mutation is synchronous and cannot be interrupted.

Every typed request is admitted against its absolute expiry, maximum lifetime,
wall-clock skew, session generation, and peer timeout before dispatch.
Idempotency keys bind the method, declared request digest, and exact encoded
request. Matching retries return the recorded result; conflicting or duplicate
requests fail closed. Active request IDs carry cancellation tokens until unary
completion or terminal stream close, and `Cancel` reports the real cancellation
state.

## Runtime credential

Session material is available only through a systemd service credential named
`d2b-guest-session-v2`. The unit must use:

```ini
LoadCredential=d2b-guest-session-v2:/run/d2b-guest-control-host/session-v2
```

`CREDENTIALS_DIRECTORY` is set by systemd. Guestd does not accept a credential
path, PSK, token, or private key through argv, another environment variable, or
a fallback file. The credential directory must be absolute, root-owned, and not
group- or other-writable. The credential must be a root-owned regular file,
must not be a symlink, must have no group or other permission bits, and must be
at most 16 KiB.

The payload is exclusively the shared
`d2b_contracts::v2_component_session::GuestSessionCredentialV1` binary
contract. Guestd uses its shared decoder and opaque
`GuestSessionCredentialBytes`; it has no private magic, layout, fallback, or
parallel decoder. The authoritative 156-byte base and optional 98-byte
`GuestBootstrapCredentialV1` block are documented in
[`component-session-v2.md`](./component-session-v2.md#guest-session-credential).
Bootstrap material is consumed once and zeroed after loading. The generated
guest private key is sealed through `systemd-creds` with TPM2 and never enters a
DTO, Debug value, log, metric, or audit field.

Initial bootstrap uses the shared unbound-identity form: both guest identity
fields are absent and a valid bootstrap block is mandatory. Guestd generates
the private key locally, seals it only after the cancellation point of no
return, and returns the resulting identity digest, exact X25519 public key, and
public-key digest. The next enrolled credential must pin those exact nonzero
values. Reconnect rejects an unbound credential or any digest/public-key
substitution. Enrolled admission and reconnect correlation both require the
identity digest to equal SHA-256 of the exact guest static public key. No private
key is derived from public material or the PSK.

Before the IKpsk2 handshake, the parent sends one length-prefixed 56-byte
evidence record: `D2BBEV2\0`, the 16-byte bootstrap operation ID, and the
32-byte replay nonce. Guestd consumes the PSK only when that peer-supplied
evidence exactly matches the credential binding. Comparing the credential to
itself is not admission.

## Controller runtime connection

The realm controller owns one 32-byte X25519 private key in zeroizing process
memory. A local-root controller loads only the fixed systemd credential
`d2b-controller-static-v2`. A parent-spawned child controller receives the same
authority shape as the allocator-issued, fully sealed, read-only inherited
resource `controller-static-identity-v2`; its fixed descriptor number is
published as `D2B_CONTROLLER_STATIC_IDENTITY_FD`. There is no configurable key
path, key-valued environment variable, generated store key, or on-demand
fallback. Missing, malformed, unsealed, or generation-mismatched authority
leaves guest routes unavailable.

The controller resolves a workload only from the integrity-verified bundle and
an adopted live runner pidfd. The resulting authority binds the controller
generation, runtime-instance digest, active vsock CID and port, and fresh boot
nonce into the ComponentSession channel binding. It connects to the configured
child-realm broker as the exact realm-controller Unix identity; the local-root
broker endpoint is rejected. `BrokerService.Apply` returns two fully sealed
memfds containing the canonical guest session credential and configured-launch
catalog. Enrollment persistence is a second typed `Apply` carrying exactly one
sealed enrolled-credential memfd.

Bootstrap listens on native vsock for the expected guest CID, sends the
operation and replay evidence, and establishes IKpsk2 with the controller key
and the broker-issued single-use PSK. The controller verifies the guest
identity digest, exact public key, parent-key digest, runtime binding, and
generation before persisting enrollment. It closes and erases bootstrap
material before initiating a fresh KK session to the active guest endpoint.
Only that verified enrolled session backs the public daemon GuestService proxy;
endpoint, nonce, identity, generation, cancellation, deadline, or disconnect
mismatches close the session and publish no fallback route.

The allocator starts a child broker with the fixed `serve-child-realm` mode and
the `d2bbr-r-<realm-id>` process title. That mode rejects `SD_LISTEN_FDS` and
adopts only `D2B_BROKER_LISTENER_FD`, `D2B_BOOTSTRAP_SESSION_FD`,
`D2B_CGROUP_LEAF_FD`, and the allocator-created sealed
`D2B_REALM_BROKER_AUTHORITY_FD` and `D2B_REALM_BROKER_GUEST_RUNTIME_FD`.
The authority record binds the realm, controller generation and uid/gid,
broker process identity and uid/gid, session generation, cgroup digest, and
digest of the sealed guest-runtime bootstrap.
The child must run as namespace uid/gid 0, while its `uid_map` and `gid_map`
independently prove that namespace root maps to the recorded host broker
principal; the controller peer must also have an explicit mapping. The sealed
guest-runtime bootstrap supplies only realm-scoped material authority. Its
handler accepts guest-material Apply and persist-enrolled operations and
rejects allocator, host-global, and unrelated broker operations. Local-root
`serve` remains valid only with the single systemd-activated broker listener.

The reciprocal controller namespace maps its own host principal to namespace
uid/gid 0 and the allocator-recorded broker principal to the translated ids in
its sealed controller bootstrap authority. Controller startup verifies both
`uid_map`/`gid_map` entries and rejects overflow ids before any realm-broker
connection. The controller then authenticates broker `SO_PEERCRED` against
those namespace ids, not against host ids.

Enrollment success is durable, not an in-memory acknowledgement. The broker
first stages and fsyncs the enrolled session/configured-launch pair, then
fsyncs a recovery prepare journal containing the prior and replacement pair
digests before changing either credential. It then replaces and fsyncs both
files, commits the enrollment/replay ledger, and fsyncs the pair commit marker.
Success audit is appended only after those fallible commits; audit failure
rolls both transactions back. Before serving after restart, the broker compares
the recovery journal with the committed replay record: a committed enrollment
finishes the replacement, while an uncommitted enrollment restores the prior
pair. The journal also carries the path-free success-audit identity, outcome,
and deduplication key. If pair and ledger are committed but that audit record
is absent, startup appends and fsyncs it exactly once, marks the audit-committed
phase, and only then removes the journal and announces readiness. Incomplete
ledger or audit records are truncated. An uncommitted bootstrap reservation is
never treated as durably consumed.

The success-audit file uses framed `D2BGMA3` records. Each payload and checksum
is fsynced before a separate commit trailer is written and fsynced. Startup
truncates only the final frame lacking a valid trailer; corruption in a
committed or earlier frame fails closed. This is the first release of the audit
surface, so only `D2BGMA3` is accepted. Unshipped development formats fail
closed without truncation or mutation.

The active V3 file rotates before the next frame would cross the largest whole
frame boundary below 64 MiB. Rotation atomically renames and fsyncs it as
`<audit>.v3-segment-<20-digit-index>`, creates and fsyncs a fresh active file,
and retains the newest eight validated segments. Startup validates every
retained segment before pruning excess segments; an interrupted rename with no
active file recreates it without losing or duplicating the sealed segment.

After audit-committed state is durable, cleanup unlinks the main recovery
journal and fsyncs the parent directory before removing any sidecar. A restart
with no journal reaps orphan sidecars; a terminal audit-committed journal does
not require sidecars and is safe to finish repeatedly.

The daemon cache key includes realm, workload, controller generation,
runtime-instance digest, and channel binding. Resolving any different key
closes every prior session for that workload, including a same-generation VM
restart. During bootstrap, the host listener closes foreign-CID connections and
continues waiting for the expected CID under the original timeout; rejected
peers never reset the deadline, and cancellation drops the pending listener.

The guest unit must also pass `--workload-id <canonical-workload-id>`. This is
the same derived workload ID used in `IdentityScope`; it is distinct from the
operator-facing `--vm-id`.

Configured launches use a second optional systemd credential:

```ini
LoadCredential=d2b-guest-configured-launches-v2:/run/d2b-guest-control-host/configured-launches-v2
```

The unit also passes
`--configured-launches-sha256 <lowercase-64-hex-sha256>`. Guestd reads only
that credential name, verifies the SHA-256 of the exact credential bytes, and
uses only the shared `GuestConfiguredLaunchesV1` decoder. The canonical catalog
binds the realm ID, workload ID, nonzero workload-definition digest, configured
IDs, graphical flag, and argv. Its exact header, entry layout, and bounds are
defined in
[`guest-configured-launches-v2.md`](./guest-configured-launches-v2.md). Guestd
additionally requires the decoded workload ID to equal `--workload-id` and the
configured operation scope to equal the catalog realm. There is no private
codec, inventory path option, or ambient fallback. Without a valid nonempty
catalog, `configured-launch` authority is unavailable.

## System activation

`ActivationService` is registered beside `GuestService` only on the direct
daemon-to-guest endpoint. It is not exposed by the public daemon
`GuestService` proxy. Both services share the authenticated session generation,
authorization state, request admission, replay table, and cancellation
registry.

The guest unit supplies absolute `systemd-run` and `systemctl` paths with
`--activation-systemd-run-path` and `--activation-systemctl-path`. Status lives
under the configured activation status directory, which must be a nonsymlink
root-owned `0700` directory outside `/nix/store`. Guestd creates only the fixed
root-owned `0600` configured-intent slot within that directory. Missing or
non-executable binaries, unsafe storage, or a missing or malformed configured
intent leaves activation readiness unavailable.

Before `Activate`, d2bd transfers one bounded, closed binary activation payload through the
contracted `GUEST_ARTIFACT_ID_ACTIVATION_PAYLOAD` file-transfer surface. The
payload binds the workload-derived private intent ID, activation operation ID,
prepared switch program, closed mode, and bounded timeout. The subsequent
`ServiceRequest` carries only that intent ID, operation ID, and payload digest;
it cannot carry a path, argv, environment, or command. Guestd resolves and
digest-checks the root-private payload and accepts only an executable
`/nix/store/<hash>-<name>/bin/switch-to-configuration` target.

Activation runs as guest root in a transient `d2b-activation-*.service` unit
with `Type=exec`, `RemainAfterExit=yes`, `StandardInput=null`,
`KillMode=control-group`, bounded `TimeoutStopSec`, and `RuntimeMaxSec`. The
program and closed mode are separate exec arguments. There is no shell,
`-c`, SSH, token, JSON transport fallback, or host-side execution.

Guestd atomically records root-private running and terminal state before
acknowledging it. `Inspect` reconciles a running record with systemd, persists
success, failure, timeout, cancellation, or loss, and permits a new guestd or
d2bd connection to rejoin the same operation. `Cancel` is generation- and
request-ID-bound and stops only the corresponding transient unit. Matching
idempotent retries return the recorded result; conflicting intent digests fail
closed.

## Terminal streams

`Exec`, `OpenExecRetainedLog`, and `OpenShell` reserve a nonzero named stream on
the server. A successful response is returned only after the backend owner and
bounded stream consumer exist. The host then opens that stream once. Each
logical named-stream message contains exactly one `TerminalStreamFrame`.
Independent client and server sequences start at zero, and every frame repeats
the generation, request ID, operation ID, and resource handle.

The guest owns PTYs, attached process teardown, detached records and logs, and
persistent shell attachments. Arbitrary exec requires delegated admin
authority. Configured launch resolves only an integrity-pinned configured item
ID. All exec paths use the configured non-root workload user through a PAM
login session; there is no root, SSH, or legacy protocol fallback.

Readiness is frozen after checking the configured non-root account, PAM login
configuration, executable files, detached-state directory, shell socket
identity, configured artifact, and shutdown executable. Absolute path syntax
alone never enables a capability.

## Other operations

`FileTransfer` resolves a closed `(artifact, configured intent)` pair. The wire
never carries a path. Application credit bounds chunks, offsets are continuous,
and the final digest covers the complete artifact including a resumed prefix.
Host-to-guest writes use an exclusive same-directory staging file. Guestd syncs
and digest-checks the complete staged artifact, atomically exchanges it with the
live file, and removes the old file. Cancellation or failure before exchange
removes staging and leaves the live artifact unchanged.

`SecurityKey` is advertised only when a guest CTAPHID backend reports ready.
The stream enforces 64-byte reports and explicit approval state. A missing
backend fails before stream reservation. Resume reuses the requested ceremony
handle and the backend must validate that existing ceremony.

`Shutdown` validates its absolute deadline, dispatches the closed power action
with a bounded timeout, and returns a typed final outcome. Concurrent
duplicates wait for the same result; only completed backend success is recorded
as applied, while expiry and failure remain retryable. `GuestService` does not
implement `OpenConsole`.
