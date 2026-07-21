# Graphics workstation

This example declares one host-local realm with a Cloud Hypervisor runtime,
the host-mediated device provider, a declared network, and one graphics
workload. It demonstrates the realm-native configuration shape; there are no
`d2b.vms` or `d2b.envs` compatibility declarations.

## Resource model

`workloads.corp-desktop.launcher.capabilities` requests `gpu` and `usbip`.
Evaluation turns those declarations into:

- canonical realm, workload, provider, and role IDs;
- `gpu`, `gpu-render-node`, and `usbip` resource rows;
- allocator lease requests for the shared render node and exclusive security
  key;
- a `host-mediated` device-provider registry entry.

Physical USB bus IDs and render-node names remain provider inputs. They never
become state, runtime, lease, or socket path components. Child realm processes
receive only broker-validated descriptors under live allocator leases.

## Try it

```bash
nix flake check
sudo nixos-rebuild switch --flake .#demo
d2b up corp-desktop.desktop.local-root.d2b --apply
```

The example leaves `autostart = false`: the graphics workload should start
after the host Wayland session is available.

## Configuration highlights

```nix
d2b.acceptDestructiveV2Cutover = true;

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

  workloads.corp-desktop = {
    provider = "runtime";
    launcher.capabilities = [ "gpu" "usbip" ];
  };
};
```

Add `tpm`, `security-key`, or `video` to the workload capability list to
request those mediated resources. `usbip` and `security-key` are mutually
exclusive for one workload. Video requires `gpu` and adds a dedicated `video`
role; render-node access remains shared and descriptor-mediated.

## Runtime paths

Paths are derived only from canonical short IDs:

```text
/var/lib/d2b/r/<realm-id>/w/<workload-id>/
/run/d2b/r/<realm-id>/w/<workload-id>/roles/<role-id>/
```

Use `d2b inspect corp-desktop.desktop.local-root.d2b` to map those IDs back to
the public canonical target. Do not construct paths from the display label or
physical device identity.
