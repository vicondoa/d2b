{ config, lib, ... }:

let
  realmStorageRows = import ./realm-storage-rows.nix {
    inherit config lib;
  };
in
{
  # Compatibility import for callers that still import this emitter directly.
  # The public module graph emits the same data through bundle-artifacts.nix.
  config.d2b._bundle.syncJson = {
    data = {
      schemaVersion = "v2";
      locks = realmStorageRows.locks;
    };
    installFileName = "sync.json";
    classification = "contractPrivateNonSecret";
    sensitivity = "nonSecret";
  };
}
