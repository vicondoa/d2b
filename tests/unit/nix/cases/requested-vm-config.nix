{ mkEval, lib, flakeRoot, system, ... }:

let
  requested = mkEval [
    (import (flakeRoot + "/examples/qemu-media-dark-live.nix"))
  ];
  requestedWithDisplay = mkEval [
    (import (flakeRoot + "/examples/qemu-media-dark-live.nix"))
    ({ ... }: {
      d2b.realms.dark.providers.display = {
        type = "display";
        implementationId = "wayland";
      };
      d2b.realms.dark.workloads.dark-live = {
        providerRefs.display = "display";
        display.wayland = true;
      };
    })
  ];

  cfg = requested.config;
  index = cfg.d2b._index;
  workload = lib.findFirst
    (row: row.workloadName == "dark-live")
    (throw "normalized dark-live workload missing")
    index.workloads.enabledList;
  realm = index.realms.byId.${workload.realmId};
  localRoot = index.realms.byName.local-root;
  runtimeBinding = workload.providerBindings.runtime;
  runtimeProvider = index.providers.byId.${runtimeBinding.providerId};
  roles = index.roles.byWorkloadId.${workload.workloadId};
  qemuRole = lib.findFirst
    (row: row.roleKind == "qemu-media")
    (throw "normalized dark-live qemu-media role missing")
    roles;
  resources = index.resources.byWorkloadId.${workload.workloadId};
  resourceFor = kind:
    lib.findFirst
      (row: row.kind == kind)
      (throw "normalized dark-live ${kind} resource missing")
      resources;
  mediaResource = resourceFor "workload-media";
  qemuRuntimeResource = lib.findFirst
    (row: row.kind == "role-runtime" && row.roleId == qemuRole.roleId)
    (throw "normalized dark-live qemu runtime resource missing")
    resources;

  processes = cfg.d2b._bundle.processesJson.data.vms;
  workloadProcess = lib.findFirst
    (row: row.vm == workload.workloadId)
    (throw "rendered dark-live process DAG missing")
    processes;
  qemuNode = lib.findFirst
    (row: row.id == qemuRole.roleId)
    (throw "rendered dark-live qemu-media node missing")
    workloadProcess.nodes;

  hostJson = cfg.d2b._bundle.hostJson.data;
  providerRegistry = cfg.d2b._bundle.providerRegistryV2Json.data.providers;
  runtimeRegistry = lib.findFirst
    (row:
      (row.binding.axis or null) == "local-runtime"
      && (row.binding.workloadId or null) == workload.workloadId)
    (throw "dark-live runtime registry binding missing")
    providerRegistry;
  storageRegistry = lib.findFirst
    (row:
      (row.binding.axis or null) == "local-storage"
      && (row.binding.workloadId or null) == workload.workloadId)
    (throw "dark-live storage registry binding missing")
    providerRegistry;
  desktopMetadata =
    cfg.d2b._bundle.extraArtifacts.desktopMetadataJson.data;
  desktopWorkload =
    desktopMetadata.workloads.${workload.canonicalTarget};

  displayCfg = requestedWithDisplay.config;
  displayWorkload = displayCfg.d2b._index.workloads.byId.${workload.workloadId};
  displayMapping = lib.findFirst
    (row: row.workloadId == workload.workloadId)
    (throw "normalized dark-live display mapping missing")
    displayCfg.d2b._index.providerRegistryV2Mappings.display;
  niriKdl =
    displayCfg.environment.etc."d2b/niri-vm-borders.kdl".text;
  resolvedAccent =
    displayCfg.d2b._uiColors.vms.${workload.workloadId}.border.active;

  rawArtifactText = builtins.toJSON {
    host = hostJson;
    process = workloadProcess;
    registry = providerRegistry;
    normalizedResources = map
      (row: {
        inherit (row) kind path providerId realmId resourceId roleId workloadId;
      })
      resources;
    desktop = desktopMetadata;
    niri = niriKdl;
  };
in
{
  "requested-vm-config/evaluates-without-hardware" = {
    expr = {
      assertionsGreen =
        lib.all (assertion: assertion.assertion) cfg.assertions;
      platformBinary =
        lib.hasSuffix
          (if system == "x86_64-linux"
           then "/bin/qemu-system-x86_64"
           else "/bin/qemu-system-aarch64")
          qemuNode.binaryPath;
    };
    expected = {
      assertionsGreen = true;
      platformBinary = true;
    };
  };

  "requested-vm-config/dark-env-declared" = {
    expr = {
      inherit (realm)
        canonicalTargetSuffix parentRealmId placement realmId realmPath;
      parentMatches = realm.parentRealmId == localRoot.realmId;
      network = {
        inherit (cfg.d2b.realms.dark.network)
          lanSubnet mode uplinkSubnet;
      };
    };
    expected = {
      canonicalTargetSuffix = "dark.local-root.d2b";
      parentRealmId = "cvudgfqzh442wwtozs7q";
      parentMatches = true;
      placement = "host-local";
      realmId = "x6oymc5vn56e3dhxriqa";
      realmPath = "dark.local-root";
      network = {
        mode = "declared";
        lanSubnet = "10.60.0.0/24";
        uplinkSubnet = "203.0.113.0/30";
      };
    };
  };

  "requested-vm-config/dark-live-manual-qemu-media" = {
    expr = {
      inherit (workload)
        canonicalTarget enabled providerRefs realmId workloadId;
      autostart = workload.spec.autostart;
      runtime = runtimeBinding;
      processIdentity = workloadProcess.workloadIdentity;
      qemu = {
        inherit (qemuNode) id role;
        startsPaused = builtins.elem "-S" qemuNode.argv;
        hasGtkDisplay =
          builtins.elem "-display" qemuNode.argv
          && builtins.elem "gtk,gl=off,show-cursor=on" qemuNode.argv;
        runtimePath = qemuRuntimeResource.path;
      };
    };
    expected = {
      canonicalTarget = "dark-live.dark.local-root.d2b";
      enabled = true;
      providerRefs.runtime = "qemu";
      realmId = "x6oymc5vn56e3dhxriqa";
      workloadId = "rx2jpwox2yeifudyjhwq";
      autostart = false;
      runtime = {
        implementationId = "qemu-media";
        providerId = "lkufv5rli2ulo7a2zgcq";
        providerType = "runtime";
      };
      processIdentity = {
        canonicalTarget = "dark-live.dark.local-root.d2b";
        providerId = "lkufv5rli2ulo7a2zgcq";
        realmId = "x6oymc5vn56e3dhxriqa";
        realmPath = [ "dark" "local-root" ];
        runtimeKind = "qemu-media";
        workloadId = "rx2jpwox2yeifudyjhwq";
        workloadName = "dark-live";
      };
      qemu = {
        id = "e7zw2q5a7pjqkto7c5ma";
        role = "qemu-media-runner";
        startsPaused = true;
        hasGtkDisplay = true;
        runtimePath =
          "/run/d2b/r/x6oymc5vn56e3dhxriqa/w/rx2jpwox2yeifudyjhwq/roles/e7zw2q5a7pjqkto7c5ma";
      };
    };
  };

  "requested-vm-config/opaque-physical-usb-refs" = {
    expr = {
      provider = {
        inherit (runtimeProvider)
          capabilityRefs configRef implementationId providerType;
      };
      media = {
        inherit (mediaResource) kind path resourceId workloadId;
      };
      hostQemuMedia = hostJson.qemuMedia;
    };
    expected = {
      provider = {
        capabilityRefs = [ "qmp-media-attach" ];
        configRef = "dark-live-media";
        implementationId = "qemu-media";
        providerType = "runtime";
      };
      media = {
        kind = "workload-media";
        path =
          "/var/lib/d2b/r/x6oymc5vn56e3dhxriqa/w/rx2jpwox2yeifudyjhwq/media";
        resourceId = "workload/rx2jpwox2yeifudyjhwq/media";
        workloadId = "rx2jpwox2yeifudyjhwq";
      };
      hostQemuMedia = null;
    };
  };

  "requested-vm-config/host-json-has-only-opaque-media" = {
    expr = {
      inherit (hostJson) qemuMedia runtimeProviders vmRuntimes;
      workloadIfName = lib.findFirst
        (row: (row.vm or null) == workload.workloadId)
        null
        hostJson.ifNameMappings;
    };
    expected = {
      qemuMedia = null;
      runtimeProviders = [ ];
      vmRuntimes = [ ];
      workloadIfName = {
        derivedIfname = "d2b-tAFA9B15C";
        env = "x6oymc5vn56e3dhxriqa";
        role = "workload-lan";
        userVisibleName = "d2b-tAFA9B15C";
        vm = "rx2jpwox2yeifudyjhwq";
      };
    };
  };

  "requested-vm-config/boot-selector-and-hotplug-source-coexist" = {
    expr = {
      qmpMediaAttach =
        builtins.elem "qmp-media-attach" runtimeProvider.capabilityRefs;
      runtimeRegistry = {
        inherit (runtimeRegistry.descriptor) implementationId;
        inherit (runtimeRegistry.binding)
          runnerIntentId vmStartIntentId workloadId;
      };
      mediaSetId = storageRegistry.binding.mediaSetId;
      mediaResourceId =
        lib.replaceStrings [ "/" ] [ "-" ] mediaResource.resourceId;
    };
    expected = {
      qmpMediaAttach = true;
      runtimeRegistry = {
        implementationId = "qemu-media";
        runnerIntentId =
          "runner:workload:rx2jpwox2yeifudyjhwq:role:e7zw2q5a7pjqkto7c5ma";
        vmStartIntentId =
          "vm-start:workload:rx2jpwox2yeifudyjhwq:role:e7zw2q5a7pjqkto7c5ma";
        workloadId = "rx2jpwox2yeifudyjhwq";
      };
      mediaSetId = "workload-rx2jpwox2yeifudyjhwq-media";
      mediaResourceId = "workload-rx2jpwox2yeifudyjhwq-media";
    };
  };

  "requested-vm-config/no-raw-usb-identities-in-artifacts" = {
    expr =
      !(lib.hasInfix "/dev/disk/by-id" rawArtifactText)
      && !(lib.hasInfix "/dev/bus/usb" rawArtifactText)
      && !(lib.hasInfix "usbSelector" rawArtifactText)
      && !(lib.hasInfix "busid" rawArtifactText)
      && !(lib.hasInfix "busId" rawArtifactText)
      && !(lib.hasInfix "SecretSerial" rawArtifactText)
      && !(lib.hasInfix "1-2.3" rawArtifactText);
    expected = true;
  };

  "requested-vm-config/no-live-os-or-process-marker-sentinels" = {
    expr =
      !(lib.hasInfix "ForbiddenLiveOSName" rawArtifactText)
      && !(lib.hasInfix "Windows" rawArtifactText)
      && !(lib.hasInfix "macOS" rawArtifactText)
      && !(lib.hasInfix "( W" rawArtifactText)
      && !(lib.hasInfix "W3fu" rawArtifactText)
      && !(lib.hasInfix "P6" rawArtifactText);
    expected = true;
  };

  "requested-vm-config/purple-qemu-media-niri-border" = {
    expr = {
      displayProvider =
        displayWorkload.providerBindings.display.implementationId;
      mapping = {
        inherit (displayMapping) realmId workloadId;
      };
      canonicalTarget =
        lib.hasInfix
          "// Borders for workload: ${workload.canonicalTarget}"
          niriKdl;
      workloadScopedAppId =
        lib.hasInfix
          ''match app-id=r#"^d2b\.${workload.workloadId}\."#''
          niriKdl;
      rawNameAbsent =
        !(lib.hasInfix ''match app-id=r#"^d2b\.dark-live\."#'' niriKdl);
      resolvedAccent =
        lib.hasInfix ''active-color "${resolvedAccent}"'' niriKdl;
      desktopIdentity = {
        inherit (desktopWorkload)
          canonicalTarget providerId realmId workloadId;
      };
    };
    expected = {
      displayProvider = "wayland";
      mapping = {
        realmId = "x6oymc5vn56e3dhxriqa";
        workloadId = "rx2jpwox2yeifudyjhwq";
      };
      canonicalTarget = true;
      workloadScopedAppId = true;
      rawNameAbsent = true;
      resolvedAccent = true;
      desktopIdentity = {
        canonicalTarget = "dark-live.dark.local-root.d2b";
        providerId = "lkufv5rli2ulo7a2zgcq";
        realmId = "x6oymc5vn56e3dhxriqa";
        workloadId = "rx2jpwox2yeifudyjhwq";
      };
    };
  };
}
