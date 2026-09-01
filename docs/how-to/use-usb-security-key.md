# Use the security-key Provider

The security-key Provider exposes a host-attached FIDO2 device to a Zone
Guest through a mediated virtual device. The physical key remains host-owned;
the Guest receives no USB ownership transfer.

## Declare the Provider resources

Declare the security-key Provider and the Guest Device/Endpoint resources in
the owning Zone. Keep selectors and credentials in the Provider's private
configuration; public Resource specs contain only typed references.

## Check status

```bash
d2b device usb security-key status --zone work
d2b device usb security-key sessions --zone work
d2b guest status work-app --zone work
```

Only one ceremony may hold a physical-device lease at a time. Status exposes
bounded lease and session state, not hidraw paths, serials, tokens, or broker
handles.

## Cancel a session

```bash
d2b device usb security-key cancel --current --zone work --apply
d2b device usb security-key cancel <session-id> --zone work --apply
```

Action/session identifiers are opaque and must be forwarded verbatim. The
client must not open hidraw devices, alter leases, or retry through USBIP.
Revoked, stale, unavailable, or unauthorized sessions fail closed.

See [the security-key Provider reference](../reference/components-usb-security-key.md)
and [the USBIP reference](../reference/components-usbip.md).
