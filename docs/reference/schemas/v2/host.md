# `host.json` schema (`v2`)

Schema: [`host.json`](./host.json)

`host.json` is the private host-topology and host-policy artifact. The
broker consumes it for read-only `host check`, host-prepare planning,
and W4+ daemon/broker execution.

## Top-level fields

- `schemaVersion` — schema directory/version for this artifact.
- `site` — host-wide site policy toggles.
- `environments` — per-env network/firewall data.
- `cloudHypervisorCapabilities` — capability matrix anchored to CH.
- `fdOwnership` — broker-opened fd ownership table.
- `hostsFile` — marked-block ownership rule for `/etc/hosts`.
- `kernelModules` — allowed kernel module matrix and load policy.
- `networkManager` — unmanaged-file materialization rules.
- `nftables` — exact `inet nixling` table declaration.
- `ifNameMappings` — derived bridge/TAP name exposure.
- `ch` — Cloud Hypervisor handoff probe result.
- `firewallCoexistencePolicy` — host firewall coexistence contract.

## Nested fields called out by the Layer-1 prose gate

- `kernelModules` documents the trusted module matrix the broker may touch.
- `ifnameMapping` is the per-link visible→derived ifname record carried by
  `ifNameMappings`.
- `bridgePortFlags` is the per-link bridge-port policy emitted under the
  environment/networking rows.
- `firewallCoexistence` is the environment-level coexistence summary that
  rolls up into the top-level `firewallCoexistencePolicy`.
- `ipv6Sysctls` records the ordered per-link IPv6-disable writes.
- `ch` stays optional for older fixtures but is part of the current v2 prose.

## Contract notes

- The schema is `additionalProperties: false` throughout the security-
  sensitive W3/W4 host-prepare objects.
- `host check --read-only` and host-prepare drift gates are expected to stay
  in lock-step with this document.
