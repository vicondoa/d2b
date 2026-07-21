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
    providerRefs = {
      runtime = "runtime";
      device = "devices";
    };
    launcher.capabilities = [ "gpu" ];
  };
};
```

## Emitted resources

The declaration emits one `gpu` row by default, or one `render-node` row when
render-node-only graphics is selected. The row binds the canonical realm,
workload, provider, and role short IDs. The corresponding role kind is `gpu` or
`gpu-render-node`.

Both modes reference the allocator-owned
`lease-device-render-node-global` shared partition. A child realm receives only
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

The authenticated `d2b.wayland.v2` `OpenDisplay` operation supplies exactly two
owned descriptors: the upstream compositor connection and the proxy listener.
Both claims are bound to the request, operation, method, session generation,
package, descriptor purpose, and credit classes. The Wayland proxy adopts the
descriptors on standard input and standard output and receives no compositor or
listener path argument.

## Related capabilities

- Add `video` together with `gpu` for the dedicated decode sidecar.
- Wayland access remains a separate mediated display concern.
- Render-node access is shared, but every attachment remains allocator-leased
  and FD-only.
