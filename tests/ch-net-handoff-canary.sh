#!/usr/bin/env bash
# tests/ch-net-handoff-canary.sh — W3 executable canary for ch-net-handoff-not-supported (test-5).
#
# Closes W3 work-review R1 finding test-5 ("the test for
# `ch-net-handoff-not-supported` is a doc-grep, not a runtime probe").
#
# W3fu2 H3 (test-2): replaced the fake-shim + grep approach with a
# direct cargo-test invocation that exercises the real
# `nixling_host::runner_shape::probe_ch_net_handoff_mode` function.
# The previous shape (fake CH shim + grep of the capability JSON +
# grep of nixling-ipc for `CreateTapFd`) would have passed even if
# `probe_ch_net_handoff_mode` were deleted from the crate, because
# nothing on the canary's execution path actually called it.
#
# Scratch state lives outside $ROOT (W2fu4 H8/H9/H14/H15).

set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
# shellcheck source=lib.sh
. "$HERE/lib.sh"

nl_activate_rust_toolchain_path || true

cd "$ROOT"

if [ -z "${NIXLING_CH_NET_HANDOFF_CANARY_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "ch-net-handoff-canary: neither cargo nor nix is on PATH"
  fi
  export NIXLING_CH_NET_HANDOFF_CANARY_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

WORKSPACE_DIR=$ROOT/packages

log "W3 ch-net-handoff executable canary"

# Step 1: exercise the real Rust probe via cargo test. These two
# tests in packages/nixling-host/src/runner_shape.rs (named
# `ch_help_with_fd_selects_tap_fd` and `ch_help_without_fd_or_tap_fails_closed`)
# call `probe_ch_net_handoff_mode` against representative `ch --help`
# excerpts and assert the documented outcomes. Running them here
# wires the L1 gate to the real production code path; if the probe
# function is deleted or its parsing logic regresses, the cargo test
# fails and this gate fails closed.
log " - cargo test -p nixling-host -- ch_help_with_fd_selects_tap_fd ch_help_without_fd_or_tap_fails_closed"
( cd "$WORKSPACE_DIR" \
  && CARGO_BUILD_RUSTC_WRAPPER="" cargo test -p nixling-host --quiet -- \
       ch_help_with_fd_selects_tap_fd \
       ch_help_without_fd_or_tap_fails_closed )

# Step 2: closed-table golden envelope check. The H4-shipped human +
# JSON goldens for `host check --json` carry the `ch-net-handoff-not-supported`
# error code; if the golden file disappears or its code field drifts,
# the operator-facing contract has silently changed and we fail closed.
GOLDEN=$ROOT/tests/golden/cli-output/host-check-ch-net-handoff-not-supported.json
if [ ! -f "$GOLDEN" ]; then
  fail "ch-net-handoff canary: golden $GOLDEN missing"
fi
if ! grep -q '"code": "ch-net-handoff-not-supported"' "$GOLDEN"; then
  fail "ch-net-handoff canary: golden envelope code field drifted"
fi
ok "host-check golden envelope for ch-net-handoff-not-supported present and correctly coded"

# Step 3: assert `CreateTapFd` is declared in the W3 broker wire
# contract. This is a structural contract check (not an execution
# one) and complements Step 1's real-probe execution: if the wire
# variant disappears, the broker can no longer refuse it with the
# documented audit shape.
if grep -RnE 'CreateTapFd' "$ROOT/packages/nixling-ipc/src" >/dev/null; then
  ok "CreateTapFd is declared in the W3 broker wire contract (nixling-ipc)"
else
  fail "ch-net-handoff canary: CreateTapFd not declared in nixling-ipc"
fi

log "OK: ch-net-handoff executable canary"
