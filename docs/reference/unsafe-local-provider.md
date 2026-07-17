# Unsafe-local provider contract

**Diataxis category:** reference.

`unsafe-local` is an explicit realm workload provider for commands and
persistent shells that run as the authenticated host user. It provides **no
isolation boundary**. The per-user runtime and transient user scopes are enabled
for configured eligible users. Dispatch runs only through the frozen
`d2b.runtime.systemd-user.v2` ComponentSession boundary for the exact
requester uid.

## Nix options

```nix
d2b.realms.host = {
  allowedUsers = [ "alice" ];
  policy.allowUnsafeLocal = true;

  workloads.tools = {
    kind = "unsafe-local";

    shell = {
      enable = true;
      defaultName = "host";
      maxSessions = 8;
    };

    launcher = {
      enable = true;
      defaultItem = "browser";
      items = {
        browser = {
          type = "exec";
          name = "Browser";
          icon.name = "firefox";
          argv = [ "firefox" ];
          graphical = true;
        };
        terminal = {
          type = "shell";
          name = "Terminal";
          icon.name = "terminal";
        };
      };
    };
  };
};
```

`policy.allowUnsafeLocal` defaults to `false`. An unsafe-local workload must
have an eligible `allowedUsers` entry and at least one exec item or enabled
persistent shell. It cannot declare VM state/runtime paths, `legacyVmName`,
local-VM settings, QEMU settings, or legacy launcher command strings.

Launcher item ids match `^[a-z][a-z0-9-]*$`. Exec `argv` is a non-empty vector,
not a shell string, and is bounded to 128 entries, 16 KiB total, and 4 KiB per
argument. A shell item requires `shell.enable = true`.
Firefox is an ordinary configured `exec` item; a terminal launcher is an
ordinary `shell` item. Neither has a provider-specific public request shape.

## Public metadata

`realm-workloads-launcher-v2.json` and the feature-negotiated workload DTOs
expose:

- canonical workload identity;
- provider kind and typed execution posture;
- item id, type, name, icon, graphical flag, and capability requirements;
- the realm accent color;
- provider availability and graphical posture on runtime status.

They never expose argv, uid, environment, cwd, scope identity, or compositor
paths. The compatibility `realm-workloads-launcher.json` remains schema v1 and
omits unsafe-local rows.

`contractPublic` describes the artifact's safe data shape, not direct filesystem
access. The bundle copy remains `0640 root:d2bd`; unprivileged CLI and desktop
clients receive launcher metadata through the authorized public daemon API.
They do not read `/etc/d2b/realm-workloads-launcher-v2.json`, and the runtime
receives only controller-resolved private operations.

The closed unsafe-local posture is:

| Field | Value |
| --- | --- |
| `providerKind` | `unsafe-local` |
| `isolation` | `unsafe-local` |
| `environment` | `systemd-user-manager-ambient` |
| `displayEnvironment` | `wayland-proxy-only` for graphical exec |
| `executionIdentity` | `authenticated-requester-uid` |
| `sessionPersistence` | `user-manager-lifetime` |

The closed service methods are `EnsureScope`, `StartProcess`, `InspectProcess`,
`AdoptProcess`, `StopProcess`, `OpenTerminal`, and `Cancel`. Mutating requests
require a nonempty operation id, idempotency key, 32-byte private request
digest, bounded lifetime, and the current ComponentSession generation. The
request scope must exactly equal the authenticated realm and workload. There is
no uid, argv, environment, cwd, unit name, compositor path, or shell command on
the wire.

Availability values are directly actionable: `runtime-unavailable` requires
restarting the caller's systemd user runtime;
`user-manager-unavailable` requires a PAM-backed graphical login;
`graphical-session-inactive` and `wayland-unavailable` require an active Wayland
session; and `proxy-unavailable` requires repairing the proxy prerequisite.
There is no direct-compositor remediation path.

## Private bundle and runtime service

Bundle version 10 added
`unsafeLocalWorkloadsPath = /etc/d2b/unsafe-local-workloads.json`; bundle
version 11 extends that artifact with local-VM configured launcher items. The
private artifact contains normalized configured argv and unsafe-local shell
policy, and is covered by the normal bundle artifact
hash. `d2bd` cross-checks its item id/type/graphical shape against public
metadata before dispatch.
For a local-VM workload, dispatch uses `legacyVmName` when present and otherwise
uses the first-class workload id as the backing VM name.

Configured exec launch accepts only local launcher/admin peers on the direct
host-local controller binding. The runtime session authenticates the Unix peer
uid and admits it only when that uid equals the runtime process's non-root uid.
Relay, remote, cross-uid, direct-compositor, root, SSH, arbitrary-command, old
helper-protocol, and host-shell fallbacks are not available.

The user manager owns `d2b-runtime-systemd-user.socket`; unit provenance is part
of its generated endpoint row. ComponentSession owns the local handshake,
generation, record protection, deadlines, cancellation, and packet-atomic
attachments. The runtime does not self-bind a substitute endpoint and does not
accept helper protocol 3 hello, heartbeat, snapshot, JSON, or generation frames.

Shell policy remains private bundle data and is handled by the separate shell
service. No public request can choose or raise it.

Supplementary groups are fixed when the login session starts. After enabling
the first unsafe-local realm for a user, or adding that user to `allowedUsers`,
the user must log out and back in before the runtime agent can connect. The
existing session remains fail-closed because neither its user manager nor
runtime agent has the new `d2b-unsafe-local` group.

For each operation the runtime reads
`org.freedesktop.systemd1.Manager.Environment` from the current user manager.
It rejects malformed or oversized data rather than trimming it, clears the
child's inherited environment, and copies the complete manager environment.
Graphical operations additionally remove `DISPLAY` and require a proxy-owned
`WAYLAND_DISPLAY`; if no proxy endpoint is ready, launch fails without a direct
display fallback.

Graphical launch obtains an authenticated display handle from
`d2b.wayland.v2`. The runtime never constructs a compositor path or directly
spawns a compositor client as a fallback. Failure to open the display prevents
process start; failure after opening closes the new handle.

Every launched process begins behind a blocked supervisor. The runtime calls
`StartTransientUnit`, verifies the returned scope's `InvocationID` and exact
control-group identity, and only then releases the supervisor to start the
child. Adoption re-queries that identity. An ambiguous scope is
preserved and reported degraded rather than killed by PID, name, or a broad
cgroup sweep. These scopes last only for the systemd user-manager lifetime;
d2b does not enable lingering.

Persistent shells use a separate hidden `shell-supervisor` process in a
`persistent-shell` transient user scope. The shell service reserves the
operation and name, reads the complete user-manager environment and passwd
identity, starts the supervisor blocked, verifies the scope identity, releases
it, waits for socket, PTY, and login-shell readiness, and only then atomically
extends the existing scope ledger. The supervisor—not the reconnectable shell
service—owns the PTY master, login-shell child, output ring, attachment state,
and private listener.
The child executes the authenticated user's absolute passwd login shell in the
passwd home with the complete manager environment; no shell string, PATH
lookup, configurable command, or journal output is involved.

The supervisor id is random and opaque. Its listener is derived beneath a
validated same-UID runtime directory with mode `0700`; the socket is mode
`0600`, rejects the wrong owner or file type, and cleanup removes only the
original owned socket inode. Ledger adoption re-verifies both the transient
scope and supervisor status. A missing or ambiguous listener is preserved and
reported degraded rather than swept or killed.

`OpenTerminal` requires exactly attachment index `0`, bound by ComponentSession
to the authenticated request id, session generation, and exact owner uid. The
attachment must be one connected `AF_UNIX` `SOCK_STREAM` with `CLOEXEC`;
listeners, datagram sockets, zero descriptors, extra descriptors, and
cross-request reuse fail before dispatch. Terminal bytes use the negotiated
named stream and do not share ttrpc control queues. There is no terminal
protocol-v1 or helper-framing fallback.

`d2bd` resolves canonical targets and unambiguous workload-id aliases before
dispatch. Transition local-VM workloads keep `legacyVmName`; first-class local
VMs use their workload id. Unsafe-local is never coerced to a VM name, and
remote, relay, non-direct-local, ambiguous, and unsupported-provider targets
fail closed. Attach returns an opaque daemon-generated public session handle.
Every later terminal operation must present that exact handle. Disconnect and
`closeAttach` detach only; they never kill the persistent shell.

List, detach, and kill are shell-service operations with controller-generated
operation ids. Idempotency and name reservations prevent duplicate named
supervisors. A controller timeout is ambiguous and is not replayed
automatically; operators should list before retrying a destructive action.
Runtime unavailability, stale generation, user-manager failure, invalid
terminal attachment, output gap, stale offset, quota, name conflict, terminal
close, and timeout all return typed errors without provider fallback.

Detach closes only the current attachment. Force attach atomically evicts the
old attachment. Kill closes the owned PTY master, waits briefly, then signals
only the still-verified transient scope with `SIGTERM` and finally `SIGKILL`;
unrelated same-UID processes and scopes are never targeted.

Every socket is created with `SOCK_CLOEXEC`, every other control or PTY fd uses
`O_CLOEXEC`, and rights are received with `MSG_CMSG_CLOEXEC`. Only descriptors
explicitly remapped for a child may survive `exec`.

## Runtime observability

Runtime session and method outcomes use bounded event kinds and result classes.
The runtime retains at most 64 closed method/outcome diagnostic events; they
contain no identity or payload fields. Provider-neutral `ShellLifecycle` is the
sole runtime shell audit event for both providers. It covers create, attach,
list, detach, kill, close, and failure boundaries with only the configured
canonical target, peer uid, closed provider/action/result values, optional
force-takeover intent, and optional fixed operation/session correlation
digests. The
`d2b_daemon_shell_lifecycle_total` metric uses only closed
provider/component/operation/outcome/error labels. Neither surface includes
argv, environment, cwd, paths, PIDs, unit names, runtime diagnostics, shell
names, supervisor ids, transcripts, terminal bytes, or public session handles.

Generated schemas:

- [`realm-workloads-launcher-v2.json`](./schemas/v2/realm-workloads-launcher-v2.json)
- [`unsafe-local-workloads.json`](./schemas/v2/unsafe-local-workloads.json)
- [`component-session-v2-schema.json`](component-session-v2-schema.json)

## Security meaning

The runtime must execute only as the exact authenticated requesting uid. User
systemd scopes provide lifecycle ownership and restart adoption, not a security
boundary from other processes running as that uid. The Wayland proxy provides
identity rails and clipboard attribution, not same-uid compositor containment.
No d2b path supplies or retries a direct compositor connection.
