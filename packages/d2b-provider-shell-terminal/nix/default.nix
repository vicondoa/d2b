# Zone resource projection for Provider/shell-terminal.
#
# Shell bytes and attach handles remain ComponentSession data. Nix emits only
# target-local supervisor Process intents from authored ShellPool and
# ShellSession resources.
{ config, lib, ... }:

let
  cfg = config.d2b;
  providerRef = "Provider/shell-terminal";
  zones = cfg.zones or { };
  resourcesFor = zoneName: zones.${zoneName}.resources or { };

  providerPresent = zoneName:
    builtins.hasAttr "shell-terminal" (resourcesFor zoneName)
    && (resourcesFor zoneName).shell-terminal.type == "Provider";

  providerAssertions = zoneName:
    let
      resources = resourcesFor zoneName;
      provider = if providerPresent zoneName
        then resources.shell-terminal
        else null;
      providerConfig =
        if provider == null then { } else provider.spec.config or { };
    in lib.optionals (provider != null) [{
      assertion = providerConfig == { };
      message = "d2b.zones.${zoneName}.resources.shell-terminal.spec.config must be empty.";
    }];

  processProviderRef = "Provider/system-systemd";

  poolRows = zoneName:
    if !(providerPresent zoneName)
    then [ ]
    else lib.mapAttrsToList
      (poolName: pool: {
        inherit zoneName poolName pool;
        spec = pool.spec or { };
      })
      (lib.filterAttrs
        (_: resource:
          resource.type == "shell-terminal.d2bus.org.ShellPool"
          && (resource.spec.providerRef or null) == providerRef)
        (resourcesFor zoneName));

  sessionRows = zoneName:
    if !(providerPresent zoneName)
    then [ ]
    else lib.mapAttrsToList
      (sessionName: session: {
        inherit zoneName sessionName session;
        spec = session.spec or { };
      })
      (lib.filterAttrs
        (_: resource:
          resource.type == "shell-terminal.d2bus.org.ShellSession"
          && (resource.spec.providerRef or null) == providerRef)
        (resourcesFor zoneName));

  poolProcessFor = row:
    let
      executionRef = row.spec.executionRef or null;
      userRef = row.spec.userRef or null;
    in lib.optionalAttrs (executionRef != null) {
    type = "Process";
    metadata = {
      name = "shell-${row.poolName}";
      zone = row.zoneName;
      ownerRef = "shell-terminal.d2bus.org.ShellPool/${row.poolName}";
    };
    spec = {
      providerRef = processProviderRef;
      inherit executionRef userRef;
      domain = if userRef == null then "system" else "user";
      processClass = "service";
      template = "shell-supervisor-main";
      desiredLifecycle = "running";
      deviceUsage = [ ];
      networkUsage = null;
    };
  };

  sessionProcessFor = row:
    let
      executionRef = row.spec.executionRef or null;
      userRef = row.spec.userRef or null;
    in lib.optionalAttrs (executionRef != null) {
    type = "Process";
    metadata = {
      name = "shell-session-${row.sessionName}";
      zone = row.zoneName;
      ownerRef = "shell-terminal.d2bus.org.ShellSession/${row.sessionName}";
    };
    spec = {
      providerRef = processProviderRef;
      inherit executionRef userRef;
      domain = if userRef == null then "system" else "user";
      processClass = "service";
      template = "shell-supervisor-main";
      desiredLifecycle = "running";
      deviceUsage = [ ];
      networkUsage = null;
    };
  };

  processesForZone = zoneName:
    let
      pools = lib.filter (resource: resource != { })
        (map poolProcessFor (poolRows zoneName));
      sessions = lib.filter (resource: resource != { })
        (map sessionProcessFor (sessionRows zoneName));
    in
    lib.listToAttrs (map
      (resource: lib.nameValuePair resource.metadata.name resource)
      (pools ++ sessions));
in
{
  config = {
    assertions = lib.concatLists
      (map providerAssertions (lib.attrNames zones));
    d2b._resourceCompiler.providerProjectionShellTerminal = {
      enabled = lib.any
        (zoneName: processesForZone zoneName != { })
        (lib.attrNames zones);
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
