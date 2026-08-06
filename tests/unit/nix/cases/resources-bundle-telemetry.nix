{ lib, ... }:

let
  bundle = import ../../../../nixos-modules/resources-bundle.nix { inherit lib; };
  resourceTypes = import ../../../../nixos-modules/resources.nix { inherit lib; };
  valid = {
    work = {
      type = "Zone";
      spec = {
        telemetry.emitter.ringCapacityBytes = 4194304;
        audit.retentionDays = 30;
        audit.maxSegmentBytes = 67108864;
      };
    };
    observability-otel = {
      type = "Provider";
      spec = {
        artifactId = "provider-observability-otel";
        config.selfMetrics.enable = true;
      };
    };
    signoz-api-key = {
      type = "Credential";
      spec.credentialRef = "Credential/signoz-api-key";
    };
  };
  validBundle = bundle.bundleForZone "work" valid;
  invalidType = bundle.validateBundle "work" {
    bad = { type = "Unknown"; spec = { }; };
  };
  invalidRing = bundle.validateBundle "work" {
    work = {
      type = "Zone";
      spec.telemetry.emitter.ringCapacityBytes = 0;
    };
  };
  invalidProvider = bundle.validateBundle "work" {
    provider = {
      type = "Provider";
      spec.config = { unknown = true; };
    };
  };
  invalidRuntime = bundle.validateBundle "work" {
    work = {
      type = "Zone";
      spec = {
        status = "authored";
        telemetry.emitter.ringCapacityBytes = 4194304;
      };
    };
  };
  invalidSecret = bundle.validateBundle "work" {
    work = {
      type = "Zone";
      spec = { config = { accessToken = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4In0.sig"; }; };
    };
  };
  typedSpec = value:
    let
      evaluated = (lib.evalModules {
        modules = [{
          options.value = lib.mkOption {
            type = resourceTypes.telemetryResourceSpecType;
          };
          config.value = value;
        }];
      }).config.value;
    in builtins.tryEval (builtins.deepSeq evaluated true);
in
{
  "resources-bundle/known-type" = {
    expr = (builtins.filter (assertion: !assertion.assertion) validBundle.assertions);
    expected = [ ];
  };
  "resources-bundle/sorted-resources" = {
    expr = map (resource: "${resource.type}/${resource.metadata.name}") validBundle.data.resources;
    expected = [
      "Credential/signoz-api-key"
      "Provider/observability-otel"
      "Zone/work"
    ];
  };
  "resources-bundle/deterministic-digest" = {
    expr = (bundle.bundleForZone "work" valid).digest == validBundle.digest;
    expected = true;
  };
  "resources-bundle/content-hash-is-prefixed" = {
    expr = validBundle.contentHash == "sha256:${validBundle.digest}";
    expected = true;
  };
  "resources-bundle/unknown-type-rejected" = {
    expr = invalidType.valid == false && invalidType.errors != [ ];
    expected = true;
  };
  "resources-bundle/ring-bound-rejected" = {
    expr = invalidRing.valid == false && invalidRing.errors != [ ];
    expected = true;
  };
  "resources-bundle/provider-config-rejected" = {
    expr = invalidProvider.valid == false && invalidProvider.errors != [ ];
    expected = true;
  };
  "resources-bundle/runtime-field-rejected" = {
    expr = invalidRuntime.valid == false && invalidRuntime.errors != [ ];
    expected = true;
  };
  "resources-bundle/secret-shaped-value-rejected" = {
    expr = invalidSecret.valid == false && invalidSecret.errors != [ ];
    expected = true;
  };
  "resources-bundle/invalid-generated-bundle-aborts" = {
    expr = (builtins.tryEval (bundle.bundleForZone "work" {
      bad = { type = "Unknown"; spec = { }; };
    })).success;
    expected = false;
  };
  "resources-bundle/invalid-generated-bundle-does-not-render" = {
    expr = (builtins.tryEval (bundle.compileBundle {
      zoneName = "work";
      resources = {
        work = {
          type = "Zone";
          spec.telemetry.emitter.ringCapacityBytes = 0;
        };
      };
    })).success;
    expected = false;
  };
  "resources/types-accept-bounded-telemetry" = {
    expr = (typedSpec {
      telemetry.emitter.ringCapacityBytes = 4194304;
      audit.retentionDays = 30;
      audit.maxSegmentBytes = 67108864;
    }).success;
    expected = true;
  };
  "resources/types-reject-out-of-bounds-telemetry" = {
    expr = (typedSpec {
      telemetry.emitter.ringCapacityBytes = 0;
    }).success;
    expected = false;
  };
}
