# nix-unit eval cases for the USB security-key proxy option schema
# (nixos-modules/options-host.nix + per-VM usb.securityKey.enable).
#
# Tests three properties:
#   A. Positive: valid host + VM config evaluates without assertion
#      failures; option values are set as expected.
#   B. The eval-time assertion fires correctly for each of the three
#      assertion categories (checked here as boolean expressions over
#      `config.assertions`, not via mkBatch — the batch evaluator in
#      eval-cases/assertions.nix covers the failure-message surface).
#   C. Host-enabled with empty devices evaluates without error.
#   D. The option defaults: host disabled, VM disabled.
{ mkEval, lib, ... }:

let
  # Minimal system fixture.
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
      config = {
        networking.hostName = lib.mkDefault "corp-vm";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
  };

  # Valid FIDO device selector fixture.
  fidoDevice = {
    label = "yubikey-primary";
    vendorId = 0x1050; # Yubico
    productId = 0x0407; # YubiKey 5 NFC
    serial = null;
  };

  # --- Eval helpers ---
  evalWith = overrides: mkEval ([ base ] ++ overrides);

  # Evaluate and check that NO assertions fail.
  assertionsOf = sys: sys.config.assertions;
  failingOf = sys:
    lib.filter (a: !a.assertion) (assertionsOf sys);
  hasNoFailures = sys: failingOf sys == [ ];

  # Eval A: host + VM both enabled with a valid device selector.
  validEnabled = evalWith [
    ({ lib, ... }: {
      d2b.host.usb.securityKey.enable = lib.mkForce true;
      d2b.host.usb.securityKey.devices = [ fidoDevice ];
      d2b.vms.corp-vm.usb.securityKey.enable = true;
    })
  ];

  # Eval B-a: VM enabled but host disabled → assertion fails.
  vmEnabledHostDisabled = evalWith [
    ({ lib, ... }: {
      d2b.host.usb.securityKey.enable = lib.mkForce false;
      d2b.vms.corp-vm.usb.securityKey.enable = true;
    })
  ];

  # Eval B-b: both securityKey and usbip.yubikey set on same VM → fails.
  vmBothSkAndUsbip = evalWith [
    ({ lib, ... }: {
      d2b.site.yubikey.enable = lib.mkForce true;
      d2b.host.usb.securityKey.enable = lib.mkForce true;
      d2b.host.usb.securityKey.devices = [ fidoDevice ];
      d2b.vms.corp-vm.usb.securityKey.enable = true;
      d2b.vms.corp-vm.usbip.yubikey = true;
      d2b.vms.corp-vm.guest.control.enable = true;
    })
  ];

  # Eval B-c: non-FIDO vendor in devices → assertion fails.
  nonFidoVendor = evalWith [
    ({ lib, ... }: {
      d2b.host.usb.securityKey.enable = lib.mkForce true;
      d2b.host.usb.securityKey.devices = [
        {
          label = "unknown";
          vendorId = 0x1234; # not in FIDO allowlist
          productId = 0x5678;
        }
      ];
    })
  ];

  # Eval B-d: duplicate labels → assertion fails.
  duplicateLabels = evalWith [
    ({ lib, ... }: {
      d2b.host.usb.securityKey.enable = lib.mkForce true;
      d2b.host.usb.securityKey.devices = [
        { label = "same"; vendorId = 0x1050; productId = 0x0407; }
        { label = "same"; vendorId = 0x1050; productId = 0x0408; }
      ];
    })
  ];

  # Eval C: host enabled, empty devices list — valid.
  hostEnabledEmptyDevices = evalWith [
    ({ lib, ... }: {
      d2b.host.usb.securityKey.enable = lib.mkForce true;
    })
  ];

  # Eval D: default state — both options off.
  defaultState = evalWith [ ];

  assertionMessageContains = sys: needle:
    lib.any
      (a: !a.assertion && lib.hasInfix needle a.message)
      (assertionsOf sys);
in
{
  # --- A: valid enabled config passes all assertions ---
  "usb-security-key/valid-config-no-assertion-failures" = {
    expr = hasNoFailures validEnabled;
    expected = true;
  };

  "usb-security-key/valid-config-host-option-set" = {
    expr = validEnabled.config.d2b.host.usb.securityKey.enable;
    expected = true;
  };

  "usb-security-key/valid-config-vm-option-set" = {
    expr = validEnabled.config.d2b.vms.corp-vm.usb.securityKey.enable;
    expected = true;
  };

  "usb-security-key/valid-config-device-label-present" = {
    expr =
      (builtins.head validEnabled.config.d2b.host.usb.securityKey.devices).label;
    expected = "yubikey-primary";
  };

  "usb-security-key/valid-config-device-vendor-id-correct" = {
    expr =
      (builtins.head validEnabled.config.d2b.host.usb.securityKey.devices).vendorId;
    expected = 0x1050;
  };

  # --- B-a: VM enabled without host enable → assertion fires ---
  "usb-security-key/vm-enabled-host-disabled-fails" = {
    expr = assertionMessageContains vmEnabledHostDisabled
      "d2b.vms.corp-vm.usb.securityKey.enable = true requires";
    expected = true;
  };

  # --- B-b: securityKey + usbip.yubikey mutual exclusion ---
  "usb-security-key/mutual-exclusion-with-usbip-fires" = {
    expr = assertionMessageContains vmBothSkAndUsbip
      "usb.securityKey.enable and usbip.yubikey";
    expected = true;
  };

  # --- B-c: non-FIDO vendor rejected ---
  "usb-security-key/non-fido-vendor-rejected" = {
    expr = assertionMessageContains nonFidoVendor "not in the FIDO-class allowlist";
    expected = true;
  };

  # --- B-d: duplicate labels rejected ---
  "usb-security-key/duplicate-label-rejected" = {
    expr = assertionMessageContains duplicateLabels "duplicate label";
    expected = true;
  };

  # --- C: host enabled, empty devices — no assertion failures ---
  "usb-security-key/host-enabled-empty-devices-valid" = {
    expr = hasNoFailures hostEnabledEmptyDevices;
    expected = true;
  };

  # --- D: default state — options off by default ---
  "usb-security-key/host-default-disabled" = {
    expr = defaultState.config.d2b.host.usb.securityKey.enable;
    expected = false;
  };

  "usb-security-key/vm-default-disabled" = {
    expr = defaultState.config.d2b.vms.corp-vm.usb.securityKey.enable;
    expected = false;
  };

  "usb-security-key/host-default-devices-empty" = {
    expr = defaultState.config.d2b.host.usb.securityKey.devices;
    expected = [ ];
  };
}
