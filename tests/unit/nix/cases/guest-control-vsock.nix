{ mkEval, pkgs, system, flakeRoot, ... }:

let
  flakeShim = {
    inputs.nixpkgs.lib.nixosSystem = { modules, ... }: mkEval modules;
    nixosModules.default = { };
  };
  evidence = builtins.fromJSON
    (import
      (flakeRoot + "/tests/unit/nix/eval-cases/guest-control-vsock-eval.nix") {
        inherit system pkgs;
        flake = flakeShim;
        scenario = "base";
      });
in
{
  "guest-control-vsock/base-positive" = {
    expr = evidence;
    expected = {
      canonicalStateSocket = true;
      exactlyOneVsockArg = true;
      healthUsesWorkloadId = true;
      processWorkloadIdentity = true;
      workloadIdIsCanonical = true;
    };
  };
}
