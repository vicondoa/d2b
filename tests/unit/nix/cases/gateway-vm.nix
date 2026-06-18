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
  gatewayProc = lib.findFirst (vm: vm.vm == "sys-work-gateway") null
    goodCfg.nixling._bundle.processesJson.data.vms;
  nodeById = id: lib.findFirst (n: n.id == id) null gatewayProc.nodes;
  clientNode = nodeById "gateway-waypipe-client";
  serverNode = nodeById "gateway-waypipe-server";
  clientProfile = goodCfg.nixling._bundle.minijailProfiles."vm-sys-work-gateway-gateway-waypipe-client".data;
  serverProfile = goodCfg.nixling._bundle.minijailProfiles."vm-sys-work-gateway-gateway-waypipe-server".data;
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

  "gateway-vm/emits-waypipe-runner-nodes" = {
    expr = {
      clientRole = clientNode.role;
      serverRole = serverNode.role;
      clientHasNoGpu = builtins.elem "--no-gpu" clientNode.argv;
      serverHasNoGpu = builtins.elem "--no-gpu" serverNode.argv;
      clientUnit = clientNode ? unit;
      serverUnit = serverNode ? unit;
    };
    expected = {
      clientRole = "gateway-waypipe-client";
      serverRole = "gateway-waypipe-server";
      clientHasNoGpu = true;
      serverHasNoGpu = true;
      clientUnit = false;
      serverUnit = false;
    };
  };

  "gateway-vm/waypipe-profiles-are-jailed-and-device-empty" = {
    expr = {
      clientCaps = clientProfile.capabilities;
      serverCaps = serverProfile.capabilities;
      clientDevices = clientProfile.mountPolicy.deviceBinds;
      serverDevices = serverProfile.mountPolicy.deviceBinds;
      clientPrincipal = clientProfile.principal;
      serverPrincipal = serverProfile.principal;
      clientSeccomp = clientProfile.seccompPolicyRef;
      serverSeccomp = serverProfile.seccompPolicyRef;
    };
    expected = {
      clientCaps = [ ];
      serverCaps = [ ];
      clientDevices = [ ];
      serverDevices = [ ];
      clientPrincipal = "nixling-sys-work-gateway-gw-wp-client";
      serverPrincipal = "nixling-sys-work-gateway-gw-wp-server";
      clientSeccomp = "w1-gateway-waypipe-client";
      serverSeccomp = "w1-gateway-waypipe-server";
    };
  };
}
