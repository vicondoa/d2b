{ mkEval, pkgs, system, flakeRoot, ... }:

let
  flakeShim = {
    inputs.nixpkgs.lib.nixosSystem = { modules, ... }: mkEval modules;
    nixosModules.default = { };
  };
  shell = scenario: import (flakeRoot + "/tests/unit/nix/eval-cases/guest-shell-policy-eval.nix") {
    inherit system pkgs scenario;
    flake = flakeShim;
  };
in
{
  "guest-shell-policy/enabled-positive" = {
    expr = shell "enabled";
    expected = "{\"defaultName\":\"default\",\"linger\":true,\"maxAttached\":1,\"maxSessions\":8,\"scenario\":\"enabled\"}";
  };

  "guest-shell-policy/defaults-positive" = {
    expr = shell "defaults";
    expected = "{\"defaultName\":\"default\",\"enabled\":false,\"scenario\":\"defaults\"}";
  };

  "guest-shell-policy/custom-positive" = {
    expr = shell "custom";
    expected = "{\"defaultName\":\"ops_1\",\"linger\":true,\"maxAttached\":2,\"maxSessions\":16,\"scenario\":\"custom\"}";
  };

  "guest-shell-policy/shell-no-control-rejected" = {
    expr = shell "shell-no-control";
    expectedError = { };
  };

  "guest-shell-policy/shell-no-exec-rejected" = {
    expr = shell "shell-no-exec";
    expectedError = { };
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

  "guest-shell-policy/shell-too-many-attached-rejected" = {
    expr = shell "shell-too-many-attached";
    expectedError = { };
  };

  "guest-shell-policy/shell-qemu-media-rejected" = {
    expr = shell "shell-qemu-media";
    expectedError = { };
  };
}
