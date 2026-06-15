# nix-unit cases migrated from tests/guest-control-auth-eval.sh.
#
# Guest-control auth token delivery invariants: the read-only `nl-gctl`
# virtiofs token share (source / mountPoint / readOnly), the
# `guest_control_token:` LoadCredential and RequiresMountsFor wiring on the
# COMPUTED GUEST `nixling-guestd` service (never a host service, and the
# token path must never leak into the serialized unit), the guestd
# ExecStart shape, the dedicated token-virtiofsd runner (distinct uid from
# cloud-hypervisor, read-only, scoped socket path, write-policy excluding
# the VM state dir while granting the guest-control runtime dir), the
# guest-control Health readiness node (and retirement of SSH readiness), and
# the production assertions that reject a `/nix/store[/...]` or relative
# `auth.tokenFile` and a `tokenFile` set without `guest.control.enable`.
#
# Reuses the existing evidence module tests/guest-control-auth-eval.nix by
# importing it with a synthetic `flake` shim whose
# `inputs.nixpkgs.lib.nixosSystem` routes through the harness `mkEval`
# (== nixosSystem with the nixling module set). This keeps the eval 100%
# faithful to the retired bash gate's `nix eval` of each parameterization
# while staying hermetic (no `builtins.getFlake`). The default
# parameterization asserts the exact evidence JSON; each rejection case
# asserts the eval THROWS (the bash gate additionally matched the assertion
# message — "must be an absolute" / "guest.control.auth.tokenFile is set" —
# which is not capturable by the harness's `expectedError` bucket, so it
# relaxes to a throw assertion, the same faithful reduction the other
# throw-case migrations took).
#
# Graphics-free fixture (corp-vm guest control only), so no aarch64 platform
# guard is required.
{ mkEval, pkgs, system, flakeRoot, ... }:

let
  flakeShim = {
    inputs.nixpkgs.lib.nixosSystem = { modules, ... }: mkEval modules;
    nixosModules.default = { };
  };
  auth = args: import (flakeRoot + "/tests/guest-control-auth-eval.nix") (args // {
    inherit system pkgs;
    flake = flakeShim;
  });
in
{
  # --- default parameterization: token share + LoadCredential -------
  "guest-control-auth/positive" = {
    expr = auth { };
    expected = "{\"loadCredential\":[\"guest_control_token:/run/nixling-guest-control-host/token\"],\"mountPoint\":\"/run/nixling-guest-control-host\",\"readOnly\":true,\"source\":\"/var/lib/nixling/guest-control-corp-vm\"}";
  };

  # --- rejection cases: each must reject at eval time ----------------
  # (bash matched "must be an absolute")
  "guest-control-auth/store-token-rejected" = {
    expr = auth { tokenFile = "/nix/store"; };
    expectedError = { };
  };
  # (bash matched "must be an absolute")
  "guest-control-auth/store-child-token-rejected" = {
    expr = auth { tokenFile = "/nix/store/not-a-token"; };
    expectedError = { };
  };
  # (bash matched "must be an absolute")
  "guest-control-auth/relative-token-rejected" = {
    expr = auth { tokenFile = "relative-token"; };
    expectedError = { };
  };
  # (bash matched "guest.control.auth.tokenFile is set")
  "guest-control-auth/token-without-control-rejected" = {
    expr = auth { guestControlEnable = false; };
    expectedError = { };
  };
}
