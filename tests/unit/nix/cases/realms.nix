# nix-unit coverage for realm option/schema foundations.
{ mkEval, lib, flakeRoot, ... }:

let
  hostBase = {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };

    d2b.site = {
      stateDir = "/var/lib/d2b";
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
    };

    d2b.envs.home = {
      lanSubnet = "10.10.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    d2b.envs.dev = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "198.51.100.0/30";
    };
    d2b.envs.work = {
      lanSubnet = "10.30.0.0/24";
      uplinkSubnet = "203.0.113.0/30";
    };

    d2b.vms.homebox = {
      env = "home";
      index = 10;
      ssh.user = "alice";
      config.users.users.alice = { isNormalUser = true; uid = 1000; };
    };
    d2b.vms.devbox = {
      env = "dev";
      index = 10;
      ssh.user = "alice";
      config.users.users.alice = { isNormalUser = true; uid = 1000; };
    };
    d2b.vms.corp = {
      env = "work";
      index = 10;
      ssh.user = "alice";
      config.users.users.alice = { isNormalUser = true; uid = 1000; };
    };
  };

  realmFixture = lib.recursiveUpdate hostBase {
    d2b.realms.home = {
      name = "Home";
      env = "home";
      network.envs = [ "home" ];
      allowedUsers = [ "alice" "alice" ];
      allowedGroups = [ "realm-home" "realm-home" ];
      broker = {
        enable = true;
        hostMutation = true;
      };
    };

    d2b.realms.dev = {
      parent = "home";
      path = "dev.home";
      env = "dev";
      network = {
        envs = [ "work" "dev" ];
        mode = "inherit-env";
        cidrRefs = [ "lab" "dev" "lab" ];
      };
    };

    d2b.realms.work = {
      parent = "home";
      path = "work.home";
      placement = "gateway-vm";
      env = "work";
      network.envs = [ "work" ];
      providers.aca = {
        kind = "aca";
        placement = "provider-agent";
        capabilityRefs = [ "relay" "aca" "relay" ];
        configRef = "work-aca-non-secret";
      };
      keys = {
        realmIdentityRef = "idref-work";
        realmIdentityFingerprint = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        controllerKeyRef = "cgref-work";
        controllerCredentialFingerprint = "sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
        trustBundleRef = "trust-work";
        enrollmentRef = "enroll-work";
        rotationPolicyRef = "rotate-work";
      };
      relay = {
        enable = true;
        mode = "static";
        endpoints = [ "relns-b.example.invalid" "relns-a.example.invalid" ];
        credentialRef = "work-relay-credential";
      };
      policy.bundleRef = "work-policy";
    };

    d2b.realms.archive = {
      enable = false;
      placement = "provider-specific";
      providerSpecificPlacement = "archived-off-host";
    };
  };

  cfg = (mkEval [ realmFixture ]).config;
  realms = cfg.d2b._index.realms;
  realmHash = path: builtins.substring 0 16 (builtins.hashString "sha256" path);
  realmUnitPrefix = path: "d2b-realm-${realmHash path}";
  homeUnitPrefix = realmUnitPrefix "home";
  devUnitPrefix = realmUnitPrefix "dev.home";
  workUnitPrefix = realmUnitPrefix "work.home";

  failureMessages = modules:
    map (a: a.message)
      (lib.filter (a: !a.assertion) (mkEval modules).config.assertions);

  hasMessage = needles: messages:
    lib.any
      (message: lib.all (needle: lib.hasInfix needle message) needles)
      messages;

  missingParentMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.realms.child = {
        parent = "missing";
        path = "child.missing";
      };
    })
  ];

  parentCycleMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.realms.alpha = {
        path = "alpha";
        parent = "beta";
      };
      d2b.realms.beta = {
        path = "beta";
        parent = "alpha";
      };
    })
  ];

  duplicateIdPathMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.realms.alpha = {
        id = "same";
        path = "same-path";
      };
      d2b.realms.beta = {
        id = "same";
        path = "same-path";
      };
    })
  ];

  duplicateRuntimePathMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.realms.alpha = { };
      d2b.realms.beta.paths = {
        stateDir = "/var/lib/d2b/realms/alpha";
        auditDir = "/var/lib/d2b/audit/realms/alpha";
        runDir = "/run/d2b/realms/alpha";
        publicSocket = "/run/d2b/realms/alpha/public.sock";
        brokerSocket = "/run/d2b/realms/alpha/broker.sock";
      };
    })
  ];

  longSocketPath = "/run/d2b/realms/${lib.concatStrings (lib.genList (_: "a") 96)}/public.sock";

  overlongPublicSocketMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.realms.work.paths.publicSocket = longSocketPath;
    })
  ];

  overlongBrokerSocketMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.realms.work.paths.brokerSocket = longSocketPath;
    })
  ];

  missingPlacementProviderMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.realms.work = {
        placement = "provider-controller";
        providers.aca.kind = "aca";
      };
    })
  ];

  missingProviderSpecificPlacementProviderMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.realms.work = {
        placement = "provider-specific";
        providerSpecificPlacement = "aca-managed-sandbox";
        providers.aca.kind = "aca";
      };
    })
  ];

  unexpectedPlacementProviderMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.realms.work = {
        placement = "gateway-vm";
        placementProvider = "aca";
        providers.aca.kind = "aca";
      };
    })
  ];

  missingRealmAllowedUserMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.realms.home = {
        allowedUsers = [ "missing-user" ];
      };
    })
  ];

  secretIdentityRefMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.realms.work.keys = {
        realmIdentityRef = "secret-identity";
        controllerKeyRef = "-----BEGIN PRIVATE KEY-----";
        trustBundleRef = "SharedAccessKey=must-not-live-in-nix";
        enrollmentRef = "bearer-enrollment-token";
        rotationPolicyRef = "private-key-policy";
      };
    })
  ];

  validProviderPlacementCfg = (mkEval [
    (lib.recursiveUpdate hostBase {
      d2b.realms.work = {
        placement = "provider-agent";
        placementProvider = "aca";
        providers.aca.kind = "aca";
      };
    })
  ]).config;
  validProviderController =
    builtins.head validProviderPlacementCfg.d2b._bundle.realmControllersJson.data.controllers;

  legacyGatewayMessages = failureMessages [
    (lib.recursiveUpdate hostBase {
      d2b.gateways.work = {
        env = "work";
        aca.endpoint = "https://example.azurecontainerapps.invalid";
        aca.resourceGroup = "rg-example";
      };
    })
  ];

  minimalCfg = (mkEval [ (import (flakeRoot + "/examples/minimal/configuration.nix")) ]).config;
  multiEnvCfg = (mkEval [ (import (flakeRoot + "/examples/multi-env/configuration.nix")) ]).config;

  # ── tombstone / migration warning fixtures ──────────────────────────────────

  # Realm linking to d2b.envs but with network.mode="none" and no workloads
  # should emit an advisory "inheritEnvNudge" warning pointing at the
  # v1.2→v2 migration guide.
  inheritEnvNudgeWarnings = (mkEval [
    (lib.recursiveUpdate hostBase {
      d2b.realms.nudge-me = {
        env = "home";
        network.envs = [ "home" ];
        network.mode = "none";
        # no workloads declared
      };
    })
  ]).config.warnings;

  # Realm workload with legacyVmName pointing to a VM that does not exist in
  # d2b.vms should emit an advisory warning.
  orphanLegacyVmWarnings = (mkEval [
    (lib.recursiveUpdate hostBase {
      d2b.realms.migrating = {
        env = "home";
        network.envs = [ "home" ];
        workloads.laptop = {
          legacyVmName = "old-laptop-vm";
          kind = "local-vm";
        };
      };
    })
  ]).config.warnings;

  # Realm with matching legacyVmName that DOES exist in d2b.vms should NOT
  # emit the orphan warning.
  legacyVmPresentWarnings = (mkEval [
    (lib.recursiveUpdate hostBase {
      d2b.realms.migrating = {
        env = "home";
        network.envs = [ "home" ];
        workloads.laptop = {
          legacyVmName = "homebox";
          kind = "local-vm";
        };
      };
    })
  ]).config.warnings;

  # Realm with workloads declared (even without env) does NOT emit the
  # inheritEnvNudge warning.
  withWorkloadsNoNudgeWarnings = (mkEval [
    (lib.recursiveUpdate hostBase {
      d2b.realms.with-workloads = {
        env = "home";
        network.envs = [ "home" ];
        network.mode = "none";
        workloads.main = {
          kind = "local-vm";
        };
      };
    })
  ]).config.warnings;

  # ── accepted workload options shape ─────────────────────────────────────────

  # Verify the full workload options submodule evaluates without assertion
  # failures for each supported kind (local-vm, qemu-media, provider-placeholder)
  # and that per-workload defaults are correctly materialized.
  acceptedWorkloadCfg = (mkEval [
    (lib.recursiveUpdate hostBase {
      d2b.realms.corp = {
        parent = "home";
        path = "corp.home";
        placement = "gateway-vm";
        # No env/network.envs: this fixture tests workload schema only;
        # env linkage would trigger the NixOS systemd.network warning.
        workloads.laptop = {
          kind = "local-vm";
          legacyVmName = null;
          localVm = {
            ssh.user = "alice";
            memoryMiB = 4096;
            vcpus = 2;
            graphics.enable = false;
            tpm.enable = false;
            autostart = false;
          };
          launcher = {
            enable = true;
            label = "Corp Laptop";
            icon.id = "computer-laptop";
            icon.name = "laptop";
            capabilities = [ "guest-exec" ];
          };
        };
        workloads.installer = {
          kind = "qemu-media";
          qemuMedia.source = {
            kind = "physical-usb";
            ref = "installer-usb";
            readOnly = true;
          };
          launcher.enable = true;
          launcher.label = "Live Installer";
        };
        workloads.cloud-service = {
          kind = "provider-placeholder";
          launcher.enable = false;
        };
      };
      d2b.realms.home = {
        name = "Home";
        allowedUsers = [ "alice" ];
      };
    })
  ]).config;

  realmHostPlan = import ../eval-cases/realm-host-wave-plan.nix;
  componentNames = builtins.attrNames realmHostPlan.components;
  componentPosition = lib.listToAttrs (lib.imap0
    (position: component: {
      name = component;
      value = position;
    })
    realmHostPlan.componentOrder);
  componentBranches = map
    (component: realmHostPlan.components.${component}.branch)
    componentNames;
  allOwnedFiles = lib.concatMap
    (component: realmHostPlan.components.${component}.ownedFiles)
    componentNames;
  allReservedPaths = lib.concatMap
    (component: realmHostPlan.components.${component}.reservedPaths)
    componentNames;
  ownershipCounts = builtins.foldl'
    (counts: path:
      counts // {
        ${path} = (counts.${path} or 0) + 1;
      })
    { }
    allOwnedFiles;
  duplicateOwnedFiles = builtins.attrNames
    (lib.filterAttrs (_: count: count != 1) ownershipCounts);
  unownedReservedPaths = lib.filter
    (path: !(builtins.elem path allOwnedFiles))
    allReservedPaths;
  missingOwnedFiles = lib.filter
    (path: !(builtins.pathExists (flakeRoot + "/${path}")))
    allOwnedFiles;
  unreservedMissingOwnedFiles = lib.filter
    (path: !(builtins.elem path allReservedPaths))
    missingOwnedFiles;
  dependencyErrors = lib.concatMap
    (component:
      let
        position = componentPosition.${component};
        dependencies = realmHostPlan.components.${component}.dependsOn;
      in
      lib.concatMap
        (dependency:
          lib.optional
            (!(builtins.hasAttr dependency realmHostPlan.components)
              || componentPosition.${dependency} >= position)
            "${component}:${dependency}")
        dependencies)
    realmHostPlan.componentOrder;
  externalDependencyErrors = lib.concatMap
    (component:
      map
        (dependency: "${component}:${dependency}")
        (lib.filter
          (dependency:
            !(builtins.hasAttr dependency realmHostPlan.externalDependencies))
          realmHostPlan.components.${component}.externalDependsOn))
    componentNames
    ++ map
      (row: "path:${row.dependency}")
      (lib.filter
        (row:
          !(builtins.hasAttr row.dependency realmHostPlan.externalDependencies))
        realmHostPlan.pathExternalDependencies);
  listNixFiles = prefix: directory:
    lib.concatMap
      (name:
        let
          entryType = (builtins.readDir directory).${name};
          relative = if prefix == "" then name else "${prefix}/${name}";
          path = directory + "/${name}";
        in
        if entryType == "directory"
        then listNixFiles relative path
        else lib.optional (entryType == "regular" && lib.hasSuffix ".nix" name)
          "nixos-modules/${relative}")
      (builtins.attrNames (builtins.readDir directory));
  currentNixFiles = lib.sort lib.lessThan
    (listNixFiles "" (flakeRoot + "/nixos-modules"));
  plannedCurrentNixFiles = lib.sort lib.lessThan (lib.filter
    (path:
      lib.hasPrefix "nixos-modules/" path
      && builtins.pathExists (flakeRoot + "/${path}"))
    allOwnedFiles);
  providerExtensionFragments =
    builtins.attrValues realmHostPlan.providerRegistryExtensionSeams.fragments;
  providerFragmentOwnershipValid = lib.all
    (fragment:
      builtins.elem fragment.path
        realmHostPlan.components.${fragment.owner}.ownedFiles)
    providerExtensionFragments;
  deletionExtensionRows =
    realmHostPlan.deletionContractTestExtensionSeam.owners;
  deletionExtensionPaths =
    lib.concatMap (row: row.extensionPaths) deletionExtensionRows;
  deletionExtensionOwnershipValid = lib.all
    (row:
      lib.all
        (path:
          builtins.elem path realmHostPlan.components.${row.component}.ownedFiles)
        row.extensionPaths
      && lib.all
        (path:
          builtins.elem path realmHostPlan.components.${row.component}.deletes)
        row.deletedFiles
      && builtins.elem
        realmHostPlan.deletionContractTestExtensionSeam.dependency
        realmHostPlan.components.${row.component}.externalDependsOn)
    deletionExtensionRows;
  affectedInventoryRows =
    lib.concatLists (builtins.attrValues realmHostPlan.affectedInventory);
  affectedInventoryPaths =
    lib.concatMap (row: row.paths) affectedInventoryRows;
  affectedInventoryReservedPaths =
    lib.concatMap (row: row.reservedPaths) affectedInventoryRows;
  isAffectedInventoryPath = path:
    path == "README.md"
    || path == "tests/migration-ledger.toml"
    || path == "tests/migration-state.d/polkit-allowlist-eval.toml"
    || path == "tests/migration-state.d/vm-submodule-cutover-eval.toml"
    || path == "tests/migration-state.d/vm-submodule-eval.toml"
    || path == "tests/static.sh"
    || lib.hasPrefix "docs/explanation/" path
    || lib.hasPrefix "docs/how-to/" path
    || lib.hasPrefix "docs/reference/" path
    || lib.hasPrefix "examples/" path
    || lib.hasPrefix "packages/d2b-contract-tests/" path
    || lib.hasPrefix "templates/" path
    || lib.hasPrefix "tests/fixtures/" path
    || lib.hasPrefix "tests/golden/" path
    || lib.hasPrefix "tests/unit/nix/" path
    || lib.hasPrefix "tests/unit/smoke/" path;
  ownedAffectedInventoryPaths =
    lib.filter isAffectedInventoryPath allOwnedFiles;
  reservedAffectedInventoryPaths =
    lib.filter isAffectedInventoryPath allReservedPaths;
  w5ReservedPaths =
    realmHostPlan.crossWaveOwnership.w5RuntimeDocs
    ++ realmHostPlan.crossWaveOwnership.w5ConsumerFiles
    ++ realmHostPlan.crossWaveOwnership.foreignW5Fixtures;
  crossWaveOverlaps = lib.intersectLists allOwnedFiles w5ReservedPaths;
  allForeignAffectedPaths =
    w5ReservedPaths
    ++ realmHostPlan.crossWaveOwnership.deferredPurgeDocs.paths
    ++ realmHostPlan.crossWaveOwnership.w6RuntimeDocs.paths
    ++ realmHostPlan.crossWaveOwnership.frozenContractDocs.paths;
  foreignAffectedOverlaps =
    lib.intersectLists allOwnedFiles allForeignAffectedPaths;
  deferredPurgeOverlaps = lib.intersectLists
    allOwnedFiles
    realmHostPlan.crossWaveOwnership.deferredPurgeDocs.paths;
  expectedBundleDependencies =
    lib.filter (component: component != "bundle-integration") componentNames;
  componentPolicy = args:
    import ../eval-cases/realm-host-component-policy.nix args;
  componentGateSource = builtins.readFile
    (flakeRoot + "/tests/unit/nix/tools/realm-host-component-diff.sh");
  allowedSchemaDiff = componentPolicy {
    branch = "adr0045-w7-realm-schema";
    pathsJson = builtins.toJSON [ "nixos-modules/options.nix" ];
  };
  deniedCrossComponentDiff = componentPolicy {
    branch = "adr0045-w7-realm-schema";
    pathsJson = builtins.toJSON [ "nixos-modules/index.nix" ];
  };
  blockedProviderRegistryDiff = componentPolicy {
    branch = "adr0045-w7-provider-registry-composition";
    pathsJson =
      builtins.toJSON [ "nixos-modules/provider-registry-v2-json.nix" ];
  };
  blockedDeletionContractDiff = componentPolicy {
    branch = "adr0045-w7-workload-processes";
    pathsJson = builtins.toJSON [
      "packages/d2b-contract-tests/tests/policy_misc.rs"
    ];
  };
  blockedFixtureDiff = componentPolicy {
    branch = "adr0045-w7-realm-devices";
    pathsJson =
      builtins.toJSON [ "tests/fixtures/runner-shape-swtpm.snap" ];
  };
  blockedAllocatorDocDiff = componentPolicy {
    branch = "adr0045-w7-allocator-emission";
    pathsJson =
      builtins.toJSON [ "docs/reference/local-root-allocator.md" ];
  };
  blockedBundleIntegrationDiff = componentPolicy {
    branch = "adr0045-w7-bundle-integration";
    pathsJson = builtins.toJSON [ "flake.nix" ];
  };
  forbiddenOwnedFiles = lib.filter
    (path:
      lib.any
        (forbidden:
          path == forbidden
          || (lib.hasSuffix "/" forbidden && lib.hasPrefix forbidden path))
        realmHostPlan.forbiddenEdits)
    allOwnedFiles;
in
{
  "realms/valid-home-dev-work-keeps-env-substrate-active" = {
    expr = {
      assertionsPass = lib.all (a: a.assertion) cfg.assertions;
      enabledEnvNames = cfg.d2b._index.enabledEnvNames;
      netVmByEnv = cfg.d2b._index.netVmByEnv;
      workloadNamesByEnv = cfg.d2b._index.workloadNamesByEnv;
    };
    expected = {
      assertionsPass = true;
      enabledEnvNames = [ "dev" "home" "work" ];
      netVmByEnv = {
        dev = "sys-dev-net";
        home = "sys-home-net";
        work = "sys-work-net";
      };
      workloadNamesByEnv = {
        dev = [ "devbox" ];
        home = [ "homebox" ];
        work = [ "corp" ];
      };
    };
  };

  "realms/index-normalizes-enabled-disabled-and-derived-paths" = {
    expr = {
      names = realms.names;
      enabledNames = realms.enabledNames;
      archiveInDeclared = realms.byId.archive.enabled;
      archiveInEnabled = builtins.hasAttr "archive" realms.enabledById;
      dev = {
        inherit (realms.byPath."dev.home") realmName id path pathParts parentPath parentId placement enabled;
        network = realms.byPath."dev.home".network;
      };
      home = {
        allowedUsers = realms.byPath.home.allowedUsers;
        allowedGroups = realms.byPath.home.allowedGroups;
        paths = realms.byPath.home.paths;
        controller = {
          controllerId = realms.byPath.home.controller.controllerId;
          runtimeState = realms.byPath.home.controller.runtimeState;
          daemon = {
            inherit (realms.byPath.home.controller.daemon)
              serviceName
              configPath
              stateLockPath
              locksDir
              socketActivated
              materializedService
              ;
            userShape = builtins.substring 0 5 realms.byPath.home.controller.daemon.user == "d2br-";
            daemonPrincipalIsShared =
              realms.byPath.home.controller.daemon.user
              == realms.byPath.home.controller.daemon.group
              && realms.byPath.home.controller.daemon.user
              == realms.byPath.home.controller.daemon.publicSocketGroup;
          };
          broker = {
            inherit (realms.byPath.home.controller.broker)
              socketUnitName
              serviceUnitName
              materializedSocket
              materializedService
              ;
          };
        };
      };
      work = {
        inherit (realms.byPath."work.home") placement placementProvider;
        providerKeys = realms.byPath."work.home".providerKeys;
        enabledProviderKeys = realms.byPath."work.home".enabledProviderKeys;
        provider = realms.byPath."work.home".providers.aca;
        relay = realms.byPath."work.home".relay;
      };
      byEnv = realms.byEnv;
      bridges = {
        dev = cfg.d2b._index.envMeta.dev.lanBridge;
        home = cfg.d2b._index.envMeta.home.lanBridge;
        work = cfg.d2b._index.envMeta.work.lanBridge;
      };
    };

    expected = {
      names = [ "archive" "dev" "home" "work" ];
      enabledNames = [ "dev" "home" "work" ];
      archiveInDeclared = false;
      archiveInEnabled = false;
      dev = {
        realmName = "dev";
        id = "dev";
        path = "dev.home";
        pathParts = [ "dev" "home" ];
        parentPath = "home";
        parentId = "home";
        placement = "host-local";
        enabled = true;
        network = {
          env = "dev";
          envNames = [ "dev" "work" ];
          declaredEnvNames = [ "dev" "work" ];
          enabledEnvNames = [ "dev" "work" ];
          missingEnvNames = [ ];
          mode = "inherit-env";
          cidrRefs = [ "dev" "lab" ];
        };
      };

      home = {
        allowedUsers = [ "alice" ];
        allowedGroups = [ "realm-home" ];
        paths = {
          stateDir = "/var/lib/d2b/realms/home";
          auditDir = "/var/lib/d2b/audit/realms/home";
          runDir = "/run/d2b/realms/home";
          publicSocket = "/run/d2b/realms/home/public.sock";
          brokerSocket = "/run/d2b/realms/home/broker.sock";
        };
        controller = {
          controllerId = "realm-${realmHash "home"}";
          runtimeState = "metadata-only";
          daemon = {
            serviceName = "${homeUnitPrefix}-daemon.service";
            configPath = "/etc/d2b/realms/home/daemon-config.json";
            stateLockPath = "/run/d2b/realms/home/daemon.lock";
            locksDir = "/run/d2b/realms/home/locks";
            socketActivated = false;
            materializedService = true;
            userShape = true;
            daemonPrincipalIsShared = false;
          };
          broker = {
            socketUnitName = "${homeUnitPrefix}-priv-broker.socket";
            serviceUnitName = "${homeUnitPrefix}-priv-broker.service";
            materializedSocket = true;
            materializedService = true;
          };
        };
      };
      work = {
        placement = "gateway-vm";
        placementProvider = null;
        providerKeys = [ "aca" ];
        enabledProviderKeys = [ "aca" ];
        provider = {
          providerName = "aca";
          id = "aca";
          enabled = true;
          kind = "aca";
          placement = "provider-agent";
          capabilityRefs = [ "aca" "relay" ];
          configRef = "work-aca-non-secret";
        };
        relay = {
          enable = true;
          mode = "static";
          endpoints = [ "relns-a.example.invalid" "relns-b.example.invalid" ];
          credentialRef = "work-relay-credential";
        };
      };
      byEnv = {
        dev = {
          realmNames = [ "dev" ];
          realmIds = [ "dev" ];
          realmPaths = [ "dev.home" ];
        };
        home = {
          realmNames = [ "home" ];
          realmIds = [ "home" ];
          realmPaths = [ "home" ];
        };
        work = {
          realmNames = [ "dev" "work" ];
          realmIds = [ "dev" "work" ];
          realmPaths = [ "dev.home" "work.home" ];
        };
      };
      bridges = {
        dev = "br-dev-lan";
        home = "br-home-lan";
        work = "br-work-lan";
      };
    };
  };

  "realms/allocator-artifact-roots-enabled-realm-index" = {
    expr =
      let
        data = cfg.d2b._bundle.allocatorJson.data;
        firstRequest = resourceId:
          lib.findFirst (row: row.resourceId == resourceId) null data.resourceRequests;
        namespaceBoundaryRequests =
          lib.filter (row: row.kind == "namespace-boundary") data.resourceRequests;
        devNamespaceBoundaryRequests =
          lib.filter (row: row.resourceId == "realm-dev-netns") namespaceBoundaryRequests;
        namespaceBoundaryResourceIds = map (row: row.resourceId) namespaceBoundaryRequests;
        envRow = realmPath: envName:
          lib.findFirst
            (row: row.realmPath == realmPath && row.envName == envName)
            null
            data.envBridge;
      in
      {
        installFileName = cfg.d2b._bundle.allocatorJson.installFileName;
        classification = cfg.d2b._bundle.allocatorJson.classification;
        sensitivity = cfg.d2b._bundle.allocatorJson.sensitivity;
        mode = cfg.d2b._bundle.allocatorJson.mode;
        user = cfg.d2b._bundle.allocatorJson.user;
        group = cfg.d2b._bundle.allocatorJson.group;
        bundleAllocatorPath = cfg.d2b._bundle.bundle.data.allocatorPath;
        storageCoversAllocator =
          lib.any (path: path.pathTemplate == "/etc/d2b/allocator.json")
            cfg.d2b._bundle.storageJson.data.paths;
        allocatorRuntimeServices =
          lib.filter
            (name: lib.hasInfix "allocator" name || lib.hasInfix "local-root" name)
            (lib.attrNames cfg.systemd.services);
        allocator = {
          inherit (data.allocator) enabled runtimeState rootSocket stateDir leaseLedger auditDir runtime;
        };
        realmPaths = map (row: row.realmPath) data.realms;
        devNetns = firstRequest "realm-dev-netns";
        devNetnsCount = lib.length devNamespaceBoundaryRequests;
        namespaceBoundaryResourceIdsUnique =
          lib.length namespaceBoundaryResourceIds == lib.length (lib.unique namespaceBoundaryResourceIds);
        homeBridge = firstRequest "env-home-bridge";
        workProvider = builtins.head data.providerPlacements;
        devWorkBridge = envRow "dev.home" "work";
        inherit (data) invariants;
      };
    expected = {
      installFileName = "allocator.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
      mode = "0640";
      user = "root";
      group = "d2bd";
      bundleAllocatorPath = "/etc/d2b/allocator.json";
      storageCoversAllocator = true;
      allocatorRuntimeServices = [ ];
      allocator = {
        enabled = true;
        runtimeState = "metadata-only";
        rootSocket = "/run/d2b/allocator/local-root.sock";
        stateDir = "/var/lib/d2b/allocator";
        leaseLedger = "/var/lib/d2b/allocator/leases.jsonl";
        auditDir = "/var/lib/d2b/allocator/audit";
        runtime = {
          spawnsService = false;
          socketActivated = false;
          serviceName = null;
        };
      };
      realmPaths = [ "dev.home" "home" "work.home" ];
      devNetns = {
        realmPath = "dev.home";
        resourceId = "realm-dev-netns";
        kind = "namespace-boundary";
        share = "exclusive";
        acquisitionOrder = {
          phase = 31;
          ordinal = 0;
        };
        source = {
          kind = "realm-network";
          refName = "dev";
        };
      };
      devNetnsCount = 1;
      namespaceBoundaryResourceIdsUnique = true;
      homeBridge = {
        realmPath = "home";
        resourceId = "env-home-bridge";
        kind = "bridge";
        share = "shared-partition";
        acquisitionOrder = {
          phase = 30;
          ordinal = 0;
        };
        source = {
          kind = "env-bridge";
          refName = "home";
        };
      };
      workProvider = {
        realmPath = "work.home";
        providerName = "aca";
        providerId = "aca";
        enabled = true;
        kind = "aca";
        placement = "provider-agent";
        capabilityRefs = [ "aca" "relay" ];
        configRef = "work-aca-non-secret";
      };
      devWorkBridge = {
        realmPath = "dev.home";
        envName = "work";
        declared = true;
        enabled = true;
        mode = "inherit-env";
        netVm = "sys-work-net";
        lanBridge = "br-work-lan";
        uplinkBridge = "br-work-up";
      };
      invariants = {
        noRuntimeAllocatorService = true;
        preservesEnvRuntimeSourceOfTruth = true;
        privateMetadataOnly = true;
      };
    };
  };

  "realms/controller-config-artifact-materializes-host-local-units" = {
    expr =
      let
        data = cfg.d2b._bundle.realmControllersJson.data;
        controllerByPath = path:
          lib.findFirst (row: row.realmPath == path) null data.controllers;
        home = controllerByPath "home";
        dev = controllerByPath "dev.home";
        work = controllerByPath "work.home";
        realmServiceNames =
          lib.filter
            (name: lib.hasPrefix "d2b-realm-" name)
            (lib.attrNames cfg.systemd.services);
        realmSocketNames =
          lib.filter
            (name: lib.hasPrefix "d2b-realm-" name)
            (lib.attrNames cfg.systemd.sockets);
        allRealmUnitNames = realmServiceNames ++ realmSocketNames;
        homeDaemonServiceName = lib.removeSuffix ".service" home.daemon.serviceName;
        homeBrokerServiceName = lib.removeSuffix ".service" home.broker.serviceUnitName;
        homeBrokerSocketName = lib.removeSuffix ".socket" home.broker.socketUnitName;
        devDaemonServiceName = lib.removeSuffix ".service" dev.daemon.serviceName;
        workDaemonServiceName = lib.removeSuffix ".service" work.daemon.serviceName;
        homeDaemonUnit = cfg.systemd.services.${homeDaemonServiceName};
        devDaemonUnit = cfg.systemd.services.${devDaemonServiceName};
        homeBrokerSocket = cfg.systemd.sockets.${homeBrokerSocketName};
        homeBrokerService = cfg.systemd.services.${homeBrokerServiceName};
        homeResourceRefs = home.allocator.resourceRequestRefs;
      in
      {
        installFileName = cfg.d2b._bundle.realmControllersJson.installFileName;
        classification = cfg.d2b._bundle.realmControllersJson.classification;
        sensitivity = cfg.d2b._bundle.realmControllersJson.sensitivity;
        mode = cfg.d2b._bundle.realmControllersJson.mode;
        user = cfg.d2b._bundle.realmControllersJson.user;
        group = cfg.d2b._bundle.realmControllersJson.group;
        bundleRealmControllersPath = cfg.d2b._bundle.bundle.data.realmControllersPath;
        bundleRealmIdentityPath = cfg.d2b._bundle.bundle.data.realmIdentityPath;
        realmIdentityArtifact = {
          installFileName = cfg.d2b._bundle.realmIdentityJson.installFileName;
          classification = cfg.d2b._bundle.realmIdentityJson.classification;
          sensitivity = cfg.d2b._bundle.realmIdentityJson.sensitivity;
          mode = cfg.d2b._bundle.realmIdentityJson.mode;
          user = cfg.d2b._bundle.realmIdentityJson.user;
          group = cfg.d2b._bundle.realmIdentityJson.group;
        };
        storageCoversRealmControllers =
          lib.any (path: path.pathTemplate == "/etc/d2b/realm-controllers.json")
            cfg.d2b._bundle.storageJson.data.paths;
        storageCoversRealmIdentity =
          lib.any (path: path.pathTemplate == "/etc/d2b/realm-identity.json")
            cfg.d2b._bundle.storageJson.data.paths;
        realmSystemdUnits = {
          serviceCount = lib.length realmServiceNames;
          socketCount = lib.length realmSocketNames;
          hasHomeDaemon = builtins.elem homeDaemonServiceName realmServiceNames;
          hasDevDaemon = builtins.elem devDaemonServiceName realmServiceNames;
          hasHomeBrokerService = builtins.elem homeBrokerServiceName realmServiceNames;
          hasHomeBrokerSocket = builtins.elem homeBrokerSocketName realmSocketNames;
          rawRealmIdsAbsent =
            !lib.any
              (name:
                lib.hasInfix "home-daemon" name
                || lib.hasInfix "dev-daemon" name
                || lib.hasInfix "work-daemon" name)
              allRealmUnitNames;
          gatewayRealmUnitAbsent = !(builtins.hasAttr workDaemonServiceName cfg.systemd.services);
        };
        accessMaterialization = {
          socketGroup = home.daemon.publicSocketGroup;
          daemonGroup = home.daemon.group;
          socketGroupIsDistinct = home.daemon.publicSocketGroup != home.daemon.group;
          aliceExtraGroups = lib.sort lib.lessThan cfg.users.users.alice.extraGroups;
          aliceInD2bLifecycleGroup =
            builtins.elem "d2b" cfg.users.users.alice.extraGroups;
          aliceInRealmSocketGroup =
            builtins.elem home.daemon.publicSocketGroup cfg.users.users.alice.extraGroups;
          daemonUserExists = builtins.hasAttr home.daemon.user cfg.users.users;
          daemonUserSupplementarySocketGroup =
            builtins.elem home.daemon.publicSocketGroup cfg.users.users.${home.daemon.user}.extraGroups;
        };
        units = {
          homeDaemon = {
            wantedBy = homeDaemonUnit.wantedBy;
            wantsRootBrokerSocket = builtins.elem "d2b-priv-broker.socket" homeDaemonUnit.wants;
            afterRootBrokerSocket = builtins.elem "d2b-priv-broker.socket" homeDaemonUnit.after;
            afterRootBrokerService = builtins.elem "d2b-priv-broker.service" homeDaemonUnit.after;
            wantsBrokerSocket = builtins.elem home.broker.socketUnitName homeDaemonUnit.wants;
            afterBrokerSocket = builtins.elem home.broker.socketUnitName homeDaemonUnit.after;
            afterBrokerService = builtins.elem home.broker.serviceUnitName homeDaemonUnit.after;
            inherit (homeDaemonUnit.serviceConfig) User Group ExecStart SupplementaryGroups Slice;
            execStartHasDaemonStateDir =
              lib.hasInfix "--daemon-state-dir /var/lib/d2b/realms/home" homeDaemonUnit.serviceConfig.ExecStart;
          };
          devDaemonAfterHome =
            builtins.elem home.daemon.serviceName devDaemonUnit.after;
          homeBrokerSocket = {
            inherit (homeBrokerSocket.socketConfig) ListenSequentialPacket SocketGroup SocketMode;
          };
          homeBroker = {
            requiresBrokerSocket = builtins.elem home.broker.socketUnitName homeBrokerService.requires;
            afterBrokerSocket = builtins.elem home.broker.socketUnitName homeBrokerService.after;
            inherit (homeBrokerService.serviceConfig) Group Slice;
            execStartHasAuditDir =
              lib.hasInfix "--audit-dir /var/lib/d2b/audit/realms/home" homeBrokerService.serviceConfig.ExecStart;
            execStartHasRealmControllersPath =
              lib.hasInfix "--realm-controllers-path /etc/d2b/realm-controllers.json" homeBrokerService.serviceConfig.ExecStart;
            execStartHasStateDir =
              lib.hasInfix "--state-dir /var/lib/d2b/realms/home" homeBrokerService.serviceConfig.ExecStart;
            execStartHasD2bdUid =
              lib.hasInfix "--d2bd-uid " homeBrokerService.serviceConfig.ExecStart;
            execStartHasD2bdGid =
              lib.hasInfix "--d2bd-gid " homeBrokerService.serviceConfig.ExecStart;
            noGlobalSetEnvironment =
              !(lib.hasInfix "set-environment" (homeBrokerService.serviceConfig.ExecStartPre or ""))
              && !(lib.hasInfix "systemctl set-environment" homeBrokerService.serviceConfig.ExecStart);
          };
        };
        runtimeState = data.runtimeState;
        controllerPaths = map (row: row.realmPath) data.controllers;
        home = {
          inherit (home) realmName realmId realmPath placement providerPlacement sockets access;
          paths = home.paths // {
            auditDirOutsideStateDir =
              !(lib.hasPrefix "${home.paths.stateDir}/" home.paths.auditDir);
          };
          daemon = {
            inherit (home.daemon)
              serviceName
              configPath
              stateLockPath
              locksDir
              socketActivated
              materializedService
              ;
            principalShape = builtins.substring 0 5 home.daemon.user == "d2br-";
            socketGroupIsDaemonGroup =
              home.daemon.user == home.daemon.group
              && home.daemon.user == home.daemon.publicSocketGroup;
          };
          broker = home.broker;
          allocator = {
            inherit (home.allocator) kind configPath rootSocket;
            resourceRefCount = lib.length homeResourceRefs;
            hasStateRef = builtins.elem "realm-home-state" homeResourceRefs;
            hasPublicSocketRef = builtins.elem "realm-home-public-socket" homeResourceRefs;
            hasBrokerSocketRef = builtins.elem "realm-home-broker-socket" homeResourceRefs;
          };
          localRuntime = {
            runtimeState = home.localRuntime.runtimeState;
            providerIds = map (provider: provider.provider.id) home.localRuntime.providers;
            workloads = map
              (workload: {
                inherit (workload) workloadId vmName env paths;
                runtimeKind = workload.runtime.kind;
                providerId = workload.runtime.provider.id;
                driver = workload.runtime.provider.driver;
                lifecycleStart = workload.runtime.operationCapabilities.lifecycle.start;
                guestExec = workload.runtime.operationCapabilities.guest.exec;
                storageStoreSync = workload.runtime.operationCapabilities.storage.storeSync;
                serviceIds = map (service: service.id) workload.runtime.services;
              })
              home.localRuntime.workloads;
            inherit (home.localRuntime) invariants;
          };
        };
        workLocalRuntime = work.localRuntime;
        workProviderCount = lib.length work.providers;
        workProvider = builtins.head work.providers;
        providerPlacement = validProviderController.providerPlacement;
        inherit (data) invariants;
      };
    expected = {
      installFileName = "realm-controllers.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
      mode = "0640";
      user = "root";
      group = "d2bd";
      bundleRealmControllersPath = "/etc/d2b/realm-controllers.json";
      bundleRealmIdentityPath = "/etc/d2b/realm-identity.json";
      realmIdentityArtifact = {
        installFileName = "realm-identity.json";
        classification = "contractPrivateNonSecret";
        sensitivity = "nonSecret";
        mode = "0640";
        user = "root";
        group = "d2bd";
      };
      storageCoversRealmControllers = true;
      storageCoversRealmIdentity = true;
      realmSystemdUnits = {
        serviceCount = 3;
        socketCount = 1;
        hasHomeDaemon = true;
        hasDevDaemon = true;
        hasHomeBrokerService = true;
        hasHomeBrokerSocket = true;
        rawRealmIdsAbsent = true;
        gatewayRealmUnitAbsent = true;
      };
      accessMaterialization = {
        socketGroup = "d2bra-${realmHash "home"}";
        daemonGroup = "d2br-${realmHash "home"}";
        socketGroupIsDistinct = true;
        aliceExtraGroups = [ "d2b" "d2bra-${realmHash "home"}" ];
        aliceInD2bLifecycleGroup = true;
        aliceInRealmSocketGroup = true;
        daemonUserExists = true;
        daemonUserSupplementarySocketGroup = true;
      };
      units = {
        homeDaemon = {
          wantedBy = [ "multi-user.target" ];
          wantsRootBrokerSocket = true;
          afterRootBrokerSocket = true;
          afterRootBrokerService = true;
          wantsBrokerSocket = true;
          afterBrokerSocket = true;
          afterBrokerService = true;
          User = "d2br-${realmHash "home"}";
          Group = "d2br-${realmHash "home"}";
          ExecStart = "${cfg.d2b._hostToolPackages.d2bd}/bin/d2bd serve --config /etc/d2b/realms/home/daemon-config.json --daemon-state-dir /var/lib/d2b/realms/home";
          execStartHasDaemonStateDir = true;
          SupplementaryGroups = [ "d2bra-${realmHash "home"}" "d2bd" ];
          Slice = "d2b.slice";
        };
        devDaemonAfterHome = true;
        homeBrokerSocket = {
          ListenSequentialPacket = "/run/d2b/realms/home/broker.sock";
          SocketGroup = "d2br-${realmHash "home"}";
          SocketMode = "0660";
        };
        homeBroker = {
          requiresBrokerSocket = true;
          afterBrokerSocket = true;
          Group = "d2br-${realmHash "home"}";
          execStartHasAuditDir = true;
          execStartHasRealmControllersPath = true;
          execStartHasStateDir = true;
          execStartHasD2bdUid = true;
          execStartHasD2bdGid = true;
          noGlobalSetEnvironment = true;
          Slice = "d2b.slice";
        };
      };
      runtimeState = "metadata-only";
      controllerPaths = [ "dev.home" "home" "work.home" ];
      home = {
        realmName = "home";
        realmId = "home";
        realmPath = "home";
        placement = "host-local";
        providerPlacement = null;
        paths = {
          runDir = "/run/d2b/realms/home";
          stateDir = "/var/lib/d2b/realms/home";
          auditDir = "/var/lib/d2b/audit/realms/home";
          auditDirOutsideStateDir = true;
        };
        sockets = {
          publicSocketPath = "/run/d2b/realms/home/public.sock";
          brokerSocketPath = "/run/d2b/realms/home/broker.sock";
        };
        access = {
          allowedUsers = [ "alice" ];
          allowedGroups = [ "realm-home" ];
          inheritedAdminUsers = [ ];
        };
        daemon = {
        serviceName = "${homeUnitPrefix}-daemon.service";
          configPath = "/etc/d2b/realms/home/daemon-config.json";
          stateLockPath = "/run/d2b/realms/home/daemon.lock";
          locksDir = "/run/d2b/realms/home/locks";
          socketActivated = false;
          materializedService = true;
          principalShape = true;
          socketGroupIsDaemonGroup = false;
        };
        broker = {
          enabled = true;
          hostMutation = true;
          user = "root";
          group = realms.byPath.home.controller.broker.group;
          socketPath = "/run/d2b/realms/home/broker.sock";
          socketUnitName = "${homeUnitPrefix}-priv-broker.socket";
          serviceUnitName = "${homeUnitPrefix}-priv-broker.service";
          auditDir = "/var/lib/d2b/audit/realms/home";
          materializedSocket = true;
          materializedService = true;
        };
        allocator = {
          kind = "local-root-metadata";
          configPath = "/etc/d2b/allocator.json";
          rootSocket = "/run/d2b/allocator/local-root.sock";
          resourceRefCount = 8;
          hasStateRef = true;
          hasPublicSocketRef = true;
          hasBrokerSocketRef = true;
        };
        localRuntime = {
          runtimeState = "metadata-only";
          providerIds = [ "local-cloud-hypervisor" ];
          workloads = [
            {
              workloadId = "homebox";
              vmName = "homebox";
              env = "home";
              paths = {
                stateDir = "/var/lib/d2b/vms/homebox";
                runDir = "/run/d2b/vms/homebox";
                storeView = "/var/lib/d2b/vms/homebox/store-view";
                guestControlDir = "/run/d2b/vms/homebox/guest-control";
              };
              runtimeKind = "nixos";
              providerId = "local-cloud-hypervisor";
              driver = "cloud-hypervisor";
              lifecycleStart = true;
              guestExec = true;
              storageStoreSync = true;
              serviceIds = [
                "host-reconcile"
                "store-virtiofs-preflight"
                "virtiofsd"
                "cloud-hypervisor"
                "guest-control-health"
                "swtpm"
                "gpu"
                "audio"
                "video"
                "usbip"
              ];
            }
          ];
          invariants = {
            metadataOnly = true;
            existingGlobalVmPathsPreserved = true;
            noStateMigrationDuringActivation = true;
            brokerEffectsRemainRealmDelegated = true;
          };
        };
      };
      workLocalRuntime = null;
      workProviderCount = 1;
      workProvider = {
        providerName = "aca";
        providerId = "aca";
        enabled = true;
        kind = "aca";
        placement = "provider-agent";
        capabilityRefs = [ "aca" "relay" ];
        configRef = "work-aca-non-secret";
      };
      providerPlacement = {
        providerName = "aca";
        providerId = "aca";
        kind = "aca";
        providerSpecificPlacement = null;
      };
      invariants = {
        metadataOnly = true;
        noSystemdUnitsMaterialized = false;
        preservesGlobalDaemonBehavior = true;
        preservesDirectUnixSocketSemantics = true;
      };
    };
  };

  "realms/identity-config-artifact-is-metadata-only" = {
    expr =
      let
        data = cfg.d2b._bundle.realmIdentityJson.data;
        work = builtins.head data.realms;
      in
      {
        runtimeState = data.runtimeState;
        realmCount = lib.length data.realms;
        work = work;
        invariants = data.invariants;
        renderedTextHasNoMaterial =
          !(lib.hasInfix "privateKey" cfg.d2b._bundle.realmIdentityJson.jsonText)
          && !(lib.hasInfix "publicKeyPem" cfg.d2b._bundle.realmIdentityJson.jsonText)
          && !(lib.hasInfix "credentialMaterial" cfg.d2b._bundle.realmIdentityJson.jsonText);
      };
    expected = {
      runtimeState = "metadata-only";
      realmCount = 1;
      work = {
        realm = [ "work" "home" ];
        realmIdentityRef = "idref-work";
        realmIdentityFingerprint = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        controllerCredentialRef = "cgref-work";
        controllerCredentialFingerprint = "sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
        trustBundleRef = "trust-work";
        enrollmentRef = "enroll-work";
        rotationPolicyRef = "rotate-work";
      };
      invariants = {
        metadataOnly = true;
        noSecretMaterial = true;
        preservesRuntimeBehavior = true;
      };
      renderedTextHasNoMaterial = true;
    };
  };

  "realms/rejects-secret-shaped-identity-key-refs" = {
    expr = hasMessage [
      "identity key refs must be opaque, non-secret locators"
      "work.realmIdentityRef"
      "work.controllerKeyRef"
      "work.trustBundleRef"
      "work.enrollmentRef"
      "work.rotationPolicyRef"
    ] secretIdentityRefMessages;
    expected = true;
  };

  "realms/host-local-units-users-groups-tmpfiles-and-no-per-vm-units" = {
    expr =
      let
        data = cfg.d2b._bundle.realmControllersJson.data;
        controllerByPath = path:
          lib.findFirst (row: row.realmPath == path) null data.controllers;
        home = controllerByPath "home";
        dev = controllerByPath "dev.home";
        homeDaemonServiceName = lib.removeSuffix ".service" home.daemon.serviceName;
        devDaemonServiceName = lib.removeSuffix ".service" dev.daemon.serviceName;
        homeBrokerSocketName = lib.removeSuffix ".socket" home.broker.socketUnitName;
        homeDaemonUnit = cfg.systemd.services.${homeDaemonServiceName};
        devDaemonUnit = cfg.systemd.services.${devDaemonServiceName};
        homeBrokerSocket = cfg.systemd.sockets.${homeBrokerSocketName};
        homeDaemonUser = cfg.users.users.${home.daemon.user};
        homeDaemonEtc = cfg.environment.etc."d2b/realms/home/daemon-config.json";
        homeDaemonConfig = builtins.fromJSON homeDaemonEtc.text;
        tmpfiles = cfg.systemd.tmpfiles.rules;
        disabledUnitPrefix = realmUnitPrefix "archive";
        disabledPrincipal = "d2br-${realmHash "archive"}";
        disabledAccessGroup = "d2bra-${realmHash "archive"}";
        unitNames = (lib.attrNames cfg.systemd.services) ++ (lib.attrNames cfg.systemd.sockets);
        perVmUnitNames =
          lib.filter
            (name:
              lib.any
                (vmName: lib.hasInfix vmName name)
                [ "homebox" "devbox" "corp" "sys-home-net" "sys-dev-net" "sys-work-net" ])
            unitNames;
      in
      {
        groups = {
          daemonGroupDeclared = builtins.hasAttr home.daemon.group cfg.users.groups;
          accessGroupDeclared = builtins.hasAttr home.daemon.publicSocketGroup cfg.users.groups;
          daemonAndAccessGroupsDistinct = home.daemon.group != home.daemon.publicSocketGroup;
          disabledDaemonGroupAbsent = !(builtins.hasAttr disabledPrincipal cfg.users.groups);
          disabledAccessGroupAbsent = !(builtins.hasAttr disabledAccessGroup cfg.users.groups);
        };
        users = {
          allowedUserInAccessGroup =
            builtins.elem home.daemon.publicSocketGroup cfg.users.users.alice.extraGroups;
          daemonUser = {
            inherit (homeDaemonUser) isSystemUser group description extraGroups;
          };
          disabledDaemonUserAbsent = !(builtins.hasAttr disabledPrincipal cfg.users.users);
        };
        tmpfiles = {
          stateDir =
            builtins.elem
              "d /var/lib/d2b/realms/home 0750 ${home.daemon.user} ${home.daemon.group} -"
              tmpfiles;
          auditDir =
            builtins.elem
              "d /var/lib/d2b/audit/realms/home 0750 root ${home.daemon.group} -"
              tmpfiles;
          auditParentDir =
            builtins.elem
              "d /var/lib/d2b/audit/realms 0750 root d2bd -"
              tmpfiles;
          runDir =
            builtins.elem
              "d /run/d2b/realms/home 1770 root ${home.daemon.publicSocketGroup} -"
              tmpfiles;
          runDirReset =
            builtins.elem
              "z /run/d2b/realms/home 1770 root ${home.daemon.publicSocketGroup} -"
              tmpfiles;
          runDirGroupAccessAcl =
            builtins.elem
              "a+ /run/d2b/realms/home - - - - g::r-x"
              tmpfiles;
          daemonRunAcl =
            builtins.elem
              "a+ /run/d2b/realms/home - - - - u:${home.daemon.user}:rwx"
              tmpfiles;
          stateLock =
            builtins.elem
              "f /run/d2b/realms/home/daemon.lock 0640 ${home.daemon.user} ${home.daemon.group} -"
              tmpfiles;
          locksDir =
            builtins.elem
              "d /run/d2b/realms/home/locks 0700 ${home.daemon.user} ${home.daemon.group} -"
              tmpfiles;
          etcD2bTraverseAcl =
            builtins.elem
              "a+ /etc/d2b - - - - u:${home.daemon.user}:--x"
              tmpfiles;
          realmControllersReadAcl =
            builtins.elem
              "a+ /etc/d2b/realm-controllers.json - - - - u:${home.daemon.user}:r--"
              tmpfiles;
          realmIdentityReadAcl =
            builtins.elem
              "a+ /etc/d2b/realm-identity.json - - - - u:${home.daemon.user}:r--"
              tmpfiles;
          etcRealmsTraverseAcl =
            builtins.elem
              "a+ /etc/d2b/realms - - - - u:${home.daemon.user}:--x"
              tmpfiles;
          etcRealmConfigDirTraverseAcl =
            builtins.elem
              "a+ /etc/d2b/realms/home - - - - u:${home.daemon.user}:--x"
              tmpfiles;
          runD2bTraverseAcl =
            builtins.elem
              "a+ /run/d2b - - - - u:${home.daemon.user}:--x"
              tmpfiles;
          runRealmsTraverseAcl =
            builtins.elem
              "a+ /run/d2b/realms - - - - u:${home.daemon.user}:--x"
              tmpfiles;
        };
        daemonConfig = {
          inherit (homeDaemonEtc) mode user group;
          publicSocketPath = homeDaemonConfig.publicSocketPath;
          brokerSocketPath = homeDaemonConfig.brokerSocketPath;
          daemonUser = homeDaemonConfig.daemonUser;
          daemonGroup = homeDaemonConfig.daemonGroup;
          publicSocketGroup = homeDaemonConfig.publicSocketGroup;
          launcherUsers = homeDaemonConfig.launcherUsers;
          realmControllersConfigPath = homeDaemonConfig.realmControllersConfigPath;
          realmIdentityConfigPath = homeDaemonConfig.realmIdentityConfigPath;
          artifacts = homeDaemonConfig.artifacts;
        };
        unitOrdering = {
          childAfterParent = builtins.elem home.daemon.serviceName devDaemonUnit.after;
          parentDoesNotAfterChild = !(builtins.elem dev.daemon.serviceName homeDaemonUnit.after);
          parentAfterRootBrokerSocket = builtins.elem "d2b-priv-broker.socket" homeDaemonUnit.after;
          parentAfterRootBrokerService = builtins.elem "d2b-priv-broker.service" homeDaemonUnit.after;
        };
        socketAccess = {
          inherit (homeBrokerSocket.socketConfig) ListenSequentialPacket SocketGroup SocketMode;
        };
        disabledRealm = {
          unitsAbsent = !lib.any (name: lib.hasPrefix disabledUnitPrefix name) unitNames;
          controllersAbsent =
            !lib.any (row: row.realmPath == "archive") data.controllers;
        };
        noPerVmSystemdUnits = perVmUnitNames;
      };
    expected = {
      groups = {
        daemonGroupDeclared = true;
        accessGroupDeclared = true;
        daemonAndAccessGroupsDistinct = true;
        disabledDaemonGroupAbsent = true;
        disabledAccessGroupAbsent = true;
      };
      users = {
        allowedUserInAccessGroup = true;
        daemonUser = {
          isSystemUser = true;
          group = "d2br-${realmHash "home"}";
          description = "d2b realm daemon user for home";
          extraGroups = [ "d2bra-${realmHash "home"}" "d2bd" ];
        };
        disabledDaemonUserAbsent = true;
      };
      tmpfiles = {
        stateDir = true;
        auditDir = true;
        auditParentDir = true;
        runDir = true;
        runDirReset = true;
        runDirGroupAccessAcl = true;
        daemonRunAcl = true;
        stateLock = true;
        locksDir = true;
        etcD2bTraverseAcl = true;
        realmControllersReadAcl = true;
        realmIdentityReadAcl = true;
        etcRealmsTraverseAcl = true;
        etcRealmConfigDirTraverseAcl = true;
        runD2bTraverseAcl = true;
        runRealmsTraverseAcl = true;
      };
      daemonConfig = {
        mode = "0640";
        user = "root";
        group = "d2br-${realmHash "home"}";
        publicSocketPath = "/run/d2b/realms/home/public.sock";
        brokerSocketPath = "/run/d2b/realms/home/broker.sock";
        daemonUser = "d2br-${realmHash "home"}";
        daemonGroup = "d2br-${realmHash "home"}";
        publicSocketGroup = "d2bra-${realmHash "home"}";
        launcherUsers = [ "alice" ];
        realmControllersConfigPath = "/etc/d2b/realm-controllers.json";
        realmIdentityConfigPath = "/etc/d2b/realm-identity.json";
        artifacts = {
          publicManifestPath = "/run/current-system/sw/share/d2b/vms.json";
          bundlePath = "/etc/d2b/bundle.json";
          hostPath = "/etc/d2b/host.json";
          processesPath = "/etc/d2b/processes.json";
          closuresDir = "/etc/d2b/closures";
        };
      };
      unitOrdering = {
        childAfterParent = true;
        parentDoesNotAfterChild = true;
        parentAfterRootBrokerSocket = true;
        parentAfterRootBrokerService = true;
      };
      socketAccess = {
        ListenSequentialPacket = "/run/d2b/realms/home/broker.sock";
        SocketGroup = "d2br-${realmHash "home"}";
        SocketMode = "0660";
      };
      disabledRealm = {
        unitsAbsent = true;
        controllersAbsent = true;
      };
      noPerVmSystemdUnits = [ ];
    };
  };

  "realms/rejects-missing-parent" = {
    expr = hasMessage [
      "enabled child realms must name an enabled parent realm"
      "child.missing -> missing"
    ] missingParentMessages;
    expected = true;
  };

  "realms/rejects-parent-cycle" = {
    expr = hasMessage [
      "enabled d2b.realms parent links must form an acyclic tree"
      "alpha -> beta -> alpha"
    ] parentCycleMessages;
    expected = true;
  };

  "realms/rejects-duplicate-id-and-path" = {
    expr = {
      duplicateId = hasMessage [
        "d2b.realms must use unique stable realm ids"
        "same"
      ] duplicateIdPathMessages;
      duplicatePath = hasMessage [
        "d2b.realms must use unique canonical realm paths"
        "same-path"
      ] duplicateIdPathMessages;
    };
    expected = {
      duplicateId = true;
      duplicatePath = true;
    };
  };

  "realms/rejects-duplicate-runtime-paths" = {
    expr = {
      stateDir = hasMessage [ "must not share stateDir paths" "/var/lib/d2b/realms/alpha" ] duplicateRuntimePathMessages;
      auditDir = hasMessage [ "must not share auditDir paths" "/var/lib/d2b/audit/realms/alpha" ] duplicateRuntimePathMessages;
      runDir = hasMessage [ "must not share runDir paths" "/run/d2b/realms/alpha" ] duplicateRuntimePathMessages;
      publicSocket = hasMessage [ "must not share publicSocket paths" "/run/d2b/realms/alpha/public.sock" ] duplicateRuntimePathMessages;
      brokerSocket = hasMessage [ "must not share brokerSocket paths" "/run/d2b/realms/alpha/broker.sock" ] duplicateRuntimePathMessages;
    };
    expected = {
      stateDir = true;
      auditDir = true;
      runDir = true;
      publicSocket = true;
      brokerSocket = true;
    };
  };

  "realms/rejects-overlong-unix-socket-paths" = {
    expr = {
      publicSocket = hasMessage [
        "paths.publicSocket must fit Linux AF_UNIX pathname"
        "at most 107 bytes"
        "work"
      ] overlongPublicSocketMessages;
      brokerSocket = hasMessage [
        "paths.brokerSocket must fit Linux AF_UNIX pathname"
        "at most 107 bytes"
        "work"
      ] overlongBrokerSocketMessages;
    };
    expected = {
      publicSocket = true;
      brokerSocket = true;
    };
  };

  "realms/rejects-missing-host-local-allowed-user" = {
    expr = hasMessage [
      "d2b.realms.home.allowedUsers contains \"missing-user\""
      "users.users.missing-user is declared"
    ] missingRealmAllowedUserMessages;
    expected = true;
  };

  "realms/requires-provider-for-provider-backed-placement" = {
    expr = {
      missingProvider = hasMessage [
        "provider-backed d2b.realms placements require"
        "placementProvider"
        "work (provider-controller)"
      ] missingPlacementProviderMessages;
      missingProviderSpecificProvider = hasMessage [
        "provider-backed d2b.realms placements require"
        "placementProvider"
        "work (provider-specific)"
      ] missingProviderSpecificPlacementProviderMessages;
      rejectsLocal = hasMessage [
        "placementProvider is valid only for provider-backed"
        "work (gateway-vm)"
      ] unexpectedPlacementProviderMessages;
      validProvider = lib.all (a: a.assertion) validProviderPlacementCfg.assertions;
      indexedProvider = validProviderPlacementCfg.d2b._index.realms.byPath.work.placementProvider;
    };
    expected = {
      missingProvider = true;
      missingProviderSpecificProvider = true;
      rejectsLocal = true;
      validProvider = true;
      indexedProvider = "aca";
    };
  };

  "realms/rejects-legacy-gateway-aca-surface-with-migration-guidance" = {
    expr = hasMessage [
      "legacy-surface-detected: d2b.gateways"
      "old gateway/ACA sandbox fields"
      "d2b.realms.work"
      "`d2b.envs` remains the current substrate"
    ] legacyGatewayMessages;
    expected = true;
  };

  "realms/examples-minimal-and-multi-env-still-eval" = {
    expr = {
      minimal = lib.all (a: a.assertion) minimalCfg.assertions;
      multiEnv = lib.all (a: a.assertion) multiEnvCfg.assertions;
    };
    expected = {
      minimal = true;
      multiEnv = true;
    };
  };

  # ── tombstone / migration advisory warnings ──────────────────────────────────

  # Realm linking to d2b.envs with mode=none and no workloads → nudge warning.
  "realms/tombstone-inherit-env-nudge-warning-fires" = {
    expr = {
      hasWarning = lib.any
        (w: lib.hasInfix "nudge-me" w && lib.hasInfix "migrate-d2b-v1-2-to-v2" w)
        inheritEnvNudgeWarnings;
      pointsAtEnvRef = lib.any
        (w: lib.hasInfix "d2b.envs.home" w)
        inheritEnvNudgeWarnings;
      mentionsWorkloadSurface = lib.any
        (w: lib.hasInfix "d2b.realms.nudge-me.workloads" w)
        inheritEnvNudgeWarnings;
    };
    expected = {
      hasWarning = true;
      pointsAtEnvRef = true;
      mentionsWorkloadSurface = true;
    };
  };

  # Realm with workloads declared — inherit-env nudge must NOT fire.
  "realms/tombstone-no-nudge-when-workloads-declared" = {
    expr = lib.any
      (w: lib.hasInfix "migrate-d2b-v1-2-to-v2" w)
      withWorkloadsNoNudgeWarnings;
    expected = false;
  };

  # Workload with legacyVmName pointing to a missing VM → orphan warning.
  "realms/tombstone-orphan-legacy-vm-warning-fires" = {
    expr = {
      hasWarning = lib.any
        (w: lib.hasInfix "old-laptop-vm" w)
        orphanLegacyVmWarnings;
      mentionsWorkload = lib.any
        (w: lib.hasInfix "migrating" w && lib.hasInfix "laptop" w)
        orphanLegacyVmWarnings;
      suggestsDeclaringVm = lib.any
        (w: lib.hasInfix "d2b.vms.old-laptop-vm" w)
        orphanLegacyVmWarnings;
    };
    expected = {
      hasWarning = true;
      mentionsWorkload = true;
      suggestsDeclaringVm = true;
    };
  };

  # Workload with legacyVmName pointing to an EXISTING VM → no orphan warning.
  "realms/tombstone-no-orphan-warning-when-vm-exists" = {
    expr = lib.any
      (w: lib.hasInfix "homebox" w && lib.hasInfix "d2b.vms" w)
      legacyVmPresentWarnings;
    expected = false;
  };

  # ── accepted workload option shapes ─────────────────────────────────────────

  # Full workload options tree evaluates without assertion failures for all three
  # supported kinds (local-vm, qemu-media, provider-placeholder).
  "realms/accepted-all-workload-kinds-eval-clean" = {
    expr = {
      assertionsPass = lib.all (a: a.assertion) acceptedWorkloadCfg.assertions;
      # Filter to d2b-originated warnings only; the NixOS systemd.network
      # combination warning fires for any d2b config and is unrelated.
      noExtraWarnings =
        lib.filter (w: lib.hasInfix "d2b" w) acceptedWorkloadCfg.warnings == [ ];
      corpRealmHasWorkloads =
        acceptedWorkloadCfg.d2b.realms.corp.workloads != { };
    };
    expected = {
      assertionsPass = true;
      noExtraWarnings = true;
      corpRealmHasWorkloads = true;
    };
  };

  # local-vm workload fields default and materialize correctly.
  "realms/accepted-local-vm-workload-fields" = {
    expr =
      let
        laptop = acceptedWorkloadCfg.d2b.realms.corp.workloads.laptop;
      in {
        kind = laptop.kind;
        legacyVmName = laptop.legacyVmName;
        launcherEnable = laptop.launcher.enable;
        launcherLabel = laptop.launcher.label;
        launcherIconId = laptop.launcher.icon.id;
        launcherIconName = laptop.launcher.icon.name;
        memoryMiB = laptop.localVm.memoryMiB;
        vcpus = laptop.localVm.vcpus;
        # stateDir defaults to /var/lib/d2b/vms/<workload-id>
        stateDirMatchesId = lib.hasSuffix "/laptop" laptop.stateDir;
      };
    expected = {
      kind = "local-vm";
      legacyVmName = null;
      launcherEnable = true;
      launcherLabel = "Corp Laptop";
      launcherIconId = "computer-laptop";
      launcherIconName = "laptop";
      memoryMiB = 4096;
      vcpus = 2;
      stateDirMatchesId = true;
    };
  };

  # qemu-media workload fields materialize correctly.
  "realms/accepted-qemu-media-workload-fields" = {
    expr =
      let
        installer = acceptedWorkloadCfg.d2b.realms.corp.workloads.installer;
      in {
        kind = installer.kind;
        sourceKind = installer.qemuMedia.source.kind;
        sourceRef = installer.qemuMedia.source.ref;
        sourceReadOnly = installer.qemuMedia.source.readOnly;
        launcherEnable = installer.launcher.enable;
        launcherLabel = installer.launcher.label;
      };
    expected = {
      kind = "qemu-media";
      sourceKind = "physical-usb";
      sourceRef = "installer-usb";
      sourceReadOnly = true;
      launcherEnable = true;
      launcherLabel = "Live Installer";
    };
  };

  # provider-placeholder workload evaluates with launcher disabled.
  "realms/accepted-provider-placeholder-workload-fields" = {
    expr =
      let
        svc = acceptedWorkloadCfg.d2b.realms.corp.workloads.cloud-service;
      in {
        kind = svc.kind;
        launcherEnable = svc.launcher.enable;
      };
    expected = {
      kind = "provider-placeholder";
      launcherEnable = false;
    };
  };

  "realms/realm-host-prep-file-ownership-is-complete-and-disjoint" = {
    expr = {
      inherit duplicateOwnedFiles;
      currentNixInventoryComplete = plannedCurrentNixFiles == currentNixFiles;
      componentCount = builtins.length componentNames;
      branchesUnique =
        builtins.length componentBranches
        == builtins.length (lib.unique componentBranches);
      inherit unownedReservedPaths unreservedMissingOwnedFiles;
    };
    expected = {
      duplicateOwnedFiles = [ ];
      currentNixInventoryComplete = true;
      componentCount = 15;
      branchesUnique = true;
      unownedReservedPaths = [ ];
      unreservedMissingOwnedFiles = [ ];
    };
  };

  "realms/realm-host-prep-dependency-graph-is-ordered" = {
    expr = {
      orderCoversEveryComponent =
        lib.sort lib.lessThan realmHostPlan.componentOrder
        == lib.sort lib.lessThan componentNames;
      inherit dependencyErrors externalDependencyErrors;
      bundleDependenciesComplete =
        lib.sort lib.lessThan
          realmHostPlan.components.bundle-integration.dependsOn
        == lib.sort lib.lessThan expectedBundleDependencies;
      promptsReady = lib.all
        (component:
          realmHostPlan.components.${component}.branch != ""
          && realmHostPlan.components.${component}.prompt != ""
          && realmHostPlan.components.${component}.scope != [ ])
        componentNames;
    };
    expected = {
      orderCoversEveryComponent = true;
      dependencyErrors = [ ];
      externalDependencyErrors = [ ];
      bundleDependenciesComplete = true;
      promptsReady = true;
    };
  };

  "realms/realm-host-prep-preserves-cross-wave-contract-boundaries" = {
    expr = {
      sharedRoot = realmHostPlan.sharedRoot;
      bundleVersion = realmHostPlan.frozenParentContracts.bundle.version;
      bundleSchemaVersion =
        realmHostPlan.frozenParentContracts.bundle.schemaVersion;
      allocatorOwner = realmHostPlan.frozenParentContracts.allocator.owner;
      w7AllocatorOutputs =
        realmHostPlan.frozenParentContracts.allocator.w7Owns;
      w5AllocatorRuntime =
        realmHostPlan.frozenParentContracts.allocator.w5Owns;
      w7DeclarativeDocs =
        realmHostPlan.crossWaveOwnership.w7DeclarativeDocs;
      allocatorOwnsDeclarativeDocs = lib.all
        (path:
          builtins.elem path
            realmHostPlan.components.allocator-emission.ownedFiles)
        realmHostPlan.crossWaveOwnership.w7DeclarativeDocs;
      inherit crossWaveOverlaps deferredPurgeOverlaps foreignAffectedOverlaps;
      foreignOwners = {
        w5 = realmHostPlan.crossWaveOwnership.w5Owner;
        w6 = realmHostPlan.crossWaveOwnership.w6RuntimeDocs.owner;
        purge = realmHostPlan.crossWaveOwnership.deferredPurgeDocs.owner;
        frozen = realmHostPlan.crossWaveOwnership.frozenContractDocs.owner;
      };
      inherit forbiddenOwnedFiles;
    };
    expected = {
      sharedRoot = "47a55e101b5b62e6a89e342512125de43bac4e68";
      bundleVersion = 12;
      bundleSchemaVersion = "v2";
      allocatorOwner = "w5";
      w7AllocatorOutputs = [
        "declarative child listener rows"
        "declarative lease requests"
        "declarative process and ordering records"
        "declarative cgroup, namespace, resource, and ownership records"
      ];
      w5AllocatorRuntime = [
        "allocator service dispatch"
        "runtime listener creation and binding"
        "typed child controller and broker spawn"
        "pidfd supervision and adoption"
        "lease allocation, reconciliation, revocation, and execution"
      ];
      w7DeclarativeDocs = [
        "docs/reference/local-root-allocator.md"
        "docs/reference/realm-identity-lifecycle.md"
      ];
      allocatorOwnsDeclarativeDocs = true;
      crossWaveOverlaps = [ ];
      deferredPurgeOverlaps = [ ];
      foreignAffectedOverlaps = [ ];
      foreignOwners = {
        w5 = "w5";
        w6 = "w6";
        purge = "w10";
        frozen = "shared-root";
      };
      forbiddenOwnedFiles = [ ];
    };
  };

  "realms/realm-host-prep-provider-registry-extension-seams-are-exclusive" = {
    expr = {
      owner = realmHostPlan.providerRegistryExtensionSeams.owner;
      approvedProtectedFiles =
        realmHostPlan.providerRegistryExtensionSeams.approvedProtectedFiles;
      ownerHasEveryProtectedFile = lib.all
        (path:
          builtins.elem path
            realmHostPlan.components.provider-registry-composition.ownedFiles)
        realmHostPlan.providerRegistryExtensionSeams.approvedProtectedFiles;
      integrationOwner =
        realmHostPlan.providerRegistryExtensionSeams.integrationOwner;
      flakeOwnedByIntegration =
        builtins.elem "flake.nix"
          realmHostPlan.components.bundle-integration.ownedFiles
        && !(builtins.elem "flake.nix"
          realmHostPlan.components.provider-registry-composition.ownedFiles);
      sharedRootConsumerDependency =
        realmHostPlan.providerRegistryExtensionSeams.sharedRootConsumerDependency;
      implementationBlocked =
        realmHostPlan.externalDependencies.${realmHostPlan.providerRegistryExtensionSeams.sharedRootConsumerDependency}.status
        == "blocked";
      consumerSeamCommit =
        realmHostPlan.externalDependencies.${realmHostPlan.providerRegistryExtensionSeams.sharedRootConsumerDependency}.landedCommit;
      inherit providerFragmentOwnershipValid;
      preservedAxes =
        realmHostPlan.frozenParentContracts.providerRegistry.preservedAxes;
    };
    expected = {
      owner = "provider-registry-composition";
      approvedProtectedFiles = [
        "docs/reference/schemas/v2/provider-registry-v2.json"
        "docs/reference/schemas/v2/provider-registry-v2.md"
        "nixos-modules/provider-registry-v2-json.nix"
        "packages/d2b-contracts/src/provider_registry_v2.rs"
      ];
      ownerHasEveryProtectedFile = true;
      integrationOwner = "bundle-integration";
      flakeOwnedByIntegration = true;
      sharedRootConsumerDependency =
        "shared-root-provider-registry-open-consumer-seam";
      implementationBlocked = false;
      consumerSeamCommit =
        "fa18c34741b8a898b4786a14e19e86e395d37325";
      providerFragmentOwnershipValid = true;
      preservedAxes = [
        "local-observability"
        "local-runtime"
      ];
    };
  };

  "realms/realm-host-prep-affected-inventory-has-exact-owners" = {
    expr = {
      affectedInventoryComplete =
        lib.sort lib.lessThan affectedInventoryPaths
        == lib.sort lib.lessThan ownedAffectedInventoryPaths;
      affectedReservedInventoryComplete =
        lib.sort lib.lessThan affectedInventoryReservedPaths
        == lib.sort lib.lessThan reservedAffectedInventoryPaths;
      inventoryPathsUnique =
        builtins.length affectedInventoryPaths
        == builtins.length (lib.unique affectedInventoryPaths);
      fixturesInventoried =
        lib.any
          (row: row.paths != [ ])
          realmHostPlan.affectedInventory.fixtures;
      examplesInventoried =
        lib.any
          (row: row.paths != [ ])
          realmHostPlan.affectedInventory.examples;
      docsInventoried =
        lib.any
          (row: row.paths != [ ])
          realmHostPlan.affectedInventory.docs;
      testsInventoried =
        lib.any
          (row: row.paths != [ ])
          realmHostPlan.affectedInventory.tests;
      contractTestsInventoried =
        lib.any
          (row: row.paths != [ ])
          realmHostPlan.affectedInventory.contractTests;
      inherit deletionExtensionOwnershipValid;
      deletionExtensionPathsUnique =
        builtins.length deletionExtensionPaths
        == builtins.length (lib.unique deletionExtensionPaths);
      deletionExtensionPaths =
        lib.sort lib.lessThan deletionExtensionPaths;
    };
    expected = {
      affectedInventoryComplete = true;
      affectedReservedInventoryComplete = true;
      inventoryPathsUnique = true;
      fixturesInventoried = true;
      examplesInventoried = true;
      docsInventoried = true;
      testsInventoried = true;
      contractTestsInventoried = true;
      deletionExtensionOwnershipValid = true;
      deletionExtensionPathsUnique = true;
      deletionExtensionPaths = [
        "packages/d2b-contract-tests/tests/policy_host_realm_relay.rs"
        "packages/d2b-contract-tests/tests/policy_misc.rs"
        "packages/d2b-contract-tests/tests/policy_modules.rs"
        "packages/d2b-contract-tests/tests/policy_source.rs"
        "packages/d2b-contract-tests/tests/realm_workload_schema_contract.rs"
        "packages/d2b-contract-tests/tests/storage_sync_contracts.rs"
        "tests/migration-ledger.toml"
        "tests/migration-state.d/polkit-allowlist-eval.toml"
        "tests/migration-state.d/vm-submodule-cutover-eval.toml"
        "tests/migration-state.d/vm-submodule-eval.toml"
        "tests/static.sh"
      ];
    };
  };

  "realms/realm-host-component-diff-policy-fails-closed" = {
    expr = {
      allowedSchema = {
        inherit (allowedSchemaDiff) component valid violations;
      };
      deniedCrossComponent = {
        inherit (deniedCrossComponentDiff) component valid violations;
      };
      blockedProviderRegistry = {
        inherit (blockedProviderRegistryDiff)
          blockedExternalDependencies
          component
          valid
          violations
          ;
      };
      blockedDeletionContract = {
        inherit (blockedDeletionContractDiff)
          blockedExternalDependencies
          component
          valid
          violations
          ;
      };
      blockedFixture = {
        inherit (blockedFixtureDiff)
          blockedExternalDependencies
          component
          valid
          violations
          ;
      };
      blockedAllocatorDoc = {
        inherit (blockedAllocatorDocDiff)
          blockedExternalDependencies
          component
          valid
          violations
          ;
      };
      blockedBundleIntegration = {
        inherit (blockedBundleIntegrationDiff)
          blockedExternalDependencies
          component
          valid
          violations
          ;
      };
      gitEnvironmentHardening = {
        sanitizedEnvironment = lib.hasInfix "env -i" componentGateSource;
        replacementDisabled =
          lib.hasInfix "export GIT_NO_REPLACE_OBJECTS=1" componentGateSource;
        externalGraftDisabled =
          lib.hasInfix "export GIT_GRAFT_FILE=/dev/null" componentGateSource
          && lib.hasInfix "GIT_GRAFT_FILE=/dev/null \\" componentGateSource;
        externalShallowDisabled =
          lib.hasInfix "export GIT_SHALLOW_FILE=/dev/null" componentGateSource
          && lib.hasInfix "GIT_SHALLOW_FILE=/dev/null \\" componentGateSource;
        globalConfigDisabled =
          lib.hasInfix "GIT_CONFIG_GLOBAL=/dev/null" componentGateSource
          && lib.hasInfix "GIT_CONFIG_NOSYSTEM=1" componentGateSource;
        repositoryMetadataRejected =
          lib.hasInfix "info/grafts" componentGateSource
          && lib.hasInfix "objects/info/alternates" componentGateSource
          && lib.hasInfix "refs/replace" componentGateSource;
        submodulesForced =
          lib.hasInfix "--ignore-submodules=none" componentGateSource
          && lib.hasInfix "diff.ignoreSubmodules=none" componentGateSource;
      };
    };
    expected = {
      allowedSchema = {
        component = "realm-schema";
        valid = true;
        violations = [ ];
      };
      deniedCrossComponent = {
        component = "realm-schema";
        valid = false;
        violations = [ "nixos-modules/index.nix" ];
      };
      blockedProviderRegistry = {
        component = "provider-registry-composition";
        valid = false;
        violations = [ ];
        blockedExternalDependencies = [
          "shared-root-deletion-contract-test-seam"
        ];
      };
      blockedDeletionContract = {
        component = "workload-processes";
        valid = false;
        violations = [ ];
        blockedExternalDependencies = [
          "shared-root-deletion-contract-test-seam"
        ];
      };
      blockedFixture = {
        component = "realm-devices";
        valid = false;
        violations = [ ];
        blockedExternalDependencies = [
          "shared-root-w7-fixture-path-ownership"
        ];
      };
      blockedAllocatorDoc = {
        component = "allocator-emission";
        valid = false;
        violations = [ ];
        blockedExternalDependencies = [
          "shared-root-deletion-contract-test-seam"
          "w5-runtime-document-split"
        ];
      };
      blockedBundleIntegration = {
        component = "bundle-integration";
        valid = false;
        violations = [ ];
        blockedExternalDependencies = [
          "shared-root-deletion-contract-test-seam"
        ];
      };
      gitEnvironmentHardening = {
        sanitizedEnvironment = true;
        replacementDisabled = true;
        externalGraftDisabled = true;
        externalShallowDisabled = true;
        globalConfigDisabled = true;
        repositoryMetadataRejected = true;
        submodulesForced = true;
      };
    };
  };
}
