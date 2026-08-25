# The single realised Phase 2 compiler used by the v3 resource bundle.
#
# The compiler is built from the workspace rather than imported from a
# consumer's ambient PATH. This keeps the Nix build hermetic and ensures the
# Rust implementation, its Cargo lock, and the bundle derivation move
# together.
{ config, lib, pkgs, d2bHostTools, d2bHostToolOverrides ? null, ... }:

let
  d2bLib = import ./lib.nix { inherit lib; };
  compilerPackage = d2bLib.selectHostToolPackage {
    overrides = d2bHostToolOverrides;
    key = "resourceCompiler";
    fallback = d2bHostTools.resourceCompiler;
  };

  resourceTypes = import ./generated/resource-types.nix;
  semanticResourceTypes = import ./generated/semantic-resource-types.nix;
  providerResourceTypes = [
    "display-wayland.d2bus.org.WaylandPolicy"
    "display-wayland.d2bus.org.WaylandSession"
  ];
  semanticSchemaFileName = resourceType:
    let parts = lib.splitString "." resourceType;
    in "${lib.concatStringsSep "." (lib.init parts)}_${lib.last parts}.schema.json";
  schemaRoot = pkgs.linkFarm "d2b-resource-schemas" (
    (map
      (resourceType: {
        name = "core.d2bus.org_${resourceType}.schema.json";
        path = ../docs/reference/schemas/v3
          + "/core.d2bus.org_${resourceType}.schema.json";
      })
      resourceTypes)
    ++ (map
      (resourceType: {
        path = ../docs/reference/schemas/v3 + "/${semanticSchemaFileName resourceType}";
        name = semanticSchemaFileName resourceType;
      })
      semanticResourceTypes)
    ++ (map
      (resourceType: {
        path = ../docs/reference/schemas/v3 + "/${semanticSchemaFileName resourceType}";
        name = semanticSchemaFileName resourceType;
      })
      providerResourceTypes)
  );
in
{
  config.d2b._resourceCompiler.phase2 = {
    compiler = compilerPackage;
    schemaRoot = schemaRoot;
    strictSecrets = true;
  };
}
