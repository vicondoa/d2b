{ config, lib, pkgs, ... }:

let
  zoneResourceBundles = config.d2b._bundle.zoneResourceBundles or { };
  zoneBundleRefs = lib.sortOn (row: row.zone) (lib.mapAttrsToList
    (zone: artifact: {
      inherit zone;
      path = artifact.installFileName;
    })
    (lib.filterAttrs
      (_: artifact: artifact.installFileName != null && artifact.path != null)
      zoneResourceBundles));

  extraArtifactHashInputs = lib.sortOn (row: row.key) (lib.mapAttrsToList
    (_: artifact: {
      key = "/etc/d2b/${artifact.installFileName}";
      path = artifact.path;
    })
    (lib.filterAttrs
      (_: artifact:
        artifact.installFileName != null
        && artifact.path != null)
      (config.d2b._bundle.extraArtifacts or { })));

  artifactHashInputs = [
    {
      key = "/etc/d2b/privileges.json";
      path = config.d2b._bundle.privilegesJson.path;
    }
    {
      key = "/etc/d2b/realm-workloads-launcher-v2.json";
      path = config.d2b._bundle.realmWorkloadsLauncherV2Json.path;
    }
  ] ++ map (row: {
    key = "/etc/d2b/${row.path}";
    path = config.d2b._bundle.zoneResourceBundles.${row.zone}.path;
  }) zoneBundleRefs ++ extraArtifactHashInputs;

  dataWithoutHash = {
    artifactHashes = null;
    bundleVersion = 1;
    schemaVersion = "v3";
    privilegesPath = "/etc/d2b/privileges.json";
    realmWorkloadsLauncherV2Path =
      "/etc/d2b/realm-workloads-launcher-v2.json";
    zones = zoneBundleRefs;
    generation = {
      generator = "nixos-modules/bundle.nix";
      sourceRevision = null;
      generatedAt = null;
    };
  };

  hashInputJson = builtins.toJSON dataWithoutHash;
  bundleHash = "sha256:${builtins.hashString "sha256" hashInputJson}";
  data = dataWithoutHash // {
    inherit bundleHash;
  };
  jsonText = builtins.toJSON data;
  evalArtifactHashes = lib.listToAttrs (map (row: {
    name = row.key;
    value = "sha256:${builtins.hashString "sha256" row.key}";
  }) artifactHashInputs);
  evalData = dataWithoutHash // {
    inherit bundleHash;
    artifactHashes = evalArtifactHashes;
  };
  baseJsonFile = pkgs.writeText "d2b-bundle-base.json" jsonText;
  artifactHashInputsFile = pkgs.writeText "d2b-bundle-artifact-inputs.json"
    (builtins.toJSON artifactHashInputs);
  jsonFile = pkgs.runCommand "d2b-bundle.json"
    {
      nativeBuildInputs = [ pkgs.python3 ];
    } ''
      python - "$out" "${baseJsonFile}" "${artifactHashInputsFile}" <<'PY'
      import hashlib
      import json
      import sys

      out, base_json, artifact_inputs_json = sys.argv[1:4]
      with open(base_json, encoding="utf-8") as handle:
          data = json.load(handle)
      with open(artifact_inputs_json, encoding="utf-8") as handle:
          artifact_inputs = json.load(handle)

      data["artifactHashes"] = {
          row["key"]: "sha256:" + hashlib.sha256(
              open(row["path"], "rb").read()
          ).hexdigest()
          for row in artifact_inputs
      }
      with open(out, "w", encoding="utf-8") as handle:
          json.dump(data, handle, sort_keys=True, separators=(",", ":"))
      PY
    '';
in
{
  config = {
    d2b._bundle.bundle = {
      inherit data jsonText;
      fixtureData = evalData;
      path = jsonFile;
      installFileName = "bundle.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };

    system.activationScripts.d2bBundleAcl = lib.stringAfter [ "etc" "users" ] ''
      if ${pkgs.getent}/bin/getent group d2b >/dev/null && [ -d /etc/d2b ]; then
        ${pkgs.acl}/bin/setfacl -m "g:d2b:rx,m::rx" /etc/d2b 2>/dev/null || true
        ${pkgs.findutils}/bin/find /etc/d2b -type d -exec ${pkgs.acl}/bin/setfacl -m "g:d2b:rx,m::rx" {} + 2>/dev/null || true
        ${pkgs.findutils}/bin/find /etc/d2b -type f -exec ${pkgs.acl}/bin/setfacl -m "g:d2b:r,m::r" {} + 2>/dev/null || true
      fi
    '';
  };
}
