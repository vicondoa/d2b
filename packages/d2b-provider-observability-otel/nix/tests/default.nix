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
  };
  # A declared Provider with a valid configuration asserts nothing.
  valid = lib.evalModules {
    modules = [
      base
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources.observability-otel = {
          type = "Provider";
          spec.config.selfMetrics.enable = true;
        };
      }
    ];
  };
  # A declared Provider carrying a field this family does not own fails, and
  # it names the field's path rather than the row's absence.
  unsupportedField = lib.evalModules {
    modules = [
      base
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources.observability-otel = {
          type = "Provider";
          spec.config.selfMetrics = { enable = true; };
          spec.config.collectorEndpoint = "http://127.0.0.1:4317";
        };
      }
    ];
  };
  # A non-boolean selfMetrics.enable fails the same closed check.
  nonBoolean = lib.evalModules {
    modules = [
      base
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources.observability-otel = {
          type = "Provider";
          spec.config.selfMetrics.enable = "yes";
        };
      }
    ];
  };
  # No declared Provider asserts nothing, and projects nothing.
  absent = lib.evalModules {
    modules = [
      base
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources.guest = { type = "Guest"; spec = { }; };
      }
    ];
  };
in
{
  cases = {
    # A declared Provider is checked by both of the family's closed config
    # checks, and a config that satisfies them passes: NixOS emits the rows
    # unconditionally and fails on a row whose `assertion` is false, so this
    # reads the verdict rather than the presence of a row.
    "provider-observability-otel/valid-config-passes-both-checks" = {
      expr = {
        verdicts = map (row: row.assertion) valid.config.assertions;
        checks = builtins.length valid.config.assertions;
      };
      expected = { verdicts = [ true true ]; checks = 2; };
    };

    "provider-observability-otel/unsupported-config-field-refuses" = {
      expr = lib.all (row: row.assertion) unsupportedField.config.assertions;
      expected = false;
    };

    "provider-observability-otel/non-boolean-self-metrics-refuses" = {
      expr = lib.all (row: row.assertion) nonBoolean.config.assertions;
      expected = false;
    };

    "provider-observability-otel/absent-provider-asserts-nothing" = {
      expr = absent.config.assertions;
      expected = [ ];
    };
  };
}
