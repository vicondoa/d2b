# QEMU media runtime

`qemu-media` is a host-local runtime-provider implementation for manually
operated removable-media workloads.

## Declaration

A workload selects an enabled realm provider:

```nix
d2b.realms.dark = {
  parent = "local-root";
  path = "dark.local-root";
  placement = "host-local";

  providers.media = {
    type = "runtime";
    implementationId = "qemu-media";
    configRef = "dark-live-media";
    capabilities = [ "qmp-media-attach" ];
  };

  workloads.dark-live = {
    provider = "media";
    autostart = false;
  };
};
```

`configRef` is an opaque reference to private provider intent. Public
workload metadata does not contain media paths, USB selectors, credentials,
or QEMU argv.

## Process contract

The owning realm controller emits and supervises a `qemu-media-runner` role.
It starts QEMU paused and uses the allocator-declared TAP and inherited
console descriptors. The QMP listener is:

```text
/run/d2b/r/<realm-id>/w/<workload-id>/roles/<role-id>/qmp.sock
```

The runner is placed directly in:

```text
d2b.slice/r-<realm-id>/workloads/w-<workload-id>/<role-id>
```

The workload and realm cgroup interiors remain process-free. There is no
per-workload systemd service or socket unit.

## Security posture

- The realm broker resolves private media intent and host resources.
- The controller receives opaque resource references and pidfds, not host
  device paths.
- QEMU runs in the role's minijail profile with `/dev/kvm` and
  `/dev/vhost-net` only.
- Display access uses the workload's mediated Wayland role when declared.
- `autostart = true` is not supported for removable-media workloads.
