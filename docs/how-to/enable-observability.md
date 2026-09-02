# Enable observability

Observability is a Provider projection in the current Zone resource plane.
There is no auto-declared telemetry environment, host-global Guest, or
second lifecycle service.

## Declare the Provider

Declare the signed Provider artifact and a Zone-local Provider resource:

```nix
{
  d2b.artifacts.observability-provider = {
    package = inputs.d2bObservabilityProvider.packages.${system}.default;
    type = "provider";
  };

  d2b.zones.work.resources.observability = {
    type = "Provider";
    spec = {
      artifactId = "observability-provider";
      config.controllerExecutionRef = "Host/host";
    };
  };
}
```

The Provider owns collector, storage, and export effects through its assigned
controller. Its configuration is semantic and private; do not place tokens,
socket paths, host paths, or executable arguments in a Resource spec.

## Attach a Guest

Guests remain ordinary Zone resources:

```nix
d2b.zones.work.resources.work-app = {
  type = "Guest";
  spec = {
    providerRef = "Provider/runtime-cloud-hypervisor";
    systemArtifactId = "work-guest-system";
  };
};
```

The Guest controller owns its child graph and waits for the observability
Provider only when the Guest contract declares that dependency. It does not
create a special telemetry Guest or bypass the Resource API.

## Verify

```bash
d2b provider list --zone work
d2b provider status observability --zone work
d2b guest status work-app --zone work
d2b op inspect --json
d2b host doctor --read-only
```

Provider and Guest status are typed and redacted. A missing Provider,
unavailable session, or transport failure is visible as a degraded result;
the CLI does not fall back to host-held credentials or a static endpoint.

## Security

Keep telemetry credentials and remote exporter configuration inside the
Provider-owned Guest or approved Provider execution context. Public Zone
status may expose bounded capability and health state, never tokens, raw
paths, collector arguments, or private runtime locators.
