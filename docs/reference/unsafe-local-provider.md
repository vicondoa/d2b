# Unsafe-local provider contract

**Diataxis category:** reference.

`unsafe-local` is an explicit realm workload provider for commands and
persistent shells that run as the authenticated host user. It provides **no
isolation boundary**. The contract is staged behind protocol features until the
daemon/helper runtime is enabled.

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

The closed unsafe-local posture is:

| Field | Value |
| --- | --- |
| `providerKind` | `unsafe-local` |
| `isolation` | `unsafe-local` |
| `environment` | `systemd-user-manager-ambient` |
| `displayEnvironment` | `wayland-proxy-only` for graphical exec |
| `executionIdentity` | `authenticated-requester-uid` |
| `sessionPersistence` | `user-manager-lifetime` |

`configured-launch-v1` and `unsafe-local-provider-v1` are additive protocol-v3
feature flags. Clients must hide or refuse unsupported operations; they must
never fall back to unsafe-local.

## Private bundle and helper wire

Bundle version 10 adds
`unsafeLocalWorkloadsPath = /etc/d2b/unsafe-local-workloads.json`. The private
artifact contains normalized configured argv and shell policy and is covered by
the normal bundle artifact hash.

The separate helper protocol is version 1 on the daemon-owned
`/run/d2b/unsafe-local-helper.sock` `SOCK_SEQPACKET` endpoint. Peer credentials,
not a uid field, establish identity. Frames contain no uid, environment, cwd,
or public-supplied command. Terminal data uses exactly one connected `AF_UNIX`
`SOCK_STREAM` passed with `SCM_RIGHTS`; listeners, datagram sockets, zero fds,
and multiple fds are rejected. The receiver also verifies the authenticated
helper generation, request correlation, and terminal protocol version before
accepting the fd. Terminal bytes do not share the helper control queue.

Every socket is created with `SOCK_CLOEXEC`, every other control or PTY fd uses
`O_CLOEXEC`, and rights are received with `MSG_CMSG_CLOEXEC`. Only descriptors
explicitly remapped for a child may survive `exec`.

## Runtime observability staging

This contract-only stage freezes DTO, schema, redaction, and cardinality rules;
it does not emit runtime events or metrics. The helper registration, scope,
proxy, launcher, and shell lifecycle signals are implemented with their owning
runtime paths. Those implementations must use bounded event kinds and result
classes and must never expose uid, argv, environment, cwd, paths, PIDs, unit
names, shell names, transcripts, or terminal bytes.

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
