# Migrate a legacy realm gateway

**Diataxis category:** how-to.

Realm gateway guest declarations are no longer a supported Nix surface.
`d2b.gateways`, its nested ACA and Relay configuration, automatic gateway VM
creation, and the associated in-guest enrollment workflow must not be used for
new deployments.

For an existing configuration:

1. Remove every `d2b.gateways.<name>` declaration.
2. Preserve the existing `d2b.envs` and `d2b.vms` declarations while they
   remain the active workload and network substrate.
3. Declare realm intent under `d2b.realms.<realm>`.
4. Record only non-secret provider references under
   `d2b.realms.<realm>.providers`.
5. Keep credentials outside Nix and wait for a supported provider-agent
   enrollment and composition surface before treating ACA or Relay metadata as
   operational.

Do not create a replacement gateway VM manually, write
`/etc/d2b/gateway.json`, or use legacy gateway enrollment binaries. Those paths
do not recreate the canonical provider-agent credential boundary.

See [the v1.2 to v2 migration guide](./migrate-d2b-v1-2-to-v2.md),
[realm options](../reference/realm-options.md), and
[provider-managed sandboxes](../reference/provider-managed-sandboxes.md).
