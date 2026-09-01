# Resolve Zone network conflicts

Network conflicts are refused at the Zone Provider boundary rather than
silently overwriting host routes or another Zone's state.

## Inspect

```bash
d2b network list --zone work
d2b host check --json
ip route show
```

Compare the declared Network Resource and the host's existing routes. Keep
Zone CIDRs disjoint from the host LAN and from other Zone networks.

## Remediate

Prefer changing the Zone Network Resource to a disjoint documented CIDR:

```nix
d2b.zones.work.resources.work-lan = {
  type = "Network";
  spec = {
    providerRef = "Provider/network-local";
    lanCidr = "10.142.142.0/24";
  };
};
```

Do not delete a foreign route, flush a host table, or bypass the broker to
force a partial overlap. Reconcile only d2b-owned state:

```bash
d2b host reconcile --dry-run
d2b host reconcile --apply
```

The Network Provider and broker preserve foreign ownership markers and return
a typed conflict when a route, bridge, TAP, or firewall rule cannot be proven
to belong to the requested Zone.
