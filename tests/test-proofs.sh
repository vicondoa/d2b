#!/usr/bin/env bash
# tests/test-proofs.sh — `make test-proofs`: clippy + test the standalone proof
# crates under proofs/ (separate Cargo workspaces, not members of packages/).
#
#   * proofs/chunked-stdio-conformance
#   * proofs/w0-ch-connect-proof
#
# These were previously only exercised by the hand-rolled pr-cargo-workspace CI
# job; they now live behind a make target so CI and local runs share one path.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
NL_LOG=${NL_LOG:-/dev/null}
export ROOT NL_LOG

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

cd "$ROOT"

toolchain_file="$ROOT/packages/rust-toolchain.toml"
pinned_channel=$(
  sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]\+\)".*/\1/p' "$toolchain_file" | head -1
)
[ -n "$pinned_channel" ] || { fail "could not read pinned Rust channel from $toolchain_file"; exit 1; }

nl_activate_rust_toolchain_path || true

# Bootstrap the pinned toolchain through rustup/nix when cargo is absent (CI).
if [ -z "${NIXLING_PROOFS_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "cargo and nix both unavailable; cannot run proofs"
    exit 1
  fi
  log "  cargo not on PATH; re-entering via nix shell for pinned Rust $pinned_channel"
  export NIXLING_PROOFS_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" nixpkgs#rustup nixpkgs#stdenv.cc \
    --command bash -lc "
      set -euo pipefail
      rustup toolchain install '$pinned_channel' --profile minimal --component clippy >/dev/null
      exec bash '$ROOT/tests/test-proofs.sh'
    "
fi

export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-$pinned_channel}"
export CARGO_BUILD_RUSTC_WRAPPER="" RUSTC_WRAPPER=""

rc=0
for proof in chunked-stdio-conformance w0-ch-connect-proof; do
  manifest="$ROOT/proofs/$proof/Cargo.toml"
  if [ ! -f "$manifest" ]; then
    log "  SKIP: proofs/$proof (absent)"
    continue
  fi
  log "--> proofs/$proof: clippy + test"
  if cargo clippy --manifest-path "$manifest" --all-targets -- -D warnings \
    && cargo test --manifest-path "$manifest"; then
    ok "proofs/$proof"
  else
    fail "proofs/$proof"
    rc=1
  fi
done

[ "$rc" -eq 0 ] || exit 1
log "test-proofs OK"
