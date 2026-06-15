#!/usr/bin/env bash
# tests/tools/gen-migration-ledger.sh — W0 seed for the test-rearchitecture
# assertion ledger (plan §4/§5, Appendix A).
#
# Emits tests/migration-ledger.toml: one row PER SCRIPT (W0 seed granularity;
# W1+ refine to one row PER ASSERTION). Each row records the target group
# (A-H / G-ci / G-hw / perf), the `make` target, the CI tier, and migration
# status. ORCH/helper scripts are intentionally excluded (they are not test
# cases; they become `make`-target plumbing).
#
# CRITICAL self-check (seed of the `check-inventory` gate, plan §3.9): every
# `tests/*.sh` MUST be assigned to exactly one group or the ORCH exclude list.
# The script fails closed on any unassigned or doubly-assigned file, so a
# newly-landed test cannot escape the taxonomy/ledger/CI.

# shellcheck disable=SC2034  # group arrays are consumed via nameref lookup.
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}
DEFAULT_OUT="$ROOT/tests/migration-ledger.toml"
MODE="write"
OUT="$DEFAULT_OUT"

usage() {
  cat <<USAGE
usage: gen-migration-ledger.sh [--check] [output-path]

  --check      Generate to a repo-local scratch file and diff against the
               committed ledger without mutating it.
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --check)
      MODE=check
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --*)
      echo "gen-migration-ledger: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      OUT="$1"
      ;;
  esac
  shift
done

# --- group → scripts (basenames, no .sh) -----------------------------------
# A: cargo-nextest (logic/argv/DTO, fake-backed canaries, KVM-free runtime).
A=(
  activation-helper-eval audio-argv-shape broker-default-features-build
  broker-enum-disposition broker-export-audit broker-scm-rights-fd-lifecycle
  broker-socket-acl broker-validate-bundle cgroup-delegation-oracle ch-argv-shape
  ch-net-handoff-canary cli-json cli-rust-native-audit cli-rust-native-auth-status
  cli-rust-native-host-check cli-rust-native-host-doctor cli-rust-native-list
  cli-rust-native-status cli-rust-native-usb cli-vm-verbs-eval daemon-metrics-eval
  daemon-socket-acl daemon-state-lock daemon-state-persistence
  daemon-version-negotiation dag-topo device-node-matrix gpu-argv-shape
  guest-control-proto guest-control-token-materializer guest-proto-bindings
  guest-static-elf guest-ttrpc-bindings host-prepare-idempotency
  host-prepare-network host-validate-verb-eval ifname-collision ioctl-negative
  kernel-module-matrix l1c-privilege-oracle manifest-fuzz-bounded
  manifest-v04-roundtrip minijail-version-check net-vm-bundle-gate-eval
  nft-coexistence nft-foreign-rule-preservation nixlingd-startup-smoke
  otel-host-bridge-argv-shape path-safety-violation-fs pidfd-handoff
  bridge-isolation-runtime runner-shape-preflight runner-shape-snapshot
  rust-workspace-checks sidecar-argv-shape ssh-host-key-preflight-eval
  stub-no-socket usbip-argv-shape usbip-firewall-skeleton
  usbip-state-machine-eval video-argv-shape video-binary-contract
  virtiofsd-argv-shape w6-argv-shape
)

# B: drift vs shipped committed artifact (xtask gen + git diff).
B=(
  bundle-drift cli-json-drift daemon-api-drift error-codes-drift
  host-json-drift-gate manpage-completion-drift vms-json-parity
  wave-evidence-schema-eval
)

# C: contract over rendered fixture artifact (DTO + snapshot).
C=(
  ifname-nix-rust-parity minijail-validator-audio
  minijail-validator-cloud-hypervisor minijail-validator-gpu
  minijail-validator-otel-host-bridge minijail-validator-swtpm
  minijail-validator-usbip minijail-validator-video minijail-validator-virtiofsd
  minijail-validator-vsock-relay minijail-validator-wayland-proxy
  privileges-json-rust-vs-nix-eval static-invariant-broad-caps
  static-invariant-opaque-key-ids static-invariant-uid0
  static-invariant-world-readable-leak static-invariant-writable-paths
  video-sidecar-hardening-eval loki-label-cardinality-eval tempo-budget-eval
)

# D: pure-Nix value/option/internal-config introspection (nix-unit).
# multi-env-daemon-backed mixes F/C/D assertions; D is the dominant W0 seed.
# W1: split per-assertion
D=(
  autostart-wiring-eval bridge-ipv6-boot-sysctl-eval broker-bundle-path-eval
  broker-caps-eval broker-socket-activation-eval broker-systemd-unit-eval
  daemon-autostart-eval daemon-default-compat-eval daemon-experimental-warning-eval
  group-migration-fresh-install-eval group-rename-semantic-eval
  guest-config-containment-eval guest-control-auth-eval guest-control-vsock-eval
  guest-exec-policy-eval ipv6-off-readback multi-env-daemon-backed
  net-vm-network-eval niri-vm-borders-eval observability-eval polkit-allowlist-eval
  per-vm-state-ownership-eval principal-uid-collision-eval readiness-waves-eval
  restart-policy-eval state-dir-acl-eval store-overlay-emit-eval
  store-sync-export-eval umask-roundtrip-eval usbip-gating-eval
  v1.1-kernel-floor-eval video-contract-eval volume-mounts-eval
)

# E: eval-must-fail (nix-unit Bucket-A value + Bucket-B expectedError).
E=(assertions-eval supervisor-option-absent-eval)

# F: build/derivation invariant + per-example evals + schema strictness.
F=(
  cli-nix-consumers-eval examples-with-observability-eval
  guest-static-consumption-eval harness-ubuntu-eval legacy-unit-denylist-eval
  static-invariant-deny-unknown-fields static-invariant-deny-unknown-fields-w3
)

# H: structural/source/doc cross-reference lint (policy scanner).
H=(
  adr-0015-presence-eval adr-index-coverage agents-md-rewrite-eval
  changelog-v1-cut-eval ci-coverage ci-uses-make cli-contract-coverage
  deliverable-gate-inventory guest-control-auth-nongoals
  guest-control-vsock-helper-static guest-exec-runtime-static host-prep-dag-eval
  kernel-module-matrix-eval kernel-modules-parity-eval l3-pin-consistency
  layer1-self-inventory legacy-group-name-denylist legacy-group-name-denylist-self-test
  manpage-completeness-eval microvm-nix-absent-eval no-bash-exec-eval no-new-deferral
  otel-acl-migration-eval pr-checklist-gate privileges-doc-completeness-eval
  privileges-matrix-completeness processes-json-eval release-tag-eval
  static-rust-dependency-direction stop-dag-reconcile-eval tap-dag-contract-doc-eval
  tracing-contract-lint vfsd-watchdog-retired-eval vm-submodule-cutover-eval
  vm-submodule-eval
)

# G-ci: device-free runNixOSTest VM tests (W0 placeholder; W4 CI harness lands later).
GCI=(audio audit-forwarding network-isolation nixling-store state-dir-acl-runtime swtpm-persistence-smoke)

# G-hw: real device passthrough / full microVM boot (OFF-CI, NixOS host w/ devices).
GHW=(hardware-smoke-gpu-yubikey live-vm-smoke)

# perf: scheduled timing budgets (stable runner only).
PERF=(performance-budgets)

# ORCH / helpers: NOT test cases (become `make`-target plumbing); excluded.
ORCH=(static static-fast static-fast-tier0 static-timing runner preflight-disk-space cli-rust-native-common lib)

GROUP_NAMES=(A B C D E F H GCI GHW PERF ORCH)

declare -A existing_group existing_make_target existing_tier
declare -A existing_status existing_successors existing_exercised
existing_order=()
PRESERVE_EXERCISED=0

contains_script() {
  local needle="$1" group_name="$2" item
  local -n group_ref="$group_name"
  for item in "${group_ref[@]}"; do
    if [ "$item" = "$needle" ]; then
      return 0
    fi
  done
  return 1
}

group_of() {
  local name="$1" group
  for group in "${GROUP_NAMES[@]}"; do
    if contains_script "$name" "$group"; then
      printf '%s' "$group"
      return 0
    fi
  done
  printf '%s' "?"
}

target_of() {
  case "$1" in
    A) echo test-rust ;;
    B) echo test-drift ;;
    C) echo test-contract ;;
    D|E) echo test-nix-unit ;;
    F) echo test-flake ;;
    H) echo test-policy ;;
    GCI) echo test-integration ;;
    GHW) echo test-hardware ;;
    PERF) echo perf ;;
    *) echo "" ;;
  esac
}

tier_of() {
  case "$1" in
    GCI) echo ci-kvm ;;
    GHW) echo manual-hw ;;
    PERF) echo scheduled ;;
    *) echo ci-l1 ;;
  esac
}

load_existing_state() {
  local ledger="$1" name group make_target tier status successors exercised
  [ -f "$ledger" ] || return 0

  if grep -qF 'exercised_today is a W0 script-level heuristic' "$ledger"; then
    PRESERVE_EXERCISED=1
  fi

  while IFS=$'\t' read -r name group make_target tier status successors exercised; do
    [ -n "$name" ] || continue
    existing_order+=("$name")
    existing_group["$name"]=${group:-}
    existing_make_target["$name"]=${make_target:-}
    existing_tier["$name"]=${tier:-}
    existing_status["$name"]=${status:-legacy}
    existing_successors["$name"]=${successors:-[]}
    existing_exercised["$name"]=${exercised:-yes}
  done < <(awk '
    function trim(s) { gsub(/^[[:space:]]+|[[:space:]]+$/, "", s); return s }
    function value(line) {
      sub(/^[^=]*=[[:space:]]*/, "", line)
      sub(/[[:space:]]+#.*/, "", line)
      return trim(line)
    }
    function unquote(s) { sub(/^"/, "", s); sub(/"$/, "", s); return s }
    function flush() {
      if (name != "") {
        print name "\t" group "\t" make_target "\t" tier "\t" status "\t" successors "\t" exercised
      }
    }
    /^\[\[script\]\]/ { flush(); name=""; group=""; make_target=""; tier=""; status=""; successors=""; exercised=""; next }
    /^name[[:space:]]*=/ { name=unquote(value($0)); next }
    /^group[[:space:]]*=/ { group=unquote(value($0)); next }
    /^make_target[[:space:]]*=/ { make_target=unquote(value($0)); next }
    /^tier[[:space:]]*=/ { tier=unquote(value($0)); next }
    /^status[[:space:]]*=/ { status=unquote(value($0)); next }
    /^successor_ids[[:space:]]*=/ { successors=value($0); next }
    /^exercised_today[[:space:]]*=/ { exercised=unquote(value($0)); next }
    END { flush() }
  ' "$ledger")
}

successors_empty() {
  local value="$1" compact
  compact=${value//[[:space:]]/}
  [ -z "$compact" ] || [ "$compact" = "[]" ]
}

detect_exercised_today() {
  local script="$1"

  if grep -Eiq 'NL_LIVE|NL_RUN_LAYER2|requires[[:space:]]+a[[:space:]]+live[[:space:]]+host|/dev/kvm|sudo[[:space:]]+-n|NIXLING_PERF_STABLE|manual-only|OFF-CI|manual[[:space:]-]+(gate|obligation|attestation)|NixOS host|real[[:space:]]+(GPU|device|hardware)|YubiKey|hardware-TPM|full[[:space:]]+microVM' "$script"; then
    echo manual
  elif grep -Eiq '(^|[^[:alnum:]_])(SKIP|NL_SKIP)|exit[[:space:]]+77|skips?' "$script"; then
    echo skip
  else
    echo yes
  fi
}

status_for() {
  local rel="$1"
  printf '%s' "${existing_status[$rel]:-legacy}"
}

successors_for() {
  local rel="$1"
  printf '%s' "${existing_successors[$rel]:-[]}"
}

exercised_for() {
  local rel="$1" script="$2" detected
  detected=$(detect_exercised_today "$script")
  if [ "$PRESERVE_EXERCISED" -eq 1 ] && [ "${existing_exercised[$rel]+set}" = set ]; then
    printf '%s' "${existing_exercised[$rel]}"
  else
    printf '%s' "$detected"
  fi
}

self_check() {
  local fail=0 file name hits group item rel successors retired_count=0

  for file in "$ROOT"/tests/*.sh; do
    name=$(basename "$file" .sh)
    hits=0
    for group in "${GROUP_NAMES[@]}"; do
      local -n group_ref="$group"
      for item in "${group_ref[@]}"; do
        [ "$item" = "$name" ] && hits=$((hits + 1))
      done
    done
    if [ "$hits" -eq 0 ]; then
      echo "UNASSIGNED: tests/$name.sh (classify it before merge)" >&2
      fail=1
    fi
    if [ "$hits" -gt 1 ]; then
      echo "DOUBLY ASSIGNED: tests/$name.sh ($hits groups)" >&2
      fail=1
    fi
  done

  for group in "${GROUP_NAMES[@]}"; do
    local -n group_ref="$group"
    for name in "${group_ref[@]}"; do
      rel="tests/$name.sh"
      if [ ! -f "$ROOT/$rel" ]; then
        successors="${existing_successors[$rel]:-[]}"
        if ! successors_empty "$successors"; then
          continue
        fi
        echo "LEDGER NAMES MISSING FILE WITHOUT SUCCESSOR: $rel ($group)" >&2
        fail=1
      fi
    done
  done

  for rel in "${existing_order[@]}"; do
    case "$rel" in
      tests/*.sh) ;;
      *) continue ;;
    esac
    [ -f "$ROOT/$rel" ] && continue
    successors="${existing_successors[$rel]:-[]}"
    if successors_empty "$successors"; then
      echo "RETIRED WITHOUT SUCCESSOR: $rel (set successor_ids before deleting the legacy script)" >&2
      fail=1
    else
      retired_count=$((retired_count + 1))
    fi
  done

  if [ "$fail" -ne 0 ]; then
    echo "check-inventory: ledger does not cover tests/ 1:1 — fix before generating" >&2
    exit 1
  fi
  echo "check-inventory: 1:1 self-check passed" >&2
  echo "check-inventory: retired-row self-check passed ($retired_count retired row(s))" >&2
}

emit_script_row() {
  local rel="$1" group="$2" make_target="$3" tier="$4" status="$5" successors="$6" exercised="$7"
  local status_comment="legacy | porting | ported"
  if [ "$status" = "retired" ]; then
    status_comment="$status_comment | retired"
  fi

  echo "[[script]]"
  echo "name = \"$rel\""
  echo "group = \"$group\""
  echo "make_target = \"$make_target\""
  echo "tier = \"$tier\""
  echo "status = \"$status\"      # $status_comment"
  echo "successor_ids = $successors"
  echo "exercised_today = \"$exercised\""
  echo
}

emit_ledger() {
  local destination="$1" file name group rel status successors exercised make_target tier
  {
    echo "# tests/migration-ledger.toml — AUTO-GENERATED by tests/tools/gen-migration-ledger.sh."
    echo "# W0 seed: one row per script. W1+ refines to one row per assertion."
    echo "# exercised_today is a W0 script-level heuristic (yes|skip|manual); W1 refines it per-assertion."
    echo "# Migration state (status, successor_ids, exercised_today) is preserved across regenerations."
    echo "# Retired rows are append-only coverage history and must keep non-empty successor_ids."
    echo "# Do not edit classification fields by hand; run make ledger-regen. Seed of the check-inventory gate."
    echo "schema_version = 0"
    echo "generated_for = \"test-rearchitecture W0\""
    echo
    for file in "$ROOT"/tests/*.sh; do
      name=$(basename "$file" .sh)
      group=$(group_of "$name")
      [ "$group" = "ORCH" ] && continue
      [ "$group" = "?" ] && continue
      rel="tests/$name.sh"
      status=$(status_for "$rel")
      successors=$(successors_for "$rel")
      exercised=$(exercised_for "$rel" "$file")
      emit_script_row "$rel" "$group" "$(target_of "$group")" "$(tier_of "$group")" "$status" "$successors" "$exercised"
    done
    for rel in "${existing_order[@]}"; do
      case "$rel" in
        tests/*.sh) ;;
        *) continue ;;
      esac
      [ -f "$ROOT/$rel" ] && continue
      successors=$(successors_for "$rel")
      successors_empty "$successors" && continue
      group="${existing_group[$rel]}"
      make_target="${existing_make_target[$rel]}"
      tier="${existing_tier[$rel]}"
      exercised="${existing_exercised[$rel]:-manual}"
      emit_script_row "$rel" "$group" "$make_target" "$tier" "retired" "$successors" "$exercised"
    done
  } > "$destination"
}

load_existing_state "$OUT"
self_check

if [ "$MODE" = check ]; then
  [ -f "$OUT" ] || { echo "check-inventory: missing committed ledger $OUT" >&2; exit 1; }
  scratch="$ROOT/tests/.migration-ledger.toml.check.${BASHPID:-$$}"
  if [ -e "$scratch" ]; then
    echo "check-inventory: scratch path already exists: $scratch" >&2
    exit 1
  fi
  trap 'rm -f "$scratch"' EXIT
  emit_ledger "$scratch"
  if diff -u "$OUT" "$scratch"; then
    echo "check-inventory: ledger is up to date ($(grep -c '^\[\[script\]\]' "$OUT") test rows; ORCH excluded)"
  else
    echo "check-inventory: ledger drift detected; run make ledger-regen and commit the result" >&2
    exit 1
  fi
else
  emit_ledger "$OUT"
  echo "wrote $OUT ($(grep -c '^\[\[script\]\]' "$OUT") test rows; ORCH excluded)"
fi
