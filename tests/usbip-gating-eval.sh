#!/usr/bin/env bash
# tests/usbip-gating-eval.sh — eval-time regression test for host-side
# USBIP gating. Confirms the per-env backend/proxy units, proxy socket,
# boot-time usbip-host kernel module, and firewall rules are emitted
# only when host-side YubiKey support is enabled AND some VM opts into
# `usbip.yubikey`, and only for envs that actually carry an opted-in VM.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/usbip-gating-eval.sh"

eval_case() {
  local name="$1" override="$2" expr
  expr=$(cat <<EOF
let
  flake = builtins.getFlake (toString $ROOT);
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
  nixos = nixosSystem {
    system = "x86_64-linux";
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
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
          yubikey.enable = false;
        };
        nixling.envs.work = {
          lanSubnet = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };
        nixling.vms.corp-vm = {
          enable = true;
          env = "work";
          index = 10;
          ssh.user = "alice";
          config = {
            networking.hostName = lib.mkDefault "corp-vm";
            users.users.alice = { isNormalUser = true; uid = 1000; };
          };
        };
      })
      $override
    ];
  };
  fw = nixos.config.networking.firewall.extraCommands;
  hasProxyService = builtins.hasAttr "nixling-sys-work-usbipd-proxy" nixos.config.systemd.services;
  hasProxySocket = builtins.hasAttr "nixling-sys-work-usbipd-proxy" nixos.config.systemd.sockets;
in {
  hasBackend = builtins.hasAttr "nixling-sys-work-usbipd-backend" nixos.config.systemd.services;
  hasProxy = hasProxyService;
  hasSocket = hasProxySocket;
  hasKernelModule = builtins.elem "usbip-host" nixos.config.boot.kernelModules;
  firewallHasProxyRule = builtins.match ".*--dport 3240.*" fw != null;
  firewallHasBackendRule = builtins.match ".*--dport 3241.*" fw != null;
  proxyListenStream =
    if hasProxySocket
    then nixos.config.systemd.sockets."nixling-sys-work-usbipd-proxy".socketConfig.ListenStream
    else "";
  proxyTargetsBackend =
    if hasProxyService
    then builtins.match ".*127\\.0\\.0\\.1:3241.*"
      nixos.config.systemd.services."nixling-sys-work-usbipd-proxy".serviceConfig.ExecStart != null
    else false;
}
EOF
)

  nix-instantiate --eval --strict --json --expr "$expr" 2>/dev/null \
    || fail "$name: eval failed"
}

assert_json_bool() {
  local json="$1" field="$2" expected="$3" msg="$4" actual
  actual=$(printf '%s' "$json" | jq -r --arg field "$field" '.[$field]') \
    || fail "$msg: could not read .$field"
  if [ "$actual" = "$expected" ]; then
    ok "$msg"
  else
    fail "$msg: got $actual, expected $expected"
  fi
}

assert_json_eq() {
  local json="$1" field="$2" expected="$3" msg="$4" actual
  actual=$(printf '%s' "$json" | jq -r --arg field "$field" '.[$field]') \
    || fail "$msg: could not read .$field"
  if [ "$actual" = "$expected" ]; then
    ok "$msg"
  else
    fail "$msg: got $actual, expected $expected"
  fi
}

disabled=$(eval_case \
  "usbip-disabled" \
  '({ ... }: { })')
assert_json_bool "$disabled" hasBackend false \
  'usbip-disabled: backend service absent' 
assert_json_bool "$disabled" hasProxy false \
  'usbip-disabled: proxy service absent'
assert_json_bool "$disabled" hasSocket false \
  'usbip-disabled: proxy socket absent'
assert_json_bool "$disabled" hasKernelModule false \
  'usbip-disabled: usbip-host kernel module absent'
assert_json_bool "$disabled" firewallHasProxyRule false \
  'usbip-disabled: proxy firewall rule absent'
assert_json_bool "$disabled" firewallHasBackendRule false \
  'usbip-disabled: backend firewall rule absent'

site_enabled_no_vm=$(eval_case \
  "usbip-site-enabled-no-vm" \
  '({ lib, ... }: {
     nixling.site.yubikey.enable = lib.mkForce true;
   })')
assert_json_bool "$site_enabled_no_vm" hasBackend false \
  'usbip-site-enabled-no-vm: backend service absent'
assert_json_bool "$site_enabled_no_vm" hasProxy false \
  'usbip-site-enabled-no-vm: proxy service absent'
assert_json_bool "$site_enabled_no_vm" hasSocket false \
  'usbip-site-enabled-no-vm: proxy socket absent'
assert_json_bool "$site_enabled_no_vm" hasKernelModule false \
  'usbip-site-enabled-no-vm: usbip-host kernel module absent'
assert_json_bool "$site_enabled_no_vm" firewallHasProxyRule false \
  'usbip-site-enabled-no-vm: proxy firewall rule absent'
assert_json_bool "$site_enabled_no_vm" firewallHasBackendRule false \
  'usbip-site-enabled-no-vm: backend firewall rule absent'

disabled_vm_opt_in=$(eval_case \
  "usbip-site-enabled-disabled-vm" \
  '({ lib, ... }: {
     nixling.site.yubikey.enable = lib.mkForce true;
     nixling.vms.corp-vm.enable = lib.mkForce false;
     nixling.vms.corp-vm.usbip.yubikey = true;
   })')
assert_json_bool "$disabled_vm_opt_in" hasBackend false \
  'usbip-site-enabled-disabled-vm: backend service absent'
assert_json_bool "$disabled_vm_opt_in" hasProxy false \
  'usbip-site-enabled-disabled-vm: proxy service absent'
assert_json_bool "$disabled_vm_opt_in" hasSocket false \
  'usbip-site-enabled-disabled-vm: proxy socket absent'
assert_json_bool "$disabled_vm_opt_in" hasKernelModule false \
  'usbip-site-enabled-disabled-vm: usbip-host kernel module absent'
assert_json_bool "$disabled_vm_opt_in" firewallHasProxyRule false \
  'usbip-site-enabled-disabled-vm: proxy firewall rule absent'
assert_json_bool "$disabled_vm_opt_in" firewallHasBackendRule false \
  'usbip-site-enabled-disabled-vm: backend firewall rule absent'

vm_enabled_site_disabled=$(eval_case \
  "usbip-vm-enabled-site-disabled" \
  '({ lib, ... }: {
     nixling.site.yubikey.enable = lib.mkForce false;
     nixling.vms.corp-vm.usbip.yubikey = true;
   })')
assert_json_bool "$vm_enabled_site_disabled" hasBackend false \
  'usbip-vm-enabled-site-disabled: backend service absent'
assert_json_bool "$vm_enabled_site_disabled" hasProxy false \
  'usbip-vm-enabled-site-disabled: proxy service absent'
assert_json_bool "$vm_enabled_site_disabled" hasSocket false \
  'usbip-vm-enabled-site-disabled: proxy socket absent'
assert_json_bool "$vm_enabled_site_disabled" hasKernelModule false \
  'usbip-vm-enabled-site-disabled: usbip-host kernel module absent'
assert_json_bool "$vm_enabled_site_disabled" firewallHasProxyRule false \
  'usbip-vm-enabled-site-disabled: proxy firewall rule absent'
assert_json_bool "$vm_enabled_site_disabled" firewallHasBackendRule false \
  'usbip-vm-enabled-site-disabled: backend firewall rule absent'

eval_multi_env_case() {
  local expr
  expr=$(cat <<EOF
let
  flake = builtins.getFlake (toString $ROOT);
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
  nixos = nixosSystem {
    system = "x86_64-linux";
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
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
          yubikey.enable = true;
        };
        nixling.envs.dev = {
          lanSubnet = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };
        nixling.envs.work = {
          lanSubnet = "10.21.0.0/24";
          uplinkSubnet = "198.51.100.0/30";
        };
        nixling.vms.dev-vm = {
          enable = true;
          env = "dev";
          index = 10;
          ssh.user = "alice";
          usbip.yubikey = true;
          config = {
            networking.hostName = lib.mkDefault "dev-vm";
            users.users.alice = { isNormalUser = true; uid = 1000; };
          };
        };
        nixling.vms.work-vm = {
          enable = true;
          env = "work";
          index = 11;
          ssh.user = "alice";
          config = {
            networking.hostName = lib.mkDefault "work-vm";
            users.users.alice = { isNormalUser = true; uid = 1000; };
          };
        };
      })
    ];
  };
  fw = nixos.config.networking.firewall.extraCommands;
  hasDevProxyService = builtins.hasAttr "nixling-sys-dev-usbipd-proxy" nixos.config.systemd.services;
  hasDevProxySocket = builtins.hasAttr "nixling-sys-dev-usbipd-proxy" nixos.config.systemd.sockets;
  hasWorkProxyService = builtins.hasAttr "nixling-sys-work-usbipd-proxy" nixos.config.systemd.services;
  hasWorkProxySocket = builtins.hasAttr "nixling-sys-work-usbipd-proxy" nixos.config.systemd.sockets;
in {
  hasDevBackend = builtins.hasAttr "nixling-sys-dev-usbipd-backend" nixos.config.systemd.services;
  hasDevProxy = hasDevProxyService;
  hasDevSocket = hasDevProxySocket;
  hasWorkBackend = builtins.hasAttr "nixling-sys-work-usbipd-backend" nixos.config.systemd.services;
  hasWorkProxy = hasWorkProxyService;
  hasWorkSocket = hasWorkProxySocket;
  firewallHasDevProxyRule = builtins.match ".*-i br-dev-up -p tcp --dport 3240 -s 192\\.0\\.2\\.0/30.*" fw != null;
  firewallHasWorkProxyRule = builtins.match ".*-i br-work-up -p tcp --dport 3240 -s 198\\.51\\.100\\.0/30.*" fw != null;
  firewallHasDevBackendRule = builtins.match ".*--dport 3241 ! -s 127\\.0\\.0\\.1.*" fw != null;
  firewallHasWorkBackendRule = builtins.match ".*--dport 3242 ! -s 127\\.0\\.0\\.1.*" fw != null;
  devProxyListenStream =
    if hasDevProxySocket
    then nixos.config.systemd.sockets."nixling-sys-dev-usbipd-proxy".socketConfig.ListenStream
    else "";
  workProxyListenStream =
    if hasWorkProxySocket
    then nixos.config.systemd.sockets."nixling-sys-work-usbipd-proxy".socketConfig.ListenStream
    else "";
  devProxyTargetsBackend =
    if hasDevProxyService
    then builtins.match ".*127\\.0\\.0\\.1:3241.*"
      nixos.config.systemd.services."nixling-sys-dev-usbipd-proxy".serviceConfig.ExecStart != null
    else false;
  workProxyTargetsBackend =
    if hasWorkProxyService
    then builtins.match ".*127\\.0\\.0\\.1:3242.*"
      nixos.config.systemd.services."nixling-sys-work-usbipd-proxy".serviceConfig.ExecStart != null
    else false;
}
EOF
)

  nix-instantiate --eval --strict --json --expr "$expr" 2>/dev/null \
    || fail 'usbip-multi-env-scoped: eval failed'
}

multi_env_scoped=$(eval_multi_env_case)
# Per-env usbipd backend/proxy
# systemd units, sockets, and the per-env iptables carve-outs were
# all deleted. The broker spawns `SpawnRunner{role: Usbip,
# vm_id: sys-<env>-usbipd}` per the per-busid state machine in
# `docs/reference/privileges.md`, and the firewall carve-outs are
# placed at runtime via the `UsbipBindFirewallRule` broker op. The
# kernel-module load (boot.kernelModules += [ "usbip-host" ]) is
# the only host-side gating that survives at NixOS eval time.
assert_json_bool "$multi_env_scoped" hasDevBackend false \
  'usbip-multi-env-scoped: dev backend service absent (P6 — broker SpawnRunner)'
assert_json_bool "$multi_env_scoped" hasDevProxy false \
  'usbip-multi-env-scoped: dev proxy service absent (P6 — broker SpawnRunner)'
assert_json_bool "$multi_env_scoped" hasDevSocket false \
  'usbip-multi-env-scoped: dev proxy socket absent (P6 — broker SpawnRunner)'
assert_json_bool "$multi_env_scoped" hasWorkBackend false \
  'usbip-multi-env-scoped: work backend service absent'
assert_json_bool "$multi_env_scoped" hasWorkProxy false \
  'usbip-multi-env-scoped: work proxy service absent'
assert_json_bool "$multi_env_scoped" hasWorkSocket false \
  'usbip-multi-env-scoped: work proxy socket absent'
assert_json_bool "$multi_env_scoped" firewallHasDevProxyRule false \
  'usbip-multi-env-scoped: dev proxy firewall rule absent (P6 — UsbipBindFirewallRule)'
assert_json_bool "$multi_env_scoped" firewallHasWorkProxyRule false \
  'usbip-multi-env-scoped: work proxy firewall rule absent'
assert_json_bool "$multi_env_scoped" firewallHasDevBackendRule false \
  'usbip-multi-env-scoped: dev backend firewall rule absent (P6 — UsbipBindFirewallRule)'
assert_json_bool "$multi_env_scoped" firewallHasWorkBackendRule false \
  'usbip-multi-env-scoped: work backend firewall rule absent'

enabled=$(eval_case \
  "usbip-enabled" \
  '({ lib, ... }: {
     nixling.site.yubikey.enable = lib.mkForce true;
     nixling.vms.corp-vm.usbip.yubikey = true;
   })')
assert_json_bool "$enabled" hasBackend false \
  'usbip-enabled: backend service absent (P6 — broker SpawnRunner)'
assert_json_bool "$enabled" hasProxy false \
  'usbip-enabled: proxy service absent (P6 — broker SpawnRunner)'
assert_json_bool "$enabled" hasSocket false \
  'usbip-enabled: proxy socket absent (P6 — broker SpawnRunner)'
assert_json_bool "$enabled" hasKernelModule true \
  'usbip-enabled: usbip-host kernel module present'
assert_json_bool "$enabled" firewallHasProxyRule false \
  'usbip-enabled: proxy firewall rule absent (P6 — UsbipBindFirewallRule)'
assert_json_bool "$enabled" firewallHasBackendRule false \
  'usbip-enabled: backend firewall rule absent (P6 — UsbipBindFirewallRule)'

log "==> usbip-gating-eval OK"
