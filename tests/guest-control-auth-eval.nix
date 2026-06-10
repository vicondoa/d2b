{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
, flake ? builtins.getFlake ("git+file://" + toString ./..)
, guestControlEnable ? true
, tokenFile ? "/run/secrets/nixling/corp-vm-token"
}:

let
  inherit (pkgs) lib;
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
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

        nixling.envs.work = {
          lanSubnet = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };

        nixling.vms.corp-vm = {
          enable = true;
          env = "work";
          index = 10;
          ssh.user = "alice";
          guest.control = {
            enable = guestControlEnable;
            auth.tokenFile = tokenFile;
          };
          config = {
            networking.hostName = lib.mkDefault "corp-vm";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };
      })
    ];
  };
  guestConfig = nixos.config.nixling._computed.corp-vm.config;
  tokenShare = lib.findFirst (share: share.tag == "nl-gctl") null guestConfig.microvm.shares;
  service = guestConfig.systemd.services.nixling-guestd;
  serviceJson = builtins.toJSON {
    inherit (service) serviceConfig unitConfig;
  };
  processVm = lib.findFirst (vm: vm.vm == "corp-vm") null
    nixos.config.nixling._bundle.processesJson.data.vms;
  processNodes = processVm.nodes;
  tokenVirtiofsd = lib.findFirst (node: node.id == "virtiofsd-nl-gctl") null processNodes;
in
assert tokenShare != null;
assert tokenShare.source == "/var/lib/nixling/vms/corp-vm/guest-control";
assert tokenShare.mountPoint == "/run/nixling-guest-control-host";
assert tokenShare.readOnly == true;
assert builtins.elem "guest_control_token:/run/nixling-guest-control-host/token"
  service.serviceConfig.LoadCredential;
assert builtins.elem "/run/nixling-guest-control-host" service.unitConfig.RequiresMountsFor;
assert !(lib.hasInfix tokenFile serviceJson);
assert processVm != null;
assert tokenVirtiofsd != null;
assert builtins.elem "--readonly" tokenVirtiofsd.argv;
builtins.toJSON {
  inherit (tokenShare) source mountPoint readOnly;
  loadCredential = service.serviceConfig.LoadCredential;
}
