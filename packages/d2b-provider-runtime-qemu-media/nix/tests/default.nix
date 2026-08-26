{ lib, ... }:

let
  evaluated = lib.evalModules {
    modules = [ (import ../default.nix) ];
  };
  configured = lib.evalModules {
    modules = [
      (import ../default.nix)
      {
        config.d2b.qemuMediaRuntime = {
          enable = true;
          qmpReadyTimeoutSeconds = 45;
          runtimeTmpfsQuotaBytes = 32 * 1024 * 1024;
        };
      }
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
          host-system = { type = "Host"; spec = { }; };
          runtime-qemu-media = {
            type = "Provider";
            spec.config.controllerExecutionRef = "Host/host-system";
          };
          guest = {
            type = "Guest";
            spec = {
              providerRef = "Provider/runtime-qemu-media";
              systemArtifactId = "guest-system";
              provider.settings = { };
            };
          };
        };
      }
    ];
  };
  incompleteProvider = lib.evalModules {
    modules = [
      projectionBase
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources = {
          runtime-qemu-media = {
            type = "Provider";
            spec = { };
          };
          guest = {
            type = "Guest";
            spec.providerRef = "Provider/runtime-qemu-media";
          };
        };
      }
    ];
  };
  sameName = lib.evalModules {
    modules = [
      projectionBase
      (import ../projection.nix)
      {
        config.d2b.zones = {
          alpha.resources = {
            runtime-qemu-media = {
              type = "Provider";
              spec.config.controllerExecutionRef = "Host/host";
            };
            host = { type = "Host"; spec = { }; };
            guest = {
              type = "Guest";
              spec.providerRef = "Provider/runtime-qemu-media";
            };
          };
          beta.resources = {
            runtime-qemu-media = {
              type = "Provider";
              spec.config.controllerExecutionRef = "Host/host";
            };
            host = { type = "Host"; spec = { }; };
            guest = {
              type = "Guest";
              spec.providerRef = "Provider/runtime-qemu-media";
            };
          };
        };
      }
    ];
  };
  invalidProvider = lib.evalModules {
    modules = [
      projectionBase
      (import ../projection.nix)
      {
        config.d2b.zones.dev.resources = {
          runtime-qemu-media = {
            type = "Provider";
            spec.config = {
              controllerExecutionRef = "Host/missing";
              unsupported = true;
            };
          };
        };
      }
    ];
  };
in
{
  cases = {
    "provider-runtime-qemu-media/modules-evaluate" = {
      expr = builtins.deepSeq evaluated.config.d2b.qemuMediaRuntime true;
      expected = true;
      propagateError = true;
    };

    "provider-runtime-qemu-media/defaults-and-bounds" = {
      expr = {
        enabled = configured.config.d2b.qemuMediaRuntime.enable;
        readyTimeout = configured.config.d2b.qemuMediaRuntime.qmpReadyTimeoutSeconds;
        quota = configured.config.d2b.qemuMediaRuntime.runtimeTmpfsQuotaBytes;
      };
      expected = {
        enabled = true;
        readyTimeout = 45;
        quota = 32 * 1024 * 1024;
      };
    };

    "provider-runtime-qemu-media/guest-process-projection" = {
      expr = {
        enabled = projected.config.d2b._resourceCompiler
          .providerProjectionRuntimeQemuMedia.enabled;
        process = projected.config.d2b._resourceCompiler
          .providerProjectionRuntimeQemuMedia.processesByZone.dev."qemu-media-guest".spec;
        endpoint = projected.config.d2b._resourceCompiler
          .providerProjectionRuntimeQemuMedia.resourcesByZone.dev
          ."qemu-guest-qmp".spec.purpose;
        guest = projected.config.d2b._resourceCompiler
          .providerProjectionRuntimeQemuMedia.guestPatchesByZone.dev.guest.provider;
        processRefs = projected.config.d2b._resourceCompiler
          .providerProjectionRuntimeQemuMedia.privateArtifact.processRefs;
        endpointRefs = projected.config.d2b._resourceCompiler
          .providerProjectionRuntimeQemuMedia.privateArtifact.endpointRefs;
      };
      expected = {
        enabled = true;
        process = {
          providerRef = "Provider/system-minijail";
          executionRef = "Host/host-system";
          domain = "system";
          processClass = "worker";
          template = "qemu-media-runner";
          desiredLifecycle = "running";
          networkUsage = null;
          deviceUsage = [ ];
        };
        endpoint = "runtime-qemu-media.d2bus.org/qmp";
        guest = {
          schemaId = "runtime-qemu-media.d2bus.org/Guest/spec";
          schemaVersion = "1.0";
          settings = {
            pauseAtBoot = true;
            displayWindow = false;
            serialConsole = true;
            tablet = true;
            rtcBase = "utc";
          };
        };
        processRefs = [ "Process/qemu-media-guest" ];
        endpointRefs = [ "Endpoint/qemu-guest-qmp" ];
      };
    };

    "provider-runtime-qemu-media/present-incomplete-provider-emits-no-projection" = {
      expr = let
        projection = incompleteProvider.config.d2b._resourceCompiler
          .providerProjectionRuntimeQemuMedia;
      in {
        enabled = projection.enabled;
        processes = projection.processesByZone.dev;
        resources = projection.resourcesByZone.dev;
        processRefs = projection.privateArtifact.processRefs;
        endpointRefs = projection.privateArtifact.endpointRefs;
        guestPatches = projection.guestPatchesByZone.dev;
      };
      expected = {
        enabled = false;
        processes = { };
        resources = { };
        processRefs = [ ];
        endpointRefs = [ ];
        guestPatches = { };
      };
    };

    "provider-runtime-qemu-media/endpoints-have-live-process-producers" = {
      expr = let
        projection = projected.config.d2b._resourceCompiler
          .providerProjectionRuntimeQemuMedia;
        processRefs = map
          (process: "${process.type}/${process.metadata.name}")
          (lib.attrValues projection.processesByZone.dev);
      in lib.all
        (endpoint: builtins.elem endpoint.spec.producerRef processRefs)
        (lib.attrValues projection.resourcesByZone.dev);
      expected = true;
    };

    "provider-runtime-qemu-media/absent-provider-emits-nothing" = {
      expr = let
        absent = lib.evalModules {
          modules = [
            projectionBase
            (import ../projection.nix)
            {
              config.d2b.zones.dev.resources.guest = {
                type = "Guest";
                spec.providerRef = "Provider/runtime-qemu-media";
              };
            }
          ];
        };
        projection = absent.config.d2b._resourceCompiler
          .providerProjectionRuntimeQemuMedia;
      in {
        processes = projection.processesByZone.dev;
        patches = projection.guestPatchesByZone;
      };
      expected = {
        processes = { };
        patches = {
          dev = { };
        };
      };
    };

    "provider-runtime-qemu-media/same-guest-name-stays-zone-scoped" = {
      expr = {
        alpha = sameName.config.d2b._resourceCompiler
          .providerProjectionRuntimeQemuMedia.processesByZone.alpha
          ."qemu-media-guest".metadata.ownerRef;
        beta = sameName.config.d2b._resourceCompiler
          .providerProjectionRuntimeQemuMedia.processesByZone.beta
          ."qemu-media-guest".metadata.ownerRef;
      };
      expected = {
        alpha = "Guest/guest";
        beta = "Guest/guest";
      };
    };

    "provider-runtime-qemu-media/rejects-invalid-provider-settings" = {
      expr = lib.any
        (record: !record.assertion)
        invalidProvider.config.assertions;
      expected = true;
    };
  };
}
