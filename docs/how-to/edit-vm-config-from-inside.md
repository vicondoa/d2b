# Edit a Guest evaluator

Guest operating-system configuration is consumer-owned. The host declares a
semantic `Guest` Resource and supplies its evaluator through
`d2b.guestSystems.<zone>.<guest>`.

```nix
d2b.guestSystems.work.work-app = {
  config = {
    environment.systemPackages = [ ];
    services.openssh.enable = true;
    system.build.toplevel = inputs.workGuestSystem;
  };
};
```

Framework settings, Provider selection, host devices, sockets, credentials,
and runtime identity remain outside the Guest evaluator. A guest module must
not define `d2b.*` host options or place raw host paths in a Resource spec.

## Apply an evaluator change

```bash
d2b activation switch Guest/work-app --zone work --dry-run
d2b activation switch Guest/work-app --zone work --apply
d2b guest status work-app --zone work
```

The activation Provider publishes the selected immutable system artifact and
uses the authenticated Guest session for live activation. Stopped Guests use
offline staging for their next start. The Guest controller and specialized
Providers remain the lifecycle owners.

For interactive iteration use [persistent shells](./use-persistent-shells.md)
or [Guest exec](../reference/cli-contract.md#guest-execution); do not invent a
second host-to-Guest config channel.
