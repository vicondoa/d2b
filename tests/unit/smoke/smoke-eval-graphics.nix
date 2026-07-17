{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
}:

let
  inherit (pkgs) lib;
  evaluated = lib.evalModules {
    modules = [
      ../../../nixos-modules/options-realms.nix
      ../../../nixos-modules/index.nix
      ../../../nixos-modules/realm-device-rows.nix
      ({ lib, ... }: {
        options.assertions = lib.mkOption {
          type = lib.types.listOf lib.types.attrs;
          default = [ ];
        };
        config.d2b.realms.work = {
          path = "work.local-root";
          providers.runtime = {
            type = "runtime";
            implementationId = "cloud-hypervisor";
          };
          providers.devices = {
            type = "device";
            implementationId = "host-mediated";
          };
          workloads.desktop = {
            provider = "runtime";
            launcher.capabilities = [ "gpu" "video" ];
          };
        };
      })
    ];
  };
  rows = evaluated.config.d2b._index.devices.list;
  requests =
    evaluated.config.d2b._index.devices.allocatorLeaseRequests;
  checks = [
    (if map (row: row.resourceKind) rows == [ "gpu" "render-node" "video" ]
      then null else throw "smoke-eval-graphics: incomplete graphics resource rows")
    (if lib.all (row: row.mediation.attachment == "fd-only") rows
      then null else throw "smoke-eval-graphics: a resource bypasses FD-only mediation")
    (if builtins.length requests == 1 && lib.all
      (request:
        request.resourceId == "device-render-node-global"
        && request.share == "shared-partition")
      requests
      then null else throw "smoke-eval-graphics: render-node leases are not shared allocator requests")
  ];
in
builtins.deepSeq checks
  (pkgs.runCommand "d2b-smoke-eval-realm-graphics" { } "touch $out")
