# Compatibility policy

The current d2b contract is the Zone resource plane, controller-owned Guest
lifecycle, and daemon-only `d2bd` plus `d2b-broker` control plane. Consumers
must use the exact `nixpkgs` input pinned by the current flake.

## Clean break

The current release line is a clean break from the v1/v2 hierarchy and
lifecycle. Older Realm, environment, VM-first, Gateway-daemon, and bash
configuration is not a supported upgrade input. d2b makes no v1/v2 data
retention, state migration, or rollback-preservation promise.

After switching a current host configuration, the operator may remove old
host declarations and old host-path state once it is no longer needed. The
current daemon and Guest controller do not adopt or preserve those paths.

Historical migration pages remain only as archaeology. They must not be used
as current runbooks or evidence of compatibility.

## Host and Guest validation

Current host acceptance is performed against the operator's `/etc/nixos`
consumer configuration. The acceptance sequence is owned by the host lane:

1. evaluate and build the selected configuration;
2. run `nixos-rebuild dry-activate`;
3. switch the host generation;
4. start d2bd and the broker;
5. boot a Cloud Hypervisor Guest.

U19 does not claim that host acceptance or any remote Provider acceptance has
passed. U20 must run both `make test-host-integration` and
`make test-integration` alongside the real-host sequence. ACA testing is
deferred until after the U20 host switch and Cloud Hypervisor Guest boot.
U19 only leaves those declarations and current inputs converged; it does not
run host acceptance.

## Input alignment

```nix
inputs.d2b.inputs.nixpkgs.follows = "nixpkgs";
```

Do not retarget d2b to an unrelated nixpkgs revision. Use the current
[Zone Nix authoring](./zone-control-nix.md), [CLI contract](./cli-contract.md),
and [daemon lifecycle](../explanation/daemon-lifecycle.md) references.
