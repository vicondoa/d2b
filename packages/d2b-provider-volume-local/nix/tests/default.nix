{ lib, modules, ... }:

let
  module = builtins.head modules;
  evaluated = lib.evalModules {
    modules = [
      { _module.check = false; }
      ({ lib, ... }: {
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
      })
      module
    ];
  };
  projectionBase = [
    { _module.check = false; }
    ({ lib, ... }: {
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
    })
    module
  ];
  baseResources = {
    storage-host = {
      type = "Host";
      spec = { };
    };
    guest = {
      type = "Guest";
      spec.deviceAttachments = [{
        deviceRef = "Device/tpm";
        exclusive = true;
      }];
    };
    tpm = {
      type = "Device";
      spec.providerRef = "Provider/device-tpm";
    };
    state = {
      type = "Volume";
      spec.attachments = [{
        executionRef = "Guest/guest";
        transport = "virtiofs";
      }];
    };
  };
  volumeLocalProvider = {
    type = "Provider";
    spec.config.controllerExecutionRef = "Host/storage-host";
  };
  volumeVirtiofsProvider = {
    type = "Provider";
    spec.config.controllerExecutionRef = "Host/storage-host";
  };
  projected = lib.evalModules {
    modules = projectionBase ++ [{
      config.d2b.zones.dev.resources = {
        volume-local = volumeLocalProvider;
        volume-virtiofs = volumeVirtiofsProvider;
      } // baseResources;
    }];
  };
  noProviders = lib.evalModules {
    modules = projectionBase ++ [{
      config.d2b.zones.dev.resources = baseResources;
    }];
  };
  incompleteProvider = lib.evalModules {
    modules = projectionBase ++ [{
      config.d2b.zones.dev.resources = {
        volume-local = {
          type = "Provider";
          spec.config = { };
        };
      } // baseResources;
    }];
  };
  mixedProviders = lib.evalModules {
    modules = projectionBase ++ [{
      config.d2b.zones.dev.resources = {
        volume-local = volumeLocalProvider;
        volume-virtiofs = {
          type = "Provider";
          spec = { };
        };
      } // baseResources;
    }];
  };
  legacyTpmFlag = lib.evalModules {
    modules = projectionBase ++ [{
      config.d2b.zones.dev.resources = {
        storage-host = {
          type = "Host";
          spec = { };
        };
        volume-local = volumeLocalProvider;
        volume-virtiofs = volumeVirtiofsProvider;
        guest = {
          type = "Guest";
          spec.tpmEnabled = true;
        };
      };
    }];
  };
in
{
  cases = {
    "provider-volume-local/modules-evaluate" = {
      expr = builtins.deepSeq evaluated.config.d2b.volumes true;
      expected = true;
      propagateError = true;
    };

    "volume-local/module-is-a-module" = {
      expr = builtins.isFunction (import ../default.nix);
      expected = true;
    };

    "volume-local/volume-options-are-declared" = {
      expr = builtins.hasAttr "volumes" evaluated.options.d2b;
      expected = true;
    };
    "volume-local/guest-store-view-projection-is-watched" = {
      expr = builtins.hasAttr "store-view-guest"
        projected.config.d2b._resourceCompiler.providerProjectionVolumeLocal.resourcesByZone.dev;
      expected = true;
    };
    "volume-local/tpm-volume-follows-device-attachment" = {
      expr = builtins.hasAttr "swtpm-guest"
        projected.config.d2b._resourceCompiler.providerProjectionVolumeLocal.resourcesByZone.dev;
      expected = true;
    };
    "volume-local/valid-providers-emit-current-generated-resources" = {
      expr = let
        compiler = projected.config.d2b._resourceCompiler;
        projection = compiler.providerProjectionVolumeLocal;
      in {
        enabled = projection.enabled;
        names = lib.attrNames projection.resourcesByZone.dev;
        compatibilityProviders = compiler.volumeGenerated.providers;
        private = projection.privateArtifact.resourceNames;
        storeSource = projection.resourcesByZone.dev.store-view-guest
          .spec.source.executionRef;
        storeOwner = projection.resourcesByZone.dev.store-view-guest
          .metadata.ownerRef or null;
        tpmSource = projection.resourcesByZone.dev.swtpm-guest
          .spec.source.executionRef;
        tpmOwner = projection.resourcesByZone.dev.swtpm-guest
          .metadata.ownerRef or null;
      };
      expected = {
        enabled = true;
        names = [ "store-view-guest" "swtpm-guest" "vol-state-vfd" ];
        compatibilityProviders = [ ];
        private = [
          "Volume/store-view-guest"
          "Volume/swtpm-guest"
          "User/vol-state-vfd"
        ];
        storeSource = "Host/storage-host";
        storeOwner = null;
        tpmSource = "Host/storage-host";
        tpmOwner = "Guest/guest";
      };
    };
    "volume-local/absent-provider-emits-no-children" = {
      expr = let
        compiler = noProviders.config.d2b._resourceCompiler;
        projection = compiler.providerProjectionVolumeLocal;
      in {
        enabled = projection.enabled;
        resources = projection.resourcesByZone.dev or { };
        compatibility = compiler.volumeGenerated.byZone.dev or { };
        private = projection.privateArtifact.resourceNames;
      };
      expected = {
        enabled = false;
        resources = { };
        compatibility = { };
        private = [ ];
      };
    };
    "volume-local/incomplete-provider-emits-no-children" = {
      expr = let
        compiler = incompleteProvider.config.d2b._resourceCompiler;
        projection = compiler.providerProjectionVolumeLocal;
      in {
        enabled = projection.enabled;
        resources = projection.resourcesByZone.dev or { };
        compatibility = compiler.volumeGenerated.byZone.dev or { };
        private = projection.privateArtifact.resourceNames;
      };
      expected = {
        enabled = false;
        resources = { };
        compatibility = { };
        private = [ ];
      };
    };
    "volume-local/mixed-providers-do-not-synthesize-virtiofs-children" = {
      expr = let
        compiler = mixedProviders.config.d2b._resourceCompiler;
        projection = compiler.providerProjectionVolumeLocal;
      in {
        names = lib.attrNames projection.resourcesByZone.dev;
        providers = compiler.volumeGenerated.providers;
      };
      expected = {
        names = [ "swtpm-guest" ];
        providers = [ ];
      };
    };
    "volume-local/legacy-tpm-flag-does-not-project-state" = {
      expr = let
        resources = legacyTpmFlag.config.d2b._resourceCompiler
          .providerProjectionVolumeLocal.resourcesByZone.dev;
      in {
        storeView = builtins.hasAttr "store-view-guest" resources;
        swtpm = builtins.hasAttr "swtpm-guest" resources;
      };
      expected = {
        storeView = true;
        swtpm = false;
      };
    };
  };
}
