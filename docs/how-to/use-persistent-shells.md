# Use persistent shells

> Diataxis: how-to. Task-oriented operator guide for `d2b shell`.

Persistent shells let you reconnect to a named interactive shell in a local VM
or an explicitly unsafe-local workload. Use them for long-lived interactive work. Use
`d2b exec run Guest/<name> -- <cmd>` for one-off commands.

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
d2b --json shell list Host/tools
```

Shell lifecycle uses qualified Resource references and an authenticated named
stream. There is no static, SSH, host-shell, or retired public-socket fallback.

## Open a shell

```bash
d2b shell open Guest/work
d2b shell open Host/tools
```

Omitting `--name` creates or attaches `ShellSession/primary`. To use another
name:

```bash
d2b shell open Guest/work --name build
```

Names must be 1-63 ASCII bytes, start with a lowercase letter, and then contain
only lowercase letters, digits, and hyphens.

## Reattach

After detaching or closing the local terminal, attach to the same name again:

```bash
d2b shell attach ShellSession/build
```

If another client is already attached to the same session, the attach fails.
Use `--force` only when you intentionally want to detach that existing client:

```bash
d2b shell attach ShellSession/build --force
```

## List sessions

```bash
d2b shell list Guest/work
d2b --json shell list Host/tools
```

The response includes `defaultName` and a `sessions` array.

## Inspect status

```bash
d2b --json shell status ShellSession/build
```

## Detach a stale client

```bash
d2b --json shell detach ShellSession/build
```

Detach is non-destructive. It is safe to retry when the session is already
detached or absent.

## Kill a session

```bash
d2b --json shell kill ShellSession/build
```

Use `list` first if you need to discover the session name.

## Avoid co-locating untrusted same-UID services

Persistent shells use a workload-user shpool socket inside the guest. Code
already running as the same workload UID can reach that AF_UNIX socket. Do not
co-locate untrusted same-UID services with persistent admin shells.

Unsafe-local has the same trust limitation on the host uid. Its shell survives
CLI, d2bd, and helper reconnects while the verified transient user scope stays
alive. Logging out terminates the non-lingering user manager and its shells by
design.
