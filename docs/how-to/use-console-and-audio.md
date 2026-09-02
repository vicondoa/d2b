# Use console and audio Providers

Console and audio are Provider projections for a Zone Guest. The Guest
controller owns lifecycle; each Provider owns its Endpoint, Process, session,
and broker effects.

## Console

```bash
d2b guest console work-app --zone work
```

The Provider must advertise a console capability and the Guest session must
be Ready. A missing capability or unavailable session is a typed failure; the
CLI does not fall back to SSH or a host shell.

## Audio

```bash
d2b audio status --zone work
d2b audio mic on --zone work
d2b audio speaker on --zone work
d2b audio off --zone work
```

Status reports bounded channel and enforcement state. Audio mutation is
daemon-mediated and checks the owning Zone, Guest, Provider generation, and
capability before reaching the broker.

## Security

PipeWire sockets, device paths, credentials, and runner arguments remain
private to the Provider execution context. Desktop companions use the public
daemon socket only and must render unavailable or degraded state without
reading private bundle files.
