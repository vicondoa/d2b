# Persistent shell component

> Diataxis: reference. Option and policy contract for persistent guest shells.

Persistent shells are disabled by default and are configured per VM:

```nix
d2b.vms.<vm>.guest.shell = {
  enable = true;
  defaultName = "default";
  maxSessions = 8;
  maxAttached = 1;
};
```

## Options

| Option | Type/default | Meaning |
| --- | --- | --- |
| `guest.shell.enable` | boolean, default `false` | Enables the persistent shell policy and guest runtime wiring for this VM. |
| `guest.shell.defaultName` | shell name, default `default` | Session name used when attach/detach omit `--name`. |
| `guest.shell.maxSessions` | integer 1-256, default `8` | Maximum persistent shell sessions tracked for the VM, attached plus detached. |
| `guest.shell.maxAttached` | integer 1-64, default `1` | Maximum concurrently attached persistent shell clients. Must be `<= maxSessions`. |

## Requirements

Enabling `guest.shell` requires:

- `guest.control.enable = true`;
- `guest.exec.enable = true`;
- a non-root workload user (`ssh.user`);
- a runtime/provider that supports guest-control shell operation capability.

`qemu-media` and providers without guest-control reject non-default
`guest.shell.*` settings at eval time.

## Guest wiring

When enabled for a workload user, the guest module:

- passes `--shell-enable`, `--shell-default-name`, `--shell-max-sessions`, and
  `--shell-max-attached` to guestd;
- wires the static `d2b-guest-shell-runner` and `systemctl` paths;
- declares `d2b-shpool-daemon.service` as the workload user with
  `PAMName=d2b-shpool-daemon`;
- sets workload-user linger so `/run/user/<uid>` exists while all shell clients
  are detached.

The shpool daemon remains in the delegated system service cgroup; the PAM service
uses `startSession = false`, `setEnvironment = true`, and `setLoginUid = true`.

## Manifest metadata

Supported providers emit a per-VM manifest block:

```json
{
  "shell": {
    "enabled": true,
    "defaultName": "default",
    "maxSessions": 8,
    "maxAttached": 1
  }
}
```

Providers without guest-control emit `shell = null`. The manifest never exposes
runtime helper sockets, terminal handles, shpool state paths, or live session
names beyond the configured default.

## Trust and redaction

Persistent shells use a same-UID AF_UNIX shpool socket inside the guest. This is
not a boundary against other processes already running as the workload user.

Daemon metrics and audit surfaces do not use raw shell names, terminal handles,
terminal bytes, helper diagnostics, argv, env, or paths as labels or log fields.
The daemon audit stream uses a fixed shell correlation digest where correlation
is needed.
