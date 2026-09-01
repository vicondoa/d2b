# Provider-managed sandboxes

**Diataxis category:** reference.

Provider-managed sandboxes are an optional Provider contract, distinct from a
local d2b host and its Cloud Hypervisor Guest resources.

## Acceptance status

The Azure Container Apps (ACA) adapter is **not U19 or U20 acceptance
evidence**. ACA testing is deferred until after the U20 `/etc/nixos` host
switch, d2b startup, and Cloud Hypervisor Guest boot are complete. No ACA
lifecycle, exec, display, audio, or isolation claim in this repository should
be read as an acceptance result.

## Contract boundary

An eventual sandbox Provider may expose a bounded subset of Provider
operations, with its own upstream authentication, retry, rate-limit, and
resource identity rules. It must:

- remain behind the Zone Resource API;
- keep upstream credentials and registries in its Provider execution context;
- return typed capability and failure states;
- avoid exposing upstream IDs, paths, tokens, or request bodies publicly; and
- never become a fallback for the local Guest controller or broker.

The current local acceptance target is the Cloud Hypervisor Guest path:

```text
Zone -> Guest controller -> Process/Endpoint/Volume children -> d2b-broker
```

## Deferred work

After U20, a separate ACA verification pass may establish which capabilities
are actually supported. Until then, use the local Zone/Guest contracts and
do not cite ACA as tested, accepted, or interchangeable with a local Guest.

See [Zone CLI contract](./zone-cli-contract.md),
[Provider capability matrix](./provider-capability-matrix.md), and
[the compatibility policy](./compatibility.md).
