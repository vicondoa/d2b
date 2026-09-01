{ lib, ... }:

let
  evaluated = lib.evalModules {
    modules = [ (import ../default.nix) ];
  };
  enabled = lib.evalModules {
    modules = [
      (import ../default.nix)
      { config.d2b.audio.v3.enable = true; }
    ];
  };
  projectionBase = {
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
  projected = lib.evalModules {
    modules = [
      projectionBase
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources = {
          audio-host = { type = "Host"; spec = { }; };
          controller-host = { type = "Host"; spec = { }; };
          audio-pipewire = {
            type = "Provider";
            spec.config = {
              captureAlias = null;
              hostExecutionRef = "Host/audio-host";
              controllerExecutionRef = "Host/controller-host";
            };
          };
          audio-binding = {
            type = "audio.d2bus.org.AudioBinding";
            spec = {
              providerRef = "Provider/audio-pipewire";
              targetRef = "Guest/guest";
              guestUsers = [ "User/alice" ];
            };
          };
          guest = { type = "Guest"; spec = { }; };
          alice = { type = "User"; spec = { }; };
        };
      }
    ];
  };
  incompleteProjection = lib.evalModules {
    modules = [
      projectionBase
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources = {
          host-system = { type = "Host"; spec = { }; };
          audio-pipewire = {
            type = "Provider";
            spec.config.captureAlias = null;
          };
          audio-binding = {
            type = "audio.d2bus.org.AudioBinding";
            spec = {
              providerRef = "Provider/audio-pipewire";
              targetRef = "Guest/guest";
            };
          };
          guest = { type = "Guest"; spec = { }; };
        };
      }
    ];
  };
  controllerOnlyProjection = lib.evalModules {
    modules = [
      projectionBase
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources = {
          audio-host = { type = "Host"; spec = { }; };
          controller-host = { type = "Host"; spec = { }; };
          audio-pipewire = {
            type = "Provider";
            spec.config.controllerExecutionRef = "Host/controller-host";
          };
          audio-binding = {
            type = "audio.d2bus.org.AudioBinding";
            spec = {
              providerRef = "Provider/audio-pipewire";
              targetRef = "Guest/guest";
            };
          };
          guest = { type = "Guest"; spec = { }; };
        };
      }
    ];
  };
  invalidHostProjection = lib.evalModules {
    modules = [
      projectionBase
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources = {
          controller-host = { type = "Host"; spec = { }; };
          audio-pipewire = {
            type = "Provider";
            spec.config = {
              hostExecutionRef = "Host/missing";
              controllerExecutionRef = "Host/controller-host";
            };
          };
          audio-binding = {
            type = "audio.d2bus.org.AudioBinding";
            spec = {
              providerRef = "Provider/audio-pipewire";
              targetRef = "Guest/guest";
            };
          };
          guest = { type = "Guest"; spec = { }; };
        };
      }
    ];
  };
  invalidProjection = lib.evalModules {
    modules = [
      projectionBase
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources.audio-pipewire = {
          type = "Provider";
          spec.config = {
            captureAlias = "bad alias";
            unsupported = true;
          };
        };
      }
    ];
  };
in
{
  cases = {
    "provider-audio-pipewire/modules-evaluate" = {
      expr = builtins.deepSeq evaluated.config.d2b._audioV3 true;
      expected = true;
      propagateError = true;
    };

    "provider-audio-pipewire/defaults-are-provider-owned" = {
      expr = {
        providerRef = evaluated.config.d2b._audioV3.providerRef;
        stateVolume = evaluated.config.d2b._audioV3.declaresStateVolume;
        enabled = enabled.config.d2b._audioV3.enabled;
      };
      expected = {
        providerRef = "Provider/audio-pipewire";
        stateVolume = false;
        enabled = true;
      };
    };

    "provider-audio-pipewire/binding-processes-are-owner-local" = {
      expr = {
        enabled = projected.config.d2b._resourceCompiler
          .providerProjectionAudioPipewire.enabled;
        processes = lib.attrNames (projected.config.d2b._resourceCompiler
          .providerProjectionAudioPipewire.processesByZone.dev);
        endpoints = lib.attrNames (projected.config.d2b._resourceCompiler
          .providerProjectionAudioPipewire.resourcesByZone.dev);
        processRefs = projected.config.d2b._resourceCompiler
          .providerProjectionAudioPipewire.privateArtifact.processRefs;
        hostExecution = projected.config.d2b._resourceCompiler
          .providerProjectionAudioPipewire.processesByZone.dev
          ."audio-host-audio-binding".spec.executionRef;
      };
      expected = {
        enabled = true;
        processes = [ "audio-guest-audio-binding" "audio-host-audio-binding" ];
        endpoints = [ "audio-guest-audio-binding" "audio-host-audio-binding" ];
        processRefs = [
          "Process/audio-host-audio-binding"
          "Process/audio-guest-audio-binding"
        ];
        hostExecution = "Host/audio-host";
      };
    };
    "provider-audio-pipewire/missing-host-execution-emits-no-host-children" = {
      expr = let
        projection = incompleteProjection.config.d2b._resourceCompiler
          .providerProjectionAudioPipewire;
      in {
        processes = lib.attrNames (projection.processesByZone.dev);
        endpoints = lib.attrNames (projection.resourcesByZone.dev);
        processRefs = projection.privateArtifact.processRefs;
      };
      expected = {
        processes = [ "audio-guest-audio-binding" ];
        endpoints = [ "audio-guest-audio-binding" ];
        processRefs = [ "Process/audio-guest-audio-binding" ];
      };
    };
    "provider-audio-pipewire/controller-ref-does-not-place-host-child" = {
      expr = let
        projection = controllerOnlyProjection.config.d2b._resourceCompiler
          .providerProjectionAudioPipewire;
      in {
        processes = lib.attrNames (projection.processesByZone.dev);
        endpoints = lib.attrNames (projection.resourcesByZone.dev);
        processRefs = projection.privateArtifact.processRefs;
      };
      expected = {
        processes = [ "audio-guest-audio-binding" ];
        endpoints = [ "audio-guest-audio-binding" ];
        processRefs = [ "Process/audio-guest-audio-binding" ];
      };
    };
    "provider-audio-pipewire/invalid-host-execution-emits-no-host-children" = {
      expr = let
        projection = invalidHostProjection.config.d2b._resourceCompiler
          .providerProjectionAudioPipewire;
      in {
        processes = lib.attrNames (projection.processesByZone.dev);
        endpoints = lib.attrNames (projection.resourcesByZone.dev);
        processRefs = projection.privateArtifact.processRefs;
      };
      expected = {
        processes = [ "audio-guest-audio-binding" ];
        endpoints = [ "audio-guest-audio-binding" ];
        processRefs = [ "Process/audio-guest-audio-binding" ];
      };
    };
    "provider-audio-pipewire/rejects-invalid-provider-settings" = {
      expr = lib.any
        (record: !record.assertion)
        invalidProjection.config.assertions;
      expected = true;
    };
  };
}
