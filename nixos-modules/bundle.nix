{ config, lib, pkgs, ... }:

let
  privateGroup = if config.nixling.daemonExperimental.enable then "nixlingd" else "root";
  privateEtc = source: {
    inherit source;
    mode = "0640";
    user = "root";
    group = privateGroup;
  };

  closureRefs = lib.sortOn (ref: ref.vm) (lib.mapAttrsToList (_: closure: {
    vm = closure.vm;
    path = closure.relativePath;
  }) (config.nixling._bundle.closures or { }));

  profileRefs = lib.sortOn (ref: ref.profileId) (lib.mapAttrsToList (_: profile: {
    profileId = profile.data.profileId;
    path = profile.relativePath;
  }) (config.nixling._bundle.minijailProfiles or { }));

  nl = import ./lib.nix { inherit lib; };
  normalNixosVms = nl.normalNixosVms config.nixling.vms;
  managedKeyOverrides = lib.sortOn (entry: entry.vm) (lib.filter (entry: entry != null)
    (lib.mapAttrsToList (name: vm:
      if vm.ssh.keyPath == null
      then null
      else {
        vm = name;
        keyPath = toString vm.ssh.keyPath;
      }
    ) normalNixosVms));

  # Per-artifact SHA-256 hashes are computed in the bundle derivation
  # below, not with builtins.hashFile at eval time. The closure artifacts
  # are pkgs.closureInfo-backed build outputs, so hashing them during
  # `nix flake check --no-build` fails before the derivation is realised.
  # Keep only the artifact path/key table in eval; the build script hashes
  # the realised files byte-for-byte.
  artifactHashInputs =
    [
      {
        key = "/etc/nixling/host.json";
        path = config.nixling._bundle.hostJson.path;
      }
      {
        key = "/etc/nixling/processes.json";
        path = config.nixling._bundle.processesJson.path;
      }
      {
        key = "/etc/nixling/privileges.json";
        path = config.nixling._bundle.privilegesJson.path;
      }
      {
        key = "/etc/nixling/storage.json";
        path = config.nixling._bundle.storageJson.path;
      }
      {
        key = "/etc/nixling/sync.json";
        path = config.nixling._bundle.syncJson.path;
      }
    ]
    ++ map (ref: {
      key = ref.path;
      path = config.nixling._bundle.closures.${ref.vm}.path;
    }) closureRefs
    ++ map (ref: {
      key = ref.path;
      path = config.nixling._bundle.minijailProfiles.${ref.profileId}.path;
    }) profileRefs;

  # dataWithoutHash is the canonical bundle content used as the hash
  # input.  builtins.toJSON produces sorted-key compact JSON, matching
  # serde_json's default (BTreeMap) serialization used by the Rust
  # verifier after stripping the bundleHash field.
  # artifactHashes is included as null so the bundleHash commits to the
  # presence of this field; the resolver nullifies it before comparing.
  dataWithoutHash = {
    artifactHashes = null;
    bundleVersion = 6;
    schemaVersion = "v2";
    publicManifestPath = "/run/current-system/sw/share/nixling/vms.json";
    hostPath = "/etc/nixling/host.json";
    processesPath = "/etc/nixling/processes.json";
    privilegesPath = "/etc/nixling/privileges.json";
    storagePath = "/etc/nixling/storage.json";
    syncPath = "/etc/nixling/sync.json";
    closures = closureRefs;
    minijailProfiles = profileRefs;
    managedKeys = {
      keysDir = toString config.nixling.site.keysDir;
      knownHostsPath = "${config.nixling.site.stateDir}/known_hosts.nixling";
      overrides = managedKeyOverrides;
    };
    generation = {
      generator = "nixos-modules/bundle.nix";
      sourceRevision = null;
      generatedAt = null;
    };
  };

  # Hash the pre-bundleHash JSON so the Rust verifier can strip the
  # bundleHash field, nullify artifactHashes, re-serialise with serde_json
  # (sorted keys), and compare SHA-256 to detect in-place tampering.
  hashInputJson = builtins.toJSON dataWithoutHash;
  bundleHash = "sha256:${builtins.hashString "sha256" hashInputJson}";

  # The final JSON contains build-time artifact hashes, so there is no
  # faithful eval-time jsonText for bundle.json. Keep `data`/`jsonText`
  # as the pre-artifact-hash shape for diagnostic consumers; production
  # installation and tests that need the final bytes must use `path`.
  data = dataWithoutHash // { inherit bundleHash; artifactHashes = null; };
  jsonText = builtins.toJSON data;
  baseJsonFile = pkgs.writeText "nixling-bundle-base.json" jsonText;
  artifactHashInputsFile = pkgs.writeText "nixling-bundle-artifact-inputs.json"
    (builtins.toJSON artifactHashInputs);
  jsonFile = pkgs.runCommand "nixling-bundle.json"
    {
      nativeBuildInputs = [ pkgs.python3 ];
    } ''
    python - "$out" "${baseJsonFile}" "${artifactHashInputsFile}" <<'PY'
    import hashlib
    import json
    import sys

    out, base_json, artifact_inputs_json = sys.argv[1:4]
    with open(base_json, encoding="utf-8") as f:
        data = json.load(f)
    with open(artifact_inputs_json, encoding="utf-8") as f:
        artifact_inputs = json.load(f)

    artifact_hashes = {}
    for row in artifact_inputs:
        with open(row["path"], "rb") as f:
            artifact_hashes[row["key"]] = "sha256:" + hashlib.sha256(f.read()).hexdigest()

    data["artifactHashes"] = artifact_hashes
    with open(out, "w", encoding="utf-8") as f:
        json.dump(data, f, sort_keys=True, separators=(",", ":"))
    PY
  '';
in
{
  options.nixling._bundle.bundle = lib.mkOption {
    type = lib.types.unspecified;
    readOnly = true;
    internal = true;
    description = "Internal schema-v1 bundle.json artifact metadata.";
  };

  config = {
    nixling._bundle.bundle = {
      inherit data jsonText;
      path = "${jsonFile}";
    };
    environment.etc."nixling/bundle.json" = privateEtc jsonFile;

    # The CLI reads these integrity-pinned bundle artifacts directly before
    # some daemon requests. They must remain non-secret: every regular file
    # under /etc/nixling is made read-only for the lifecycle group below.
    system.activationScripts.nixlingBundleAcl = lib.stringAfter [ "etc" "users" ] ''
      if ${pkgs.getent}/bin/getent group nixling >/dev/null && [ -d /etc/nixling ]; then
        ${pkgs.acl}/bin/setfacl -m "g:nixling:rx,m::rx" /etc/nixling 2>/dev/null || true
        ${pkgs.findutils}/bin/find /etc/nixling -type d -exec ${pkgs.acl}/bin/setfacl -m "g:nixling:rx,m::rx" {} + 2>/dev/null || true
        ${pkgs.findutils}/bin/find /etc/nixling -type f -exec ${pkgs.acl}/bin/setfacl -m "g:nixling:r,m::r" {} + 2>/dev/null || true
      fi
    '';
  };
}
