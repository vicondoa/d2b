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
      aca.endpoint = "https://example.azurecontainerapps.io";
    };
  };

  goodCfg = (mkEval [ base ]).config;
  gatewayGuestCfg = goodCfg.nixling._computed."sys-work-gateway".config;
  gatewayProc = lib.findFirst (vm: vm.vm == "sys-work-gateway") null
    goodCfg.nixling._bundle.processesJson.data.vms;
  badCfg = (mkEval [
    (lib.recursiveUpdate base {
      nixling.gateways.work.credentialPath = "SharedAccessKey=bad";
    })
  ]).config;
  badMessages = map (a: a.message) (lib.filter (a: !a.assertion) badCfg.assertions);
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
      hasWaypipeClient = builtins.hasAttr "nixling-gateway-waypipe-client" gatewayGuestCfg.systemd.services;
      hasWaypipeServer = builtins.hasAttr "nixling-gateway-waypipe-server" gatewayGuestCfg.systemd.services;
    };
    expected = {
      hasNixlingd = true;
      gatewayJson = true;
      daemonJson = true;
      hasWaypipeClient = false;
      hasWaypipeServer = false;
    };
  };
}
