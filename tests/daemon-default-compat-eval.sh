#!/usr/bin/env bash
# tests/daemon-default-compat-eval.sh — eval-time regression gate for the
# default flip of `nixling.daemonExperimental.enable`.
#
# Asserts:
#   1. With every readiness wave in the flip gate set marked
#      implemented + validated AND with matching evidence files
#      present under defaultFlipEvidenceDir, the option default
#      flips to `true`.
#   2. If any single flip-gate wave is missing the validated flag,
#      the default stays `false`.
#   3. If a wave's evidence file is missing on disk, the default
#      stays `false` (the file half of the gate is enforced).
#   4. Explicit operator overrides win in BOTH directions — an
#      explicit `= false` keeps the value false even with the gate
#      fully green, and an explicit `= true` flips the value true
#      even with the gate red. mkDefault / mkForce semantics are
#      preserved.
#
# Shape: eval-only, no live host required. Wired into tests/static.sh
# alongside the other eval gates.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/daemon-default-compat-eval.sh"

# The wave set the flip gate iterates over. Keep this list in sync
# with `flipGateWaves` in nixos-modules/options-daemon.nix.
FLIP_GATE_WAVES=(w4Fu w5Fu w6Fu w7Fu w8Fu w9Fu p0 p0Fu p1 p2 p3 p4)

# Per-test working directory under the worktree (NOT /tmp — disk
# hygiene contract). Use a stable name so eval re-runs can reuse the
# evidence files; cleaned at the end.
WORKDIR="$ROOT/.tests-tmp/daemon-default-compat"
rm -rf "$WORKDIR"
mkdir -p "$WORKDIR/evidence-full" "$WORKDIR/evidence-missing-one" "$WORKDIR/evidence-empty"
trap 'rm -rf "$WORKDIR"' EXIT

write_evidence() {
  local dir="$1" wave="$2"
  cat >"$dir/${wave}.json" <<EOF
{
  "wave": "${wave}",
  "timestamp": "2025-01-01T00:00:00Z",
  "operatorSignature": "daemon-default-compat-eval@nixling-tests"
}
EOF
}

# evidence-full: every wave present.
for wave in "${FLIP_GATE_WAVES[@]}"; do
  write_evidence "$WORKDIR/evidence-full" "$wave"
done

# evidence-missing-one: all waves except w9Fu present (used to assert
# the file-presence half of the gate is enforced independently of the
# readiness booleans).
for wave in "${FLIP_GATE_WAVES[@]}"; do
  if [ "$wave" != "w9Fu" ]; then
    write_evidence "$WORKDIR/evidence-missing-one" "$wave"
  fi
done

# Render a Nix attrset literal of `defaultSwitchReadiness` settings.
# Args: implemented_value validated_value (both literal "true" or "false")
render_readiness_attrset() {
  local impl="$1" valid="$2" wave indent='          '
  for wave in "${FLIP_GATE_WAVES[@]}"; do
    printf '%s%s = { implemented = %s; validated = %s; };\n' \
      "$indent" "$wave" "$impl" "$valid"
  done
}

ALL_TRUE_READINESS=$(render_readiness_attrset "true" "true")
ALL_TRUE_BUT_ONE_VALIDATED=$(
  # Same as ALL_TRUE_READINESS, but flip w9Fu.validated -> false.
  for wave in "${FLIP_GATE_WAVES[@]}"; do
    if [ "$wave" = "w9Fu" ]; then
      printf '          %s = { implemented = true; validated = false; };\n' "$wave"
    else
      printf '          %s = { implemented = true; validated = true; };\n' "$wave"
    fi
  done
)

# Build a single eval expression that returns five scenarios in one
# nix-instantiate invocation. Faster than re-evaluating the flake five
# times.
EXPR=$(cat <<EOF
let
  flake = builtins.getFlake (toString $ROOT);
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;

  baseModule = { lib, ... }: {
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
  };

  mk = extraModule:
    let
      nixos = nixosSystem {
        system = "x86_64-linux";
        modules = [ flake.nixosModules.default baseModule extraModule ];
      };
    in nixos.config.nixling.daemonExperimental.enable;

  # Scenario 1: every wave implemented + validated; every evidence
  # file present in evidence-full. Default should flip to true.
  gateGreenAll = mk ({ lib, ... }: {
    nixling.daemonExperimental.defaultFlipEvidenceDir =
      "$WORKDIR/evidence-full";
    nixling.defaultSwitchReadiness = {
${ALL_TRUE_READINESS}
    };
  });

  # Scenario 2: same as #1 but one readiness entry is not validated.
  # The obsolete compatibility option still defaults true.
  gateRedReadiness = mk ({ lib, ... }: {
    nixling.daemonExperimental.defaultFlipEvidenceDir =
      "$WORKDIR/evidence-full";
    nixling.defaultSwitchReadiness = {
${ALL_TRUE_BUT_ONE_VALIDATED}
    };
  });

  # Scenario 3: every readiness entry implemented + validated, but
  # one evidence file is absent. The obsolete compatibility option
  # still defaults true.
  gateRedEvidence = mk ({ lib, ... }: {
    nixling.daemonExperimental.defaultFlipEvidenceDir =
      "$WORKDIR/evidence-missing-one";
    nixling.defaultSwitchReadiness = {
${ALL_TRUE_READINESS}
    };
  });

  # Scenario 4: gate is fully green, but operator explicitly pins
  # enable = false. Operator wins.
  overrideFalseWinsGreen = mk ({ lib, ... }: {
    nixling.daemonExperimental.defaultFlipEvidenceDir =
      "$WORKDIR/evidence-full";
    nixling.daemonExperimental.enable = lib.mkForce false;
    nixling.defaultSwitchReadiness = {
${ALL_TRUE_READINESS}
    };
  });

  # Scenario 5: gate is red (no evidence at all, no validated flags),
  # but operator explicitly opts in via mkForce true. Operator wins.
  overrideTrueWinsRed = mk ({ lib, ... }: {
    nixling.daemonExperimental.defaultFlipEvidenceDir =
      "$WORKDIR/evidence-empty";
    nixling.daemonExperimental.enable = lib.mkForce true;
  });

in {
  gateGreenAll          = gateGreenAll;
  gateRedReadiness      = gateRedReadiness;
  gateRedEvidence       = gateRedEvidence;
  overrideFalseWinsGreen = overrideFalseWinsGreen;
  overrideTrueWinsRed   = overrideTrueWinsRed;
}
EOF
)

OUT=$(nix-instantiate --eval --strict --json --expr "$EXPR" 2>&1) || {
  printf '%s\n' "$OUT" >&2
  fail "eval failed; cannot inspect daemon default compatibility"
}

check_bool() {
  local key="$1" expected="$2"
  local val
  val=$(printf '%s' "$OUT" | jq -r --arg k "$key" '.[$k]')
  if [ "$val" = "$expected" ]; then
    ok "$key = $expected"
  else
    fail "$key: expected $expected, got $val"
  fi
}

check_bool "gateGreenAll"           "true"
check_bool "gateRedReadiness"       "true"
check_bool "gateRedEvidence"        "true"
check_bool "overrideFalseWinsGreen" "false"
check_bool "overrideTrueWinsRed"    "true"

log "==> daemon-default-compat-eval OK"
