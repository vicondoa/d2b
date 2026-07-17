# Security-key device resource

The `security-key` capability provides ceremony-scoped FIDO access without
passing a raw USB device into the workload:

```nix
d2b.realms.work.workloads.desktop = {
  provider = "runtime";
  launcher.capabilities = [ "security-key" ];
};
```

The owning realm must contain one enabled device provider with
`type = "device"` and `implementationId = "host-mediated"`.

## Emitted contract

Evaluation emits a `fido` resource row with:

- capability `fido-ceremony`;
- role kind `security-key-frontend`;
- canonical realm, workload, provider, and role short IDs;
- exclusive allocator lease `device-security-key-global`;
- FD-only, realm-local broker mediation.

The frontend endpoint is:

```text
/run/d2b/r/<realm-id>/w/<workload-id>/roles/<frontend-role-id>/security-key.sock
```

Vendor IDs, product IDs, USB bus IDs, HID node names, and physical paths are
provider inputs. None may appear in the endpoint, resource ID, selector ID, or
state path.

`security-key` and `usbip` are mutually exclusive for one workload. Choose
FIDO mediation unless software inside the workload requires a complete USB
device.
