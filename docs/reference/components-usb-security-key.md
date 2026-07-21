# Security-key device resource

The `security-key` capability provides ceremony-scoped FIDO access without
passing a raw USB device into the workload:

```nix
d2b.realms.work.workloads.desktop = {
  providerRefs = {
    runtime = "runtime";
    device = "devices";
  };
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
- exclusive allocator lease `lease-device-security-key-global`;
- FD-only, realm-local broker mediation.

The frontend endpoint is:

```text
/run/d2b/r/<realm-id>/w/<workload-id>/roles/<frontend-role-id>/security-key.sock
```

Vendor IDs, product IDs, USB bus IDs, HID node names, and physical paths are
provider inputs. None may appear in the endpoint, resource ID, selector ID, or
state path.

## Authenticated frontend boundary

Fixed 64-byte CTAPHID reports use the authenticated `security-key`
ComponentSession stream. The guest frontend accepts no descriptor attachments:
the broker-opened host device remains with the host controller and never enters
the guest. The guest module does not yet receive the channel binding and
reconnect generation required to establish that session, so the frontend fails
closed until the guest module gains the matching controller and session
material.

The typed `SecurityKeyOpenDevice` and `SecurityKeyApplyUdevRules` requests
remain explicitly unimplemented. They are not alternate routes around the live
`OpenHidrawSecurityKey` broker operation. Read-only discovery is allowed;
credential creation and assertion require controller approval. Reset,
credential deletion or management, biometric enrollment, authenticator
configuration, vendor commands, legacy CTAPHID message commands, and unknown
commands are denied locally.

`security-key` and `usbip` are mutually exclusive for one workload. Choose
FIDO mediation unless software inside the workload requires a complete USB
device.
