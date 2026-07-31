{
  tier = "host-integration";
  provider = "credential-secret-service";
  scenario = "user-domain Host Process placement";
  requiredObservations = [
    "controller runs as the exact User"
    "CredentialReady follows an acquired lease"
    "status contains no Secret Service material"
  ];
  status = "requires Zone runtime and runNixOSTest orchestration";
}
