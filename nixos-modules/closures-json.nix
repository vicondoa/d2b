{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  # d2b-owned access helpers (see lib.nix).
  d2bLib = import ./lib.nix { inherit lib pkgs; };
  normalNixosVms = d2bLib.normalNixosVms cfg.vms;

  privateEtc = source: {
    inherit source;
    mode = "0640";
    user = "root";
    group = if cfg.daemonExperimental.enable then "d2bd" else "root";
  };

  vmTopOf = name: d2bLib.vmToplevel config name;

  vmRunnerOf = name: d2bLib.vmDeclaredRunner config name;

  vmClosureInfo = name:
    let
      runner = vmRunnerOf name;
    in
    pkgs.closureInfo {
      rootPaths = [ (vmTopOf name) ]
        ++ lib.optional (runner != null) runner;
    };

  closureArtifact = name:
    let
      top = "${vmTopOf name}";
      # per-VM declared runner is null (broker generates
      # argv in Rust via packages/d2b-host/src/*_argv.rs); the
      # bundle's `declaredRunner` / `runnerParityPath` are kept in
      # the schema for tooling that still reads them but rendered
      # as the empty string when no derivation exists. The runner-
      # parity invariant is enforced in the broker by comparing the
      # bundle's prebuilt argv to the Rust regenerator's output
      # (see packages/d2b-priv-broker/src/runtime.rs SpawnRunner
      # dispatch arm).
      runnerDrv = vmRunnerOf name;
      runner = if runnerDrv == null then "" else "${runnerDrv}";
      closure = vmClosureInfo name;
      relativePath = "closures/${name}.json";
      file = pkgs.runCommand "d2b-${name}-closure.json" { nativeBuildInputs = [ pkgs.python3 ]; } ''
        python - "$out" "${closure}/store-paths" "${closure}/registration" <<'PY'
        import hashlib
        import json
        import sys

        out, store_paths, db_dump = sys.argv[1], sys.argv[2], sys.argv[3]
        with open(store_paths, encoding="utf-8") as f:
            paths = [line.strip() for line in f if line.strip()]

        # Deterministic per-VM store-view generation. Derived at eval
        # time from the toplevel store path (whose Nix-base32 hash
        # component already captures the full closure content), reduced
        # to a non-zero u32 so it fits the broker's store-sync /
        # activation generation field. The broker's
        # `build_store_view_intents` SKIPS any closure whose
        # `hostGeneration` is null, so leaving this null disables every
        # store-view intent and breaks `d2b switch`/`boot`/`test`.
        # Stable per closure (no runtime state), changes whenever the
        # closure changes. The astronomically-rare u32 collision between
        # two distinct closures of the same VM is caught fail-closed by
        # the hardlink-farm generation-marker identity check
        # (packages/d2b-host/src/hardlink_farm.rs::build_farm).
        host_generation = (
            int(hashlib.sha256("${top}".encode("utf-8")).hexdigest(), 16) % 4294967295
        ) + 1

        data = {
            "schemaVersion": "v2",
            "vm": "${name}",
            "toplevel": "${top}",
            "closurePaths": paths,
            "dbDumpPath": db_dump,
            "declaredRunner": "${runner}",
            "runnerParityPath": "${runner}",
            "runnerParityOk": True,
            "generation": {
                "hostGeneration": host_generation,
                "vmGeneration": None,
                "sourceRevision": None,
                "generatedAt": None,
            },
        }
        with open(out, "w", encoding="utf-8") as f:
            json.dump(data, f, sort_keys=True, separators=(",", ":"))
        PY
      '';
    in {
      vm = name;
      data = {
        schemaVersion = "v2";
        vm = name;
        toplevel = top;
        closurePaths = [ top ];
        dbDumpPath = "${top}-registration";
        declaredRunner = runner;
        runnerParityPath = runner;
        runnerParityOk = true;
        generation = {
          hostGeneration = 1;
          vmGeneration = null;
          sourceRevision = null;
          generatedAt = null;
        };
      };
      path = file;
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
      inherit relativePath;
    };

  closures = lib.mapAttrs (name: _: closureArtifact name) normalNixosVms;

  # v3 Guest closure view. The Guest names and system artifact IDs come only
  # from authored Zone resources; there is no implicit VM or topology
  # derivation here.
  v3Guests = lib.concatMap
    (zoneName:
      let zone = cfg.zones.${zoneName};
      in lib.mapAttrsToList
        (resourceName: resource: {
          inherit zoneName resourceName resource;
          spec = resource.spec or { };
        })
        (lib.filterAttrs (_: resource: resource.type == "Guest") zone.resources))
    (lib.sort lib.lessThan (lib.attrNames (cfg.zones or { })));

  v3ClosureArtifact = guest:
    let
      artifactId = guest.spec.systemArtifactId or null;
      artifact =
        if artifactId != null && builtins.hasAttr artifactId (cfg.artifacts or { })
        then cfg.artifacts.${artifactId}
        else null;
      closure =
        if artifact == null
        then null
        else pkgs.closureInfo { rootPaths = [ artifact.package ]; };
      relativePath = "closures/zones/${guest.zoneName}/${guest.resourceName}.json";
      file =
        if closure == null
        then pkgs.writeText "d2b-${guest.resourceName}-closure-unresolved.json" "{}"
        else pkgs.runCommand "d2b-${guest.resourceName}-v3-closure.json"
          { nativeBuildInputs = [ pkgs.python3 ]; } ''
            python3 - "$out" "${closure}/store-paths" <<'PY'
            import json
            import sys
            out, store_paths = sys.argv[1:]
            with open(store_paths, encoding="utf-8") as handle:
                paths = sorted(line.strip() for line in handle if line.strip())
            with open(out, "w", encoding="utf-8") as handle:
                json.dump({
                    "artifactId": "${artifactId}",
                    "closurePaths": paths,
                    "guest": "${guest.resourceName}",
                    "schemaVersion": 3,
                    "zone": "${guest.zoneName}",
                }, handle, sort_keys=True, separators=(",", ":"))
            PY
          '';
    in {
      guest = guest.resourceName;
      zone = guest.zoneName;
      artifactId = artifactId;
      storePath = if artifact == null then null else "${artifact.package}";
      closurePaths = if artifact == null then [ ] else [ "${artifact.package}" ];
      path = file;
      relativePath = relativePath;
    };
  v3Closures = lib.listToAttrs (map
    (guest: lib.nameValuePair "${guest.zoneName}/${guest.resourceName}"
      (v3ClosureArtifact guest))
    v3Guests);
in
{
  options.d2b._bundle.closuresV3 = lib.mkOption {
    type = lib.types.attrsOf lib.types.anything;
    default = { };
    internal = true;
    visible = false;
  };

  config = {
    d2b._bundle.closures = closures;
    d2b._bundle.closuresV3 = v3Closures;
    environment.etc = lib.mkMerge [
      (lib.mapAttrs'
        (_: closure:
          lib.nameValuePair "d2b/${closure.relativePath}" (privateEtc closure.path))
        closures)
      (lib.mapAttrs'
        (_: closure:
          lib.nameValuePair "d2b/${closure.relativePath}" (privateEtc closure.path))
        v3Closures)
    ];
  };
}
