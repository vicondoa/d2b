# shellcheck shell=bash
# tests/lib/minijail-validator-common.sh
#
# Shared helpers for the per-role minijail validator scripts
# (tests/minijail-validator-<role>.sh). Sourced by each per-role
# validator. Provides:
#
#   - evaluate_minijail_profile_caps <profile-id> [ch-handoff-mode]
#       Evaluate the in-tree NixOS module configuration with
#       daemonExperimental.enable = true and a single test VM, then
#       echo the JSON array of capabilities declared on the named
#       profile. The optional second argument overrides
#       nixling.site.ch.netHandoffMode (e.g. "persistent-tap"); if
#       omitted the options-site.nix default ("tap-fd") applies.
#       Caller is expected to compare against the role's documented
#       cap set for that mode.
#
#   - assert_caps_exact <expected-json-array> <actual-json-array> <role-name>
#       Compare two jq-parseable JSON arrays as sorted sets; pass /
#       fail using the caller-provided pass_check / fail_check
#       counters (which must be visible in the sourcing script).
#
#   - write_role_evidence <role-name> <evidence-path>
#       Emit the canonical {wave, timestamp, operatorSignature} JSON
#       to <evidence-path>. operatorSignature is sha256 of the
#       documented quad (plan|daemon.version|broker.version|bundle.hash);
#       when any input is unavailable in the test environment we
#       substitute a deterministic placeholder so the schema shape is
#       preserved and downstream readers don't need to special-case
#       Layer-2 vs production evidence.
#
#   - probe_seccomp_kills_ptrace <minijail0> <seccomp-policy>
#       Spawn a tiny python helper that issues SYS_ptrace under the
#       caller-provided minijail0 binary + seccomp policy file;
#       echo "killed" on SIGSYS, "ran" on a clean exit, "error" on
#       any other outcome. Caller asserts the value.
#
# Convention: every helper writes diagnostic output to stderr via the
# caller's `log` (from tests/lib.sh, which the caller is expected to
# have already sourced) and echoes structured return values to stdout
# so they're shell-parseable.
#
# Layer split: every helper is safe to run in Layer-1 (eval-only) mode
# except probe_seccomp_kills_ptrace, which requires NL_LIVE=1 + the
# nixpkgs#minijail binary + python on PATH.

set -u

# --- Layer-1: profile cap extraction -------------------------------------
#
# Evaluates ROOT's flake with a synthetic single-VM configuration, then
# emits the JSON capability list for the requested profile id. Returns
# non-zero on eval failure.
evaluate_minijail_profile_caps() {
  local profile_id=$1
  local ch_handoff_override=${2:-}
  local ch_handoff_nix=""
  if [ -n "$ch_handoff_override" ]; then
    ch_handoff_nix="ch.netHandoffMode = \"${ch_handoff_override}\";"
  fi
  local expr
  expr=$(cat <<NIXEOF
let
  flake   = builtins.getFlake "git+file://${ROOT}";
  lib     = flake.inputs.nixpkgs.lib;
  nixpkgs = flake.inputs.nixpkgs;
  nixos   = lib.nixosSystem {
    system = "x86_64-linux";
    pkgs   = import nixpkgs {
      system = "x86_64-linux";
      config.allowUnsupportedSystem = true;
    };
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        boot.loader.grub.enable           = false;
        boot.loader.systemd-boot.enable   = false;
        boot.initrd.includeDefaultModules = false;
        fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
        environment.etc."machine-id".text = "00000000000000000000000000000000";
        system.stateVersion               = "25.11";
        users.users.alice = { isNormalUser = true; uid = 1000; };
        nixling.site = {
          waylandUser   = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
          ${ch_handoff_nix}
        };
        nixling.envs.work = {
          lanSubnet    = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };
        nixling.vms.corp-vm = {
          enable   = true;
          env      = "work";
          index    = 10;
          ssh.user = "alice";
          config = { lib, ... }: {
            networking.hostName = lib.mkDefault "corp-vm";
            users.users.alice   = { isNormalUser = true; uid = 1000; };
          };
        };
        nixling.daemonExperimental.enable = true;
      })
    ];
  };
  caps = nixos.config.nixling._bundle.minijailProfiles."${profile_id}".data.capabilities;
in builtins.toJSON caps
NIXEOF
)
  nix --extra-experimental-features 'nix-command flakes' \
    eval --impure --raw --expr "$expr"
}

# --- Layer-1: set-equal cap assertion ------------------------------------
assert_caps_exact() {
  local expected_json=$1
  local actual_json=$2
  local role=$3
  local expected_sorted actual_sorted
  expected_sorted=$(printf '%s' "$expected_json" | jq -ec 'sort')
  actual_sorted=$(printf '%s'   "$actual_json"   | jq -ec 'sort')
  if [ "$expected_sorted" = "$actual_sorted" ]; then
    pass_check "${role}: profile caps == ${expected_sorted}"
  else
    fail_check "${role}: caps mismatch; expected=${expected_sorted} actual=${actual_sorted}"
  fi
}

# --- Layer-2: canonical evidence writer ----------------------------------
write_role_evidence() {
  local role=$1
  local evidence_path=$2

  local plan_sha="unknown"
  local plan_md="$ROOT/plan.md"
  if [ -f "$plan_md" ]; then
    plan_sha=$(sha256sum "$plan_md" | awk '{print $1}')
  fi

  local daemon_ver="unknown"
  if [ -f /run/nixling/version ]; then
    daemon_ver=$(jq -r '.server_version // "unknown"' /run/nixling/version 2>/dev/null || printf 'unknown')
  fi

  local broker_ver="unknown"
  if command -v nixling-priv-broker >/dev/null 2>&1; then
    broker_ver=$(nixling-priv-broker --version 2>/dev/null | awk '{print $NF}' || printf 'unknown')
  fi

  local bundle_hash="unknown"
  local bundle_json="/var/lib/nixling/bundle/bundle.json"
  if [ -f "$bundle_json" ]; then
    bundle_hash="sha256:$(sha256sum "$bundle_json" | awk '{print $1}')"
  fi

  local sig_input="${plan_sha}|${daemon_ver}|${broker_ver}|${bundle_hash}"
  local operator_sig
  operator_sig="sha256:$(printf '%s' "$sig_input" | sha256sum | awk '{print $1}')"
  local ts
  ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

  local json
  json=$(printf '{"wave":"p1-%s","timestamp":"%s","operatorSignature":"%s"}\n' \
    "$role" "$ts" "$operator_sig")

  sudo mkdir -p "$(dirname "$evidence_path")"
  printf '%s' "$json" | sudo tee "$evidence_path" >/dev/null
}

# --- Layer-2: negative seccomp probe -------------------------------------
#
# Returns one of {"killed", "ran", "error"} on stdout. Caller decides
# which is the expected verdict (typically "killed" for the documented
# undeclared syscall on this role).
probe_seccomp_kills_ptrace() {
  local minijail0=$1
  local policy=$2
  local rc

  set +e
  "$minijail0" -n -S "$policy" -- \
    python3 -c 'import ctypes; ctypes.CDLL(None, use_errno=True).ptrace(0,0,0,0)' \
    </dev/null >/dev/null 2>&1
  rc=$?
  set -e

  # minijail0 propagates SIGSYS as exit code 128+31 = 159.
  case "$rc" in
    0)   echo "ran" ;;
    159) echo "killed" ;;
    # Some kernels report 132 (128+SIGSYS=4 on some arches) — also kill.
    132) echo "killed" ;;
    *)   echo "error:$rc" ;;
  esac
}
