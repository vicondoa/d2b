# Write a d2b addon

An addon is a sibling flake that exports a normal NixOS module or Provider
contract. It is composed into a consumer-owned Guest evaluator or declared as
a Zone Provider; it is not a private d2b lifecycle extension.

## Keep ownership separate

- d2b owns Zone resources, daemon/broker lifecycle, and host effects.
- The addon owns its Guest-side behavior or Provider implementation.
- The consumer selects the addon for a particular Zone and Guest.

Keep addon options outside `d2b.*` unless the option is part of an explicitly
reviewed d2b contract.

## Compose a Guest evaluator

```nix
{
  d2b.guestSystems.work.work-app = {
    config = {
      imports = [ inputs.my-addon.nixosModules.default ];
      system.build.toplevel = inputs.workGuestSystem;
    };
  };
}
```

The host still declares only `Guest/work-app` and its immutable
`systemArtifactId`. The Guest controller owns child Resources; the addon does
not add a per-Guest systemd service or a second lifecycle API.

## Compose a Provider

Provider artifacts are declared under `d2b.artifacts`; a Zone-local Provider
Resource selects the artifact by ID. Provider effects use assigned
controllers and typed broker operations. Never expose a host path, credential,
argv, pidfd, or private runtime locator in the public spec.

## Align inputs and test

Use the consumer's `nixpkgs` for d2b and the addon:

```nix
inputs.my-addon.inputs.nixpkgs.follows = "nixpkgs";
```

Add an eval-only test for the Guest or Provider contract and owner-local
tests for behavior. Run `make test-nix-unit`, `make test-policy`,
`make test-drift`, and the relevant fixture target.

## Host effects

An addon must not mutate cgroups, nftables, NetworkManager, systemd-networkd,
devices, sockets, or d2b state directly. Request a reviewed typed contract
extension if the existing Provider and broker operations cannot express the
required effect.
