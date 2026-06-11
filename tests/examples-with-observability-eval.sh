#!/usr/bin/env bash
# tests/examples-with-observability-eval.sh—
# gate.
#
# Asserts that `examples/with-observability/` evaluates cleanly via
# its own `flake.nix` and materialises the operator-visible surface
# documented in its README:
#
#   * the example's `nix flake check --no-build --all-systems
#     --no-write-lock-file` passes against the in-tree framework
#     (the example pins `nixling.url = "path:../.."`);
#   * `configuration.nix` sets the documented host/per-VM
#     observability toggles and uses the canonical `sys-obs` VM name;
#   * the resolved NixOS configuration auto-declares
#     `nixling.envs.obs` and the `sys-obs` VM, and the
#     workload VM `work-app` has per-VM observability enabled.
#
# Skips with SKIP=75 if `nix` is unavailable. Fails closed on any
# eval error.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
EXAMPLE_DIR="$ROOT/examples/with-observability"
CONFIG_NIX="$EXAMPLE_DIR/configuration.nix"
FLAKE_NIX="$EXAMPLE_DIR/flake.nix"

export NL_LOG=${NL_LOG:-$ROOT/.examples-with-observability-eval.log}
export TMPDIR=${TMPDIR:-$ROOT/.copilot-work}
mkdir -p "$TMPDIR"

# shellcheck source=lib.sh
. "$HERE/lib.sh"

log "==> tests/examples-with-observability-eval.sh"

PASS=0
FAIL=0

ok()   { log "  PASS: $*"; PASS=$((PASS + 1)); }
bad()  { log "  FAIL: $*"; FAIL=$((FAIL + 1)); }

# -----------------------------------------------------------------------------
# 1. File-level invariants — the example must exist with the expected layout.
# -----------------------------------------------------------------------------

for f in "$FLAKE_NIX" "$CONFIG_NIX" "$EXAMPLE_DIR/README.md"; do
  if [ -f "$f" ]; then
    ok "present: ${f#$ROOT/}"
  else
    bad "missing: ${f#$ROOT/}"
  fi
done

# `flake.nix` must wire `./configuration.nix` into the `demo`
# config — the example layout this gate asserts on.
if grep -q '\./configuration\.nix' "$FLAKE_NIX"; then
  ok "flake.nix imports ./configuration.nix"
else
  bad "flake.nix does not import ./configuration.nix"
fi

# -----------------------------------------------------------------------------
# 2. configuration.nix surface assertions — operator-visible toggles.
# -----------------------------------------------------------------------------

config_grep() {
  local pattern="$1"
  local label="$2"
  if grep -Eq "$pattern" "$CONFIG_NIX"; then
    ok "configuration.nix sets $label"
  else
    bad "configuration.nix missing $label"
  fi
}

config_grep 'nixling\.observability[[:space:]]*=|nixling\.observability\.enable[[:space:]]*=[[:space:]]*true' \
  'nixling.observability.enable = true'
config_grep 'nixling\.envs\.work[[:space:]]*=' \
  'workload env nixling.envs.work'
config_grep 'nixling\.vms\.work-app[[:space:]]*=' \
  'workload VM nixling.vms.work-app'
config_grep 'observability\.enable[[:space:]]*=[[:space:]]*true' \
  'per-VM observability.enable = true on work-app'

# -----------------------------------------------------------------------------
# 3. Eval-time gate — the example's flake.nix must evaluate cleanly.
#    Two layers:
#      (a) `nix flake check` in the example dir (matches the
#          per-example loop in tests/static.sh);
#      (b) targeted eval of the resolved NixOS config to assert the
#          framework-visible toggles took effect.
# -----------------------------------------------------------------------------

if ! command -v nix >/dev/null 2>&1; then
  log "  SKIP: nix not on PATH — skipping eval-time assertions"
  log "==> summary: PASS=$PASS FAIL=$FAIL SKIP=1"
  if [ "$FAIL" -gt 0 ]; then
    exit 1
  fi
  exit 0
fi

scratch=$(nl_mktemp .with-observability-eval.XXXXXX)
flake_check_log="$scratch/flake-check.log"
if (cd "$EXAMPLE_DIR" && nix flake check --no-build --all-systems --no-write-lock-file) \
    >"$flake_check_log" 2>&1; then
  ok "nix flake check (examples/with-observability)"
else
  bad "nix flake check (examples/with-observability)"
  tail -40 "$flake_check_log" | sed 's/^/    /' >&2 || true
fi

# Targeted eval — read back the resolved NixOS config and assert the
# observability surface the README documents.
# (eval_expr removed — we evaluate the resolved flake output directly via
# `nix eval EXAMPLE_DIR#nixosConfigurations.demo.config.nixling --apply` below.)

eval_log="$scratch/eval.log"
if eval_json=$(nix --no-warn-dirty eval --json --no-write-lock-file \
    "$EXAMPLE_DIR#nixosConfigurations.demo.config.nixling" --apply '
      cfg: {
        obsEnable        = cfg.observability.enable;
        obsVmName        = cfg.observability.vmName;
        obsEnvName       = cfg.observability.env;
        obsEnvDeclared   = builtins.hasAttr cfg.observability.env cfg.envs;
        obsVmDeclared    = builtins.hasAttr cfg.observability.vmName cfg.vms;
        workEnvDeclared  = builtins.hasAttr "work" cfg.envs;
        workAppDeclared  = builtins.hasAttr "work-app" cfg.vms;
        workAppObsEnable =
          if builtins.hasAttr "work-app" cfg.vms
          then cfg.vms.work-app.observability.enable
          else false;
      }
    ' 2>"$eval_log"); then
  ok "nix eval (resolved nixling.observability surface)"

  check_json_field() {
    local field="$1"
    local expected="$2"
    local actual
    actual=$(printf '%s' "$eval_json" | jq -c ".$field")
    if [ "$actual" = "$expected" ]; then
      ok "resolved $field = $expected"
    else
      bad "resolved $field = $actual (expected $expected)"
    fi
  }

  check_json_field obsEnable          true
  check_json_field obsVmName          '"sys-obs"'
  check_json_field obsEnvName         '"obs"'
  check_json_field obsEnvDeclared     true
  check_json_field obsVmDeclared      true
  check_json_field workEnvDeclared    true
  check_json_field workAppDeclared    true
  check_json_field workAppObsEnable   true
else
  bad "nix eval (resolved nixling.observability surface)"
  tail -40 "$eval_log" | sed 's/^/    /' >&2 || true
fi

log "==> summary: PASS=$PASS FAIL=$FAIL"

if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
