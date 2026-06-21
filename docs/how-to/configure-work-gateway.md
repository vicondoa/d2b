# Configure a work realm gateway

**Diataxis category:** how-to.

Use a dedicated gateway guest for each work or provider realm. Do not share a
gateway guest, nixling env, or L2 bridge with personal realms.

## Declare the realm and gateway

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
  relay.entity = "hc-nixling-work";
};
```

Then start the gateway like any other VM:

```bash
nixling vm start sys-work-gateway --apply
```

## Inspect the policy

```bash
nixling realm list
nixling realm inspect work
```

The output reports whether a realm is host-resident or gateway-backed, the
gateway VM when present, its local lifecycle state, and the default-deny
cross-realm posture.

## Enroll credentials inside the gateway

Realm relay/provider credentials are enrolled from inside the gateway guest.
The host declaration contains only non-secret coordinates and never parses or
stores credential material.

```bash
nixling realm enter work
sudo -u nixlingd NIXLING_GATEWAY_STATE_DIR=<gateway-state-dir> \
  nixling-gateway-enroll enroll \
  <gateway-state-dir>/credential.sealed.json \
  <gateway-state-dir>/seal.key < enrollment.json
```

Use placeholder or test credentials only in examples and fixtures. Do not
commit live provider ids, tokens, keys, host paths, or user identifiers.
