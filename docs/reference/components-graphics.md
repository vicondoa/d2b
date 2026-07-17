# Graphics device resources

Graphics is a host-mediated device capability for a realm workload. Declare
`gpu` in the workload capability list and provide one enabled
`host-mediated` device provider in the same host-local realm:

```nix
d2b.realms.desktop = {
  path = "desktop.local-root";
  placement = "host-local";
  providers.runtime = {
    type = "runtime";
    implementationId = "cloud-hypervisor";
  };
  providers.devices = {
    type = "device";
    implementationId = "host-mediated";
  };
  workloads.workstation = {
    provider = "runtime";
    launcher.capabilities = [ "gpu" ];
  };
};
```

## Emitted resources

The declaration emits one `gpu` row and one `render-node` row. Each row binds
the canonical realm, workload, provider, and role short IDs. Role kinds are
`gpu` and `gpu-render-node`.

Both resources reference the allocator-owned
`device-render-node-global` shared partition. A child realm receives only
validated descriptors; it never receives ambient `/dev` access or allocator
authority. The provider fragment advertises the closed plan, attach, inspect,
adopt, and detach capability set.

The physical render node is provider configuration, not identity. It is never
included in a resource ID, selector ID, state path, runtime path, audit root, or
socket name.

## Runtime paths

Role runtime paths use only canonical short IDs:

```text
/run/d2b/r/<realm-id>/w/<workload-id>/roles/<gpu-role-id>/
/run/d2b/r/<realm-id>/w/<workload-id>/roles/<render-role-id>/
```

Graphics remains Cloud Hypervisor based. The broker-spawned GPU runner and
runtime process are pidfd-supervised by the owning realm controller; there is
no per-workload systemd unit.

## Related capabilities

- Add `video` together with `gpu` for the dedicated decode sidecar.
- Wayland access remains a separate mediated display concern.
- Render-node access is shared, but every attachment remains allocator-leased
  and FD-only.
