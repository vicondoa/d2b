# Daemon autostart contract

The daemon performs one bounded startup pass over current Zone Guest
resources. It is idempotent, degraded-aware, and never creates a second
lifecycle authority.

## Startup order

After restoring and reconciling broker-owned runner state, d2bd:

1. relists committed Zone resources;
2. resolves Provider and dependency readiness;
3. starts eligible Guest/Process resources in deterministic order; and
4. records typed outcomes without blocking the public socket.

A missing bundle or unavailable Provider leaves the daemon serving
read-only status and reports the failure; it does not guess a Guest name or
fall back to SSH.

## Concurrency and outcomes

`d2b.daemon.autostart.parallelism` bounds concurrent startup work. The pass
continues for independent Zones and records:

- `Started`;
- `AlreadyRunning`;
- `NotAutostart`;
- `Failed { reason }`; or
- `Degraded { reason }`.

The report is persisted under the daemon state owner and contains bounded
Zone, Resource, generation, and outcome metadata.

## Idempotency

Every start is fenced by the committed Resource identity and Provider
generation. Repeating the pass against a live current Process returns
`AlreadyRunning` and does not spawn a duplicate. A restart adopts only
matching immutable identity; stale or uncertain state is quarantined.

## Configuration

```nix
d2b.daemon.autostart.parallelism = 3;
```

Autostart policy is part of the private daemon/Provider contract. It is not a
public Guest child graph or a caller-supplied start argument.

## Verification

```bash
d2b zone list
d2b guest list --zone local-root
d2b host doctor --read-only
```

Owner-local tests cover deterministic ordering, concurrency bounds,
degraded propagation, and idempotent re-entry. The current private
compatibility bundle may retain historical field names while the Zone
Resource migration completes; that projection cannot authorize a new
lifecycle path.
