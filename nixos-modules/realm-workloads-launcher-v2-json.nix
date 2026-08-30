# Provider-neutral, argv-free launcher metadata.
#
# The v2 artifact remains installed for current launcher clients while their
# Rust consumer migrates to the Zone resource catalog. Workload lifecycle and
# launcher ownership no longer come from a Nix realm/VM hierarchy, so the
# transitional document contains no controller-authored workload rows.
{ config, ... }:

{
  config.d2b._bundle.realmWorkloadsLauncherV2Json = {
    data = {
      schemaVersion = "v2";
      runtimeState = "contract-only";
      workloads = [ ];
      invariants = {
        argvPrivate = true;
        providerNeutral = true;
        typedExecutionPosture = true;
        realmAccentColorOnly = true;
        noSecretsOrCredentials = true;
      };
    };
    installFileName = "realm-workloads-launcher-v2.json";
    classification = "contractPublic";
    sensitivity = "nonSecret";
  };
}
