# Resolve realm route conflicts

A declared realm LAN or uplink must not overlap another declared realm or a
host LAN CIDR. Nix evaluation rejects known overlap. Runtime reconciliation
also fails closed when an allocator-observed kernel route has foreign
ownership.

## Diagnose

Inspect the configured realm CIDRs and host routes:

```bash
d2b host check --json
ip -4 route show table main
```

Look for a route containing either
`d2b.realms.<realm>.network.lanSubnet` or `uplinkSubnet`. VPN and broad
connected routes are common conflicts.

## Preferred remediation

Choose disjoint realm ranges:

```nix
d2b.realms.work.network = {
  mode = "declared";
  lanSubnet = "10.142.0.0/24";
  uplinkSubnet = "192.0.2.0/30";
};
```

Also list every physical host LAN under `d2b.hostLanCidrs`; the evaluator uses
that inventory both for overlap rejection and for the net workload's
destination blocklist.

Do not delete a foreign route automatically or broaden a realm's allocator
lease. If a route must remain, move the realm CIDR. If the detected route is
stale, remove it through the system that owns it, then rerun the read-only host
check before applying reconciliation.
