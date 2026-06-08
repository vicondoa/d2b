#!/usr/bin/env bash
# tests/assertions-eval.sh — eval-time-assertion regression tests for
# the nixling option schema (W3b H10).
#
# Each test constructs a synthetic consumer-style nixosSystem that
# imports `nixling.nixosModules.default` with one known-bad option,
# runs `nix-instantiate --eval --strict` against
# `config.system.build.toplevel`, and asserts BOTH that the eval
# FAILS and that stderr contains a specific substring identifying
# which assertion fired. The goal is regression coverage on every
# eval-time invariant the nixling schema enforces — so a future
# refactor that silently drops an assertion turns into a red CI run.
#
# Each test is a single shell function `test_<short_name>`. Tests
# share a constructed "base" config and only override one field at a
# time; this keeps each test focused on the assertion under exercise.
#
# Run via:
#   tests/assertions-eval.sh
# Wired into tests/static.sh.

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=$(dirname "$HERE")

# Per-test scratch dir. Each test writes its synthetic config there
# and points `getFlake` / `import` at the same flake checkout under
# $ROOT. We do NOT mutate $ROOT.
SCRATCH=$(mktemp -d -p "$ROOT" .assertions-eval.XXXXXX)
trap 'rm -rf -- "$SCRATCH"' EXIT

PASS=0
FAIL=0

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; PASS=$((PASS+1)); }
fail() { log "  FAIL: $*"; FAIL=$((FAIL+1)); }

# Build a nixosSystem expression around an override block.
# Arg 1 = the per-test override module (a Nix attrset string).
# Arg 2 (optional) = the target system. Defaults to x86_64-linux.
# Returns a self-contained Nix expression that evaluates
# `nixos.config.system.build.toplevel.drvPath`.
mk_expr() {
  local override="$1"
  local system="${2:-x86_64-linux}"
  cat <<EOF
let
  pkgs = import <nixpkgs> { system = "$system"; };
  inherit (pkgs) lib;
  flake = builtins.getFlake (toString $ROOT);
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
  # On non-x86_64 the consumer-side nixpkgs needs to tolerate the
  # unconditional spectrum-ch reference inside store.nix. The
  # platform gate this file is exercising fires BEFORE store.nix
  # forces that path, so we only ever reach it on x86_64; on
  # aarch64 we want the gate's error to surface cleanly without
  # nixpkgs's own "package not available" complaint short-circuiting
  # earlier.
  pkgsForSystem = import flake.inputs.nixpkgs {
    system = "$system";
    config = { allowUnsupportedSystem = true; };
  };
  nixos = nixosSystem {
    system = "$system";
    pkgs = pkgsForSystem;
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        boot.loader.grub.enable = false;
        boot.loader.systemd-boot.enable = false;
        fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
        environment.etc."machine-id".text =
          "00000000000000000000000000000000";
        system.stateVersion = "25.11";
        users.users.alice = { isNormalUser = true; uid = 1000; };
        nixling.site = {
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
        };
        nixling.envs.work = {
          lanSubnet    = "10.20.0.0/24";
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
in
  nixos.config.system.build.toplevel.drvPath
EOF
}

# Run a single assertion test. eval MUST fail, AND stderr MUST contain
# `$expected_substr`. Otherwise the test fails.
# Optional 4th arg: target system (default x86_64-linux).
run_assertion_test() {
  local name="$1" override="$2" expected_substr="$3" system="${4:-x86_64-linux}"
  local expr_file out_file
  expr_file="$SCRATCH/$name.nix"
  out_file="$SCRATCH/$name.stderr"
  mk_expr "$override" "$system" > "$expr_file"
  if nix-instantiate --eval --strict \
       --expr "$(cat "$expr_file")" \
       > /dev/null 2> "$out_file"; then
    fail "$name: eval succeeded but the assertion was expected to fire"
    return 1
  fi
  if grep -q -F -- "$expected_substr" "$out_file"; then
    ok "$name (found: '$expected_substr')"
  else
    fail "$name: eval failed but stderr did not match '$expected_substr'"
    log "    --- stderr (tail) ---"
    tail -15 "$out_file" | sed 's/^/      /' >&2
  fi
}

# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

# H10/1 — private-key marker in userAuthorizedKeys must be rejected.
test_private_key_in_authorized_keys() {
  run_assertion_test \
    "private-key-in-authorized-keys" \
    '({ ... }: {
       nixling.site.userAuthorizedKeys = [
         "-----BEGIN OPENSSH PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEILa...\n-----END OPENSSH PRIVATE KEY-----"
       ];
     })' \
    'does not look like a valid SSH public key'
}

# H10/2 — graphics VM declared but waylandUser = null.
test_graphics_without_wayland_user() {
  run_assertion_test \
    "graphics-without-wayland-user" \
    '({ ... }: {
       nixling.site.waylandUser = null;
       nixling.vms.corp-vm.graphics.enable = true;
     })' \
    'nixling.site.waylandUser'
}

# H10/3 — waylandUser names a user that does not exist.
test_wayland_user_missing() {
  run_assertion_test \
    "wayland-user-missing" \
    '({ lib, ... }: {
       nixling.site.waylandUser = lib.mkForce "ghost";
       # corp-vm references env=work whose tap depends on alice; we
       # do NOT remove alice from users.users — only the waylandUser
       # rebinds.
     })' \
    'config.users.users.ghost is not declared'
}

# H10/4 — lanSubnet must be /24.
test_lansubnet_wrong_mask() {
  run_assertion_test \
    "lansubnet-wrong-mask" \
    '({ lib, ... }: {
       nixling.envs.work.lanSubnet = lib.mkForce "10.99.0.0/23";
     })' \
    'must be a /24'
}

# H10/5 — uplinkSubnet must be /30.
test_uplinksubnet_wrong_mask() {
  run_assertion_test \
    "uplinksubnet-wrong-mask" \
    '({ lib, ... }: {
       nixling.envs.work.uplinkSubnet = lib.mkForce "192.0.2.0/29";
     })' \
    'must be a /30'
}

# H10/6 — lanSubnet network address must end in .0.
test_lansubnet_nonzero_host() {
  run_assertion_test \
    "lansubnet-nonzero-host" \
    '({ lib, ... }: {
       nixling.envs.work.lanSubnet = lib.mkForce "10.99.0.5/24";
     })' \
    "ending in '.0'"
}

# H10/7 — two envs whose CIDRs OVERLAP (H3 containment case).
test_overlap_containment() {
  run_assertion_test \
    "overlap-containment" \
    '({ ... }: {
       # work env declared in the base config: lanSubnet 10.20.0.0/24.
       # Add a second env whose lanSubnet contains it.
       nixling.envs.other = {
         lanSubnet    = "10.20.0.0/16";
         uplinkSubnet = "198.51.100.0/30";
       };
     })' \
    'CIDR overlap'
}

# H10/8 — env subnet overlaps with a hostLanCidrs entry.
test_env_vs_host_overlap() {
  run_assertion_test \
    "env-vs-host-overlap" \
    '({ ... }: {
       # Default hostLanCidrs is RFC1918 + link-local. Set it
       # explicitly to a value that contains the work env'"'"'s lan
       # subnet so the H3 cidrOverlaps check fires deterministically.
       nixling.hostLanCidrs = [ "10.20.0.0/16" ];
     })' \
    'overlaps with `nixling.hostLanCidrs`'
}

# Phase 4 — graphics.enable = true on aarch64-linux must trip the
# host.nix platform gate at the microvm.vms translation. The error
# message is the authoritative one consumers see.
test_platform_gate_graphics_aarch64() {
  run_assertion_test \
    "platform-gate-graphics-aarch64" \
    '({ ... }: {
       nixling.vms.corp-vm.graphics.enable = true;
     })' \
    'graphics/audio components are' \
    "aarch64-linux"
}

# Phase 4 — audio.enable = true on aarch64-linux must also trip the
# platform gate. Mic/speaker defaults can stay at their normal values;
# the gate fires before any audio host.nix code runs.
test_platform_gate_audio_aarch64() {
  run_assertion_test \
    "platform-gate-audio-aarch64" \
    '({ ... }: {
       nixling.vms.corp-vm.audio.enable = true;
       # audio.enable requires autostart = false (existing assertion);
       # the default is false so no override needed.
     })' \
    'graphics/audio components are' \
    "aarch64-linux"
}

# ---------------------------------------------------------------------------

log "==> tests/assertions-eval.sh"

test_private_key_in_authorized_keys
test_graphics_without_wayland_user
test_wayland_user_missing
test_lansubnet_wrong_mask
test_uplinksubnet_wrong_mask
test_lansubnet_nonzero_host
test_overlap_containment
test_env_vs_host_overlap
test_platform_gate_graphics_aarch64
test_platform_gate_audio_aarch64

log "==> assertions-eval: $PASS passed, $FAIL failed"
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
