{ lib, ... }@ctx:

let
  h = import ../helpers/bundle-artifacts.nix ctx;
  inherit (h)
    activeDigestBundle
    compilerMainSource
    compilerSource
    digestBundle
    installedDigestBundle
    realisedDigestBundle
    ;
in
{
  "bundle-artifacts/v3-zone-data-matches-realised-json" = {
    expr = if ctx.system != "x86_64-linux"
      then true
      else digestBundle.data == realisedDigestBundle;
    expected = true;
  };

  "bundle-artifacts/v3-active-zone-installs-coherent-emitter" = {
    expr = if ctx.system != "x86_64-linux" then true else {
      activePathMatchesV3 = activeDigestBundle.path == digestBundle.path;
      installedSourceMatchesV3 = installedDigestBundle.source == digestBundle.path;
      shippedDataMatchesV3 = digestBundle.data == realisedDigestBundle;
      nonEmptyResources = digestBundle.data.resources != [ ];
      legacyPathNotExposed = !(h.compatibilityDigestBundle ? path);
    };
    expected = {
      activePathMatchesV3 = true;
      installedSourceMatchesV3 = true;
      shippedDataMatchesV3 = true;
      nonEmptyResources = true;
      legacyPathNotExposed = true;
    };
  };

  "bundle-artifacts/phase2-compiler-is-the-build-validator" = {
    expr = {
      sourceUsesCompiler =
        lib.hasInfix "d2b-resource-compiler compile" compilerSource
        && !(lib.hasInfix "python3 -" compilerSource);
      sourceUsesFramedDigest =
        lib.hasInfix "framed_canonical_digest" compilerMainSource;
      hostileFixture = builtins.readFile h.hostileCompilerBuild;
    };
    expected = {
      sourceUsesCompiler = true;
      sourceUsesFramedDigest = true;
      hostileFixture = "compiler-ran\n";
    };
  };

  "bundle-artifacts/phase2-input-does-not-inline-duplicate-large-payloads" = {
    expr = {
      usesPrivatePathRefs =
        lib.hasInfix "artifactCatalogPath =" compilerSource
        && lib.hasInfix "schemaRoot =" compilerSource;
      noCatalogPayloadCopy = !(lib.hasInfix "catalogData" compilerSource);
      noSchemaPayloadCopy = !(lib.hasInfix "schemaRootData" compilerSource);
      noPythonCompiler = !(lib.hasInfix "python3 -" compilerSource);
    };
    expected = {
      usesPrivatePathRefs = true;
      noCatalogPayloadCopy = true;
      noSchemaPayloadCopy = true;
      noPythonCompiler = true;
    };
  };

  "bundle-artifacts/phase2-accepted-elf-shim-builds" = {
    expr = builtins.readFile h.acceptedShimBuild;
    expected = "accepted-shim\n";
  };
}
