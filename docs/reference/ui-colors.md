# Zone UI identity

**Diataxis category:** reference.

UI colors are presentation metadata derived from Zone and Resource identity.
They never authorize an operation and must not be used to recover private
state.

## Current source

Desktop consumers should use the public daemon/CLI projections:

```bash
d2b zone status Zone/work --json
d2b guest status work-app --zone work --json
d2b display list --zone work --json
```

The current projection may include bounded Zone, Guest, Provider, lifecycle,
and capability labels. It does not include host paths, credentials, argv,
private runtime locators, or broker handles.

## Consumer behavior

Consumers may map stable ResourceType/name and lifecycle values to their own
palette. A missing, malformed, or unavailable presentation artifact must
produce a visible neutral state while leaving unrelated controls usable.
Never read `/var/lib/d2b`, private bundles, compositor sockets, or broker state
to fill a missing color.

## Wayland

Wayland and graphics Providers own compositor mediation. The host declares
`d2b.site.waylandUser`; a Provider opens the approved compositor endpoint
inside its brokered runner. The CLI and public Resource spec never carry that
endpoint path.

See [display and virtual I/O capabilities](./display-io-capabilities.md) and
[the graphics Provider reference](./components-graphics.md).
