# v3 replacement contracts for desktop clients

**Diataxis category:** reference.

**Contract version:** d2b 3.0 (v3) replacement surface.

This is the actionable contract for desktop clients consuming d2b's current
Zone resource plane. It describes the public socket, ResourceRefs, Guest
lifecycle, shell sessions, launcher operations, and redaction boundaries.

## Source precedence

Use these sources in order:

1. the generated ResourceType or CLI schema;
2. the matching reference page;
3. human examples only for presentation.

Committed code is authoritative when prose drifts. A client must fail visibly
on an unsupported contract; it must not guess a retired shape or fall back to
SSH, a static manifest, or a raw broker operation.

## Public daemon transport

Clients use `/run/d2b/public.sock`, an AF_UNIX `SOCK_SEQPACKET` socket. The
frame is a four-byte little-endian body length followed by one UTF-8 JSON
object. Bodies larger than 1 MiB are rejected before JSON decoding.

The first frame is:

```json
{
  "type": "hello",
  "clientVersion": ">=3.0.0",
  "supportedFeatures": ["typed-errors"]
}
```

The daemon replies with `helloOk` and the selected version/capabilities, or
`helloRejected` with a typed reason. The client must use the `selectedVersion` and returned feature list from `helloOk`. Unknown fields and malformed frames fail closed.

After the handshake, Resource requests carry a Zone route assertion:

```json
{
  "type": "resourceRequest",
  "service": "d2b.zone.v3",
  "method": "List",
  "zoneRef": "Zone/local-root",
  "resourceType": "core.d2bus.org.Guest"
}
```

`zoneRef` never grants access or selects a private socket. Peer credentials,
Zone session state, Resource UID/generation/revision, Provider generation,
and capability evidence are checked by the daemon before dispatch.

## Resource identity

Use `ResourceType/name` addresses:

```text
Zone/work
Guest/work-app
Process/work-app-worker
EphemeralProcess/exec-1
Provider/runtime-cloud-hypervisor
```

Names are bounded lowercase labels. The name resolves in the selected Zone;
store-assigned UIDs fence the resolved incarnation. A same-named Guest in two
Zones is two different resources.

## Guest lifecycle

```text
d2b guest start work-app --zone work --apply
d2b guest stop work-app --zone work --apply
d2b guest restart work-app --zone work --apply
d2b guest status work-app --zone work
```

The wire operation is `Start`, `Stop`, or `Restart` with a canonical
`Guest/<name>` `resourceRef`, `force`, `dryRun`, `apply`, and
`waitForReady`. It is never encoded as an arbitrary update or a static
manifest lookup.

The Guest controller creates and observes its direct child Resources through
the authenticated Resource API. It waits for required Process, Endpoint,
Volume, Network, Device, Provider, and Guest-session status before reporting
Ready. Deletion drains descendants in reverse dependency order and clears the
Guest finalizer only after the session and owned children are gone.

## Guest execution and shells

Execution creates an `EphemeralProcess` Resource:

```text
d2b exec run Guest/work-app -- /bin/sh
d2b exec status EphemeralProcess/exec-1
d2b exec logs EphemeralProcess/exec-1
d2b exec kill EphemeralProcess/exec-1
```

Persistent shells use `ShellSession` Resources:

```text
d2b shell open Guest/work-app --name build
d2b shell attach ShellSession/build
d2b shell detach ShellSession/build --apply
```

Guest execution and shell I/O use an authenticated ComponentSession or
ProcessAttachClient. The client never supplies a host path, raw credential,
pidfd, cgroup path, executable locator, or private runtime scope.

Persistent shell operations are admin-only. A shell client must not open a hidraw device, use private proxy paths, or read private d2b state to recover a missing session. It never contains host credentials, raw locators, or broker
handles in a public request.

## Launcher operations

Configured launchers send only a canonical target, item ID, and operation ID:

```text
d2b launch Guest/work-app --item browser
```

Provider and shell items are resolved by the Zone resource graph. Launcher
metadata is argv-free and public-safe; an omitted or unavailable item returns
a typed response rather than a guessed command.

## UI and mediated devices

`display`, `audio`, `clipboard`, `device`, and `usb` commands are Provider
projections. They address the owning Zone and ResourceRefs and report
capability, enforcement, and readiness state. They do not open host devices,
PipeWire sockets, compositor sockets, or Guest state directly.

Companion-facing security-key spellings are:

```text
d2b audio status --json
d2b usb security-key status
d2b usb security-key sessions
d2b usb security-key cancel
```

The Rust parser's current namespace is `d2b device security-key ...`.
Clients must not open the private file directly or bypass the Provider and
broker lease. A client must not read private d2b state to recover a missing
action.

## Redaction

Public responses may contain bounded Resource names, status, generation,
revision, capability, and operation metadata. They must not contain:

- credentials or secret bytes;
- raw host paths, socket paths, or device paths;
- argv, environment, cwd, or process output outside the bounded output
  projection;
- pidfds, namespace IDs, cgroup paths, or private broker handles;
- remote node registries or Gateway Guest credentials.

## Errors and retries

Typed errors include an owning operation, stable kind, exit code, redacted
message, and safe remediation. Clients must preserve the distinction between
usage, authorization, capability, transport, protocol, and degraded lifecycle
errors. Operation IDs make retries idempotent; a retry must not create a
duplicate child or execution.

Closed DTOs are decoded with the documented version. A client that cannot
decode the selected version or capability set fails visibly.

## Historical material

Realm, environment, VM-first, Gateway-daemon, and bash-fallback forms belong
to historical ADRs and migration notes. They are not current wire contracts
and must not be used as compatibility fallbacks.

The archived wording `normal `d2b vm stop --apply`` means the current
replacement `d2b guest stop <name> --zone <zone> --apply`.
