{
  tier = "host-integration";
  provider = "credential-entra";
  scenario = "identity Guest and exact consumer Guest placement";
  requiredObservations = [
    "Host placement is rejected"
    "login Endpoint producer belongs to the identity Guest"
    "access-token delivery reaches only the authenticated consumer"
    "identity Guest private state remains mounted only inside that Guest"
  ];
  externalDependency = "fake Entrablau login and token Endpoint";
  status = "requires Zone runtime and runNixOSTest orchestration";
}
