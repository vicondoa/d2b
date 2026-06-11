{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
, flake ? builtins.getFlake ("git+file://" + toString ./..)
, scenario ? "base"
}:

let
  inherit (pkgs) lib;
  nl = import ../nixos-modules/lib.nix { inherit lib; };
  chVsockConnect = import ../nixos-modules/nixling-ch-vsock-connect.nix { inherit pkgs; };
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;

  scenarioModule =
    if scenario == "user-vsock-cid" then
      { ... }: { nixling.vms.alpha-vm.config.microvm.vsock.cid = 42; }
    else if scenario == "user-vsock-socket" then
      { ... }: { nixling.vms.alpha-vm.config.microvm.vsock.socket = "/tmp/user.sock"; }
    else if scenario == "user-vsock-extra-split" then
      { ... }: {
        nixling.vms.alpha-vm.config.microvm.cloud-hypervisor.extraArgs = [
          "--vsock"
          "socket=/tmp/user.sock"
        ];
      }
    else if scenario == "user-vsock-extra-equals" then
      { ... }: {
        nixling.vms.alpha-vm.config.microvm.cloud-hypervisor.extraArgs = [
          "--vsock=cid=42,socket=/tmp/user.sock"
        ];
      }
    else if scenario == "long-socket" then
      { lib, ... }: {
        nixling.vms."aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" = {
          enable = true;
          env = "alpha";
          index = 12;
          ssh.user = "alice";
          config = {
            networking.hostName = lib.mkDefault "long-vsock-name";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };
      }
    else
      { ... }: { };

  nixos = nixosSystem {
    inherit system;
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        boot.loader.grub.enable = false;
        boot.loader.systemd-boot.enable = false;
        boot.initrd.includeDefaultModules = false;
        fileSystems."/" = {
          device = "tmpfs";
          fsType = "tmpfs";
        };
        environment.etc."machine-id".text = "00000000000000000000000000000000";
        system.stateVersion = "25.11";

        users.users.alice = {
          isNormalUser = true;
          uid = 1000;
        };

        nixling.site = {
          stateDir = lib.mkForce "/var/lib/nixling";
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
        };

        nixling.observability.enable = true;

        nixling.envs.alpha = {
          lanSubnet = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };
        nixling.envs.beta = {
          lanSubnet = "10.21.0.0/24";
          uplinkSubnet = "198.51.100.0/30";
        };

        nixling.vms.alpha-vm = {
          enable = true;
          env = "alpha";
          index = 10;
          ssh.user = "alice";
          observability.enable = true;
          config = {
            networking.hostName = lib.mkDefault "alpha-vm";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };
        nixling.vms.alpha-high = {
          enable = true;
          env = "alpha";
          index = 110;
          ssh.user = "alice";
          config = {
            networking.hostName = lib.mkDefault "alpha-high";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };
        nixling.vms.beta-vm = {
          enable = true;
          env = "beta";
          index = 10;
          ssh.user = "alice";
          config = {
            networking.hostName = lib.mkDefault "beta-vm";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };
        nixling.vms.legacy-vm = {
          enable = true;
          ssh.user = null;
          config = {
            networking.hostName = lib.mkDefault "legacy-vm";
          };
        };
        nixling.vms.disabled-vm = {
          enable = false;
          env = "alpha";
          index = 12;
          config.networking.hostName = lib.mkDefault "disabled-vm";
        };
      })
      scenarioModule
    ];
  };

  manifest = nixos.config.nixling.manifest;
  processRows = nixos.config.nixling._bundle.processesJson.data.vms;
  tmpfilesRules = nixos.config.systemd.tmpfiles.rules;
  processVm = name: lib.findFirst (vm: vm.vm == name) null processRows;
  processNode = name: nodeId:
    lib.findFirst (node: node.id == nodeId) null (processVm name).nodes;
  chArgv = name: (processNode name "cloud-hypervisor").argv;
  vsockValues = argv:
    let
      go = args:
        if args == [ ] then [ ]
        else if builtins.head args == "--vsock" && builtins.length args > 1 then
          [ (builtins.elemAt args 1) ] ++ go (builtins.tail (builtins.tail args))
        else
          go (builtins.tail args);
    in
    go argv;
  expectedVsockArg = name:
    "cid=${toString manifest.${name}.observability.vsockCid},socket=${manifest.${name}.observability.vsockHostSocket}";
  assertManifestVsock = name:
    assert nixos.config.nixling._computed.${name}.config.microvm.vsock.cid
      == manifest.${name}.observability.vsockCid;
    assert nixos.config.nixling._computed.${name}.config.microvm.vsock.socket
      == manifest.${name}.observability.vsockHostSocket;
    true;
  assertChVsock = name:
    assert assertManifestVsock name;
    assert vsockValues (chArgv name) == [ (expectedVsockArg name) ];
    true;
  assertStateDirTmpfile = name:
    assert builtins.elem "d /var/lib/nixling/vms/${name} 2770 microvm kvm -" tmpfilesRules;
    true;
  alphaRelay = processNode "alpha-vm" "vsock-relay";
  alphaReadiness = processNode "alpha-vm" "guest-ssh-readiness";
  allProcessesJson = builtins.toJSON processRows;
  legacyExpectedCid = nl.guestControlVsockCid {
    name = "legacy-vm";
    envIndex = null;
    index = null;
    isNetVm = false;
    isObservabilityVm = false;
  };

  positive =
    assert assertChVsock "alpha-vm";
    assert assertChVsock "alpha-high";
    assert assertChVsock "beta-vm";
    assert assertChVsock "sys-alpha-net";
    assert assertChVsock "sys-beta-net";
    assert assertChVsock "sys-obs-stack";
    assert assertChVsock "legacy-vm";
    assert assertStateDirTmpfile "alpha-vm";
    assert assertStateDirTmpfile "alpha-high";
    assert assertStateDirTmpfile "beta-vm";
    assert assertStateDirTmpfile "sys-alpha-net";
    assert assertStateDirTmpfile "sys-beta-net";
    assert assertStateDirTmpfile "sys-obs-stack";
    assert assertStateDirTmpfile "legacy-vm";
    assert manifest.alpha-vm.observability.vsockCid == 110;
    assert manifest.alpha-high.observability.vsockCid == 210;
    assert manifest.beta-vm.observability.vsockCid == 1110;
    assert manifest.sys-alpha-net.observability.vsockCid == 101;
    assert manifest.sys-beta-net.observability.vsockCid == 1101;
    assert manifest.sys-obs-stack.observability.vsockCid == 1000;
    assert manifest.legacy-vm.observability.vsockCid == legacyExpectedCid;
    assert !(builtins.hasAttr "disabled-vm" manifest);
    assert builtins.elem
      "UNIX-LISTEN:/var/lib/nixling/vms/alpha-vm/vsock.sock_14317,fork,max-children=16,reuseaddr,mode=0660"
      alphaRelay.argv;
    assert builtins.elem
      "EXEC:${chVsockConnect}/bin/nixling-ch-vsock-connect /var/lib/nixling/vms/sys-obs-stack/vsock.sock 14317"
      alphaRelay.argv;
    assert !(lib.hasInfix "14318" allProcessesJson);
    assert alphaReadiness != null;
    assert alphaReadiness.role == "guest-ssh-readiness";
    assert alphaReadiness.readiness == [
      {
        kind = "tcp-port";
        value = {
          host = "10.20.0.10";
          port = 22;
        };
      }
    ];
    assert !(builtins.hasAttr "guest-control-health" (builtins.listToAttrs (map (node: { name = node.id; value = true; }) (processVm "alpha-vm").nodes)));
    builtins.toJSON {
      inherit (manifest.alpha-vm.observability) vsockCid vsockHostSocket;
      alphaArgv = vsockValues (chArgv "alpha-vm");
      betaCid = manifest.beta-vm.observability.vsockCid;
      netCid = manifest.sys-alpha-net.observability.vsockCid;
      obsCid = manifest.sys-obs-stack.observability.vsockCid;
      legacyCid = manifest.legacy-vm.observability.vsockCid;
    };
in
if scenario == "base" then
  positive
else
  builtins.seq
    (builtins.unsafeDiscardStringContext nixos.config.system.build.toplevel.drvPath)
    (builtins.unsafeDiscardStringContext nixos.config.nixling._bundle.processesJson.path)
