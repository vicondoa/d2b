#!/usr/bin/env bash
# W2 invariant: the Rust workspace dependency graph flows in one
# direction. Binaries (`nixling`, `nixlingd`) and the privileged
# broker (`nixling-priv-broker`, currently a sibling workspace)
# may depend on `nixling-ipc` and `nixling-core`. `nixling-ipc`
# may depend on `nixling-core`. `nixling-core` is a leaf and
# depends on no internal crate. `nixling-priv-broker` must NOT
# depend on `nixlingd` or `nixling` (it sits BELOW the daemon and
# CLI in the trust hierarchy). The CLI/daemon must NOT depend on
# the broker (they reach it only over IPC).
#
# This gate is a pure static parse of `Cargo.toml` files; no cargo
# invocation required.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

cd "$ROOT"

if [ ! -d packages/nixling-core ]; then
  log "packages/ absent — skipping static-rust-dependency-direction (W2 unstaged)"
  exit 0
fi

# Extract direct in-workspace deps from a Cargo.toml. Internal crate
# names match `^(nixling-core|nixling-host|nixling-ipc|nixling-priv-broker|nixling|nixlingd|xtask)$`.
internal_deps() {
  local toml="$1"
  awk '
    /^\[dependencies\]/      { in_deps=1; next }
    /^\[dev-dependencies\]/  { in_deps=1; next }
    /^\[build-dependencies\]/{ in_deps=1; next }
    /^\[target\..*\.dependencies\]/ { in_deps=1; next }
    /^\[/                    { in_deps=0 }
    in_deps && /^[a-zA-Z0-9_-]+/ {
      gsub(/[[:space:]].*$/, "", $1)
      gsub(/=.*$/, "", $1)
      print $1
    }
  ' "$toml" | sort -u
}

declare -A WANT
WANT["nixling-core"]=""
WANT["nixling-ipc"]="nixling-core"
# W3 prep: nixling-host depends only on nixling-core + nixling-ipc.
WANT["nixling-host"]="nixling-core nixling-ipc"
WANT["xtask"]="nixling-core nixling-ipc nixling nixlingd"
WANT["nixling"]="nixling-core nixling-ipc"
WANT["nixlingd"]="nixling-core nixling-host nixling-ipc"
WANT["nixling-priv-broker"]="nixling-core nixling-host nixling-ipc"

INTERNAL_CRATES='^(nixling-core|nixling-host|nixling-ipc|nixling-priv-broker|nixling|nixlingd|xtask)$'

violations=0
for crate in "${!WANT[@]}"; do
  toml="packages/$crate/Cargo.toml"
  if [ ! -f "$toml" ]; then
    log "  SKIP: $crate (Cargo.toml absent)"
    continue
  fi
  actual=$(internal_deps "$toml" | grep -E "$INTERNAL_CRATES" || true)
  expected=${WANT[$crate]}
  # Disallowed = actual ∩ (everything internal) − expected
  for dep in $actual; do
    case " $expected " in
      *" $dep "*) ;;
      *)
        log "  FAIL: $crate depends on $dep (not in allowed set: ${expected:-<none>})"
        violations=$((violations + 1))
        ;;
    esac
  done
done

if [ "$violations" -gt 0 ]; then
  fail "static-rust-dependency-direction: $violations disallowed in-workspace dep edge(s)"
fi

for crate in nixling nixlingd; do
  if ! grep -R -Eq 'use[[:space:]]+nixling_ipc::' "packages/$crate/src"; then
    fail "static-rust-dependency-direction: $crate does not import nixling_ipc from its source tree"
  fi
done

ok "static-rust-dependency-direction: workspace dependency graph flows ipc/core → broker/daemon/cli"
