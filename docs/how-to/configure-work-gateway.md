# Configure gateway-backed Zone transport

Gateway-backed isolation uses one Gateway Guest per Zone. The Gateway Guest
is an execution context, not a second d2b control plane. Separate Zones never
share a Gateway Guest or L2 bridge.

## Declare the Zone resources

```nix
{
  d2b.zones.work = {
    parentZone = "local-root";
    resources = {
      host = {
        type = "Host";
        spec.providerRef = "Provider/system-core";
      };
      gateway = {
        type = "Guest";
        spec = {
          providerRef = "Provider/runtime-cloud-hypervisor";
          systemArtifactId = "gateway-guest-system";
        };
      };
      zone-link = {
        type = "ZoneLink";
        spec = {
          childZoneName = "work";
          transportProviderRef = "Provider/transport-unix";
          transportSettings = { };
          transportCredentials = [ ];
        };
      };
    };
  };
}
```

Supply the matching evaluator through
`d2b.guestSystems.work.gateway`. The Guest controller creates and reconciles
its direct child Resources; the ZoneLink controller owns transport effects.

## Credential custody

Relay credentials, remote registries, Provider configuration, and Zone audit
remain inside the Gateway Guest execution context. The host declaration and
public Resource API carry only typed references and bounded metadata.

```bash
d2b guest status gateway --zone work
d2b guest start gateway --zone work --apply
d2b zone status Zone/work --json
```

Do not put tokens, raw socket paths, remote node names, or enrollment keys in
Nix Resource specs, examples, or CLI output. A missing session, stale
generation, or forged relay identity fails closed rather than falling back to
host-held credentials.
