{ config, lib, ... }:

let
  cfg = config.d2b.observability;
  rows = import ./realm-observability-rows.nix {
    inherit config lib;
  };
in
{
  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion =
          builtins.length rows.secrets == 3
          && lib.all
            (secret:
              secret.owner == "realm-broker"
              && lib.hasPrefix
                "/var/lib/d2b/r/${rows.workload.realmId}/w/${rows.workload.workloadId}/"
                secret.path)
            rows.secrets;
        message =
          "Observability credentials must be emitted as realm-broker-owned "
          + "workload resources under the canonical short-ID state root.";
      }
    ];
  };
}
