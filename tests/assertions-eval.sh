#!/usr/bin/env bash
# tests/assertions-eval.sh — eval-time-assertion regression tests for
# the nixling option schema (W3b H10).
#
# Most cases construct a synthetic consumer-style nixosSystem that
# imports `nixling.nixosModules.default` with one known-bad option,
# runs `nix-instantiate --eval --strict`, and asserts BOTH that the
# eval FAILS and that stderr contains a specific substring identifying
# which assertion fired. The reserved-prefix exemption case is the one
# success-path exception: it proves the auto-declared observability VM's
# configured `vmName` is exempt from the reserved `sys-` prefix rule.
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
SKIP=0

export EVAL_EXPR_FILE=""
EVAL_OUT_FILE=""
EVAL_ERR_FILE=""

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; PASS=$((PASS+1)); }
fail() { log "  FAIL: $*"; FAIL=$((FAIL+1)); }
skip() { log "  SKIP: $*"; SKIP=$((SKIP+1)); }

show_stderr_tail() {
  local file="$1"
  log "    --- stderr (tail) ---"
  tail -15 "$file" | sed 's/^/      /' >&2
}

stderr_contains_all() {
  local file="$1"
  shift
  local needle
  for needle in "$@"; do
    if ! grep -q -F -- "$needle" "$file"; then
      return 1
    fi
  done
}

# Build a nixosSystem expression around an override block.
# Arg 1 = the per-test override module (a Nix attrset string).
# Arg 2 (optional) = the target system. Defaults to x86_64-linux.
# Arg 3 (optional) = the expression returned from `in ...`.
# Defaults to `nixos.config.system.build.toplevel.drvPath`.
mk_expr() {
  local override="$1"
  local system="${2:-x86_64-linux}"
  local body="${3:-nixos.config.system.build.toplevel.drvPath}"
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
  $body
EOF
}

run_eval_json() {
  local name="$1" override="$2" body="$3" system="${4:-x86_64-linux}"
  local expr_file out_file err_file
  expr_file="$SCRATCH/$name.nix"
  out_file="$SCRATCH/$name.stdout"
  err_file="$SCRATCH/$name.stderr"
  mk_expr "$override" "$system" "$body" > "$expr_file"
  EVAL_EXPR_FILE="$expr_file"
  EVAL_OUT_FILE="$out_file"
  EVAL_ERR_FILE="$err_file"
  if nix-instantiate --eval --strict \
       --json \
       --expr "$(cat "$expr_file")" \
       > "$out_file" 2> "$err_file"; then
    return 0
  fi
  return 1
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
    show_stderr_tail "$out_file"
  fi
}

feature_auto_obs_ready() {
  local vm_name="$1"
  local probe_name="__probe-auto-obs-${vm_name//[^a-zA-Z0-9]/-}"
  local override body
  override=$(cat <<EOF
({ ... }: {
  nixling.observability.enable = true;
  nixling.observability.vmName = "$vm_name";
})
EOF
)
  body=$(cat <<EOF
{
  hasObsEnv = builtins.hasAttr "obs" nixos.config.nixling.envs;
  hasObsVm = builtins.hasAttr "$vm_name" nixos.config.nixling.vms;
}
EOF
)
  run_eval_json "$probe_name" "$override" "$body" || return 1
  jq -e '.hasObsEnv and .hasObsVm' "$EVAL_OUT_FILE" >/dev/null 2>&1
}

feature_transport_vsock_ready() {
  run_eval_json \
    "__probe-transport-vsock" \
    '({ ... }: {
       nixling.observability.enable = true;
       nixling.vms.corp-vm.observability.enable = true;
     })' \
    'builtins.hasAttr "nixling-otel-relay@" nixos.config.systemd.services' \
    || return 1
  jq -e '. == true' "$EVAL_OUT_FILE" >/dev/null 2>&1
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

# v0.1.6 SWArch-M9 — graphics VMs cannot be autostart. The wrapper's
# default path (microvm@<vm>) bypasses the GPU sidecar that binds to
# /run/user/<uid>/wayland-0, so an autostart=true graphics VM would
# silently boot without display. Must fail at eval time.
test_graphics_with_autostart() {
  run_assertion_test \
    "graphics-with-autostart" \
    '({ ... }: {
       nixling.vms.corp-vm.graphics.enable = true;
       nixling.vms.corp-vm.autostart = true;
     })' \
    'graphics.enable = true is incompatible'
}

# Wave-1 observability — colliding transport CIDs must fail with a
# message naming both VMs. Until transport-vsock lands in this worktree,
# skip with a TODO marker instead of making Layer 1 red.
test_observability_cid_collision() {
  local override err_file
  override=$(cat <<'EOF'
({ lib, ... }: {
  nixling.observability.enable = true;
  nixling.envs.aaa = {
    lanSubnet = "10.30.0.0/24";
    uplinkSubnet = "198.51.100.0/30";
  };
  nixling.envs.bbb = {
    lanSubnet = "10.31.0.0/24";
    uplinkSubnet = "198.18.0.0/30";
  };
  nixling.vms.corp-vm.env = lib.mkForce "aaa";
  nixling.vms.corp-vm.index = lib.mkForce 110;
  nixling.vms.corp-vm.observability.enable = true;
  nixling.vms.other-vm = {
    enable = true;
    env = "bbb";
    index = 10;
    ssh.user = "alice";
    observability.enable = true;
    config = {
      networking.hostName = lib.mkDefault "other-vm";
      users.users.alice = { isNormalUser = true; uid = 1000; };
    };
  };
})
EOF
)

  if run_eval_json \
      'observability-cid-collision' \
      "$override" \
      'nixos.config.system.build.toplevel.drvPath'; then
    if feature_transport_vsock_ready; then
      fail 'observability-cid-collision: eval succeeded but the CID collision should fail'
      return 1
    fi
    skip 'observability-cid-collision: TODO post-integration — transport-vsock relay/assertions have not landed in this worktree'
    return 0
  fi

  err_file="$EVAL_ERR_FILE"
  if stderr_contains_all "$err_file" 'CID' 'corp-vm' 'other-vm'; then
    ok "observability-cid-collision (found: 'CID', 'corp-vm', 'other-vm')"
    return 0
  fi

  if feature_transport_vsock_ready; then
    fail 'observability-cid-collision: eval failed but stderr did not name the colliding VMs/CID'
    show_stderr_tail "$err_file"
    return 1
  fi

  skip 'observability-cid-collision: TODO post-integration — transport-vsock CID-collision assertion has not landed in this worktree'
}

# Wave-1 observability — cfg.vmName is allowed to keep the reserved
# sys- prefix, but only for the framework's auto-declared VM. If the
# auto-obs-vm track has not landed yet, skip instead of failing.
test_observability_vmname_reserved_prefix_exempt() {
  local override
  override=$(cat <<'EOF'
({ ... }: {
  nixling.observability.enable = true;
  nixling.observability.vmName = "sys-custom-obs";
})
EOF
)

  if run_eval_json \
      'observability-vmname-reserved-prefix-exempt' \
      "$override" \
      'builtins.hasAttr "sys-custom-obs" nixos.config.nixling.vms'; then
    if jq -e '. == true' "$EVAL_OUT_FILE" >/dev/null 2>&1; then
      ok 'observability-vmname-reserved-prefix-exempt'
      return 0
    fi
    if feature_auto_obs_ready 'sys-custom-obs'; then
      fail 'observability-vmname-reserved-prefix-exempt: auto-declared VM missing despite observability.enable = true'
      return 1
    fi
    skip 'observability-vmname-reserved-prefix-exempt: TODO post-integration — auto-obs-vm has not landed in this worktree'
    return 0
  fi

  if feature_auto_obs_ready 'sys-custom-obs'; then
    fail 'observability-vmname-reserved-prefix-exempt: eval failed even though cfg.vmName should be exempt from the reserved sys- prefix rule'
    show_stderr_tail "$EVAL_ERR_FILE"
    return 1
  fi

  skip 'observability-vmname-reserved-prefix-exempt: TODO post-integration — auto-obs-vm has not landed in this worktree'
}

# Wave-6 follow-up: consumer extensions of the auto-declared
# observability VM are EXPECTED and supported as of v0.2.0. The
# framework's `observability-vm.nix` block uses `lib.mkDefault` for
# every value it sets, so a consumer extension under
# `nixling.vms.<obsVmName>` MERGES via the module system — there is
# no collision to detect. This test pins that behaviour: eval must
# succeed when a consumer extends the auto-declared VM.
test_observability_vmname_collision() {
  local override
  override=$(cat <<'EOF'
({ lib, ... }: {
  nixling.observability.enable = true;
  nixling.observability.vmName = "obs-stack";
  # Consumer-side extension of the auto-declared VM. Pre-v0.2.0 the
  # framework rejected this; v0.2.0 allows it so downstream sites can
  # attach an operator ssh.user, sudoers rules, or extra imports.
  nixling.vms.obs-stack = {
    ssh.user = "alice";
    config = {
      users.users.alice = { isNormalUser = true; uid = 1000; };
    };
  };
})
EOF
)

  if run_eval_json \
      'observability-vmname-extension-allowed' \
      "$override" \
      'nixos.config.system.build.toplevel.drvPath'; then
    ok 'observability-vmname-extension-allowed (consumer can extend auto-declared obs VM)'
    return 0
  fi

  if feature_auto_obs_ready 'obs-stack'; then
    fail 'observability-vmname-extension-allowed: eval failed but consumer-side extension should be permitted'
    show_stderr_tail "$EVAL_ERR_FILE"
    return 1
  fi

  skip 'observability-vmname-extension-allowed: TODO post-integration — auto-obs-vm has not landed in this worktree'
}

# ---------------------------------------------------------------------------

log '==> tests/assertions-eval.sh'

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
test_graphics_with_autostart
test_observability_cid_collision
test_observability_vmname_reserved_prefix_exempt
test_observability_vmname_collision

log "==> assertions-eval: $PASS passed, $FAIL failed, $SKIP skipped"
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
