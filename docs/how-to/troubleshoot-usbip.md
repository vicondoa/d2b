# Troubleshoot USBIP

USBIP is a Zone-owned Device Provider. The broker owns device leases,
firewall rules, attach runners, and cleanup.

## Inspect the current state

```bash
d2b device usb probe --zone work
d2b resource list Device --zone work
d2b guest status work-app --zone work
d2b host doctor --read-only
```

Probe output is redacted. It does not echo serial numbers, device paths, or
private broker selectors.

## Attach and detach

```bash
d2b device usb attach Device/security-key 1-2.3 --zone work --dry-run
d2b device usb attach Device/security-key 1-2.3 --zone work --apply
d2b device usb detach Device/security-key 1-2.3 --zone work --apply
```

The bus ID is transient runtime evidence. The Provider resolves it against
the committed Device Resource and refuses an identity, lease, or ownership
mismatch. Do not place it in a Guest spec or shared documentation.

## Recovery

If a device is stuck, inspect the Device and Guest generation before retrying.
Do not flush all USBIP state, remove foreign firewall rules, or kill a parent
cgroup. A typed degraded or finalization-blocked state is safer than guessing;
repair is owned by the broker and the Device Provider.
