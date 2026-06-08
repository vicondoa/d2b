#!/usr/bin/env bash
# tests/broker-default-features-build.sh— clean-break gate.
#
# Verifies the broker's default feature set is now empty and the
# production binary compiles clean against the real opaque-ID
# `nixling_ipc::broker_wire::BrokerRequest` shape (no longer
# `layer1-bootstrap`-gated). The clean-break refactor moved
# the bootstrap dispatch path behind an opt-in feature for legacy
# probe-* test harnesses; the production binary uses the real
# wire dispatch from `runtime::dispatch_request` (the
# `#[cfg(not(feature = "layer1-bootstrap"))]` arm).
#
# Scratch state lives outside $ROOT.

set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
# shellcheck source=lib.sh
. "$HERE/lib.sh"

nl_activate_rust_toolchain_path || true

cd "$ROOT"

if [ ! -f packages/nixling-priv-broker/Cargo.toml ]; then
  log "broker-default-features-build inputs absent — skipping"
  exit 0
fi

if [ -z "${NIXLING_BROKER_DEFAULT_FEATURES_BUILD_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "broker-default-features-build: neither cargo nor nix is on PATH"
  fi
  log "  cargo not on PATH; re-entering via nix shell to acquire toolchain"
  export NIXLING_BROKER_DEFAULT_FEATURES_BUILD_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

# Clean-break: assert layer1-bootstrap is NO LONGER a default
# feature so unconfigured builds pick the real-wire surface. If a
# future change re-adds it to the default set, this gate fails fast
# with a clear pointer at the clean-break rationale.
if grep -qE '^default[[:space:]]*=[[:space:]]*\[[^]]*"layer1-bootstrap"[^]]*\]' \
     "$ROOT/packages/nixling-priv-broker/Cargo.toml"; then
  fail "broker-default-features-build: packages/nixling-priv-broker/Cargo.toml [features].default re-added \"layer1-bootstrap\". The W4-fu clean-break moved the bootstrap dispatch shape to an opt-in feature for legacy probe-* test harnesses; the production binary uses the real opaque-ID wire dispatch. Remove \"layer1-bootstrap\" from [features].default or land a justified revert with updated CHANGELOG entry."
fi

log "--> cargo check -p nixling-priv-broker (default features = real wire dispatch)"
(
  cd "$ROOT/packages/nixling-priv-broker"
  CARGO_BUILD_RUSTC_WRAPPER="" cargo check --quiet
)

ok "broker-default-features-build: nixling-priv-broker compiles clean with default features (real opaque-ID wire dispatch; layer1-bootstrap opt-in only)"
