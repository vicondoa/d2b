{ mkEval, pkgs, system, flakeRoot, ... }:

let
  flakeShim = {
    inputs.nixpkgs.lib.nixosSystem = { modules, ... }: mkEval modules;
    nixosModules.default = { };
  };
  shell = scenario:
    builtins.fromJSON
      (import
        (flakeRoot + "/tests/unit/nix/eval-cases/guest-shell-policy-eval.nix") {
          inherit system pkgs scenario;
          flake = flakeShim;
        });
in
{
  "guest-shell-policy/enabled-positive" = {
    expr = {
      inherit (shell "enabled")
        controlEnable defaultName execEnable maxAttached maxSessions
        processWorkloadIdentity shellEnable;
    };
    expected = {
      controlEnable = true;
      defaultName = "default";
      execEnable = true;
      maxAttached = 4;
      maxSessions = 8;
      processWorkloadIdentity = true;
      shellEnable = true;
    };
  };
  "guest-shell-policy/defaults-positive" = {
    expr = {
      inherit (shell "defaults") controlEnable execEnable shellEnable;
    };
    expected = {
      controlEnable = true;
      execEnable = false;
      shellEnable = false;
    };
  };
  "guest-shell-policy/custom-positive" = {
    expr = {
      inherit (shell "custom") defaultName maxAttached maxSessions;
    };
    expected = {
      defaultName = "ops_1";
      maxAttached = 4;
      maxSessions = 16;
    };
  };
  "guest-shell-policy/control-is-host-forced" = {
    expr = {
      inherit (shell "shell-no-control") controlEnable shellEnable;
    };
    expected = { controlEnable = true; shellEnable = true; };
  };
  "guest-shell-policy/shell-implies-exec" = {
    expr = {
      inherit (shell "shell-no-exec") execEnable shellEnable;
    };
    expected = { execEnable = true; shellEnable = true; };
  };
  "guest-shell-policy/shell-no-user-rejected" = {
    expr = shell "shell-no-user";
    expectedError = { };
  };
  "guest-shell-policy/shell-root-user-rejected" = {
    expr = shell "shell-root-user";
    expectedError = { };
  };
  "guest-shell-policy/shell-invalid-name-rejected" = {
    expr = shell "shell-invalid-name";
    expectedError = { };
  };
  "guest-shell-policy/shell-session-limit-rejected" = {
    expr = shell "shell-too-many-attached";
    expectedError = { };
  };
  "guest-shell-policy/qemu-process-is-realm-owned" = {
    expr = {
      inherit (shell "shell-qemu-media")
        computedGuest materializedSystemdUnit runtimeKind;
    };
    expected = {
      computedGuest = false;
      materializedSystemdUnit = false;
      runtimeKind = "qemu-media";
    };
  };
}
