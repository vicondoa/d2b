#!/usr/bin/env bash
# tests/test-flake.sh — `make test-flake`: `nix flake check` for the build's
# NATIVE system only (bounded memory).
#
# CI shards the x86_64-linux checks one-job-per-check. The aarch64 PR job is a
# lightweight smoke eval only, not a full flake check, to avoid spending ARM
# runner resources on the longest evaluation leg. The previous monolithic
# `nix flake check --all-systems` cross-evaluated both architectures in one
# process and OOM-killed the 16 GB GitHub runner once the rearchitecture grew
# flake.checks (nix-unit corpus, cargo-deny/cargo-audit derivations, more
# example evals).
#
# Set NL_FLAKE_ALL_SYSTEMS=1 to cross-evaluate every supported system in one
# process (the heavier `make check` / tests/static.sh local gate does this on a
# large-memory host).

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
NL_LOG=${NL_LOG:-/dev/null}
export ROOT NL_LOG

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"
cd "$ROOT"

# git+file:// (never a bare path): source-capture from the git tree only, so the
# sibling cargo target/ + scratch dirs stay invisible to the eval (disk-hygiene
# contract — see tests/lib.sh nl_flake_ref).
flake_ref=$(nl_flake_ref "$ROOT")

dump_flake_eval_segfault() {
  local label="$1"
  shift

  log "  SEGFAULT diagnostics for $label"
  {
    echo "=== command ==="
    printf '%q ' "$@"
    echo
    echo "=== system ==="
    uname -a || true
    nix --version || true
    nix-instantiate --version || true
    echo "=== limits ==="
    ulimit -a || true
    echo "=== memory ==="
    free -h || true
    echo "=== disk ==="
    df -h || true
    echo "=== process status ==="
    grep -E '^(Name|State|Threads|VmPeak|VmSize|VmRSS|VmStk|Sig)' /proc/$$/status || true
  } >&2

  if command -v gdb >/dev/null 2>&1; then
    log "  SEGFAULT gdb backtrace for $label"
    gdb --batch --quiet \
      -ex 'set pagination off' \
      -ex 'set print frame-arguments all' \
      -ex 'set print elements 64' \
      -ex run \
      -ex 'thread apply all bt full' \
      -ex 'info registers' \
      -ex quit \
      --args "$@" >&2 || true
  else
    log "  SEGFAULT gdb backtrace skipped for $label (gdb not found)"
  fi
}

# Local all-shards mode: mirror CI's x86 flake fan-out on one host instead of
# re-combining every check into a monolithic evaluator process. This is used by
# the Layer-1 manifest runner behind `make check`; `NL_FLAKE_JOBS` bounds local
# concurrency so operators can tune Nix-daemon pressure.
if [ "${NL_FLAKE_LOCAL_SHARDS:-0}" = 1 ]; then
  native=$(nix eval --raw --impure --expr builtins.currentSystem 2>/dev/null || echo "native")
  log "--> flake local shards: checks.$native.* + packages.$native.*"

  flake_jobs=${NL_FLAKE_JOBS:-4}
  case "$flake_jobs" in
    ""|*[!0-9]*)
      fail "NL_FLAKE_JOBS must be a positive integer"
      exit 2
      ;;
  esac
  if [ "$flake_jobs" -lt 1 ]; then
    fail "NL_FLAKE_JOBS must be >= 1"
    exit 2
  fi

  mapfile -t shard_checks < <(
    nix eval --raw "${flake_ref}#checks.${native}" --apply \
      'checks: builtins.concatStringsSep "\n" (builtins.attrNames checks)'
  )
  if [ "${#shard_checks[@]}" -eq 0 ]; then
    fail "flake local shards: no checks discovered for $native"
    exit 1
  fi

  shard_dir=$(mktemp -d "${TMPDIR:-/tmp}/nixling-flake-shards.XXXXXX")
  add_cleanup "rm -rf -- $(printf '%q' "$shard_dir")"
  declare -A shard_label=()
  declare -A shard_log=()
  declare -A shard_status=()
  declare -A shard_done=()
  active=0
  failed=0

  run_flake_local_item() {
    local kind="$1" value="${2:-}"
    case "$kind" in
      check)
        env -u NL_FLAKE_LOCAL_SHARDS -u NL_FLAKE_OUTPUTS \
          NL_FLAKE_CHECK="$value" bash "$0"
        ;;
      outputs)
        env -u NL_FLAKE_LOCAL_SHARDS -u NL_FLAKE_CHECK \
          NL_FLAKE_OUTPUTS=1 bash "$0"
        ;;
      *)
        echo "unknown local flake shard kind: $kind" >&2
        return 2
        ;;
    esac
  }

  harvest_flake_shards() {
    local pid rc label log_path status_path
    for pid in "${!shard_label[@]}"; do
      [ -n "${shard_done[$pid]:-}" ] && continue
      status_path=${shard_status[$pid]}
      [ -f "$status_path" ] || continue
      rc=$(cat "$status_path")
      rm -f -- "$status_path"
      shard_done[$pid]=1
      active=$((active - 1))
      label=${shard_label[$pid]}
      log_path=${shard_log[$pid]}
      if [ "$rc" -eq 0 ]; then
        ok "$label"
      else
        log "$label FAILED - tail follows:"
        tail -120 "$log_path" >&2 || true
        failed="$rc"
      fi
    done
  }

  wait_one_flake_shard() {
    wait -n || true
    harvest_flake_shards
  }

  spawn_flake_shard() {
    local kind="$1" value="${2:-}" label key pid log_path status_path
    if [ "$kind" = "outputs" ]; then
      label="flake non-checks outputs: packages.$native"
      key="outputs"
    else
      label="flake check shard: $value"
      key="check-$value"
    fi
    while [ "$active" -ge "$flake_jobs" ]; do
      wait_one_flake_shard
    done
    log_path="$shard_dir/${key}.log"
    status_path="$shard_dir/${key}.status"
    rm -f -- "$log_path" "$status_path"
    (
      rc=0
      set +e
      run_flake_local_item "$kind" "$value" >"$log_path" 2>&1
      rc=$?
      set -e
      printf '%s\n' "$rc" > "$status_path"
      exit 0
    ) &
    pid=$!
    shard_label[$pid]="$label"
    shard_log[$pid]="$log_path"
    shard_status[$pid]="$status_path"
    active=$((active + 1))
  }

  for check in "${shard_checks[@]}"; do
    spawn_flake_shard check "$check"
  done
  spawn_flake_shard outputs
  while [ "$active" -gt 0 ]; do
    wait_one_flake_shard
  done

  if [ "$failed" -ne 0 ]; then
    fail "flake local shards"
    exit "$failed"
  fi
  ok "flake local shards (${#shard_checks[@]} checks + outputs)"
  log "test-flake (local shards) OK"
  exit 0
fi

# Single-check shard mode (CI dynamic matrix): NL_FLAKE_CHECK=<name> instantiates
# just that one flake check's derivation for the native system, matching the
# `--no-build` semantics of the full sweep (evaluate + instantiate, do not
# build). Sharding lets CI fan the checks out across parallel runners so no
# single evaluator process holds every nixosSystem toplevel at once — the
# OOM/swap-spill the monolithic `nix flake check` hit on a 16 GB hosted runner.
# The complementary `test-flake-aarch64` job runs only the dedicated
# smoke-eval-aarch64 expression. `NL_FLAKE_OUTPUTS=1` (below) sweeps x86
# non-`checks` outputs.
if [ -n "${NL_FLAKE_CHECK:-}" ]; then
  # Defense in depth: the CI matrix sources these names from the flake's check
  # attrNames, but reject anything outside a safe charset before it reaches the
  # nix attr path / any shell so a hostile attr name can neither inject nor
  # silently no-op a shard.
  case "$NL_FLAKE_CHECK" in
    ""|*[!A-Za-z0-9._-]*)
      fail "NL_FLAKE_CHECK '${NL_FLAKE_CHECK}' has characters outside [A-Za-z0-9._-]"
      exit 1
      ;;
  esac
  native=$(nix eval --raw --impure --expr builtins.currentSystem 2>/dev/null || echo "native")
  log "--> flake check shard: checks.$native.${NL_FLAKE_CHECK} (instantiate-only)"
  set +e
  nix eval --raw "${flake_ref}#checks.${native}.${NL_FLAKE_CHECK}.drvPath" >/dev/null
  rc=$?
  set -e
  if [ "$rc" -eq 0 ]; then
    ok "flake check shard: ${NL_FLAKE_CHECK}"
  elif [ "$rc" -eq 139 ]; then
    log "  WARN: nix eval segfaulted for shard ${NL_FLAKE_CHECK}; retrying via nix-instantiate"
    dump_flake_eval_segfault \
      "nix eval ${NL_FLAKE_CHECK}" \
      nix eval --raw "${flake_ref}#checks.${native}.${NL_FLAKE_CHECK}.drvPath"
    if nix-instantiate --eval --strict -E \
      "let f = builtins.getFlake \"${flake_ref}\"; in f.checks.${native}.${NL_FLAKE_CHECK}.drvPath" >/dev/null; then
      ok "flake check shard: ${NL_FLAKE_CHECK} (nix-instantiate fallback)"
    else
      fallback_rc=$?
      if [ "$fallback_rc" -eq 139 ]; then
        dump_flake_eval_segfault \
          "nix-instantiate ${NL_FLAKE_CHECK}" \
          nix-instantiate --eval --strict -E \
          "let f = builtins.getFlake \"${flake_ref}\"; in f.checks.${native}.${NL_FLAKE_CHECK}.drvPath"
      fi
      fail "flake check shard: ${NL_FLAKE_CHECK}"
      exit 1
    fi
  else
    fail "flake check shard: ${NL_FLAKE_CHECK}"
    exit 1
  fi
  log "test-flake (shard ${NL_FLAKE_CHECK}) OK"
  exit 0
fi

# Non-`checks` output sweep (CI x86 completeness): the per-check shards above
# cover `checks.<sys>.*`, but `nix flake check` also validates the other
# per-system outputs. This flake only exposes `packages.<sys>` with content
# (apps is empty; lib is system-agnostic), so instantiate every package
# derivation. This closes the gap where the sharded `test-flake-x86` context
# could pass with a broken x86 `packages` output that the lightweight aarch64
# smoke job would not catch.
if [ "${NL_FLAKE_OUTPUTS:-0}" = 1 ]; then
  native=$(nix eval --raw --impure --expr builtins.currentSystem 2>/dev/null || echo "native")
  log "--> flake non-checks outputs: packages.$native.* (instantiate-only)"
  if nix eval --raw "${flake_ref}#packages.${native}" --apply \
       'ps: builtins.concatStringsSep "\n" (builtins.map (p: p.drvPath) (builtins.attrValues ps))' \
       >/dev/null; then
    ok "flake non-checks outputs: packages.$native"
  else
    fail "flake non-checks outputs: packages.$native"
    exit 1
  fi
  log "test-flake (outputs) OK"
  exit 0
fi

systems_flag=()
if [ "${NL_FLAKE_ALL_SYSTEMS:-0}" = 1 ]; then
  systems_flag=(--all-systems)
  log "--> nix flake check --no-build --all-systems"
else
  native=$(nix eval --raw --impure --expr builtins.currentSystem 2>/dev/null || echo "native")
  log "--> nix flake check --no-build (native system: $native)"
fi

if nix flake check "$flake_ref" --no-build "${systems_flag[@]}"; then
  ok "nix flake check"
else
  fail "nix flake check"
  exit 1
fi

log "test-flake OK"
