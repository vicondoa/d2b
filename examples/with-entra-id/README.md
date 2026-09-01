# External identity in a Zone Guest

This example reserves `Guest/work-entra` in the `work` Zone for a
consumer-owned identity evaluator. The host keeps only the semantic Guest
resource and selected artifact references. Entra credentials and identity
state belong inside the Guest execution context.

`work-entra.nix` is a guest-side module for a consumer's nested NixOS
evaluation. Wire that evaluator into
`d2b.guestSystems.work.work-entra` and set `d2b.artifacts.guest-system` to its
matching `system.build.toplevel` before deployment.

## Evaluate

```bash
nix flake check --no-build --no-write-lock-file
```

## Operate

```bash
d2b guest status work-entra --zone work
d2b guest start work-entra --zone work --apply
```

The checked host configuration uses placeholder artifacts so it does not
fetch or build tenant-specific identity software during evaluation.
