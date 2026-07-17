{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  workloads = import ./workload-process-rows.nix {
    inherit config lib pkgs;
  };
  nixosWorkloads = lib.filter
    (row: row.runtimeImplementation == "cloud-hypervisor")
    workloads;

  privateEtc = source: {
    inherit source;
    mode = "0640";
    user = "root";
    group = "d2bd";
  };

  workloadTopLevel = workloadId:
    cfg._computedWorkloads.${workloadId}.config.system.build.toplevel;

  workloadClosureInfo = workloadId:
    pkgs.closureInfo {
      rootPaths = [ (workloadTopLevel workloadId) ];
    };

  closureArtifact = workload:
    let
      workloadId = workload.workloadId;
      top = "${workloadTopLevel workloadId}";
      closure = workloadClosureInfo workloadId;
      relativePath = "closures/${workloadId}.json";
      file = pkgs.runCommand "d2b-${workloadId}-closure.json"
        { nativeBuildInputs = [ pkgs.python3 ]; } ''
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
            "vm": "${workloadId}",
            "toplevel": "${top}",
            "closurePaths": paths,
            "dbDumpPath": db_dump,
            "declaredRunner": "",
            "runnerParityPath": "",
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
      vm = workloadId;
      path = file;
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
      inherit relativePath;
    };

  closures = lib.listToAttrs (map
    (workload: {
      name = workload.workloadId;
      value = closureArtifact workload;
    })
    nixosWorkloads);
in
{
  config = {
    d2b._bundle.closures = closures;
    environment.etc = lib.mapAttrs'
      (_: closure: lib.nameValuePair "d2b/${closure.relativePath}" (privateEtc closure.path))
      closures;
  };
}
