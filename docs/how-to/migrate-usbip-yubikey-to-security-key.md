# Migrate USBIP security keys to FIDO mediation

Use this migration when the workload needs FIDO ceremonies but does not need a
complete USB device.

## 1. Remove the USBIP capability

Change the realm workload declaration from:

```nix
launcher.capabilities = [ "usbip" ];
```

to:

```nix
launcher.capabilities = [ "security-key" ];
```

Keep the realm's device provider as:

```nix
providers.devices = {
  type = "device";
  implementationId = "host-mediated";
};
```

The two capabilities are intentionally mutually exclusive.

## 2. Rebuild before attaching

Apply the host configuration and restart the workload through the normal
lifecycle. Do not retain scripts or state keyed by a USB bus ID: a bus ID is
physical discovery data, not canonical workload identity.

## 3. Verify the new resource

The evaluated row should now report:

- resource kind `fido`;
- role kind `security-key-frontend`;
- capability `fido-ceremony`;
- exclusive lease `device-security-key-global`;
- FD-only, realm-local broker mediation.

The frontend endpoint must contain only canonical realm and workload IDs:

```text
/run/d2b/r/<realm-id>/w/<workload-id>/roles/<frontend-role-id>/security-key.sock
```

After verification, remove obsolete USBIP-specific operational instructions
from the consumer configuration.
