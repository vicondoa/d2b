# nix-unit cases migrated from tests/niri-vm-borders-eval.sh (group D).
#
# Opt-in niri window-rule include generation plus the generic UI color
# artifacts: disabled by default; enabling the generic UI artifacts emits
# JSON/CSS but not KDL; enabling the niri backend (or legacy
# niriVmBorders) emits KDL rendered from the generic resolved color model.
#
# Uses `mkEval` (== nixosSystem with the nixling module set) to render the
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
    nixling.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
    };
    nixling.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    nixling.vms.work = {
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
    nixling.vms.headless = {
      enable = true;
      env = "work";
      index = 11;
      ssh.user = "alice";
      config = {
        networking.hostName = lib.mkDefault "headless";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
    nixling.vms."work-aad" = {
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
    nixling.vms.media = {
      runtime.kind = "qemu-media";
      env = "work";
      index = 12;
    };
  };

  etcOf = overrides: (mkEval ([ base ] ++ overrides)).config.environment.etc;
  kdlKey = "nixling/niri-vm-borders.kdl";
  jsonKey = "nixling/ui-colors.json";
  cssKey = "nixling/ui-colors.css";
  kdlText = etc: if builtins.hasAttr kdlKey etc then etc.${kdlKey}.text else "";
  jsonText = etc: if builtins.hasAttr jsonKey etc then etc.${jsonKey}.text else "";
  cssText = etc: if builtins.hasAttr cssKey etc then etc.${cssKey}.text else "";

  disabledEtc = etcOf [ ];
  uiEtc = etcOf [ ({ ... }: { nixling.site.ui.enable = true; }) ];
  uiJson = builtins.fromJSON (jsonText uiEtc);
  uiCss = cssText uiEtc;
  newNiriEtc = etcOf [ ({ ... }: { nixling.site.ui.compositors.niri.enable = true; }) ];
  newNiriKdl = kdlText newNiriEtc;
  enabledEtc = etcOf [ ({ ... }: { nixling.site.niriVmBorders.enable = true; }) ];
  enabledKdl = kdlText enabledEtc;
  colorKdl = kdlText (etcOf [
    ({ ... }: {
      nixling.site.ui.compositors.niri.enable = true;
      nixling.vms.work.ui.border = {
        activeColor = "#AABBCC";
        urgentColor = "#112233";
      };
    })
  ]);
  qemuMediaColorKdl = kdlText (etcOf [
    ({ ... }: {
      nixling.site.niriVmBorders.enable = true;
      nixling.vms.media.qemuMedia.window.niriBorderColor = "#800080";
    })
  ]);
  customEtc = etcOf [
    ({ ... }: {
      nixling.site.niriVmBorders.enable = true;
      nixling.site.niriVmBorders.outputPath = "/etc/nixling/custom-borders.kdl";
    })
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
    expr = uiJson.vms.work.border;
    expected = {
      active = "#ffa07a";
      inactive = "#ffa07a";
      urgent = "#ffa07a";
    };
  };
  "niri-vm-borders/ui-json-env-accent-present" = {
    expr = builtins.hasAttr "accent" uiJson.envs.work;
    expected = true;
  };
  "niri-vm-borders/ui-css-host-color" = {
    expr = lib.hasInfix "@define-color nixling_host_accent #89b4fa;" uiCss;
    expected = true;
  };
  "niri-vm-borders/ui-css-state-color" = {
    expr = lib.hasInfix "@define-color nixling_state_running #a6e3a1;" uiCss;
    expected = true;
  };
  "niri-vm-borders/ui-css-env-color" = {
    expr = lib.hasInfix "@define-color nixling_env_work_accent" uiCss;
    expected = true;
  };
  "niri-vm-borders/ui-css-vm-color" = {
    expr = lib.hasInfix "@define-color nixling_vm_work_border_active #ffa07a;" uiCss;
    expected = true;
  };
  "niri-vm-borders/ui-css-hyphenated-vm-color" = {
    expr = lib.hasInfix "@define-color nixling_vm_work_aad_border_active #ffa500;" uiCss;
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
      lib.hasInfix ''inactive-color "#ffa07a"'' newNiriKdl
      && lib.hasInfix ''urgent-color "#ffa07a"'' newNiriKdl;
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
  "niri-vm-borders/enabled-headless-no-rule" = {
    expr = lib.hasInfix "// Borders for VM: headless" enabledKdl;
    expected = false;
  };
  "niri-vm-borders/enabled-qemu-media-host-rule" = {
    expr = lib.hasInfix "// Borders for qemu-media VM host window: media" enabledKdl;
    expected = true;
  };
  "niri-vm-borders/enabled-qemu-media-stable-title-match" = {
    expr = lib.hasInfix ''match app-id=r#"^nixling\.media\."#'' enabledKdl;
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
    expr = lib.hasInfix ''include "/etc/nixling/niri-vm-borders.kdl"'' enabledKdl;
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
      lib.hasInfix ''match app-id=r#"^nixling\.media\."#'' qemuMediaColorKdl
      && lib.hasInfix ''active-color "#800080"'' qemuMediaColorKdl;
    expected = true;
  };
  "niri-vm-borders/default-color-stable" = {
    # The default palette color for VM "work" is the deterministic
    # derivation #ffa07a; asserting the concrete value is a stronger
    # faithful successor than the bash's two-process equality check
    # (vacuous under pure single-eval).
    expr = lib.hasInfix ''active-color "#ffa07a"'' enabledKdl;
    expected = true;
  };
  "niri-vm-borders/default-inactive-color-is-identity" = {
    expr = lib.hasInfix ''inactive-color "#ffa07a"'' enabledKdl;
    expected = true;
  };
  "niri-vm-borders/kdl-mode" = {
    expr = enabledEtc.${kdlKey}.mode;
    expected = "0644";
  };
  "niri-vm-borders/custom-output-path-present" = {
    expr = builtins.hasAttr "nixling/custom-borders.kdl" customEtc;
    expected = true;
  };
  "niri-vm-borders/custom-output-path-default-absent" = {
    expr = builtins.hasAttr "nixling/niri-vm-borders.kdl" customEtc;
    expected = false;
  };
}
)
