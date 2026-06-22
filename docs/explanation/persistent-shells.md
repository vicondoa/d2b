# Persistent shell sessions

> Diataxis: explanation. Conceptual model for `nixling shell`.

`nixling shell` attaches an admin's terminal to a named shell session for a
target workload. The user-facing surface is:

```text
nixling shell <target> [ACTION]
```

where `ACTION` is `attach`, `list`, `detach`, or `kill`. Omitting `ACTION`
attaches to the target's configured default session. Current local-only
generations accept local VM names as targets; ADR 0039 reserves gateway,
remote-node, and provider target routing for a future generation.

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

The host CLI currently connects to the local `nixlingd` public socket. Current
local-shell-only generations reject gateway-backed realm targets locally;
operators manage those guests by entering the realm gateway and running the
command there, for example `nixling realm enter work` followed by
`nixling shell <target>`. That rejection is current-generation behavior, not a
permanent contract: ADR 0039 reserves future constellation routing for gateway,
remote, and provider target addresses.

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
