# Nix eval test cases for d2b.host.usb.securityKey and
# d2b.vms.<vm>.usb.securityKey.
#
# These are scaffolded test cases that capture the eval-time invariants
# described in docs/reference/components-usb-security-key.md and the plan.
# They are written against the expected module shape; cases marked
# `expectedError = true` assert eval-rejection of invalid configurations.
#
# SCAFFOLDING NOTE: These cases depend on nixos-modules/components/
# usb-security-key.nix being present (the runtime implementation workstream).
# Until that module exists, running `make test-unit` will fail on eval
# because the options are undeclared. The cases are committed now so that:
#   1. The test surface is defined alongside the docs, not after the fact.
#   2. The implementation workstream can make them pass as a completion gate.
#   3. The policy gate `usb_security_key_test_cases_exist` in
#      packages/d2b-contract-tests/tests/policy_docs.rs can assert this
#      file is present before the module lands.
#
# When the module is implemented, run:
#   bash tests/tools/gen-nix-unit-pins.sh
# to regenerate tests/unit/nix/pinned/ and add this file to the pin list.
{ mkEval, lib, ... }:

let
  # Minimal base NixOS module shared across cases.
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
    d2b.envs.personal = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    d2b.vms.personal-dev = {
      enable = true;
      env = "personal";
      index = 10;
      ssh.user = "alice";
      guest.control.enable = true;
      config = {
        networking.hostName = lib.mkDefault "personal-dev";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
  };

  evalSingle = overrides: mkEval ([ base ] ++ overrides);

  # ---- helper predicates ---------------------------------------------------

  # True when the system has a FIDO udev rule under /run/udev/rules.d/.
  hasFidoUdevRule = sys:
    let
      rules = sys.config.services.udev.extraRules or "";
    in
      lib.hasInfix "FIDO" rules || lib.hasInfix "d2b-fido" rules;

  # True when the named VM bundle key is present in the rendered bundle.
  # This is a proxy check; real contract tests use D2B_FIXTURES.
  hasSecurityKeyCapability = sys: vm:
    let
      vms = sys.config.d2b.vms or { };
    in
      (vms.${vm} or { }).usb.securityKey.enable or false;

  # ---- eval configs --------------------------------------------------------

  # Neither host nor VM configured: disabled.
  disabled = evalSingle [ ];

  # Host enabled, no VM opted in.
  hostOnly = evalSingle [
    ({ lib, ... }: {
      d2b.host.usb.securityKey.enable = lib.mkForce true;
    })
  ];

  # Host and VM both enabled (well-formed).
  bothEnabled = evalSingle [
    ({ lib, ... }: {
      d2b.host.usb.securityKey.enable = lib.mkForce true;
      d2b.vms.personal-dev.usb.securityKey.enable = true;
    })
  ];

  # VM enabled without host: should trigger assertion.
  vmWithoutHost = evalSingle [
    ({ lib, ... }: {
      d2b.host.usb.securityKey.enable = lib.mkForce false;
      d2b.vms.personal-dev.usb.securityKey.enable = true;
    })
  ];

  # VM opts into both security-key and usbip.yubikey: mutual-exclusion should fire.
  mutualExclusion = evalSingle [
    ({ lib, ... }: {
      d2b.host.usb.securityKey.enable = lib.mkForce true;
      d2b.site.yubikey.enable = lib.mkForce true;
      d2b.vms.personal-dev = {
        usb.securityKey.enable = true;
        usbip.yubikey = true;
        usbip.busids = [ "1-2" ];
      };
    })
  ];

  # VM enabled without guest.control: should trigger assertion.
  noGuestControl = evalSingle [
    ({ lib, ... }: {
      d2b.host.usb.securityKey.enable = lib.mkForce true;
      d2b.vms.personal-dev.guest.control.enable = lib.mkForce false;
      d2b.vms.personal-dev.usb.securityKey.enable = true;
    })
  ];

  # Two VMs: both enabled — legal configuration.
  twoVms = evalSingle [
    ({ lib, ... }: {
      d2b.envs.work = {
        lanSubnet = "10.21.0.0/24";
        uplinkSubnet = "198.51.100.0/30";
      };
      d2b.vms.work-aad = {
        enable = true;
        env = "work";
        index = 11;
        ssh.user = "alice";
        guest.control.enable = true;
        usb.securityKey.enable = true;
        config = {
          networking.hostName = lib.mkDefault "work-aad";
          users.users.alice = { isNormalUser = true; uid = 1000; };
        };
      };
      d2b.host.usb.securityKey.enable = lib.mkForce true;
      d2b.vms.personal-dev.usb.securityKey.enable = true;
    })
  ];

in
{
  # --- disabled: nothing opted in ------------------------------------------
  "usb-security-key/disabled-host-option-absent" = {
    expr = (disabled.config.d2b.host.usb.securityKey.enable or false);
    expected = false;
  };
  "usb-security-key/disabled-vm-option-absent" = {
    expr = hasSecurityKeyCapability disabled "personal-dev";
    expected = false;
  };

  # --- host-only: host enabled, no VM opted in -----------------------------
  "usb-security-key/host-only-option-present" = {
    expr = (hostOnly.config.d2b.host.usb.securityKey.enable or false);
    expected = true;
  };
  "usb-security-key/host-only-vm-not-opted-in" = {
    expr = hasSecurityKeyCapability hostOnly "personal-dev";
    expected = false;
  };

  # --- both-enabled: well-formed configuration -----------------------------
  "usb-security-key/both-enabled-host-option-true" = {
    expr = (bothEnabled.config.d2b.host.usb.securityKey.enable or false);
    expected = true;
  };
  "usb-security-key/both-enabled-vm-opted-in" = {
    expr = hasSecurityKeyCapability bothEnabled "personal-dev";
    expected = true;
  };

  # --- vm-without-host: assertion must fire --------------------------------
  # Bucket A: eval succeeds but config.assertions carries the failure.
  "usb-security-key/vm-without-host-assertion-fires" = {
    expr =
      (vmWithoutHost.evalSucceeded or true)
      && lib.any
           (m: lib.hasInfix "usb.securityKey" m && lib.hasInfix "requires" m)
           (vmWithoutHost.config.assertions or [ ]);
    expected = true;
  };

  # --- mutual-exclusion: assertion must fire --------------------------------
  "usb-security-key/mutual-exclusion-assertion-fires" = {
    expr =
      (mutualExclusion.evalSucceeded or true)
      && lib.any
           (m: lib.hasInfix "mutually exclusive" m
               && lib.hasInfix "personal-dev" m)
           (mutualExclusion.config.assertions or [ ]);
    expected = true;
  };

  # --- no-guest-control: assertion must fire --------------------------------
  "usb-security-key/no-guest-control-assertion-fires" = {
    expr =
      (noGuestControl.evalSucceeded or true)
      && lib.any
           (m: lib.hasInfix "guest.control" m)
           (noGuestControl.config.assertions or [ ]);
    expected = true;
  };

  # --- two-vm: both VMs opted in with host enabled — legal config ----------
  "usb-security-key/two-vms-personal-dev-opted-in" = {
    expr = hasSecurityKeyCapability twoVms "personal-dev";
    expected = true;
  };
  "usb-security-key/two-vms-work-aad-opted-in" = {
    expr = hasSecurityKeyCapability twoVms "work-aad";
    expected = true;
  };
  "usb-security-key/two-vms-no-spurious-assertions" = {
    expr = lib.all (a: a.assertion) (twoVms.config.assertions or [ ]);
    expected = true;
  };

  # --- no process or OS markers leak into bundle/artifact text -------------
  # Mirrors the pattern from tests/unit/nix/cases/requested-vm-config.nix.
  "usb-security-key/no-process-markers-in-both-enabled" = {
    expr =
      let
        # Probe the rendered NixOS config as text if possible.
        # We use the option value directly; a real artifact check runs via
        # D2B_FIXTURES in the Type-4 contract tests.
        cfg = builtins.toJSON bothEnabled.config.d2b;
      in
        !(lib.hasInfix "W3fu" cfg)
        && !(lib.hasInfix "P6" cfg)
        && !(lib.hasInfix "ForbiddenLiveOSName" cfg)
        && !(lib.hasInfix "autopilot" cfg);
    expected = true;
  };
}
