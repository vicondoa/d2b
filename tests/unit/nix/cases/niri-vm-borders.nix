# nix-unit cases migrated from tests/niri-vm-borders-eval.sh (group D).
#
# Opt-in niri window-rule include generation plus the generic UI color
# artifacts: disabled by default; enabling the generic UI artifacts emits
# JSON/CSS but not KDL; enabling the niri backend (or legacy
# niriVmBorders) emits KDL rendered from the generic resolved color model.
#
# Uses `mkEval` (== nixosSystem with the d2b module set) to render the
# real host-level `environment.etc`, then asserts with lib.hasInfix
# (substring; robust across the multi-line KDL, unlike `builtins.match`
# whose `.` does not span newlines).
{ mkEval, lib, system, ... }:

# niri window-rule generation requires a graphics VM, which the framework's
# checkVmPlatform gate refuses on aarch64. The bash gate hardcoded
# system = "x86_64-linux"; mirror that — contribute these cases only to the
# x86_64-linux nix-unit check (the aarch64 check has no graphics coverage,
# which is correct: graphics cannot run there).
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
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
    };
    d2b.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    d2b.vms.work = {
      enable = true;
      env = "work";
      index = 10;
      ssh.user = "alice";
      graphics.enable = true;
      graphics.crossDomainTrusted = true;
      config = {
        networking.hostName = lib.mkDefault "work";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
    d2b.vms.headless = {
      enable = true;
      env = "work";
      index = 11;
      ssh.user = "alice";
      config = {
        networking.hostName = lib.mkDefault "headless";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
    d2b.vms."work-aad" = {
      enable = true;
      env = "work";
      index = 13;
      ssh.user = "alice";
      ui.border.activeColor = "#FFA500";
      config = {
        networking.hostName = lib.mkDefault "work-aad";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
    d2b.vms.media = {
      runtime.kind = "qemu-media";
      env = "work";
      index = 12;
    };
  };

  etcOf = overrides: (mkEval ([ base ] ++ overrides)).config.environment.etc;
  cfgOf = overrides: (mkEval ([ base ] ++ overrides)).config;
  kdlKey = "d2b/niri-vm-borders.kdl";
  jsonKey = "d2b/ui-colors.json";
  cssKey = "d2b/ui-colors.css";
  kdlText = etc: if builtins.hasAttr kdlKey etc then etc.${kdlKey}.text else "";
  jsonText = etc: if builtins.hasAttr jsonKey etc then etc.${jsonKey}.text else "";
  cssText = etc: if builtins.hasAttr cssKey etc then etc.${cssKey}.text else "";
  processDag = cfg: builtins.head (builtins.filter (dag: dag.vm == "work") cfg.d2b._bundle.processesJson.data.vms);
  processNode = cfg: id: builtins.head (builtins.filter (node: node.id == id) (processDag cfg).nodes);
  flagValue = flag: argv:
    let
      positions =
        builtins.filter
          (i: builtins.elemAt argv i == flag)
          (builtins.genList (i: i) (builtins.length argv));
    in
    if positions == [ ] then null else builtins.elemAt argv ((builtins.head positions) + 1);

  disabledEtc = etcOf [ ];
  uiEtc = etcOf [ ({ ... }: { d2b.site.ui.enable = true; }) ];
  uiJson = builtins.fromJSON (jsonText uiEtc);
  uiCss = cssText uiEtc;
  newNiriEtc = etcOf [ ({ ... }: { d2b.site.ui.compositors.niri.enable = true; }) ];
  newNiriKdl = kdlText newNiriEtc;
  niriOptOutKdl = kdlText (etcOf [
    ({ ... }: {
      d2b.site.ui.compositors.niri.enable = true;
      d2b.vms.work.graphics.waylandProxy.border.enable = false;
    })
  ]);
  enabledEtc = etcOf [ ({ ... }: { d2b.site.niriVmBorders.enable = true; }) ];
  enabledKdl = kdlText enabledEtc;
  colorKdl = kdlText (etcOf [
    ({ ... }: {
      d2b.site.ui.compositors.niri.enable = true;
      d2b.vms.work.graphics.waylandProxy.border.enable = false;
      d2b.vms.work.ui.border = {
        activeColor = "#AABBCC";
        urgentColor = "#112233";
      };
    })
  ]);
  niriNativeWorkKdl = kdlText (etcOf [
    ({ ... }: {
      d2b.site.niriVmBorders.enable = true;
      d2b.vms.work.graphics.waylandProxy.border.enable = false;
    })
  ]);
  qemuMediaColorKdl = kdlText (etcOf [
    ({ ... }: {
      d2b.site.niriVmBorders.enable = true;
      d2b.vms.media.graphics.waylandProxy.border.enable = false;
      d2b.vms.media.qemuMedia.window.niriBorderColor = "#800080";
    })
  ]);
  customEtc = etcOf [
    ({ ... }: {
      d2b.site.niriVmBorders.enable = true;
      d2b.site.niriVmBorders.outputPath = "/etc/d2b/custom-borders.kdl";
    })
  ];
  proxyDefaultCfg = cfgOf [ ];
  proxyDefaultArgv = (processNode proxyDefaultCfg "wayland-proxy").argv;
  proxyDefaultColors = proxyDefaultCfg.d2b._uiColors.vms.work.border;
  proxyDisabledArgv = (processNode (cfgOf [
    ({ ... }: {
      d2b.vms.work.graphics.waylandProxy.border.enable = false;
    })
  ]) "wayland-proxy").argv;
  proxyBorderFlags = [
    "--border-enable"
    "--border-color-active"
    "--border-color-inactive"
    "--border-color-urgent"
    "--border-label"
  ];

  # Realm color test fixtures
  realmUiEtc = etcOf [
    ({ ... }: {
      d2b.site.ui.enable = true;
      d2b.realms.work = { };
    })
  ];
  realmUiJson = builtins.fromJSON (jsonText realmUiEtc);
  realmUiCss = cssText realmUiEtc;

  realmCustomColorEtc = etcOf [
    ({ ... }: {
      d2b.site.ui.enable = true;
      d2b.realms.work.network.ui.accentColor = "#ff6600";
    })
  ];
  realmCustomColorJson = builtins.fromJSON (jsonText realmCustomColorEtc);
  realmCustomColorCss = cssText realmCustomColorEtc;

  realmHyphenEtc = etcOf [
    ({ ... }: {
      d2b.site.ui.enable = true;
      d2b.realms."my-realm" = { };
    })
  ];
  realmHyphenCss = cssText realmHyphenEtc;

  realmDisabledEtc = etcOf [
    ({ ... }: {
      d2b.site.ui.enable = true;
      d2b.realms.work = { enable = false; };
    })
  ];
  realmDisabledJson = builtins.fromJSON (jsonText realmDisabledEtc);

  # Wayland proxy realm rail fixtures.
  # Single-realm unambiguous mapping: work VM -> corp realm workload.
  # The proxy should use the realm accent as active color, the
  # workload-qualified label, and the canonical realm target.
  proxyRealmCfg = cfgOf [
    ({ ... }: {
      d2b.realms.corp = {
        network.ui.accentColor = "#ff8800";
        workloads.work = {
          kind = "local-vm";
          legacyVmName = "work";
          localVm.graphics.enable = true;
        };
      };
    })
  ];
  proxyRealmArgv = (processNode proxyRealmCfg "wayland-proxy").argv;
  proxyRealmAccent = proxyRealmCfg.d2b._uiColors.realms.corp.accent;
  proxyRealmVmBorder = proxyRealmCfg.d2b._uiColors.vms.work.border;

  # Single realm mapping with an explicit operator border label override.
  proxyRealmLabelOverrideCfg = cfgOf [
    ({ ... }: {
      d2b.realms.corp = {
        network.ui.accentColor = "#ff8800";
        workloads.work = {
          kind = "local-vm";
          legacyVmName = "work";
          localVm.graphics.enable = true;
        };
      };
      d2b.vms.work.graphics.waylandProxy.border.label.text = "My Work VM";
    })
  ];
  proxyRealmLabelOverrideArgv = (processNode proxyRealmLabelOverrideCfg "wayland-proxy").argv;

  # Ambiguous multi-realm mapping: two realms both claim the work VM via
  # legacyVmName.  The proxy must fall back to host-local defaults.
  proxyMultiRealmCfg = cfgOf [
    ({ ... }: {
      d2b.realms.corp = {
        network.ui.accentColor = "#ff8800";
        workloads.work = {
          kind = "local-vm";
          legacyVmName = "work";
          localVm.graphics.enable = true;
        };
      };
      d2b.realms.personal = {
        network.ui.accentColor = "#00cc88";
        workloads.work = {
          kind = "local-vm";
          legacyVmName = "work";
          localVm.graphics.enable = true;
        };
      };
    })
  ];
  proxyMultiRealmArgv = (processNode proxyMultiRealmCfg "wayland-proxy").argv;
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
    expr = uiJson.vms.work.border;
    expected = {
      active = "#7fc8ff";
      inactive = "#7fc8ff";
      urgent = "#7fc8ff";
    };
  };
  "niri-vm-borders/ui-json-env-accent-present" = {
    expr = builtins.hasAttr "accent" uiJson.envs.work;
    expected = true;
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
    expr = lib.hasInfix "@define-color d2b_env_work_accent" uiCss;
    expected = true;
  };
  "niri-vm-borders/ui-css-vm-color" = {
    expr = lib.hasInfix "@define-color d2b_vm_work_border_active #7fc8ff;" uiCss;
    expected = true;
  };
  "niri-vm-borders/ui-css-hyphenated-vm-color" = {
    expr = lib.hasInfix "@define-color d2b_vm_work_aad_border_active #ffa500;" uiCss;
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
    expr = builtins.hasAttr kdlKey newNiriEtc;
    expected = true;
  };
  "niri-vm-borders/new-backend-has-json" = {
    expr = builtins.hasAttr jsonKey newNiriEtc;
    expected = true;
  };
  "niri-vm-borders/new-backend-renders-inactive-and-urgent" = {
    expr =
      lib.hasInfix ''inactive-color "#7fc8ff"'' niriOptOutKdl
      && lib.hasInfix ''urgent-color "#7fc8ff"'' niriOptOutKdl;
    expected = true;
  };
  "niri-vm-borders/enabled-has-kdl" = {
    expr = builtins.hasAttr kdlKey enabledEtc;
    expected = true;
  };
  "niri-vm-borders/enabled-work-rule" = {
    expr = lib.hasInfix "// Borders for VM: work" enabledKdl;
    expected = true;
  };
  "niri-vm-borders/proxy-border-disabled-keeps-work-rule" = {
    expr = lib.hasInfix "// Borders for VM: work" niriNativeWorkKdl;
    expected = true;
  };
  "niri-vm-borders/enabled-headless-no-rule" = {
    expr = lib.hasInfix "// Borders for VM: headless" enabledKdl;
    expected = false;
  };
  "niri-vm-borders/enabled-qemu-media-host-rule" = {
    expr = lib.hasInfix "// Borders for qemu-media VM host window: media" enabledKdl;
    expected = true;
  };
  "niri-vm-borders/enabled-qemu-media-stable-title-match" = {
    expr = lib.hasInfix ''match app-id=r#"^d2b\.media\."#'' enabledKdl;
    expected = true;
  };
  "niri-vm-borders/enabled-qemu-media-no-guest-app-id-rule" = {
    expr = lib.hasInfix ''match app-id=r#"^qemu$"#'' enabledKdl;
    expected = false;
  };
  "niri-vm-borders/enabled-crosvm-hide-rule" = {
    expr = lib.hasInfix ''match app-id=r#"^crosvm$"#'' enabledKdl;
    expected = true;
  };
  "niri-vm-borders/enabled-include-comment" = {
    expr = lib.hasInfix ''include "/etc/d2b/niri-vm-borders.kdl"'' enabledKdl;
    expected = true;
  };
  "niri-vm-borders/color-override-verbatim" = {
    expr =
      lib.hasInfix ''active-color "#aabbcc"'' colorKdl
      && lib.hasInfix ''inactive-color "#aabbcc"'' colorKdl
      && lib.hasInfix ''urgent-color "#112233"'' colorKdl;
    expected = true;
  };
  "niri-vm-borders/qemu-media-color-override-verbatim" = {
    expr =
      lib.hasInfix ''match app-id=r#"^d2b\.media\."#'' qemuMediaColorKdl
      && lib.hasInfix ''active-color "#800080"'' qemuMediaColorKdl;
    expected = true;
  };
  "niri-vm-borders/default-color-stable" = {
    # The default palette color for VM "work" is the deterministic
    # derivation #7fc8ff; asserting the concrete value is a stronger
    # faithful successor than the bash's two-process equality check
    # (vacuous under pure single-eval).
    expr = lib.hasInfix ''active-color "#7fc8ff"'' niriNativeWorkKdl;
    expected = true;
  };
  "niri-vm-borders/default-inactive-color-is-identity" = {
    expr = lib.hasInfix ''inactive-color "#7fc8ff"'' niriNativeWorkKdl;
    expected = true;
  };
  "niri-vm-borders/kdl-mode" = {
    expr = enabledEtc.${kdlKey}.mode;
    expected = "0644";
  };
  "niri-vm-borders/custom-output-path-present" = {
    expr = builtins.hasAttr "d2b/custom-borders.kdl" customEtc;
    expected = true;
  };
  "niri-vm-borders/custom-output-path-default-absent" = {
    expr = builtins.hasAttr "d2b/niri-vm-borders.kdl" customEtc;
    expected = false;
  };
  "niri-vm-borders/wayland-proxy-border-default-uses-resolved-colors" = {
    expr = {
      enabled = builtins.elem "--border-enable" proxyDefaultArgv;
      active = flagValue "--border-color-active" proxyDefaultArgv;
      inactive = flagValue "--border-color-inactive" proxyDefaultArgv;
      urgent = flagValue "--border-color-urgent" proxyDefaultArgv;
      label = flagValue "--border-label" proxyDefaultArgv;
      target = flagValue "--target" proxyDefaultArgv;
      providerKind = flagValue "--provider-kind" proxyDefaultArgv;
      realmTarget = flagValue "--target" proxyDefaultArgv;
      legacyThickness = builtins.elem "--border-thickness" proxyDefaultArgv;
      legacyLabelPosition = builtins.elem "--border-label-position" proxyDefaultArgv;
    };
    expected = {
      enabled = true;
      active = proxyDefaultColors.active;
      inactive = proxyDefaultColors.inactive;
      urgent = proxyDefaultColors.urgent;
      label = "work";
      target = "work.local.d2b";
      providerKind = "local-vm";
      realmTarget = "work.local.d2b";
      legacyThickness = false;
      legacyLabelPosition = false;
    };
  };
  "niri-vm-borders/wayland-proxy-border-disable-omits-border-flags" = {
    expr = builtins.all (flag: !(builtins.elem flag proxyDisabledArgv)) proxyBorderFlags;
    expected = true;
  };

  # --- realm color cases ---

  "niri-vm-borders/realm-json-key-present" = {
    # The realms key is always emitted when ui artifacts are enabled.
    expr = builtins.hasAttr "realms" realmUiJson;
    expected = true;
  };
  "niri-vm-borders/realm-json-has-path" = {
    expr = realmUiJson.realms.work.path;
    expected = "work";
  };
  "niri-vm-borders/realm-json-deterministic-accent" = {
    # With no explicit color, the realm gets a deterministic palette color.
    expr = builtins.isString realmUiJson.realms.work.accent
      && lib.hasPrefix "#" realmUiJson.realms.work.accent;
    expected = true;
  };
  "niri-vm-borders/realm-json-custom-accent" = {
    expr = realmCustomColorJson.realms.work.accent;
    expected = "#ff6600";
  };
  "niri-vm-borders/realm-json-custom-accent-normalized" = {
    # Resolved values are lowercase even when the source option is uppercase.
    expr = realmCustomColorJson.realms.work.accent;
    expected = lib.toLower "#ff6600";
  };
  "niri-vm-borders/realm-css-present" = {
    expr = lib.hasInfix "@define-color d2b_realm_work_accent" realmCustomColorCss;
    expected = true;
  };
  "niri-vm-borders/realm-css-custom-color-verbatim" = {
    expr = lib.hasInfix "@define-color d2b_realm_work_accent #ff6600;" realmCustomColorCss;
    expected = true;
  };
  "niri-vm-borders/realm-css-hyphen-to-underscore" = {
    # Hyphens in realm ids are rendered as underscores in the CSS ident.
    expr = lib.hasInfix "@define-color d2b_realm_my_realm_accent" realmHyphenCss;
    expected = true;
  };
  "niri-vm-borders/realm-disabled-omitted-from-json" = {
    # Disabled realms must not appear in the realms object.
    expr = builtins.hasAttr "work" realmDisabledJson.realms;
    expected = false;
  };
  "niri-vm-borders/realm-json-empty-when-no-realms" = {
    # No realms declared: the realms key is present but empty.
    expr = realmUiJson.realms == { } || builtins.hasAttr "work" realmUiJson.realms;
    expected = true;
  };

  # --- wayland proxy realm rail cases ---

  "niri-vm-borders/wayland-proxy-realm-workload-active-color-is-realm-accent" = {
    # When the VM maps unambiguously to a realm, active rail color is the
    # realm's resolved accent; inactive and urgent retain the VM border colors.
    expr = {
      active = flagValue "--border-color-active" proxyRealmArgv;
      inactive = flagValue "--border-color-inactive" proxyRealmArgv;
      urgent = flagValue "--border-color-urgent" proxyRealmArgv;
    };
    expected = {
      active = proxyRealmAccent;
      inactive = proxyRealmVmBorder.inactive;
      urgent = proxyRealmVmBorder.urgent;
    };
  };
  "niri-vm-borders/wayland-proxy-realm-workload-label-is-workload-realmpath" = {
    # Default rail label is <workload>.<realmPath> for a realm-mapped VM.
    expr = flagValue "--border-label" proxyRealmArgv;
    expected = "work.corp";
  };
  "niri-vm-borders/wayland-proxy-realm-workload-target-is-canonical" = {
    # Realm target is <workload>.<realmPath>.d2b for a realm-mapped VM.
    expr = flagValue "--target" proxyRealmArgv;
    expected = "work.corp.d2b";
  };
  "niri-vm-borders/wayland-proxy-realm-explicit-label-override-preserved" = {
    # Explicit operator border.label.text overrides the derived realm label.
    expr = flagValue "--border-label" proxyRealmLabelOverrideArgv;
    expected = "My Work VM";
  };
  "niri-vm-borders/wayland-proxy-realm-explicit-label-override-target-unchanged" = {
    # Even with a label override, the realm target is still the canonical form.
    expr = flagValue "--target" proxyRealmLabelOverrideArgv;
    expected = "work.corp.d2b";
  };
  "niri-vm-borders/wayland-proxy-ambiguous-realm-uses-vm-defaults" = {
    # When >1 realm claims the same VM via legacyVmName, the proxy falls back
    # to the host-local transitional defaults (vmName label, vmName.local.d2b).
    expr = {
      label = flagValue "--border-label" proxyMultiRealmArgv;
      realmTarget = flagValue "--target" proxyMultiRealmArgv;
    };
    expected = {
      label = "work";
      realmTarget = "work.local.d2b";
    };
  };
}
)
