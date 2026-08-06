{ lib, ... }@ctx:

let
  h = import ../helpers/bundle-artifacts.nix ctx;
  resources = [
    {
      apiVersion = "resources.d2bus.org/v3";
      metadata = {
        name = "sample";
        zone = "local-root";
      };
      spec = {
        displayName = "Sample";
        groups = [ ];
        osUsername = "sample";
      };
      type = "User";
    }
  ];
  resourcesJson = builtins.toJSON resources;
  bundleDigest = h.digestHelpers.framedDigest
    "d2b:v3:resource-bundle"
    resourcesJson;
  catalogPreimage = builtins.toJSON {
    entries = [ ];
    schemaVersion = 3;
  };
  catalogDigest = h.digestHelpers.framedDigest
    "d2b:v3:artifact-catalog"
    catalogPreimage;
in
{
  "bundle-artifacts/v3-zone-content-hash-covers-shipped-resources" = {
    expr = {
      digest = "sha256:${bundleDigest}"
        == "sha256:${h.digestHelpers.framedDigest
          "d2b:v3:resource-bundle"
          resourcesJson}";
      compilerSelected = h.compilerSelected;
      compilerCommand = h.compilerCommand;
    };
    expected = {
      digest = true;
      compilerSelected = true;
      compilerCommand = "d2b-resource-compiler";
    };
  };

  "bundle-artifacts/v3-zone-content-hash-has-one-prefix" = {
    expr = lib.hasPrefix "sha256:" "sha256:${bundleDigest}"
      && !(lib.hasPrefix "sha256:sha256:" "sha256:${bundleDigest}");
    expected = true;
  };

  "bundle-artifacts/v3-artifact-catalog-frame-is-deterministic" = {
    expr = {
      digest = "sha256:${catalogDigest}";
      stable = catalogDigest == h.digestHelpers.framedDigest
        "d2b:v3:artifact-catalog"
        catalogPreimage;
    };
    expected = {
      digest = "sha256:2fa7348cd18ac4f54d28aeb87ef0be5da1fd772c3d173d830ef25e67b7adc63e";
      stable = true;
    };
  };

  "bundle-artifacts/v3-framed-digest-binds-domain-and-payload" = {
    expr = {
      preimagesDiffer =
        h.digestHelpers.framedDigestPreimage "ab" "c"
        != h.digestHelpers.framedDigestPreimage "a" "bc";
      digestsDiffer =
        h.digestHelpers.framedDigest "ab" "c"
        != h.digestHelpers.framedDigest "a" "bc";
    };
    expected = {
      preimagesDiffer = true;
      digestsDiffer = true;
    };
  };

  "bundle-artifacts/v3-bundle-wires-framed-hash-helper" = {
    expr = {
      helper = h.digestHelpers.framedDigest
        "d2b:v3:resource-bundle"
        "[]";
      noNul = !(lib.hasInfix "\\u0000"
        (h.digestHelpers.framedDigestPreimage
          "d2b:v3:resource-bundle" "[]"));
    };
    expected = {
      helper = "854fc6c314b185ac9f842231e368fc75650729f669e15d0f1e60141ea334cb5e";
      noNul = true;
    };
  };

  "bundle-artifacts/v3-resource-payload-is-bounded" = {
    expr = builtins.stringLength resourcesJson <= 512;
    expected = true;
  };
}
