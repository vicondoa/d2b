# Use persistent shells

> Diataxis: how-to. Task-oriented operator guide for `d2b shell`.

Persistent shells let you reconnect to a named interactive shell in a local VM
or an explicitly unsafe-local workload. Use them for long-lived interactive work. Use
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

## Enable an unsafe-local shell

Unsafe-local runs the login shell directly as the authenticated host user and
provides no VM or same-UID containment:

```nix
d2b.realms.host = {
  allowedUsers = [ "alice" ];
  policy.allowUnsafeLocal = true;
  workloads.tools = {
    kind = "unsafe-local";
    shell = {
      enable = true;
      defaultName = "primary";
      maxSessions = 4;
    };
    launcher.items.terminal = {
      type = "shell";
      name = "Terminal";
    };
  };
};
```

Rebuild the host, log in through a PAM-backed session, and verify the user
helper is active:

```bash
systemctl --user status d2b-unsafe-local-helper.service
d2b shell tools.host.d2b list
```

The daemon must negotiate `unsafe-local-shell-v1`. Version skew fails with an
update recommendation; there is no static, SSH, or host-shell fallback.

## Attach to the default shell

```bash
d2b shell work
d2b shell tools.host.d2b
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

## Provider-managed targets

A provider-managed target supports persistent shells only when its runtime
advertises the positive `persistent-shell` capability through its authenticated
provider agent. The ACA runtime provider does not gain shell capability by
implication. There is no gateway-guest fallback, provider-native shell
fallback, or SSH fallback.

## Avoid co-locating untrusted same-UID services

Persistent shells use a workload-user shpool socket inside the guest. Code
already running as the same workload UID can reach that AF_UNIX socket. Do not
co-locate untrusted same-UID services with persistent admin shells.

Unsafe-local has the same trust limitation on the host uid. Its shell survives
CLI, d2bd, and helper reconnects while the verified transient user scope stays
alive. Logging out terminates the non-lingering user manager and its shells by
design.
