{ mkModuleEval, lib, pkgs, system, d2bLib, ... }:

let
  guestSystemPackage = pkgs.writeText "zone-control-guest-system" "guest-system";
  runtimePackage = pkgs.writeText "zone-control-runtime-provider" "runtime-provider";
  processPackage = pkgs.writeText "zone-control-process-provider" "process-provider";

  providerResources = {
    runtime = {
      type = "Provider";
      spec.artifactId = "runtime-provider";
    };
    process = {
      type = "Provider";
      spec.artifactId = "process-provider";
    };
  };

  guestResources = zoneName: providerResources // {
    guest = {
      type = "Guest";
      spec = {
        providerRef = "Provider/runtime";
        systemArtifactId = "guest-system";
      };
    };
    worker = {
      type = "Process";
      metadata.ownerRef = "Provider/runtime";
      spec = {
        providerRef = "Provider/process";
        executionRef = "Guest/guest";
        domain = "system";
        defaultDomain = "user";
        allowedDomains = [ "system" "user" ];
        processClass = "service";
        template = "guest-service";
      };
    };
    activation = {
      type = "EphemeralProcess";
      metadata.ownerRef = "Provider/runtime";
      spec = {
        providerRef = "Provider/process";
        executionRef = "Guest/guest";
        processClass = "worker";
        template = "activation-nixos-runner";
      };
    };
  };

  validFixture = { ... }: {
    d2b.artifacts = {
      guest-system = {
        package = guestSystemPackage;
        type = "nixos-system";
      };
      runtime-provider = {
        package = runtimePackage;
        type = "provider";
      };
      process-provider = {
        package = processPackage;
        type = "provider";
      };
    };
    d2b.zones.alpha.resources = guestResources "alpha";
    d2b.zones.beta.resources = guestResources "beta";
    d2b.guestSystems.alpha.guest = {
      config.system.build.toplevel = guestSystemPackage;
    };
    d2b.guestSystems.beta.guest = {
      config.system.build.toplevel = guestSystemPackage;
    };
  };

  assertionsModule = { lib, ... }: {
    options.assertions = lib.mkOption {
      type = lib.types.listOf lib.types.attrs;
      default = [ ];
    };
  };

  evalFixture = extra:
    (mkModuleEval [ assertionsModule validFixture extra ]).config;

  bundleResources = cfg: zone:
    cfg.d2b._bundle.zoneResourceBundlesV3.${zone}.data.resources;

  findResource = type: name: resources:
    lib.findFirst
      (resource:
        resource.type == type && resource.metadata.name == name)
      null
      resources;

  invalidAssertions = extra:
    let cfg = evalFixture extra;
    in lib.filter (assertion: !assertion.assertion) cfg.assertions;

  invalidMessages = extra:
    map (assertion: assertion.message) (invalidAssertions extra);

  disabledProjection = (mkModuleEval [
    assertionsModule
    validFixture
    {
      d2b._resourceCompiler.providerProjectionRuntimeQemuMedia = lib.mkForce {
        enabled = false;
        processesByZone.alpha.leaked-process = {
          type = "Process";
          metadata = {
            name = "leaked-process";
            zone = "alpha";
            ownerRef = "Provider/runtime";
          };
          spec = {
            providerRef = "Provider/system-systemd";
            executionRef = "Guest/guest";
            processClass = "service";
            template = "leaked-process";
          };
        };
        resourcesByZone = {
          alpha.leaked = {
            type = "User";
            metadata = {
              name = "leaked";
              zone = "alpha";
            };
            spec = { };
          };
        };
        guestPatchesByZone.alpha.guest = {
          provider = {
            leaked = true;
          };
        };
        privateArtifact = {
          schemaVersion = 1;
          providerRef = "Provider/runtime-qemu-media";
          processRefs = [ ];
          endpointRefs = [ ];
        };
      };
      d2b._resourceCompiler.volumeGenerated = lib.mkForce {
        byZone.alpha.compat-leaked = {
          type = "User";
          metadata = {
            name = "compat-leaked";
            zone = "alpha";
          };
          spec = { };
        };
        users = [ ];
        providers = [ ];
      };
    }
  ]).config;
in
{
  "zone-control/headless-guest-and-provider-process-render" =
    let
      cfg = evalFixture { };
      guest = findResource "Guest" "guest" (bundleResources cfg "alpha");
      process = findResource "Process" "worker" (bundleResources cfg "alpha");
      ephemeral = findResource "EphemeralProcess" "activation"
        (bundleResources cfg "alpha");
      guestRow = builtins.head (d2bLib.v3GuestRows {
        zones = cfg.d2b.zones;
        guestSystems = cfg.d2b.guestSystems;
        artifacts = cfg.d2b.artifacts;
      });
      compatibilityProcess = findResource
        "Process"
        "worker"
        cfg.d2b._bundle.zoneResourceBundlesCompatibility.alpha.data.resources;
    in {
      expr = {
        guestZone = guest.metadata.zone;
        guestArtifact = guest.spec.systemArtifactId;
        guestEvaluatorReady = d2bLib.v3GuestEvaluatorReady guestRow.system;
        guestEvaluatorToplevel = toString
          (d2bLib.v3GuestConfigFor guestRow.system).system.build.toplevel;
        processZone = process.metadata.zone;
        processOwner = process.metadata.ownerRef;
        processProvider = process.spec.providerRef;
        processExecution = process.spec.executionRef;
        compatibilityProcessZone = compatibilityProcess.metadata.zone;
        processGenericFieldsAbsent =
          !(builtins.hasAttr "allowedDomains" process.spec)
          && !(builtins.hasAttr "defaultDomain" process.spec)
          && !(builtins.hasAttr "defaultUserRef" process.spec)
          && !(builtins.hasAttr "networkAttachments" process.spec)
          && !(builtins.hasAttr "deviceAttachments" process.spec)
          && !(builtins.hasAttr "volumeAttachmentDefaults" process.spec);
        processDefaults = {
          desiredLifecycle = process.spec.desiredLifecycle;
          adoptionPolicy = process.spec.adoptionPolicy;
          drainTimeout = process.spec.drainTimeout;
          sandboxEnvironment = process.spec.sandbox.environmentClass;
        };
        ephemeralDefaults = {
          startDeadline = ephemeral.spec.startDeadline;
          runtimeDeadline = ephemeral.spec.runtimeDeadline;
          successfulTtl = ephemeral.spec.successfulTtl;
          failedTtl = ephemeral.spec.failedTtl;
        };
      };
      expected = {
        guestZone = "alpha";
        guestArtifact = "guest-system";
        guestEvaluatorReady = true;
        guestEvaluatorToplevel = toString guestSystemPackage;
        processZone = "alpha";
        processOwner = "Provider/runtime";
        processProvider = "Provider/process";
        processExecution = "Guest/guest";
        compatibilityProcessZone = "alpha";
        processGenericFieldsAbsent = true;
        processDefaults = {
          desiredLifecycle = "running";
          adoptionPolicy = "adopt-on-restart";
          drainTimeout = "30s";
          sandboxEnvironment = "minimal";
        };
        ephemeralDefaults = {
          startDeadline = "60s";
          runtimeDeadline = "300s";
          successfulTtl = "1h";
          failedTtl = "24h";
        };
      };
    };

  "zone-control/same-guest-name-is-zone-local" =
    let
      cfg = evalFixture { };
      alphaGuest = findResource "Guest" "guest" (bundleResources cfg "alpha");
      betaGuest = findResource "Guest" "guest" (bundleResources cfg "beta");
      alphaProcess = findResource "Process" "worker" (bundleResources cfg "alpha");
      betaProcess = findResource "Process" "worker" (bundleResources cfg "beta");
    in {
      expr = {
        guestZones = [ alphaGuest.metadata.zone betaGuest.metadata.zone ];
        processZones = [ alphaProcess.metadata.zone betaProcess.metadata.zone ];
      };
      expected = {
        guestZones = [ "alpha" "beta" ];
        processZones = [ "alpha" "beta" ];
      };
    };

  "zone-control/disabled-provider-projection-does-not-merge" =
    let
      cfg = disabledProjection;
      resources = bundleResources cfg "alpha";
      compatibility =
        cfg.d2b._bundle.zoneResourceBundlesCompatibility.alpha.data.resources;
      guest = findResource "Guest" "guest" resources;
    in {
      expr = {
        bundleLeaked = findResource "User" "leaked" resources == null;
        compatibilityProjectionLeaked =
          findResource "User" "compat-leaked" resources == null;
        compatibilityLeaked =
          findResource "User" "leaked" compatibility == null;
        processLeaked =
          findResource "Process" "leaked-process" resources == null;
        guestPatched = builtins.hasAttr "provider" guest.spec;
        privateArtifact = builtins.hasAttr "runtime-qemu-media"
          cfg.d2b._bundle.extraArtifacts;
      };
      expected = {
        bundleLeaked = true;
        compatibilityProjectionLeaked = true;
        compatibilityLeaked = true;
        processLeaked = true;
        guestPatched = false;
        privateArtifact = false;
      };
    };

  "zone-control/missing-guest-system-refuses" = {
    expr = invalidMessages {
      d2b.guestSystems.alpha = lib.mkForce { };
    };
    expected = [
      "d2b.zones.alpha.resources.guest.spec.systemArtifactId must have a matching d2b.guestSystems.alpha.guest evaluator."
    ];
  };

  "zone-control/unknown-guest-artifact-refuses" = {
    expr = invalidMessages {
      d2b.zones.alpha.resources.guest.spec.systemArtifactId =
        lib.mkForce "missing-system";
    };
    expected = [
      "d2b.zones.alpha.resources.guest: systemArtifactId and source.systemArtifactId must resolve to nixos-system artifacts."
      "d2b.zones.alpha.resources.guest.spec.systemArtifactId must match the Guest evaluator toplevel."
      "d2b.zones.alpha.resources.guest: systemArtifactId and source.systemArtifactId must resolve to nixos-system artifacts."
    ];
  };

  "zone-control/cross-zone-process-reference-refuses" = {
    expr = invalidMessages {
      d2b.zones.alpha.resources.worker.spec.executionRef =
        lib.mkForce "Guest/only-in-beta";
    };
    expected = [
      "d2b.zones.alpha.resources.worker.spec.executionRef must resolve to a Host or Guest in the same Zone."
      "d2b.zones.alpha.resources.worker: every ResourceRef must be canonical and resolve in the same Zone."
      "d2b.zones.alpha.resources.worker.spec.executionRef must resolve to a Host or Guest in the same Zone."
    ];
  };

  "zone-control/invalid-process-owner-refuses" = {
    expr = invalidMessages {
      d2b.zones.alpha.resources.worker.metadata.ownerRef =
        lib.mkForce "Provider/missing";
    };
    expected = [
      "d2b.zones.alpha.resources.worker.metadata.ownerRef must resolve in Zone alpha."
    ];
  };

  "zone-control/process-domain-defaults-from-guest" =
    let
      cfg = evalFixture {
        d2b.zones.alpha.resources.worker.spec.domain = lib.mkForce null;
      };
      process = findResource "Process" "worker" (bundleResources cfg "alpha");
    in {
      expr = {
        failures = invalidMessages {
          d2b.zones.alpha.resources.worker.spec.domain = lib.mkForce null;
        };
        domain = process.spec.domain;
        guestDefault = (findResource "Guest" "guest" (bundleResources cfg "alpha"))
          .spec.defaultDomain;
      };
      expected = {
        failures = [ ];
        domain = null;
        guestDefault = "system";
      };
    };

  "zone-control/legacy-guest-system-aliases-do-not-resolve" =
    let
      guestSystem = {
        config.system.build.toplevel = guestSystemPackage;
      };
      legacyResults = map
        (key: d2bLib.v3GuestSystemFor { ${key} = guestSystem; } "alpha" "guest")
        [ "guest" "Guest/guest" "alpha/guest" ];
    in {
      expr = {
        canonicalReady = d2bLib.v3GuestEvaluatorReady (d2bLib.v3GuestSystemFor {
          alpha.guest = guestSystem;
        } "alpha" "guest");
        canonicalToplevel = toString
          (d2bLib.v3GuestConfigFor (d2bLib.v3GuestSystemFor {
            alpha.guest = guestSystem;
          } "alpha" "guest")).system.build.toplevel;
        aliases = map (value: value == null) legacyResults;
      };
      expected = {
        canonicalReady = true;
        canonicalToplevel = toString guestSystemPackage;
        aliases = [ true true true ];
      };
    };

  "zone-control/empty-surface-case-set-fails-closed" =
    let
      support = import ../eval-jobs.nix {
        inherit lib pkgs system;
      };
    in {
      expr = builtins.tryEval (support.evalSurface {
        name = "zone-control-empty";
        cases = { };
      }).message;
      expected = {
        success = false;
        value = false;
      };
    };
}
