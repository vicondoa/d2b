# Zone resource projection for Provider/runtime-azure-container-apps.
#
# ACA control components execute only in the configured gateway Guest. The
# projection carries typed Process intents; credentials and Azure locators
# remain private Provider configuration.
{ config, lib, ... }:

let
  cfg = config.d2b;
  providerRef = "Provider/runtime-azure-container-apps";
  processProviderRef = "Provider/system-systemd";
  zones = cfg.zones or { };
  resourcesFor = zoneName: zones.${zoneName}.resources or { };

  providerFor = zoneName:
    if builtins.hasAttr "runtime-azure-container-apps" (resourcesFor zoneName)
      && (resourcesFor zoneName).runtime-azure-container-apps.type == "Provider"
    then (resourcesFor zoneName).runtime-azure-container-apps
    else null;

  processFor = zoneName: name: template: executionRef: {
    type = "Process";
    metadata = {
      name = name;
      zone = zoneName;
      ownerRef = providerRef;
    };
    spec = {
      providerRef = processProviderRef;
      inherit executionRef;
      domain = "system";
      processClass = "service";
      inherit template;
      desiredLifecycle = "running";
      deviceUsage = [ ];
      networkUsage = null;
    };
  };

  processesForZone = zoneName:
    let
      provider = providerFor zoneName;
      executionRef =
        if provider == null
        then null
        else (provider.spec.config or { }).gatewayExecutionRef or null;
    in if executionRef == null
      then { }
      else {
        "aca-controller" = processFor zoneName
          "aca-controller" "aca-controller" executionRef;
        "aca-deployment-service" = processFor zoneName
          "aca-deployment-service" "aca-deployment-service" executionRef;
      };

  providerAssertions = zoneName:
    let
      provider = providerFor zoneName;
      resources = resourcesFor zoneName;
      executionRef =
        if provider == null
        then null
        else (provider.spec.config or { }).gatewayExecutionRef or null;
      parts = if builtins.isString executionRef
        then lib.splitString "/" executionRef
        else [ ];
      validGuest = builtins.isString executionRef
        && lib.length parts == 2
        && builtins.elemAt parts 0 == "Guest"
        && builtins.hasAttr (builtins.elemAt parts 1) resources
        && (resources.${builtins.elemAt parts 1}).type == "Guest";
    in lib.optionals (provider != null) [{
      assertion = validGuest;
      message = "d2b.zones.${zoneName}.resources.runtime-azure-container-apps.spec.config.gatewayExecutionRef must resolve to a same-Zone Guest.";
    }];

  enabled = lib.any
    (zoneName: processesForZone zoneName != { })
    (lib.attrNames zones);
in
{
  config = {
    assertions = lib.concatLists
      (map providerAssertions (lib.attrNames zones));
    d2b._resourceCompiler.providerProjectionRuntimeAzureContainerApps = {
      inherit enabled;
      processesByZone = lib.genAttrs (lib.attrNames zones) processesForZone;
      resourcesByZone = { };
      guestPatchesByZone = { };
      privateArtifact = {
        schemaVersion = 1;
        providerRef = providerRef;
        processRefs = lib.concatMap
          (zoneName: map
            (resource: "Process/${resource.metadata.name}")
            (lib.attrValues (processesForZone zoneName)))
          (lib.attrNames zones);
      };
    };
  };
}
