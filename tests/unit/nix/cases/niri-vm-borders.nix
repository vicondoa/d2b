# nix-unit cases migrated from tests/niri-vm-borders-eval.sh (group D).
#
# Opt-in niri window-rule include generation: disabled by default; when
# enabled, the KDL at config.environment.etc."nixling/niri-vm-borders.kdl"
# carries a per-graphics-VM border rule (and none for headless VMs), the
# qemu-media host-window border rule, the crosvm scanout-window hide rule,
# and the include-path comment; per-VM color overrides appear verbatim; the
# default palette color is the stable deterministic derivation; a custom
# outputPath relocates the file.
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
    nixling.vms.media = {
      runtime.kind = "qemu-media";
      env = "work";
      index = 12;
    };
  };

  etcOf = overrides: (mkEval ([ base ] ++ overrides)).config.environment.etc;
  kdlKey = "nixling/niri-vm-borders.kdl";
  kdlText = etc: if builtins.hasAttr kdlKey etc then etc.${kdlKey}.text else "";

  disabledEtc = etcOf [ ];
  enabledEtc = etcOf [ ({ ... }: { nixling.site.niriVmBorders.enable = true; }) ];
  enabledKdl = kdlText enabledEtc;
  colorKdl = kdlText (etcOf [
    ({ ... }: {
      nixling.site.niriVmBorders.enable = true;
      nixling.vms.work.graphics.niriBorderColor = "#aabbcc";
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
    expr = lib.hasInfix ''match title=r#"^nixling-media-qemu-media$"#'' enabledKdl;
    expected = true;
  };
  "niri-vm-borders/enabled-qemu-media-no-guest-app-id-rule" = {
    expr = lib.hasInfix ''match app-id=r#"^nixling\.media\."#'' enabledKdl;
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
    expr = lib.hasInfix ''"#aabbcc"'' colorKdl;
    expected = true;
  };
  "niri-vm-borders/qemu-media-color-override-verbatim" = {
    expr =
      lib.hasInfix ''match title=r#"^nixling-media-qemu-media$"#'' qemuMediaColorKdl
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
