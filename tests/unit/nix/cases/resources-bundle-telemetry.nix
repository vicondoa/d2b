{ lib, ... }:

let
  bundle = import ../../../../nixos-modules/resources-bundle.nix { inherit lib; };
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
  invalidType = bundle.bundleForZone "work" {
    bad = { type = "Unknown"; spec = { }; };
  };
  invalidRing = bundle.bundleForZone "work" {
    work = {
      type = "Zone";
      spec.telemetry.emitter.ringCapacityBytes = 0;
    };
  };
  invalidProvider = bundle.bundleForZone "work" {
    provider = {
      type = "Provider";
      spec.config = { unknown = true; };
    };
  };
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
  "resources-bundle/unknown-type-rejected" = {
    expr = (builtins.length (builtins.filter (assertion: !assertion.assertion) invalidType.assertions)) > 0;
    expected = true;
  };
  "resources-bundle/ring-bound-rejected" = {
    expr = (builtins.length (builtins.filter (assertion: !assertion.assertion) invalidRing.assertions)) > 0;
    expected = true;
  };
  "resources-bundle/provider-config-rejected" = {
    expr = (builtins.length (builtins.filter (assertion: !assertion.assertion) invalidProvider.assertions)) > 0;
    expected = true;
  };
}
