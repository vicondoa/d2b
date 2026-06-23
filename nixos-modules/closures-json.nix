{ config, lib, pkgs, ... }:

let
  cfg = config.nixling;
  # nixling-owned access helpers (see lib.nix).
  nl = import ./lib.nix { inherit lib pkgs; };
  normalNixosVms = nl.normalNixosVms cfg.vms;

  privateEtc = source: {
    inherit source;
    mode = "0640";
    user = "root";
    group = if cfg.daemonExperimental.enable then "nixlingd" else "root";
  };

  vmTopOf = name: nl.vmToplevel config name;

  vmRunnerOf = name: nl.vmDeclaredRunner config name;

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
      # argv in Rust via packages/nixling-host/src/*_argv.rs); the
      # bundle's `declaredRunner` / `runnerParityPath` are kept in
      # the schema for tooling that still reads them but rendered
      # as the empty string when no derivation exists. The runner-
      # parity invariant is enforced in the broker by comparing the
      # bundle's prebuilt argv to the Rust regenerator's output
      # (see packages/nixling-priv-broker/src/runtime.rs SpawnRunner
      # dispatch arm).
      runnerDrv = vmRunnerOf name;
      runner = if runnerDrv == null then "" else "${runnerDrv}";
      closure = vmClosureInfo name;
      relativePath = "closures/${name}.json";
      file = pkgs.runCommand "nixling-${name}-closure.json" { nativeBuildInputs = [ pkgs.python3 ]; } ''
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
        # store-view intent and breaks `nixling switch`/`boot`/`test`.
        # Stable per closure (no runtime state), changes whenever the
        # closure changes. The astronomically-rare u32 collision between
        # two distinct closures of the same VM is caught fail-closed by
        # the hardlink-farm generation-marker identity check
        # (packages/nixling-host/src/hardlink_farm.rs::build_farm).
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
      path = file;
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
      inherit relativePath;
    };

  closures = lib.mapAttrs (name: _: closureArtifact name) normalNixosVms;
in
{
  config = {
    nixling._bundle.closures = closures;
    environment.etc = lib.mapAttrs'
      (_: closure: lib.nameValuePair "nixling/${closure.relativePath}" (privateEtc closure.path))
      closures;
  };
}
