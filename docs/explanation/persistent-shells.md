# Persistent shell sessions

> Diataxis: explanation. Conceptual model for `nixling shell`.

`nixling shell` attaches an admin's terminal to a named shell session for a
target workload. The user-facing surface is:

```text
nixling shell <target> [ACTION]
```

where `ACTION` is `attach`, `list`, `detach`, or `kill`. Omitting `ACTION`
attaches to the target's configured default session. Local VM names stay on the
local daemon fast path. Gateway-backed management actions route through the
configured realm gateway; interactive gateway attach remains fail-closed until
semantic ADR 0039 attach support lands.

## Persistence boundary

Persistent shell state belongs to the guest-local shell pool, not to the host
CLI process. A session is expected to survive:

- the local CLI disconnecting;
- the terminal window closing;
- guestd restart when guestd can adopt the still-running shell pool.

It is not expected to survive:

- VM reboot or target workload recreation;
- shell-pool daemon restart or loss;
- explicit `nixling shell <target> kill --name <name>`;
- `exit` or `Ctrl-D` inside the shell.

This is intentionally different from `nixling vm exec -it`, whose command is
connection-owned and exits with the command's status.

## Local dispatch and network surface

The host CLI connects to the local `nixlingd` public socket for local targets.
For gateway-backed `list`, `detach`, and `kill`, it enters the realm trust
boundary by running the same `nixling shell <target> ...` command inside the
gateway VM over the typed guest-control exec path. The host still does not load
realm credentials or provider transports. Gateway-backed interactive attach
fails closed on the host facade; operators can enter the realm gateway and run
`nixling shell <target>` there until the semantic ADR 0039 attach stream lands.

Persistent shells do not add TCP or UDP listeners, network ports, or
network-bound debug/metrics surfaces. The host-to-guest path reuses the existing
daemon public socket and authenticated guest-control transport.

## Same-UID AF_UNIX boundary

Inside the guest, shpool exposes an AF_UNIX socket under the workload user's
runtime directory. Helpers that connect to that socket run as the workload UID.
The socket is a same-UID IPC boundary, not a cryptographic separation boundary:
code already running as that workload user can potentially interact with the
same shell pool.

For that reason, persistent shells are appropriate for a trusted workload-user
environment. They are not a way to hide admin shell state from other code
already executing as the same guest user.

## Non-goals

Persistent shells do not provide tmux-style multiplexing, panes, windows, SSH
fallbacks, or shell templates/start-command customization. One CLI invocation
attaches to one named session.
