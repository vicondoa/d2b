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
        # Eval keeps deterministic placeholders so pure evaluation can inspect
        # the private shape. The realised emitter replaces these with content
        # hashes from the package output and closure registration.
        packageDigest = "sha256:${builtins.hashString "sha256" "${artifact.package}"}";
        closureDigest = "sha256:${builtins.hashString "sha256" "${artifact.package}/closure"}";
        closureSize = 0;
      })
    ids;

  buildRows = map
    (artifactId:
      let
        artifact = cfg.artifacts.${artifactId};
        closure = pkgs.closureInfo { rootPaths = [ artifact.package ]; };
      in {
        inherit artifactId;
        type = artifact.type;
        storePath = "${artifact.package}";
        closureStorePaths = "${closure}/store-paths";
        closureRegistration = "${closure}/registration";
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
      buildRowsJson = builtins.toJSON buildRows;
      nativeBuildInputs = [ pkgs.python3 ];
      passAsFile = [ "buildRowsJson" ];
    } ''
      set -euo pipefail
      python3 - "$buildRowsJsonPath" "$out" <<'PY'
      import hashlib
      import json
      import pathlib
      import sys

      rows_path, output_path = sys.argv[1:]
      with open(rows_path, encoding="utf-8") as handle:
          rows = json.load(handle)

      def digest_path(path):
          digest = hashlib.sha256()
          root = pathlib.Path(path)
          if root.is_file():
              digest.update(root.read_bytes())
          else:
              for child in sorted(p for p in root.rglob("*") if p.is_file()):
                  digest.update(str(child.relative_to(root)).encode())
                  digest.update(b"\0")
                  digest.update(child.read_bytes())
          return "sha256:" + digest.hexdigest()

      entries = []
      for row in rows:
          closure_paths = [
              line.strip()
              for line in pathlib.Path(row["closureStorePaths"]).read_text().splitlines()
              if line.strip()
          ]
          closure_size = sum(
              path.stat().st_size
              for root in closure_paths
              for path in pathlib.Path(root).rglob("*")
              if path.is_file()
          )
          entries.append({
              "artifactId": row["artifactId"],
              "closureDigest": digest_path(row["closureRegistration"]),
              "closureSize": closure_size,
              "packageDigest": digest_path(row["storePath"]),
              "storePath": row["storePath"],
              "type": row["type"],
          })
      preimage = {"entries": entries, "schemaVersion": 3}
      encoded = json.dumps(preimage, sort_keys=True, separators=(",", ":")).encode()
      digest = hashlib.sha256(b"d2b:v3:artifact-catalog\0" + encoded).hexdigest()
      catalog = preimage | {"catalogDigest": "sha256:" + digest}
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
