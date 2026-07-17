# Use a USB security key

Choose one of two mediated modes:

- `security-key` for ceremony-scoped FIDO access;
- `usbip` when software requires the complete USB device.

Do not request both modes for one workload.

## Configure FIDO mediation

Add a host-mediated device provider to the workload's host-local realm and
request the capability:

```nix
d2b.realms.work = {
  path = "work.local-root";
  providers.runtime = {
    type = "runtime";
    implementationId = "cloud-hypervisor";
  };
  providers.devices = {
    type = "device";
    implementationId = "host-mediated";
  };
  workloads.desktop = {
    provider = "runtime";
    launcher.capabilities = [ "security-key" ];
  };
};
```

Rebuild the host, start the workload, and initiate a FIDO operation in the
guest. The provider resolves the connected key and exposes only the mediated
frontend descriptor.

## Configure full-device USBIP

Replace `security-key` with `usbip`, rebuild, and use the USB attach workflow
for the canonical workload target. USB discovery may show a physical bus ID,
but that value is operation input only. Do not persist it as a workload name,
resource ID, lock path, or runtime path.

## Verify isolation

Inspect the evaluated device rows and confirm:

- each row uses canonical short IDs;
- mediation is `host-mediated`, `realm-local`, and `fd-only`;
- the lease is `device-security-key-global` and exclusive;
- no vendor, product, bus, HID, or device-node identity appears in a path.
