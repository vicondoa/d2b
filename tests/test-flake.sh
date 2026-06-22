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
    set +e
    nix-instantiate --eval --strict -E \
      "let f = builtins.getFlake \"${flake_ref}\"; in f.checks.${native}.${NL_FLAKE_CHECK}.drvPath" >/dev/null
    inst_rc=$?
    set -e
    if [ "$inst_rc" -eq 0 ]; then
      ok "flake check shard: ${NL_FLAKE_CHECK} (nix-instantiate fallback)"
    elif [ "$inst_rc" -eq 139 ]; then
      log "  WARN: nix-instantiate also segfaulted for shard ${NL_FLAKE_CHECK}; retrying via nix build --dry-run --offline"
      if nix build --offline --dry-run --no-link "${flake_ref}#checks.${native}.${NL_FLAKE_CHECK}" >/dev/null; then
        ok "flake check shard: ${NL_FLAKE_CHECK} (nix build --dry-run --offline fallback)"
      else
        fail "flake check shard: ${NL_FLAKE_CHECK}"
        exit 1
      fi
    else
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
