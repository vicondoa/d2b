# Minimal Zone Guest

This example evaluates one headless `Guest/personal-dev` in
`Zone/local-root`.

Nix supplies two immutable artifacts and the Guest evaluator:

- `guest-system` is the selected NixOS system artifact.
- `cloud-hypervisor-provider` is the installed runtime Provider artifact.
- `d2b.guestSystems.local-root.personal-dev` supplies the evaluator result.

The Guest controller derives and reconciles its Process, Endpoint, and Volume
children. Nix does not author those children or a separate lifecycle service.

## Evaluate

```bash
nix flake check --no-build --no-write-lock-file
```

## Operate

After replacing the placeholder artifacts with real signed artifacts and
switching a host configuration:

```bash
d2b guest list --zone local-root
d2b guest status personal-dev --zone local-root
d2b guest start personal-dev --zone local-root --apply
d2b guest stop personal-dev --zone local-root --apply
```

The example uses placeholder derivations so evaluation is hermetic. A
production Guest evaluator should build a real NixOS system closure.
