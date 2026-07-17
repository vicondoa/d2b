{ config, lib, pkgs, ... }:

let
  topConfig = config;
  types = lib.types;
  realmStorageRows = import ./realm-storage-rows.nix {
    inherit config lib;
  };
  networkPlan = import ./realm-network-rows.nix {
    inherit config lib;
  };

  runtimeWorkloads = lib.filter
    (workload:
      let runtime = workload.providerBindings.runtime or null;
      in runtime != null
        && builtins.elem runtime.implementationId [
          "cloud-hypervisor"
          "qemu-media"
        ])
    topConfig.d2b._index.workloads.enabledList;

  networkRowFor = workload:
    lib.findFirst
      (row: row.canonicalWorkloadId == workload.workloadId)
      null
      (lib.flatten (map
        (realm: map
          (row: row // { bridge = realm.resources.bridges.lan.ifName; })
          realm.addressing.workloadRows)
        networkPlan.realms));
  roleFor = workload: kind:
    lib.findFirst
      (role: role.roleKind == kind)
      null
      workload.roles;
  roleRuntime = role:
    if role == null then null else
    (lib.findFirst
      (resource: resource.kind == "role-runtime")
      (throw "workload role ${role.roleId} is missing its runtime resource")
      (topConfig.d2b._index.resources.byRoleId.${role.roleId} or [ ])).path;

  manifestEntry = workload:
    let
      implementation =
        workload.providerBindings.runtime.implementationId;
      nixos = implementation == "cloud-hypervisor";
      network = networkRowFor workload;
      stateDir =
        "/var/lib/d2b/r/${workload.realmId}/w/${workload.workloadId}";
      roleKinds = map (role: role.roleKind) workload.roles;
      hasRole = role: builtins.elem role roleKinds;
      runtimeRole = roleFor workload
        (if nixos then "cloud-hypervisor" else "qemu-media");
      gpuRole =
        let full = roleFor workload "gpu";
        in if full != null then full else roleFor workload "gpu-render-node";
      tpmRole = roleFor workload "swtpm";
    in
    {
      name = workload.workloadId;
      apiSocket =
        if nixos then "${roleRuntime runtimeRole}/api.sock" else null;
      audio = hasRole "audio";
      audioService = null;
      audioStateFile =
        if hasRole "audio" then "${stateDir}/audio/audio-state.json" else null;
      bridge = if network == null then null else network.bridge;
      env = workload.realmId;
      gpuSocket =
        if gpuRole != null then "${roleRuntime gpuRole}/gpu.sock" else null;
      graphics = hasRole "gpu";
      isNetVm = false;
      lifecycle = {
        gracefulShutdown = {
          enable = true;
          timeoutSeconds = null;
        };
        liveActivation.timeoutSeconds = null;
      };
      netVm = null;
      observability = {
        enabled = hasRole "observability-agent";
        vsockCid = null;
        vsockHostSocket = null;
        agentSocket = null;
      };
      runtime = {
        kind = if nixos then "nixos" else "qemu-media";
        provider = {
          id =
            if nixos
            then "local-cloud-hypervisor"
            else "local-qemu-media";
          type = "local";
          driver = if nixos then "cloud-hypervisor" else "qemu";
        };
        capabilities =
          if nixos then {
            lifecycle = true;
            display = true;
            usbHotplug = true;
            guestControl = true;
            exec = true;
            configSync = true;
            ssh = true;
            storeSync = true;
            keys = true;
            inGuestObservability = true;
          } else {
            lifecycle = true;
            display = true;
            usbHotplug = true;
            guestControl = false;
            exec = false;
            configSync = false;
            ssh = false;
            storeSync = false;
            keys = false;
            inGuestObservability = false;
          };
      };
      securityKey = hasRole "security-key-frontend";
      shell =
        if nixos && (workload.spec.shell.enable or false) then {
          enabled = true;
          defaultName = workload.spec.shell.defaultName;
          maxSessions = workload.spec.shell.maxSessions;
          maxAttached = 1;
        } else null;
      sshUser = null;
      inherit stateDir;
      staticIp = if network == null then null else network.ip;
      tap = if network == null then "none" else network.tap.ifName;
      tpm = hasRole "swtpm";
      tpmSocket =
        if tpmRole != null then "${roleRuntime tpmRole}/tpm.sock" else null;
      usbipYubikey = hasRole "usbip";
      usbipdHostIp = null;
    };

  publicManifest = {
    _manifest.manifestVersion = 7;
    _observability = {
      enabled = topConfig.d2b.observability.enable;
      vmName = topConfig.d2b.observability.vmName;
      obsVsockCid = 1000;
      obsVsockHostSocket =
        "/var/lib/d2b/r/local-root/observability/vsock.sock";
      signozUrl = "http://127.0.0.1:8080";
      signozOtlpGrpcPort = topConfig.d2b.observability.signoz.otlpGrpcPort;
      signozOtlpHttpPort = topConfig.d2b.observability.signoz.otlpHttpPort;
    };
  } // lib.listToAttrs (map
    (workload: {
      name = workload.workloadId;
      value = manifestEntry workload;
    })
    runtimeWorkloads);
  publicManifestText = builtins.toJSON publicManifest;
  publicManifestPkg = pkgs.writeTextFile {
    name = "d2b-realm-workloads-manifest";
    text = publicManifestText;
    destination = "/share/d2b/vms.json";
  };

  runtimeBinding = workload:
    workload.providerBindings.runtime or {
      implementationId = "provider-managed";
      providerId = null;
    };
  providerKind = implementation:
    if implementation == "cloud-hypervisor" then "local-vm"
    else if implementation == "qemu-media" then "qemu-media"
    else if implementation == "systemd-user" then "unsafe-local"
    else "provider-managed";
  executionPosture = implementation:
    if implementation == "systemd-user" then {
      isolation = "unsafe-local";
      environment = "systemd-user-manager-ambient";
      displayEnvironment = "wayland-proxy-only";
      executionIdentity = "authenticated-requester-uid";
      sessionPersistence = "user-manager-lifetime";
    } else {
      isolation =
        if builtins.elem implementation [ "cloud-hypervisor" "qemu-media" ]
        then "virtual-machine"
        else "provider-managed";
      environment = "runtime-managed";
      displayEnvironment = "runtime-managed";
      executionIdentity =
        if builtins.elem implementation [ "cloud-hypervisor" "qemu-media" ]
        then "workload-user"
        else "provider-managed";
      sessionPersistence = "runtime-managed";
    };
  publicIcon = icon:
    lib.filterAttrs (_: value: value != null) icon;
  publicLauncherItem = itemId: item: {
    id = itemId;
    inherit (item) type name graphical;
    icon = publicIcon item.icon;
    capabilities =
      if item.type == "shell"
      then [ "persistent-shell" "pty" ]
      else [ "configured-launch" ]
        ++ lib.optional item.graphical "window-forwarding";
  };
  launcherWorkload = workload:
    let
      runtime = runtimeBinding workload;
    in
    {
      identity = {
        inherit (workload) workloadId canonicalTarget realmId;
        realmPath = lib.splitString "." workload.realmPath;
        workloadName =
          if workload.metadata.label == workload.configuredName
          then null
          else workload.metadata.label;
        legacyVmName = null;
        runtimeKind = runtime.implementationId;
        providerId = runtime.providerId;
      };
      providerKind = providerKind runtime.implementationId;
      executionPosture = executionPosture runtime.implementationId;
      label = workload.metadata.label;
      icon = publicIcon workload.metadata.icon;
      realmAccentColor =
        topConfig.d2b._uiColors.realms.${workload.realmName}.accent;
      launcherEnabled = workload.launcher.enabled;
      defaultItemId = workload.launcher.defaultItem;
      capabilities = workload.capabilityRefs;
      items = lib.mapAttrsToList publicLauncherItem workload.launcher.items;
    };
  launcherData = {
    schemaVersion = "v2";
    runtimeState = "contract-only";
    workloads = map launcherWorkload
      topConfig.d2b._index.workloads.enabledList;
    invariants = {
      argvPrivate = true;
      noSecretsOrCredentials = true;
      providerNeutral = true;
      realmAccentColorOnly = true;
      typedExecutionPosture = true;
    };
  };

  privateLauncherItem = itemId: item:
    {
      id = itemId;
      inherit (item) type name;
      icon = publicIcon item.icon;
    }
    // lib.optionalAttrs (item.type == "exec") {
      inherit (item) argv graphical;
    };
  privateWorkload = workload:
    let
      runtime = runtimeBinding workload;
    in
    {
      identity = {
        inherit (workload) workloadId canonicalTarget realmId;
        realmPath = lib.splitString "." workload.realmPath;
        workloadName =
          if workload.metadata.label == workload.configuredName
          then null
          else workload.metadata.label;
        legacyVmName = null;
        runtimeKind = runtime.implementationId;
        providerId = runtime.providerId;
      };
      defaultItemId = workload.launcher.defaultItem;
      items = lib.mapAttrsToList privateLauncherItem workload.launcher.items;
    }
    // lib.optionalAttrs (runtime.implementationId == "systemd-user"
      && (workload.spec.shell.enable or false)) {
      shell = {
        inherit (workload.spec.shell) defaultName maxSessions;
      };
    };
  privateLauncherWorkloads = lib.filter
    (workload:
      let implementation = (runtimeBinding workload).implementationId;
      in implementation == "systemd-user"
        || workload.launcher.items != { }
        || workload.launcher.defaultItem != null)
    topConfig.d2b._index.workloads.enabledList;
  unsafeLocalData = {
    schemaVersion = "v2";
    workloads = map privateWorkload (lib.filter
      (workload:
        (runtimeBinding workload).implementationId == "systemd-user")
      privateLauncherWorkloads);
    localVmWorkloads = map privateWorkload (lib.filter
      (workload:
        builtins.elem (runtimeBinding workload).implementationId [
          "cloud-hypervisor"
          "qemu-media"
        ])
      privateLauncherWorkloads);
  };

  artifactDataModule = types.submodule {
    freeformType = types.attrsOf types.anything;

    options.resourceRequests = lib.mkOption {
      type = types.listOf types.attrs;
      default = [ ];
      internal = true;
      visible = false;
      description = "Composable allocator resource request rows.";
    };
  };

  artifactModule = types.submodule ({ name, config, ... }: {
    options = {
      data = lib.mkOption {
        type =
          if name == "allocatorJson"
          then artifactDataModule
          else types.attrsOf types.anything;
        default = { };
        internal = true;
        visible = false;
        description = "Internal non-secret bundle artifact data.";
      };

      jsonText = lib.mkOption {
        type = types.str;
        default = builtins.toJSON config.data;
        internal = true;
        visible = false;
        description = "Internal JSON rendering for this bundle artifact.";
      };

      path = lib.mkOption {
        type = types.nullOr (types.oneOf [ types.path types.str types.package ]);
        default =
          if config.installFileName == null
          then null
          else pkgs.writeText config.derivationName config.jsonText;
        internal = true;
        visible = false;
        description = "Internal realised path for this bundle artifact.";
      };

      derivationName = lib.mkOption {
        type = types.str;
        default =
          if config.installFileName == null
          then "d2b-${name}.json"
          else "d2b-${baseNameOf config.installFileName}";
        internal = true;
        visible = false;
        description = "Internal derivation name used when path is generated from jsonText.";
      };

      installFileName = lib.mkOption {
        type = types.nullOr types.str;
        default = null;
        internal = true;
        visible = false;
        description = "Internal path below /etc/d2b for central bundle artifact installation.";
      };

      mode = lib.mkOption {
        type = types.str;
        default = "0640";
        internal = true;
        visible = false;
        description = "Internal /etc mode for this bundle artifact.";
      };

      user = lib.mkOption {
        type = types.str;
        default = "root";
        internal = true;
        visible = false;
        description = "Internal /etc owner for this bundle artifact.";
      };

      group = lib.mkOption {
        type = types.str;
        default = privateGroup;
        internal = true;
        visible = false;
        description = "Internal /etc group for this bundle artifact.";
      };

      classification = lib.mkOption {
        type = types.enum [ "contractPublic" "contractPrivateNonSecret" ];
        default = "contractPrivateNonSecret";
        internal = true;
        visible = false;
        description = ''
          Internal payload exposure classification. This is independent of the
          /etc installation ACL: contractPublic data may remain 0640 root:d2bd
          when authorized unprivileged consumers receive it through the daemon
          API rather than reading the bundle directly.
        '';
      };

      sensitivity = lib.mkOption {
        type = types.enum [ "nonSecret" ];
        default = "nonSecret";
        internal = true;
        visible = false;
        description = "Internal sensitivity marker for store-materialised bundle artifacts.";
      };

      enableEtc = lib.mkOption {
        type = types.bool;
        default = config.installFileName != null;
        internal = true;
        visible = false;
        description = "Internal switch for central /etc/d2b artifact installation.";
      };
    };
  });

  nestedArtifactModule = types.submodule {
    freeformType = types.attrsOf types.unspecified;

    options = {
      classification = lib.mkOption {
        type = types.enum [ "contractPublic" "contractPrivateNonSecret" ];
        default = "contractPrivateNonSecret";
        internal = true;
        visible = false;
        description = "Internal non-secret nested bundle artifact classification.";
      };

      sensitivity = lib.mkOption {
        type = types.enum [ "nonSecret" ];
        default = "nonSecret";
        internal = true;
        visible = false;
        description = "Internal sensitivity marker for nested store-materialised bundle artifacts.";
      };
    };
  };

  privateGroup =
    if topConfig.d2b.daemonExperimental.enable
    then "d2bd"
    else "root";

  singletonArtifactNames = [
    "bundle"
    "hostJson"
    "processesJson"
    "privilegesJson"
    "storageJson"
    "syncJson"
    "allocatorJson"
    "realmControllersJson"
    "realmIdentityJson"
    "realmWorkloadsLauncherV2Json"
    "unsafeLocalWorkloadsJson"
    "providerRegistryV2Json"
  ];

  shouldInstall = artifact:
    artifact.enableEtc && artifact.installFileName != null;

  singletonArtifacts = lib.genAttrs singletonArtifactNames
    (name: topConfig.d2b._bundle.${name});

  extraArtifacts = topConfig.d2b._bundle.extraArtifacts;

  collidingExtraArtifactNames =
    lib.attrNames (builtins.intersectAttrs singletonArtifacts extraArtifacts);

  centrallyInstalledArtifacts =
    lib.filterAttrs
      (_: artifact: shouldInstall artifact)
      (singletonArtifacts // extraArtifacts);

in
{
  options.d2b._bundle = {
    bundle = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed bundle.json artifact metadata.";
    };

    hostJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed host.json artifact metadata.";
    };

    processesJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed processes.json artifact metadata.";
    };

    privilegesJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed privileges.json artifact metadata.";
    };

    storageJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed storage.json artifact metadata.";
    };

    syncJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed sync.json artifact metadata.";
    };

    allocatorJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed allocator.json artifact metadata.";
    };

    realmControllersJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed realm-controllers.json artifact metadata.";
    };

    realmIdentityJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed realm-identity.json artifact metadata.";
    };

    realmWorkloadsLauncherV2Json = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed realm-workloads-launcher-v2.json public metadata artifact.";
    };

    unsafeLocalWorkloadsJson = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed unsafe-local-workloads.json private configured-item artifact.";
    };

    providerRegistryV2Json = lib.mkOption {
      type = artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed provider-registry-v2.json private provider composition artifact.";
    };

    extraArtifacts = lib.mkOption {
      type = types.attrsOf artifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed extension point for future singleton bundle artifacts.";
    };

    closures = lib.mkOption {
      type = types.attrsOf nestedArtifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed closures/<vm>.json artifact metadata table.";
    };

    minijailProfiles = lib.mkOption {
      type = types.attrsOf nestedArtifactModule;
      default = { };
      internal = true;
      visible = false;
      description = "Internal typed minijail profile artifact metadata table.";
    };
  };

  options.d2b._manifestJsonPath = lib.mkOption {
    type = types.str;
    internal = true;
    visible = false;
    readOnly = true;
  };

  options.d2b._manifestPkg = lib.mkOption {
    type = types.package;
    internal = true;
    visible = false;
    readOnly = true;
  };

  config = {
    d2b._manifestJsonPath =
      "${publicManifestPkg}/share/d2b/vms.json";
    d2b._manifestPkg = publicManifestPkg;
    environment.systemPackages = [ publicManifestPkg ];

    d2b._bundle.storageJson = {
      data = {
        schemaVersion = "v2";
        paths = realmStorageRows.paths;
      };
      installFileName = "storage.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };

    d2b._bundle.syncJson = {
      data = {
        schemaVersion = "v2";
        locks = realmStorageRows.locks;
      };
      installFileName = "sync.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };

    d2b._bundle.realmWorkloadsLauncherV2Json = {
      data = launcherData;
      installFileName = "realm-workloads-launcher-v2.json";
      classification = "contractPublic";
      sensitivity = "nonSecret";
    };

    d2b._bundle.unsafeLocalWorkloadsJson = {
      data = unsafeLocalData;
      installFileName = "unsafe-local-workloads.json";
      classification = "contractPrivateNonSecret";
      sensitivity = "nonSecret";
    };

    assertions = [
      {
        assertion = collidingExtraArtifactNames == [ ];
        message =
          "d2b internal bundle extraArtifacts collide with reserved artifact names: "
          + lib.concatStringsSep ", " collidingExtraArtifactNames;
      }
    ];

    environment.etc = lib.mkMerge (lib.mapAttrsToList
      (_: artifact: {
        "d2b/${artifact.installFileName}" = {
          source = artifact.path;
          inherit (artifact) mode user group;
        };
      })
      centrallyInstalledArtifacts);
  };
}
