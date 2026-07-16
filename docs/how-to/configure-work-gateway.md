# Migrate a work gateway declaration

**Diataxis category:** how-to.

The former `d2b.gateways.<name>` configuration and its nested Relay and ACA
sandbox fields are removed. They no longer declare a gateway guest. A
non-empty declaration fails evaluation with a migration error.

Keep the existing env as the active network substrate and move non-secret
realm intent to `d2b.realms`:

```nix
d2b.envs.work = {
  lanSubnet = "10.44.0.0/24";
  uplinkSubnet = "192.0.2.0/30";
};

d2b.realms.work = {
  placement = "provider-agent";
  placementProvider = "aca";
  env = "work";
  network.envs = [ "work" ];

  providers.aca = {
    kind = "aca";
    placement = "provider-agent";
    capabilityRefs = [ "runtime" ];
    configRef = "work-aca";
  };
};
```

`providers.aca` and `configRef` are non-secret planning metadata. They do not
launch an ACA provider agent, enroll Azure credentials, or open Relay
connections in the current Nix implementation. Do not put provider
coordinates, credentials, tokens, or enrollment payloads in Nix.

There is currently no supported replacement for starting or entering an
auto-declared gateway VM. Do not hand-write gateway artifacts or invoke legacy
gateway enrollment helpers. Follow
[the v1.2 to v2 migration guide](./migrate-d2b-v1-2-to-v2.md), then consult
[provider-managed sandboxes](../reference/provider-managed-sandboxes.md) for
the implemented provider contract and its availability boundary.
