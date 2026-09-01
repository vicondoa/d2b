# Graphics Provider

**Diataxis category:** reference.

Graphics is a Provider projection owned by a Zone. A Guest selects the
Cloud Hypervisor runtime and may reference the graphics Provider contract
through its Zone-local resources. There is no per-Guest graphics service or
caller-owned compositor socket.

The graphics Provider owns GPU mediation, Wayland proxying, process templates,
device allowlists, and cleanup through the assigned controller and broker.
Nix supplies semantic Provider settings and immutable artifacts; it never
places a host socket, device path, argv, or credential in a public Guest spec.

## Lifecycle

The Guest controller waits for the graphics Provider assignment and child
Endpoint/Process status before reporting Ready. On session loss, the daemon
reports a typed degraded state and revokes session-bound authority. Restart
adopts only the matching Zone/Guest/Provider generation.

## Host admission

Set `d2b.site.waylandUser` for a host compositor session and grant the user
the normal `d2b` lifecycle admission. The broker opens the compositor socket
inside the approved runner namespace; the CLI never accepts that path.

## Inspection

```bash
d2b guest status <name> --zone <zone>
d2b display list --zone <zone>
d2b host doctor --read-only
```

Status and audit output is bounded and redacted. Foreign ownership, missing
capabilities, stale identity, and unavailable compositor sessions fail closed.
