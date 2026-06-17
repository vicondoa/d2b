# nix-unit cases migrated from tests/guest-exec-policy-eval.sh.
#
# Guest exec policy invariants for the workload-user-only exec model: the
# `guest.exec` defaults; that exec stays OFF unless `guest.exec.enable` is set
# even when guest-control is up (`control-no-exec`); the detached / interactive
# exec surface (slice + run-dir + guestd `--systemd-run-path` /
# `--exec-runner-path` / `--detached-max-runtime-sec` /
# `--interactive-max-runtime-sec` flags) and runtime-ceiling propagation; and
# the policy assertions that reject exec-without-control, a missing workload
# user, a root workload user (by name and by UID-0 alias), an invalid user
# name, and a user not declared on the guest.
#
# Reuses the evidence module tests/unit/nix/eval-cases/guest-exec-policy-eval.nix (which
# flake.checks.<sys>.guest-exec-policy already builds for the positive
# "enabled" scenario) by importing it with a synthetic `flake` shim whose
# `inputs.nixpkgs.lib.nixosSystem` routes through the harness `mkEval` (==
# nixosSystem with the nixling module set). This keeps the eval 100% faithful
# to the retired bash gate's `nix eval` of each scenario while staying hermetic
# (no `builtins.getFlake`). Each positive scenario asserts the exact evidence
# JSON; each policy-violation scenario asserts the eval THROWS (the bash gate
# additionally matched the assertion message — that precise text is not
# capturable by the harness's `expectedError` bucket, so it relaxes to a throw
# assertion, the same faithful reduction the other throw-case migrations took).
#
# Graphics-free fixture (corp-vm + side-vm, guest control/exec only), so no
# aarch64 platform guard is required; the existing
# flake.checks.aarch64-linux.guest-exec-policy proves the "enabled" eval is
# arch-portable.
{ mkEval, pkgs, system, flakeRoot, ... }:

let
  flakeShim = {
    inputs.nixpkgs.lib.nixosSystem = { modules, ... }: mkEval modules;
    nixosModules.default = { };
  };
  exec = scenario: import (flakeRoot + "/tests/unit/nix/eval-cases/guest-exec-policy-eval.nix") {
    inherit system pkgs scenario;
    flake = flakeShim;
  };
in
{
  # --- positive scenarios: exact evidence JSON ----------------------
  "guest-exec-policy/enabled-positive" = {
    expr = exec "enabled";
    expected = "{\"execUser\":\"alice\",\"scenario\":\"enabled\"}";
  };
  "guest-exec-policy/default-positive" = {
    expr = exec "default";
    expected = "{\"execUser\":\"alice\",\"scenario\":\"default\"}";
  };
  # Guest control up, `guest.exec` block omitted: exec stays OFF and no exec
  # wiring leaks into the guestd ExecStart.
  "guest-exec-policy/control-no-exec-positive" = {
    expr = exec "control-no-exec";
    expected = "{\"controlEnable\":true,\"execEnable\":false,\"scenario\":\"control-no-exec\"}";
  };
  "guest-exec-policy/detached-ceiling-positive" = {
    expr = exec "detached-ceiling";
    expected = "{\"maxRuntimeSec\":3600,\"scenario\":\"detached-ceiling\"}";
  };
  "guest-exec-policy/interactive-ceiling-positive" = {
    expr = exec "interactive-ceiling";
    expected = "{\"interactiveMaxRuntimeSec\":7200,\"scenario\":\"interactive-ceiling\"}";
  };

  # --- policy violations: each must reject at eval time -------------
  # (bash matched "guest.exec.enable requires")
  "guest-exec-policy/exec-no-control-rejected" = {
    expr = exec "exec-no-control";
    expectedError = { };
  };
  # (bash matched "no workload user")
  "guest-exec-policy/exec-no-user-rejected" = {
    expr = exec "exec-no-user";
    expectedError = { };
  };
  # (bash matched "must not be root")
  "guest-exec-policy/root-user-rejected" = {
    expr = exec "root-user";
    expectedError = { };
  };
  # (bash matched "must match")
  "guest-exec-policy/invalid-user-rejected" = {
    expr = exec "invalid-user";
    expectedError = { };
  };
  # (bash matched "declared as a normal")
  "guest-exec-policy/missing-user-rejected" = {
    expr = exec "missing-user";
    expectedError = { };
  };
  # A non-root NAME aliased to UID 0 (bash matched "with uid = 0").
  "guest-exec-policy/uid-zero-alias-rejected" = {
    expr = exec "uid-zero-alias";
    expectedError = { };
  };
}
