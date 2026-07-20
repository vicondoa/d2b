{ mkEval, lib, system, ... }:

lib.optionalAttrs (system == "x86_64-linux") (
let
  base = { lib, ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };

    d2b.acceptDestructiveV2Cutover = true;
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
    };
    d2b.realms.work = {
      path = "work.local-root";
      network.ui.accentColor = "#FF8800";
      providers = {
        runtime = {
          type = "runtime";
          implementationId = "cloud-hypervisor";
        };
        media-runtime = {
          type = "runtime";
          implementationId = "qemu-media";
        };
        devices = {
          type = "device";
          implementationId = "host-mediated";
        };
        display = {
          type = "display";
          implementationId = "wayland";
        };
      };
      workloads = {
        editor = {
          providerRefs = {
            runtime = "runtime";
            device = "devices";
            display = "display";
          };
          graphics.enable = true;
          display.wayland = true;
          config = {
            networking.hostName = lib.mkDefault "editor";
            users.users.alice = { isNormalUser = true; uid = 1000; };
          };
        };
        headless = {
          providerRefs.runtime = "runtime";
          config = {
            networking.hostName = lib.mkDefault "headless";
            users.users.alice = { isNormalUser = true; uid = 1000; };
          };
        };
        media = {
          providerRefs = {
            runtime = "media-runtime";
            display = "display";
          };
          display.wayland = true;
        };
      };
    };
  };

  cfgOf = overrides: (mkEval ([ base ] ++ overrides)).config;
  etcOf = overrides: (cfgOf overrides).environment.etc;
  kdlKey = "d2b/niri-vm-borders.kdl";
  jsonKey = "d2b/ui-colors.json";
  cssKey = "d2b/ui-colors.css";
  kdlText = etc: if builtins.hasAttr kdlKey etc then etc.${kdlKey}.text else "";
  jsonText = etc: if builtins.hasAttr jsonKey etc then etc.${jsonKey}.text else "";
  cssText = etc: if builtins.hasAttr cssKey etc then etc.${cssKey}.text else "";

  disabledEtc = etcOf [ ];
  uiEtc = etcOf [ ({ ... }: { d2b.site.ui.enable = true; }) ];
  uiJson = builtins.fromJSON (jsonText uiEtc);
  uiCss = cssText uiEtc;
  niriEtc = etcOf [
    ({ ... }: { d2b.site.ui.compositors.niri.enable = true; })
  ];
  niriKdl = kdlText niriEtc;
  legacyNiriEtc = etcOf [
    ({ ... }: { d2b.site.niriVmBorders.enable = true; })
  ];
  legacyNiriKdl = kdlText legacyNiriEtc;
  customEtc = etcOf [
    ({ ... }: {
      d2b.site.ui.compositors.niri.enable = true;
      d2b.site.ui.compositors.niri.outputPath =
        "/etc/d2b/custom-borders.kdl";
    })
  ];
  customKdlKey = "d2b/custom-borders.kdl";

  cfg = cfgOf [ ];
  index = cfg.d2b._index;
  editor = index.workloads.byCanonicalTarget."editor.work.local-root.d2b";
  headless = index.workloads.byCanonicalTarget."headless.work.local-root.d2b";
  media = index.workloads.byCanonicalTarget."media.work.local-root.d2b";
  editorDisplay = lib.findFirst
    (mapping: mapping.workloadId == editor.workloadId)
    null
    index.providerRegistryV2Mappings.display;
  mediaDisplay = lib.findFirst
    (mapping: mapping.workloadId == media.workloadId)
    null
    index.providerRegistryV2Mappings.display;

  processDag = workload:
    lib.findFirst
      (dag: dag.vm == workload.workloadId)
      null
      cfg.d2b._bundle.processesJson.data.vms;
  processNode = workload: roleKind:
    let
      role = lib.findFirst
        (candidate: candidate.roleKind == roleKind)
        null
        index.roles.byWorkloadId.${workload.workloadId};
    in
    lib.findFirst
      (node: node.id == role.roleId)
      null
      (processDag workload).nodes;
  editorProxy = processNode editor "wayland-proxy";
  editorGpu = processNode editor "gpu";
  mediaProxy = processNode media "wayland-proxy";
  mediaRunner = processNode media "qemu-media";
  editorEndpoint =
    index.resources.byId.${editorDisplay.endpointIds.wayland};
  mediaEndpoint =
    index.resources.byId.${mediaDisplay.endpointIds.wayland};
  flagValue = flag: argv:
    let
      positions = builtins.filter
        (i: builtins.elemAt argv i == flag)
        (builtins.genList (i: i) (builtins.length argv));
    in
    if positions == [ ]
    then null
    else builtins.elemAt argv ((builtins.head positions) + 1);
  editorBorder = uiJson.vms.${editor.workloadId}.border;
  mediaBorder = uiJson.vms.${media.workloadId}.border;
  editorRule = ''match app-id=r#"^d2b\.${editor.workloadId}\."#'';
  mediaRule = ''match app-id=r#"^d2b\.${media.workloadId}\."#'';
  headlessRule = ''match app-id=r#"^d2b\.${headless.workloadId}\."#'';
  borderFlags = [
    "--border-enable"
    "--border-color-active"
    "--border-color-inactive"
    "--border-color-urgent"
    "--border-label"
  ];
in
{
  "niri-vm-borders/disabled-no-kdl" = {
    expr = builtins.hasAttr kdlKey disabledEtc;
    expected = false;
  };
  "niri-vm-borders/disabled-no-ui-json" = {
    expr = builtins.hasAttr jsonKey disabledEtc;
    expected = false;
  };
  "niri-vm-borders/disabled-no-ui-css" = {
    expr = builtins.hasAttr cssKey disabledEtc;
    expected = false;
  };
  "niri-vm-borders/ui-enable-has-json" = {
    expr = builtins.hasAttr jsonKey uiEtc;
    expected = true;
  };
  "niri-vm-borders/ui-enable-has-css" = {
    expr = builtins.hasAttr cssKey uiEtc;
    expected = true;
  };
  "niri-vm-borders/ui-enable-no-kdl" = {
    expr = builtins.hasAttr kdlKey uiEtc;
    expected = false;
  };
  "niri-vm-borders/ui-json-version" = {
    expr = uiJson.version;
    expected = 1;
  };
  "niri-vm-borders/ui-json-default-vm-border" = {
    expr = editorBorder;
    expected = {
      active = "#ff8800";
      inactive = "#ff8800";
      urgent = "#ff8800";
    };
  };
  "niri-vm-borders/ui-json-env-accent-present" = {
    expr = uiJson.envs;
    expected = { };
  };
  "niri-vm-borders/ui-css-host-color" = {
    expr = lib.hasInfix "@define-color d2b_host_accent #89b4fa;" uiCss;
    expected = true;
  };
  "niri-vm-borders/ui-css-state-color" = {
    expr = lib.hasInfix "@define-color d2b_state_running #a6e3a1;" uiCss;
    expected = true;
  };
  "niri-vm-borders/ui-css-env-color" = {
    expr = lib.hasInfix "@define-color d2b_env_" uiCss;
    expected = false;
  };
  "niri-vm-borders/ui-css-vm-color" = {
    expr = lib.hasInfix
      "@define-color d2b_vm_${editor.workloadId}_border_active #ff8800;"
      uiCss;
    expected = true;
  };
  "niri-vm-borders/ui-css-hyphenated-vm-color" = {
    expr = lib.hasInfix
      "@define-color d2b_vm_${media.workloadId}_border_active #ff8800;"
      uiCss;
    expected = true;
  };
  "niri-vm-borders/ui-json-mode" = {
    expr = uiEtc.${jsonKey}.mode;
    expected = "0644";
  };
  "niri-vm-borders/ui-css-mode" = {
    expr = uiEtc.${cssKey}.mode;
    expected = "0644";
  };
  "niri-vm-borders/new-backend-has-kdl" = {
    expr = builtins.hasAttr kdlKey niriEtc;
    expected = true;
  };
  "niri-vm-borders/new-backend-has-json" = {
    expr = builtins.hasAttr jsonKey niriEtc;
    expected = true;
  };
  "niri-vm-borders/new-backend-renders-inactive-and-urgent" = {
    expr =
      lib.hasInfix ''inactive-color "#ff8800"'' niriKdl
      && lib.hasInfix ''urgent-color "#ff8800"'' niriKdl;
    expected = true;
  };
  "niri-vm-borders/enabled-has-kdl" = {
    expr = builtins.hasAttr kdlKey legacyNiriEtc;
    expected = true;
  };
  "niri-vm-borders/enabled-work-rule" = {
    expr = lib.hasInfix editorRule legacyNiriKdl;
    expected = true;
  };
  "niri-vm-borders/proxy-border-disabled-keeps-work-rule" = {
    expr = lib.hasInfix editorRule niriKdl;
    expected = true;
  };
  "niri-vm-borders/enabled-headless-no-rule" = {
    expr = lib.hasInfix headlessRule niriKdl;
    expected = false;
  };
  "niri-vm-borders/enabled-qemu-media-host-rule" = {
    expr = lib.hasInfix mediaRule niriKdl;
    expected = true;
  };
  "niri-vm-borders/enabled-qemu-media-stable-title-match" = {
    expr = lib.hasInfix "Borders for workload: media.work.local-root.d2b" niriKdl;
    expected = true;
  };
  "niri-vm-borders/enabled-qemu-media-no-guest-app-id-rule" = {
    expr = lib.hasInfix ''^d2b\.media\.'' niriKdl;
    expected = false;
  };
  "niri-vm-borders/enabled-crosvm-hide-rule" = {
    expr = lib.hasInfix ''match app-id=r#"^crosvm$"#'' niriKdl;
    expected = true;
  };
  "niri-vm-borders/enabled-include-comment" = {
    expr = lib.hasInfix ''include "/etc/d2b/niri-vm-borders.kdl"'' niriKdl;
    expected = true;
  };
  "niri-vm-borders/color-override-verbatim" = {
    expr = editorBorder.active;
    expected = "#ff8800";
  };
  "niri-vm-borders/qemu-media-color-override-verbatim" = {
    expr = mediaBorder.active;
    expected = "#ff8800";
  };
  "niri-vm-borders/default-color-stable" = {
    expr = editorBorder.active == mediaBorder.active;
    expected = true;
  };
  "niri-vm-borders/default-inactive-color-is-identity" = {
    expr = editorBorder.inactive == editorBorder.active;
    expected = true;
  };
  "niri-vm-borders/kdl-mode" = {
    expr = niriEtc.${kdlKey}.mode;
    expected = "0644";
  };
  "niri-vm-borders/custom-output-path-present" = {
    expr = builtins.hasAttr customKdlKey customEtc;
    expected = true;
  };
  "niri-vm-borders/custom-output-path-default-absent" = {
    expr = builtins.hasAttr kdlKey customEtc;
    expected = false;
  };
  "niri-vm-borders/wayland-proxy-border-default-uses-resolved-colors" = {
    expr = {
      target = flagValue "--target" editorProxy.argv;
      providerKind = flagValue "--provider-kind" editorProxy.argv;
      realmId = flagValue "--realm-id" editorProxy.argv;
      workloadId = flagValue "--workload-id" editorProxy.argv;
      providerId = flagValue "--provider-id" editorProxy.argv;
      noPathArgs =
        !(builtins.elem "--listen" editorProxy.argv)
        && !(builtins.elem "--connect" editorProxy.argv);
    };
    expected = {
      target = editor.canonicalTarget;
      providerKind = "local-vm";
      realmId = editor.realmId;
      workloadId = editor.workloadId;
      providerId = editorDisplay.providerId;
      noPathArgs = true;
    };
  };
  "niri-vm-borders/wayland-proxy-border-disable-omits-border-flags" = {
    expr = lib.all (flag: !(builtins.elem flag editorProxy.argv)) borderFlags;
    expected = true;
  };
  "niri-vm-borders/realm-json-key-present" = {
    expr = builtins.hasAttr "work" uiJson.realms;
    expected = true;
  };
  "niri-vm-borders/realm-json-has-path" = {
    expr = uiJson.realms.work.path;
    expected = "work.local-root";
  };
  "niri-vm-borders/realm-json-deterministic-accent" = {
    expr = uiJson.realms.work.accent;
    expected = "#ff8800";
  };
  "niri-vm-borders/realm-json-custom-accent" = {
    expr = uiJson.realms.work.accent;
    expected = "#ff8800";
  };
  "niri-vm-borders/realm-json-custom-accent-normalized" = {
    expr = uiJson.realms.work.accent;
    expected = "#ff8800";
  };
  "niri-vm-borders/realm-css-present" = {
    expr = lib.hasInfix "@define-color d2b_realm_work_accent #ff8800;" uiCss;
    expected = true;
  };
  "niri-vm-borders/realm-css-custom-color-verbatim" = {
    expr = lib.hasInfix "#ff8800" uiCss;
    expected = true;
  };
  "niri-vm-borders/realm-css-hyphen-to-underscore" = {
    expr = lib.hasInfix "d2b_realm_work_accent" uiCss;
    expected = true;
  };
  "niri-vm-borders/realm-disabled-omitted-from-json" = {
    expr = !(builtins.hasAttr "disabled" uiJson.realms);
    expected = true;
  };
  "niri-vm-borders/realm-json-empty-when-no-realms" = {
    expr = builtins.length (lib.attrNames uiJson.realms);
    expected = 1;
  };
  "niri-vm-borders/wayland-proxy-realm-workload-active-color-is-realm-accent" = {
    expr = editorBorder.active == uiJson.realms.work.accent;
    expected = true;
  };
  "niri-vm-borders/wayland-proxy-realm-workload-label-is-workload-realmpath" = {
    expr = flagValue "--title-prefix" editorProxy.argv;
    expected = "[editor.work.local-root.d2b] ";
  };
  "niri-vm-borders/wayland-proxy-realm-workload-target-is-canonical" = {
    expr = flagValue "--target" editorProxy.argv;
    expected = "editor.work.local-root.d2b";
  };
  "niri-vm-borders/wayland-proxy-realm-explicit-label-override-preserved" = {
    expr = flagValue "--app-id-prefix" editorProxy.argv;
    expected = "d2b.${editor.workloadId}.";
  };
  "niri-vm-borders/wayland-proxy-realm-explicit-label-override-target-unchanged" = {
    expr = flagValue "--target" mediaProxy.argv;
    expected = "media.work.local-root.d2b";
  };
  "niri-vm-borders/wayland-proxy-ambiguous-realm-uses-vm-defaults" = {
    expr = {
      mappings = builtins.length index.providerRegistryV2Mappings.display;
      uniqueProviderIds =
        builtins.length (lib.unique
          (map (mapping: mapping.providerId)
            index.providerRegistryV2Mappings.display));
      configuredAuthorityShared =
        editor.providerBindings.display.providerId
        == media.providerBindings.display.providerId;
    };
    expected = {
      mappings = 2;
      uniqueProviderIds = 2;
      configuredAuthorityShared = true;
    };
  };
  "niri-vm-borders/processes-use-normalized-wayland-endpoints" = {
    expr = {
      editorReady = (builtins.head editorProxy.readiness).value;
      editorGpuSocket = flagValue "--wayland-sock" editorGpu.argv;
      mediaReady = (builtins.head mediaProxy.readiness).value;
      mediaRuntimeDir = builtins.elem
        "XDG_RUNTIME_DIR=${lib.removeSuffix "/wayland-0" mediaEndpoint.path}"
        mediaRunner.env;
      mediaDisplay = builtins.elem "WAYLAND_DISPLAY=wayland-0" mediaRunner.env;
    };
    expected = {
      editorReady = editorEndpoint.path;
      editorGpuSocket = editorEndpoint.path;
      mediaReady = mediaEndpoint.path;
      mediaRuntimeDir = true;
      mediaDisplay = true;
    };
  };
}
)
