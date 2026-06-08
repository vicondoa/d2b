#!/usr/bin/env bash
# W3 s4 L1c canary: runner-shape preflight drift detection.
#
# Drives nixling-host::runner_shape against the parity-drift golden
# fixture (plan.md §"W3 runner-shape preflight"). Asserts every
# expected failure class fires.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
  PATH="$NL_RUST_TOOLCHAIN_PATH:$PATH"
  export PATH
fi

if [ -z "${NIXLING_W3_S4_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "runner-shape-preflight: neither cargo nor nix is on PATH" >&2
    exit 1
  fi
  export NIXLING_W3_S4_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc nixpkgs#sccache nixpkgs#jq \
    --command bash "$0" "$@"
fi

# shellcheck source=lib.sh
. "$HERE/lib.sh"

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$(nl_cargo_target_dir workspace)}
export RUSTC_WRAPPER=${RUSTC_WRAPPER:-}

log "==> tests/runner-shape-preflight.sh"

FIXTURE="$ROOT/tests/golden/runner-shape/parity-drift.json"
[ -r "$FIXTURE" ] || { echo "missing fixture: $FIXTURE" >&2; exit 1; }

# Assert every drift class declared in the fixture is recognized by jq
# (catches silent schema drift in the fixture).
log "  fixture: $FIXTURE"
jq -e '
  (.chCapabilitiesDeclared | length) >= 1 and
  (.declaredRunnerArgv[0].argvHash == "") and
  (.chApiSocketPaths[0].owner == "") and
  (.vsockTransports[0].transport != "unix") and
  ((.sidecarNodes[0].dagNodeId) as $id | ((.processesDagNodeIds | index($id)) == null))
' "$FIXTURE" >/dev/null

cd "$ROOT/packages"

log "  canary: runner-shape-drift"
for t in \
  happy_path_yields_only_ok_finding \
  missing_declared_runner_fails_closed \
  empty_argv_hash_is_runner_shape_drift \
  capability_drift_surfaces \
  non_unix_vsock_transport_rejected \
  sidecar_dag_mismatch_surfaces \
  parity_drift_fixture_fails_closed \
  ch_help_with_fd_selects_tap_fd \
  ch_help_with_tap_only_selects_persistent_tap \
  ch_help_without_fd_or_tap_fails_closed; do
  cargo test -p nixling-host --all-features --lib runner_shape::tests::$t 2>&1 | tail -3
done

ok "tests/runner-shape-preflight.sh: every W3 runner-shape canary passed"
