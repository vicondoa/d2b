# Private, content-addressed artifact catalog for v3 configuration.
#
# Resource specs carry only artifact IDs. The store path is retained in this
# root-readable private document for activation staging and is never copied
# into a public resource, status, audit, or telemetry projection.
{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  ids = lib.sort lib.lessThan (lib.attrNames (cfg.artifacts or { }));
  artifactRows = map
    (artifactId:
      let artifact = cfg.artifacts.${artifactId};
      in {
        inherit artifactId;
        type = artifact.type;
        storePath = "${artifact.package}";
        # The path hash is an eval-time identity anchor. The realised emitter
        # recomputes the package and closure hashes from the same derivation.
        packageDigest = "sha256:${builtins.hashString "sha256" "${artifact.package}"}";
        closureDigest = "sha256:${builtins.hashString "sha256" "${artifact.package}/closure"}";
        closureSize = 0;
      })
    ids;

  preimage = {
    schemaVersion = 3;
    entries = artifactRows;
  };
  preimageJson = builtins.toJSON preimage;
  catalogDigest =
    "sha256:${builtins.hashString "sha256"
      ("d2b:v3:artifact-catalog\000" + preimageJson)}";
  catalogData = preimage // { inherit catalogDigest; };
  catalogJson = builtins.toJSON catalogData;

  catalogPath = pkgs.runCommand "d2b-artifact-catalog.json"
    {
      inherit catalogJson preimageJson;
      nativeBuildInputs = [ pkgs.python3 ];
      passAsFile = [ "catalogJson" "preimageJson" ];
    } ''
      set -euo pipefail
      python3 - "$catalogJsonPath" "$preimageJsonPath" "$out" <<'PY'
      import hashlib
      import json
      import pathlib
      import sys

      catalog_path, preimage_path, output_path = sys.argv[1:]
      with open(preimage_path, encoding="utf-8") as handle:
          preimage = json.load(handle)
      encoded = json.dumps(preimage, sort_keys=True, separators=(",", ":")).encode()
      digest = hashlib.sha256(b"d2b:v3:artifact-catalog\0" + encoded).hexdigest()
      with open(catalog_path, encoding="utf-8") as handle:
          catalog = json.load(handle)
      catalog["catalogDigest"] = "sha256:" + digest
      pathlib.Path(output_path).write_text(
          json.dumps(catalog, sort_keys=True, separators=(",", ":")) + "\n",
          encoding="utf-8",
      )
      PY
    '';
in
{
  options.d2b._artifactCatalogV3 = lib.mkOption {
    type = lib.types.attrsOf lib.types.anything;
    default = { };
    internal = true;
    visible = false;
  };

  config = {
    d2b._artifactCatalogV3 = {
      inherit ids artifactRows preimage catalogDigest catalogData catalogJson;
      path = catalogPath;
      publicEntries = map (entry: builtins.removeAttrs entry [ "storePath" ]) artifactRows;
    };

    d2b._bundle.extraArtifacts.artifactCatalog = {
      data = catalogData;
      jsonText = catalogJson;
      path = catalogPath;
      installFileName = "artifact-catalog.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };
  };
}
