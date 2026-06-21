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

Use `nixling realm list` and `nixling realm inspect <realm>` to inspect the
rendered host-resident vs gateway-backed policy and the gateway's local
lifecycle state.

## Credential boundary

The host declaration carries non-secret coordinates and state paths only.
Relay/provider credentials are enrolled from inside the gateway guest and
stored as an encrypted runtime envelope under that gateway's state
directory. Host-side gateway credential reads and Relay Send bearer
minting are rejected; `allowHostRelayCredentials` is retained only as a
compatibility option that produces a clear error for older configs.

## Enroll relay credentials in the gateway

Start the gateway, enter it, then run enrollment from inside the guest.
The helper reads the plaintext enrollment JSON from stdin so long-lived
keys never appear in argv:

```bash
nixling realm enter work
sudo -u nixlingd NIXLING_GATEWAY_STATE_DIR=/var/lib/nixling/gateways/work \
  nixling-gateway-enroll enroll \
  /var/lib/nixling/gateways/work/credential.sealed.json \
  /var/lib/nixling/gateways/work/seal.key <<'JSON'
{
  "relayListen": { "keyName": "gateway-listen", "key": "<listen-rule-key>" },
  "relaySend": { "keyName": "gateway-send", "key": "<send-rule-key>" }
}
JSON
```

Enrollment creates the sealing key if it does not exist, writes both
files with mode `0600`, and emits only the new credential generation as
JSON. The sealed credential file does not contain the Relay rule keys in
plaintext.

## Rotate credentials

Rotate by passing the replacement JSON through the same in-guest helper:

```bash
nixling realm enter work
sudo -u nixlingd NIXLING_GATEWAY_STATE_DIR=/var/lib/nixling/gateways/work \
  nixling-gateway-enroll rotate \
  /var/lib/nixling/gateways/work/credential.sealed.json \
  /var/lib/nixling/gateways/work/seal.key <<'JSON'
{
  "relayListen": { "keyName": "gateway-listen", "key": "<new-listen-rule-key>" },
  "relaySend": { "keyName": "gateway-send", "key": "<new-send-rule-key>" }
}
JSON
```

Rotation must unseal the existing envelope, increments the gateway
credential generation, and rewrites the envelope atomically. Existing
display/session credentials bound to an older generation are rejected by
the gateway verifier on reconnect.

## Recovery

If the seal key is lost, the existing credential envelope cannot be
unsealed. Remove both the stale `credential.sealed.json` and `seal.key`
inside the gateway guest, then enroll fresh Relay credentials. Treat this
as credential rotation at the provider: revoke the old Relay rules or keys
before enrolling replacements.
