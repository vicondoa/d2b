# `provider-registry-v2.json` schema (`v2`)

Schema: [`provider-registry-v2.json`](./provider-registry-v2.json)

This private artifact binds canonical provider descriptors to closed,
per-axis configuration rows. Bindings carry canonical realm, workload, and
provider IDs plus opaque intent IDs resolved through the integrity-verified
bundle. They never carry argv, host paths, or credential material.

## Contract notes

- `schemaVersion` is `v2`.
- `registryGeneration` must match every descriptor generation.
- Provider entries are bounded, unique, and sorted by canonical provider ID.
- Each local-runtime descriptor's provider ID is re-derived from its realm and
  workload IDs. Its configuration fingerprint and scope digest are recomputed
  from the closed binding, while startup factory composition verifies exact
  placement, implementation, and capabilities.
- The current closed binding variant is `local-runtime`. It maps explicit
  local-VM and qemu-media workloads to matching VM-start and runner intents.
- An explicit zero-row artifact is valid. A missing artifact is not.
- Azure VM implementations, credential providers, and `RuntimeExecute` are not
  live host registrations.
