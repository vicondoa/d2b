# Domain-separated resource artifact renderers.
{ pkgs }:

let
  resourceBundleGoldenDigest =
    "38bbe7643fbe19b682a1c266fd6b0b6d3dd41e9e5a5abdf5d13be38b8fc37894";
  artifactCatalogGoldenDigest =
    "e2d86f09e58fd957f7750ebcb9b7b194976db06a5578e823afbc2501ef6f4464";

  digestFunctions = ''
    domain_digest() {
      local domain="$1"
      {
        printf '%s\000' "$domain"
        cat
      } | sha256sum | cut -d' ' -f1
    }

    verify_digest_vectors() {
      local resource_bundle_vector artifact_catalog_vector
      resource_bundle_vector=$(printf '%s' '[]' \
        | domain_digest 'd2b:v3:resource-bundle')
      artifact_catalog_vector=$(printf '%s' '{"entries":[],"schemaVersion":3}' \
        | domain_digest 'd2b:v3:artifact-catalog')
      test "$resource_bundle_vector" = "${resourceBundleGoldenDigest}"
      test "$artifact_catalog_vector" = "${artifactCatalogGoldenDigest}"
    }
  '';
in
{
  mkArtifactCatalog = { entriesJson, preimageJson }:
    pkgs.runCommand "d2b-artifact-catalog.json"
      {
        inherit entriesJson preimageJson;
        passAsFile = [ "entriesJson" "preimageJson" ];
      }
      ''
        set -euo pipefail
        ${digestFunctions}
        verify_digest_vectors

        catalogDigest=$(domain_digest 'd2b:v3:artifact-catalog' \
          < "$preimageJsonPath")
        {
          printf '%s' '{"catalogDigest":"sha256:'
          printf '%s' "$catalogDigest"
          printf '%s' '","entries":'
          cat "$entriesJsonPath"
          printf '%s' ',"schemaVersion":3}'
        } > "$out"
      '';

  mkZoneResourceBundle =
    {
      zoneName,
      resourcesJson,
      providerSchemaDigestsJson,
      schemaFingerprintsJson ? "{}",
      zoneJson,
      artifactCatalogPreimageJson,
    }:
    pkgs.runCommand "d2b-zone-${zoneName}-resource-bundle.json"
      {
        inherit
          resourcesJson
          providerSchemaDigestsJson
          schemaFingerprintsJson
          zoneJson
          artifactCatalogPreimageJson
          ;
        passAsFile = [
          "resourcesJson"
          "providerSchemaDigestsJson"
          "schemaFingerprintsJson"
          "artifactCatalogPreimageJson"
        ];
      }
      ''
        set -euo pipefail
        ${digestFunctions}
        verify_digest_vectors

        contentHash=$(domain_digest 'd2b:v3:resource-bundle' \
          < "$resourcesJsonPath")
        catalogDigest=$(domain_digest 'd2b:v3:artifact-catalog' \
          < "$artifactCatalogPreimageJsonPath")
        {
          printf '%s' '{"artifactCatalogDigest":"sha256:'
          printf '%s' "$catalogDigest"
          printf '%s' '","bundleVersion":1,"contentHash":"sha256:'
          printf '%s' "$contentHash"
          printf '%s' '","generatedAt":"1970-01-01T00:00:00.000Z"'
          printf '%s' ',"providerSchemaDigests":'
          cat "$providerSchemaDigestsJsonPath"
          printf '%s' ',"schemaFingerprints":'
          cat "$schemaFingerprintsJsonPath"
          printf '%s' ',"resources":'
          cat "$resourcesJsonPath"
          printf '%s' ',"schemaVersion":3,"zone":'
          printf '%s' "$zoneJson"
          printf '%s' '}'
        } > "$out"
      '';
}
