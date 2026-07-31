{
  tier = "host-integration";
  provider = "credential-managed-identity";
  scenario = "system-domain Host and Guest agent placement";
  requiredObservations = [
    "controller remains secret-free"
    "Host agent receives the azure-imds effect-port client"
    "Guest agent receives the azure-imds-aca effect-port client"
    "user-domain placement is rejected before agent creation"
    "agent network egress remains disabled"
  ];
  externalDependency = "fake IMDS effect-port provider";
  status = "requires Zone runtime and runNixOSTest orchestration";
}
