# Use persistent guest shells

> Diataxis: how-to. Task-oriented operator guide for `d2b shell`.

Persistent shells let you reconnect to a named interactive shell inside a
running guest. Use them for long-lived interactive work. Use
`d2b vm exec <target> -- <cmd>` for one-off commands.

For the persistence model, local IPC boundary, and same-UID trust model,
see [Persistent shell sessions](../explanation/persistent-shells.md).

## Enable persistent shells for a VM

Enable guest control, exec, and shell for a VM with a non-root workload user:

```nix
d2b.vms.work = {
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
d2b shell work
```

The CLI prints the resolved session name before entering raw terminal mode. To
detach without ending the shell, press `Ctrl-Space` followed by `Ctrl-q`.

Typing `exit` or pressing `Ctrl-D` at an empty prompt ends the persistent shell
session.

## Attach to a named shell

```bash
d2b shell work --name build
```

Names must be 1-64 ASCII bytes, start with `[A-Za-z0-9_]`, and then contain only
`[A-Za-z0-9._-]`.

## Reattach

After detaching or closing the local terminal, attach to the same name again:

```bash
d2b shell work --name build
```

If another client is already attached to the same session, the attach fails.
Use `--force` only when you intentionally want to detach that existing client:

```bash
d2b shell work --name build --force
```

## List sessions

```bash
d2b shell work list
d2b shell work list --json
```

The human output marks the configured default session. JSON output includes
`default_name` and a `sessions` array.

## Detach a stale client

Detach defaults to the target's configured default name when `--name` is omitted:

```bash
d2b shell work detach
d2b shell work detach --name build
```

Detach is non-destructive. It is safe to retry when the session is already
detached or absent.

## Kill a session

Killing is destructive and always requires an explicit name:

```bash
d2b shell work kill --name build
```

Use `list` first if you need to discover the configured default name.

## Gateway-backed targets

`d2b shell list`, `detach`, and `kill` route gateway-backed realm targets
through the selected gateway in current generations. Interactive gateway-backed
`attach` still fails closed on the host facade until semantic gateway attach
lands; for that case, enter the realm gateway first, then run the shell command
from inside that gateway boundary:

```bash
d2b realm enter work
work-gw$ d2b shell <target>
```

## Avoid co-locating untrusted same-UID services

Persistent shells use a workload-user shpool socket inside the guest. Code
already running as the same workload UID can reach that AF_UNIX socket. Do not
co-locate untrusted same-UID services with persistent admin shells.
