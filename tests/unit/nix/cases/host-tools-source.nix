# Nix source contract for profile-capable host tools.
#
# The isolated fixture source is the same source root consumed by the Nix
# host-tool builders. Keep the provider schemas in that closure because the
# Rust provider crates embed them with include_str!.
{ lib, flakeRoot, d2bLib, ... }:

let
  schemaPaths = [
    "docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json"
    "docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json"
  ];
  filteredSource = d2bLib.cleanRustPackagesSource flakeRoot;
  fixtureSource =
    builtins.readFile (flakeRoot + "/bazel/checks/fixtures/BUILD.bazel");
  hostToolsSource =
    builtins.readFile (flakeRoot + "/nixos-modules/rust-host-tools.nix");
  vmEvaluatorSource =
    builtins.readFile (flakeRoot + "/nixos-modules/vm-evaluator.nix");
  hostSourceLines = lib.splitString "\n" hostToolsSource;
  hostSourceBuilderLines =
    lib.filter (line: lib.hasInfix "src = hostSource;" line) hostSourceLines;
in
{
  "host-tools-source/fixture-declares-provider-schemas" = {
    expr = lib.hasInfix ''"//:d2b_resource_schemas_v3"'' fixtureSource;
    expected = true;
  };

  "host-tools-source/filtered-source-has-provider-schemas" = {
    expr = lib.all
      (path: builtins.pathExists (filteredSource + "/${path}"))
      schemaPaths;
    expected = true;
  };

  "host-tools-source/profile-builders-use-schema-capable-source" = {
    expr = lib.hasInfix "cp -r " hostToolsSource
      && lib.hasInfix ''packagesSrc}/. "$out/"'' hostToolsSource
      && builtins.length hostSourceBuilderLines == 2;
    expected = true;
  };

  "host-tools-source/guest-evaluator-uses-host-tool-overrides" = {
    expr = lib.all (needle: lib.hasInfix needle vmEvaluatorSource) [
      "broker = d2bHostToolOverrides.broker"
      "d2bd = d2bHostToolOverrides.d2bd"
      "d2bHostTools = guestHostTools"
    ];
    expected = true;
  };
}
