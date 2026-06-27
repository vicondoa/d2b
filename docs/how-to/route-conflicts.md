# Resolving ApplyRoute conflicts with kernel auto-routes

This runbook documents the v1.x site-specific finding tracked as
§1 #11 in the v1.2 plan: when a consumer flake's d2b bundle
declares a CIDR that intersects with a wider CIDR already
auto-routed by the kernel (typically from a non-d2b bridge or
interface configured via `networking.interfaces.*` /
`systemd.network`), the broker's `ApplyRoute` op will conflict.

This is **site-specific non-deferral** behavior (per plan §8) —
NOT a framework bug. The framework's code path is correct;
operators must reconcile their on-host network configuration with
the d2b env CIDRs.

## Example failure pattern

Concrete observation from one consumer site:

```
ApplyRoute conflict for 10.42.42.0/23 — bundle's route plan
conflicts with kernel auto-route from eno1's bridge config.
```

Root cause:

- `eno1` (the host's physical NIC) is bridged via a non-d2b
  `br-corp` interface and configured with `10.42.0.0/16`.
- The d2b consumer flake declares `d2b.envs.work.cidr =
  "10.42.42.0/23"` — entirely contained within `10.42.0.0/16`.
- The kernel auto-installs a `10.42.0.0/16 dev br-corp` route
  when `br-corp` comes up.
- Broker's `ApplyRoute` tries to install `10.42.42.0/23 dev br-work-lan`
  but the kernel rejects the partial overlap.

## Diagnostic

```bash
ip route show | grep -E '10\.42\.'
# look for routes that overlap the env CIDR you're declaring
```

## Remediation (in order of preference)

### Option A — re-pick the d2b env CIDR

Easiest. Pick a CIDR that doesn't intersect any host-side route:

```nix
d2b.envs.work.cidr = "10.142.142.0/23";  # disjoint from eno1's 10.42.0.0/16
```

### Option B — delete the auto-route before d2b reconciles

If you must keep the chosen CIDR, manually delete the kernel auto-
route after each network state change. This is fragile but works
for non-rebooting use:

```bash
sudo ip route del 10.42.0.0/16 dev br-corp
sudo d2b host reconcile --network --apply
```

### Option C — confine the host bridge's CIDR

If the host bridge `br-corp` doesn't need the full `10.42.0.0/16`,
narrow its declared range:

```nix
networking.interfaces.br-corp.ipv4.addresses = [
  { address = "10.42.1.1"; prefixLength = 24; }  # /24 instead of /16
];
```

This frees `10.42.42.0/23` for d2b.

### Option D — file a support request

If none of the above are viable for your site, file an issue at
`vicondoa/d2b` describing your network topology. Cross-host
route reconciliation may be added in a future release.

## Why this isn't a framework bug

The intersection is a configuration choice. The broker's
`ApplyRoute` op correctly refuses to install an overlapping route
(kernel `EEXIST`) — silent overwriting could mis-route operator
traffic. The plan §8 explicitly categorizes this as site-specific
non-deferral; v1.2 documents the remediation here rather than
adding cross-host network-state arbitration to the broker.

## Related

- ADR 0005 — `docs/adr/0005-network-firewall-and-tap-model.md`
- v1.2 plan §1 #11 — site-specific non-deferral categorization
- Plan-v1.1-archived.md L80 — original observation
