# Nix eval tests for d2b.vms.<name>.usb.securityKey.enable gating.
#
# Tests cover:
#   1. Default-off: manifest.securityKey = false when option not set.
#   2. Enabled: manifest.securityKey = true when option is true.
#   3. No host DAG node claims the uncomposed security-key controller.
#   4. Guest frontend remains fail-closed without runtime session material.
#   5. Assertion fires for security-key + qemu-media conflict.
#   6. Assertion fires for security-key + usbip.yubikey conflict.
{ mkEval, lib, ... }:

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
      yubikey.enable = false;
    };
    d2b.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    d2b.vms.corp-vm = {
      enable = true;
      env = "work";
      index = 10;
      ssh.user = "alice";
      guest.control.enable = true;
      config = {
        networking.hostName = lib.mkDefault "corp-vm";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
  };

  evalWith = overrides: mkEval ([ base ] ++ overrides);

  defaultEval = evalWith [ ];
  enabledEval = evalWith [
    ({ ... }: {
      d2b.host.usb.securityKey.enable = true;
      d2b.vms.corp-vm.usb.securityKey.enable = true;
    })
  ];

  # processes.json helpers
  procs = cfg: cfg.config.d2b._bundle.processesJson.data;
  vmNodes = cfg: vm:
    (builtins.head (builtins.filter (v: v.vm == vm) (procs cfg).vms)).nodes;
  hasNodeId = cfg: vm: id:
    builtins.any (n: n.id == id) (vmNodes cfg vm);

  # manifest helpers: parse vms.json text (the manifest per-VM entry)
  manifestData = cfg: builtins.fromJSON cfg.config.d2b._manifestPkg.text;
  vmManifest = cfg: vm: (manifestData cfg).${vm};
  guestConfig = cfg: vm: cfg.config.d2b._computed.${vm}.config;

  # assertion helper: tryEval + check failing assertion message
  mkEvalCfg = overrides: (evalWith overrides).config;
  assertionFires = overrides: substr:
    let
      cfg = builtins.tryEval (mkEvalCfg overrides);
    in
    if cfg.success then
      lib.any (a: !a.assertion && lib.hasInfix substr a.message)
        cfg.value.assertions
    else
      false;

in
{
  # --- default (option not set) ---

  "security-key-gating/default-manifest-false" = {
    expr = (vmManifest defaultEval "corp-vm").securityKey;
    expected = false;
  };

  "security-key-gating/default-dag-node-absent" = {
    expr = hasNodeId defaultEval "corp-vm" "sk-frontend";
    expected = false;
  };

  # --- enabled ---

  "security-key-gating/enabled-manifest-true" = {
    expr = (vmManifest enabledEval "corp-vm").securityKey;
    expected = true;
  };

  "security-key-gating/enabled-host-dag-node-absent" = {
    expr = hasNodeId enabledEval "corp-vm" "sk-frontend";
    expected = false;
  };

  "security-key-gating/guest-frontend-awaits-runtime-session-material" = {
    expr =
      let
        service =
          (guestConfig enabledEval "corp-vm").systemd.services.d2b-sk-frontend;
      in {
        unitPresent = service.serviceConfig.ExecStart != null;
        channelBindingAbsent =
          !(service.environment ? D2B_SK_CHANNEL_BINDING_HEX);
        reconnectGenerationAbsent =
          !(service.environment ? D2B_SK_RECONNECT_GENERATION);
      };
    expected = {
      unitPresent = true;
      channelBindingAbsent = true;
      reconnectGenerationAbsent = true;
    };
  };

  "security-key-gating/guest-udev-uses-plugdev" = {
    expr =
      let rules = (guestConfig enabledEval "corp-vm").services.udev.extraRules;
      in
      lib.hasInfix ''KERNELS=="0003:1050:0407.*"'' rules
      && lib.hasInfix ''GROUP="plugdev"'' rules;
    expected = true;
  };

  "security-key-gating/guest-declares-plugdev-group" = {
    expr = (guestConfig enabledEval "corp-vm").users.groups ? plugdev;
    expected = true;
  };

  # --- assertion: usbip.yubikey + security-key conflict ---

  "security-key-gating/yubikey-conflict-fires" = {
    expr = assertionFires [
      ({ ... }: {
        d2b.vms.corp-vm.usbip.yubikey = true;
        d2b.vms.corp-vm.usb.securityKey.enable = true;
      })
    ] "usbip.yubikey = true and";
    expected = true;
  };

  # --- assertion: qemu-media + security-key conflict ---

  "security-key-gating/qemu-media-conflict-fires" = {
    expr = assertionFires [
      ({ ... }: {
        d2b.vms.corp-vm.runtime.kind = "qemu-media";
        d2b.vms.corp-vm.usb.securityKey.enable = true;
      })
    ] "runtime.kind = \"qemu-media\" is incompatible";
    expected = true;
  };
}
