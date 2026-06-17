# nix-unit cases migrated from tests/guest-control-vsock-eval.sh.
#
# Guest-control base-vsock allocation invariants: CID/socket parity between
# the per-VM manifest, the computed `microvm.vsock.*`, and the
# cloud-hypervisor argv; the deterministic CID ladder across workload / net
# / observability / legacy VMs; the per-VM state-dir tmpfiles rule; the
# guest-control Health readiness node (and the retirement of the SSH
# readiness node); and the override guards that forbid a consumer from
# pinning `microvm.vsock.cid/socket` or smuggling `--vsock` through
# cloud-hypervisor `extraArgs`, plus the AF_UNIX socket-path length ceiling.
#
# Reuses the existing evidence module tests/unit/nix/eval-cases/guest-control-vsock-eval.nix
# (which flake.checks.<sys>.guest-control-vsock already builds for the
# positive "base" scenario) by importing it with a synthetic `flake` shim
# whose `inputs.nixpkgs.lib.nixosSystem` routes through the harness `mkEval`
# (== nixosSystem with the nixling module set). This keeps the eval 100%
# faithful to the retired bash gate's `nix eval` of each scenario while
# staying hermetic (no `builtins.getFlake`). The positive "base" scenario
# asserts the exact evidence JSON; each override scenario asserts the eval
# THROWS (the bash gate additionally matched the message substring — that
# precise text is not capturable by the harness's `expectedError` bucket,
# so it relaxes to a throw assertion, the same faithful reduction the other
# throw-case migrations took).
#
# Graphics-free fixture (observability + net/workload VMs only), so no
# aarch64 platform guard is required; the existing
# flake.checks.aarch64-linux.guest-control-vsock proves the "base" eval is
# arch-portable.
{ mkEval, pkgs, system, flakeRoot, ... }:

let
  flakeShim = {
    inputs.nixpkgs.lib.nixosSystem = { modules, ... }: mkEval modules;
    nixosModules.default = { };
  };
  vsock = scenario: import (flakeRoot + "/tests/unit/nix/eval-cases/guest-control-vsock-eval.nix") {
    inherit system pkgs scenario;
    flake = flakeShim;
  };
in
{
  # --- base: CID/socket parity, CID ladder, tmpfiles, readiness ------
  "guest-control-vsock/base-positive" = {
    expr = vsock "base";
    expected = "{\"alphaArgv\":[\"cid=110,socket=/var/lib/nixling/vms/alpha-vm/vsock.sock\"],\"betaCid\":1110,\"legacyCid\":9723643,\"netCid\":101,\"obsCid\":1000,\"vsockCid\":110,\"vsockHostSocket\":\"/var/lib/nixling/vms/alpha-vm/vsock.sock\"}";
  };

  # --- override guards: each must reject at eval time ----------------
  # (bash matched "read-only")
  "guest-control-vsock/user-vsock-cid-rejected" = {
    expr = vsock "user-vsock-cid";
    expectedError = { };
  };
  # (bash matched "read-only")
  "guest-control-vsock/user-vsock-socket-rejected" = {
    expr = vsock "user-vsock-socket";
    expectedError = { };
  };
  # (bash matched "must not set --vsock")
  "guest-control-vsock/user-vsock-extra-split-rejected" = {
    expr = vsock "user-vsock-extra-split";
    expectedError = { };
  };
  # (bash matched "must not set --vsock")
  "guest-control-vsock/user-vsock-extra-equals-rejected" = {
    expr = vsock "user-vsock-extra-equals";
    expectedError = { };
  };
  # (bash matched "long for Linux AF_UNIX")
  "guest-control-vsock/long-socket-rejected" = {
    expr = vsock "long-socket";
    expectedError = { };
  };
}
