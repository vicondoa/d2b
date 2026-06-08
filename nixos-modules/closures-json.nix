{ config, lib, pkgs, ... }:

let
  cfg = config.nixling;
  # v1.1-P8: nixling-owned access helpers (see lib.nix).
  nl = import ./lib.nix { inherit lib pkgs; };
  enabledVms = lib.filterAttrs (_: vm: vm.enable) cfg.vms;

  privateEtc = source: {
    inherit source;
    mode = "0640";
    user = "root";
    group = if cfg.daemonExperimental.enable then "nixlingd" else "root";
  };

  vmTopOf = name: nl.vmToplevel config name;

  vmRunnerOf = name: nl.vmDeclaredRunner config name;

  vmClosureInfo = name:
    pkgs.closureInfo {
      rootPaths = [
        (vmTopOf name)
        (vmRunnerOf name)
      ];
    };

  closureArtifact = name:
    let
      top = "${vmTopOf name}";
      runner = "${vmRunnerOf name}";
      closure = vmClosureInfo name;
      relativePath = "closures/${name}.json";
      file = pkgs.runCommand "nixling-${name}-closure.json" { nativeBuildInputs = [ pkgs.python3 ]; } ''
        python - "$out" "${closure}/store-paths" <<'PY'
        import json
        import sys

        out, store_paths = sys.argv[1], sys.argv[2]
        with open(store_paths, encoding="utf-8") as f:
            paths = [line.strip() for line in f if line.strip()]

        data = {
            "schemaVersion": "v2",
            "vm": "${name}",
            "toplevel": "${top}",
            "closurePaths": paths,
            "declaredRunner": "${runner}",
            "runnerParityPath": "${runner}",
            "runnerParityOk": True,
            "generation": {
                "hostGeneration": None,
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
      inherit relativePath;
    };

  closures = lib.mapAttrs (name: _: closureArtifact name) enabledVms;
in
{
  options.nixling._bundle.closures = lib.mkOption {
    type = lib.types.unspecified;
    readOnly = true;
    internal = true;
    description = "Internal W1 per-VM schema-v1 closures/<vm>.json artifact metadata.";
  };

  config = {
    nixling._bundle.closures = closures;
    environment.etc = lib.mapAttrs'
      (_: closure: lib.nameValuePair "nixling/${closure.relativePath}" (privateEtc closure.path))
      closures;
  };
}
