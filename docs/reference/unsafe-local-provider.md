# Unsafe-local provider contract

**Diataxis category:** reference.

`unsafe-local` is an explicit realm workload provider for commands and
persistent shells that run as the authenticated host user. It provides **no
isolation boundary**. The helper connection and user-scope runtime are enabled
for configured eligible users. Public configured launch is feature-negotiated;
the user helper contains the persistent-shell backend, while public daemon
routing and feature advertisement remain unavailable until integration lands.

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
They do not read `/etc/d2b/realm-workloads-launcher-v2.json`, and the helper
receives only daemon-resolved private operations.

The closed unsafe-local posture is:

| Field | Value |
| --- | --- |
| `providerKind` | `unsafe-local` |
| `isolation` | `unsafe-local` |
| `environment` | `systemd-user-manager-ambient` |
| `displayEnvironment` | `wayland-proxy-only` for graphical exec |
| `executionIdentity` | `authenticated-requester-uid` |
| `sessionPersistence` | `user-manager-lifetime` |

`configured-launch-v1`, `unsafe-local-provider-v1`, and
`unsafe-local-shell-v1` are additive protocol-v3 feature flags. Clients may
recognize the shell token with this contract revision, but `d2bd` does not
advertise it until runtime dispatch is enabled. Clients must hide or refuse
unsupported operations; they must never fall back to unsafe-local.

Availability values are directly actionable: `helper-unavailable` and
`helper-stale` require restarting the caller's user helper;
`user-manager-unavailable` requires a PAM-backed graphical login;
`graphical-session-inactive` and `wayland-unavailable` require an active Wayland
session; and `proxy-unavailable` requires repairing the proxy prerequisite.
There is no direct-compositor remediation path.

## Private bundle and helper wire

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
host-local Unix binding. The helper lookup is keyed by the requester's
`SO_PEERCRED` UID, so requester and helper identity are exactly equal. Relay,
remote, stale-helper, cross-UID, direct-compositor, root, SSH, and arbitrary
command fallbacks are not available.

The separate helper protocol is version 2 on the daemon-owned
`/run/d2b/unsafe-local-helper.sock` `SOCK_SEQPACKET` endpoint. Peer credentials,
not a uid field, establish identity. Version 1 is rejected without a
compatibility fallback because the daemon and helper are installed together.
Frames contain no uid, environment, cwd, or public-supplied command. Both peers
request at least 256 KiB for
`SO_SNDBUF` and `SO_RCVBUF` before exchanging frames and verify that Linux
reports effective buffers of at least 512 KiB. A smaller effective buffer makes
the helper unavailable rather than allowing a valid 256 KiB frame to fail with
`EMSGSIZE`. d2b does not write the host-wide `net.core.rmem_max` or
`net.core.wmem_max` sysctls because a fixed value could lower a host's existing
limits. Operators may raise restrictive host limits independently; helper
registration remains fail-closed if the effective per-socket requirement is not
met.

Shell requests additionally carry a closed trusted policy containing
`defaultName` and `maxSessions`. The later daemon routing layer must populate
those fields only from the bundle-hashed unsafe-local workload record; no public
shell request can choose or raise them.

The globally installed `d2b-unsafe-local-helper.service` is a systemd user
service. `ConditionGroup=d2b-unsafe-local` prevents users who are not allowed to
access an enabled unsafe-local realm from registering or entering a restart
loop. The helper connects outward; `d2bd` does not discover a user bus or
impersonate a user. Both peers verify `SO_PEERCRED`, and a valid reconnect
atomically supersedes the prior generation for that UID.

For each operation the helper reads
`org.freedesktop.systemd1.Manager.Environment` from the current user manager.
It rejects malformed or oversized data rather than trimming it, clears the
child's inherited environment, and copies the complete manager environment.
Graphical operations additionally remove `DISPLAY` and require a proxy-owned
`WAYLAND_DISPLAY`; if no proxy endpoint is ready, launch fails without a direct
display fallback.

Every launched process begins behind a blocked supervisor. The helper calls
`StartTransientUnit`, verifies the returned scope's `InvocationID` and exact
control-group identity, and only then releases the supervisor to start the
child. Reconnect snapshots re-query that identity. An ambiguous scope is
preserved and reported degraded rather than killed by PID, name, or a broad
cgroup sweep. These scopes last only for the systemd user-manager lifetime;
d2b does not enable lingering.

Persistent shells use a separate hidden `shell-supervisor` process in a
`persistent-shell` transient user scope. The helper reserves the operation and
name, reads the complete user-manager environment and passwd identity, starts
the supervisor blocked, verifies the scope identity, releases it, waits for
socket, PTY, and login-shell readiness, and only then atomically extends the
existing scope ledger. The supervisor—not the reconnectable helper—owns the PTY
master, login-shell child, output ring, attachment state, and private listener.
The child executes the authenticated user's absolute passwd login shell in the
passwd home with the complete manager environment; no shell string, PATH
lookup, configurable command, or journal output is involved.

The supervisor id is random and opaque. Its listener is derived beneath a
validated same-UID runtime directory with mode `0700`; the socket is mode
`0600`, rejects the wrong owner or file type, and cleanup removes only the
original owned socket inode. Ledger adoption re-verifies both the transient
scope and supervisor status. A missing or ambiguous listener is preserved and
reported degraded rather than swept or killed.

Terminal data uses exactly one connected `AF_UNIX`
`SOCK_STREAM` passed with `SCM_RIGHTS`; listeners, datagram sockets, zero fds,
and multiple fds are rejected. The receiver requires
`getsockopt(SO_TYPE) == SOCK_STREAM`, `getsockopt(SO_ACCEPTCONN) == 0`, and a
successful `getpeername`, then verifies the authenticated helper generation,
request correlation, and terminal protocol version before accepting the fd.
Terminal protocol version 1 uses bounded JSON frames with a four-byte
little-endian body-length prefix for stdin writes, output reads, resize, wait,
stdin close, attachment close, and typed rejections. The connected socket binds
one attachment, so these frames never accept a client-supplied session handle.
Frames are limited to 128 KiB, decoded chunks to 64 KiB, per-stream output rings
to the contract ceiling of 8 MiB, and waits to 1000 ms. The helper currently
reserves 512 KiB per merged PTY output ring and caps all such reservations for
one helper at 32 MiB. `stdout` is the authoritative merged PTY stream; `stderr`
reads return an empty terminal result. Reads use absolute cursors and report
dropped bytes after wrap. Writes and control sequences are strictly monotonic,
and a long read poll does not block writes or resize.
Terminal bytes do not share the helper control queue. Public daemon shell
routing remains unavailable in this revision.

Detach closes only the current attachment. Force attach atomically evicts the
old attachment. Kill closes the owned PTY master, waits briefly, then signals
only the still-verified transient scope with `SIGTERM` and finally `SIGKILL`;
unrelated same-UID processes and scopes are never targeted.

Every socket is created with `SOCK_CLOEXEC`, every other control or PTY fd uses
`O_CLOEXEC`, and rights are received with `MSG_CMSG_CLOEXEC`. Only descriptors
explicitly remapped for a child may survive `exec`.

## Runtime observability

Helper registration, reconnect, supersede, and stale events use bounded event
kinds and result classes. Runtime signals never expose uid, argv, environment,
cwd, paths, PIDs, unit names, shell names, transcripts, or terminal bytes.
Scope, proxy, launcher, and shell signals follow the same rule as those provider
routes become available.

Generated schemas:

- [`realm-workloads-launcher-v2.json`](./schemas/v2/realm-workloads-launcher-v2.json)
- [`unsafe-local-workloads.json`](./schemas/v2/unsafe-local-workloads.json)
- [`unsafe-local-helper-wire.json`](./schemas/v2/unsafe-local-helper-wire.json)

## Security meaning

The helper must execute only as the exact authenticated requesting uid. User
systemd scopes provide lifecycle ownership and restart adoption, not a security
boundary from other processes running as that uid. The Wayland proxy provides
identity rails and clipboard attribution, not same-uid compositor containment.
No d2b path supplies or retries a direct compositor connection.
