{ flakeRoot }:

let
  shared = import ./shared.nix { inherit flakeRoot; };
  flake = builtins.getFlake "git+file://${toString flakeRoot}";
  lib = flake.inputs.nixpkgs.lib;
  system = shared.defaultSystem;

  nixos = flake.inputs.nixpkgs.lib.nixosSystem {
    inherit system;
    pkgs = shared.pkgsFor system;
    modules = [
      flake.nixosModules.default
      shared.baseModule
      ({ lib, ... }: {
        d2b.realms.local-root = {
          path = "local-root";
          placement = "host-local";
        };
        d2b.realms.work = {
          parent = "local-root";
          path = "work.local-root";
          placement = "host-local";
          allowedUsers = [ "alice" ];
          network = {
            mode = "declared";
            lanSubnet = "10.20.0.0/24";
            uplinkSubnet = "192.0.2.0/30";
          };
          providers.runtime = {
            type = "runtime";
            implementationId = "cloud-hypervisor";
          };
          workloads.dev = {
            provider = "runtime";
            config = {
              networking.hostName = lib.mkDefault "dev";
              users.users.alice = {
                isNormalUser = true;
                uid = 1000;
              };
            };
          };
        };
      })
    ];
  };

  dag = builtins.head (builtins.filter
    (row: row.workloadIdentity.canonicalTarget == "dev.work.local-root.d2b")
    nixos.config.d2b._bundle.processesJson.data.vms);
  nodesByRole = lib.listToAttrs (map
    (node: lib.nameValuePair node.role node)
    (builtins.filter (node: node.role != "virtiofsd") dag.nodes));
  shares = builtins.filter (node: node.role == "virtiofsd") dag.nodes;
  directDeps = target:
    builtins.sort builtins.lessThan
      (map (entry: entry.from)
        (builtins.filter (entry: entry.to == target) dag.edges));
  preflight = nodesByRole.store-virtiofs-preflight;
  hypervisor = nodesByRole.cloud-hypervisor-runner;
  guestHealth = nodesByRole.guest-control-health;
in
assert lib.assertMsg (shares != [ ])
  "canonical workload DAG should include virtiofsd share nodes";
assert lib.assertMsg
  (builtins.all
    (share: directDeps share.id == [ preflight.id ])
    shares)
  "all virtiofsd roles should follow the store/resource preflight";
assert lib.assertMsg
  (builtins.elem hypervisor.id (map (entry: entry.to) dag.edges))
  "the workload runner should have prerequisite edges";
assert lib.assertMsg (directDeps guestHealth.id == [ hypervisor.id ])
  "guest-control health should follow the workload runner";
assert lib.assertMsg
  (builtins.all (node: !(node ? unit)) dag.nodes)
  "realm-controller workload roles must not reference systemd units";
assert lib.assertMsg
  (builtins.all
    (node:
      lib.hasPrefix
        "d2b.slice/r-${dag.workloadIdentity.realmId}/workloads/w-${dag.vm}/"
        node.profile.cgroupPlacement.subtree)
    dag.nodes)
  "every workload process must be placed directly in its canonical role leaf";
{
  workloadId = dag.vm;
  inherit (dag) workloadIdentity;
  shareNodeIds = map (node: node.id) shares;
  hypervisorDeps = directDeps hypervisor.id;
}
