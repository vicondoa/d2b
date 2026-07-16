# Persistent shell sessions

> Diataxis: explanation. Conceptual model for `d2b shell`.

`d2b shell` attaches an admin's terminal to a named shell session for a
target workload. The user-facing surface is:

```text
d2b shell <target> [ACTION]
```

where `ACTION` is `attach`, `list`, `detach`, or `kill`. Omitting `ACTION`
attaches to the target's configured default session. Local VM names stay on the
local daemon fast path. Provider-managed targets require an authenticated
provider agent that positively advertises persistent-shell capability.

## Persistence boundary

Persistent shell state belongs to the target runtime, not to the host CLI
process. For a VM that runtime is the guest-local shell pool. For an
unsafe-local workload it is a separate user-scope supervisor that owns the PTY
and reconnect listener rather than the short-lived user helper. A session is
expected to survive:

- the local CLI disconnecting;
- the terminal window closing;
- guestd restart when guestd can adopt the still-running shell pool;
- unsafe-local helper or d2bd reconnect while the verified user scope and
  supervisor remain alive.

It is not expected to survive:

- VM reboot or target workload recreation;
- shell-pool daemon restart or loss;
- logout/termination of the non-lingering user manager for unsafe-local;
- explicit `d2b shell <target> kill --name <name>`;
- `exit` or `Ctrl-D` inside the shell.

This is intentionally different from `d2b vm exec -it`, whose command is
connection-owned and exits with the command's status.

## Local dispatch and network surface

The host CLI connects to the local `d2bd` public socket for local targets.
Provider-managed shell operations stay semantic provider operations and
terminal streams. They are never translated into provider-native exec, raw
guest-control, or a gateway-guest command. Missing provider-agent capability
fails closed.

Persistent shells do not add TCP or UDP listeners, network ports, or
network-bound debug/metrics surfaces. The host-to-guest path reuses the existing
daemon public socket and authenticated guest-control transport.

Unsafe-local uses only same-UID Unix sockets. Its per-shell listener lives
beneath the validated user runtime directory and is not a root service, broker
operation, or per-VM unit. `d2bd` resolves the target and bundle-owned shell
policy, asks the exact requester-UID helper to create or reconnect, validates the
single connected terminal fd, and multiplexes it behind a fresh opaque public
attachment handle. Closing that public connection detaches the helper-owned
terminal stream; it does not kill the user-scope shell.

Daemon and helper restarts are reconnect events. The daemon intentionally keeps
no persisted fd authority, while the helper snapshot revalidates the
user-scope `InvocationID`, cgroup, and supervisor status before adoption.
Ambiguous metadata is reported degraded and never triggers a broad kill.

## Same-UID AF_UNIX boundary

Inside a guest, shpool exposes an AF_UNIX socket under the workload user's
runtime directory. Unsafe-local supervisors use the authenticated host user's
runtime directory for the equivalent reconnect boundary. Helpers that connect
to either socket run as the workload UID.
The socket is a same-UID IPC boundary, not a cryptographic separation boundary:
code already running as that workload user can potentially interact with the
same shell pool.

For unsafe-local this is also the containment boundary: there is **no
containment from other processes running as the same host uid**. The transient
scope gives exact lifecycle ownership, not isolation. Persistence ends with the
user-manager lifetime because d2b does not enable linger.

For that reason, persistent shells are appropriate for a trusted workload-user
environment. They are not a way to hide admin shell state from other code
already executing as the same guest user.

## Non-goals

Persistent shells do not provide tmux-style multiplexing, panes, windows, SSH
fallbacks, or shell templates/start-command customization. One CLI invocation
attaches to one named session.
