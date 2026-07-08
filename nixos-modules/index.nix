{ config, lib, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  inherit (d2bLib) mkMac subnetIp subnetMask;

  sortNames = names: lib.sort lib.lessThan names;
  sortedAttrNames = attrs: sortNames (lib.attrNames attrs);
  sortedAttrs = attrs:
    lib.listToAttrs (map (name: { inherit name; value = attrs.${name}; })
      (sortedAttrNames attrs));
  sortedMapAttrsToList = f: attrs:
    map (name: f name attrs.${name}) (sortedAttrNames attrs);

  enabledEnvs = sortedAttrs (lib.filterAttrs (_: env: env.enable) cfg.envs);
  enabledVms = sortedAttrs (lib.filterAttrs (_: vm: vm.enable) cfg.vms);
  declaredRealms = sortedAttrs cfg.realms;
  enabledRealms = sortedAttrs (lib.filterAttrs (_: realm: realm.enable) cfg.realms);
  normalNixosVms = sortedAttrs (d2bLib.normalNixosVms cfg.vms);
  qemuMediaVms = sortedAttrs (d2bLib.qemuMediaVms cfg.vms);

  workloadsInEnv = envName:
    sortedAttrs (lib.filterAttrs (_: vm: vm.env == envName) enabledVms);
  workloadNamesInEnv = envName: sortedAttrNames (workloadsInEnv envName);

  externalNetworkConfigured = env:
    env.externalNetwork.enable
    || env.externalNetwork.attachment.enable
    || env.externalNetwork.egress.enable
    || env.externalNetwork.portForwards != [ ]
    || env.externalNetwork.mdns.enable;

  netMeta = envName: net:
    let
      peerEnvCidrs = lib.flatten (sortedMapAttrsToList
        (otherName: otherNet:
          lib.optionals (otherName != envName) [
            otherNet.lanSubnet
            otherNet.uplinkSubnet
          ])
        enabledEnvs);
    in
    rec {
      name = envName;
      inherit (net) lanSubnet uplinkSubnet netName mtu mssClamp;
      allowEastWest = net.lan.allowEastWest;
      hostBlocklist = sortNames (lib.unique (net.hostBlocklist ++ cfg.hostLanCidrs ++ peerEnvCidrs));
      lanBridge = "br-${envName}-lan";
      uplinkBridge = "br-${envName}-up";
      hostUplinkIp = subnetIp uplinkSubnet 1;
      netUplinkIp = subnetIp uplinkSubnet 2;
      netLanIp = subnetIp lanSubnet 1;
      uplinkMask = subnetMask uplinkSubnet;
      lanMask = subnetMask lanSubnet;
      dhcpRangeStart = subnetIp lanSubnet 251;
      dhcpRangeEnd = subnetIp lanSubnet 254;
      netUplinkMac = mkMac envName "up" 2;
      netLanMac = mkMac envName "lan" 1;
      externalNetwork =
        let
          attachment = net.externalNetwork.attachment;
          envWorkloads = workloadsInEnv envName;
          homeMac =
            if attachment.macAddress != null
            then attachment.macAddress
            else mkMac envName "home" 3;
          resolveForward = pf:
            let
              targetPort =
                if pf.targetPort != null
                then pf.targetPort
                else pf.listenPort;
              targetIp =
                if pf.targetIp != null then pf.targetIp
                else if pf.vm != null && builtins.hasAttr pf.vm envWorkloads
                then subnetIp lanSubnet envWorkloads.${pf.vm}.index
                else null;
            in
            {
              inherit (pf) protocol listenPort vm;
              sourceCidrs = sortNames (lib.unique pf.sourceCidrs);
              inherit targetIp targetPort;
            };
        in
        {
          enable = net.externalNetwork.enable;
          attachment = {
            inherit (attachment) enable interface mode macvtapMode;
            macAddress = homeMac;
            hostIfName = "${envName}-h0";
            guestIfName = "external0";
            ipv4 = attachment.ipv4;
          };
          egress = net.externalNetwork.egress // {
            allowedCidrs = sortNames (lib.unique net.externalNetwork.egress.allowedCidrs);
          };
          portForwards = map resolveForward net.externalNetwork.portForwards;
          mdns = net.externalNetwork.mdns;
        };
      workloads = lib.mapAttrs
        (vmName: vm: {
          ip = subnetIp lanSubnet vm.index;
          mac = mkMac envName "lan" vm.index;
          hostName = vmName;
        })
        (workloadsInEnv envName);
    };

  envMeta = lib.mapAttrs netMeta enabledEnvs;
  externalNetworkEnvs = sortedAttrs (lib.filterAttrs (_: env: externalNetworkConfigured env) enabledEnvs);

  realmEnvNames = realm:
    sortNames (lib.unique (
      lib.optionals (realm.env != null) [ realm.env ]
      ++ realm.network.envs
    ));

  # Compute a single workload index row from a declared realm workload.
  # Does NOT reference cfg.manifest to avoid circular deps with manifest.nix;
  # vsockCid derivation lives in realm-workloads-launcher-json.nix.
  realmWorkloadRow = realmName: realm: workloadName: workload:
    let
      vmRef = workload.vmRef;
      hasVm = vmRef != null && builtins.hasAttr vmRef enabledVms;
      runtimeKind =
        if hasVm
        then d2bLib.vmRuntimeKind enabledVms.${vmRef}
        else null;
      runtimeProviderId =
        if runtimeKind != null
        then (d2bLib.runtimeProviderCatalog.${runtimeKind}).provider.id
        else null;
    in {
      inherit realmName workloadName;
      realmId = realm.id;
      realmPath = realm.path;
      # Canonical target address for realm-native tooling: <workload>.<realmPath>.d2b
      targetAddress = "${workloadName}.${realm.path}.d2b";
      inherit (workload) enable actionId label icon;
      capabilityRefs = sortNames (lib.unique workload.capabilityRefs);
      preflightRefs = sortNames (lib.unique workload.preflightRefs);
      inherit vmRef;
      # substrateId: stable reference to the local VM substrate, if any.
      substrateId = vmRef;
      inherit runtimeKind runtimeProviderId;
    };

  realmWorkloadRows = realmName: realm:
    sortedMapAttrsToList
      (workloadName: workload:
        realmWorkloadRow realmName realm workloadName workload)
      realm.workloads;

  # Cross-realm external network attachment conflict detection.
  # Yields a list of conflict records where more than one realm's associated
  # envs share the same external-network attachment interface.
  crossRealmExternalNetworkConflicts =
    let
      realmEnvPairs = lib.flatten (map
        (row:
          map
            (envName: {
              realmName = row.realmName;
              realmPath = row.path;
              inherit envName;
            })
            row.network.enabledEnvNames)
        enabledRealmRows);
      pairsWithAttachment = lib.filter
        (pair:
          builtins.hasAttr pair.envName enabledEnvs
          && (enabledEnvs.${pair.envName}).externalNetwork.attachment.enable)
        realmEnvPairs;
      byInterface = lib.groupBy
        (pair:
          let iface = (enabledEnvs.${pair.envName}).externalNetwork.attachment.interface;
          in if iface != null then iface else "_unspecified")
        pairsWithAttachment;
      conflicting = lib.filterAttrs
        (_: pairs:
          lib.length (lib.unique (map (p: p.realmName) pairs)) > 1)
        byInterface;
    in
    lib.mapAttrsToList
      (interface: pairs: {
        inherit interface;
        realmNames = sortNames (lib.unique (map (p: p.realmName) pairs));
        realmPaths = sortNames (lib.unique (map (p: p.realmPath) pairs));
        envNames = sortNames (lib.unique (map (p: p.envName) pairs));
      })
      conflicting;

  realmProviderRows = realmName: realm:
    lib.listToAttrs (sortedMapAttrsToList
      (providerName: provider: {
        name = providerName;
        value = {
          inherit providerName;
          id = provider.id;
          enabled = provider.enable;
          kind = provider.kind;
          placement =
            if provider.placement != null
            then provider.placement
            else realm.placement;
          capabilityRefs = sortNames (lib.unique provider.capabilityRefs);
          configRef = provider.configRef;
        };
      })
      realm.providers);

  realmControllerMeta = realm:
    let
      realmHash = builtins.substring 0 16 (builtins.hashString "sha256" realm.path);
      principal = "d2br-${realmHash}";
      accessGroup = "d2bra-${realmHash}";
      unitPrefix = "d2b-realm-${realmHash}";
      localHostRealm = realm.enable && realm.placement == "host-local";
      brokerMaterialized = localHostRealm && realm.broker.enable && realm.broker.hostMutation;
    in
    {
      controllerId = "realm-${realmHash}";
      runtimeState = "metadata-only";
      daemon = {
        user = principal;
        group = principal;
        publicSocketGroup = accessGroup;
        serviceName = "${unitPrefix}-daemon.service";
        configPath = "/etc/d2b/realms/${realm.id}/daemon-config.json";
        stateLockPath = "${realm.paths.runDir}/daemon.lock";
        locksDir = "${realm.paths.runDir}/locks";
        socketActivated = false;
        materializedService = localHostRealm;
      };
      broker = {
        enabled = realm.broker.enable;
        hostMutation = realm.broker.hostMutation;
        user = "root";
        group = principal;
        socketPath = realm.paths.brokerSocket;
        socketUnitName = "${unitPrefix}-priv-broker.socket";
        serviceUnitName = "${unitPrefix}-priv-broker.service";
        auditDir = realm.paths.auditDir;
        materializedSocket = brokerMaterialized;
        materializedService = brokerMaterialized;
      };
    };

  realmRow = realmName: realm:
    let
      envNames = realmEnvNames realm;
      providerRows = realmProviderRows realmName realm;
      enabledProviderRows = lib.filterAttrs (_: provider: provider.enabled) providerRows;
      workloadRowList = realmWorkloadRows realmName realm;
    in
    {
      inherit realmName;
      id = realm.id;
      name = realm.name;
      path = realm.path;
      pathParts = lib.splitString "." realm.path;
      enabled = realm.enable;
      parentPath = realm.parent;
      parentId =
        if realm.parent == null
        then null
        else builtins.head (lib.splitString "." realm.parent);
      placement = realm.placement;
      placementProvider = realm.placementProvider;
      providerSpecificPlacement = realm.providerSpecificPlacement;
      allowedUsers = sortNames (lib.unique realm.allowedUsers);
      allowedGroups = sortNames (lib.unique realm.allowedGroups);
      defaultWorkloadNamespace = realm.defaultWorkloadNamespace;
      network = {
        env = realm.env;
        envNames = envNames;
        declaredEnvNames = lib.filter (envName: builtins.hasAttr envName cfg.envs) envNames;
        enabledEnvNames = lib.filter (envName: builtins.hasAttr envName enabledEnvs) envNames;
        missingEnvNames = lib.filter (envName: !(builtins.hasAttr envName cfg.envs)) envNames;
        mode = realm.network.mode;
        cidrRefs = sortNames (lib.unique realm.network.cidrRefs);
      };
      providers = providerRows;
      providerKeys = sortedAttrNames providerRows;
      enabledProviderKeys = sortedAttrNames enabledProviderRows;
      relay = {
        inherit (realm.relay) enable mode credentialRef;
        endpoints = sortNames (lib.unique realm.relay.endpoints);
      };
      discovery = realm.discovery;
      policy = realm.policy;
      keys = realm.keys;
      paths = realm.paths;
      broker = realm.broker;
      controller = realmControllerMeta realm;
      # Workload rows derived from realm.workloads declarations.
      workloads = workloadRowList;
      workloadNames = map (w: w.workloadName) workloadRowList;
      enabledWorkloadNames = map (w: w.workloadName) (lib.filter (w: w.enable) workloadRowList);
    };

  realmRows = sortedMapAttrsToList realmRow declaredRealms;
  enabledRealmRows = lib.filter (realm: realm.enabled) realmRows;
  realmAttrsBy = field: rows:
    lib.listToAttrs (map (row: {
      name = row.${field};
      value = row;
    }) rows);
  realmNamesByEnv = rows:
    lib.listToAttrs (map
      (envName: {
        name = envName;
        value = {
          realmNames = sortNames (map (row: row.realmName)
            (lib.filter (row: builtins.elem envName row.network.envNames) rows));
          realmIds = sortNames (map (row: row.id)
            (lib.filter (row: builtins.elem envName row.network.envNames) rows));
          realmPaths = sortNames (map (row: row.path)
            (lib.filter (row: builtins.elem envName row.network.envNames) rows));
        };
      })
      (sortedAttrNames cfg.envs));

  # Flat list of all workload rows across all realms (declared, including disabled).
  allRealmWorkloadRows = lib.flatten (map (row: row.workloads) realmRows);

  # Flat list of workload rows for enabled realms whose workload.enable = true.
  enabledRealmWorkloadRows = lib.filter (w: w.enable)
    (lib.flatten (map (row: row.workloads) enabledRealmRows));

  # Map from vmRef -> list of enabled realm workload rows that reference that VM.
  realmWorkloadsByVm =
    lib.foldl
      (acc: row:
        if row.vmRef == null then acc
        else
          let existing = acc.${row.vmRef} or [ ];
          in acc // { ${row.vmRef} = existing ++ [ row ]; })
      { }
      enabledRealmWorkloadRows;



  subset = pred: sortedAttrs (lib.filterAttrs pred enabledVms);
  subsetNames = pred: sortedAttrNames (subset pred);

  graphicsVms = subset (_: vm: vm.graphics.enable);
  audioVms = subset (_: vm: vm.audio.enable);
  videoVms = subset (_: vm: vm.graphics.enable && vm.graphics.videoSidecar);
  tpmVms = subset (_: vm: vm.tpm.enable);
  usbipVms = subset (_: vm: vm.usbip.yubikey);
  observedVms = subset
    (name: vm: vm.observability.enable && name != cfg.observability.vmName);
  shellVms = subset (_: vm: vm.guest.shell.enable);

  usbipVmNamesByEnv = lib.mapAttrs
    (envName: _: lib.filter
      (vmName: enabledVms.${vmName}.env == envName)
      (sortedAttrNames usbipVms))
    enabledEnvs;
  usbipEnvNames = lib.filter (envName: usbipVmNamesByEnv.${envName} != [ ])
    (sortedAttrNames enabledEnvs);
  activeUsbipEnvNames =
    if cfg.site.yubikey.enable then usbipEnvNames else [ ];
  activeUsbipVmNamesByEnv = lib.mapAttrs
    (envName: vmNames:
      if builtins.elem envName activeUsbipEnvNames then vmNames else [ ])
    usbipVmNamesByEnv;
  usbipBackendPorts = lib.listToAttrs (lib.imap0
    (i: envName: {
      name = envName;
      value = 3241 + i;
    })
    (sortedAttrNames enabledEnvs));

  obsOtlpPort = d2bLib.observabilityOtlpVsockPort;
  observedVmNames = sortedAttrNames observedVms;
  obsSourcePortMap = lib.listToAttrs (lib.imap0
    (i: name: { inherit name; value = obsOtlpPort + 1 + i; })
    observedVmNames);
  obsVmEnabled =
    cfg.observability.enable
    && enabledVms ? ${cfg.observability.vmName};
  obsSourceRow = name:
    let vm = enabledVms.${name};
    in {
      vmName = name;
      envName = if vm.env == null then "none" else vm.env;
      role = "workload";
      vsockPort = obsSourcePortMap.${name};
      receiverGrpcPort = obsSourcePortMap.${name};
      receiverHttpPort = null;
    };
  obsSources =
    if cfg.observability.enable then
      {
        host = {
          vmName = cfg.observability.host.identityName;
          envName = "host";
          role = "host";
          vsockPort = obsOtlpPort;
          receiverGrpcPort = cfg.observability.signoz.otlpGrpcPort;
          receiverHttpPort = cfg.observability.signoz.otlpHttpPort;
        };
      } // lib.listToAttrs (map
        (name: { inherit name; value = obsSourceRow name; })
        observedVmNames)
    else
      { };

  qemuMediaSourceId = vmName: slotName: source:
    if source.kind == "physical-usb"
    then (if source.ref != null then source.ref else "invalid-missing-ref")
    else "image-${builtins.substring 0 16
      (builtins.hashString "sha256"
        "${vmName}/${slotName}/${if source.path != null then source.path else "missing-path"}")}";

  qemuMediaSourceRow = vmName: slotName: source: ({
    vm = vmName;
    mediaRef = qemuMediaSourceId vmName slotName source;
    slot = slotName;
    sourceKind = source.kind;
    format = source.format;
    readOnly = source.readOnly;
    registryScope =
      if source.kind == "image-file"
      then "direct-config-path"
      else "root-only-runtime-state";
  } // lib.optionalAttrs (source.kind == "image-file") {
    imagePath = source.path;
  } // lib.optionalAttrs (source.kind == "physical-usb" && source.usbSelector != null) {
    usbSelector = source.usbSelector;
  });

  qemuMediaSourceRowsForVm = vmName: vm:
    let
      bootRows =
        if vm.qemuMedia.source == null
        then [ ]
        else [ (qemuMediaSourceRow vmName "boot" vm.qemuMedia.source) ];
      slotRows = lib.flatten (sortedMapAttrsToList
        (slotName: slot:
          if slot.source == null
          then [ ]
          else [ (qemuMediaSourceRow vmName slotName slot.source) ])
        vm.qemuMedia.removableSlots);
    in
    bootRows ++ slotRows;

  qemuMediaSources = lib.sortOn (row: "${row.vm}/${row.mediaRef}/${row.slot}")
    (lib.concatLists (sortedMapAttrsToList qemuMediaSourceRowsForVm qemuMediaVms));

  runtimeRows = lib.listToAttrs (sortedMapAttrsToList
    (name: vm: {
      inherit name;
      value = {
        kind = d2bLib.vmRuntimeKind vm;
        metadata = d2bLib.vmRuntimeMetadata name vm;
      };
    })
    enabledVms);

  runtimeProviders = lib.sortOn (provider: provider.provider.id)
    (map (provider: builtins.removeAttrs provider [ "_hypervisorService" ])
      (lib.attrValues d2bLib.runtimeProviderCatalog));

  index = {
    enabledEnvs = enabledEnvs;
    enabledEnvNames = sortedAttrNames enabledEnvs;
    enabledVms = enabledVms;
    enabledVmNames = sortedAttrNames enabledVms;
    normalNixosVms = normalNixosVms;
    normalNixosVmNames = sortedAttrNames normalNixosVms;
    qemuMediaVms = qemuMediaVms;
    qemuMediaVmNames = sortedAttrNames qemuMediaVms;

    netVmByEnv = lib.mapAttrs (_: env: env.netName) enabledEnvs;
    netVmNames = sortNames (lib.attrValues (lib.mapAttrs (_: env: env.netName) enabledEnvs));
    workloadsByEnv = lib.mapAttrs (envName: _: workloadsInEnv envName) enabledEnvs;
    workloadNamesByEnv = lib.mapAttrs (envName: _: workloadNamesInEnv envName) enabledEnvs;
    envMeta = envMeta;

    externalNetwork = {
      envs = externalNetworkEnvs;
      envNames = sortedAttrNames externalNetworkEnvs;
      envMeta = lib.filterAttrs (envName: _: builtins.elem envName (sortedAttrNames externalNetworkEnvs)) envMeta;
    };

    components = {
      graphics = { vms = graphicsVms; vmNames = sortedAttrNames graphicsVms; };
      audio = { vms = audioVms; vmNames = sortedAttrNames audioVms; };
      video = { vms = videoVms; vmNames = sortedAttrNames videoVms; };
      tpm = { vms = tpmVms; vmNames = sortedAttrNames tpmVms; };
      usbip = { vms = usbipVms; vmNames = sortedAttrNames usbipVms; };
      observability = { vms = observedVms; vmNames = observedVmNames; };
    };

    usbip = {
      hostEnabled = cfg.site.yubikey.enable;
      vms = usbipVms;
      vmNames = sortedAttrNames usbipVms;
      vmNamesByEnv = usbipVmNamesByEnv;
      activeVmNamesByEnv = activeUsbipVmNamesByEnv;
      envNames = usbipEnvNames;
      activeEnvNames = activeUsbipEnvNames;
      backendPorts = usbipBackendPorts;
      envMeta = lib.filterAttrs (envName: _: builtins.elem envName activeUsbipEnvNames) envMeta;
      busidLocksByEnv = lib.mapAttrs
        (envName: vmNames:
          map (vmName: {
            vm = vmName;
            lockOwner = "daemon";
            scope = "per-busid";
            busIds = sortNames enabledVms.${vmName}.usbip.busids;
          })
          vmNames)
        activeUsbipVmNamesByEnv;
    };

    observability = {
      enabled = cfg.observability.enable;
      stackVmName = cfg.observability.vmName;
      stackVmEnabled = obsVmEnabled;
      sourceBasePort = obsOtlpPort;
      sourcePorts = obsSourcePortMap;
      sources = obsSources;
      backendPorts = {
        grafana = cfg.observability.grafana.listenPort;
        signoz = cfg.observability.signoz.listenPort;
        otlpGrpc = cfg.observability.signoz.otlpGrpcPort;
        otlpHttp = cfg.observability.signoz.otlpHttpPort;
        hostRelayVsock = obsOtlpPort;
      };
      relayVmNames = if obsVmEnabled then observedVmNames else [ ];
      byRole = {
        host = lib.optional cfg.observability.enable "host";
        workload = observedVmNames;
        relay = if obsVmEnabled then observedVmNames else [ ];
        stack = lib.optional obsVmEnabled cfg.observability.vmName;
      };
    };

    guestShell = {
      vms = shellVms;
      vmNames = sortedAttrNames shellVms;
      limits = lib.mapAttrs (_: vm: {
        enable = vm.guest.shell.enable;
        defaultName = vm.guest.shell.defaultName;
        maxSessions = vm.guest.shell.maxSessions;
        maxAttached = vm.guest.shell.maxAttached;
        controlEnabled = vm.guest.control.enable;
        execEnabled = vm.guest.exec.enable;
      }) shellVms;
    };

    qemuMedia = {
      vms = qemuMediaVms;
      vmNames = sortedAttrNames qemuMediaVms;
      manualOnlyVmNames = sortedAttrNames qemuMediaVms;
      runtimeMediaVmNames = sortedAttrNames qemuMediaVms;
      sources = qemuMediaSources;
      physicalUsbSources = builtins.filter (row: row.sourceKind == "physical-usb") qemuMediaSources;
      imageFileSources = builtins.filter (row: row.sourceKind == "image-file") qemuMediaSources;
    };

    runtime = {
      byVm = runtimeRows;
      providers = runtimeProviders;
      kinds = sortNames (lib.unique (map (name: runtimeRows.${name}.kind) (sortedAttrNames runtimeRows)));
    };

    realms = {
      declared = declaredRealms;
      enabled = enabledRealms;
      names = sortedAttrNames declaredRealms;
      enabledNames = sortedAttrNames enabledRealms;
      list = realmRows;
      enabledList = enabledRealmRows;
      byId = realmAttrsBy "id" realmRows;
      byPath = realmAttrsBy "path" realmRows;
      enabledById = realmAttrsBy "id" enabledRealmRows;
      enabledByPath = realmAttrsBy "path" enabledRealmRows;
      byEnv = realmNamesByEnv enabledRealmRows;
      # Realm-owned workload index.
      workloads = {
        # All workload rows across all declared realms (includes disabled).
        all = allRealmWorkloadRows;
        # Workload rows for enabled realms with workload.enable = true.
        enabled = enabledRealmWorkloadRows;
        # Map from vmRef -> list of enabled realm workload rows.
        byVm = realmWorkloadsByVm;
      };
      # Cross-realm external network attachment conflict data.
      # A non-empty list indicates realms sharing an attachment interface;
      # this is advisory metadata — hard assertions live in assertions.nix.
      externalNetworkConflicts = crossRealmExternalNetworkConflicts;
    };
  };
in
{
  options.d2b._index = lib.mkOption {
    type = lib.types.attrs;
    default = { };
    internal = true;
    visible = false;
    description = "Internal normalized, deterministic VM/env index derived from declared d2b inputs.";
  };

  config.d2b = {
    _index = index;
    _envMeta = config.d2b._index.envMeta;
  };
}
