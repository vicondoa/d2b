{ lib, ... }:

let
  base = {
    options.assertions = lib.mkOption {
      type = lib.types.listOf lib.types.anything;
      default = [ ];
    };
    options.d2b.zones = lib.mkOption {
      type = lib.types.attrs;
      default = { };
    };
    options.d2b._resourceCompiler = lib.mkOption {
      type = lib.types.attrs;
      default = { };
      internal = true;
      visible = false;
    };
  };
  enabled = lib.evalModules {
    modules = [
      base
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources = {
          observability-otel = { type = "Provider"; spec = { }; };
          guest = { type = "Guest"; spec = { }; };
          telemetry = {
            type = "telemetry.d2bus.org.TelemetryBinding";
            spec = {
              providerRef = "Provider/observability-otel";
              producerRef = "Guest/guest";
            };
          };
        };
      }
    ];
  };
  absent = lib.evalModules {
    modules = [
      base
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources.guest = {
          type = "Guest";
          spec = { };
        };
      }
    ];
  };
in
{
  cases = {
    "provider-observability-otel/guest-process" = {
      expr = enabled.config.d2b._resourceCompiler
        .providerProjectionObservabilityOtel.processesByZone.dev
        ."otel-binding-telemetry".spec.template;
      expected = "otel-collector-edge";
    };

    "provider-observability-otel/absent-provider" = {
      expr = absent.config.d2b._resourceCompiler
        .providerProjectionObservabilityOtel.processesByZone.dev;
      expected = { };
    };
  };
}
