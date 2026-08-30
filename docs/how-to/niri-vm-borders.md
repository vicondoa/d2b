# Niri presentation for Zone Guests

Graphics Providers may expose compositor presentation metadata for Zone
Guests. Colors and window rules are presentation only; they never authorize a
Guest or reveal private runtime identity.

## Enable the host compositor

Declare the host Wayland user:

```nix
d2b.site.waylandUser = "alice";
```

The selected graphics Provider opens the approved compositor endpoint inside
its brokered runner. Do not put a compositor socket path or private window
identifier in a Guest Resource.

## Verify

```bash
d2b guest status <name> --zone <zone>
d2b display list --zone <zone>
d2b host doctor --read-only
```

Niri integrations should consume the Provider's public, bounded display
projection and render a neutral state when it is unavailable. Do not source
old VM-border option paths or read private d2b state to reconstruct a label.
