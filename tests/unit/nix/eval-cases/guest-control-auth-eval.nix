{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
, flake ? builtins.getFlake ("git+file://" + toString ./../../../..)
, guestControlEnable ? true
, tokenFile ? "/run/secrets/d2b/corp-vm-token"
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

        d2b.site = {
          stateDir = lib.mkForce "/var/lib/d2b";
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
        };

        d2b.envs.work = {
          lanSubnet = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };

        d2b.vms.corp-vm = {
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
  guestConfig = nixos.config.d2b._computed.corp-vm.config;
  configuredTokenFile =
    nixos.config.d2b.vms.corp-vm.guest.control.auth.tokenFile;
  tokenShare = lib.findFirst (share: share.tag == "d2b-gctl") null guestConfig.microvm.shares;
  service = guestConfig.systemd.services.d2b-guestd;
  serviceJson = builtins.toJSON {
    inherit (service) serviceConfig unitConfig;
  };
  processVm = lib.findFirst (vm: vm.vm == "corp-vm") null
    nixos.config.d2b._bundle.processesJson.data.vms;
  processNodes = processVm.nodes;
  tokenVirtiofsd = lib.findFirst (node: node.id == "virtiofsd-d2b-gctl") null processNodes;
  cloudHypervisor = lib.findFirst (node: node.id == "cloud-hypervisor") null processNodes;
  validTokenFile =
    lib.hasPrefix "/" tokenFile
    && tokenFile != "/nix/store"
    && !(lib.hasPrefix "/nix/store/" tokenFile);
  positive =
    assert tokenShare != null;
    assert configuredTokenFile == tokenFile;
    assert tokenShare.source == "/var/lib/d2b/guest-control-corp-vm";
    assert tokenShare.mountPoint == "/run/d2b-guest-control-host";
    assert tokenShare.readOnly == true;
    assert builtins.elem "guest_control_token:/run/d2b-guest-control-host/token"
      service.serviceConfig.LoadCredential;
    assert builtins.elem "/run/d2b-guest-control-host" service.unitConfig.RequiresMountsFor;
    assert service.wantedBy == [ "multi-user.target" ];
    assert service.restartIfChanged == false;
    assert lib.hasInfix "/bin/d2b-guestd --serve --vm-id corp-vm"
      service.serviceConfig.ExecStart;
    assert lib.hasInfix "--activation-systemd-run-path" service.serviceConfig.ExecStart;
    assert lib.hasInfix "--activation-systemctl-path" service.serviceConfig.ExecStart;
    assert builtins.elem "d /run/d2b-guestd 0700 root root -" guestConfig.systemd.tmpfiles.rules;
    assert builtins.elem "d /run/d2b-guestd/activations 0700 root root -" guestConfig.systemd.tmpfiles.rules;
    assert !(builtins.hasAttr "d2b-guestd" nixos.config.systemd.services);
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
    assert builtins.elem "--socket-path=/run/d2b/vms/corp-vm/guest-control/d2b-gctl.sock"
      tokenVirtiofsd.argv;
    assert tokenVirtiofsd.profile.uid != cloudHypervisor.profile.uid;
    assert !(lib.hasInfix "/var/lib/d2b/vms/corp-vm"
      (builtins.toJSON tokenVirtiofsd.profile.mountPolicy.writablePaths));
    assert !(lib.hasInfix "\"path\":\"/run/d2b/vms/corp-vm\""
      (builtins.toJSON tokenVirtiofsd.profile.mountPolicy.writablePaths));
    assert lib.hasInfix "/run/d2b/vms/corp-vm/guest-control"
      (builtins.toJSON tokenVirtiofsd.profile.mountPolicy.writablePaths);
    builtins.toJSON {
      inherit (tokenShare) source mountPoint readOnly;
      loadCredential = service.serviceConfig.LoadCredential;
    };
in
if guestControlEnable && validTokenFile
then positive
else builtins.unsafeDiscardStringContext nixos.config.system.build.toplevel.drvPath
