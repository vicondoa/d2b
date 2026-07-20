# d2b: a generic framework for declaring microVMs on this host.
#
# This module is the public entry point — pulled in by
# `nixosModules.default = import ./nixos-modules { inherit inputs; }`
# in the flake. The closure-passed `inputs` argument lets each
# sub-module that needs flake inputs get them via partial
# application instead of through `_module.args.inputs`. The latter
# infinite-recurses on host.nix's import-list resolution; the
# deep-eval regression test in the smoke flake covers this wiring.
#
# Workload guest modules receive `inputs` through host.nix's
# closure-passed evaluator. This avoids `_module.args.inputs`
# recursion while keeping the host module graph realm-native.
{ inputs }:

{ ... }:

{
  imports = [
    ./options.nix
    ./options-observability.nix
    ./options-ownership-matrix.nix
    ./bundle-artifacts.nix
    ./index.nix
    ./assertions.nix
    ./realm-users.nix
    ./realm-access.nix
    ./network.nix
    ./realm-device-rows.nix
    ./store.nix
    ./unsafe-local-helper.nix
    ./user-services.nix
    ./ui-colors.nix
    ./niri-vm-borders.nix
    ./clipboard.nix
    ./notifications.nix
    ./desktop-metadata-json.nix
    # Keep the workload evaluator portable to Nix releases without builtins.mod.
    ((builtins.scopedImport {
      builtins = builtins // {
        mod = dividend: divisor:
          dividend - divisor * builtins.div dividend divisor;
      };
    } ./host.nix) { inherit inputs; })
    ./guest-control-host.nix
    ./components/audio/host.nix
    ./components/observability/default.nix
    ./allocator-json.nix
    ./realm-controller-config-json.nix
    ./realm-identity-config-json.nix
    ./host-json.nix
    ./processes-json.nix
    ./provider-registry-v2-json.nix
    ./privileges-json.nix
    ./closures-json.nix
    ./minijail-profiles.nix
    ./bundle.nix
    (import ./host-broker.nix { inherit inputs; })
  ];

  # Entra ID / Himmelblau is not auto-imported here. Consumers compose
  # it into a realm workload:
  #
  #   d2b.realms.<realm>.workloads.<workload>.config.imports = [
  #     inputs.entrablau.nixosModules.default
  #   ];
}
