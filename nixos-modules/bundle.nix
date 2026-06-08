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

  enabledVms = lib.filterAttrs (_: vm: vm.enable) config.nixling.vms;
  managedKeyOverrides = lib.sortOn (entry: entry.vm) (lib.filter (entry: entry != null)
    (lib.mapAttrsToList (name: vm:
      if vm.ssh.keyPath == null
      then null
      else {
        vm = name;
        keyPath = toString vm.ssh.keyPath;
      }
    ) enabledVms));

  # Per-artifact SHA-256 hashes.  Keys match the path fields stored in
  # the bundle JSON so the Rust resolver can look up the hash by the same
  # string it uses to resolve the file path.
  artifactHashesMap =
    {
      "/etc/nixling/host.json" =
        "sha256:${builtins.hashFile "sha256" config.nixling._bundle.hostJson.path}";
      "/etc/nixling/processes.json" =
        "sha256:${builtins.hashFile "sha256" config.nixling._bundle.processesJson.path}";
      "/etc/nixling/privileges.json" =
        "sha256:${builtins.hashFile "sha256" config.nixling._bundle.privilegesJson.path}";
    }
    // lib.listToAttrs (map (ref: {
        name = ref.path;
        value = "sha256:${builtins.hashFile "sha256" (config.nixling._bundle.closures.${ref.vm}.path)}";
      }) closureRefs)
    // lib.listToAttrs (map (ref: {
        name = ref.path;
        value = "sha256:${builtins.hashFile "sha256" (config.nixling._bundle.minijailProfiles.${ref.profileId}.path)}";
      }) profileRefs);

  # dataWithoutHash is the canonical bundle content used as the hash
  # input.  builtins.toJSON produces sorted-key compact JSON, matching
  # serde_json's default (BTreeMap) serialization used by the Rust
  # verifier after stripping the bundleHash field.
  # artifactHashes is included as null so the bundleHash commits to the
  # presence of this field; the resolver nullifies it before comparing.
  dataWithoutHash = {
    artifactHashes = null;
    bundleVersion = 4;
    schemaVersion = "v2";
    publicManifestPath = "/run/current-system/sw/share/nixling/vms.json";
    hostPath = "/etc/nixling/host.json";
    processesPath = "/etc/nixling/processes.json";
    privilegesPath = "/etc/nixling/privileges.json";
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
  hashInputFile = pkgs.writeText "nixling-bundle-hash-input.json"
    (builtins.toJSON dataWithoutHash);
  bundleHash = "sha256:${builtins.hashFile "sha256" hashInputFile}";

  data = dataWithoutHash // { inherit bundleHash; artifactHashes = artifactHashesMap; };

  jsonText = builtins.toJSON data;
  jsonFile = pkgs.writeText "nixling-bundle.json" jsonText;
in
{
  options.nixling._bundle.bundle = lib.mkOption {
    type = lib.types.unspecified;
    readOnly = true;
    internal = true;
    description = "Internal W1 schema-v1 bundle.json artifact metadata.";
  };

  config = {
    nixling._bundle.bundle = {
      inherit data jsonText;
      path = "${jsonFile}";
    };
    environment.etc."nixling/bundle.json" = privateEtc jsonFile;
  };
}
