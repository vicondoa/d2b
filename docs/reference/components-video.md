# Video device resource

The `video` capability adds the dedicated hardware-decode sidecar. It must be
declared with `gpu`:

```nix
d2b.realms.desktop.workloads.workstation = {
  provider = "runtime";
  launcher.capabilities = [ "gpu" "video" ];
};
```

A `video` declaration without `gpu` fails evaluation.

## Emitted resources

The pair emits `gpu`, `render-node`, and `video` rows with separate canonical
role IDs. All reference the allocator-owned
`device-render-node-global` shared partition and require FD-only attachment.

The vhost-user video socket is:

```text
/run/d2b/r/<realm-id>/w/<workload-id>/roles/<video-role-id>/video.sock
```

The guest's `virtio_media` driver connects through the patched Cloud
Hypervisor media transport to the broker-spawned video runner. The runner is
pidfd-supervised by the owning realm controller; there is no per-workload
systemd unit.

## Device isolation

Render-node and optional vendor-specific nodes are provider configuration.
They are never part of canonical IDs or paths. The broker validates the closed
device set and passes descriptors to the video role; the role does not receive
broad `/dev` access.

This is the V4L2 media decode path. It is distinct from graphics display,
Wayland mediation, and experimental VA-API exposure through the GPU path.
