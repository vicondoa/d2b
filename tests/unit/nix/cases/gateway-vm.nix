# Eval coverage for realm gateway declarations.
{ lib, mkEval, flakeRoot, ... }:

let
  hostBase = {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";

    nixling.envs.work = {
      lanSubnet = "10.44.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
  };

  base = lib.recursiveUpdate hostBase {
    nixling.gateways.work = {
      env = "work";
      index = 20;
      relay.namespace = "relns-example.servicebus.windows.net";
      relay.entity = "hc-nixling-display";
      aca = {
        endpoint = "https://example.azurecontainerapps.io";
        subscription = "00000000-0000-0000-0000-000000000000";
        resourceGroup = "rg-nixling-centralus";
        sandboxGroup = "casbx-nixling-demo";
        region = "centralus";
        image = "registry.example.azurecr.io/nixling-wayland:mi";
        diskName = "nixling-wayland-mi";
        managedIdentityResourceId = "/subscriptions/00000000-0000-0000-0000-000000000000/resourceGroups/rg/providers/Microsoft.ManagedIdentity/userAssignedIdentities/nixling";
        managedIdentityClientId = "11111111-1111-1111-1111-111111111111";
      };
      display.waypipeSocket = "/run/user/1000/wpc.sock";
    };
  };

  goodCfg = (mkEval [ base ]).config;
  noGatewayCfg = (mkEval [ hostBase ]).config;
  noRelayGatewayCfg = (mkEval [
    (lib.recursiveUpdate hostBase {
      nixling.gateways.work = {
        env = "work";
        index = 20;
      };
    })
  ]).config;
  gatewayGuestCfg = goodCfg.nixling._computed."sys-work-gateway".config;
  gatewayGuestService = gatewayGuestCfg.systemd.services.nixlingd.serviceConfig;
  gatewayGuestTmpfiles = gatewayGuestCfg.systemd.tmpfiles.rules;
  hostTmpfiles = goodCfg.systemd.tmpfiles.rules;
  gatewayJson = builtins.fromJSON gatewayGuestCfg.environment.etc."nixling/gateway.json".text;
  hostDaemonJson = builtins.fromJSON goodCfg.environment.etc."nixling/daemon-config.json".text;
  hostGatewayJsonPresent = builtins.hasAttr "nixling/gateway.json" goodCfg.environment.etc;
  hostRealmEntrypoints = goodCfg.nixling._computed.realmEntrypoints;
  hostRealmRelayEgressPolicy = goodCfg.nixling._computed.hostRealmRelayEgressPolicy;
  renderText = value:
    if builtins.isString value then value
    else if builtins.isList value then lib.concatStringsSep "\n" (map renderText value)
    else if builtins.isPath value then toString value
    else if builtins.isAttrs value && value ? outPath then toString value
    else "";
  hostActivationText = lib.concatStringsSep "\n"
    (map (script: renderText (script.text or "")) (builtins.attrValues goodCfg.system.activationScripts));
  hostServiceText = lib.concatStringsSep "\n" (lib.mapAttrsToList
    (name: service:
      let serviceConfig = service.serviceConfig or { };
      in lib.concatStringsSep "\n" [
        name
        (renderText (serviceConfig.ExecStart or ""))
        (renderText (serviceConfig.Environment or ""))
      ])
    goodCfg.systemd.services);
  hostPackageRefs = map (pkg: {
    name = pkg.pname or (pkg.name or (lib.getName pkg));
    path = toString pkg;
  }) goodCfg.environment.systemPackages;
  forbiddenHostRealmMaterial = [
    "SharedAccessKey"
    "Endpoint=sb://"
    "AccountKey"
    "relns-example.servicebus.windows.net"
    "hc-nixling-display"
    "https://example.azurecontainerapps.io"
    "00000000-0000-0000-0000-000000000000"
    "rg-nixling-centralus"
    "casbx-nixling-demo"
    "centralus"
    "registry.example.azurecr.io/nixling-wayland:mi"
    "nixling-wayland-mi"
    "/subscriptions/00000000-0000-0000-0000-000000000000/resourceGroups/rg/providers/Microsoft.ManagedIdentity/userAssignedIdentities/nixling"
    "11111111-1111-1111-1111-111111111111"
    "/var/lib/nixling/gateways/work/credential.sealed.json"
    "/var/lib/nixling/gateways/work/seal.key"
    "NIXLING_RELAY_NAMESPACE"
    "NIXLING_RELAY_ENTITY"
    "NIXLING_RELAY_SAS_TOKEN"
    "NIXLING_RELAY_ENTRA_TOKEN"
    "NIXLING_RELAY_KEY_NAME"
    "NIXLING_RELAY_KEY"
  ];
  forbiddenRemoteRegistryMarkers = [
    "\"remoteNodes\""
    "\"remoteNodeRegistry\""
    "\"nodeRegistry\""
    "\"realmNodeRegistry\""
    "\"realmRegistry\""
    "\"registryNodes\""
    "remote-node-registry"
  ];
  jsonContainsAny = needles: value:
    lib.any (needle: lib.hasInfix needle (builtins.toJSON value)) needles;
  containsForbiddenRealmMaterial = jsonContainsAny forbiddenHostRealmMaterial;
  containsRemoteRegistryMarker = jsonContainsAny forbiddenRemoteRegistryMarkers;
  localFastPathSnapshot = cfg:
    let daemonJson = builtins.fromJSON cfg.environment.etc."nixling/daemon-config.json".text;
    in {
      daemonConfigPresent = builtins.hasAttr "nixling/daemon-config.json" cfg.environment.etc;
      publicSocketPath = daemonJson.publicSocketPath;
      publicSocketGroup = daemonJson.publicSocketGroup;
      brokerSocketPath = daemonJson.brokerSocketPath;
      nixlingdServicePresent = builtins.hasAttr "nixlingd" cfg.systemd.services;
      nixlingdSupplementaryGroups = cfg.systemd.services.nixlingd.serviceConfig.SupplementaryGroups;
      runDirAllowsLocalLaunchers =
        builtins.elem "d /run/nixling 1770 root nixling -" cfg.systemd.tmpfiles.rules
        && builtins.elem "a+ /run/nixling - - - - g::r-x" cfg.systemd.tmpfiles.rules
        && builtins.elem "a+ /run/nixling - - - - u:nixlingd:rwx" cfg.systemd.tmpfiles.rules
        && builtins.elem "a+ /run/nixling - - - - m::rwx" cfg.systemd.tmpfiles.rules;
      realmEntries = lib.sort lib.lessThan (builtins.attrNames cfg.nixling._computed.realmEntrypoints.entries);
      localEntrypoint = cfg.nixling._computed.realmEntrypoints.entries.local;
      hostGatewayJsonPresent = builtins.hasAttr "nixling/gateway.json" cfg.environment.etc;
    };
  gatewayProc = lib.findFirst (vm: vm.vm == "sys-work-gateway") null
    goodCfg.nixling._bundle.processesJson.data.vms;
  badCfg = (mkEval [
    (lib.recursiveUpdate base {
      nixling.gateways.work.credentialPath = "SharedAccessKey=bad";
    })
  ]).config;
  failureMessages = cfg: map (a: a.message) (lib.filter (a: !a.assertion) cfg.assertions);
  badMessages = failureMessages badCfg;
  badStateOutsideMessages = failureMessages ((mkEval [
    (lib.recursiveUpdate base {
      nixling.gateways.work = {
        stateDir = "/var/lib/other/work";
        credentialPath = "/var/lib/other/work/credential.json";
      };
    })
  ]).config);
  badCredentialOutsideStateMessages = failureMessages ((mkEval [
    (lib.recursiveUpdate base {
      nixling.gateways.work.credentialPath = "/var/lib/nixling/other/credential.json";
    })
  ]).config);
  badSealKeyOutsideStateMessages = failureMessages ((mkEval [
    (lib.recursiveUpdate base {
      nixling.gateways.work.sealKeyPath = "/var/lib/nixling/other/seal.key";
    })
  ]).config);
  badTraversalMessages = failureMessages ((mkEval [
    (lib.recursiveUpdate base {
      nixling.gateways.work = {
        stateDir = "/var/lib/nixling/gateways/../work";
        credentialPath = "/var/lib/nixling/gateways/../work/credential.json";
      };
    })
  ]).config);
  badPerVmStateMessages = failureMessages ((mkEval [
    (lib.recursiveUpdate base {
      nixling.gateways.work = {
        stateDir = "/var/lib/nixling/vms/sys-work-gateway";
        credentialPath = "/var/lib/nixling/vms/sys-work-gateway/credential.json";
      };
    })
  ]).config);
  badDaemonDisabledMessages = failureMessages ((mkEval [
    (lib.recursiveUpdate base {
      nixling.daemonExperimental.enable = false;
    })
  ]).config);
  multiGatewayCfg = (mkEval [
    (lib.recursiveUpdate base {
      nixling.envs.personal = {
        lanSubnet = "10.45.0.0/24";
        uplinkSubnet = "198.51.100.0/30";
      };
      nixling.gateways.personal = {
        env = "personal";
        index = 21;
        relay.namespace = "relns-personal.servicebus.windows.net";
        relay.entity = "hc-nixling-display";
      };
    })
  ]).config;
  multiGatewayMessages = map (a: a.message) (lib.filter (a: !a.assertion) multiGatewayCfg.assertions);
  multiGatewayRealmEntrypoints = multiGatewayCfg.nixling._computed.realmEntrypoints;
  customGatewayNameCfg = (mkEval [
    (lib.recursiveUpdate base {
      nixling.gateways.work.vmName = "corp-gateway";
    })
  ]).config;
  customGatewayNameEntrypoints = customGatewayNameCfg.nixling._computed.realmEntrypoints;
  duplicateGatewayRealmMessages = failureMessages ((mkEval [
    (lib.recursiveUpdate base {
      nixling.envs.personal = {
        lanSubnet = "10.45.0.0/24";
        uplinkSubnet = "198.51.100.0/30";
      };
      nixling.gateways.personal = {
        realm = "work";
        env = "personal";
        index = 21;
        relay.namespace = "relns-personal.servicebus.windows.net";
        relay.entity = "hc-nixling-display";
      };
    })
  ]).config);
  sharedGatewayEnvMessages = failureMessages ((mkEval [
    (lib.recursiveUpdate base {
      nixling.gateways.personal = {
        realm = "personal";
        env = "work";
        index = 21;
        relay.namespace = "relns-personal.servicebus.windows.net";
        relay.entity = "hc-nixling-display";
      };
    })
  ]).config);
  localRealmGatewayMessages = failureMessages ((mkEval [
    (lib.recursiveUpdate base {
      nixling.gateways.work.realm = "local";
    })
  ]).config);
  retiredHostRelayCredentialMessages = failureMessages ((mkEval [
    (lib.recursiveUpdate base {
      nixling.gateways.work.allowHostRelayCredentials = true;
    })
  ]).config);
  sourceToolsCfg = (mkEval [
    (lib.recursiveUpdate base {
      nixling.site.usePrebuiltHostTools = false;
    })
  ]).config;
  sourceGatewayGuestCfg = sourceToolsCfg.nixling._computed."sys-work-gateway".config;
  gatewayModuleSource = builtins.readFile (flakeRoot + "/nixos-modules/gateway-vm.nix");
  packageNames = map (pkg: pkg.pname or (lib.getName pkg)) gatewayGuestCfg.environment.systemPackages;
in
{
  "gateway-vm/auto-declared-name" = {
    expr = builtins.elem "sys-work-gateway" (builtins.attrNames goodCfg.nixling.vms);
    expected = true;
  };

  "gateway-vm/auto-declared-env-index" = {
    expr = {
      env = goodCfg.nixling.vms."sys-work-gateway".env;
      index = goodCfg.nixling.vms."sys-work-gateway".index;
      sshUser = goodCfg.nixling.vms."sys-work-gateway".ssh.user;
    };
    expected = {
      env = "work";
      index = 20;
      sshUser = "gateway";
    };
  };

  "gateway-vm/admits-framework-sys-prefix" = {
    expr = lib.any (m: lib.hasInfix "prefixes are reserved" m)
      (map (a: a.message) (lib.filter (a: !a.assertion) goodCfg.assertions));
    expected = false;
  };

  "gateway-vm/rejects-secret-shaped-credential-path" = {
    expr = lib.any
      (m: lib.hasInfix "nixling.gateways.work.credentialPath must be an absolute runtime" m)
      badMessages;
    expected = true;
  };

  "gateway-vm/rejects-state-dir-outside-site-state" = {
    expr = lib.any
      (m: lib.hasInfix "nixling.gateways.work.stateDir must be an absolute runtime" m)
      badStateOutsideMessages;
    expected = true;
  };

  "gateway-vm/rejects-credential-path-outside-gateway-state" = {
    expr = lib.any
      (m: lib.hasInfix "nixling.gateways.work.credentialPath must live under" m)
      badCredentialOutsideStateMessages;
    expected = true;
  };

  "gateway-vm/rejects-seal-key-path-outside-gateway-state" = {
    expr = lib.any
      (m: lib.hasInfix "nixling.gateways.work.sealKeyPath must live under" m)
      badSealKeyOutsideStateMessages;
    expected = true;
  };

  "gateway-vm/rejects-parent-traversal-in-state-paths" = {
    expr = lib.any
      (m: lib.hasInfix "must not contain `..` path" m)
      badTraversalMessages;
    expected = true;
  };

  "gateway-vm/rejects-per-vm-state-root-for-gateway-secrets" = {
    expr = lib.any
      (m: lib.hasInfix "stateDir must not live under" m && lib.hasInfix "nixling.store.stateDir" m)
      badPerVmStateMessages;
    expected = true;
  };

  "gateway-vm/requires-daemon-control-plane" = {
    expr = lib.any
      (m: lib.hasInfix "nixling.gateways requires nixling.daemonExperimental.enable = true" m)
      badDaemonDisabledMessages;
    expected = true;
  };

  "gateway-vm/waypipe-not-in-host-runner-dag" = {
    expr = lib.any (n: n.id == "gateway-waypipe-client" || n.id == "gateway-waypipe-server")
      gatewayProc.nodes;
    expected = false;
  };

  "gateway-vm/guest-services-installed-without-static-waypipe" = {
    expr = {
      hasNixlingd = builtins.hasAttr "nixlingd" gatewayGuestCfg.systemd.services;
      gatewayJson = builtins.hasAttr "nixling/gateway.json" gatewayGuestCfg.environment.etc;
      daemonJson = builtins.hasAttr "nixling/daemon-config.json" gatewayGuestCfg.environment.etc;
      gatewayAca = {
        subscription = gatewayJson.aca.subscription;
        resourceGroup = gatewayJson.aca.resourceGroup;
        sandboxGroup = gatewayJson.aca.sandboxGroup;
        region = gatewayJson.aca.region;
        image = gatewayJson.aca.image;
        diskName = gatewayJson.aca.diskName;
        managedIdentityClientId = gatewayJson.aca.managedIdentityClientId;
        cpu = gatewayJson.aca.cpu;
        memory = gatewayJson.aca.memory;
        autoSuspendIntervalSecs = gatewayJson.aca.autoSuspendIntervalSecs;
      };

      hasWaypipeSocket = gatewayJson.display ? waypipeSocket;
      sealKeyPath = gatewayJson.sealKeyPath;
      credentialPath = gatewayJson.credentialPath;
      hasEnrollmentHelper = builtins.elem "nixling-gateway-runtime" packageNames;
      hasWaypipeClient = builtins.hasAttr "nixling-gateway-waypipe-client" gatewayGuestCfg.systemd.services;
      hasWaypipeServer = builtins.hasAttr "nixling-gateway-waypipe-server" gatewayGuestCfg.systemd.services;
    };
    expected = {
      hasNixlingd = true;
      gatewayJson = true;
      daemonJson = true;
      gatewayAca = {
        subscription = "00000000-0000-0000-0000-000000000000";
        resourceGroup = "rg-nixling-centralus";
        sandboxGroup = "casbx-nixling-demo";
        region = "centralus";
        image = "registry.example.azurecr.io/nixling-wayland:mi";
        diskName = "nixling-wayland-mi";
        managedIdentityClientId = "11111111-1111-1111-1111-111111111111";
        cpu = "1000m";
        memory = "2048Mi";
        autoSuspendIntervalSecs = 600;
      };
      sealKeyPath = "/var/lib/nixling/gateways/work/seal.key";
      credentialPath = "/var/lib/nixling/gateways/work/credential.sealed.json";
      hasEnrollmentHelper = true;
      hasWaypipeSocket = false;
      hasWaypipeClient = false;
      hasWaypipeServer = false;
    };
  };

  "gateway-vm/gateway-guest-json-retains-realm-provider-material" = {
    expr = {
      guestGatewayJsonPresent = builtins.hasAttr "nixling/gateway.json" gatewayGuestCfg.environment.etc;
      guestCarriesGatewayProviderMaterial = containsForbiddenRealmMaterial gatewayJson;
      inherit hostGatewayJsonPresent;
    };
    expected = {
      guestGatewayJsonPresent = true;
      guestCarriesGatewayProviderMaterial = true;
      hostGatewayJsonPresent = false;
    };
  };

  "gateway-vm/guest-daemon-runs-as-nixlingd-without-no-drop-flag" = {
    expr = {
      user = gatewayGuestService.User;
      group = gatewayGuestService.Group;
      supplementaryGroups = gatewayGuestService.SupplementaryGroups;
      execStartHasNoDropFlag = lib.hasInfix "--no-drop-privileges" (toString gatewayGuestService.ExecStart);
      noNewPrivileges = gatewayGuestService.NoNewPrivileges;
      capabilityBoundingSet = gatewayGuestService.CapabilityBoundingSet;
      ambientCapabilities = gatewayGuestService.AmbientCapabilities;
      restartIfChanged = gatewayGuestCfg.systemd.services.nixlingd.restartIfChanged;
    };
    expected = {
      user = "nixlingd";
      group = "nixlingd";
      supplementaryGroups = [ "nixling" ];
      execStartHasNoDropFlag = false;
      noNewPrivileges = true;
      capabilityBoundingSet = [ "" ];
      ambientCapabilities = [ "" ];
      restartIfChanged = false;
    };
  };

  "gateway-vm/host-guest-state-ownership-boundary" = {
    expr = {
      hostStateDir = builtins.elem "d /var/lib/nixling/gateways/work 0750 root nixlingd -" hostTmpfiles;
      guestStateDir = builtins.elem "d /var/lib/nixling/gateways/work 0700 nixlingd nixlingd -" gatewayGuestTmpfiles;
      guestDaemonStateDir = builtins.elem "d /var/lib/nixling/daemon-state 0700 nixlingd nixlingd -" gatewayGuestTmpfiles;
      guestCacheDir = builtins.elem "d /var/cache/nixling 0750 root nixlingd -" gatewayGuestTmpfiles;
      guestLockFile = builtins.elem "f /run/nixling/daemon.lock 0640 nixlingd nixlingd -" gatewayGuestTmpfiles;
      gatewayUserCanReachPublicSocket = builtins.elem "nixling" gatewayGuestCfg.users.users.gateway.extraGroups;
    };
    expected = {
      hostStateDir = false;
      guestStateDir = true;
      guestDaemonStateDir = true;
      guestCacheDir = true;
      guestLockFile = true;
      gatewayUserCanReachPublicSocket = true;
    };
  };

  "gateway-vm/host-activation-and-services-exclude-realm-provider-material" = {
    expr = {
      activationCarriesRelayOrAcaMaterial = containsForbiddenRealmMaterial hostActivationText;
      servicesCarryRelayOrAcaMaterial = containsForbiddenRealmMaterial hostServiceText;
      servicesCarryGatewayRuntime = lib.hasInfix "nixling-gateway-relay" hostServiceText
        || lib.hasInfix "nixling-gateway-enroll" hostServiceText;
    };
    expected = {
      activationCarriesRelayOrAcaMaterial = false;
      servicesCarryRelayOrAcaMaterial = false;
      servicesCarryGatewayRuntime = false;
    };
  };

  "gateway-vm/host-daemon-stays-credential-free-facade" = {
    expr = {
      daemonConfigCarriesGateway = hostDaemonJson ? gateway;
      inherit hostGatewayJsonPresent;
    };
    expected = {
      daemonConfigCarriesGateway = false;
      hostGatewayJsonPresent = false;
    };
  };

  "gateway-vm/host-daemon-config-excludes-realm-provider-material" = {
    expr = {
      daemonConfigCarriesGateway = hostDaemonJson ? gateway;
      carriesRelayOrAcaMaterial = containsForbiddenRealmMaterial hostDaemonJson;
      carriesRemoteNodeRegistry = containsRemoteRegistryMarker hostDaemonJson;
      gatewayConfigPath = hostDaemonJson.gatewayConfigPath;
    };
    expected = {
      daemonConfigCarriesGateway = false;
      carriesRelayOrAcaMaterial = false;
      carriesRemoteNodeRegistry = false;
      gatewayConfigPath = "/etc/nixling/gateway.json";
    };
  };

  "gateway-vm/host-system-packages-exclude-realm-provider-material" = {
    expr = {
      carriesRelayOrAcaMaterial = containsForbiddenRealmMaterial hostPackageRefs;
      carriesRemoteNodeRegistry = containsRemoteRegistryMarker hostPackageRefs;
      hasGatewayRelayPackage = lib.any
        (pkg: lib.hasInfix "nixling-gateway-relay" pkg.name || lib.hasInfix "nixling-gateway-relay" pkg.path)
        hostPackageRefs;
      hasGatewayRuntimePackage = lib.any
        (pkg: lib.hasInfix "nixling-gateway-runtime" pkg.name || lib.hasInfix "nixling-gateway-runtime" pkg.path)
        hostPackageRefs;
    };
    expected = {
      carriesRelayOrAcaMaterial = false;
      carriesRemoteNodeRegistry = false;
      hasGatewayRelayPackage = false;
      hasGatewayRuntimePackage = false;
    };
  };

  "gateway-vm/host-realm-relay-egress-policy-is-redacted-and-gateway-scoped" = {
    expr = {
      path = hostRealmRelayEgressPolicy.path;
      mode = hostRealmRelayEgressPolicy.mode;
      gatewayInterfaces = hostRealmRelayEgressPolicy.gatewayInterfaces;
      forbiddenHostEnvPrefixes = hostRealmRelayEgressPolicy.forbiddenHostEnvPrefixes;
      diagnostics = hostRealmRelayEgressPolicy.diagnostics;
      carriesRelayOrAcaMaterial =
        containsForbiddenRealmMaterial (removeAttrs hostRealmRelayEgressPolicy [ "forbiddenHostEnvPrefixes" ]);
    };
    expected = {
      path = "/etc/nixling/host-realm-relay-egress-policy.json";
      mode = "host-realm-relay-deny";
      gatewayInterfaces = [ "work-l20" ];
      forbiddenHostEnvPrefixes = [ "NIXLING_RELAY_" ];
      diagnostics = {
        redacted = true;
        rateLimited = true;
        fields = [ "event" "protocol" "reason" "gatewayInterfaceClass" ];
        omitted = [ "payload" "headers" "token" "endpoint" "credential" ];
      };
      carriesRelayOrAcaMaterial = false;
    };
  };

  "gateway-vm/host-bundle-process-artifacts-exclude-realm-provider-material" = {
    expr = {
      gatewayVmProcessPresent = gatewayProc != null;
      realmMaterial = {
        bundle = containsForbiddenRealmMaterial goodCfg.nixling._bundle.bundle.data;
        host = containsForbiddenRealmMaterial goodCfg.nixling._bundle.hostJson.data;
        processes = containsForbiddenRealmMaterial goodCfg.nixling._bundle.processesJson.data;
      };
      remoteNodeRegistry = {
        bundle = containsRemoteRegistryMarker goodCfg.nixling._bundle.bundle.data;
        host = containsRemoteRegistryMarker goodCfg.nixling._bundle.hostJson.data;
        processes = containsRemoteRegistryMarker goodCfg.nixling._bundle.processesJson.data;
      };
    };
    expected = {
      gatewayVmProcessPresent = true;
      realmMaterial = {
        bundle = false;
        host = false;
        processes = false;
      };
      remoteNodeRegistry = {
        bundle = false;
        host = false;
        processes = false;
      };
    };
  };

  "gateway-vm/host-realm-entrypoint-table-defaults-local-and-gateway" = {
    expr = {
      path = hostRealmEntrypoints.path;
      local = hostRealmEntrypoints.entries.local;
      work = hostRealmEntrypoints.entries.work;
      workCarriesProviderConfig =
        (hostRealmEntrypoints.entries.work ? credentialPath)
        || (hostRealmEntrypoints.entries.work ? relay)
        || (hostRealmEntrypoints.entries.work ? aca);
    };
    expected = {
      path = "/run/current-system/sw/share/nixling/realm-entrypoints.json";
      local = {
        mode = "host-resident";
        gateway = null;
      };
      work = {
        mode = "gateway-backed";
        gateway = "sys-work-gateway.nixling";
      };
      workCarriesProviderConfig = false;
    };
  };

  "gateway-vm/host-realm-entrypoints-exclude-realm-provider-material" = {
    expr = {
      entries = hostRealmEntrypoints.entries;
      carriesRelayOrAcaMaterial = containsForbiddenRealmMaterial hostRealmEntrypoints;
      carriesRemoteNodeRegistry = containsRemoteRegistryMarker hostRealmEntrypoints;
      workCarriesProviderConfig =
        (hostRealmEntrypoints.entries.work ? credentialPath)
        || (hostRealmEntrypoints.entries.work ? relay)
        || (hostRealmEntrypoints.entries.work ? aca)
        || (hostRealmEntrypoints.entries.work ? remoteNodes)
        || (hostRealmEntrypoints.entries.work ? remoteNodeRegistry)
        || (hostRealmEntrypoints.entries.work ? nodeRegistry);
    };
    expected = {
      entries = {
        local = {
          mode = "host-resident";
          gateway = null;
        };
        work = {
          mode = "gateway-backed";
          gateway = "sys-work-gateway.nixling";
        };
      };
      carriesRelayOrAcaMaterial = false;
      carriesRemoteNodeRegistry = false;
      workCarriesProviderConfig = false;
    };
  };

  "gateway-vm/local-fast-path-auth-socket-does-not-require-gateway-relay" = {
    expr = {
      noGateway = localFastPathSnapshot noGatewayCfg;
      gatewayWithoutRelay = localFastPathSnapshot noRelayGatewayCfg;
    };
    expected = {
      noGateway = {
        daemonConfigPresent = true;
        publicSocketPath = "/run/nixling/public.sock";
        publicSocketGroup = "nixling";
        brokerSocketPath = "/run/nixling/priv.sock";
        nixlingdServicePresent = true;
        nixlingdSupplementaryGroups = [ "nixling" ];
        runDirAllowsLocalLaunchers = true;
        realmEntries = [ "local" ];
        localEntrypoint = {
          mode = "host-resident";
          gateway = null;
        };
        hostGatewayJsonPresent = false;
      };
      gatewayWithoutRelay = {
        daemonConfigPresent = true;
        publicSocketPath = "/run/nixling/public.sock";
        publicSocketGroup = "nixling";
        brokerSocketPath = "/run/nixling/priv.sock";
        nixlingdServicePresent = true;
        nixlingdSupplementaryGroups = [ "nixling" ];
        runDirAllowsLocalLaunchers = true;
        realmEntries = [ "local" "work" ];
        localEntrypoint = {
          mode = "host-resident";
          gateway = null;
        };
        hostGatewayJsonPresent = false;
      };
    };
  };

  "gateway-vm/realm-entrypoint-table-uses-custom-gateway-vm-name" = {
    expr = {
      declared = builtins.elem "corp-gateway" (builtins.attrNames customGatewayNameCfg.nixling.vms);
      work = customGatewayNameEntrypoints.entries.work;
    };
    expected = {
      declared = true;
      work = {
        mode = "gateway-backed";
        gateway = "corp-gateway.nixling";
      };
    };
  };

  "gateway-vm/transitional-host-relay-guard-defaults-off" = {
    expr = {
      inherit hostGatewayJsonPresent;
      guest = gatewayJson.allowHostRelayCredentials;
    };
    expected = {
      hostGatewayJsonPresent = false;
      guest = false;
    };
  };

  "gateway-vm/rejects-retired-host-relay-credential-escape-hatch" = {
    expr = lib.any
      (m: lib.hasInfix "allowHostRelayCredentials has been retired" m)
      retiredHostRelayCredentialMessages;
    expected = true;
  };

  "gateway-vm/accepts-multiple-gateways-with-separate-realms-and-envs" = {
    expr = {
      noAtMostOneFailure = !(lib.any
        (m: lib.hasInfix "at most one enabled gateway" m)
        multiGatewayMessages);
      workGatewayEnv = multiGatewayCfg.nixling.vms."sys-work-gateway".env;
      personalGatewayEnv = multiGatewayCfg.nixling.vms."sys-personal-gateway".env;
      legacySingleGatewayJson = builtins.hasAttr "nixling/gateway.json" multiGatewayCfg.environment.etc;
      entries = multiGatewayRealmEntrypoints.entries;
    };
    expected = {
      noAtMostOneFailure = true;
      workGatewayEnv = "work";
      personalGatewayEnv = "personal";
      legacySingleGatewayJson = false;
      entries = {
        local = {
          mode = "host-resident";
          gateway = null;
        };
        work = {
          mode = "gateway-backed";
          gateway = "sys-work-gateway.nixling";
        };
        personal = {
          mode = "gateway-backed";
          gateway = "sys-personal-gateway.nixling";
        };
      };
    };
  };

  "gateway-vm/rejects-duplicate-gateway-realm-entrypoints" = {
    expr = lib.any
      (m: lib.hasInfix "at most one gateway-backed realm" m
        && lib.hasInfix "work" m)
      duplicateGatewayRealmMessages;
    expected = true;
  };

  "gateway-vm/rejects-gateway-realms-sharing-env-l2" = {
    expr = lib.any
      (m: lib.hasInfix "must not place multiple gateway-backed realms on the" m
        && lib.hasInfix "work" m)
      sharedGatewayEnvMessages;
    expected = true;
  };

  "gateway-vm/rejects-gateway-entrypoint-for-local-realm" = {
    expr = lib.any
      (m: lib.hasInfix "may not declare realm `local`" m)
      localRealmGatewayMessages;
    expected = true;
  };

  "gateway-vm/source-host-tools-opt-out-selects-source-daemon" = {
    expr = lib.hasInfix "nixlingd-0.0.0-bootstrap"
      (toString sourceGatewayGuestCfg.systemd.services.nixlingd.serviceConfig.ExecStart);
    expected = true;
  };

  "gateway-vm/reuses-standard-host-tool-package-plumbing" = {
    expr = {
      noInlineBuildRustPackage = !(lib.hasInfix "buildRustPackage" gatewayModuleSource);
      noInlineBuildRustBin = !(lib.hasInfix "buildRustBin" gatewayModuleSource);
      noSystemPackagesScan = !(lib.hasInfix "config.environment.systemPackages" gatewayModuleSource);
      hasNixling = builtins.elem "nixling" packageNames;
      hasNixlingd = builtins.elem "nixlingd" packageNames;
      hasGatewayRuntime = builtins.elem "nixling-gateway-runtime" packageNames;
    };
    expected = {
      noInlineBuildRustPackage = true;
      noInlineBuildRustBin = true;
      noSystemPackagesScan = true;
      hasNixling = true;
      hasNixlingd = true;
      hasGatewayRuntime = true;
    };
  };
}
