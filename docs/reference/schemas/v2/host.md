# `host.json` schema (`v2`)

Schema: [`host.json`](./host.json)

`host.json` is the private host-topology and host-policy artifact. The
broker consumes it for read-only `host check`, host-prepare planning,
and daemon/broker execution.

## Top-level fields

- `schemaVersion` — schema directory/version for this artifact.
- `site` — host-wide site policy toggles.
- `environments` — per-env network/firewall data.
- `cloudHypervisorCapabilities` — capability matrix anchored to CH.
- `fdOwnership` — broker-opened fd ownership table.
- `runtimeProviders` — local runtime/provider catalog and support matrix.
- `vmRuntimes` — per-VM runtime/provider rows with provider-neutral topology.
- `qemuMedia` — optional qemu-media source contract. Physical USB sources
  carry opaque refs and use the root-only enrollment registry; direct
  `image-file` sources carry operator-authored absolute image paths from Nix
  config and use `registryScope = "direct-config-path"`.
- `hostsFile` — marked-block ownership rule for `/etc/hosts`.
- `kernelModules` — allowed kernel module matrix and load policy.
- `networkManager` — unmanaged-file materialization rules.
- `nftables` — exact `inet d2b` table declaration.
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
- `runtimeProviders` and `vmRuntimes` are additive v2 fields; older fixtures may
  omit them, while current Nix emitters include them for daemon lifecycle/status
  joins.
- `qemuMedia.sources[].imagePath` is present only for direct image-file
  sources. The path is not a physical USB identity and may appear in this
  private bundle, but the broker still fail-closes on unsafe paths, non-raw
  formats, non-regular files, mounted/loop-backed files, and held locks before
  returning a media fd.

## Contract notes

- The schema is `additionalProperties: false` throughout the security-
  sensitive host-prepare objects.
- `host check --read-only` and host-prepare drift gates are expected to stay
  in lock-step with this document.
