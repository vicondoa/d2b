# Zone resource facts for Provider/observability-otel.
#
# This module validates the Provider's declared configuration and projects
# nothing. Telemetry payloads remain ComponentSession data, and the collector
# and forwarder a TelemetryBinding owns are its runtime children's own
# declarations (`otel-collector` and `otel-vsock-forwarder`), which that
# family's controller composes at reconcile time.
#
# It previously also projected one target-local `Process` row per
# Guest-scoped TelemetryBinding (`otel-collector-edge`). That row declared a
# worker template no signed Provider artifact pinned, so it never received a
# trusted launch intent and the daemon refused its launch terminally with
# `provider-ticket:template-not-found`. Nothing in production read the row or
# its phase, so the declaration was a capability the tree could not install
# and did not need. It is gone rather than left to refuse.
{ config, lib, ... }:

let
  cfg = config.d2b;
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
in
{
  config.assertions = lib.concatLists
    (map providerAssertions (lib.attrNames zones));
}
