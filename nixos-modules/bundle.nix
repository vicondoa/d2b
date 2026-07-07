{ config, lib, pkgs, ... }:

let
  closureRefs = lib.sortOn (ref: ref.vm) (lib.mapAttrsToList (_: closure: {
    vm = closure.vm;
    path = closure.relativePath;
  }) (config.d2b._bundle.closures or { }));

  profileRefs = lib.sortOn (ref: ref.profileId) (lib.mapAttrsToList (_: profile: {
    profileId = profile.data.profileId;
    path = profile.relativePath;
  }) (config.d2b._bundle.minijailProfiles or { }));

  d2bLib = import ./lib.nix { inherit lib; };
  normalNixosVms = d2bLib.normalNixosVms config.d2b.vms;
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
        key = "/etc/d2b/host.json";
        path = config.d2b._bundle.hostJson.path;
      }
      {
        key = "/etc/d2b/processes.json";
        path = config.d2b._bundle.processesJson.path;
      }
      {
        key = "/etc/d2b/privileges.json";
        path = config.d2b._bundle.privilegesJson.path;
      }
      {
        key = "/etc/d2b/storage.json";
        path = config.d2b._bundle.storageJson.path;
      }
      {
        key = "/etc/d2b/sync.json";
        path = config.d2b._bundle.syncJson.path;
      }
      {
        key = "/etc/d2b/allocator.json";
        path = config.d2b._bundle.allocatorJson.path;
      }
      {
        key = "/etc/d2b/realm-controllers.json";
        path = config.d2b._bundle.realmControllersJson.path;
      }
    ]
    ++ map (ref: {
      key = ref.path;
      path = config.d2b._bundle.closures.${ref.vm}.path;
    }) closureRefs
    ++ map (ref: {
      key = ref.path;
      path = config.d2b._bundle.minijailProfiles.${ref.profileId}.path;
    }) profileRefs;

  # dataWithoutHash is the canonical bundle content used as the hash
  # input.  builtins.toJSON produces sorted-key compact JSON, matching
  # serde_json's default (BTreeMap) serialization used by the Rust
  # verifier after stripping the bundleHash field.
  # artifactHashes is included as null so the bundleHash commits to the
  # presence of this field; the resolver nullifies it before comparing.
  dataWithoutHash = {
    artifactHashes = null;
    bundleVersion = 8;
    schemaVersion = "v2";
    publicManifestPath = "/run/current-system/sw/share/d2b/vms.json";
    hostPath = "/etc/d2b/host.json";
    processesPath = "/etc/d2b/processes.json";
    privilegesPath = "/etc/d2b/privileges.json";
    storagePath = "/etc/d2b/storage.json";
    syncPath = "/etc/d2b/sync.json";
    allocatorPath = "/etc/d2b/allocator.json";
    realmControllersPath = "/etc/d2b/realm-controllers.json";
    closures = closureRefs;
    minijailProfiles = profileRefs;
    managedKeys = {
      keysDir = toString config.d2b.site.keysDir;
      knownHostsPath = "${config.d2b.site.stateDir}/known_hosts.d2b";
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
  config = {
    d2b._bundle.bundle = {
      inherit data jsonText;
      path = "${jsonFile}";
      installFileName = "bundle.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };

    # The CLI reads these integrity-pinned bundle artifacts directly before
    # some daemon requests. They must remain non-secret: every regular file
    # under /etc/d2b is made read-only for the lifecycle group below.
    system.activationScripts.d2bBundleAcl = lib.stringAfter [ "etc" "users" ] ''
      if ${pkgs.getent}/bin/getent group d2b >/dev/null && [ -d /etc/d2b ]; then
        ${pkgs.acl}/bin/setfacl -m "g:d2b:rx,m::rx" /etc/d2b 2>/dev/null || true
        ${pkgs.findutils}/bin/find /etc/d2b -type d -exec ${pkgs.acl}/bin/setfacl -m "g:d2b:rx,m::rx" {} + 2>/dev/null || true
        ${pkgs.findutils}/bin/find /etc/d2b -type f -exec ${pkgs.acl}/bin/setfacl -m "g:d2b:r,m::r" {} + 2>/dev/null || true
      fi
    '';
  };
}
