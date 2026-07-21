{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
, flake ? builtins.getFlake ("git+file://" + toString ./../../../..)
, scenario ? "base"
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
  processDag = builtins.head cfg.d2b._bundle.processesJson.data.vms;
  cloud = builtins.head
    (builtins.filter
      (node: node.role == "cloud-hypervisor-runner")
      processDag.nodes);
  health = builtins.head
    (builtins.filter
      (node: node.role == "guest-control-health")
      processDag.nodes);
  vsockArgs =
    let
      collect = args:
        if args == [ ] then [ ]
        else if builtins.head args == "--vsock"
        then [ (builtins.elemAt args 1) ]
          ++ collect (builtins.tail (builtins.tail args))
        else collect (builtins.tail args);
    in
    collect cloud.argv;
  expectedSocket = "${workloadRow.stateRoot}/vsock.sock";
in
assert scenario == "base";
assert processDag.vm == workload.workloadId;
assert processDag.workloadIdentity.workloadId == workload.workloadId;
assert cfg.d2b._computedWorkloads.${workload.workloadId}
  .config.microvm.vsock.socket == expectedSocket;
assert builtins.length vsockArgs == 1;
assert lib.hasInfix "socket=${expectedSocket}" (builtins.head vsockArgs);
assert health.readiness == [{
  kind = "guest-control-health";
  value.vm = workload.workloadId;
}];
builtins.toJSON {
  canonicalStateSocket = true;
  exactlyOneVsockArg = true;
  healthUsesWorkloadId = true;
  processWorkloadIdentity = true;
  workloadIdIsCanonical = workload.workloadId != workload.workloadName;
}
