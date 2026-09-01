# Security-key Provider

**Diataxis category:** reference.

The security-key Provider mediates CTAPHID/WebAuthn access through Zone-owned
Device and Endpoint resources. Host hidraw access, virtual-device projection,
leases, and cancellation remain behind typed broker operations.

```bash
d2b device usb security-key status --zone work
d2b device usb security-key sessions --zone work
d2b device usb security-key cancel <session> --zone work --apply
```

Action keys are opaque, bounded, and single-purpose. Clients forward them
verbatim and must not open hidraw devices or reconstruct a privileged request.
Stale keys, revoked sessions, missing capabilities, and Provider failure are
visible typed errors.
