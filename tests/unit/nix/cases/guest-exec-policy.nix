{ mkEval, pkgs, system, flakeRoot, ... }:

let
  flakeShim = {
    inputs.nixpkgs.lib.nixosSystem = { modules, ... }: mkEval modules;
    nixosModules.default = { };
  };
  exec = scenario:
    builtins.fromJSON
      (import
        (flakeRoot + "/tests/unit/nix/eval-cases/guest-exec-policy-eval.nix") {
          inherit system pkgs scenario;
          flake = flakeShim;
        });
in
{
  "guest-exec-policy/enabled-positive" = {
    expr = {
      inherit (exec "enabled")
        controlEnable execEnable execUser noPerWorkloadUnits
        processWorkloadIdentity;
    };
    expected = {
      controlEnable = true;
      execEnable = true;
      execUser = "alice";
      noPerWorkloadUnits = true;
      processWorkloadIdentity = true;
    };
  };
  "guest-exec-policy/default-positive" = {
    expr = (exec "default").execEnable;
    expected = false;
  };
  "guest-exec-policy/control-no-exec-positive" = {
    expr = {
      inherit (exec "control-no-exec") controlEnable execEnable;
    };
    expected = { controlEnable = true; execEnable = false; };
  };
  "guest-exec-policy/runtime-ceilings-fixed" = {
    expr = {
      inherit (exec "detached-ceiling")
        detachedMaxRuntimeSec interactiveMaxRuntimeSec;
    };
    expected = {
      detachedMaxRuntimeSec = 0;
      interactiveMaxRuntimeSec = 0;
    };
  };
  "guest-exec-policy/control-is-host-forced" = {
    expr = {
      inherit (exec "exec-no-control") controlEnable execEnable;
    };
    expected = { controlEnable = true; execEnable = true; };
  };
  "guest-exec-policy/exec-no-user-rejected" = {
    expr = exec "exec-no-user";
    expectedError = { };
  };
  "guest-exec-policy/root-user-rejected" = {
    expr = exec "root-user";
    expectedError = { };
  };
  "guest-exec-policy/invalid-user-rejected" = {
    expr = exec "invalid-user";
    expectedError = { };
  };
  "guest-exec-policy/missing-user-rejected" = {
    expr = exec "missing-user";
    expectedError = { };
  };
  "guest-exec-policy/uid-zero-alias-rejected" = {
    expr = exec "uid-zero-alias";
    expectedError = { };
  };
}
