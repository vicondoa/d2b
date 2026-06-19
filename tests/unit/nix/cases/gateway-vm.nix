# Eval coverage for realm gateway declarations.
{ lib, mkEval, ... }:

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
  badMessages = map (a: a.message) (lib.filter (a: !a.assertion) badCfg.assertions);
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
      (m: lib.hasInfix "nixling.gateways.work.credentialPath must be a runtime path" m)
      badMessages;
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

  "gateway-vm/host-daemon-stays-credential-free-facade" = {
    expr = {
      daemonConfigCarriesGateway = hostDaemonJson ? gateway;
      hostGateway = {
        gateway = hostGatewayJson.gateway;
        realm = hostGatewayJson.realm;
        credentialPath = hostGatewayJson.credentialPath;
        relayEntity = hostGatewayJson.relay.entity;
        waypipeSocket = hostGatewayJson.display.waypipeSocket;
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
      };
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
      (toString sourceToolsCfg.systemd.services.nixlingd.serviceConfig.ExecStart);
    expected = true;
  };
}
