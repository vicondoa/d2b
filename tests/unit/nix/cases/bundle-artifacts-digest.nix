{ lib, ... }@ctx:

let
  h = import ../helpers/bundle-artifacts.nix ctx;
  inherit (h)
    activeCatalog
    activeDigestBundle
    digestBundle
    digestCfg
    digestHelpers
    installedCatalog
    installedDigestBundle
    nullProviderCfg
    realisedCatalog
    realisedDigestBundle
    ;
in
{
  "bundle-artifacts/v3-zone-content-hash-covers-shipped-resources" = {
    expr = if ctx.system != "x86_64-linux" then true else
      digestBundle.data.contentHash
        == "sha256:${digestHelpers.framedDigest
          "d2b:v3:resource-bundle"
          (builtins.toJSON digestBundle.data.resources)}";
    expected = true;
  };

  "bundle-artifacts/v3-zone-content-hash-has-one-prefix" = {
    expr = if ctx.system != "x86_64-linux" then true else
      lib.hasPrefix "sha256:" digestBundle.data.contentHash
      && !(lib.hasPrefix "sha256:sha256:" digestBundle.data.contentHash);
    expected = true;
  };

  "bundle-artifacts/v3-artifact-catalog-data-matches-realised-json" = {
    expr = if ctx.system != "x86_64-linux" then true else
      digestCfg.d2b._artifactCatalogV3.catalogData == realisedCatalog
      && digestBundle.data.artifactCatalogDigest == realisedCatalog.catalogDigest;
    expected = true;
  };

  "bundle-artifacts/v3-artifact-catalog-digest-eval-realised-equal" = {
    expr = if ctx.system != "x86_64-linux" then true else {
      evalDigest = digestBundle.data.artifactCatalogDigest;
      realisedDigest = realisedCatalog.catalogDigest;
      shippedDigest = realisedDigestBundle.artifactCatalogDigest;
      evalMatchesRealised =
        digestBundle.data.artifactCatalogDigest == realisedCatalog.catalogDigest;
      shippedMatchesRealised =
        realisedDigestBundle.artifactCatalogDigest == realisedCatalog.catalogDigest;
    };
    expected = {
      evalDigest = realisedCatalog.catalogDigest;
      realisedDigest = realisedCatalog.catalogDigest;
      shippedDigest = realisedCatalog.catalogDigest;
      evalMatchesRealised = true;
      shippedMatchesRealised = true;
    };
  };

  "bundle-artifacts/v3-central-install-classification-and-mode" = {
    expr = if ctx.system != "x86_64-linux" then true else {
      zoneClassification = activeDigestBundle.classification;
      zoneSensitivity = activeDigestBundle.sensitivity;
      zoneSourceMatches = installedDigestBundle.source == activeDigestBundle.path;
      zoneMode = installedDigestBundle.mode;
      zoneUser = installedDigestBundle.user;
      zoneGroup = installedDigestBundle.group;
      catalogClassification = activeCatalog.classification;
      catalogSensitivity = activeCatalog.sensitivity;
      catalogSourceMatches = installedCatalog.source == activeCatalog.path;
      catalogMode = installedCatalog.mode;
      catalogUser = installedCatalog.user;
      catalogGroup = installedCatalog.group;
      nonEmptyCatalog = realisedCatalog.entries != [ ];
    };
    expected = {
      zoneClassification = "contractPrivateNonSecret";
      zoneSensitivity = "nonSecret";
      zoneSourceMatches = true;
      zoneMode = "0640";
      zoneUser = "root";
      zoneGroup = "d2bd";
      catalogClassification = "contractPrivateNonSecret";
      catalogSensitivity = "nonSecret";
      catalogSourceMatches = true;
      catalogMode = "0640";
      catalogUser = "root";
      catalogGroup = "d2bd";
      nonEmptyCatalog = true;
    };
  };

  "bundle-artifacts/v3-bundle-wires-shared-resource-validation" = {
    expr = lib.any
      (assertion:
        !assertion.assertion
        && lib.hasInfix "ringCapacityBytes is out of bounds"
          assertion.message)
      h.helperWiringCfg.assertions;
    expected = true;
  };

  "bundle-artifacts/v3-provider-secret-config-rejected" = {
    expr = h.providerSecretCfg.assertions;
    expectedError = { };
  };

  "bundle-artifacts/v3-null-provider-digest-is-not-verified" = {
    expr = if ctx.system != "x86_64-linux" then true else
      nullProviderCfg.d2b._bundle.zoneResourceBundlesV3.local-root.data
        .providerSchemaDigests == { };
    expected = true;
  };
}
