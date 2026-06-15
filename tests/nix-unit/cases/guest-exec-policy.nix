# nix-unit cases migrated from tests/guest-exec-policy-eval.sh.
#
# Guest exec policy invariants: the `guest.exec` defaults; the per-user
# dormant `nixling-userd-<user>` units (present in the COMPUTED GUEST config
# only, never on the host, never templated, never wanted); the detached /
# interactive exec surface (slice + run-dir + guestd `--systemd-run-path` /
# `--exec-runner-path` / `--detached-max-runtime-sec` /
# `--interactive-max-runtime-sec` flags) gated on `exec.enable && allowRoot`
# rather than mere `control.enable`; the runtime-ceiling propagation; and
# the policy assertions that reject exec-without-control,
# allowRoot/users-without-enable, an empty exec target, duplicate / root /
# wildcard / unknown users, and a guest-internal override of the host-owned
# `exec.users` list.
#
# Reuses the existing evidence module tests/guest-exec-policy-eval.nix
# (which flake.checks.<sys>.guest-exec-policy already builds for the
# positive "enabled" scenario) by importing it with a synthetic `flake`
# shim whose `inputs.nixpkgs.lib.nixosSystem` routes through the harness
# `mkEval` (== nixosSystem with the nixling module set). This keeps the eval
# 100% faithful to the retired bash gate's `nix eval` of each scenario while
# staying hermetic (no `builtins.getFlake`). Each positive scenario asserts
# the exact evidence JSON; each policy-violation scenario asserts the eval
# THROWS (the bash gate additionally matched the assertion message — that
# precise text is not capturable by the harness's `expectedError` bucket,
# so it relaxes to a throw assertion, the same faithful reduction the other
# throw-case migrations took).
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
  exec = scenario: import (flakeRoot + "/tests/guest-exec-policy-eval.nix") {
    inherit system pkgs scenario;
    flake = flakeShim;
  };
in
{
  # --- positive scenarios: exact evidence JSON ----------------------
  "guest-exec-policy/enabled-positive" = {
    expr = exec "enabled";
    expected = "{\"hostUserd\":[],\"scenario\":\"enabled\",\"sideUserd\":[],\"userd\":[\"nixling-userd-alice\"]}";
  };
  "guest-exec-policy/default-positive" = {
    expr = exec "default";
    expected = "{\"hostUserd\":[],\"scenario\":\"default\",\"sideUserd\":[],\"userd\":[]}";
  };
  "guest-exec-policy/allow-root-only-positive" = {
    expr = exec "allow-root-only";
    expected = "{\"hostUserd\":[],\"scenario\":\"allow-root-only\",\"sideUserd\":[],\"userd\":[]}";
  };
  "guest-exec-policy/allow-root-ceiling-positive" = {
    expr = exec "allow-root-ceiling";
    expected = "{\"maxRuntimeSec\":3600,\"scenario\":\"allow-root-ceiling\"}";
  };
  "guest-exec-policy/allow-root-interactive-ceiling-positive" = {
    expr = exec "allow-root-interactive-ceiling";
    expected = "{\"interactiveMaxRuntimeSec\":7200,\"scenario\":\"allow-root-interactive-ceiling\"}";
  };

  # --- policy violations: each must reject at eval time -------------
  # (bash matched "guest.exec.enable requires")
  "guest-exec-policy/exec-no-control-rejected" = {
    expr = exec "exec-no-control";
    expectedError = { };
  };
  # (bash matched "guest.exec.allowRoot/users are set")
  "guest-exec-policy/exec-disabled-users-rejected" = {
    expr = exec "exec-disabled-users";
    expectedError = { };
  };
  # (bash matched "no exec target is")
  "guest-exec-policy/exec-empty-rejected" = {
    expr = exec "exec-empty";
    expectedError = { };
  };
  # (bash matched "must not contain duplicate")
  "guest-exec-policy/duplicate-user-rejected" = {
    expr = exec "duplicate-user";
    expectedError = { };
  };
  # (bash matched "must not include root")
  "guest-exec-policy/root-user-rejected" = {
    expr = exec "root-user";
    expectedError = { };
  };
  # (bash matched "must match")
  "guest-exec-policy/wildcard-user-rejected" = {
    expr = exec "wildcard-user";
    expectedError = { };
  };
  # (bash matched "declared as a normal or system user")
  "guest-exec-policy/missing-user-rejected" = {
    expr = exec "missing-user";
    expectedError = { };
  };
  # (bash matched "read-only")
  "guest-exec-policy/internal-override-rejected" = {
    expr = exec "internal-override";
    expectedError = { };
  };
}
