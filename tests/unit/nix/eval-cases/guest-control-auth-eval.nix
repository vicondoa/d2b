{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
, flake ? builtins.getFlake ("git+file://" + toString ./../../../..)
}:

let
  inherit (pkgs) lib;
  nixos = flake.inputs.nixpkgs.lib.nixosSystem {
    inherit system;
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        boot.loader.grub.enable = false;
        boot.loader.systemd-boot.enable = false;
        boot.initrd.includeDefaultModules = false;
        fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
        environment.etc."machine-id".text =
          "00000000000000000000000000000000";
        system.stateVersion = "25.11";
        users.users.alice = { isNormalUser = true; uid = 1000; };
        d2b.site = {
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
        };
        d2b.acceptDestructiveV2Cutover = true;
        d2b.realms.work = {
          path = "work";
          placement = "host-local";
          broker = {
            enable = true;
            hostMutation = true;
          };
          network = {
            mode = "declared";
            lanSubnet = "10.20.0.0/24";
            uplinkSubnet = "192.0.2.0/30";
          };
          providers.runtime = {
            type = "runtime";
            implementationId = "cloud-hypervisor";
          };
          workloads.corp = {
            providerRefs.runtime = "runtime";
            config = {
              d2b.sshUser = "alice";
              networking.hostName = lib.mkDefault "corp";
              users.users.alice = { isNormalUser = true; uid = 1000; };
            };
          };
        };
      })
    ];
  };
  cfg = nixos.config;
  workload = builtins.head cfg.d2b._index.workloads.enabledList;
  workloadRow = builtins.head
    (import ../../../../nixos-modules/workload-process-rows.nix {
      config = cfg;
      inherit lib pkgs;
    });
  guest = cfg.d2b._computedWorkloads.${workload.workloadId}.config;
  tokenShare = lib.findFirst
    (share: share.tag == "d2b-gctl")
    null
    workloadRow.shares;
  tokenRow = builtins.head cfg.d2b._workloadGuestControlRows;
  service = guest.systemd.services.d2b-guestd;
in
assert tokenShare != null;
assert tokenShare.source == "${workloadRow.keyRoot}/guest-control";
assert tokenShare.mountPoint == "/run/d2b-guest-control-host";
assert tokenShare.readOnly;
assert tokenRow.workloadId == workload.workloadId;
assert tokenRow.roleId
  == (builtins.head
    (builtins.filter
      (role: role.roleKind == "virtiofsd")
      workloadRow.roles)).roleId;
assert tokenRow.target == "${workloadRow.keyRoot}/guest-control/token";
assert tokenRow.source == "generated";
assert tokenRow.creator == "realm-broker";
assert tokenRow.repairOwner == "realm-broker";
assert tokenRow.materializedByHostActivation == false;
assert service.serviceConfig.LoadCredential
  == [ "guest_control_token:/run/d2b-guest-control-host/token" ];
assert builtins.elem "/run/d2b-guest-control-host"
  service.unitConfig.RequiresMountsFor;
{
  inherit (tokenRow) creator materializedByHostActivation repairOwner resourceRef;
  inherit (tokenShare) mountPoint readOnly;
  sourceIsCanonicalKeyRoot =
    tokenShare.source == "${workloadRow.keyRoot}/guest-control";
  workloadId = workload.workloadId;
}
