#!/usr/bin/env bash
# tests/bridge-ipv6-boot-sysctl-eval.sh— eval gate.
#
# Asserts that every bridge declared in the multi-env consumer example
# (br-work-lan, br-work-up, br-personal-lan, br-personal-up) has a
# corresponding `boot.kernel.sysctl."net.ipv6.conf.<bridge>.disable_ipv6"
# = 1` entry in the rendered NixOS config.
#
# This pins the invariant: NixOS activation applies bridge IPv6
# suppression declaratively BEFORE any nixlingd/broker invocation,
# closing the boot-time window where bridges had IPv6 active until the
# first VM in the env started.  Retains per-VM ApplySysctl as
# defense-in-depth (no assertion change there).

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/bridge-ipv6-boot-sysctl-eval.sh"

# Eval the rendered boot.kernel.sysctl attrset using the same inline
# nixosSystem pattern as the other eval tests.  The config mirrors
# examples/multi-env/configuration.nix (work + personal envs).
EXPR=$(cat <<EOF
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
        nixling.site = { launcherUsers = [ "alice" ]; yubikey.enable = false; };
        nixling.hostLanCidrs = [ "192.168.1.0/24" ];
        nixling.envs.work = {
          lanSubnet    = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };
        nixling.envs.personal = {
          lanSubnet    = "10.30.0.0/24";
          uplinkSubnet = "198.51.100.0/30";
        };
        nixling.vms.work-app = {
          enable = true;
          env    = "work";
          index  = 10;
          ssh.user = "alice";
          config = {
            networking.hostName = lib.mkDefault "work-app";
            users.users.alice = { isNormalUser = true; uid = 1000; };
          };
        };
        nixling.vms.personal-app = {
          enable = true;
          env    = "personal";
          index  = 10;
          ssh.user = "alice";
          config = {
            networking.hostName = lib.mkDefault "personal-app";
            users.users.alice = { isNormalUser = true; uid = 1000; };
          };
        };
      })
    ];
  };
in nixos.config.boot.kernel.sysctl
EOF
)

SYSCTL_JSON=$(nix eval --json --impure --expr "$EXPR" 2>/dev/null)

if [ -z "$SYSCTL_JSON" ]; then
  nix eval --json --impure --expr "$EXPR" >&2 || true
  fail "nix eval returned empty output — check flake eval"
fi

check_key() {
  local key="$1" expected="$2"
  local got
  got=$(printf '%s' "$SYSCTL_JSON" | jq -r --arg k "$key" '.[$k] // "MISSING"')
  if [ "$got" = "MISSING" ]; then
    fail "boot.kernel.sysctl.\"${key}\" is absent"
  elif [ "$got" != "$expected" ]; then
    fail "boot.kernel.sysctl.\"${key}\" = ${got} (expected ${expected})"
  else
    ok "boot.kernel.sysctl.\"${key}\" = ${got}"
  fi
}

# work env bridges
check_key "net.ipv6.conf.br-work-lan.disable_ipv6"  "1"
check_key "net.ipv6.conf.br-work-lan.accept_ra"     "0"
check_key "net.ipv6.conf.br-work-lan.autoconf"      "0"
check_key "net.ipv6.conf.br-work-up.disable_ipv6"   "1"
check_key "net.ipv6.conf.br-work-up.accept_ra"      "0"
check_key "net.ipv6.conf.br-work-up.autoconf"       "0"

# personal env bridges
check_key "net.ipv6.conf.br-personal-lan.disable_ipv6"  "1"
check_key "net.ipv6.conf.br-personal-lan.accept_ra"     "0"
check_key "net.ipv6.conf.br-personal-lan.autoconf"      "0"
check_key "net.ipv6.conf.br-personal-up.disable_ipv6"   "1"
check_key "net.ipv6.conf.br-personal-up.accept_ra"      "0"
check_key "net.ipv6.conf.br-personal-up.autoconf"       "0"

log "==> bridge-ipv6-boot-sysctl-eval: all assertions passed"
