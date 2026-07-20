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
        d2b.observability.enable = true;
        d2b.realms.work.workloads.dev = {
          providerRefs.runtime = "runtime";
          config = {
            networking.hostName = lib.mkDefault "dev";
            users.users.alice = {
              isNormalUser = true;
              uid = 1000;
            };
          };
        };
      })
    ];
  };

  identity = import (flakeRoot + "/nixos-modules/v2-identity.nix");
  obsRealmId = identity.deriveRealmId "local-root";
  obsWorkloadId = identity.deriveWorkloadId obsRealmId
    nixos.config.d2b.observability.vmName;
  obsBridgeRoleId =
    identity.deriveRoleId obsRealmId obsWorkloadId "vsock-relay";

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
  genericRelay = nodesByRole.vsock-relay;

  obsDag = builtins.head (builtins.filter
    (row: row.workloadIdentity.workloadId == obsWorkloadId)
    nixos.config.d2b._bundle.processesJson.data.vms);
  bridgeNode = lib.findFirst
    (node: node.id == obsBridgeRoleId)
    (throw "observability workload is missing its otel-host-bridge node")
    obsDag.nodes;
  bridgeProfile =
    nixos.config.d2b._bundle.minijailProfiles."role-${obsBridgeRoleId}".roleProfile;
  bridgeArgvJoined = lib.concatStringsSep " " bridgeNode.argv;
  obsEndpoints = nixos.config.d2b._realmObservability.endpoints;
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
assert lib.assertMsg (!(genericRelay ? binaryPath))
  "a non-observability workload's generic vsock-relay role must stay an argv-less placeholder";
assert lib.assertMsg (bridgeNode.role == "otel-host-bridge")
  "the observability workload's vsock-relay role must route to the otel-host-bridge process";
assert lib.assertMsg
  (lib.hasSuffix "/bin/socat" bridgeNode.binaryPath)
  "the otel-host-bridge process must exec the configured socat-compatible relay package";
assert lib.assertMsg
  (lib.hasInfix
    "UNIX-LISTEN:${obsEndpoints.hostEgress.path},fork,reuseaddr,mode=0660"
    bridgeArgvJoined)
  "the otel-host-bridge process must listen on the canonical host-egress socket";
assert lib.assertMsg
  (lib.hasInfix
    (builtins.unsafeDiscardStringContext
      ''EXEC:"${import (flakeRoot + "/nixos-modules/d2b-ch-vsock-connect.nix") { pkgs = nixos.pkgs; }}/bin/d2b-ch-vsock-connect ${obsEndpoints.stackVsock.path} ${toString obsEndpoints.stackVsock.port}"'')
    bridgeArgvJoined)
  "the otel-host-bridge process must exec d2b-ch-vsock-connect against the stack VM's CH vsock endpoint";
assert lib.assertMsg (bridgeProfile.seccompPolicyRef == "w1-otel-host-bridge")
  "the otel-host-bridge role must carry its own dedicated seccomp policy";
assert lib.assertMsg (bridgeProfile.mountPolicy.deviceBinds == [ ])
  "the otel-host-bridge role must not bind any host devices";
assert lib.assertMsg
  (builtins.length bridgeProfile.mountPolicy.writablePaths == 2)
  "the otel-host-bridge role must be writable only to its role-runtime socket and the workload's vsock state";
{
  workloadId = dag.vm;
  inherit (dag) workloadIdentity;
  shareNodeIds = map (node: node.id) shares;
  hypervisorDeps = directDeps hypervisor.id;
  otelHostBridgeArgv = bridgeNode.argv;
}
