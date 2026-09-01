# Zone-owned observability Provider

This example places an observability Provider and `Guest/work-app` in the
`work` Zone. The Provider is a Zone resource; it is not an auto-declared
environment or a host-global VM.

The Guest controller owns Guest lifecycle and child Resources. The
observability Provider remains an effect owner and may create its own
transitive resources through the normal controller graph.

## Evaluate

```bash
nix flake check --no-build --no-write-lock-file
```

## Operate

```bash
d2b provider list --zone work
d2b guest status work-app --zone work
d2b guest start work-app --zone work --apply
```

The Provider and Guest artifacts are placeholders for eval-only use. Replace
them with the signed packages and catalog entries for the deployment.
