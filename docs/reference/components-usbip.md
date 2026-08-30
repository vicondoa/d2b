# USBIP Provider

**Diataxis category:** reference.

USBIP is a broker-mediated Device Provider projection owned by a Zone. The
Provider and its assigned controller own the attach process, firewall rule,
device lease, and cleanup. It does not create a per-Guest service or accept a
host device path from the CLI.

Probe and mutate through the current Resource surface:

```bash
d2b device usb probe --zone work
d2b device usb attach Device/<name> <busid> --zone work --apply
d2b device usb detach Device/<name> <busid> --zone work --apply
```

Runtime selectors are transient evidence. Public status and audit output
contains only bounded device identity and lease state. Foreign ownership,
stale Guest identity, missing capability, and uncertain detach fail closed.
