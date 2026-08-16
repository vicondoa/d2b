# Focused Device Provider eval coverage for GPU shape, settings, and platform
# admission. The physical backing remains opaque to Nix.
{ mkEval, lib, pkgs, system, ... }:

let
  gpuSettingsSchema = {
    type = "object";
    additionalProperties = false;
    properties = {
      renderNodeOnly = { type = "boolean"; };
      videoSidecar = { type = "boolean"; };
      videoNvidiaDecode = { type = "boolean"; };
      contextTypes = {
        type = "array";
        minItems = 1;
        maxItems = 3;
        uniqueItems = true;
        items = {
          type = "string";
          enum = [ "virgl" "virgl2" "cross-domain" ];
        };
      };
      displays = {
        type = "array";
        maxItems = 8;
        items = {
          type = "object";
          additionalProperties = false;
          required = [ "hidden" ];
          properties.hidden = { type = "boolean"; };
        };
      };
      egl = { type = "boolean"; };
      vulkan = { type = "boolean"; };
      crossDomainTrusted = { type = "boolean"; };
      virglVideo = { type = "boolean"; };
    };
  };

  base = { ... }: {
    d2b.artifacts.device-gpu = {
      package = pkgs.writeText "d2b-device-gpu-provider" "device-gpu";
      type = "provider";
    };
    d2b._providerSettingsValidation.schemas =
      { "device-gpu.d2bus.org/Device/spec" = {
          schemaId = "device-gpu.d2bus.org/Device/spec";
          schemaVersion = "v1.0";
          settingsSchema = gpuSettingsSchema;
        }; };
    d2b.zones.local-root.resources = {
      device-gpu = {
        type = "Provider";
        spec = {
          artifactId = "device-gpu";
          config = { };
        };
      };
      gpu = {
        type = "Device";
        spec = {
          providerRef = "Provider/device-gpu";
          deviceClass = "physical";
          arbitration = "exclusive";
          maxConcurrentClaims = 1;
          inventory.selector = {
            busClass = "drm";
            label = "host-gpu";
          };
          provider = {
            schemaId = "device-gpu.d2bus.org/Device/spec";
            schemaVersion = "v1.0";
            settings = {
              renderNodeOnly = false;
              videoSidecar = false;
              videoNvidiaDecode = false;
              contextTypes = [ "virgl" "virgl2" ];
              displays = [ { hidden = true; } ];
              egl = true;
              vulkan = true;
              crossDomainTrusted = false;
              virglVideo = false;
            };
          };
        };
      };
    };
  };

  evaluated = mkEval [ base ];
  failures = lib.filter (assertion: !assertion.assertion)
    evaluated.config.assertions;
  gpuFailures = lib.filter
    (assertion: lib.hasInfix "d2b.zones.local-root.resources.gpu" assertion.message)
    failures;
  hasPlatformFailure = lib.any
    (assertion: lib.hasInfix "gpu-platform-unsupported" assertion.message)
    gpuFailures;
in
{
  "device-gpu/valid-shape" = {
    expr = gpuFailures;
    expected = if system == "x86_64-linux" then [ ] else gpuFailures;
  };

  "device-gpu/platform-gate" = {
    expr = hasPlatformFailure;
    expected = system != "x86_64-linux";
  };
}
