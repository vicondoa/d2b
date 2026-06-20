# Configure and use a realm gateway

**Diataxis category:** how-to.

Realm gateways are the local entrypoint for gateway-backed realms. The
host starts and enters the gateway VM as a normal nixling workload, while
realm relay credentials, provider configuration, remote registries, and
realm audit live inside the gateway guest.

## Declare a gateway-backed realm

Add one gateway per trust-boundary realm and keep each gateway in a
separate nixling environment:

```nix
nixling.envs.work = {
  lanSubnet = "10.44.0.0/24";
  uplinkSubnet = "192.0.2.0/30";
};

nixling.gateways.work = {
  realm = "work";
  env = "work";
  index = 20;
  relay.namespace = "relns-example.servicebus.windows.net";
  relay.entity = "hc-nixling-display";
};
```

The module auto-declares the gateway VM, publishes a
`realm-entrypoints.json` table, and keeps the local realm host-resident.
Multiple gateways are allowed only when they use distinct realm paths,
gateway VM names, and nixling env/L2 segments.

## Start and enter the gateway

Start the gateway like any other VM:

```bash
nixling vm start sys-work-gateway --apply
```

Then enter the realm trust boundary:

```bash
nixling realm enter work
```

For scripts, run a one-shot command inside the gateway:

```bash
nixling realm run work -- nixling vm list
```

## Route a realm target

Local VM names still use the host fast path:

```bash
nixling vm start personal-dev --apply
```

Gateway-backed targets use DNS-shaped names:

```bash
nixling vm exec demo.aca.work.nixling -- foot
```

If the gateway is missing, stopped, or not reported by the daemon, the
CLI fails closed with a typed remediation instead of falling back to host
credentials or SSH.

## Credential boundary

The host declaration carries non-secret coordinates and state paths only.
The transitional host-resident ACA/Relay proof path is guarded by
`allowHostRelayCredentials = false` by default and is not the production
realm model. Production realm rollout keeps relay/provider credentials in
the gateway guest.
