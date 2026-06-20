# Eval coverage for realm gateway declarations.
{ lib, mkEval, flakeRoot, ... }:

let
  base = {
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
  gatewayGuestCfg = goodCfg.nixling._computed."sys-work-gateway".config;
  gatewayGuestService = gatewayGuestCfg.systemd.services.nixlingd.serviceConfig;
  gatewayGuestTmpfiles = gatewayGuestCfg.systemd.tmpfiles.rules;
  hostTmpfiles = goodCfg.systemd.tmpfiles.rules;
  gatewayJson = builtins.fromJSON gatewayGuestCfg.environment.etc."nixling/gateway.json".text;
  hostDaemonJson = builtins.fromJSON goodCfg.environment.etc."nixling/daemon-config.json".text;
  hostGatewayJson = builtins.fromJSON goodCfg.environment.etc."nixling/gateway.json".text;
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
        uplinkSubnet = "192.0.2.4/30";
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
      hasWaypipeSocket = false;
      hasWaypipeClient = false;
      hasWaypipeServer = false;
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
      guestLockFile = builtins.elem "f /run/nixling/daemon.lock 0640 nixlingd nixlingd -" gatewayGuestTmpfiles;
      gatewayUserCanReachPublicSocket = builtins.elem "nixling" gatewayGuestCfg.users.users.gateway.extraGroups;
    };
    expected = {
      hostStateDir = true;
      guestStateDir = true;
      guestDaemonStateDir = true;
      guestLockFile = true;
      gatewayUserCanReachPublicSocket = true;
    };
  };

  "gateway-vm/host-daemon-stays-credential-free-facade" = {
    expr = {
      daemonConfigCarriesGateway = hostDaemonJson ? gateway;
      hostGateway = {
        gateway = hostGatewayJson.gateway;
        realm = hostGatewayJson.realm;
        credentialPath = hostGatewayJson.credentialPath;
        relayEntity = hostGatewayJson.relay.entity;
        waypipeSocket = hostGatewayJson.display.waypipeSocket;
        allowHostRelayCredentials = hostGatewayJson.allowHostRelayCredentials;
      };
    };
    expected = {
      daemonConfigCarriesGateway = false;
      hostGateway = {
        gateway = "work";
        realm = "work";
        credentialPath = "/var/lib/nixling/gateways/work/credential.json";
        relayEntity = "hc-nixling-display";
        waypipeSocket = "/run/user/1000/wpc.sock";
        allowHostRelayCredentials = false;
      };
    };
  };

  "gateway-vm/transitional-host-relay-guard-defaults-off" = {
    expr = {
      host = hostGatewayJson.allowHostRelayCredentials;
      guest = gatewayJson.allowHostRelayCredentials;
    };
    expected = {
      host = false;
      guest = false;
    };
  };

  "gateway-vm/rejects-multiple-enabled-gateways" = {
    expr = lib.any
      (m: lib.hasInfix "at most one enabled gateway" m)
      multiGatewayMessages;
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
      hasGatewayRelaySourceBuild = builtins.elem "nixling-gateway-relay" packageNames;
    };
    expected = {
      noInlineBuildRustPackage = true;
      noInlineBuildRustBin = true;
      noSystemPackagesScan = true;
      hasNixling = true;
      hasNixlingd = true;
      hasGatewayRelaySourceBuild = false;
    };
  };
}
