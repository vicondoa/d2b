{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
, flake ? builtins.getFlake ("git+file://" + toString ./../../../..)
, scenario ? "base"
}:

let
  inherit (pkgs) lib;
  d2bLib = import ../../../../nixos-modules/lib.nix { inherit lib; };
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;

  scenarioModule =
    if scenario == "user-vsock-cid" then
      { ... }: { d2b.vms.alpha-vm.config.microvm.vsock.cid = 42; }
    else if scenario == "user-vsock-socket" then
      { ... }: { d2b.vms.alpha-vm.config.microvm.vsock.socket = "/tmp/user.sock"; }
    else if scenario == "user-vsock-extra-split" then
      { ... }: {
        d2b.vms.alpha-vm.config.microvm.cloud-hypervisor.extraArgs = [
          "--vsock"
          "socket=/tmp/user.sock"
        ];
      }
    else if scenario == "user-vsock-extra-equals" then
      { ... }: {
        d2b.vms.alpha-vm.config.microvm.cloud-hypervisor.extraArgs = [
          "--vsock=cid=42,socket=/tmp/user.sock"
        ];
      }
    else if scenario == "long-socket" then
      { lib, ... }: {
        d2b.vms."aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" = {
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

        d2b.site = {
          stateDir = lib.mkForce "/var/lib/d2b";
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
        };

        d2b.observability.enable = true;

        d2b.envs.alpha = {
          lanSubnet = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };
        d2b.envs.beta = {
          lanSubnet = "10.21.0.0/24";
          uplinkSubnet = "198.51.100.0/30";
        };

        d2b.vms.alpha-vm = {
          enable = true;
          env = "alpha";
          index = 10;
          ssh.user = "alice";
          guest.control.enable = true;
          observability.enable = true;
          config = {
            networking.hostName = lib.mkDefault "alpha-vm";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };
        d2b.vms.alpha-high = {
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
        d2b.vms.beta-vm = {
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
        d2b.vms.legacy-vm = {
          enable = true;
          ssh.user = null;
          config = {
            networking.hostName = lib.mkDefault "legacy-vm";
          };
        };
        d2b.vms.disabled-vm = {
          enable = false;
          env = "alpha";
          index = 12;
          config.networking.hostName = lib.mkDefault "disabled-vm";
        };
      })
      scenarioModule
    ];
  };

  manifest = nixos.config.d2b.manifest;
  processRows = nixos.config.d2b._bundle.processesJson.data.vms;
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
    assert nixos.config.d2b._computed.${name}.config.microvm.vsock.cid
      == manifest.${name}.observability.vsockCid;
    assert nixos.config.d2b._computed.${name}.config.microvm.vsock.socket
      == manifest.${name}.observability.vsockHostSocket;
    true;
  assertChVsock = name:
    assert assertManifestVsock name;
    assert vsockValues (chArgv name) == [ (expectedVsockArg name) ];
    true;
  assertStateDirTmpfile = name:
    assert builtins.elem "d /var/lib/d2b/vms/${name} 3770 d2bd users -" tmpfilesRules;
    assert builtins.elem "z /var/lib/d2b/vms/${name} 3770 d2bd users -" tmpfilesRules;
    true;
  alphaReadiness = processNode "alpha-vm" "guest-control-health";
  legacyExpectedCid = d2bLib.guestControlVsockCid {
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
    assert assertChVsock "sys-obs";
    assert assertChVsock "legacy-vm";
    assert assertStateDirTmpfile "alpha-vm";
    assert assertStateDirTmpfile "alpha-high";
    assert assertStateDirTmpfile "beta-vm";
    assert assertStateDirTmpfile "sys-alpha-net";
    assert assertStateDirTmpfile "sys-beta-net";
    assert assertStateDirTmpfile "sys-obs";
    assert assertStateDirTmpfile "legacy-vm";
    assert manifest.alpha-vm.observability.vsockCid == 110;
    assert manifest.alpha-high.observability.vsockCid == 210;
    assert manifest.beta-vm.observability.vsockCid == 1110;
    assert manifest.sys-alpha-net.observability.vsockCid == 101;
    assert manifest.sys-beta-net.observability.vsockCid == 1101;
    assert manifest.sys-obs.observability.vsockCid == 1000;
    assert manifest.legacy-vm.observability.vsockCid == legacyExpectedCid;
    assert !(builtins.hasAttr "disabled-vm" manifest);
    # The observability vsock-relay transport argv is validated by the
    # SigNoz observability eval suite (observability-eval / tempo / loki),
    # not here: this eval owns guest-control vsock allocation and dormancy.
    # W15: framework readiness on a guest-control-capable VM is the
    # authenticated guest-control Health probe, not a raw TCP-22 SSH probe.
    assert alphaReadiness != null;
    assert alphaReadiness.role == "guest-control-health";
    assert alphaReadiness.readiness == [
      {
        kind = "guest-control-health";
        value = {
          vm = "alpha-vm";
        };
      }
    ];
    # The retired SSH-readiness node is no longer emitted as framework readiness.
    assert !(builtins.hasAttr "guest-ssh-readiness" (builtins.listToAttrs (map (node: { name = node.id; value = true; }) (processVm "alpha-vm").nodes)));
    builtins.toJSON {
      inherit (manifest.alpha-vm.observability) vsockCid vsockHostSocket;
      alphaArgv = vsockValues (chArgv "alpha-vm");
      betaCid = manifest.beta-vm.observability.vsockCid;
      netCid = manifest.sys-alpha-net.observability.vsockCid;
      obsCid = manifest.sys-obs.observability.vsockCid;
      legacyCid = manifest.legacy-vm.observability.vsockCid;
    };
in
if scenario == "base" then
  positive
else
  builtins.seq
    (builtins.unsafeDiscardStringContext nixos.config.system.build.toplevel.drvPath)
    (builtins.unsafeDiscardStringContext nixos.config.d2b._bundle.processesJson.path)
