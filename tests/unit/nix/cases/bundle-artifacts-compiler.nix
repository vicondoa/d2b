{ lib, ... }@ctx:

let
  h = import ../helpers/bundle-artifacts.nix ctx;
in
{
  "bundle-artifacts/phase2-compiler-is-the-build-validator" = {
    expr = {
      sourceUsesCompiler =
        lib.hasInfix "d2b-resource-compiler compile" h.compilerSource
        && !(lib.hasInfix "python3 -" h.compilerSource);
      sourceUsesFramedDigest =
        lib.hasInfix "framed_canonical_digest" h.compilerMainSource;
      commandReceivesExpectedHash =
        lib.hasInfix "expectedContentHash = data.contentHash" h.compilerSource;
    };
    expected = {
      sourceUsesCompiler = true;
      sourceUsesFramedDigest = true;
      commandReceivesExpectedHash = true;
    };
  };

  "bundle-artifacts/phase2-input-does-not-inline-duplicate-large-payloads" = {
    expr = {
      usesPrivatePathRefs =
        lib.hasInfix "artifactCatalogPath =" h.compilerSource
        && lib.hasInfix "schemaRoot =" h.compilerSource;
      noCatalogPayloadCopy = !(lib.hasInfix "catalogData" h.compilerSource);
      noSchemaPayloadCopy = !(lib.hasInfix "schemaRootData" h.compilerSource);
      noPythonCompiler = !(lib.hasInfix "python3 -" h.compilerSource);
      fakeCompilerIsEvalOnly =
        lib.hasInfix "d2b-resource-compiler-eval-stub"
          (toString h.compilerStub);
    };
    expected = {
      usesPrivatePathRefs = true;
      noCatalogPayloadCopy = true;
      noSchemaPayloadCopy = true;
      noPythonCompiler = true;
      fakeCompilerIsEvalOnly = true;
    };
  };
}
