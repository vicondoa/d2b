# Domain-separated resource artifact renderers.
{ pkgs }:

let
  resourceBundleGoldenDigest =
    "854fc6c314b185ac9f842231e368fc75650729f669e15d0f1e60141ea334cb5e";
  artifactCatalogGoldenDigest =
    "2fa7348cd18ac4f54d28aeb87ef0be5da1fd772c3d173d830ef25e67b7adc63e";

  digestFunctions = ''
    domain_digest() {
      local domain="$1"
      python3 -c '
import hashlib
import json
import sys

domain = sys.argv[1]
payload = sys.stdin.buffer.read().decode("utf-8")
frame = {
    "domain": domain,
    "framing": "d2b-digest/v1",
    "payload": payload,
}
encoded = json.dumps(
    frame,
    ensure_ascii=False,
    sort_keys=True,
    separators=(",", ":"),
).encode("utf-8")
print(hashlib.sha256(encoded).hexdigest())
' "$domain"
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
      zoneJson,
      artifactCatalogPreimageJson,
      artifactCatalogPath ? null,
      schemaValidationPath ? null,
    }:
    let
      artifactCatalogPathArg =
        if artifactCatalogPath == null then "" else "${artifactCatalogPath}";
      schemaValidationPathArg =
        if schemaValidationPath == null then "" else "${schemaValidationPath}";
    in
    pkgs.runCommand "d2b-zone-${zoneName}-resource-bundle.json"
      {
        inherit
          resourcesJson
          providerSchemaDigestsJson
          zoneJson
          artifactCatalogPreimageJson
          ;
        inherit artifactCatalogPathArg;
        inherit schemaValidationPathArg;
        passAsFile = [
          "resourcesJson"
          "providerSchemaDigestsJson"
          "artifactCatalogPreimageJson"
        ];
        nativeBuildInputs = [ pkgs.python3 ];
      }
      ''
        set -euo pipefail
        ${digestFunctions}
        verify_digest_vectors
        if [ -n "$schemaValidationPathArg" ]; then
          test -e "$schemaValidationPathArg"
        fi

        contentHash=$(domain_digest 'd2b:v3:resource-bundle' \
          < "$resourcesJsonPath")
        if [ -n "$artifactCatalogPathArg" ]; then
          catalogDigest=$(python3 - "$artifactCatalogPathArg" <<'PY'
        import json
        import pathlib
        import sys

        print(json.loads(pathlib.Path(sys.argv[1]).read_text())["catalogDigest"])
        PY
          )
        else
          catalogDigest="sha256:$(domain_digest 'd2b:v3:artifact-catalog' \
            < "$artifactCatalogPreimageJsonPath")"
        fi
        {
          printf '%s' '{"artifactCatalogDigest":"'
          printf '%s' "$catalogDigest"
          printf '%s' '","bundleVersion":1,"contentHash":"sha256:'
          printf '%s' "$contentHash"
          printf '%s' '","generatedAt":"1970-01-01T00:00:00.000Z"'
          printf '%s' ',"providerSchemaDigests":'
          cat "$providerSchemaDigestsJsonPath"
          printf '%s' ',"resources":'
          cat "$resourcesJsonPath"
          printf '%s' ',"schemaVersion":3,"zone":'
          printf '%s' "$zoneJson"
          printf '%s' '}'
        } > "$out"
      '';
}
