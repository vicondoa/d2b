#!/usr/bin/env bash
# tests/host-prepare-idempotency.sh — W3 idempotency drift-digest invariant (L1c fake-backend variant).
#
# Closes W3 work-review R1 findings software-4 + test-2 ("plan §"W3
# idempotency no-op invariant (testable)" §2694-2704 is documented but
# no script exercises the apply→dry-run-empty→apply-zero-mut→destroy
# →destroy-noop sequence").
#
# W3fu3 H7 (test-1) scope correction: the gate as it stands today only
# runs the `idempotency_*` Rust tests under nixling-host that exercise
# the *drift-digest stability* invariant the broker relies on
# (`hash_inet_nixling_table` returns the same canonical digest when
# given the same input, and stays stable under kernel-assigned
# `handle`/`index` volatile fields). The full apply→dry-run-empty
# →apply-zero-mut→destroy→destroy-noop state-machine oracle requires
# the per-module fake netlink/nft/NM/sysctl backends to be implemented
# in nixling-host/src/fake.rs — that scope ships with W4 alongside
# the production broker reconcile ops (see plan.md "Spec corrections"
# row for the W4 `--apply` wiring). Until W4 lands, the gate only
# guarantees the drift-digest portion of the invariant; the verb
# state-machine portion is asserted only at the wire-level by
# `tests/broker-validate-bundle.sh` and the `cli-rust-native-*`
# gates today.
#
# What this gate guarantees today:
#
#   * the `cargo test -- idempotency` invocation runs at least one
#     test prefixed `idempotency_` under nixling-host;
#   * the zero-test branch fails closed if no `idempotency_*` tests
#     exist (the W3fu2 H3 honesty fix);
#   * any `idempotency_*` test that fails fails the gate.
#
# What this gate does NOT guarantee today (W4 follow-up):
#
#   * actual `nixling host prepare --apply` / `host destroy --apply`
#     CLI execution against a fake host;
#   * end-to-end mutation-counting on repeat apply;
#   * foreign nft / NM / /etc/hosts / sysctl byte-preservation
#     during destroy.
#
# Scratch state lives outside $ROOT per AGENTS.md disk-hygiene
# contract (W2fu4 H8/H9/H14/H15).
#
# TODO(integrator): wire into tests/static.sh after the existing
# nft-coexistence test invocation. Per the W3fu1 H4 contract,
# tests/static.sh is integrator-owned — H4 ships the script + the
# wiring instruction only.

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
# shellcheck source=lib.sh
. "$HERE/lib.sh"

cd "$ROOT"

nl_activate_rust_toolchain_path || true

# The Rust workspace sits under packages/.
WORKSPACE_DIR="$ROOT/packages"

LOG=${TMPDIR:-/tmp}/nixling-host-prepare-idempotency.$$.log
: > "$LOG"
exec > >(tee -a "$LOG") 2>&1

# Scratch outside $ROOT (W2fu4 H8/H9/H14/H15).
SCRATCH=${TMPDIR:-/tmp}/nl-host-prepare-idempotency.$$
mkdir -p "$SCRATCH"
add_cleanup "rm -rf -- '$SCRATCH'"

log "W3 host-prepare idempotency no-op invariant"

# Step 0: build fake-backend driver binaries the Rust test layer needs.
# `nixling-host` exposes the `fake-backends` feature; broker forwards
# it via its `fake-backends` feature.
log " - cargo build -p nixling-host --features fake-backends"
( cd "$WORKSPACE_DIR" && CARGO_BUILD_RUSTC_WRAPPER="" cargo build -p nixling-host --features fake-backends --quiet )

log " - cargo build -p nixling-priv-broker --features fake-backends (best-effort: broker is H1-owned and may not compile at every W3 follow-up cut)"
if ! ( cd "$ROOT/packages/nixling-priv-broker" && CARGO_BUILD_RUSTC_WRAPPER="" cargo build --features fake-backends --quiet 2>/dev/null ); then
  log "   (broker build failed — skipping broker-side oracle; H1 owns the broker runtime per W3 file-ownership map)"
fi

# Step 1-5: run the Rust idempotency oracle tests under nixling-host.
# The Rust layer owns the apply/dry-run/destroy state machine + the
# netlink/nft/NM/sysctl readback helpers; the shell layer asserts the
# closed sequence runs end-to-end. W3fu2 H3 (test-1 / software-1):
# removed the `|| true` mask so a failing cargo test fails the gate.
log " - cargo test -p nixling-host --features fake-backends -- idempotency"
( cd "$WORKSPACE_DIR" && CARGO_BUILD_RUSTC_WRAPPER="" cargo test -p nixling-host --features fake-backends --quiet -- \
  idempotency )

# Step 6: detect "no idempotency_* tests defined yet" — the shell gate
# fails fast in that case so the test never silently passes. W3fu2 H3:
# changed the zero-test branch from log-only to fail so the gate is
# honest about the missing oracle.
log " - probing nixling-host for idempotency_* test functions"
test_count=$( { grep -REh '#\[test\][[:space:]]*\n[[:space:]]*fn[[:space:]]+idempotency_' \
  "$ROOT/packages/nixling-host/src" "$ROOT/packages/nixling-host/tests" 2>/dev/null || true; } \
  | wc -l)
test_count_oneline=$( { grep -RnE '\bfn[[:space:]]+idempotency_[a-z0-9_]+\(' \
  "$ROOT/packages/nixling-host/src" "$ROOT/packages/nixling-host/tests" 2>/dev/null || true; } \
  | wc -l)
if [ "$test_count" -eq 0 ] && [ "$test_count_oneline" -eq 0 ]; then
  fail "host-prepare-idempotency: no idempotency_* test functions found under nixling-host. The Rust oracle bodies must be in tree before this gate can PASS."
fi

ok "host-prepare-idempotency: drift-digest stability invariant exercised (idempotency_* nixling-host tests pass); full prepare/destroy state-machine oracle is W4"
