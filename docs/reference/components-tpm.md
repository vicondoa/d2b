# TPM Provider

**Diataxis category:** reference.

The TPM Provider owns persistent Guest TPM state, the broker-spawned emulator,
its private socket, and the exact device/cgroup policy. A Guest selects the
Provider through its Zone resource graph; there is no per-Guest systemd unit.

TPM state is identity-bound. A missing, replaced, foreign, or symlinked
previously provisioned state directory fails closed. The broker repairs only
the path it owns and records a redacted audit event.

Inspect the owning Guest and host posture with:

```bash
d2b guest status <name> --zone <zone>
d2b host doctor --read-only
```

Never place TPM state paths, control sockets, or credentials in a public
Resource spec or CLI request.
