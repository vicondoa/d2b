# nix-unit cases for the with-observability example.
#
# PARTIAL migration. The source/file-layout assertions are now Rust policy
# lints in packages/nixling-contract-tests/tests/policy_examples_observability.rs.
# The example-level flake check is covered by the root
# `eval-with-observability` check; this file keeps the resolved value cases.
#
# Covered here: the targeted `nix eval` of
# examples/with-observability#nixosConfigurations.demo.config.nixling,
# reconstructed through the root flake's mkEval helper to avoid the example
# flake's `path:../..` mutable-lock fragility.
{ mkEval, flakeRoot, ... }:

let
  configMod = import (flakeRoot + "/examples/with-observability/configuration.nix");
  cfg = (mkEval [ configMod ]).config.nixling;
in
{
  "examples-with-observability/obs-enable" = {
    expr = cfg.observability.enable;
    expected = true;
  };

  "examples-with-observability/obs-vm-name" = {
    expr = cfg.observability.vmName;
    expected = "sys-obs";
  };

  "examples-with-observability/obs-env-name" = {
    expr = cfg.observability.env;
    expected = "obs";
  };

  "examples-with-observability/obs-env-declared" = {
    expr = builtins.hasAttr cfg.observability.env cfg.envs;
    expected = true;
  };

  "examples-with-observability/obs-vm-declared" = {
    expr = builtins.hasAttr cfg.observability.vmName cfg.vms;
    expected = true;
  };

  "examples-with-observability/work-env-declared" = {
    expr = builtins.hasAttr "work" cfg.envs;
    expected = true;
  };

  "examples-with-observability/work-app-declared" = {
    expr = builtins.hasAttr "work-app" cfg.vms;
    expected = true;
  };

  "examples-with-observability/work-app-obs-enable" = {
    expr =
      if builtins.hasAttr "work-app" cfg.vms
      then cfg.vms."work-app".observability.enable
      else false;
    expected = true;
  };
}
