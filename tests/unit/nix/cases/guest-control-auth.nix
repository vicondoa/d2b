{ mkEval, pkgs, system, flakeRoot, ... }:

let
  flakeShim = {
    inputs.nixpkgs.lib.nixosSystem = { modules, ... }: mkEval modules;
    nixosModules.default = { };
  };
  evidence = import
    (flakeRoot + "/tests/unit/nix/eval-cases/guest-control-auth-eval.nix") {
      inherit system pkgs;
      flake = flakeShim;
    };
in
{
  "guest-control-auth/realm-broker-owns-token" = {
    expr = {
      inherit (evidence) creator repairOwner materializedByHostActivation;
    };
    expected = {
      creator = "realm-broker";
      repairOwner = "realm-broker";
      materializedByHostActivation = false;
    };
  };
  "guest-control-auth/share-is-read-only" = {
    expr = {
      inherit (evidence) mountPoint readOnly sourceIsCanonicalKeyRoot;
    };
    expected = {
      mountPoint = "/run/d2b-guest-control-host";
      readOnly = true;
      sourceIsCanonicalKeyRoot = true;
    };
  };
  "guest-control-auth/resource-ref-is-workload-scoped" = {
    expr =
      builtins.match
        "workload/[a-z2-7]{20}/keys"
        evidence.resourceRef != null;
    expected = true;
  };
}
