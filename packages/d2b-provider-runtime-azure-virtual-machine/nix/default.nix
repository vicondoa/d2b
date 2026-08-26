# Zone resource projection for Provider/runtime-azure-virtual-machine.
#
# Azure VM control stays inside its configured gateway Guest. Nix emits no
# Host fallback and no cloud identifiers into child Process resources.
{ config, lib, ... }:

let
  cfg = config.d2b;
  providerRef = "Provider/runtime-azure-virtual-machine";
  processProviderRef = "Provider/system-systemd";
  zones = cfg.zones or { };
  resourcesFor = zoneName: zones.${zoneName}.resources or { };

  providerFor = zoneName:
    if builtins.hasAttr "runtime-azure-virtual-machine" (resourcesFor zoneName)
      && (resourcesFor zoneName).runtime-azure-virtual-machine.type == "Provider"
    then (resourcesFor zoneName).runtime-azure-virtual-machine
    else null;

  processesForZone = zoneName:
    let
      provider = providerFor zoneName;
      config = if provider == null then { } else provider.spec.config or { };
      executionRef = config.controllerExecutionRef or null;
    in if executionRef == null
      then { }
      else {
        "azure-vm-controller" = {
          type = "Process";
          metadata = {
            name = "azure-vm-controller";
            zone = zoneName;
            ownerRef = providerRef;
          };
          spec = {
            providerRef = processProviderRef;
            inherit executionRef;
            domain = "system";
            processClass = "service";
            template = "azure-vm-controller";
            desiredLifecycle = "running";
            deviceUsage = [ ];
            networkUsage = null;
          };
        };
      };

  providerAssertions = zoneName:
    let
      provider = providerFor zoneName;
      resources = resourcesFor zoneName;
      config = if provider == null then { } else provider.spec.config or { };
      executionRef = config.controllerExecutionRef or null;
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
      message = "d2b.zones.${zoneName}.resources.runtime-azure-virtual-machine.spec.config.controllerExecutionRef must resolve to a same-Zone Guest.";
    }];

  enabled = lib.any
    (zoneName: processesForZone zoneName != { })
    (lib.attrNames zones);
in
{
  config = {
    assertions = lib.concatLists
      (map providerAssertions (lib.attrNames zones));
    d2b._resourceCompiler.providerProjectionRuntimeAzureVirtualMachine = {
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
