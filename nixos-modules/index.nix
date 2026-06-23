{ config, lib, ... }:

let
  cfg = config.nixling;
  nl = import ./lib.nix { inherit lib; };
  inherit (nl) mkMac subnetIp subnetMask;

  sortNames = names: lib.sort lib.lessThan names;
  sortedAttrNames = attrs: sortNames (lib.attrNames attrs);
  sortedAttrs = attrs:
    lib.listToAttrs (map (name: { inherit name; value = attrs.${name}; })
      (sortedAttrNames attrs));
  sortedMapAttrsToList = f: attrs:
    map (name: f name attrs.${name}) (sortedAttrNames attrs);

  enabledEnvs = sortedAttrs (lib.filterAttrs (_: env: env.enable) cfg.envs);
  enabledVms = sortedAttrs (lib.filterAttrs (_: vm: vm.enable) cfg.vms);
  normalNixosVms = sortedAttrs (nl.normalNixosVms cfg.vms);
  qemuMediaVms = sortedAttrs (nl.qemuMediaVms cfg.vms);

  workloadsInEnv = envName:
    sortedAttrs (lib.filterAttrs (_: vm: vm.env == envName) enabledVms);
  workloadNamesInEnv = envName: sortedAttrNames (workloadsInEnv envName);

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
      hostBlocklist = lib.unique (net.hostBlocklist ++ cfg.hostLanCidrs ++ peerEnvCidrs);
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
      workloads = lib.mapAttrs
        (vmName: vm: {
          ip = subnetIp lanSubnet vm.index;
          mac = mkMac envName "lan" vm.index;
          hostName = vmName;
        })
        (workloadsInEnv envName);
    };

  envMeta = lib.mapAttrs netMeta enabledEnvs;

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

  obsOtlpPort = nl.observabilityOtlpVsockPort;
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
        kind = nl.vmRuntimeKind vm;
        metadata = nl.vmRuntimeMetadata name vm;
      };
    })
    enabledVms);

  runtimeProviders = lib.sortOn (provider: provider.provider.id)
    (map (provider: builtins.removeAttrs provider [ "_hypervisorService" ])
      (lib.attrValues nl.runtimeProviderCatalog));

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
      envNames = usbipEnvNames;
      activeEnvNames = activeUsbipEnvNames;
      envMeta = lib.filterAttrs (envName: _: builtins.elem envName activeUsbipEnvNames) envMeta;
      busidLocksByEnv = lib.mapAttrs
        (envName: vmNames:
          map (vmName: {
            vm = vmName;
            lockOwner = "daemon";
            scope = "per-busid";
            busIds = enabledVms.${vmName}.usbip.busids;
          })
          vmNames)
        usbipVmNamesByEnv;
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
  };
in
{
  options.nixling._index = lib.mkOption {
    type = lib.types.attrs;
    default = { };
    internal = true;
    visible = false;
    description = "Internal normalized, deterministic VM/env index derived from declared nixling inputs.";
  };

  config.nixling = {
    _index = index;
    _envMeta = config.nixling._index.envMeta;
  };
}
