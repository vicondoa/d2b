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
  configuredTokenFile =
    nixos.config.nixling.vms.corp-vm.guest.control.auth.tokenFile;
  tokenShare = lib.findFirst (share: share.tag == "nl-gctl") null guestConfig.microvm.shares;
  service = guestConfig.systemd.services.nixling-guestd;
  serviceJson = builtins.toJSON {
    inherit (service) serviceConfig unitConfig;
  };
  processVm = lib.findFirst (vm: vm.vm == "corp-vm") null
    nixos.config.nixling._bundle.processesJson.data.vms;
  processNodes = processVm.nodes;
  tokenVirtiofsd = lib.findFirst (node: node.id == "virtiofsd-nl-gctl") null processNodes;
  cloudHypervisor = lib.findFirst (node: node.id == "cloud-hypervisor") null processNodes;
  validTokenFile =
    lib.hasPrefix "/" tokenFile
    && tokenFile != "/nix/store"
    && !(lib.hasPrefix "/nix/store/" tokenFile);
  positive =
    assert tokenShare != null;
    assert configuredTokenFile == tokenFile;
    assert tokenShare.source == "/var/lib/nixling/guest-control-corp-vm";
    assert tokenShare.mountPoint == "/run/nixling-guest-control-host";
    assert tokenShare.readOnly == true;
    assert builtins.elem "guest_control_token:/run/nixling-guest-control-host/token"
      service.serviceConfig.LoadCredential;
    assert builtins.elem "/run/nixling-guest-control-host" service.unitConfig.RequiresMountsFor;
    assert service.wantedBy == [ ];
    assert lib.hasInfix "/bin/nixling-guestd --serve --vm-id corp-vm"
      service.serviceConfig.ExecStart;
    assert !(builtins.hasAttr "nixling-guestd" nixos.config.systemd.services);
    assert !(lib.hasInfix tokenFile serviceJson);
    assert processVm != null;
    assert tokenVirtiofsd != null;
    assert cloudHypervisor != null;
    assert lib.all (node: !(lib.hasInfix "guestd" node.id)) processNodes;
    # W15: a guest-control-capable VM emits the authenticated guest-control
    # Health readiness node (and never the retired SSH-readiness node).
    assert lib.any (node: node.id == "guest-control-health"
      && node.role == "guest-control-health"
      && node.readiness == [{ kind = "guest-control-health"; value = { vm = "corp-vm"; }; }]) processNodes;
    assert lib.all (node: node.id != "guest-ssh-readiness") processNodes;
    assert builtins.elem "--readonly" tokenVirtiofsd.argv;
    assert builtins.elem "--socket-path=/run/nixling/vms/corp-vm/guest-control/nl-gctl.sock"
      tokenVirtiofsd.argv;
    assert tokenVirtiofsd.profile.uid != cloudHypervisor.profile.uid;
    assert !(lib.hasInfix "/var/lib/nixling/vms/corp-vm"
      (builtins.toJSON tokenVirtiofsd.profile.mountPolicy.writablePaths));
    assert !(lib.hasInfix "\"path\":\"/run/nixling/vms/corp-vm\""
      (builtins.toJSON tokenVirtiofsd.profile.mountPolicy.writablePaths));
    assert lib.hasInfix "/run/nixling/vms/corp-vm/guest-control"
      (builtins.toJSON tokenVirtiofsd.profile.mountPolicy.writablePaths);
    builtins.toJSON {
      inherit (tokenShare) source mountPoint readOnly;
      loadCredential = service.serviceConfig.LoadCredential;
    };
in
if guestControlEnable && validTokenFile
then positive
else builtins.unsafeDiscardStringContext nixos.config.system.build.toplevel.drvPath
