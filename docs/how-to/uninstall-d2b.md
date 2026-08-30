# Uninstall or roll back d2b

Use the smallest rollback that solves the problem. Keep a known-good NixOS
generation and preserve d2b state until the Guest and broker owners have
drained cleanly.

## Roll back a Guest

```bash
d2b activation rollback Guest/work-app --zone work --dry-run
d2b activation rollback Guest/work-app --zone work --apply
d2b guest status work-app --zone work
```

The activation Provider and Guest controller fence rollback by identity and
revision. Do not delete TPM, credential, lock, or store-view state to repair a
failed generation.

## Stop d2b

```bash
d2b guest stop work-app --zone work --apply
sudo systemctl stop d2bd.service
```

Only `d2bd.service`, `d2b-broker.socket`, and `d2b-broker.service` are
framework root units. There are no per-Guest lifecycle units to stop.

## Remove the module

1. Remove the d2b module and Zone resource declarations from the host flake.
2. Rebuild to the known-good generation.
3. Confirm no d2b unit is active and back up `/var/lib/d2b/` if recovery data
   is needed.
4. Delete d2b state only after confirming that audit, key, TPM, credential,
   lock, and Guest store data is intentionally disposable.

Do not sweep `/run/d2b` or mutate foreign ownership markers. Use the broker's
typed status and audit output to identify any remaining owned state.
