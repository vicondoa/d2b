# Same-name Guests in separate Zones

This example declares `work` and `personal` as child Zones. Each Zone owns a
Guest with a different local name, and both use the same Provider contract.
ResourceRefs stay Zone-local while the controller and broker derive private
runtime identity from immutable Zone and Guest identity.

The important shape is:

```text
Zone/work     -> Guest/work-app
Zone/personal -> Guest/personal-app
```

There is no global Guest-name lookup and no second environment hierarchy.

## Evaluate

```bash
nix flake check --no-build --no-write-lock-file
```

## Operate

```bash
d2b guest list --zone work
d2b guest list --zone personal
d2b guest start work-app --zone work --apply
d2b guest start personal-app --zone personal --apply
```

The artifacts are placeholders for a fast eval-only fixture. Replace them
with signed Provider and Guest system artifacts before deployment.
