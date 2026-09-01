# Zone resource projection for Provider/observability-otel.
#
# Telemetry payloads remain ComponentSession data. Nix contributes only
# target-local edge Process intents and private Endpoint identities.
{ config, lib, ... }:

let
  cfg = config.d2b;
  providerRef = "Provider/observability-otel";
  zones = cfg.zones or { };
  resourcesFor = zoneName: zones.${zoneName}.resources or { };

  providerFor = zoneName:
    if builtins.hasAttr "observability-otel" (resourcesFor zoneName)
      && (resourcesFor zoneName).observability-otel.type == "Provider"
    then (resourcesFor zoneName).observability-otel
    else null;

  providerAssertions = zoneName:
    let
      provider = providerFor zoneName;
      providerConfig =
        if provider == null then { } else provider.spec.config or { };
      selfMetrics = providerConfig.selfMetrics or { };
    in lib.optionals (provider != null) [
      {
        assertion = lib.all (key: key == "selfMetrics")
          (lib.attrNames providerConfig);
        message = "d2b.zones.${zoneName}.resources.observability-otel.spec.config contains an unsupported Provider field.";
      }
      {
        assertion = !(builtins.hasAttr "selfMetrics" providerConfig)
          || (builtins.isAttrs selfMetrics
            && lib.all (key: key == "enable") (lib.attrNames selfMetrics)
            && builtins.isBool (selfMetrics.enable or null));
        message = "d2b.zones.${zoneName}.resources.observability-otel.spec.config.selfMetrics.enable must be boolean.";
      }
    ];

  bindingRows = zoneName:
    if providerFor zoneName == null
    then [ ]
    else lib.mapAttrsToList
      (bindingName: binding: {
        inherit zoneName bindingName binding;
        spec = binding.spec or { };
      })
      (lib.filterAttrs
        (_: resource:
          resource.type == "telemetry.d2bus.org.TelemetryBinding"
          && (resource.spec.providerRef or null) == providerRef)
        (resourcesFor zoneName));

  processFor = row:
    let producerRef = row.spec.producerRef or null;
    in lib.optionalAttrs (
      builtins.isString producerRef && lib.hasPrefix "Guest/" producerRef
    ) {
      type = "Process";
      metadata = {
        name = "otel-binding-${row.bindingName}";
        zone = row.zoneName;
        ownerRef = "telemetry.d2bus.org.TelemetryBinding/${row.bindingName}";
      };
      spec = {
        providerRef = "Provider/system-systemd";
        executionRef = producerRef;
        domain = "system";
        processClass = "worker";
        template = "otel-collector-edge";
        desiredLifecycle = "running";
        deviceUsage = [ ];
        networkUsage = null;
      };
    };

  processesForZone = zoneName:
    lib.listToAttrs (lib.filter
      (entry: entry.value != { })
      (map
        (row: lib.nameValuePair "otel-binding-${row.bindingName}"
          (processFor row))
        (bindingRows zoneName)));

in
{
  config = {
    assertions = lib.concatLists
      (map providerAssertions (lib.attrNames zones));
    d2b._resourceCompiler.providerProjectionObservabilityOtel = {
      enabled = lib.any
        (zoneName: processesForZone zoneName != { })
        (lib.attrNames zones);
      processesByZone = lib.genAttrs (lib.attrNames zones) processesForZone;
      resourcesByZone = { };
      guestPatchesByZone = { };
      privateArtifact = {
        schemaVersion = 1;
        providerRef = providerRef;
        processRefs = lib.concatMap
          (zoneName: map
            (resource: "Process/${resource.metadata.name}")
            (lib.attrValues (processesForZone zoneName)))
          (lib.attrNames zones);
      };
    };
  };
}
