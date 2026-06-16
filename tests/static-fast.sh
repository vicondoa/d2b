#!/usr/bin/env bash
# tests/static-fast.sh — tier-2 PR-loop subset of tests/static.sh
# (layer split).
#
# This is NOT the sub-60s first-pass gate. For the lightweight syntax /
# shell-lint presubmit, run tests/static-fast-tier0.sh. static-fast.sh is
# the heavier Nix-aware fast gate.
#
# Runs:
#   * preflight-disk-space
#   * nix-instantiate --parse on every .nix file
#   * shellcheck --severity=warning on every .sh
#   * guest-static ELF build/inspection for the current host system
#   * nix flake check --no-build --all-systems
#   * rust-workspace-checks (cargo workspace check / clippy / fmt)
#   * bundle/schema static gates (12 tests, ~3 min)
#   * host-prepare gates (17 tests, ~1 min— pure shell)
#   * static-rust-dependency-direction (parse-only)
#   * consolidated drift gate (drift-check: schemas, error-codes,
#     daemon-api, manpages/completions, guest proto + ttrpc bindings)
#
# Skipped vs tests/static.sh (run those in the full panel gate):
#   - smoke-eval-*.nix (5 toplevel evals, ~4 min, ~50 G disk)
#   - assertions-eval (~37 min after)
#   - mid-tier evals (autostart, restart-policy, video, usbip,
#     bridge-isolation) — ~9 min, each materializes a system closure
#   - manifest contract (~1 min)
#   - control-plane gates (~12 min cargo + broker test daemons)
#   - per-example/template flake-check (~3 min wall but ~700 G disk)
#   - cli-contract-coverage (~7 min cold; builds nixling CLI binary
#     to validate parser/help against docs)
#   - cli-json-drift (~6 min cold; renders + diffs every host-check
#     golden against live CLI output)
#   - audio component (Layer 2; requires live host)
#
# Measured cold-cache run (baseline at HEAD f5a44b7): ~13 min
# wall, ~520 G `/nix/store` peak. Warm-cache: ~2 min. Full
# tests/static.sh remains the canonical panel + wave-exit gate.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
export ROOT
export FLAKE=${FLAKE:-$ROOT}
export NL_STATIC_CACHE="$ROOT/.static-fast-cache.bootstrap"

# shellcheck source=lib.sh
. "$HERE/lib.sh"

# Honor the same inter-process serializer as tests/static.sh so two
# static-fast.sh invocations don't trample the nix-daemon socket.
# Use a separate lock file from tests/static.sh so the two gates can
# run concurrently if needed (e.g. running static.sh in one terminal
# and static-fast.sh in another). `-o`/`--close` closes the lock fd in
# the spawned child so no descendant (sccache, broker daemons, nix
# evals) inherits it and can leak the lock past our exit; see the
# longer note in tests/static.sh.
if [ -z "${NL_STATIC_NO_LOCK:-}" ] && [ "${1:-}" != "--internal-locked" ]; then
  exec flock -o -x -E 0 -w 5 "$ROOT/.static-fast.lock" "$0" --internal-locked
fi
# Drop the --internal-locked arg before downstream gates see it.
if [ "${1:-}" = "--internal-locked" ]; then
  shift
fi

# Local scratch cache, mirrored from static.sh.
NL_STATIC_CACHE_DIR=$(nl_mktemp .static-fast-cache.XXXXXX)
export NL_STATIC_CACHE="$NL_STATIC_CACHE_DIR"

NL_LOG=${NL_LOG:-$ROOT/.static-fast.log}
export NL_LOG
: > "$NL_LOG"

log() {
  printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2
  printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >> "$NL_LOG"
}

ok() {
  log "  PASS: $*"
}

fail() {
  log "  FAIL: $*"
  exit 1
}

run_gate() {
  local label="$1" cmd="$2"
  log "--> $label"
  if bash -c "$cmd" >> "$NL_LOG" 2>&1; then
    ok "$label"
  else
    log "  FAIL: $label"
    tail -40 "$NL_LOG" >&2 || true
    exit 1
  fi
}

run_script_gate_if_present() {
  local label="$1" path="$2"
  if [ -x "$path" ]; then
    run_gate "$label" "bash '$path'"
  else
    log "  SKIP: $label (not present)"
  fi
}

# ---------------------------------------------------------------------------
# Preflight
# ---------------------------------------------------------------------------
run_script_gate_if_present "tests/preflight-disk-space.sh" "$ROOT/tests/preflight-disk-space.sh"

# ---------------------------------------------------------------------------
# Parse + lint
# ---------------------------------------------------------------------------
log "==> Static fast: parse + lint"

run_gate "nix-instantiate --parse on all .nix files" "
  set -e
  cd '$ROOT'
  pass=0
  for f in \$(find nixos-modules tests -name '*.nix' -type f) flake.nix; do
    if ! nix-instantiate --parse \"\$f\" >/dev/null 2>&1; then
      echo \"PARSE FAIL: \$f\" >&2
      exit 1
    fi
    pass=\$((pass+1))
  done
  echo \"parsed \$pass files\"
"

run_gate "shellcheck --severity=warning on all nixling shell scripts" "
  set -e
  if ! command -v shellcheck >/dev/null 2>&1; then
    sc_path=\$(nix shell --quiet --inputs-from '$ROOT' nixpkgs#shellcheck --command bash -lc 'printf %s \"\$PATH\"')
    PATH=\"\$sc_path:\$PATH\"
    export PATH
  fi
  cd '$ROOT'
  files=\$(find tests scripts harness/ubuntu -maxdepth 1 -name '*.sh' -type f 2>/dev/null | sort)
  if [ -z \"\$files\" ]; then
    echo 'shellcheck: no .sh files found' >&2
    exit 1
  fi
  shellcheck --severity=warning -x \$files
"

run_script_gate_if_present "tests/guest-control-auth-eval.sh" "$ROOT/tests/guest-control-auth-eval.sh"
run_script_gate_if_present "tests/guest-control-token-materializer.sh" "$ROOT/tests/guest-control-token-materializer.sh"
run_script_gate_if_present "tests/guest-control-vsock-eval.sh" "$ROOT/tests/guest-control-vsock-eval.sh"
run_script_gate_if_present "tests/guest-exec-policy-eval.sh" "$ROOT/tests/guest-exec-policy-eval.sh"
run_script_gate_if_present "tests/guest-static-elf.sh" "$ROOT/tests/guest-static-elf.sh"
run_script_gate_if_present "tests/guest-static-consumption-eval.sh" "$ROOT/tests/guest-static-consumption-eval.sh"

run_gate "nix flake check --no-build --all-systems" '
  nix flake check "'$ROOT'" --no-build --all-systems
'

# ---------------------------------------------------------------------------
# Rust workspace
# ---------------------------------------------------------------------------
run_script_gate_if_present "tests/rust-workspace-checks.sh" "$ROOT/tests/rust-workspace-checks.sh"

# ---------------------------------------------------------------------------
#  bundle/schema static gates (pure shell + small Nix evals)
# ---------------------------------------------------------------------------
log "==> Bundle/schema static gates"
for gate in \
  drift-check \
  vms-json-parity \
  static-invariant-uid0 \
  ifname-nix-rust-parity \
  static-invariant-deny-unknown-fields; do
  if [ -x "$ROOT/tests/$gate.sh" ]; then
    run_gate "tests/$gate.sh" "bash '$ROOT/tests/$gate.sh'"
  fi
done

# ---------------------------------------------------------------------------
#  Host-prepare gates (mostly pure shell; some need cargo)
# ---------------------------------------------------------------------------
log "==> Host-prepare gates"

# Provision rustup + compiler support without injecting an unpinned
# cargo/rustc ahead of packages/rust-toolchain.toml. Rust gates that need
# cargo bootstrap the pinned channel through rustup.
if ! command -v cargo >/dev/null 2>&1; then
  rust_path=$(nix shell --quiet --inputs-from "$ROOT" nixpkgs#rustup nixpkgs#stdenv.cc nixpkgs#sccache --command bash -lc 'printf %s "$PATH"')
  PATH="$rust_path:$PATH"
  export PATH
fi

for gate in \
  pidfd-handoff \
  ifname-collision \
  l3-pin-consistency \
  ioctl-negative \
  kernel-module-matrix \
  dag-topo \
  video-contract-eval \
  device-node-matrix; do
  if [ -x "$ROOT/tests/$gate.sh" ]; then
    run_gate "tests/$gate.sh" "bash '$ROOT/tests/$gate.sh'"
  fi
done

# Heavier drift gates intentionally skipped here; they fire in the
# full tests/static.sh used by panel review:
#   - cli-contract-coverage.sh (~7 min cold; builds nixling CLI binary
#     to validate parser/help against docs)
#   - cli-json-drift.sh (~6 min cold; renders + diffs every host-check
#     golden against live CLI output)

log "Static-fast checks OK"
log "(skipped: smoke-eval, assertions-eval, mid-tier evals, manifest contract, broker daemon checks, per-example flake-check, audio.)"
log "(run tests/static.sh before panel dispatch / release-exit gates.)"
