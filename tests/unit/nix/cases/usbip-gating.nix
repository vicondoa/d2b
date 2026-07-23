# nix-unit cases migrated from tests/usbip-gating-eval.sh.
#
# Host-side USBIP gating: the per-env usbipd backend/proxy systemd units,
# the proxy socket, and the per-env iptables carve-outs were all deleted in
# the daemon-only end state — the broker now spawns
# `SpawnRunner{role: Usbip, vm_id: sys-<env>-usbipd}` and places the
# firewall carve-outs at runtime via the `UsbipBindFirewallRule` op. The
# only host-side gating that still survives at NixOS eval time is the
# boot-time `usbip-host` kernel-module load, which appears iff host-side
# YubiKey support is enabled AND some enabled VM opts into `usbip.yubikey`.
#
# Uses `mkEval` (== nixosSystem with the d2b module set) to render the
# real host-level systemd / firewall / boot config, then asserts presence
# with `builtins.hasAttr` / `builtins.elem` and firewall-substring presence
# with `lib.hasInfix` (substring; robust across the multi-line
# extraCommands string, unlike `builtins.match` whose `.` does not span
# newlines).
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
      config = {
        networking.hostName = lib.mkDefault "corp-vm";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
  };

  # Multi-env fixture: host YubiKey enabled, `dev-vm` opts in, `work-vm`
  # does not. Mirrors the bash gate's env-scoping case.
  multiEnvModule = { lib, ... }: {
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
      yubikey.enable = true;
    };
    d2b.envs.dev = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    d2b.envs.work = {
      lanSubnet = "10.21.0.0/24";
      uplinkSubnet = "198.51.100.0/30";
    };
    d2b.vms.dev-vm = {
      enable = true;
      env = "dev";
      index = 10;
      ssh.user = "alice";
      usbip.yubikey = true;
      guest.control.enable = true;
      config = {
        networking.hostName = lib.mkDefault "dev-vm";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
    d2b.vms.work-vm = {
      enable = true;
      env = "work";
      index = 11;
      ssh.user = "alice";
      config = {
        networking.hostName = lib.mkDefault "work-vm";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
  };

  evalSingle = overrides: mkEval ([ base ] ++ overrides);

  hasService = sys: name: builtins.hasAttr name sys.config.systemd.services;
  hasSocket = sys: name: builtins.hasAttr name sys.config.systemd.sockets;
  hasKmod = sys: builtins.elem "usbip-host" sys.config.boot.kernelModules;
  fwOf = sys: sys.config.networking.firewall.extraCommands;

  backendName = "d2b-sys-work-usbipd-backend";
  proxyName = "d2b-sys-work-usbipd-proxy";

  disabled = evalSingle [ ];
  siteEnabledNoVm = evalSingle [
    ({ lib, ... }: {
      d2b.site.yubikey.enable = lib.mkForce true;
    })
  ];
  siteEnabledDisabledVm = evalSingle [
    ({ lib, ... }: {
      d2b.site.yubikey.enable = lib.mkForce true;
      d2b.vms.corp-vm.enable = lib.mkForce false;
      d2b.vms.corp-vm.usbip.yubikey = true;
    })
  ];
  vmEnabledSiteDisabled = evalSingle [
    ({ lib, ... }: {
      d2b.site.yubikey.enable = lib.mkForce false;
      d2b.vms.corp-vm.usbip.yubikey = true;
      d2b.vms.corp-vm.guest.control.enable = true;
    })
  ];
  enabled = evalSingle [
    ({ lib, ... }: {
      d2b.site.yubikey.enable = lib.mkForce true;
      d2b.vms.corp-vm.usbip.yubikey = true;
      d2b.vms.corp-vm.guest.control.enable = true;
    })
  ];

  multiEnv = mkEval [ multiEnvModule ];
  multiFw = fwOf multiEnv;
in
{
  # --- usbip-disabled: nothing opted in -------------------------------
  "usbip-gating/disabled-backend-absent" = {
    expr = hasService disabled backendName;
    expected = false;
  };
  "usbip-gating/disabled-proxy-absent" = {
    expr = hasService disabled proxyName;
    expected = false;
  };
  "usbip-gating/disabled-socket-absent" = {
    expr = hasSocket disabled proxyName;
    expected = false;
  };
  "usbip-gating/disabled-kernel-module-absent" = {
    expr = hasKmod disabled;
    expected = false;
  };
  "usbip-gating/disabled-proxy-firewall-rule-absent" = {
    expr = lib.hasInfix "--dport 3240" (fwOf disabled);
    expected = false;
  };
  "usbip-gating/disabled-backend-firewall-rule-absent" = {
    expr = lib.hasInfix "--dport 3241" (fwOf disabled);
    expected = false;
  };

  # --- usbip-site-enabled-no-vm: host opted in, no VM opted in --------
  "usbip-gating/site-enabled-no-vm-backend-absent" = {
    expr = hasService siteEnabledNoVm backendName;
    expected = false;
  };
  "usbip-gating/site-enabled-no-vm-proxy-absent" = {
    expr = hasService siteEnabledNoVm proxyName;
    expected = false;
  };
  "usbip-gating/site-enabled-no-vm-socket-absent" = {
    expr = hasSocket siteEnabledNoVm proxyName;
    expected = false;
  };
  "usbip-gating/site-enabled-no-vm-kernel-module-absent" = {
    expr = hasKmod siteEnabledNoVm;
    expected = false;
  };
  "usbip-gating/site-enabled-no-vm-proxy-firewall-rule-absent" = {
    expr = lib.hasInfix "--dport 3240" (fwOf siteEnabledNoVm);
    expected = false;
  };
  "usbip-gating/site-enabled-no-vm-backend-firewall-rule-absent" = {
    expr = lib.hasInfix "--dport 3241" (fwOf siteEnabledNoVm);
    expected = false;
  };

  # --- usbip-site-enabled-disabled-vm: VM opts in but is disabled -----
  "usbip-gating/site-enabled-disabled-vm-backend-absent" = {
    expr = hasService siteEnabledDisabledVm backendName;
    expected = false;
  };
  "usbip-gating/site-enabled-disabled-vm-proxy-absent" = {
    expr = hasService siteEnabledDisabledVm proxyName;
    expected = false;
  };
  "usbip-gating/site-enabled-disabled-vm-socket-absent" = {
    expr = hasSocket siteEnabledDisabledVm proxyName;
    expected = false;
  };
  "usbip-gating/site-enabled-disabled-vm-kernel-module-absent" = {
    expr = hasKmod siteEnabledDisabledVm;
    expected = false;
  };
  "usbip-gating/site-enabled-disabled-vm-proxy-firewall-rule-absent" = {
    expr = lib.hasInfix "--dport 3240" (fwOf siteEnabledDisabledVm);
    expected = false;
  };
  "usbip-gating/site-enabled-disabled-vm-backend-firewall-rule-absent" = {
    expr = lib.hasInfix "--dport 3241" (fwOf siteEnabledDisabledVm);
    expected = false;
  };

  # --- usbip-vm-enabled-site-disabled: VM opts in, host opted out -----
  "usbip-gating/vm-enabled-site-disabled-backend-absent" = {
    expr = hasService vmEnabledSiteDisabled backendName;
    expected = false;
  };
  "usbip-gating/vm-enabled-site-disabled-proxy-absent" = {
    expr = hasService vmEnabledSiteDisabled proxyName;
    expected = false;
  };
  "usbip-gating/vm-enabled-site-disabled-socket-absent" = {
    expr = hasSocket vmEnabledSiteDisabled proxyName;
    expected = false;
  };
  "usbip-gating/vm-enabled-site-disabled-kernel-module-absent" = {
    expr = hasKmod vmEnabledSiteDisabled;
    expected = false;
  };
  "usbip-gating/vm-enabled-site-disabled-proxy-firewall-rule-absent" = {
    expr = lib.hasInfix "--dport 3240" (fwOf vmEnabledSiteDisabled);
    expected = false;
  };
  "usbip-gating/vm-enabled-site-disabled-backend-firewall-rule-absent" = {
    expr = lib.hasInfix "--dport 3241" (fwOf vmEnabledSiteDisabled);
    expected = false;
  };

  # --- usbip-enabled: host + VM both opt in. Backend/proxy/socket and
  # firewall carve-outs are still absent (daemon-only — broker
  # SpawnRunner / UsbipBindFirewallRule); only the usbip-host kernel
  # module survives at eval time. ------------------------------------
  "usbip-gating/enabled-backend-absent" = {
    expr = hasService enabled backendName;
    expected = false;
  };
  "usbip-gating/enabled-proxy-absent" = {
    expr = hasService enabled proxyName;
    expected = false;
  };
  "usbip-gating/enabled-socket-absent" = {
    expr = hasSocket enabled proxyName;
    expected = false;
  };
  "usbip-gating/enabled-kernel-module-present" = {
    expr = hasKmod enabled;
    expected = true;
  };
  "usbip-gating/enabled-proxy-firewall-rule-absent" = {
    expr = lib.hasInfix "--dport 3240" (fwOf enabled);
    expected = false;
  };
  "usbip-gating/enabled-backend-firewall-rule-absent" = {
    expr = lib.hasInfix "--dport 3241" (fwOf enabled);
    expected = false;
  };

  # --- usbip-multi-env-scoped: host enabled, dev-vm opts in, work-vm
  # does not. All per-env units / sockets / firewall carve-outs are
  # absent (daemon-only — broker SpawnRunner / UsbipBindFirewallRule).
  "usbip-gating/multi-env-dev-backend-absent" = {
    expr = hasService multiEnv "d2b-sys-dev-usbipd-backend";
    expected = false;
  };
  "usbip-gating/multi-env-dev-proxy-absent" = {
    expr = hasService multiEnv "d2b-sys-dev-usbipd-proxy";
    expected = false;
  };
  "usbip-gating/multi-env-dev-socket-absent" = {
    expr = hasSocket multiEnv "d2b-sys-dev-usbipd-proxy";
    expected = false;
  };
  "usbip-gating/multi-env-work-backend-absent" = {
    expr = hasService multiEnv "d2b-sys-work-usbipd-backend";
    expected = false;
  };
  "usbip-gating/multi-env-work-proxy-absent" = {
    expr = hasService multiEnv "d2b-sys-work-usbipd-proxy";
    expected = false;
  };
  "usbip-gating/multi-env-work-socket-absent" = {
    expr = hasSocket multiEnv "d2b-sys-work-usbipd-proxy";
    expected = false;
  };
  "usbip-gating/multi-env-dev-proxy-firewall-rule-absent" = {
    expr = lib.hasInfix "-i br-dev-up -p tcp --dport 3240 -s 192.0.2.0/30" multiFw;
    expected = false;
  };
  "usbip-gating/multi-env-work-proxy-firewall-rule-absent" = {
    expr = lib.hasInfix "-i br-work-up -p tcp --dport 3240 -s 198.51.100.0/30" multiFw;
    expected = false;
  };
  "usbip-gating/multi-env-dev-backend-firewall-rule-absent" = {
    expr = lib.hasInfix "--dport 3241 ! -s 127.0.0.1" multiFw;
    expected = false;
  };
  "usbip-gating/multi-env-work-backend-firewall-rule-absent" = {
    expr = lib.hasInfix "--dport 3242 ! -s 127.0.0.1" multiFw;
    expected = false;
  };
}
