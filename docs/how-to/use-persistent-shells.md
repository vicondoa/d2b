# Use persistent guest shells

> Diataxis: how-to. Task-oriented operator guide for `nixling shell`.

Persistent shells let you reconnect to a named interactive shell inside a
running guest. Use them for long-lived interactive work. Use
`nixling vm exec <target> -- <cmd>` for one-off commands.

For the persistence model, local IPC boundary, and same-UID trust model,
see [Persistent shell sessions](../explanation/persistent-shells.md).

## Enable persistent shells for a VM

Enable guest control, exec, and shell for a VM with a non-root workload user:

```nix
nixling.vms.work = {
  ssh.user = "alice";

  guest.control.enable = true;
  guest.exec.enable = true;
  guest.shell = {
    enable = true;
    defaultName = "default";
    maxSessions = 8;
    maxAttached = 1;
  };
};
```

Switch the host configuration, then restart the affected VM so guestd sees the
new shell policy.

## Attach to the default shell

```bash
nixling shell work
```

The CLI prints the resolved session name before entering raw terminal mode. To
detach without ending the shell, press `Ctrl-Space` followed by `Ctrl-q`.

Typing `exit` or pressing `Ctrl-D` at an empty prompt ends the persistent shell
session.

## Attach to a named shell

```bash
nixling shell work --name build
```

Names must be 1-64 ASCII bytes, start with `[A-Za-z0-9_]`, and then contain only
`[A-Za-z0-9._-]`.

## Reattach

After detaching or closing the local terminal, attach to the same name again:

```bash
nixling shell work --name build
```

If another client is already attached to the same session, the attach fails.
Use `--force` only when you intentionally want to detach that existing client:

```bash
nixling shell work --name build --force
```

## List sessions

```bash
nixling shell work list
nixling shell work list --json
```

The human output marks the configured default session. JSON output includes
`default_name` and a `sessions` array.

## Detach a stale client

Detach defaults to the target's configured default name when `--name` is omitted:

```bash
nixling shell work detach
nixling shell work detach --name build
```

Detach is non-destructive. It is safe to retry when the session is already
detached or absent.

## Kill a session

Killing is destructive and always requires an explicit name:

```bash
nixling shell work kill --name build
```

Use `list` first if you need to discover the configured default name.

## Gateway-backed targets

Current local-only `nixling shell` generations talk to the local host daemon's
public socket and reject gateway-backed realm targets locally. That rejection is
not permanent: [ADR 0039](../adr/0039-constellation-persistent-shell-routing.md)
defines constellation routing for gateway, remote-node, and provider target
addresses. Until that routing lands, enter the realm gateway first, then run the
shell command from inside that gateway boundary:

```bash
nixling realm enter work
work-gw$ nixling shell <target>
```

## Avoid co-locating untrusted same-UID services

Persistent shells use a workload-user shpool socket inside the guest. Code
already running as the same workload UID can reach that AF_UNIX socket. Do not
co-locate untrusted same-UID services with persistent admin shells.
