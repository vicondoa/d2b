# Audio Provider

**Diataxis category:** reference.

Audio is a Zone-owned Provider projection. The Provider owns the audio
endpoint, guest/host policy, PipeWire mediation, bounded channel state, and
broker cleanup. Nix declares semantic settings and artifacts; it does not
expose a host PipeWire socket or audio process argument.

The Guest controller waits for the audio Provider and its child resources.
Capability or session loss is reported as typed degraded status rather than
silently granting host audio access.

```bash
d2b audio status --zone work
d2b audio mic --zone work
d2b audio speaker --zone work
d2b audio off --zone work
d2b guest status <name> --zone work
```

Public output carries bounded channel and enforcement state only. Credentials,
socket paths, device paths, and private runner identity remain broker-local.
