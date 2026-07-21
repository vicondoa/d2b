# USBIP device resource

USBIP supplies an explicitly selected physical security key to a workload that
requires a complete USB device. It is host-mediated and default-off.

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
    launcher.capabilities = [ "usbip" ];
  };
};
```

## Emitted contract

Evaluation emits a `usbip` row with role kind `usbip`, capability
`usbip-exclusive`, and canonical realm, workload, provider, and role short
IDs. It requests the exclusive allocator lease
`device-security-key-global`.

Host bind, firewall, carrier, and guest import operations remain broker
mediated. The child realm receives validated descriptors and lease authority,
not ambient USB host authority. There is no per-workload USBIP systemd unit.

## Physical identity

USB bus IDs are discovery data and may change after replug. They are never
used in canonical IDs, lock paths, runtime paths, state paths, or socket names.
The resource row contains an opaque canonical selector; the provider resolves
the current physical device only while executing the mediated operation.

`usbip` and `security-key` cannot be requested together by one workload.
