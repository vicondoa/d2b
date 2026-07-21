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

    # Register the observability secret-generation rows in the private
    # bundle so the realm broker has a canonical, non-secret-value
    # authority to provision them from — mirroring storage.json/sync.json's
    # "typed row + single repair owner" contract. Only metadata (source
    # path, generated size, minimum size, mode) crosses into the bundle;
    # no secret bytes are ever materialised here.
    d2b._bundle.extraArtifacts.observabilitySecretsJson = {
      data = {
        schemaVersion = "v1";
        secrets = rows.secrets;
      };
      installFileName = "observability-secrets.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };
  };
}
