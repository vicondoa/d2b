{
  flakeRoot,
}:

let
  shared = import ./shared.nix { inherit flakeRoot; };
  flake = builtins.getFlake (toString flakeRoot);
  lib = flake.inputs.nixpkgs.lib;
  system = shared.defaultSystem;

  nixos = flake.inputs.nixpkgs.lib.nixosSystem {
    inherit system;
    pkgs = shared.pkgsFor system;
    modules = [
      flake.nixosModules.default
      shared.baseModule
      ({ lib, ... }: {
        nixling.observability.enable = true;
        nixling.vms = lib.mkForce {
          personal-dev = {
            enable = true;
            env = "work";
            index = 10;
            ssh.user = "alice";
            graphics.enable = true;
            audio.enable = true;
            tpm.enable = true;
            observability.enable = true;
            config = {
              networking.hostName = lib.mkDefault "personal-dev";
              users.users.alice = {
                isNormalUser = true;
                uid = 1000;
              };
            };
          };
          work-entra = {
            enable = true;
            env = "work";
            index = 11;
            ssh.user = "alice";
            tpm.enable = true;
            observability.enable = true;
            config = {
              networking.hostName = lib.mkDefault "work-entra";
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

  dags = lib.listToAttrs (map (dag: lib.nameValuePair dag.vm dag) nixos.config.nixling._bundle.processesJson.data.vms);
  directDeps = dag: target:
    builtins.sort builtins.lessThan (
      map (edge: edge.from) (builtins.filter (edge: edge.to == target) dag.edges)
    );
  shareIds = dag:
    builtins.sort builtins.lessThan (
      map (node: node.id) (builtins.filter (node: node.role == "virtiofsd") dag.nodes)
    );

  personal = dags.personal-dev;
  work = dags.work-entra;
  personalShareIds = shareIds personal;
  workShareIds = shareIds work;
in
assert lib.assertMsg (personalShareIds != [ ]) "personal-dev should include virtiofsd share nodes";
assert lib.assertMsg (workShareIds != [ ]) "work-entra should include virtiofsd share nodes";
assert lib.assertMsg (directDeps personal "store-virtiofs-preflight" == [ "host-reconcile" ]) "personal-dev should start with host-reconcile -> store-virtiofs-preflight";
assert lib.assertMsg (builtins.all (shareId: directDeps personal shareId == [ "store-virtiofs-preflight" ]) personalShareIds) "personal-dev virtiofsd nodes should fan out from store-virtiofs-preflight";
assert lib.assertMsg (directDeps personal "swtpm-flush" == personalShareIds) "personal-dev should fan swtpm-flush in from every virtiofsd share";
assert lib.assertMsg (directDeps personal "vsock-relay" == [ "swtpm" ]) "personal-dev vsock-relay should wait for swtpm";
assert lib.assertMsg (directDeps personal "gpu" == [ "vsock-relay" ]) "personal-dev gpu should wait for vsock-relay";
assert lib.assertMsg (directDeps personal "audio" == [ "vsock-relay" ]) "personal-dev audio should wait for vsock-relay";
assert lib.assertMsg (directDeps personal "video" == [ "gpu" ]) "personal-dev video should wait for gpu";
assert lib.assertMsg (directDeps personal "cloud-hypervisor" == [ "audio" "video" ]) "personal-dev cloud-hypervisor should fan in from audio and video";
assert lib.assertMsg (directDeps work "store-virtiofs-preflight" == [ "host-reconcile" ]) "work-entra should start with host-reconcile -> store-virtiofs-preflight";
assert lib.assertMsg (builtins.all (shareId: directDeps work shareId == [ "store-virtiofs-preflight" ]) workShareIds) "work-entra virtiofsd nodes should fan out from store-virtiofs-preflight";
assert lib.assertMsg (directDeps work "swtpm-flush" == workShareIds) "work-entra should fan swtpm-flush in from every virtiofsd share";
assert lib.assertMsg (directDeps work "vsock-relay" == [ "swtpm" ]) "work-entra vsock-relay should wait for swtpm";
assert lib.assertMsg (directDeps work "cloud-hypervisor" == [ "vsock-relay" ]) "work-entra cloud-hypervisor should wait for vsock-relay when optional sidecars are absent";
{
  personalCloudHypervisorDeps = directDeps personal "cloud-hypervisor";
  workCloudHypervisorDeps = directDeps work "cloud-hypervisor";
  personalSwtpmFlushDeps = directDeps personal "swtpm-flush";
  workSwtpmFlushDeps = directDeps work "swtpm-flush";
}
